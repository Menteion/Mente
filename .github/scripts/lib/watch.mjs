// Whether an instrument has anything to check in this run: the first step of every instrument
// workflow after its checkout (docs/design-review.md, guard-per-problem). The workflows run on
// every pull request and merge group, so that each can be a required check of `main`, and this
// step ends their work at once, successfully, when no path the instrument watches changed.
//
//   node .github/scripts/watch.mjs <workflow file name>     (the CLI, ../watch.mjs)
//
// It reads the instrument's paths from .github/instrument-paths.txt in the current directory,
// the event from GITHUB_EVENT_NAME and GITHUB_EVENT_PATH, and writes `run=true` or `run=false` to
// GITHUB_OUTPUT, with the reason in the job's summary. Every later step of the job carries
// `if: steps.watch.outputs.run != 'false'`, which the policy test checks: a decision that was
// never written runs the instrument.
//
// - pull_request: the files `git diff --name-only <base>...<head>` names, from the event's SHAs.
// - merge_group: the same over the group's base and head; an instrument marked
//   `merge_group: skip` (the heavy ones, which ran on the pull request's head) does not run.
// - push: the files between the push's `before` and `after`.
// - schedule, workflow_dispatch and any other event: always runs.
// A change to a file ALWAYS names runs every instrument. Any error fails the step, so that a
// broken diff never reads as "nothing changed".

import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";

export const PATHS_FILE = ".github/instrument-paths.txt";
/// The files whose change runs every instrument: the lists, the code that reads them, and what
/// configures every cargo command (a `.cargo` directory, the toolchain files).
export const ALWAYS = [
  PATHS_FILE,
  ".github/scripts/watch.mjs",
  ".github/scripts/lib/watch.mjs",
  // Every job that builds in release runs it, so a change to it runs them all.
  ".github/scripts/overflow-proof.sh",
  ".cargo/**",
  "rust-toolchain",
  "rust-toolchain.toml",
];
const ZERO = /^0+$/;

/// Reads the path lists: `[<workflow file>]` opens a section, `merge_group: skip` marks it
/// heavy, every other line is a glob. `#` starts a comment.
export function parsePaths(text) {
  const sections = new Map();
  let current = null;
  for (const [n, raw] of text.split(/\r?\n/).entries()) {
    const line = raw.replace(/(^|\s)#.*$/, "").trim();
    if (line === "") continue;
    const where = `${PATHS_FILE}:${n + 1}`;
    const header = /^\[([\w.-]+\.yml)\]$/.exec(line);
    if (header) {
      if (sections.has(header[1])) throw new Error(`${where}: section ${header[1]} twice`);
      current = { mergeGroup: "run", patterns: [] };
      sections.set(header[1], current);
      continue;
    }
    if (!current) throw new Error(`${where}: a line outside a section`);
    const directive = /^([\w-]+):\s*(.*)$/.exec(line);
    if (directive) {
      if (directive[1] !== "merge_group" || directive[2] !== "skip") {
        throw new Error(`${where}: unknown directive "${line}"`);
      }
      current.mergeGroup = "skip";
      continue;
    }
    if (/\s/.test(line)) throw new Error(`${where}: a glob with a space`);
    if (current.patterns.includes(line)) throw new Error(`${where}: ${line} twice`);
    current.patterns.push(line);
  }
  for (const [name, section] of sections) {
    if (section.patterns.length === 0) throw new Error(`${PATHS_FILE}: section ${name} has no paths`);
  }
  return sections;
}

/// A glob as GitHub's `paths` filters read it: `**` matches any characters, `/` included; `*`
/// and `?` match within one path segment; everything else is literal.
export function globToRegExp(glob) {
  let source = "";
  for (let k = 0; k < glob.length; k++) {
    const c = glob[k];
    if (c === "*" && glob[k + 1] === "*") {
      source += ".*";
      k++;
    } else if (c === "*") source += "[^/]*";
    else if (c === "?") source += "[^/]";
    else source += c.replace(/[.+^${}()|[\]\\]/g, "\\$&");
  }
  return new RegExp(`^${source}$`);
}

export function matches(patterns, file) {
  return patterns.some((p) => globToRegExp(p).test(file));
}

/// The decision for one run: `{ run, reason, files }`. `changed(base, head)` lists the files.
export function decide(section, eventName, payload, changed) {
  if (eventName === "merge_group" && section.mergeGroup === "skip") {
    return { run: false, reason: "a heavy instrument: it ran on the pull request's head, not again on the merge group" };
  }
  let base;
  let head;
  if (eventName === "pull_request") {
    base = payload?.pull_request?.base?.sha;
    head = payload?.pull_request?.head?.sha;
  } else if (eventName === "merge_group") {
    base = payload?.merge_group?.base_sha;
    head = payload?.merge_group?.head_sha;
  } else if (eventName === "push") {
    base = payload?.before;
    head = payload?.after;
    if (!base || ZERO.test(base)) return { run: true, reason: "a push with no commit before it" };
  } else {
    return { run: true, reason: `a ${eventName || "local"} run checks everything` };
  }
  if (!base || !head) throw new Error(`the ${eventName} event names no base and head commits`);
  const files = changed(base, head);
  const shown = (list) =>
    list
      .slice(0, 10)
      .map((f) => JSON.stringify(f))
      .join(", ") + (list.length > 10 ? `, and ${list.length - 10} more` : "");
  const always = files.filter((f) => matches(ALWAYS, f));
  if (always.length > 0) return { run: true, reason: `files every instrument depends on changed: ${shown(always)}`, files };
  const watched = files.filter((f) => matches(section.patterns, f));
  if (watched.length > 0) return { run: true, reason: `${watched.length} watched files changed: ${shown(watched)}`, files };
  return { run: false, reason: `none of the ${files.length} changed files is a path this instrument watches`, files };
}

/// The files changed between the merge base of `base` and `head`, and `head`, renames listed as
/// a deletion and an addition so that a file moved out of a watched path still counts.
export function gitChanged(base, head, cwd) {
  const out = execFileSync("git", ["diff", "--name-only", "--no-renames", "-z", `${base}...${head}`], {
    cwd,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  return out.split("\0").filter((f) => f !== "");
}

/// The CLI: decides for `args[0]`, a workflow file name, in the repository at `cwd`.
export function main(args, env, cwd) {
  const workflow = args[0];
  if (!workflow || args.length !== 1) throw new Error("usage: node .github/scripts/watch.mjs <workflow file name>");
  if (!env.GITHUB_OUTPUT) throw new Error("GITHUB_OUTPUT is not set, so the decision would be lost");
  const sections = parsePaths(fs.readFileSync(path.join(cwd, PATHS_FILE), "utf8"));
  const section = sections.get(workflow);
  if (!section) throw new Error(`${PATHS_FILE} has no section [${workflow}]`);
  const eventName = env.GITHUB_EVENT_NAME ?? "";
  const payload = env.GITHUB_EVENT_PATH ? JSON.parse(fs.readFileSync(env.GITHUB_EVENT_PATH, "utf8")) : {};
  const verdict = decide(section, eventName, payload, (b, h) => gitChanged(b, h, cwd));
  const line = `${verdict.run ? "Runs" : "Nothing to check"}: ${verdict.reason}.`;
  console.log(line);
  fs.appendFileSync(env.GITHUB_OUTPUT, `run=${verdict.run}\n`);
  if (env.GITHUB_STEP_SUMMARY) fs.appendFileSync(env.GITHUB_STEP_SUMMARY, `${line}\n\n`);
  return verdict;
}
