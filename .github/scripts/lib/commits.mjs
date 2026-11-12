// The "Commits" check (.github/workflows/commits.yml): commit and pull request text that would
// otherwise reach `main`'s history unchecked, since pull requests merge with a merge commit, and
// a base the instruments never saw.
//
// - No commit subject in <base>..<head> is an unfinished one: "WIP" or "[WIP]", "Draft:" or
//   "[Draft]", "fixup" in any form, "squash!", "squash:" or "[squash]", "amend!".
// - No commit message, and not the pull request's title or body, holds CJK text: the repository's
//   text is English (CLAUDE.md, invariant 3). CJK_RANGES are `is_cjk`'s in
//   compiler/mente-cli/src/main.rs, and a test keeps the two equal.
// - A pull request whose base was changed fails: retargeting fires only `pull_request: edited`,
//   which no instrument runs on, so their results were for the old base. A new commit runs them
//   again.
//
// The commits and the base change come from the event payload (GITHUB_EVENT_PATH); the title and
// body come from the environment (PR_TITLE, PR_BODY), set from the payload by the workflow, so
// that no text of the pull request is ever written into a script. Commit subjects are printed
// JSON-escaped, so that none can form a workflow command.

import fs from "node:fs";
import { execFileSync } from "node:child_process";

/// The code points `is_cjk` rejects, as inclusive ranges: Hangul Jamo; the CJK radicals and
/// Kangxi radicals; everything from the ideographic description characters through the unified
/// ideographs (punctuation, kana, Bopomofo, Hangul compatibility Jamo, Kanbun, strokes, enclosed
/// and compatibility forms); Hangul Jamo extensions and syllables; compatibility ideographs and
/// forms; full-width forms; and the supplementary ideographic planes.
export const CJK_RANGES = [
  [0x1100, 0x11ff],
  [0x2e80, 0x2fdf],
  [0x2ff0, 0x9fff],
  [0xa960, 0xa97f],
  [0xac00, 0xd7ff],
  [0xf900, 0xfaff],
  [0xfe30, 0xfe4f],
  [0xff00, 0xffef],
  [0x20000, 0x3ffff],
];

export function isCjk(c) {
  const code = c.codePointAt(0);
  return CJK_RANGES.some(([lo, hi]) => code >= lo && code <= hi);
}

/// The 1-based numbers of the lines of `text` that hold CJK text.
export function cjkLines(text) {
  return text.split(/\r?\n/).flatMap((line, n) => ([...line].some(isCjk) ? [n + 1] : []));
}

const UNFINISHED = /^[\s[(]*(wip\b|draft\s*[:\]]|fixup\b|squash\s*[!:\]]|squash\s*$|amend!)/i;

/// Why a subject may not reach `main`, or null.
export function badSubject(subject) {
  const m = UNFINISHED.exec(subject);
  return m ? `starts with ${JSON.stringify(m[1])}` : null;
}

/// Every problem in the commits, the pull request's text and its base, one line each.
export function problems(commits, pullRequest, baseChanged = false) {
  const found = [];
  if (baseChanged) found.push("base changed: push a commit to re-run the instruments");
  for (const c of commits) {
    const short = c.sha.slice(0, 7);
    const why = badSubject(c.subject);
    if (why) found.push(`commit ${short} ${JSON.stringify(c.subject)}: its subject ${why}`);
    const lines = cjkLines(c.message);
    if (lines.length > 0) found.push(`commit ${short} ${JSON.stringify(c.subject)}: CJK text in its message, line ${lines.join(", ")}`);
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

/// The CLI: checks the event's commits in the repository at `cwd` and returns the exit code.
export function main(env, cwd) {
  const eventName = env.GITHUB_EVENT_NAME ?? "";
  if (!env.GITHUB_EVENT_PATH) throw new Error("GITHUB_EVENT_PATH is not set");
  const payload = JSON.parse(fs.readFileSync(env.GITHUB_EVENT_PATH, "utf8"));
  let base;
  let head;
  let pullRequest = null;
  let baseChanged = false;
  if (eventName === "pull_request") {
    base = payload.pull_request?.base?.sha;
    head = payload.pull_request?.head?.sha;
    pullRequest = { title: env.PR_TITLE ?? "", body: env.PR_BODY ?? "" };
    baseChanged = payload.action === "edited" && payload.changes?.base !== undefined;
  } else if (eventName === "merge_group") {
    base = payload.merge_group?.base_sha;
    head = payload.merge_group?.head_sha;
  } else {
    throw new Error(`the Commits check runs on pull_request and merge_group, not ${JSON.stringify(eventName)}`);
  }
  if (!base || !head) throw new Error(`the ${eventName} event names no base and head commits`);
  const commits = commitsBetween(base, head, cwd);
  if (commits.length === 0) throw new Error(`no commits in ${base}..${head}`);
  const found = problems(commits, pullRequest, baseChanged);
  console.log(`${commits.length} commits in ${base.slice(0, 7)}..${head.slice(0, 7)}${pullRequest ? ", and the pull request's title and body" : ""}.`);
  for (const p of found) console.log(`::error::${p}`);
  if (found.length > 0) {
    console.log("Reword the commits, edit the pull request's text, or push a commit, as each error says.");
    return 1;
  }
  return 0;
}
