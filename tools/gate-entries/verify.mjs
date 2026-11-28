// The facts the gates' stable entries and their refresher share (briefs/gate-tools-from-main.md),
// defined once: how their processes run, where git is, a blob's id, a copy's manifest, the local
// origin/main read as files, and the check every entry makes of the copy `current` names before it
// imports or runs anything of it.
//
// It imports nothing but node's own modules: in <common git directory>/mente-gates/entry/ it stands
// beside the other entries alone, and they import it before any file of a copy is trusted. The
// refresher (tools/gates.mjs) imports it from its copy, where it is tools/gate-entries/verify.mjs.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const WINDOWS = process.platform === "win32";

/// The version of the interface between the hook's and the runner's entries and a copy: what an
/// entry imports or runs of a copy, and how. A copy whose tools/gate-set.json declares another
/// version is newer or older than the entries, which then report themselves as stale. The refresh
/// entry is exempt: its interface (the manifest's format, and running the copy's tools/gates.mjs
/// with no argument) is fixed, and a change to it needs a new bootstrap.
export const INTERFACE = 1;

/// The versions of node this repository declares (package.json, engines) and the refresher
/// requires, which every process that starts the refresher checks first: those with import.meta.main, by which it runs its command only as node's entry point,
/// which came in node 24.2.0 and was carried into 22.18.0, but never into the 23 line.
export const NODE_ENGINES = "^22.18.0 || >=24.2.0";

/// Whether node `version` (process.versions.node) is one of NODE_ENGINES.
export function nodeMeetsFloor(version) {
  const [major, minor] = String(version).split(".").map(Number);
  return (major === 22 && minor >= 18) || (major === 24 && minor >= 2) || major > 24;
}

/// Whether `dir` lies in a checkout of this repository: it or a parent holds Cargo.toml and
/// tools/capped.ps1. Cargo's runner entry decides `--mente-only` by it before it checks any copy,
/// since the runner line serves every Rust project of the machine.
export function inMente(dir) {
  for (let at = path.resolve(dir); ; at = path.dirname(at)) {
    if (fs.existsSync(path.join(at, "Cargo.toml")) && fs.existsSync(path.join(at, "tools", "capped.ps1"))) return true;
    if (path.dirname(at) === at) return false;
  }
}

/// The ref the copy's commit must be, or an ancestor of, and that the refresher compares with
/// `current` before it asks the remote anything.
export const MAIN_REF = "refs/remotes/origin/main";

/// A commit's full id.
export const COMMIT = /^[0-9a-f]{40}$/;

/// A staging directory of a commit's set, which no entry runs.
export const STAGING = /^staging-([0-9a-f]{40})-[0-9a-f]+$/;

/// The commit a copy's directory name stands for: the commit itself, or a staging directory of it.
export const commitOf = (name) => (COMMIT.test(name) ? name : (STAGING.exec(name)?.[1] ?? null));

/// The stable entries, written from a copy's tools/gate-entries/ into mente-gates/entry/, in this
/// order: the hook's first, since git runs no hook it cannot find and a push would go through, and
/// it refuses until the files beside it are written; then what it and the others import.
export const ENTRIES = ["hooks/pre-push", "verify.mjs", "hook.mjs", "refresh.mjs", "capped.mjs"];

/// Variables removed from every process the refresher starts and every git command of the entries'
/// checks: git's own, node's (NODE_OPTIONS preloads code), and those that name certificate files.
const DROPPED = [/^GIT_/i, /^NODE_/i, /^SSL_CERT_FILE$/i, /^SSL_CERT_DIR$/i, /^CURL_CA_BUNDLE$/i];

/**
 * The environment of the gates' processes: `env` with DROPPED removed, and git's system and user
 * configuration closed, so that no setting of either (an insteadOf, an sslVerify) reaches git,
 * wherever HOME or XDG_CONFIG_HOME points; GIT_CONFIG_GLOBAL is /dev/null, which Git for Windows
 * accepts when node starts it, so no file is left for a command to fill. No prompt for credentials.
 */
