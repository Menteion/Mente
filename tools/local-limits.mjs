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

import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { bootstrapText, gateEnv, inMente, mainCheckoutWords, systemGit } from "./gate-entries/verify.mjs";

export { inMente };
import { SYSTEM_POWERSHELL, storePowerShell, withoutStoreApps } from "./store-apps.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
/// The script that runs a command in a job object.
export const CAPPED = path.join(HERE, "capped.ps1");
/// Builds the machine runs at once.
export const SLOTS = 2;
/// The cores each build takes: half the machine's.
export const JOBS = String(Math.max(1, Math.floor(os.availableParallelism() / 2)));
/// The limits of a build or a test run, and of an ad-hoc run of mn.
export const BUILD = Object.freeze({ processGB: 8, jobGB: 16, minutes: 30 });
/// The file of a checkout's git directory (`git rev-parse --git-path`) that holds the id of the tree
/// the last passing run of tools/quick.mjs read, which the push guard (tools/push.mjs) compares
/// with the tree of each commit a push would send.
export const PASSED = "quick-passed";
/**
 * The directory of the gates' copies and stable entries for the repository of `cwd`: mente-gates/
 * in its common git directory, the same for every worktree (briefs/gate-tools-from-main.md). The
 * tools give their own checkout; nothing here reads this module's own place, which inside a copy
 * would name the copy.
 */
export function gatesDir(cwd = process.cwd()) {
  try {
    return path.join(execFileSync(systemGit() ?? "git", ["rev-parse", "--path-format=absolute", "--git-common-dir"], { cwd, env: gateEnv(), encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], windowsHide: true }).trim(), "mente-gates");
  } catch (e) {
    throw new Error(String(e.stderr || e.message).trim());
  }
}

/// The directory core.hooksPath names: the stable hook entry of the gates in `gates`, a path that
/// never changes, so that Norman writes it once.
export const hooksDir = (gates) => path.join(gates, "entry", "hooks");

/// The file of cargo's configuration on this laptop that holds its runner.
const cargoConfig = (env = process.env) => path.join(env.CARGO_HOME || path.join(os.homedir(), ".cargo"), "config.toml");

/// The bootstrap line for `gates`, as every entry prints it (verify.mjs, bootstrapText), this
/// checkout's own tools/gate-set.json its last source.
const bootstrapWords = (gates) => bootstrapText(gates, { checkout: path.resolve(HERE, "..") });

// Between this round's two pull requests (briefs/gate-tools-from-main.md, "The rollout"): while no
// copy of main's gates is installed, the tools keep today's working-tree gates, the main checkout's
// tools/capped.mjs and tools/git-hooks, and say loudly how to install the copy. The second pull
// request removes this fallback, and everything marked FALLBACK with it.

/// FALLBACK: the main checkout of this repository, whose tools/capped.mjs and tools/git-hooks are
/// today's working-tree gates: the directory a worktree's .git file points into, or `root` itself.
export function mainCheckout(root = path.resolve(HERE, "..")) {
  try {
    const dotgit = path.join(root, ".git");
    if (fs.statSync(dotgit).isFile()) {
      const at = /^gitdir:\s*(.+?)\s*$/m.exec(fs.readFileSync(dotgit, "utf8"))?.[1];
      const main = at ? path.resolve(root, at).replace(/[\\/]\.git[\\/]worktrees[\\/][^\\/]+$/, "") : null;
      if (main && main !== path.resolve(root, at)) return main;
    }
  } catch {
    // A checkout whose .git cannot be read is taken as the main one.
  }
  return root;
}

/// FALLBACK: the runner line of today's working-tree gates.
export const workingTreeRunnerLine = (main = mainCheckout()) => `runner = ["node", ${JSON.stringify(path.join(main, "tools", "capped.mjs").replace(/\\/g, "/"))}, "--mente-only", "--"]`;

