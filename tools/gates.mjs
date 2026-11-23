// The local gates' refresher (briefs/gate-tools-from-main.md): it installs, under the common git
// directory's mente-gates/, a copy of the set (tools/gate-set.json) of the commit the remote
// confirms is main, written from git's objects of that commit, never from a working tree; the
// stable entries of mente-gates/entry/ run the copy that mente-gates/current names.
//
//   node <common git directory>/mente-gates/<commit or staging directory>/tools/gates.mjs
//
// It takes no argument, and starts only from a copy under mente-gates/: tools/quick.mjs, cargo.mjs
// and mn.mjs run it through the refresh entry when they start; the bootstrap starts a staged one.
//
// - It asks the remote only when the local origin/main names a commit other than `current`, at the
//   URL its own copy declares, which must be https, and installs that commit's set only when the
//   remote names the same commit. An answer that keeps the current copy is remembered for that
//   local commit for a while, so an offline machine waits out the time once.
// - The refresher of the copy in use writes nothing but a staging directory of the new set, under
//   the lock, then starts the new set's refresher, handing the lock down. That one checks its whole
//   directory against the commit's tree, confirms the commit with the remote itself, and installs:
//   the manifest, the rename to the commit's name, the entries, then `current`, so that a kill at
//   any step leaves a state the next refresh completes.
// - With nothing to install, it restores the entries of the current copy, so that the refresh a
//   stale entry names clears it.
// - A refresh runs under a time limit with a tree kill, its work in a child process of its own.

