// The body of Norman's bootstrap (briefs/gate-tools-from-main.md, "The bootstrap"). The bootstrap
// line of CLAUDE.md's Verification section asks the remote for main's commit, reads this file from
// that commit's objects, and runs it from standard input, in the main checkout, with the commit as
// its one argument:
//
//   node --input-type=module - <commit>
//
// It writes the commit's set, each file with the bytes `git cat-file blob` gives and the mode the
// tree records, into a staging directory of the common git directory's mente-gates/, and starts the
// staged refresher with no argument. That refresher takes the lock, confirms the commit with the
// remote itself at the URL its own set declares, checks its whole directory against the commit's
// tree, and installs; whatever this writes wrong, it refuses. Run from standard input, this imports
// nothing but node's own modules. It runs in Norman's own terminal, with that terminal's git and
// environment, a limit the brief names, and starts the refresher without git's and node's variables.

import { spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs";
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
const declared = JSON.parse(git(["cat-file", "blob", `${commit}:tools/gate-set.json`]));
const staging = path.join(common, "mente-gates", `staging-${commit}-${randomBytes(6).toString("hex")}`);
fs.mkdirSync(staging, { recursive: true });
for (const entry of git(["ls-tree", "-r", "-z", "--full-tree", commit, "--", ...declared.set]).split("\0").filter(Boolean)) {
  const m = /^(100644|100755) blob ([0-9a-f]{40})\t(.+)$/s.exec(entry);
  if (!m) fail(`the set of ${commit.slice(0, 12)} holds an entry that is not a file of mode 100644 or 100755: ${entry}`);
  const [, mode, id, rel] = m;
  if (rel.split("/").some((part) => part === "" || part === "." || part === "..")) fail(`the set names a path outside it: ${rel}`);
  const at = path.join(staging, ...rel.split("/"));
  fs.mkdirSync(path.dirname(at), { recursive: true });
  const bits = mode === "100755" ? 0o755 : 0o644;
  fs.writeFileSync(at, git(["cat-file", "blob", id], "buffer"), { mode: bits });
  if (process.platform !== "win32") fs.chmodSync(at, bits);
}
// The staged refresher runs its command only as node's entry point, which a node outside the
// staged set's NODE_ENGINES cannot tell it.
const { NODE_ENGINES, nodeMeetsFloor } = await import(pathToFileURL(path.join(staging, "tools", "gate-entries", "verify.mjs")).href);
if (!nodeMeetsFloor(process.versions.node)) fail(`the refresher needs node ${NODE_ENGINES}, not ${process.versions.node}`);
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(GIT_|NODE_)/i.test(key)));
const r = spawnSync(process.execPath, [path.join(staging, "tools", "gates.mjs")], { stdio: "inherit", env });
process.exit(r.status ?? 1);
