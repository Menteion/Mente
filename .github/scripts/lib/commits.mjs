// The "Commits" check (.github/workflows/commits.yml): commit and pull request text that would
// otherwise reach `main`'s history unchecked, since pull requests merge with a merge commit, and
// a base the instruments never saw.
//
// - No commit subject in <base>..<head> is an unfinished one: "WIP" or "[WIP]", "Draft:" or
//   "[Draft]", "fixup" in any form, "squash!", "squash:" or "[squash]", "amend!".
// - No commit message, and not the pull request's title or body, holds CJK text: the repository's
//   text is English (CLAUDE.md, invariant 3). CJK_RANGES are `is_cjk`'s in
//   compiler/mente-cli/tests/gates.rs, and a test keeps the two equal.
// - Every pull request's body declares, in one canonical form, the root cause of what it fixes and
//   whether the fix removes it, or that it fixes nothing (root-cause-first in
//   docs/design-review.md; declarationProblems). Dependabot's declare themselves by their author.
// - A pull request whose base changed after its head was pushed fails, on every event, until a new
//   commit is pushed: retargeting fires only `pull_request: edited`, which no instrument runs on,
//   so their results are for the old base, and a later edit of the title or a re-run of an earlier
//   run must not hide that. The base changes come from three sources, any of which fails it: the
//   event's own `changes.base` (the retarget's run, before the API may show it), the issue events
//   API, which documents `base_ref_changed`, and the timeline, which also returns it (seen on
//   2026-11-13). The push is the first workflow run of the head commit. The API is read with the
//   job's read-only token.
//
// The commits come from the event payload (GITHUB_EVENT_PATH); the title and body come from the
// environment (PR_TITLE, PR_BODY), set from the payload by the workflow, so that no text of the
// pull request is ever written into a script. Commit subjects are printed JSON-escaped, so that
// none can form a workflow command.

import fs from "node:fs";

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

// ------------------------------------------------------------------------- the declaration

/// The labels of a pull request's declaration, each written at the start of a line, in this very
/// case, followed by its text on the same line (root-cause-first, Norman, 2026-11-22: "every
/// problem must be evaluated for whether it can be solved at its root cause"):
/// - a fix: `Root cause:` the cause, then exactly one of `Removed at the root:` how the fix removes
///   it, or `Not removable at the root:` why not, followed by `Guard:` what holds the class
///   instead;
/// - or `Not a fix:` what the pull request is, a feature, a refactor, a brief, instruments, a
///   dependency update.
export const ROOT_CAUSE = "Root cause:";
export const REMOVED = "Removed at the root:";
export const NOT_REMOVABLE = "Not removable at the root:";
export const GUARD = "Guard:";
export const NOT_A_FIX = "Not a fix:";
const LABELS = [ROOT_CAUSE, REMOVED, NOT_REMOVABLE, GUARD, NOT_A_FIX];
/// The authors whose pull requests the policy declares, by what every one of them is: Dependabot
/// writes its own text, and every pull request it opens updates a dependency. The author is the
/// event's, which GitHub writes, never a word of the body.
export const DECLARED_BY_AUTHOR = new Map([["dependabot[bot]", `${NOT_A_FIX} a dependency update`]]);
/// A line that reads like a label in another case, spelling or decoration (`root cause:`,
/// `**Root cause:**`, `## Root cause`, `- Not a Fix:`), which the one canonical form refuses
/// rather than pass over.
const NEAR_LABEL = /^[\s>#*_+-]*(?:\*\*|__)?\s*(root\s*[-_]?\s*cause|removed\s+at\s+the\s+root|not\s+removable\s+at\s+the\s+root|guard|not\s+a\s+fix)\s*(?:\*\*|__)?\s*(?::|$)/i;

/// The lines of a body that it shows as prose, each with its 1-based number: no line of a fenced
/// code block, which may quote the labels, and no HTML comment, which GitHub does not show.
export function proseLines(body) {
  const text = String(body ?? "").replace(/\r\n?/g, "\n");
  // A comment's lines are blanked, not removed, so that each line keeps its number; an unclosed one
  // hides the rest of the body, as GitHub renders it.
  const shown = text.replace(/<!--[\s\S]*?(?:-->|$)/g, (c) => c.replace(/[^\n]/g, " "));
  const out = [];
  let fence = null;
  for (const [k, line] of shown.split("\n").entries()) {
    const marker = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (marker && marker[1][0] === fence[0] && marker[1].length >= fence.length && line.trim() === marker[1]) fence = null;
      continue;
    }
    if (marker) {
      fence = marker[1];
      continue;
    }
    out.push({ n: k + 1, line });
  }
  return out;
}