import { spawn, spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { handDown, inTime, inheritedLock, lockHolder, takeLock } from "./gate-processes.mjs";
import { COMMIT, ENTRIES, MAIN_REF, NODE_ENGINES, STAGING, bootstrapLine, commitOf, declaredUrl, entriesHold, entryHolds, filesOf, gateEnv, git, gitLocations, laggingSince, localMain, manifestText, mismatch, nodeMeetsFloor, readManifest, setPath, systemGit } from "./gate-entries/verify.mjs";

const SELF = fileURLToPath(import.meta.url);
const HERE = path.dirname(SELF);

/// How long the remote has to say which commit is main.
export const REMOTE_MS = 60000;
/// How long `git ls-remote --get-url`, a local process, has.
export const GET_URL_MS = 15000;
/// How long a start of quick, cargo or mn waits for the lock before it runs the current copy.
export const WAIT_MS = 20000;
/// How long writing a set may take.
export const WRITE_MS = 60000;
/// The time limit of a refresh: both refreshers' asks of the remote, the confirming one's and the
/// new set's, the wait for the lock, and writing a set, so that a slow but legitimate refresh is
/// not killed at every start.
export const REFRESH_MS = 2 * (GET_URL_MS + REMOTE_MS) + WAIT_MS + WRITE_MS;
/// How long an answer that keeps the current copy is remembered for its local commit.
export const REMEMBER_MS = 10 * 60000;
/// How long an answer is remembered while no copy is current: an install left half done is tried
/// again soon, and a remote that does not answer costs a start at most once in this time.
export const RETRY_MS = 60000;

const ASIDE = /^aside-([0-9a-f]{40})-[0-9a-f]+$/;
const WINDOWS = process.platform === "win32";

const unique = () => randomBytes(6).toString("hex");

/// The commit `current` names in `gates`, or null.
function readCurrent(gates) {
  try {
    const text = fs.readFileSync(path.join(gates, "current"), "utf8").trim();
    return COMMIT.test(text) ? text : null;
  } catch {
    return null;
  }
}

/// Writes `data` to `file` by a new file renamed over it, so that a reader sees the old or the new,
/// whole, never a file replaced in place; with `mode`, the new file has it.
function replaceFile(file, data, mode) {
  const tmp = `${file}.tmp-${unique()}`;
  fs.writeFileSync(tmp, data, mode ? { mode } : undefined);
  if (mode && !WINDOWS) fs.chmodSync(tmp, mode);
  try {
    fs.renameSync(tmp, file);
  } catch (e) {
    fs.rmSync(tmp, { force: true });
    throw e;
  }
}

/// The git the context runs: the system's, or none where no git of the list is found (or the
/// seam says none is).
const binOf = (ctx) => (ctx.noGit ? null : systemGit());

/// The set `commit` declares, from its objects: its URL and its files, each a blob of mode 100644
/// or 100755 (verify.mjs, filesOf); throws, naming why it cannot be installed.
function setOf(ctx, commit) {
  const declared = JSON.parse(git(ctx.common, ["cat-file", "blob", `${commit}:tools/gate-set.json`], { bin: binOf(ctx) }));
  if (!Array.isArray(declared.set) || !declared.set.every(setPath) || !declared.set.includes("tools/gate-set.json") || !declared.set.includes("tools/gates.mjs")) {
    throw new Error(`the tools/gate-set.json of ${commit.slice(0, 12)} declares no set the gates can install`);
  }
  return { url: declared.url, files: filesOf(git(ctx.common, ["ls-tree", "-r", "-z", "--full-tree", commit, "--", ...declared.set], { bin: binOf(ctx) }), declared.set) };
}

/// Writes `files` into the new directory `dir`, each with exactly the bytes `git cat-file blob`
/// gives (never filtered, converted or checked out, so its id in node is its id in the tree) and
/// the mode the tree records.
function writeSet(ctx, files, dir) {
  fs.mkdirSync(dir);
  for (const f of files) {
    const at = path.join(dir, ...f.path.split("/"));
    fs.mkdirSync(path.dirname(at), { recursive: true });
    const mode = f.mode === "100755" ? 0o755 : 0o644;
    fs.writeFileSync(at, git(ctx.common, ["cat-file", "blob", f.id], { encoding: "buffer", bin: binOf(ctx) }), { mode });
    if (!WINDOWS) fs.chmodSync(at, mode);
    ctx.halfway?.();
  }
}

/// Whether `dir` holds exactly `files` and a manifest that names them.
function holds(dir, files) {
  try {
    return manifestText(readManifest(fs.readFileSync(path.join(dir, "manifest"), "utf8"))) === manifestText(files) && mismatch(dir, files) === null;
  } catch {
    return false;
  }
}

/// Moves `dir`, a copy of `commit` that cannot be reused, aside, for the sweep to remove.
const moveAside = (gates, dir, commit) => fs.renameSync(dir, path.join(gates, `aside-${commit}-${unique()}`));

/// Removes every copy, staging directory and copy moved aside in `gates` but those of `keep`,
/// skipping any a process still uses.
function sweep(gates, keep) {
  for (const name of fs.readdirSync(gates)) {
    if (keep.includes(name) || !(COMMIT.test(name) || STAGING.test(name) || ASIDE.test(name))) continue;
    try {
      fs.rmSync(path.join(gates, name), { recursive: true });
    } catch {
      // In use: a later sweep removes it.
    }
  }
}

/// The answer that keeps the current copy, remembered for the local commit `local`, or null.
function remembered(gates, local) {
  try {
    const kept = JSON.parse(fs.readFileSync(path.join(gates, "kept"), "utf8"));
    if (kept.local === local && Date.now() - kept.at < (kept.ms ?? REMEMBER_MS)) return kept.why;
  } catch {
    // Nothing remembered.
  }
  return null;
}

/// Remembers why the current copy stays while the local origin/main names `local`, for `ms`. What
/// is remembered can only keep the current copy, never install one.
function remember(gates, local, why, ms) {
  try {
    replaceFile(path.join(gates, "kept"), JSON.stringify({ local, at: Date.now(), ms, why }));
  } catch {
    // Not remembered: the next start asks again.
  }
}

/**
 * The commit the remote at `url` names main, asked by the system's git in an empty directory this
 * creates outside the common git directory, with GIT_CEILING_DIRECTORIES at its parent, so that git
 * finds no repository and no repository's configuration reaches it; `ls-remote --get-url` must give
 * back `url` itself; each within its time, its whole process tree killed when the time passes. On
 * Windows the TLS backend is schannel, so the system's certificate store decides what https trusts.
 * Returns { main } or { refused } with why. Only the seam's policy admits a URL that is not https.
 */
function confirm(ctx, url) {
  if (typeof url !== "string" || (!ctx.allowLocal && !url.startsWith("https://"))) return { refused: `the remote's URL ${JSON.stringify(url)} is not https` };
  const bin = binOf(ctx);
  if (!bin) return { refused: `no git is found at ${gitLocations().join(" or ")}` };
  const empty = fs.mkdtempSync(path.join(os.tmpdir(), "mente-gates-ls-"));
  try {
    const env = { ...gateEnv(), GIT_CEILING_DIRECTORIES: path.dirname(empty) };
    const tls = WINDOWS ? ["-c", "http.sslBackend=schannel"] : [];
    const asked = inTime([bin, "--no-replace-objects", ...tls, "ls-remote", "--get-url", "--", url], ctx.getUrlMs ?? GET_URL_MS, { env, cwd: empty });
    if (asked.timedOut || asked.code !== 0) return { refused: `git could not say which URL ${url} is: ${asked.timedOut ? "it took too long" : asked.err.trim().split(/\r?\n/)[0]}` };
    if (asked.out.trim() !== url) return { refused: `asking ${url} would ask ${asked.out.trim()} instead` };
    const remoteMs = ctx.remoteMs ?? REMOTE_MS;
    const r = inTime([bin, "--no-replace-objects", ...tls, "ls-remote", "--", url, "refs/heads/main"], remoteMs, { env, cwd: empty });
    if (r.timedOut) return { refused: `${url} did not say which commit is its main within ${remoteMs / 1000} s` };
    if (r.code !== 0) return { refused: `${url} could not be asked for its main: ${r.err.trim().split(/\r?\n/)[0]}` };
    const line = r.out.split(/\r?\n/).find((l) => /\trefs\/heads\/main$/.test(l));
    if (!line || !COMMIT.test(line.split("\t")[0])) return { refused: `${url} has no main` };
    return { main: line.split("\t")[0] };
  } finally {
    fs.rmSync(empty, { recursive: true, force: true });
  }
}

/// Whether the repository holds `commit`.
function hasCommit(ctx, commit) {
  try {
    git(ctx.common, ["cat-file", "-e", `${commit}^{commit}`], { bin: binOf(ctx) });
    return true;
  } catch {
    return false;
  }
}

/// How long the copy of `current` has lagged the local origin/main, as words, or "".
function lagWords(common, current) {
  const since = current ? laggingSince(common, current) : null;
  return since ? `; ${MAIN_REF} has named another commit since ${since.toISOString()}` : "";
}

/// Replaces each entry that does not hold its file of the set `files`, in ENTRIES' order, the
/// hook's first, each by a new file renamed over it; `bytesOf(file)` reads a file, from a copy or
/// from git's objects, and `step` names the kill points the seam gives. With `absentOnly`, as
/// before the remote has confirmed a commit, it writes only an entry that does not stand. Returns
/// the entries Windows refused to replace, in use.
function writeEntries(ctx, files, bytesOf, step = "entry", { absentOnly = false } = {}) {
  const refused = [];
  for (const name of ENTRIES) {
    const rel = `tools/gate-entries/${name}`;
    const file = files.find((f) => f.path === rel);
    if (!file) throw new Error(`the set holds no ${rel}`);
    const bytes = bytesOf(file);
    const to = path.join(ctx.gates, "entry", ...name.split("/"));
    if (!(absentOnly ? fs.existsSync(to) : entryHolds(to, bytes, file.mode))) {
      fs.mkdirSync(path.dirname(to), { recursive: true });
      try {
        replaceFile(to, bytes, file.mode === "100755" ? 0o755 : 0o644);
      } catch (e) {
        if (!["EPERM", "EBUSY", "EACCES"].includes(e.code)) throw e;
        refused.push(name);
      }
    }
    ctx.stopSync?.(`${step}:${name}`);
  }
  return refused;
}

/**
 * The refresh the refresher of the copy in use makes. When the local origin/main names `current`,
 * it asks no remote and only restores the current copy's entries. Otherwise, unless an answer is
 * remembered, it asks the remote, and readies only a commit the remote names and the local
 * origin/main names too, handing it to that commit's own refresher.
 */
async function refreshFrom(ctx) {
  const { common, gates, url } = ctx;
  const current = readCurrent(gates);
  if (!binOf(ctx)) {
    ctx.say(`no git is found at ${gitLocations().join(" or ")}, so the current copy stays${lagWords(common, current)}. Add the location to gitLocations in tools/gate-entries/verify.mjs, then run the bootstrap:\n\n${bootstrapLine(url)}\n`);
    return 0;
  }
  const local = localMain(common);
  if (!local) {
    ctx.say(`this repository has no ${MAIN_REF}, so the current copy stays; fetch first`);
    return 0;
  }
  if (local === current) return restoreEntries(ctx, current);
  const lag = lagWords(common, current);
  const kept = remembered(gates, local);
  if (kept) {
    ctx.say(`the current copy stays, as it did at the last ask for ${local.slice(0, 12)}: ${kept}${lag}`);
    return 0;
  }
  // An answer that keeps the current copy is remembered for REMEMBER_MS; with no copy current, for
  // RETRY_MS only, so that a transient failure, the network or a file in use, holds a half install
  // unfinished no longer, while a remote that does not answer still costs few starts.
  const ms = current ? REMEMBER_MS : RETRY_MS;
  const keep = (why) => {
    remember(gates, local, why, ms);
    ctx.say(`the current copy${current ? ` of ${current.slice(0, 12)}` : ""} stays: ${why}${lag}`);
    return 0;
  };
  const asked = confirm(ctx, url);
  if (asked.refused) return keep(asked.refused);
  if (asked.main !== local) {
    if (!hasCommit(ctx, asked.main)) return keep(`the remote's main (${asked.main.slice(0, 12)}) is not in this repository; fetch first`);
    return keep(`${MAIN_REF} names ${local.slice(0, 12)}, and the remote's main is ${asked.main.slice(0, 12)}`);
  }
  const lock = await takeLock(ctx.lockFile, { waitMs: ctx.waitMs, label: `a refresh to ${local.slice(0, 12)}` });
  if (!lock) {
    ctx.say(`a refresh holds the lock (${lockHolder(ctx.lockFile)}), so this start writes nothing and runs the current copy`);
    return 0;
  }
  try {
    await ctx.stop?.("locked");
    if (readCurrent(gates) === local) return 0;
    let dir;
    try {
      dir = ready(ctx, local);
    } catch (e) {
      return keep(`the set of ${local.slice(0, 12)} cannot be installed: ${e.message}`);
    }
    await ctx.stop?.("ready");
    const status = await ctx.startNew(dir, lock);
    // A new main whose install fails is not tried again at every start: its answer is remembered
    // for that local commit, as one that keeps the copy, and its staging is swept by the next.
    if (status !== 0 && readCurrent(gates) !== local) remember(gates, local, `the install of ${local.slice(0, 12)} failed`, ms);
    return status;
  } finally {
    lock.release();
  }
}

/// Restores the entries of the current copy, under the lock, where any differs from that copy's
/// tools/gate-entries/: the refresh a stale or edited entry names.
async function restoreEntries(ctx, current) {
  // The copy and its entries as they stand, read whole: first to see whether anything differs,
  // then again under the lock, where another refresh may have switched `current` since.
  const read = (commit) => {
    const copy = path.join(ctx.gates, commit);
    try {
      const files = readManifest(fs.readFileSync(path.join(copy, "manifest"), "utf8"));
      return mismatch(copy, files) === null ? { copy, files } : null;
    } catch {
      return null;
    }
  };
  const seen = read(current);
  if (!seen || entriesHold(ctx.gates, seen.copy, seen.files)) return 0;
  const lock = await takeLock(ctx.lockFile, { waitMs: ctx.waitMs, label: `entries of ${current.slice(0, 12)}` });
  if (!lock) return 0;
  try {
    const now = readCurrent(ctx.gates);
    const held = now ? read(now) : null;
    if (!held) return 0;
    const { copy, files } = held;
    const refused = writeEntries(ctx, files, (f) => fs.readFileSync(path.join(copy, ...f.path.split("/"))));
    ctx.say(refused.length > 0 ? `Windows refused to restore the entries ${refused.join(", ")}, in use; the next start tries again` : `restored the entries of the copy of ${now.slice(0, 12)}`);
    return 0;
  } finally {
    lock.release();
  }
}

/// The directory of `commit`'s set, ready for its refresher: an existing copy of it that holds its
/// files and manifest, or a staging directory written from its objects and checked against its
/// tree; an existing copy that does not hold them is moved aside.
function ready(ctx, commit) {
  // Under the lock no other refresh stages anything, so the staging directories and copies moved
  // aside that earlier refreshes left, a failed install's among them, go first.
  for (const name of fs.readdirSync(ctx.gates)) {
    if (!STAGING.test(name) && !ASIDE.test(name)) continue;
    try {
      fs.rmSync(path.join(ctx.gates, name), { recursive: true });
    } catch {
      // In use: a later refresh removes it.
    }
  }
  const { files } = setOf(ctx, commit);
  const target = path.join(ctx.gates, commit);
  if (fs.existsSync(target)) {
    if (holds(target, files)) return target;
    moveAside(ctx.gates, target, commit);
  }
  const staging = path.join(ctx.gates, `staging-${commit}-${unique()}`);
  writeSet(ctx, files, staging);
  const wrong = mismatch(staging, files, []);
  if (wrong) throw new Error(`the staged set of ${commit.slice(0, 12)} does not match its tree: ${wrong}`);
  return staging;
}

/**
 * The install the new set's refresher makes, the lock held, from `dir` (its own directory in the
 * command; the seam's argument in the tests): it writes only when `dir` holds exactly the set of
 * the commit its name names, and the remote, asked at the URL its own set declares, and the local
 * origin/main name that commit. Then the manifest, the rename to the commit's name, the entries,
 * and `current` last, once every entry is replaced.
 */
async function installAt(ctx, dir) {
  const commit = commitOf(path.basename(dir));
  const nothing = (why) => {
    ctx.say(`the refresher of ${commit.slice(0, 12)} writes nothing: ${why}`);
    return 1;
  };
  let files;
  try {
    files = setOf(ctx, commit).files;
  } catch (e) {
    return nothing(e.message);
  }
  const wrong = mismatch(dir, files);
  if (wrong) return nothing(`its directory does not hold exactly the commit's set: ${wrong}`);
  if (!ctx.seam && path.resolve(HERE, "..") !== path.resolve(dir)) return nothing(`it does not run from ${dir}`);
  const asked = confirm(ctx, declaredUrl(dir));
  if (asked.refused) return nothing(asked.refused);
  const local = localMain(ctx.common);
  if (asked.main !== commit || local !== commit) return nothing(`the remote's main is ${asked.main.slice(0, 12)}, and ${MAIN_REF} ${local ? local.slice(0, 12) : "names nothing"}`);
  replaceFile(path.join(dir, "manifest"), manifestText(files));
  await ctx.stop?.("manifest");
  const target = path.join(ctx.gates, commit);
  let installed = dir;
  if (path.resolve(dir) !== path.resolve(target)) {
    if (fs.existsSync(target) && holds(target, files)) installed = target;
    else {
      if (fs.existsSync(target)) moveAside(ctx.gates, target, commit);
      fs.renameSync(dir, target);
      installed = target;
    }
  }
  await ctx.stop?.("rename");
  const refused = writeEntries(ctx, files, (f) => fs.readFileSync(path.join(installed, ...f.path.split("/"))));
  if (refused.length > 0) {
    ctx.say(`the copy of ${commit.slice(0, 12)} is installed but not yet current: Windows refused to replace the entries ${refused.join(", ")}, in use; the next start of quick, cargo or mn tries again`);
    return 0;
  }
  const previous = readCurrent(ctx.gates);
  replaceFile(path.join(ctx.gates, "current"), `${commit}\n`);
  await ctx.stop?.("current");
  sweep(ctx.gates, [commit, previous].filter(Boolean));
  fs.rmSync(path.join(ctx.gates, "kept"), { force: true });
  ctx.say(`installed main's gates at ${commit.slice(0, 12)}`);
  return 0;
}

/// A staged refresher started with no lock handed down: the refresh entry finishing a set an
/// install left staged, since the bootstrap hands its lock down. It takes the lock itself, waiting
/// at most as long as a refresh does, so that a start is held no longer for it, then installs; when
/// the time runs out it gives up, writes nothing, and names the holder.
async function stagedFrom(ctx, dir) {
  const commit = commitOf(path.basename(dir));
  const kept = remembered(ctx.gates, commit);
  if (kept) {
    ctx.say(`the staged set of ${commit.slice(0, 12)} waits, as it did at the last start: ${kept}`);
    return 1;
  }
  const lock = await takeLock(ctx.lockFile, { waitMs: ctx.waitMs, label: `the staged set of ${commitOf(path.basename(dir)).slice(0, 12)}` });
  if (!lock) {
    ctx.say(`the staged set waits for the next start and writes nothing: a refresh holds the lock (${lockHolder(ctx.lockFile)})`);
    return 1;
  }
  try {
    await ctx.stop?.("locked");
    const status = await installAt(ctx, dir);
    if (status !== 0 && !readCurrent(ctx.gates)) remember(ctx.gates, commit, `the install of the staged set of ${commit.slice(0, 12)} failed`, RETRY_MS);
    return status;
  } finally {
    lock.release();
  }
}

/// The context of the command, from where its copy lies.
function commandContext() {
  const copy = path.resolve(HERE, "..");
  const gates = path.dirname(copy);
  const common = path.dirname(gates);
  const say = (line) => console.error(`mente-gates: ${line}`);
  const ctx = { copy, gates, common, lockFile: path.join(gates, "lock"), say, allowLocal: false, seam: false, waitMs: WAIT_MS };
  ctx.startNew = (dir, lock) =>
    new Promise((resolve) => {
      const child = spawn(process.execPath, [path.join(dir, "tools", "gates.mjs")], { ...handDown(lock, ["ignore", "inherit", "inherit"], gateEnv()), windowsHide: true });
      child.on("exit", (code) => resolve(code ?? 1));
      child.on("error", (e) => {
        say(`the new refresher in ${dir} did not start: ${e.message}`);
        resolve(1);
      });
    });
  return ctx;
}

/// The work of a refresh, in the child process the command starts under its time limit.
export async function work() {
  const ctx = commandContext();
  if (STAGING.test(path.basename(ctx.copy))) return stagedFrom(ctx, ctx.copy);
  return refreshFrom({ ...ctx, url: declaredUrl(ctx.copy) });
}

/// The command: refuses any argument and any place but a copy under mente-gates/; with a lock
/// handed down, installs its own directory; otherwise runs the work in a child of its own, under a
/// refresh's time limit, staged set or not, its whole tree killed when the time passes.
async function command() {
  const say = (line) => console.error(`mente-gates: ${line}`);
  if (!nodeMeetsFloor(process.versions.node)) {
    say(`tools/gates.mjs needs node ${NODE_ENGINES}, not ${process.versions.node}`);
    return 2;
  }
  if (process.argv.length > 2) {
    say(`tools/gates.mjs takes no argument, and refuses ${JSON.stringify(process.argv.slice(2))}`);
    return 2;
  }
  const ctx = commandContext();
  const name = path.basename(ctx.copy);
  if (path.basename(ctx.gates) !== "mente-gates" || commitOf(name) === null || !fs.existsSync(path.join(ctx.common, "objects")) || !fs.existsSync(path.join(ctx.common, "HEAD"))) {
    say(`tools/gates.mjs starts only from a copy under <common git directory>/mente-gates/, never from ${ctx.copy}`);
    return 2;
  }
  if (inheritedLock(ctx.lockFile)) return installAt(ctx, ctx.copy);
  const limit = REFRESH_MS;
  const code = `import(${JSON.stringify(import.meta.url)}).then((m) => m.work()).then((code) => process.exit(code), (e) => { console.error("mente-gates: the refresh failed: " + (e && e.stack ? e.stack : e)); process.exit(1); });`;
  const r = inTime([process.execPath, "--input-type=module", "-e", code], limit, { env: gateEnv() });
  process.stdout.write(r.out);
  process.stderr.write(r.err);
  if (r.timedOut) {
    say(`the refresh took more than ${limit / 1000} s, so it was killed with its whole tree, and the current copy stays`);
    return 1;
  }
  return r.code;
}

// The seam, for the tests alone (briefs/gate-tools-from-main.md, "The seam, for the tests alone").

/// The absolute common git directory of the repository found from `cwd`, or of the git directory
/// `gitDir`; throws where there is none.
function commonDir({ cwd, gitDir }) {
  const bin = systemGit();
  if (!bin) throw new Error("no git");
  const r = spawnSync(bin, [...(gitDir ? ["--git-dir", gitDir] : []), "rev-parse", "--path-format=absolute", "--git-common-dir"], { cwd, env: gateEnv(), encoding: "utf8", windowsHide: true });
  if (r.status !== 0) throw new Error(r.stderr.trim() || "not a repository");
  return fs.realpathSync(r.stdout.trim());
}

/// Why this module honours no seam argument against `repo`, or null: a module under a mente-gates/
/// directory, or outside any repository, honours none; a working-tree module honours them only
/// against a repository whose common git directory differs from its own, every worktree of its
/// own repository, the main checkout included, counting as its own.
function seamRefusal(repo) {
  if (SELF.split(/[\\/]/).includes("mente-gates")) return "a module under mente-gates/ honours no seam argument";
  let own;
  try {
    own = commonDir({ cwd: HERE });
  } catch {
    return "a module outside any repository honours no seam argument";
  }
  let theirs;
  try {
    theirs = commonDir({ gitDir: repo });
  } catch (e) {
    return `${repo} is not a repository: ${e.message}`;
  }
  const fold = (p) => (WINDOWS ? p.toLowerCase() : p);
  if (fold(own) === fold(theirs)) return "the seam is for a repository other than the module's own, and every worktree of it counts as its own";
  return null;
}

/**
 * The seam: a refresh of the repository whose git directory is `repo`, as its current copy's
 * refresher would make it, with `policy.allowLocal` admitting the fixtures' local-path remotes,
 * and the new set installed in this process instead of by its own refresher, since the command
 * admits only https. With no copy installed, or with `bootstrap`, it installs the commit the local
 * origin/main names, as the bootstrap would. `stopAt` names a step at which this process is killed,
 * as a kill would end it (halfway, locked, early:<name>, staged, ready, manifest, rename,
 * entry:<name>, current); `hang`
 * makes it hang there. `remoteMs` and `getUrlMs` shorten the remote's times, and `noGit` stands for
 * a machine where no git of the list is found. Honoured only as seamRefusal says; resolves to the
 * status the command would end with.
 */
export async function refresh(options = {}) {
  const { repo, policy = {}, stopAt = null, hang = null, bootstrap = false, say = (line) => console.error(`mente-gates: ${line}`) } = options;
  const why = seamRefusal(repo);
  if (why) throw new Error(why);
  const common = commonDir({ gitDir: repo });
  const gates = path.join(common, "mente-gates");
  fs.mkdirSync(gates, { recursive: true });
  const ctx = {
    gates,
    common,
    lockFile: path.join(gates, "lock"),
    say,
    allowLocal: Boolean(policy.allowLocal),
    seam: true,
    waitMs: options.waitMs ?? WAIT_MS,
    bootstrapWaitMs: options.bootstrapWaitMs ?? REFRESH_MS,
    remoteMs: options.remoteMs,
    getUrlMs: options.getUrlMs,
    noGit: Boolean(options.noGit),
  };
  const kill = () => process.kill(process.pid, "SIGKILL");
  ctx.stopSync = (step) => {
    if (step === stopAt) kill();
  };
  ctx.stop = (step) => {
    if (step === hang) {
      setInterval(() => {}, 1000);
      return new Promise(() => {});
    }
    ctx.stopSync(step);
    return undefined;
  };
  if (stopAt === "halfway") {
    let written = 0;
    ctx.halfway = () => {
      written += 1;
      if (written === 2) kill();
    };
  }
  ctx.startNew = (dir) => installAt(ctx, dir);
  const current = readCurrent(gates);
  if (current && !bootstrap) return refreshFrom({ ...ctx, url: declaredUrl(path.join(gates, current)) });
  const local = localMain(common);
  if (!local) {
    say(`this repository has no ${MAIN_REF}`);
    return 1;
  }
  // As the bootstrap does: the lock first, then, while no copy is current, the entries that do not
  // stand, the hook's first, from the commit's objects, then the staged set, installed with the
  // lock held, and removed if its install fails.
  const lock = await takeLock(ctx.lockFile, { waitMs: ctx.bootstrapWaitMs, label: "the bootstrap" });
  if (!lock) {
    say(`the bootstrap gives up and writes nothing: a refresh holds the lock (${lockHolder(ctx.lockFile)})`);
    return 1;
  }
  try {
    await ctx.stop?.("locked");
    const files = setOf(ctx, local).files;
    if (!current) writeEntries(ctx, files, (f) => git(common, ["cat-file", "blob", f.id], { encoding: "buffer", bin: binOf(ctx) }), "early", { absentOnly: true });
    const staging = path.join(gates, `staging-${local}-${unique()}`);
    writeSet(ctx, files, staging);
    await ctx.stop?.("staged");
    const status = await installAt(ctx, staging);
    if (status !== 0) fs.rmSync(staging, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
    return status;
  } finally {
    lock.release();
  }
}

// The command runs only when this file is node's entry point, never when the tests or the
// refresh's own worker import it: import.meta.main says so without comparing paths. Every process
// that starts the command (the refresh entry, the bootstrap, a refresher handing over) first
// refuses a node outside NODE_ENGINES, which may have no such field, rather than skip it.
if (import.meta.main) {
  command().then(
    (code) => process.exit(code),
    (e) => {
      console.error(`mente-gates: the refresh failed: ${e?.stack ?? e}`);
      process.exit(1);
    },
  );
}
