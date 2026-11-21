// The limits of every local build, test and probe of this repository on the laptop, defined once:
// tools/quick.mjs, tools/cargo.mjs and tools/mn.mjs run their commands through runCapped, in one of
// the machine's build slots (acquireSlot). On 2026-11-22 a test binary committed 107 GB and the
// laptop, where several sessions and their reviewers build and test at once, ran out of memory and
// reset; nothing bounded a local process, only CI's. CI keeps its own limits
// (.github/scripts/limits.sh); this module is for the laptop alone.
//
// - Each command runs in a Windows job object (tools/capped.ps1, which shares its job object with
//   CI's Windows jobs): each process commits at most `processGB`, all of them together at most
//   `jobGB`, and the whole tree ends at `minutes`. A runaway fails alone.
// - The machine runs at most SLOTS builds at once, across every session and worktree, each at
//   JOBS cores (CARGO_BUILD_JOBS and RUST_TEST_THREADS). A slot is a lock file opened with no
//   sharing (libuv's UV_FS_O_EXLOCK): Windows drops the lock when its holder ends, however it
//   ends, so a crash or a reset leaves no slot taken.
//
// Off Windows the commands run unbounded and take no slot, and each says so: the laptop is the
// machine this guards.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
/// The script that runs a command in a job object.
export const CAPPED = path.join(HERE, "capped.ps1");
/// Builds the machine runs at once.
export const SLOTS = 2;
/// The cores each build takes: half the machine's.
export const JOBS = String(Math.max(1, Math.floor(os.availableParallelism() / 2)));
/// The limits of a build or a test run, and of an ad-hoc run of mn.
export const BUILD = Object.freeze({ processGB: 8, jobGB: 16, minutes: 30 });
export const PROBE = Object.freeze({ processGB: 4, jobGB: 8, minutes: 5 });
/// libuv's UV_FS_O_EXLOCK: Windows opens the file sharing it with no other handle.
const EXCLUSIVE = 0x10000000;
const WINDOWS = process.platform === "win32";

/// Where the slots' lock files are: one directory for the whole machine, or MENTE_SLOTS_DIR, which
/// the tests of this module set to keep apart from the real ones.
export const slotsDir = () => process.env.MENTE_SLOTS_DIR ?? path.join(process.env.LOCALAPPDATA ?? os.tmpdir(), "mente-slots");

const sleep = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

/// Takes one of the machine's SLOTS build slots for this process, waiting while all are held and
/// saying once who holds them; `label` says what it is for. Returns { slot, release }: the slot is
/// also let go when this process ends.
export function acquireSlot(label, { pollMs = 2000, say = (line) => console.error(line) } = {}) {
  if (!WINDOWS) {
    say("local-limits: no build slots off Windows");
    return { slot: 0, release() {} };
  }
  const dir = slotsDir();
  fs.mkdirSync(dir, { recursive: true });
  let told = false;
  for (;;) {
    for (let k = 1; k <= SLOTS; k++) {
      let fd;
      try {
        fd = fs.openSync(path.join(dir, `slot-${k}.lock`), fs.constants.O_RDWR | fs.constants.O_CREAT | EXCLUSIVE);
      } catch (e) {
        if (e.code === "EBUSY") continue;
        throw e;
      }
      fs.writeFileSync(path.join(dir, `slot-${k}.who`), `pid ${process.pid} since ${new Date().toISOString()}: ${label}\n`);
      return { slot: k, release: () => fs.closeSync(fd) };
    }
    if (!told) {
      const who = [];
      for (let k = 1; k <= SLOTS; k++) {
        let text = "";
        try {
          text = fs.readFileSync(path.join(dir, `slot-${k}.who`), "utf8").trim();
        } catch {
          text = "held";
        }
        who.push(`slot ${k}: ${text}`);
      }
      say(`local-limits: waiting for one of the machine's ${SLOTS} build slots (${who.join("; ")})`);
      told = true;
    }
    sleep(pollMs);
  }
}

/// Windows PowerShell, by its path in System32, which runs capped.ps1: the Store's pwsh, started in
/// a job, takes every process it starts out of it (the review of 8f03074).
export const WINDOWS_POWERSHELL = path.join(process.env.SystemRoot || "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");

/// How capped.ps1 runs `run` under `limits`: Windows PowerShell on the script, and the command, the
/// limits and this process's id as JSON in one variable, MENTE_CAPPED, never as arguments, which
/// PowerShell's parser would change (the review of 8f03074: `--skip=a::b` arrived as two arguments,
/// and `--%` vanished). The script ends the command when this process ends.
export function capped(run, limits) {
  const call = { command: run, processGB: limits.processGB, jobGB: limits.jobGB, minutes: limits.minutes, parent: process.pid };
  return { cmd: WINDOWS_POWERSHELL, args: ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", CAPPED], env: { MENTE_CAPPED: JSON.stringify(call) } };
}

/// Whether `dir` lies in a checkout of this repository: it or a parent holds Cargo.toml and this
/// very tool.
export function inMente(dir) {
  for (let at = path.resolve(dir); ; at = path.dirname(at)) {
    if (fs.existsSync(path.join(at, "Cargo.toml")) && fs.existsSync(path.join(at, "tools", "capped.ps1"))) return true;
    if (path.dirname(at) === at) return false;
  }
}

/// Runs `run` under `limits`, its output this process's, at the slot's share of the cores, in
/// `cwd`; returns its status, 1 when it could not start. With `menteOnly`, a command whose
/// directory lies outside a checkout of this repository runs untouched, its environment as given:
/// cargo's runner for every project of the machine. Off Windows every command runs unbounded, and
/// says so.
export function runCapped(run, limits, { cwd, menteOnly = false } = {}) {
  let spawned;
  if (menteOnly && !inMente(cwd ?? process.cwd())) {
    spawned = { cmd: run[0], args: run.slice(1), env: process.env };
  } else if (!WINDOWS) {
    console.error("local-limits: no job object off Windows: the command runs unbounded");
    spawned = { cmd: run[0], args: run.slice(1), env: { ...process.env, CARGO_BUILD_JOBS: JOBS, RUST_TEST_THREADS: JOBS } };
  } else {
    const { cmd, args, env } = capped(run, limits);
    spawned = { cmd, args, env: { ...process.env, CARGO_BUILD_JOBS: JOBS, RUST_TEST_THREADS: JOBS, ...env } };
  }
  const r = spawnSync(spawned.cmd, spawned.args, { stdio: "inherit", env: spawned.env, cwd });
  if (r.error) console.error(`local-limits: ${run[0]} did not start: ${r.error.message}`);
  return r.status ?? 1;
}

/// The most minutes `--minutes` may give a run.
export const MAX_MINUTES = 60;

/// Reads `--minutes N` from the front of `args`, if it is there: the arguments left, and the
/// limits. N is above 0 and at most MAX_MINUTES.
export function minutesFrom(args, limits) {
  if (args[0] !== "--minutes") return { args, limits };
  const minutes = Number(args[1]);
  if (!(minutes > 0 && minutes <= MAX_MINUTES)) {
    console.error(`local-limits: --minutes takes a number above 0 and at most ${MAX_MINUTES}, not ${args[1]}`);
    process.exit(2);
  }
  return { args: args.slice(2), limits: { ...limits, minutes } };
}