/// What keeps a pull request's body from declaring, in the one canonical form, either the root cause
/// of what it fixes and whether the fix removes it, or that it fixes nothing. One message a problem;
/// none for an author DECLARED_BY_AUTHOR declares. No text of the body is echoed, only line numbers
/// and labels, so that none can reach the log as a workflow command.
export function declarationProblems(body, author) {
  if (DECLARED_BY_AUTHOR.has(author)) return [];
  const found = [];
  const at = new Map(LABELS.map((l) => [l, []]));
  for (const { n, line } of proseLines(body)) {
    const label = LABELS.find((l) => line.startsWith(l));
    if (label) {
      at.get(label).push(n);
      if (line.slice(label.length).trim() === "") found.push(`line ${n}: \`${label}\` holds no text after it`);
    } else if (NEAR_LABEL.test(line)) {
      found.push(`line ${n} reads like a label of the declaration in another case, spelling or form: write it at the start of the line exactly as one of ${LABELS.map((l) => `\`${l}\``).join(", ")}`);
    }
  }
  const [root, removed, notRemovable, guard, notAFix] = LABELS.map((l) => at.get(l));
  const fix = root.length + removed.length + notRemovable.length + guard.length;
  const lines = (list) => list.join(", ");
  if (fix === 0 && notAFix.length === 0) {
    found.push(`it declares neither \`${ROOT_CAUSE}\` with \`${REMOVED}\` or \`${NOT_REMOVABLE}\`, nor \`${NOT_A_FIX}\`: write one, each label at the start of a line`);
  } else if (fix > 0 && notAFix.length > 0) {
    found.push(`it declares both a fix (line ${lines([...root, ...removed, ...notRemovable, ...guard].sort((a, b) => a - b))}) and \`${NOT_A_FIX}\` (line ${lines(notAFix)}): keep the one that is true`);
  } else if (notAFix.length > 1) {
    found.push(`it declares \`${NOT_A_FIX}\` ${notAFix.length} times, lines ${lines(notAFix)}: once`);
  } else if (fix > 0) {
    if (root.length !== 1) found.push(root.length === 0 ? `it says how the fix meets the root (line ${lines([...removed, ...notRemovable, ...guard])}) with no \`${ROOT_CAUSE}\` before it` : `it declares \`${ROOT_CAUSE}\` ${root.length} times, lines ${lines(root)}: once`);
    const follow = [...removed, ...notRemovable];
    if (follow.length === 0) found.push(`its \`${ROOT_CAUSE}\` is followed by neither \`${REMOVED}\` nor \`${NOT_REMOVABLE}\``);
    else if (follow.length > 1) found.push(`it says ${follow.length} times how the fix meets the root, lines ${lines(follow.sort((a, b) => a - b))}: once, as \`${REMOVED}\` or \`${NOT_REMOVABLE}\``);
    else if (root.length === 1 && follow[0] < root[0]) found.push(`its \`${removed.length ? REMOVED : NOT_REMOVABLE}\` (line ${follow[0]}) stands before its \`${ROOT_CAUSE}\` (line ${root[0]})`);
    if (notRemovable.length === 1 && guard.length !== 1) found.push(guard.length === 0 ? `its \`${NOT_REMOVABLE}\` names no \`${GUARD}\` that holds the class instead` : `it names \`${GUARD}\` ${guard.length} times, lines ${lines(guard)}: once`);
    else if (notRemovable.length === 1 && guard[0] < notRemovable[0]) found.push(`its \`${GUARD}\` (line ${guard[0]}) stands before its \`${NOT_REMOVABLE}\` (line ${notRemovable[0]})`);
    if (notRemovable.length === 0 && guard.length > 0) found.push(`it names \`${GUARD}\` (line ${lines(guard)}) with no \`${NOT_REMOVABLE}\`: a guard holds a class only where the root stays`);
  }
  return found;
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
    for (const p of declarationProblems(pullRequest.body, pullRequest.author)) found.push(`the pull request's body: ${p}`);
  }
  return found;
}

/// The commits of git's log in the form .github/scripts/commits.mjs asks for, oldest first: each
/// record its id, its subject and its message, separated by \x1f and ended by \x1e.
export function parseLog(out) {
  return out
    .split("\x1e")
    .map((record) => record.replace(/^\n/, ""))
    .filter((record) => record !== "")
    .map((record) => {
      const [sha, subject, message] = record.split("\x1f");
      return { sha, subject, message };
    });
}

/// Whether the pull request's base changed after its head was pushed: the newest
/// `base_ref_changed` of its timeline against the first run of its head commit.
export function baseChangedSincePush(timeline, headRuns) {
  const changes = timeline.filter((e) => e.event === "base_ref_changed").map((e) => Date.parse(e.created_at));
  if (changes.length === 0) return false;
  const runs = headRuns.map((r) => Date.parse(r.created_at)).filter(Number.isFinite);
  if (runs.length === 0) throw new Error("the head commit has no workflow run, so its push time is unknown");
  return Math.max(...changes) > Math.min(...runs);
}

