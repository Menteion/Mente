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
import { createHash } from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
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
 * The directory core.hooksPath is to name for the repository of `cwd`: the main checkout's
 * tools/git-hooks, as an absolute path. A relative one is read against the work tree a push runs
 * in, so a worktree of a branch from before the hook, or a push by --git-dir, ran none (the third
 * review of #203); every checkout runs the main checkout's hook, whatever branch it holds, as
 * cargo's runner is the main checkout's tools/capped.mjs (runnerLine).
 */
export function hooksDir(cwd = path.resolve(HERE, "..")) {
  const top = execFileSync("git", ["rev-parse", "--show-toplevel"], { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  return path.join(mainCheckout(top), "tools", "git-hooks");
}

/**
 * Whether the repository of `cwd` runs its own git hooks: core.hooksPath names hooksDir, which
 * holds the pre-push hook (tools/git-hooks/pre-push), refusing a push of a tree the quick checks
 * have not passed. If not, says what to do, as requireRunner does for cargo's runner (Norman,
 * 2026-11-22), and returns false, so that tools/quick.mjs, cargo.mjs and mn.mjs run nothing until it
 * does. The settings' guard refuses a session's write of core.hooksPath (tools/push.mjs), so the
 * command is Norman's to run. The repository is the tools' own checkout, wherever they are run
 * from (cargo.mjs and mn.mjs); quick.mjs names the one it checks.
 */
export function requireHooks({ cwd = path.resolve(HERE, ".."), say = (line) => console.error(line) } = {}) {
  let want;
  try {
    want = hooksDir(cwd);
  } catch (e) {
    say(`local-limits: git could not name the repository of ${cwd}, so whether its pre-push hook runs is unknown, and nothing here runs: ${String(e.stderr || e.message).trim()}`);
    return false;
  }
  let value = "";
  try {
    value = execFileSync("git", ["config", "--get", "core.hooksPath"], { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    value = "";
  }
  const fold = (p) => (WINDOWS ? path.resolve(p).toLowerCase() : path.resolve(p));
  if (value === "" || !path.isAbsolute(value) || fold(value) !== fold(want)) {
    say(`local-limits: core.hooksPath is ${value ? JSON.stringify(value) : "not set"}, not the main checkout's hooks, so git's pre-push hook, which refuses a push of a tree the quick checks have not passed, does not run for every checkout, and nothing here runs until it does. Norman runs, once, in his own terminal:\n\ngit config core.hooksPath ${want.replace(/\\/g, "/")}\n`);
    return false;
  }
  if (!fs.existsSync(path.join(want, "pre-push"))) {
    say(`local-limits: core.hooksPath names ${want}, which holds no pre-push hook: the main checkout is on a commit from before the hook (#203). Bring it up to main; nothing here runs until then.`);
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

/// The main checkout of this repository, whose tools/capped.mjs cargo's runner names: the directory
/// a worktree's .git file points into, or `root` itself.
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

/// The line cargo's configuration needs under [target.<RUNNER_TARGET>].
export const runnerLine = (main = mainCheckout()) => `runner = ["node", ${JSON.stringify(path.join(main, "tools", "capped.mjs").replace(/\\/g, "/"))}, "--mente-only", "--"]`;

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

/// Whether cargo, on this laptop, runs every test binary and program it starts through
/// tools/capped.mjs, however it was started: the tools refuse to run until it does, and say which
/// lines to add where (the review of round 4: CLAUDE.md claimed a runner nothing set or checked).
/// Off Windows, where no job object bounds anything, it holds.
export function requireRunner({ cwd = process.cwd(), env = process.env, say = (line) => console.error(line) } = {}) {
  if (!WINDOWS) return true;
  const words = cargoRunner(cwd, env);
  const held = Array.isArray(words) && words.length === 4 && /^node(\.exe)?$/i.test(path.basename(words[0])) && /[\\/]tools[\\/]capped\.mjs$/.test(words[1]) && fs.existsSync(words[1]) && words[2] === "--mente-only" && words[3] === "--";
  if (held) return true;
  // The variable cargo reads before every file: when it names the runner, it is what to change
  // (the review of PR 195's last round).
  if (env[RUNNER_VARIABLE]) {
    say(`local-limits: ${RUNNER_VARIABLE} is set to ${JSON.stringify(env[RUNNER_VARIABLE])}, which cargo uses as its runner over every configuration file, so a test binary cargo starts would run unbounded, and nothing here runs until it is the capped one. Unset it, or set it to:\n\n${RUNNER_VARIABLE}=node ${path.join(mainCheckout(), "tools", "capped.mjs").replace(/\\/g, "/")} --mente-only --\n`);
    return false;
  }
  const home = env.CARGO_HOME || path.join(os.homedir(), ".cargo");
  say(`local-limits: cargo's runner on this laptop is ${words ? JSON.stringify(words) : "not set"}, so a test binary cargo starts would run unbounded, and nothing here runs until it is the capped one. Add these two lines to ${path.join(home, "config.toml")}:\n\n[target.${RUNNER_TARGET}]\n${runnerLine()}\n`);
  return false;
}

// The processes of the gates' refresher and entries (briefs/gate-tools-from-main.md, "How the
// refresher's processes are run"): an environment of their own making, git by its absolute path,
// a command's whole process tree held to a time, a blob's id as git defines it, and the lock of a
// refresh, which the operating system releases when its holder ends.

/// Variables removed from every process the refresher starts and every git command of the entries'
/// checks: git's own, node's (NODE_OPTIONS preloads code), and those that name certificate files.
const DROPPED = [/^GIT_/i, /^NODE_/i, /^SSL_CERT_FILE$/i, /^SSL_CERT_DIR$/i, /^CURL_CA_BUNDLE$/i];

/**
 * The environment of the gates' processes: `env` with DROPPED removed, and git's system and user
 * configuration closed, so that no setting of either (an insteadOf, a proxy's sslVerify) reaches
 * git, wherever HOME or XDG_CONFIG_HOME points; GIT_CONFIG_GLOBAL is /dev/null, which Git for
 * Windows accepts when node starts it, so no file is left for a command to fill. No prompt for
 * credentials.
 */
export function gateEnv(env = process.env) {
  const kept = Object.fromEntries(Object.entries(env).filter(([key]) => !DROPPED.some((drop) => drop.test(key))));
  return { ...kept, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null", GIT_TERMINAL_PROMPT: "0" };
}

/// Where the gates look for git, in order, by absolute path, never on PATH: the system's, then on
/// Windows Git for Windows' per-user install (through LOCALAPPDATA, a limit the brief names).
export function gitLocations(env = process.env) {
  if (!WINDOWS) return ["/usr/bin/git"];
  const own = env.LOCALAPPDATA ? [path.join(env.LOCALAPPDATA, "Programs", "Git", "cmd", "git.exe").replace(/\\/g, "/")] : [];
  return ["C:/Program Files/Git/cmd/git.exe", ...own];
}

/// The first of gitLocations that exists, or null.
export const systemGit = (env = process.env) => gitLocations(env).find((at) => fs.existsSync(at)) ?? null;

/// A blob's id as git defines it, the SHA-1 of `blob <size>`, a zero byte and the bytes, so that no
/// attribute or filter changes it, and drift is caught even when no git is found.
export const blobId = (bytes) => createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");

/**
 * A runner of one command whose whole process tree is held to a time, run by `node -e`. A time
 * limit on the command alone would kill git and leave its transport, git-remote-http, holding its
 * connection (the eighth review of #203). The tree is never found by process ids, which Windows
 * reuses and does not clear from a dead parent's children (the ninth review):
 * - on posix the command leads a process group of its own, killed whole whenever its time passes
 *   before the runner has seen it close, even once git has exited while a transport holds its pipes:
 *   a group's id is not reused while any member lives (the tenth review);
 * - on Windows the command is tools/capped.ps1, which runs git in a job object that ends every
 *   process in it when the job's owner ends (capped); killing that one direct child, by its own
 *   handle, ends exactly the job's members.
 * It reports once, as JSON: the command's status, output, and whether its time passed.
 */
export const TREE_RUNNER = `
const { spawn } = require("node:child_process");
const [cmd, ...args] = JSON.parse(process.env.TREE_ARGS);
const windows = process.platform === "win32";
const child = spawn(cmd, args, { detached: !windows, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
let out = "", err = "", timedOut = false, finished = false;
child.stdout.setEncoding("utf8").on("data", (d) => (out += d));
child.stderr.setEncoding("utf8").on("data", (d) => (err += d));
const timer = setTimeout(() => {
  timedOut = true;
  if (windows) {
    if (child.exitCode === null && child.signalCode === null) child.kill();
  } else {
    try { process.kill(-child.pid, "SIGKILL"); } catch {}
  }
}, Number(process.env.TREE_TIMEOUT));
const done = (code) => {
  if (finished) return;
  finished = true;
  clearTimeout(timer);
  process.stdout.write(JSON.stringify({ code, out, err, timedOut }));
};
child.on("close", (code) => done(code));
child.on("error", (e) => { err += String(e.message); done(-1); });
`;

/**
 * Runs `run` in `cwd` with `env`, its whole process tree held to `timeoutMs` (TREE_RUNNER): on
 * Windows inside capped.ps1's job, whose own limit, in whole minutes, is a backstop behind the
 * runner's time. Returns { code, out, err, timedOut }; throws only where the runner itself fails.
 */
export function inTime(run, timeoutMs, { env = process.env, cwd } = {}) {
  const job = WINDOWS ? capped(run, { processGB: 4, jobGB: 8, minutes: Math.ceil(timeoutMs / 60000) + 1 }) : null;
  const r = spawnSync(process.execPath, ["-e", TREE_RUNNER], {
    encoding: "utf8",
    cwd,
    env: { ...env, ...(job ? job.env : {}), TREE_ARGS: JSON.stringify(job ? [job.cmd, ...job.args] : run), TREE_TIMEOUT: String(timeoutMs) },
    timeout: timeoutMs + 30000,
    windowsHide: true,
  });
  if (r.error) throw r.error;
  return JSON.parse(r.stdout);
}

/// The variable by which a refresher tells the refresher it starts that the descriptor LOCK_FD it
/// hands down holds the lock (briefs/gate-tools-from-main.md, "The lock").
export const LOCK_FD_VARIABLE = "MENTE_GATES_LOCK_FD";
/// The descriptor a handed-down lock arrives on: the first after standard input, output and error.
export const LOCK_FD = 3;

/// The name of the socket that is the lock of the file `file` on Linux, in the abstract namespace,
/// which no file stands for and the kernel frees when the last descriptor of it closes.
const lockSocket = (file) => `\0mente-gates-lock:${createHash("sha1").update(path.resolve(file)).digest("hex")}`;

/// The lock of `file` taken now, as { fd, release }, or null while another holds it.
function tryLock(file) {
  if (WINDOWS) {
    let fd;
    try {
      fd = fs.openSync(file, fs.constants.O_RDWR | fs.constants.O_CREAT | EXCLUSIVE);
    } catch (e) {
      if (e.code === "EBUSY") return Promise.resolve(null);
      throw e;
    }
    return Promise.resolve({ fd, release: () => fs.closeSync(fd) });
  }
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", (e) => (e.code === "EADDRINUSE" ? resolve(null) : reject(e)));
    server.listen({ path: lockSocket(file) }, () => {
      server.unref();
      resolve({ fd: server._handle.fd, release: () => server.close() });
    });
  });
}

/**
 * Takes the lock of `file`: an open handle the operating system releases when its holder ends,
 * however it ends, so that a kill, Ctrl-C or a reset leaves it free. On Windows it is `file`
 * opened with no sharing, as the build slots are; on Linux, a socket in the abstract namespace
 * named from `file`. Waits at most `waitMs` while another holds it. Resolves to { fd, release }
 * and writes `<file>.who` (`label`, this process and since when), or to null when the time passed.
 */
export async function takeLock(file, { waitMs, label, pollMs = 250 }) {
  const until = Date.now() + waitMs;
  for (;;) {
    const lock = await tryLock(file);
    if (lock) {
      fs.writeFileSync(`${file}.who`, `pid ${process.pid} since ${new Date().toISOString()}: ${label}\n`);
      return lock;
    }
    const left = until - Date.now();
    if (left <= 0) return null;
    await new Promise((resolve) => setTimeout(resolve, Math.min(pollMs, left)));
  }
}

/// Who holds the lock of `file`, as its holder wrote it, or "unknown".
export function lockHolder(file) {
  try {
    return fs.readFileSync(`${file}.who`, "utf8").trim();
  } catch {
    return "unknown";
  }
}

/// The options by which a process hands the lock `lock` down to a process it starts with `stdio`:
/// the lock's descriptor as LOCK_FD, and LOCK_FD_VARIABLE saying so.
export const handDown = (lock, stdio, env) => ({ stdio: [...stdio, lock.fd], env: { ...env, [LOCK_FD_VARIABLE]: String(LOCK_FD) } });

/**
 * Whether this process holds the lock of `file` by the descriptor its starter handed down: the
 * variable names LOCK_FD, and that descriptor is the lock itself, the same file on Windows, the
 * same socket on Linux (/proc/self/fd and /proc/net/unix), never only a claim.
 */
export function inheritedLock(file, env = process.env) {
  if (env[LOCK_FD_VARIABLE] !== String(LOCK_FD)) return false;
  try {
    if (WINDOWS) {
      const held = fs.fstatSync(LOCK_FD, { bigint: true });
      const named = fs.statSync(file, { bigint: true });
      return held.ino === named.ino && held.dev === named.dev;
    }
    const inode = /^socket:\[(\d+)\]$/.exec(fs.readlinkSync(`/proc/self/fd/${LOCK_FD}`))?.[1];
    if (!inode) return false;
    const name = `@${lockSocket(file).slice(1)}`;
    return fs
      .readFileSync("/proc/net/unix", "utf8")
      .split("\n")
      .some((line) => {
        const fields = line.trim().split(/\s+/);
        return fields[6] === inode && fields[7] === name;
      });
  } catch {
    return false;
  }
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