/// FALLBACK: the hooks of today's working-tree gates for the repository of `cwd`.
function workingTreeHooks(cwd) {
  const top = execFileSync("git", ["rev-parse", "--show-toplevel"], { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  return path.join(mainCheckout(top), "tools", "git-hooks");
}

/// Two paths, the same directory: Windows' are compared without case.
const fold = (p) => (WINDOWS ? path.resolve(p).toLowerCase() : path.resolve(p));

/// The value of core.hooksPath for the repository of `cwd`, or "".
function hooksPathOf(cwd) {
  try {
    return execFileSync("git", ["config", "--get", "core.hooksPath"], { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    return "";
  }
}

/// FALLBACK: whether Norman's lines name the gates' copy for the repository of `cwd`: core.hooksPath
/// names the stable hook entry. Until the round's second pull request the tools take their mode
/// from his lines, never from what .git/mente-gates holds, so that a copy installed early by any
/// session, or half installed, changes nothing until he points his lines at it.
function namesCopy(cwd, gates) {
  const value = hooksPathOf(cwd);
  return value !== "" && path.isAbsolute(value) && fold(value) === fold(hooksDir(gates));
}

/// FALLBACK: what the tools say while Norman's lines name today's working-tree gates.
function fallbackWords(gates, installed) {
  const lines = `git's hooks:\n\ngit config core.hooksPath ${hooksDir(gates).replace(/\\/g, "/")}\n\nthen cargo's runner, under [target.${RUNNER_TARGET}] in ${cargoConfig()}:\n\n${runnerLine(gates)}\n`;
  const branches = "Before Norman's lines change, every open branch merges main: a branch from before this round asks for today's lines, and refuses once they change.";
  if (installed) return `local-limits: WARNING: a copy of main's gates is installed in ${gates}, but Norman's lines still name today's working-tree gates, the main checkout's tools/capped.mjs and tools/git-hooks, which any checkout of it can change, so these tools run those. ${branches} Norman then points, in his own terminal and in this order, ${lines}`;
  return `local-limits: WARNING: no copy of main's gates is installed in ${gates}, so these tools still run today's working-tree gates, the main checkout's tools/capped.mjs and tools/git-hooks, which any checkout of it can change. ${branches} Norman, in his own terminal and in this order, runs the bootstrap from ${mainCheckoutWords(gates)}:\n\n${bootstrapWords(gates)}\n\nand once it has succeeded, points ${lines}`;
}

/**
 * Refreshes the gates of the repository of `cwd` (briefs/gate-tools-from-main.md, "Who writes it"):
 * runs the stable refresh entry, by this node and with the gates' environment, which checks the
 * copy in use and runs its refresher, before requireRunner and requireHooks, since the runner line
 * and the hooks name entries a first install writes. Where Norman's lines name the copy, returns
 * "copy" when a verified copy is in place, and false when none is, having said the one command that
 * clears it. Where they name today's working-tree gates, it still refreshes a copy if one is
 * current, never finishing an install left half done, says loudly what is left to do, and returns
 * "fallback": the tools then keep today's gates (FALLBACK, until the round's second pull request).
 */
export function refreshGates({ cwd = process.cwd(), say = (line) => console.error(line) } = {}) {
  let gates;
  try {
    gates = gatesDir(cwd);
  } catch (e) {
    say(`local-limits: git could not name the repository of ${cwd}, so its gates are unknown, and nothing here runs: ${e.message}`);
    return false;
  }
  const entry = path.join(gates, "entry", "refresh.mjs");
  // A copy is installed once `current` names one: the entries alone, which a bootstrap writes
  // before any copy is confirmed, are not one.
  const installed = fs.existsSync(path.join(gates, "current"));
  const refreshed = () => {
    const r = spawnSync(process.execPath, [entry], { stdio: "inherit", env: gateEnv(), cwd, windowsHide: true });
    if (r.error) say(`local-limits: the refresh entry did not start: ${r.error.message}`);
    return !r.error && r.status === 0;
  };
  if (namesCopy(cwd, gates)) {
    // The refresh entry also finishes an install a kill left half done.
    if (!fs.existsSync(entry)) {
      say(`local-limits: core.hooksPath names the gates' stable hook entry, but ${installed ? `the refresh entry ${entry} is missing` : `no copy of main's gates is installed in ${gates}`}, so nothing here runs until Norman, in his own terminal, runs the bootstrap from ${mainCheckoutWords(gates)}:\n\n${bootstrapWords(gates)}\n`);
      return false;
    }
    return refreshed() ? "copy" : false;
  }
  say(fallbackWords(gates, installed));
  // FALLBACK: only a current copy is refreshed. An install left half done waits for the copy mode or
  // the bootstrap, so that no start pays for a copy its tools do not run.
  if (installed && fs.existsSync(entry) && !refreshed()) say(`local-limits: the copy of main's gates in ${gates} was not refreshed, which changes nothing until Norman's lines name it`);
  return "fallback";
}

/**
 * Whether the repository of `cwd` runs the gates' pre-push hook: core.hooksPath names the stable
 * hook entry (hooksDir), as an absolute path, and it holds the hook, which runs the copy's
 * tools/git-hooks/pre-push, refusing a push of a tree the quick checks have not passed. If not,
 * says what to do, as requireRunner does for cargo's runner (Norman, 2026-11-22), and returns
 * false, so that tools/quick.mjs, cargo.mjs and mn.mjs run nothing until it does. The settings'
 * guard refuses a session's write of core.hooksPath (tools/push.mjs), so the command is Norman's to
 * run. cargo.mjs and mn.mjs give their own checkout; quick.mjs, the one it checks.
 */
export function requireHooks({ cwd = process.cwd(), say = (line) => console.error(line), fallback = false } = {}) {
  let want;
  try {
    want = fallback ? workingTreeHooks(cwd) : hooksDir(gatesDir(cwd));
  } catch (e) {
    say(`local-limits: git could not name the repository of ${cwd}, so whether its pre-push hook runs is unknown, and nothing here runs: ${e.message}`);
    return false;
  }
  const value = hooksPathOf(cwd);
  if (value === "" || !path.isAbsolute(value) || fold(value) !== fold(want)) {
    say(`local-limits: core.hooksPath is ${value ? JSON.stringify(value) : "not set"}, not ${fallback ? "the main checkout's hooks" : "the gates' stable hook entry"}, so git's pre-push hook, which refuses a push of a tree the quick checks have not passed, does not run main's gates for every checkout, and nothing here runs until it does. Norman runs, once, in his own terminal:\n\ngit config core.hooksPath ${want.replace(/\\/g, "/")}\n`);
    return false;
  }
  if (!fs.existsSync(path.join(want, "pre-push"))) {
    say(`local-limits: core.hooksPath names ${want}, which holds no pre-push hook; Norman runs the bootstrap from ${mainCheckoutWords(path.dirname(path.dirname(want)))}:\n\n${bootstrapWords(path.dirname(path.dirname(want)))}\n`);
    return false;
  }
  return true;
}
export const PROBE = Object.freeze({ processGB: 4, jobGB: 8, minutes: 5 });
/// libuv's UV_FS_O_EXLOCK: Windows opens the file sharing it with no other handle.
const EXCLUSIVE = 0x10000000;
const WINDOWS = process.platform === "win32";

/// Where the slots' lock files are: one directory for the whole machine, or MENTE_SLOTS_DIR, which
/// the tests of this module set to keep apart from the real ones.
export const slotsDir = () => process.env.MENTE_SLOTS_DIR ?? path.join(process.env.LOCALAPPDATA ?? os.tmpdir(), "mente-slots");

const sleep = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

/// The variable by which a process holding a slot tells every process it starts which one, as
/// `<slot> <directory>`: a tool started within it, as quick's checks start the tools' tests and
/// they start cargo.mjs, runs in the slot its tree holds, and takes no second one, which it would
/// wait for while its tree held the first (the flake of stack-budget's first quick). A claim counts
/// only while that slot of the same directory is held. That its holder is an ancestor, not another
/// session's process, is held by review only.
export const SLOT_HELD = "MENTE_SLOT_HELD";

/// Whether the slot lock `file` is held now, by any process.
function slotHeld(file) {
  try {
    fs.closeSync(fs.openSync(file, fs.constants.O_RDWR | fs.constants.O_CREAT | EXCLUSIVE));
    return false;
  } catch (e) {
    return e.code === "EBUSY";
  }
}

/// Takes one of the machine's SLOTS build slots for this process, waiting while all are held and
/// saying once who holds them; `label` says what it is for. A process its tree's slot already
/// covers (SLOT_HELD) takes none. Returns { slot, release, covered }: the slot is also let go when
/// this process ends.
export function acquireSlot(label, { pollMs = 2000, say = (line) => console.error(line) } = {}) {
  if (!WINDOWS) {
    say("local-limits: no build slots off Windows");
    return { slot: 0, release() {}, covered: false };
  }
  const dir = slotsDir();
  fs.mkdirSync(dir, { recursive: true });
  const claim = /^(\d+) (.+)$/s.exec(process.env[SLOT_HELD] ?? "");
  if (claim && fold(claim[2]) === fold(dir) && slotHeld(path.join(dir, `slot-${claim[1]}.lock`))) {
    return { slot: Number(claim[1]), release() {}, covered: true };
  }
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
      process.env[SLOT_HELD] = `${k} ${dir}`;
      return {
        slot: k,
        release: () => {
          fs.closeSync(fd);
          delete process.env[SLOT_HELD];
        },
        covered: false,
      };
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
    if (storePowerShell(run[0])) {
      console.error(`local-limits: ${run[0]} is the Store's PowerShell, or a program of a WindowsApps directory like it, which takes every process it starts out of the job: run ${SYSTEM_POWERSHELL} instead`);
      return 2;
    }
    const { cmd, args, env } = capped(run, limits);
    spawned = { cmd, args, env: { ...withoutStoreApps(process.env), CARGO_BUILD_JOBS: JOBS, RUST_TEST_THREADS: JOBS, ...env } };
  }
  const r = spawnSync(spawned.cmd, spawned.args, { stdio: "inherit", env: spawned.env, cwd });
  if (r.error) console.error(`local-limits: ${run[0]} did not start: ${r.error.message}`);
  return r.status ?? 1;
}

/// The target cargo builds for on the laptop, whose runner the tools require.
export const RUNNER_TARGET = "x86_64-pc-windows-msvc";
/// The variable that names that runner over every configuration file.
export const RUNNER_VARIABLE = `CARGO_TARGET_${RUNNER_TARGET.toUpperCase().replace(/-/g, "_")}_RUNNER`;

/// The line cargo's configuration needs under [target.<RUNNER_TARGET>]: the stable runner entry of
/// the gates in `gates`, a path that never changes, so that Norman writes it once.
export const runnerLine = (gates) => `runner = ["node", ${JSON.stringify(path.join(gates, "entry", "capped.mjs").replace(/\\/g, "/"))}, "--mente-only", "--"]`;

/// The runner a configuration file's text names for RUNNER_TARGET, its words, or null: under the
/// table [target.<RUNNER_TARGET>], or as the dotted key before any table.
export function runnerIn(text) {
  let table = "";
  for (const raw of String(text).split(/\r?\n/)) {
    const line = raw.replace(/^\s+|\s+$/g, "");
    const header = /^\[\s*([^\]]+?)\s*\]$/.exec(line);
    if (header) {
      table = header[1].replace(/["']/g, "");
      continue;
    }
    const key = /^([A-Za-z0-9_.\-"']+)\s*=\s*(.+)$/.exec(line);
    if (!key) continue;
    const name = `${table ? `${table}.` : ""}${key[1].replace(/["']/g, "")}`;
    if (name !== `target.${RUNNER_TARGET}.runner`) continue;
    const value = key[2].replace(/\s+#.*$/, "");
    try {
      const parsed = JSON.parse(value.replace(/'([^']*)'/g, (_, s) => JSON.stringify(s)));
      return Array.isArray(parsed) ? parsed.map(String) : String(parsed).trim().split(/\s+/);
    } catch {
      return null;
    }
  }
  return null;
}

/// The runner cargo uses for RUNNER_TARGET from `cwd`: CARGO_TARGET_<TARGET>_RUNNER, or the first
/// configuration file that names one, as cargo reads them, from `cwd` up and then CARGO_HOME.
export function cargoRunner(cwd = process.cwd(), env = process.env) {
  const variable = env[RUNNER_VARIABLE];
  if (variable) return variable.trim().split(/\s+/);
  const files = [];
  for (let at = path.resolve(cwd); ; at = path.dirname(at)) {
    files.push(path.join(at, ".cargo", "config.toml"), path.join(at, ".cargo", "config"));
    if (path.dirname(at) === at) break;
  }
  const home = env.CARGO_HOME || path.join(os.homedir(), ".cargo");
  files.push(path.join(home, "config.toml"), path.join(home, "config"));
  for (const file of files) {
    let text;
    try {
      text = fs.readFileSync(file, "utf8");
    } catch {
      continue;
    }
    const words = runnerIn(text);
    if (words) return words;
  }
  return null;
}

/// Whether cargo, on this laptop, runs every test binary and program it starts through the gates'
/// stable runner entry, <common git directory>/mente-gates/entry/capped.mjs, which runs the copy's
/// tools/capped.mjs, however cargo was started: the tools refuse to run until it does, and say which
/// lines to add where (the review of round 4: CLAUDE.md claimed a runner nothing set or checked).
/// Off Windows, where no job object bounds anything, it holds.
export function requireRunner({ cwd = process.cwd(), env = process.env, say = (line) => console.error(line), fallback = false } = {}) {
  if (!WINDOWS) return true;
  const words = cargoRunner(cwd, env);
  // FALLBACK: while Norman's lines name today's hooks, either runner holds, today's or the gates'
  // stable entry, since his lines change one at a time; the entry checks its own copy and refuses,
  // naming the remedy, while none is current.
  const entryRunner = /[\\/]mente-gates[\\/]entry[\\/]capped\.mjs$/;
  const runner = fallback ? /[\\/](tools|mente-gates[\\/]entry)[\\/]capped\.mjs$/ : entryRunner;
  const held = Array.isArray(words) && words.length === 4 && /^node(\.exe)?$/i.test(path.basename(words[0])) && runner.test(words[1]) && fs.existsSync(words[1]) && words[2] === "--mente-only" && words[3] === "--";
  // The gates' runner entry, named before an install wrote it: cargo would die on node's own error.
  const unwritten = Array.isArray(words) && words.length > 1 && entryRunner.test(words[1]) && !fs.existsSync(words[1]);
  let gates;
  try {
    gates = gatesDir(cwd);
  } catch {
    gates = "<common git directory>/mente-gates";
  }
  const file = fallback ? path.join(mainCheckout(), "tools", "capped.mjs") : path.join(gates, "entry", "capped.mjs");
  const line = fallback ? workingTreeRunnerLine() : runnerLine(gates);
  if (held) return true;
  if (unwritten) {
    say(`local-limits: cargo's runner names the gates' runner entry, ${words[1]}, which no install has written yet, so a test binary cargo starts would die on node's own error, and nothing here runs until it stands. Norman runs the bootstrap from ${mainCheckoutWords(gates)}, in his own terminal:\n\n${bootstrapWords(gates)}\n`);
    return false;
  }
  // The variable cargo reads before every file: when it names the runner, it is what to change
  // (the review of PR 195's last round).
  if (env[RUNNER_VARIABLE]) {
    say(`local-limits: ${RUNNER_VARIABLE} is set to ${JSON.stringify(env[RUNNER_VARIABLE])}, which cargo uses as its runner over every configuration file, so a test binary cargo starts would run unbounded, and nothing here runs until it is the capped one. Unset it, or set it to:\n\n${RUNNER_VARIABLE}=node ${file.replace(/\\/g, "/")} --mente-only --\n`);
    return false;
  }
  say(`local-limits: cargo's runner on this laptop is ${words ? JSON.stringify(words) : "not set"}, so a test binary cargo starts would run unbounded, and nothing here runs until it is the ${fallback ? "capped one" : "gates' capped one"}. Add these two lines to ${cargoConfig(env)}:\n\n[target.${RUNNER_TARGET}]\n${line}\n`);
  return false;
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
