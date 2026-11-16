// The workflow policy's collector (policy-runtime-judge in docs/design-review.md): the facts the
// policy reads from git, gathered here, so that the policy itself starts no process. It runs git
// alone, with arguments fixed here, and writes target/policy-facts.json, which
// .github/scripts/policy.test.mjs reads and refuses when they are of another commit than the one
// checked out.
//
//   node .github/scripts/policy-collect.mjs
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
/// git with every key by which a command here could start a program set on its own command line,
/// which wins over the repository's configuration: `git ls-files` runs core.fsmonitor from
/// .git/config (the review of 100e1e5).
const GIT_ISOLATION = [
  "-c", "core.fsmonitor=false",
  "-c", "core.hooksPath=/dev/null",
  "-c", "core.pager=cat",
  "-c", "core.sshCommand=",
  "-c", "core.askPass=",
  "-c", "credential.helper=",
  "-c", "diff.external=",
  "-c", "log.showSignature=false",
  "-c", "gpg.program=",
];
/// git's environment: the runner's without a variable of git's own, any of which could name another
/// repository (GIT_DIR) or more configuration (GIT_CONFIG_PARAMETERS, GIT_CONFIG_COUNT), and with no
/// system or global configuration. /dev/null names no configuration on Linux and to Git for Windows
/// alike, which refuses node's os.devNull.
const GIT_ENV = {
  ...Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.toUpperCase().startsWith("GIT_"))),
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_CONFIG_GLOBAL: "/dev/null",
};
const git = (args) => execFileSync("git", [...GIT_ISOLATION, ...args], { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024, env: GIT_ENV });

/// The base the pull request or merge group is checked against: the event's, or the merge base
/// with origin/main.
function policyBase() {
  // The event the runner describes: the step sets no environment of its own (the Rust guard), so
  // nothing in a pull request can point this elsewhere.
  const name = process.env.GITHUB_EVENT_NAME;
  if (name && process.env.GITHUB_EVENT_PATH) {
    const payload = JSON.parse(fs.readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
    const base = { pull_request: payload.pull_request?.base?.sha, merge_group: payload.merge_group?.base_sha, push: payload.before }[name];
    if (base && !/^0+$/.test(base)) return base;
  }
  try {
    return git(["merge-base", "HEAD", "origin/main"]).trim();
  } catch {
    return "HEAD";
  }
}

// The base as a full commit id, whatever form named it.
const base = git(["rev-parse", "--verify", `${policyBase()}^{commit}`]).trim();
const facts = {
  // The commit the facts are of, which the policy compares with the one checked out.
  head: git(["rev-parse", "HEAD"]).trim(),
  // The files of the repository, as git's index holds them.
  tracked: git(["ls-files", "-z"]).split("\0").filter(Boolean),
  // The milestones' tags, and ROADMAP.md as the base holds it.
  base,
  tags: git(["tag", "-l"]).split(/\r?\n/).filter(Boolean),
  // The blob as stored: cat-file runs no textconv filter, which `git show` may.
  baseRoadmap: git(["cat-file", "blob", `${base}:ROADMAP.md`]),
};
fs.mkdirSync(path.join(ROOT, "target"), { recursive: true });
fs.writeFileSync(path.join(ROOT, "target", "policy-facts.json"), JSON.stringify(facts));
console.log(`the workflow policy's facts: ${facts.tracked.length} tracked files at ${facts.head.slice(0, 12)}, base ${base.slice(0, 12)}`);
