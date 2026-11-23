// The body of Norman's bootstrap (briefs/gate-tools-from-main.md, "The bootstrap"). The bootstrap
// line of CLAUDE.md's Verification section asks the remote for main's commit, reads this file from
// that commit's objects, and runs it from standard input, in the main checkout, with the commit as
// its one argument:
//
//   node --input-type=module - <commit>
//
// It refuses a commit the local origin/main does not name, before writing anything: fetch first.
// It writes the commit's set, each file with the bytes `git cat-file blob` gives and the mode the
// tree records, first into a directory of its own outside the common git directory, which no
// refresh's sweep reaches. With that set's own modules, the commit's, it takes the lock of
// mente-gates/ before writing anything there. While no copy is current, it writes each stable entry
// that does not stand yet, the hook's first, so that wherever Norman's lines point, git's hook and
// cargo's runner refuse with a remedy rather than run nothing; it never replaces an entry, since
// nothing has confirmed the commit yet, and the install replaces them only once the remote has.
// Then it stages the set and starts the staged refresher with no argument and the lock handed
// down. That refresher confirms the commit with the remote itself, at the URL its own set declares,
// checks its whole directory against the commit's tree, and installs; whatever this writes wrong,
// it refuses, and this then removes the staged set, so that an install left half done only ever
// means a kill. Run from standard input, this imports nothing but
// node's own modules and the set it writes. It runs in Norman's own terminal, with that terminal's
// git and environment, a limit the brief names.

import { spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

const commit = process.argv[2];

function fail(why) {
  console.error(`mente-gates bootstrap: ${why}`);
  process.exit(1);
}

function git(args, encoding = "utf8") {
  const r = spawnSync("git", ["--no-replace-objects", "--literal-pathspecs", ...args], { encoding: encoding === "buffer" ? "buffer" : encoding, maxBuffer: 1 << 28 });
  if (r.error) fail(`git did not start: ${r.error.message}`);
  if (r.status !== 0) fail(`git ${args[0]} failed: ${String(r.stderr).trim()}`);
  return r.stdout;
}

if (!/^[0-9a-f]{40}$/.test(commit ?? "")) fail(`the remote named no commit (${JSON.stringify(commit)}): check the line's URL and the network, then run it again`);
const common = git(["rev-parse", "--path-format=absolute", "--git-common-dir"]).trim();
if (spawnSync("git", ["--no-replace-objects", "cat-file", "-e", `${commit}^{commit}`]).status !== 0) fail(`main's commit ${commit.slice(0, 12)} is not in this repository: fetch first (git fetch origin), then run the line again`);
const local = spawnSync("git", ["--no-replace-objects", "rev-parse", "--verify", "--quiet", "refs/remotes/origin/main"], { encoding: "utf8" }).stdout?.trim();
if (local !== commit) fail(`main's commit ${commit.slice(0, 12)} is not what refs/remotes/origin/main names (${local ? local.slice(0, 12) : "nothing"}), so the bootstrap writes nothing: fetch first (git fetch origin), then run the line again`);
const declared = JSON.parse(git(["cat-file", "blob", `${commit}:tools/gate-set.json`]));

// The set, written outside mente-gates/ first.
const outside = fs.mkdtempSync(path.join(os.tmpdir(), "mente-gates-bootstrap-"));
const modes = new Map();
for (const entry of git(["ls-tree", "-r", "-z", "--full-tree", commit, "--", ...declared.set]).split("\0").filter(Boolean)) {
  const m = /^(100644|100755) blob ([0-9a-f]{40})\t(.+)$/s.exec(entry);
  if (!m) fail(`the set of ${commit.slice(0, 12)} holds an entry that is not a file of mode 100644 or 100755: ${entry}`);
  const [, mode, id, rel] = m;
  if (rel.split("/").some((part) => part === "" || part === "." || part === "..")) fail(`the set names a path outside it: ${rel}`);
  const at = path.join(outside, ...rel.split("/"));
  fs.mkdirSync(path.dirname(at), { recursive: true });
  const bits = mode === "100755" ? 0o755 : 0o644;
  fs.writeFileSync(at, git(["cat-file", "blob", id], "buffer"), { mode: bits });
  if (process.platform !== "win32") fs.chmodSync(at, bits);
  modes.set(rel, bits);
}

// The set's own modules: node's floor, the stable entries, the lock and the refresh's time limit.
const load = (rel) => import(pathToFileURL(path.join(outside, ...rel.split("/"))).href);
const { ENTRIES, NODE_ENGINES, gateEnv, nodeMeetsFloor } = await load("tools/gate-entries/verify.mjs");
if (!nodeMeetsFloor(process.versions.node)) fail(`the refresher needs node ${NODE_ENGINES}, not ${process.versions.node}`);
const { handDown, lockHolder, takeLock } = await load("tools/gate-processes.mjs");
const { REFRESH_MS } = await load("tools/gates.mjs");

// The lock, before anything of mente-gates/ is written, waited for at most a refresh's time limit,
// by which any holder has been killed; a wait is said, since Norman runs this in his terminal.
const gates = path.join(common, "mente-gates");
fs.mkdirSync(gates, { recursive: true });
const lockFile = path.join(gates, "lock");
let lock = await takeLock(lockFile, { waitMs: 0, label: "the bootstrap" });
if (!lock) {
  console.error(`mente-gates bootstrap: a refresh holds the lock (${lockHolder(lockFile)}), so the bootstrap waits for it, at most ${REFRESH_MS / 1000} s, before it writes anything`);
  lock = await takeLock(lockFile, { waitMs: REFRESH_MS, label: "the bootstrap" });
}
if (!lock) fail(`a refresh holds the lock (${lockHolder(lockFile)}), so the bootstrap writes nothing; run the line again once it ends`);

// The stable entries that do not stand yet, the hook's first, each by a new file renamed into
// place, and only while no copy is current: an entry that stands is never replaced before the
// remote has confirmed the commit.
const current = fs.existsSync(path.join(gates, "current"));
for (const name of current ? [] : ENTRIES) {
  const rel = `tools/gate-entries/${name}`;
  const to = path.join(gates, "entry", ...name.split("/"));
  if (fs.existsSync(to)) continue;
  fs.mkdirSync(path.dirname(to), { recursive: true });
  const tmp = `${to}.tmp-${randomBytes(6).toString("hex")}`;
  fs.copyFileSync(path.join(outside, ...rel.split("/")), tmp);
  if (process.platform !== "win32") fs.chmodSync(tmp, modes.get(rel) ?? 0o644);
  fs.renameSync(tmp, to);
}

// The set, staged under the lock, and its refresher, which installs it, started with the lock.
const staging = path.join(gates, `staging-${commit}-${randomBytes(6).toString("hex")}`);
fs.cpSync(outside, staging, { recursive: true });
fs.rmSync(outside, { recursive: true, force: true });
const r = spawnSync(process.execPath, [path.join(staging, "tools", "gates.mjs")], { ...handDown(lock, ["inherit", "inherit", "inherit"], gateEnv()), timeout: REFRESH_MS, windowsHide: true });
lock.release();
// A refused or unfinished install leaves no staged set, for no start to take it for a kill's.
if (r.error || r.status !== 0) {
  fs.rmSync(staging, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  console.error("mente-gates bootstrap: the staged set is removed, since its refresher did not install it");
}
if (r.error) fail(`the staged refresher did not finish: ${r.error.message}`);
process.exit(r.status ?? 1);