/// Every item of a paginated list of the GitHub API, read with the job's token.
async function apiList(env, path, key) {
  const api = env.GITHUB_API_URL || "https://api.github.com";
  if (!env.GITHUB_TOKEN) throw new Error("GITHUB_TOKEN is not set, so the pull request's timeline cannot be read");
  const out = [];
  for (let page = 1; page <= 50; page++) {
    const response = await fetch(`${api}${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`, {
      headers: { authorization: `Bearer ${env.GITHUB_TOKEN}`, accept: "application/vnd.github+json" },
    });
    if (!response.ok) throw new Error(`${path}: the API answered ${response.status}`);
    const body = await response.json();
    const items = key ? body[key] : body;
    if (!Array.isArray(items)) throw new Error(`${path}: the API answered no list`);
    out.push(...items);
    if (items.length < 100) return out;
  }
  throw new Error(`${path}: more than 50 pages`);
}

/// The pull request the event is about, and its head where the event names it: a pull request's
/// own, or the number a merge group's branch names (`gh-readonly-queue/<base>/pr-<number>-<sha>`,
/// whose sha is the group's base, not the pull request's head, which the API gives instead).
export function pullRequestOf(eventName, payload) {
  if (eventName === "pull_request") return { number: payload.pull_request?.number, head: payload.pull_request?.head?.sha };
  const m = /\/pr-(\d+)-[0-9a-f]{40}$/.exec(payload.merge_group?.head_ref ?? "");
  if (!m) throw new Error(`the merge group's branch names no pull request: ${JSON.stringify(payload.merge_group?.head_ref)}`);
  return { number: Number(m[1]), head: null };
}

/// One object of the GitHub API, read with the job's token.
async function apiGet(env, path) {
  const api = env.GITHUB_API_URL || "https://api.github.com";
  if (!env.GITHUB_TOKEN) throw new Error("GITHUB_TOKEN is not set, so the pull request cannot be read");
  const response = await fetch(`${api}${path}`, {
    headers: { authorization: `Bearer ${env.GITHUB_TOKEN}`, accept: "application/vnd.github+json" },
  });
  if (!response.ok) throw new Error(`${path}: the API answered ${response.status}`);
  return response.json();
}

/// The CLI's logic: checks the event's commits, `between(base, head)` oldest first, which
/// .github/scripts/commits.mjs reads with git, and returns the exit code. This module starts no
/// process (policy-runtime-judge).
export async function main(env, between) {
  if (typeof between !== "function") throw new Error("the Commits check needs a reader of the commits, which its caller gives");
  const eventName = env.GITHUB_EVENT_NAME ?? "";
  if (!env.GITHUB_EVENT_PATH) throw new Error("GITHUB_EVENT_PATH is not set");
  if (!env.GITHUB_REPOSITORY) throw new Error("GITHUB_REPOSITORY is not set");
  const payload = JSON.parse(fs.readFileSync(env.GITHUB_EVENT_PATH, "utf8"));
  let base;
  let head;
  let pullRequest = null;
  if (eventName === "pull_request") {
    base = payload.pull_request?.base?.sha;
    head = payload.pull_request?.head?.sha;
    // The author is the event's, which GitHub writes, as the base and the head are.
    pullRequest = { title: env.PR_TITLE ?? "", body: env.PR_BODY ?? "", author: payload.pull_request?.user?.login ?? "" };
  } else if (eventName === "merge_group") {
    base = payload.merge_group?.base_sha;
    head = payload.merge_group?.head_sha;
  } else {
    throw new Error(`the Commits check runs on pull_request and merge_group, not ${JSON.stringify(eventName)}`);
  }
  if (!base || !head) throw new Error(`the ${eventName} event names no base and head commits`);
  const pr = pullRequestOf(eventName, payload);
  if (!Number.isInteger(pr.number)) throw new Error(`the ${eventName} event names no pull request`);
  const repo = env.GITHUB_REPOSITORY;
  if (!pr.head) pr.head = (await apiGet(env, `/repos/${repo}/pulls/${pr.number}`)).head?.sha;
  if (!pr.head) throw new Error(`pull request ${pr.number} names no head commit`);
  // The retarget's own run knows from its event, whatever the API shows yet.
  const retargeted = eventName === "pull_request" && payload.action === "edited" && payload.changes?.base !== undefined;
  const events = await apiList(env, `/repos/${repo}/issues/${pr.number}/events`);
  const timeline = await apiList(env, `/repos/${repo}/issues/${pr.number}/timeline`);
  const headRuns = await apiList(env, `/repos/${repo}/actions/runs?event=pull_request&head_sha=${pr.head}`, "workflow_runs");
  const baseChanged = retargeted || baseChangedSincePush([...events, ...timeline], headRuns);
  const commits = between(base, head);
  if (commits.length === 0) throw new Error(`no commits in ${base}..${head}`);
  const found = problems(commits, pullRequest, baseChanged);
  console.log(`${commits.length} commits in ${base.slice(0, 7)}..${head.slice(0, 7)}${pullRequest ? ", and the pull request's title and body" : ""}.`);
  for (const p of found) console.log(`::error::${p}`);
  if (found.length > 0) {
    console.log("Reword the commits, edit the pull request's text, or push a commit, as each error says. An edit of the title or the body runs this check again.");
    return 1;
  }
  return 0;
}
