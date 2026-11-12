// The "Commits" check (.github/workflows/commits.yml): commit and pull request text that would
// otherwise reach `main`'s history unchecked, since pull requests merge with a merge commit.
//
// - No commit subject in <base>..<head> starts with "WIP", "fixup!", "squash!" or "amend!".
// - No commit message, and not the pull request's title or body, holds CJK text: the repository's
//   text is English (CLAUDE.md, invariant 3). The ranges are `is_cjk`'s in
//   test_repository_text_is_english (compiler/mente-cli/src/main.rs), and a test keeps them equal.
//
// The commits come from the event payload's SHAs (GITHUB_EVENT_PATH); the title and body come from
// the environment (PR_TITLE, PR_BODY), set from the payload by the workflow, so that no text of the
// pull request is ever written into a script.

import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

/// The code points `is_cjk` rejects, as inclusive ranges.
export const CJK_RANGES = [
  [0x3000, 0x30ff],
  [0x3400, 0x9fff],
  [0xac00, 0xd7af],
  [0xf900, 0xfaff],
  [0xff00, 0xffef],
];

export function isCjk(c) {
  const code = c.codePointAt(0);
  return CJK_RANGES.some(([lo, hi]) => code >= lo && code <= hi);
}

/// The 1-based numbers of the lines of `text` that hold CJK text.
export function cjkLines(text) {
  return text
    .split(/\r?\n/)
    .flatMap((line, n) => ([...line].some(isCjk) ? [n + 1] : []));
}

/// Why a subject may not reach `main`, or null.
export function badSubject(subject) {
  const m = /^(wip\b|fixup!|squash!|amend!)/i.exec(subject);
  return m ? `starts with "${m[1]}"` : null;
}

/// Every problem in the commits and the pull request's text, one line each.
export function problems(commits, pullRequest) {
  const found = [];
  for (const c of commits) {
    const short = c.sha.slice(0, 7);
    const why = badSubject(c.subject);
    if (why) found.push(`commit ${short} "${c.subject}": its subject ${why}`);
    const lines = cjkLines(c.message);
    if (lines.length > 0) found.push(`commit ${short} "${c.subject}": CJK text in its message, line ${lines.join(", ")}`);
  }
  if (pullRequest) {
    for (const [what, text] of [
      ["title", pullRequest.title],
      ["body", pullRequest.body],
    ]) {
      const lines = cjkLines(text ?? "");
      if (lines.length > 0) found.push(`the pull request's ${what}: CJK text, line ${lines.join(", ")}`);
    }
  }
  return found;
}

/// The commits reachable from `head` and not from `base`, oldest first.
export function commitsBetween(base, head, cwd) {
  const out = execFileSync("git", ["log", "--reverse", "--format=%H%x1f%s%x1f%B%x1e", `${base}..${head}`], {
    cwd,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  return out
    .split("\x1e")
    .map((record) => record.replace(/^\n/, ""))
    .filter((record) => record !== "")
    .map((record) => {
      const [sha, subject, message] = record.split("\x1f");
      return { sha, subject, message };
    });
}

function main() {
  const eventName = process.env.GITHUB_EVENT_NAME ?? "";
  const payload = JSON.parse(fs.readFileSync(process.env.GITHUB_EVENT_PATH ?? "", "utf8"));
  let base;
  let head;
  let pullRequest = null;
  if (eventName === "pull_request") {
    base = payload.pull_request?.base?.sha;
    head = payload.pull_request?.head?.sha;
    pullRequest = { title: process.env.PR_TITLE ?? "", body: process.env.PR_BODY ?? "" };
  } else if (eventName === "merge_group") {
    base = payload.merge_group?.base_sha;
    head = payload.merge_group?.head_sha;
  } else {
    throw new Error(`the Commits check runs on pull_request and merge_group, not ${eventName}`);
  }
  if (!base || !head) throw new Error(`the ${eventName} event names no base and head commits`);
  const commits = commitsBetween(base, head, path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../.."));
  if (commits.length === 0) throw new Error(`no commits in ${base}..${head}`);
  const found = problems(commits, pullRequest);
  console.log(`${commits.length} commits in ${base.slice(0, 7)}..${head.slice(0, 7)}${pullRequest ? ", and the pull request's title and body" : ""}.`);
  for (const p of found) console.log(`::error::${p}`);
  if (found.length > 0) {
    console.log("Reword the commits (git rebase), or edit the pull request's text, and push again.");
    process.exitCode = 1;
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (e) {
    console.error(`error: ${e.message}`);
    process.exit(1);
  }
}
