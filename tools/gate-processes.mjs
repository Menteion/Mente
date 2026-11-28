// The local gates' processes (briefs/gate-tools-from-main.md): a command's whole process tree held
// to a time, and the lock of a refresh, which the operating system releases when its holder ends.
// tools/gates.mjs runs its asks of the remote and its work through them, and tools/push.mjs the
// pre-push hook's ask of the remote. A module of the set with no effect at import: what imports
// it, the settings' guard through push.mjs among them, runs on whatever node it runs on.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { capped } from "./local-limits.mjs";

const WINDOWS = process.platform === "win32";

/// libuv's UV_FS_O_EXLOCK: Windows opens the file sharing it with no other handle, as the build
/// slots of local-limits.mjs do.
const EXCLUSIVE_OPEN = 0x10000000;

/**
 * A runner of one command whose whole process tree is held to a time, run by `node -e`. A time
 * limit on the command alone would kill git and leave its transport, git-remote-http, holding its
 * connection (the eighth review of #203). The tree is never found by process ids, which Windows
 * reuses and does not clear from a dead parent's children (the ninth review):
 * - on posix the command leads a process group of its own, killed whole whenever its time passes
 *   before the runner has seen it close, even once git has exited while a transport holds its pipes:
 *   a group's id is not reused while any member lives (the tenth review);
 * - on Windows the command is tools/capped.ps1, which runs git in a job object that ends every
 *   process in it when the job's owner ends (local-limits.mjs, capped); killing that one direct child, by its own
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
 * Runs `run` in `cwd` with `env`, which every caller gives, its whole process tree held to `timeoutMs` (TREE_RUNNER): on
 * Windows inside capped.ps1's job, whose own limit, in whole minutes, is a backstop behind the
 * runner's time. Returns { code, out, err, timedOut }; throws only where the runner itself fails.
 */
export function inTime(run, timeoutMs, { env, cwd }) {
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
      fd = fs.openSync(file, fs.constants.O_RDWR | fs.constants.O_CREAT | EXCLUSIVE_OPEN);
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
