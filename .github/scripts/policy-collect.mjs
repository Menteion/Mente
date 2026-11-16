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
const git = (args) => execFileSync("git", args, { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });

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

const base = policyBase();
const facts = {
  // The commit the facts are of, which the policy compares with the one checked out.
  head: git(["rev-parse", "HEAD"]).trim(),
  // The files of the repository, as git's index holds them.
  tracked: git(["ls-files", "-z"]).split("\0").filter(Boolean),
  // The milestones' tags, and ROADMAP.md as the base holds it.
  base,
  tags: git(["tag", "-l"]).split(/\r?\n/).filter(Boolean),
  baseRoadmap: git(["show", `${base}:ROADMAP.md`]),
};
fs.mkdirSync(path.join(ROOT, "target"), { recursive: true });
fs.writeFileSync(path.join(ROOT, "target", "policy-facts.json"), JSON.stringify(facts));
console.log(`the workflow policy's facts: ${facts.tracked.length} tracked files at ${facts.head.slice(0, 12)}, base ${base === "HEAD" ? "HEAD" : base.slice(0, 12)}`);