export function gateEnv(env = process.env) {
  const kept = Object.fromEntries(Object.entries(env).filter(([key]) => !DROPPED.some((drop) => drop.test(key))));
  return { ...kept, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null", GIT_TERMINAL_PROMPT: "0" };
}

/// Where the gates look for git, in order, by absolute path, never on PATH: the system's, then on
/// Windows Git for Windows' per-user install (through LOCALAPPDATA, a limit the brief names). A
/// location missing here is added here, then installed by the bootstrap, since the installed
/// refresher keeps the list it was installed with.
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
 * Runs `bin`, the system's git unless given, on the repository `gitDir`, with the gates'
 * environment, git's replace refs ignored and pathspecs read literally: its standard output, a
 * string or with `encoding` "buffer" the bytes, or throws with git's words and its status.
 */
export function git(gitDir, args, { encoding = "utf8", cwd, bin = systemGit() } = {}) {
  if (!bin) throw Object.assign(new Error(`no git is found at ${gitLocations().join(" or ")}`), { code: "NOGIT" });
  const r = spawnSync(bin, ["--git-dir", gitDir, "--no-replace-objects", "--literal-pathspecs", ...args], { env: gateEnv(), encoding: encoding === "buffer" ? "buffer" : encoding, cwd, maxBuffer: 1 << 28, windowsHide: true });
  if (r.error) throw r.error;
  if (r.status !== 0) throw Object.assign(new Error(`git ${args[0]} failed: ${String(r.stderr).trim()}`), { status: r.status, stderr: String(r.stderr) });
  return r.stdout;
}

/// The modes a file of the set may have: a plain blob, executable or not. A symlink (120000) or a
/// submodule (160000) cannot stand as a file.
export const FILE_MODES = new Set(["100644", "100755"]);

/// Whether `rel`, a path of the set, is one: relative, forward slashes, no empty, `.` or `..` part.
export const setPath = (rel) => typeof rel === "string" && rel !== "" && !rel.startsWith("/") && !/\\/.test(rel) && rel.split("/").every((part) => part !== "" && part !== "." && part !== "..") && !/^[A-Za-z]:/.test(rel);

/**
 * The files of `listing`, the output of `git ls-tree -r -z --full-tree <commit> -- <set>`, as
 * [{ path, mode, id }] sorted by path, which must be exactly `set`, each a blob of a mode of
 * FILE_MODES; throws, naming the first that is not.
 */
export function filesOf(listing, set) {
  const files = [];
  for (const entry of String(listing).split("\0").filter((e) => e !== "")) {
    const m = /^(\d{6}) (\w+) ([0-9a-f]{40})\t(.+)$/s.exec(entry);
    if (!m) throw new Error(`git ls-tree gave a line the gates cannot read: ${JSON.stringify(entry)}`);
    const [, mode, type, id, file] = m;
    if (type !== "blob" || !FILE_MODES.has(mode)) throw new Error(`${file} is a ${type} of mode ${mode}, not a file of mode 100644 or 100755, so it cannot stand in the set`);
    files.push({ path: file, mode, id });
  }
  const listed = files.map((f) => f.path).sort();
  const wanted = [...set].sort();
  if (listed.length !== wanted.length || listed.some((p, i) => p !== wanted[i])) {
    const missing = wanted.filter((p) => !listed.includes(p));
    const extra = listed.filter((p) => !wanted.includes(p));
    throw new Error(`the commit's tree does not hold exactly the set: ${missing.length ? `missing ${missing.join(", ")}` : ""}${missing.length && extra.length ? "; " : ""}${extra.length ? `beyond it ${extra.join(", ")}` : ""}`);
  }
  return files.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
}

/// A manifest's text: each file's mode, blob id and path, a line each, sorted by path.
export const manifestText = (files) => files.map((f) => `${f.mode} ${f.id}\t${f.path}\n`).join("");

/// The files a manifest's text names, as manifestText writes them; throws on a line it does not.
export function readManifest(text) {
  return String(text)
    .split("\n")
    .filter((line) => line !== "")
    .map((line) => {
      const m = /^(\d{6}) ([0-9a-f]{40})\t(.+)$/.exec(line);
      if (!m || !FILE_MODES.has(m[1]) || !setPath(m[3])) throw new Error(`the manifest holds a line that is not a file of the set: ${JSON.stringify(line)}`);
      return { mode: m[1], id: m[2], path: m[3] };
    });
}

/// Every file under `dir`, as paths relative to it with forward slashes.
export function filesUnder(dir) {
  const out = [];
  const walk = (at, rel) => {
    for (const entry of fs.readdirSync(at, { withFileTypes: true })) {
      const next = rel ? `${rel}/${entry.name}` : entry.name;
      if (entry.isDirectory()) walk(path.join(at, entry.name), next);
      else out.push(next);
    }
  };
  walk(dir, "");
  return out.sort();
}

/**
 * Why `dir` does not hold exactly `files`, or null when it does: each file with its blob id and,
 * off Windows, its executable bit as its mode says; no file beyond them but `allowed`.
 */
export function mismatch(dir, files, allowed = ["manifest"]) {
  for (const f of files) {
    const at = path.join(dir, ...f.path.split("/"));
    let bytes;
    try {
      bytes = fs.readFileSync(at);
    } catch {
      return `${f.path} is missing`;
    }
    if (blobId(bytes) !== f.id) return `${f.path} is not the blob ${f.id.slice(0, 12)} the commit holds`;
    if (!WINDOWS && Boolean(fs.statSync(at).mode & 0o100) !== (f.mode === "100755")) return `${f.path} is ${f.mode === "100755" ? "not executable, though its mode is 100755" : "executable, though its mode is 100644"}`;
  }
  const known = new Set([...files.map((f) => f.path), ...allowed]);
  const extra = filesUnder(dir).find((rel) => !known.has(rel));
  return extra ? `${extra} is not a file of the set` : null;
}

/// The commit the local origin/main names, read as files, which needs no git: its loose file, or
/// packed-refs when the loose file is gone (git gc and pack-refs can leave a ref only there); null
/// when neither names it.
export function localMain(common) {
  try {
    const text = fs.readFileSync(path.join(common, ...MAIN_REF.split("/")), "utf8").trim();
    if (COMMIT.test(text)) return text;
  } catch {
    // Packed, or absent.
  }
  try {
    for (const line of fs.readFileSync(path.join(common, "packed-refs"), "utf8").split(/\r?\n/)) {
      const [id, ref] = line.split(" ");
      if (ref === MAIN_REF && COMMIT.test(id)) return id;
    }
  } catch {
    // No packed refs.
  }
  return null;
}

/// Since when the local origin/main has named a commit other than `commit`, from its reflog, read
/// as a file (logs/refs/remotes/origin/main, which packing leaves in place): the time of the first
/// entry after the last that named `commit`, or null when the last names it or there is no reflog.
export function laggingSince(common, commit) {
  let lines;
  try {
    lines = fs.readFileSync(path.join(common, "logs", ...MAIN_REF.split("/")), "utf8").split(/\r?\n/);
  } catch {
    return null;
  }
  let since = null;
  for (const line of lines) {
    const m = /^[0-9a-f]{40} ([0-9a-f]{40}) .*> (\d+) [+-]\d{4}\t?/.exec(line);
    if (!m) continue;
    if (m[1] === commit) since = null;
    else if (since === null) since = new Date(Number(m[2]) * 1000);
  }
  return since;
}

/// The command a session runs to refresh the copy: any start of these tools refreshes first.
export const REFRESH = "node tools/quick.mjs (or node tools/cargo.mjs, or node tools/mn.mjs)";

/**
 * Norman's bootstrap, the one line that installs a copy when none is, or the current one is bad,
 * from the main checkout: the remote at `url` names main's commit, and that commit's
 * tools/gate-entries/bootstrap.mjs, read from its objects, writes its set into a staging directory
 * and starts the staged refresher, which confirms the commit itself. CLAUDE.md holds this line.
 */
export const bootstrapLine = (url) => `$c = (git ls-remote ${url} refs/heads/main).Split()[0]; git --no-replace-objects cat-file blob "\${c}:tools/gate-entries/bootstrap.mjs" | node --input-type=module - $c`;

/// The URL a copy's tools/gate-set.json declares, read where the bootstrap line is printed.
export const declaredUrl = (copy) => JSON.parse(fs.readFileSync(path.join(copy, "tools", "gate-set.json"), "utf8")).url;

/**
 * The check every entry makes before it imports or runs anything of a copy: the copy `current`
 * names in `gates` (<common git directory>/mente-gates), held to its manifest by blob ids and
 * executable bits, and, with git, the manifest to `git ls-tree` of the commit and the commit to the
 * local origin/main or one of its ancestors; with no git, the manifest alone, and it says so. Unless
 * `exempt` (the refresh entry), the copy's interface version must be INTERFACE.
 *
 * Returns { copy, commit, notes, files } to run it, or { refusal, command } with the one command
 * that clears the refusal: the bootstrap for no copy or a bad one, the refresh for a stale entry;
 * `missing` says that no copy is current at all. `bin` is the system's git unless given; null
 * stands for a machine where none of the list is found.
 */
export function verifyCopy(gates, options = {}) {
  let commit;
  try {
    commit = fs.readFileSync(path.join(gates, "current"), "utf8").trim();
  } catch {
    // With `pending`, an install a kill or a refused rename left half done is named, since the
    // next start's refresh entry finishes it, with no bootstrap: the hook's and the runner's words.
    const bin = options.bin ?? systemGit();
    const pending = options.pending ? pendingCopy(gates, { bin }) : null;
    // A start of quick, cargo or mn finishes it only where Norman's lines name the copy; otherwise
    // the bootstrap does.
    if (pending) return { refusal: `the install of main's gates at ${pending.commit.slice(0, 12)} was left half done in ${gates}`, command: hooksNamed(gates, { bin }) ? "refresh" : "bootstrap", missing: true };
    return { refusal: `no copy of main's gates is installed in ${gates}`, command: "bootstrap", missing: true };
  }
  if (!COMMIT.test(commit)) return { refusal: `${path.join(gates, "current")} names no commit`, command: "bootstrap" };
  return verifyDir(gates, commit, options);
}

/// The check verifyCopy makes, of the copy of `commit` in `gates`, current or not.
export function verifyDir(gates, commit, { exempt = false, bin = systemGit() } = {}) {
  const common = path.dirname(gates);
  const bootstrap = (why) => ({ refusal: why, command: "bootstrap" });
  const copy = path.join(gates, commit);
  let files;
  try {
    files = readManifest(fs.readFileSync(path.join(copy, "manifest"), "utf8"));
  } catch (e) {
    return bootstrap(`the copy of ${commit.slice(0, 12)} has no manifest the entries can read: ${e.message}`);
  }
  const wrong = mismatch(copy, files);
  if (wrong) return bootstrap(`the copy of ${commit.slice(0, 12)} has drifted from its manifest: ${wrong}`);
  const notes = [];
  let declared;
  try {
    declared = JSON.parse(fs.readFileSync(path.join(copy, "tools", "gate-set.json"), "utf8"));
  } catch (e) {
    return bootstrap(`the copy of ${commit.slice(0, 12)} has no tools/gate-set.json the entries can read: ${e.message}`);
  }
  if (bin) {
    try {
      const tree = filesOf(git(common, ["ls-tree", "-r", "-z", "--full-tree", commit, "--", ...declared.set], { bin }), declared.set);
      if (manifestText(tree) !== manifestText(files)) return bootstrap(`the manifest of the copy of ${commit.slice(0, 12)} disagrees with that commit's tree`);
    } catch (e) {
      return bootstrap(`the copy of ${commit.slice(0, 12)} cannot be held to its commit: ${e.message}`);
    }
    const main = localMain(common);
    if (!main) return { refusal: `this repository has no ${MAIN_REF}, which the copy's commit must be or precede`, command: "fetch" };
    let ancestor = false;
    try {
      git(common, ["merge-base", "--is-ancestor", commit, main], { bin });
      ancestor = true;
    } catch (e) {
      if (e.status !== 1) return bootstrap(`whether the copy's commit ${commit.slice(0, 12)} precedes ${MAIN_REF} is unknown: ${e.message}`);
    }
    if (!ancestor) return bootstrap(`the copy's commit ${commit.slice(0, 12)} is not ${MAIN_REF} or one of its ancestors`);
    if (main !== commit) {
      const since = laggingSince(common, commit);
      notes.push(`the gates run the verified copy of ${commit.slice(0, 12)}, behind ${MAIN_REF} (${main.slice(0, 12)})${since ? ` since ${since.toISOString()}` : ""}; ${REFRESH} refreshes it`);
    }
  } else {
    const main = localMain(common);
    const since = main && main !== commit ? laggingSince(common, commit) : null;
    notes.push(`no git is found at ${gitLocations().join(" or ")}, so the copy of ${commit.slice(0, 12)} is held to its manifest alone${since ? `, and has lagged ${MAIN_REF} since ${since.toISOString()}` : ""}; add the location to gitLocations in tools/gate-entries/verify.mjs, then run the bootstrap`);
  }
  if (!exempt && declared.interface !== INTERFACE) return { refusal: `this entry speaks version ${INTERFACE} of the interface with a copy, and the copy of ${commit.slice(0, 12)} version ${declared.interface}: the entry is stale`, command: "refresh" };
  return { copy, commit, notes, files };
}

/// Whether the entry `to` holds `bytes` and, off Windows, the executable bit of `mode`.
export function entryHolds(to, bytes, mode) {
  try {
    return fs.readFileSync(to).equals(bytes) && (WINDOWS || Boolean(fs.statSync(to).mode & 0o100) === (mode === "100755"));
  } catch {
    return false;
  }
}

/// Whether every entry of `gates` holds the file of `copy`'s tools/gate-entries/ that `files`,
/// its manifest, names, with its mode.
export function entriesHold(gates, copy, files) {
  return ENTRIES.every((name) => {
    const file = files.find((f) => f.path === `tools/gate-entries/${name}`);
    if (!file) return false;
    try {
      return entryHolds(path.join(gates, "entry", ...name.split("/")), fs.readFileSync(path.join(copy, "tools", "gate-entries", ...name.split("/"))), file.mode);
    } catch {
      return false;
    }
  });
}

/// Whether a refresh would do nothing, the refresh entry's fast path: the local origin/main names
/// the verified copy `checked`, so no remote is asked, and every entry holds that copy's file, so
/// none is restored.
export function nothingToRefresh(gates, checked) {
  return localMain(path.dirname(gates)) === checked.commit && entriesHold(gates, checked.copy, checked.files);
}

/**
 * The copy an install left in `gates` but not yet current, for the refresh entry to finish it: a
 * directory named by a commit that verifyDir accepts, which holds it to origin/main, or a staging
 * directory of the local origin/main's own commit whose files are exactly its tree's, never a
 * branch's set a hand run left there; null when there is none, or no git.
 */
export function pendingCopy(gates, { bin = systemGit() } = {}) {
  if (!bin) return null;
  const common = path.dirname(gates);
  const main = localMain(common);
  let names = [];
  try {
    names = fs.readdirSync(gates).filter((name) => commitOf(name) !== null);
  } catch {
    return null;
  }
  names.sort((a, b) => Number(commitOf(b) === main) - Number(commitOf(a) === main));
  for (const name of names) {
    const commit = commitOf(name);
    if (COMMIT.test(name)) {
      if (!verifyDir(gates, commit, { exempt: true, bin }).refusal) return { dir: path.join(gates, name), commit };
      continue;
    }
    if (commit !== main) continue;
    try {
      const declared = JSON.parse(git(common, ["cat-file", "blob", `${commit}:tools/gate-set.json`], { bin }));
      const files = filesOf(git(common, ["ls-tree", "-r", "-z", "--full-tree", commit, "--", ...declared.set], { bin }), declared.set);
      if (mismatch(path.join(gates, name), files) === null) return { dir: path.join(gates, name), commit };
    } catch {
      // Not a staged set of that commit.
    }
  }
  return null;
}

/**
 * Whether core.hooksPath, as the repository's own configuration holds it, which is where Norman's
 * line writes it, names the stable hook entry of `gates`: the tools then require the copy, and a
 * start of quick, cargo or mn finishes an install left half done. False where it cannot be read.
 */
export function hooksNamed(gates, { bin = systemGit() } = {}) {
  try {
    const value = git(path.dirname(gates), ["config", "--get", "core.hooksPath"], { bin }).trim();
    const fold = (p) => (WINDOWS ? path.resolve(p).toLowerCase() : path.resolve(p));
    return path.isAbsolute(value) && fold(value) === fold(path.join(gates, "entry", "hooks"));
  } catch {
    return false;
  }
}

/// The URL the copies in `gates` declare, the current one first, for the bootstrap line an entry
/// prints; null when none can be read.
function urlIn(gates) {
  let names = [];
  try {
    names = fs.readdirSync(gates);
  } catch {
    return null;
  }
  try {
    names.unshift(fs.readFileSync(path.join(gates, "current"), "utf8").trim());
  } catch {
    // No copy current: the others, in turn.
  }
  for (const name of names) {
    const url = httpsUrlOf(path.join(gates, name));
    if (url) return url;
  }
  return null;
}

/// The https URL a file of the set declares, from the copy or checkout `dir`, or null.
function httpsUrlOf(dir) {
  try {
    const url = declaredUrl(dir);
    return typeof url === "string" && url.startsWith("https://") ? url : null;
  } catch {
    return null;
  }
}

/// The https URL main's own tools/gate-set.json declares, as the local origin/main holds it, which
/// the bootstrap requires its commit to be; null where it cannot be read.
function mainUrl(gates, bin) {
  try {
    const url = JSON.parse(git(path.dirname(gates), ["cat-file", "blob", `${MAIN_REF}:tools/gate-set.json`], { bin })).url;
    return typeof url === "string" && url.startsWith("https://") ? url : null;
  } catch {
    return null;
  }
}

/**
 * The bootstrap line for the repository of `gates`, the one every refusal and every warning of the
 * tools prints, with the https URL main's gates declare, taken from the most trusted source that
 * has one: a copy, the current one first; main's own tools/gate-set.json in the local origin/main;
 * the caller's `checkout`, where it runs from one. Words naming that line where none has one.
 */
export function bootstrapText(gates, { checkout = null, bin = systemGit() } = {}) {
  const url = (gates ? urlIn(gates) : null) ?? (gates && bin ? mainUrl(gates, bin) : null) ?? (checkout ? httpsUrlOf(checkout) : null);
  return url ? bootstrapLine(url) : "the bootstrap line, with the https URL main's tools/gate-set.json declares, which no copy, no object of the local origin/main and no checkout here could give";
}

/**
 * Where Norman runs the bootstrap, as every message names it: "the main checkout", with its path,
 * the directory that holds the common git directory of `gates`, read from where the gates lie and
 * never written as a literal, since the checkout may move; without the path where the common
 * directory is not a checkout's `.git`.
 */
export function mainCheckoutWords(gates) {
  const common = typeof gates === "string" && path.isAbsolute(gates) ? path.dirname(gates) : null;
  return common && path.basename(common) === ".git" ? `the main checkout, ${path.resolve(path.dirname(common))}` : "the main checkout";
}

/// What an entry says when it refuses: why, and the one command that clears it.
export function refusalWords(gates, result) {
  const how = {
    bootstrap: () => `Norman runs the bootstrap, from ${mainCheckoutWords(gates)}, in his own terminal:\n\n${bootstrapText(gates)}\n`,
    refresh: () => `run ${REFRESH}, whose start refreshes the copy and finishes an install left half done`,
    fetch: () => "fetch first (git fetch origin), then run the command again",
  }[result.command];
  return `mente-gates: ${result.refusal}; ${how()}`;
}
