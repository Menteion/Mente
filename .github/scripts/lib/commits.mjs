// The "Commits" check (.github/workflows/commits.yml): commit and pull request text that would
// otherwise reach `main`'s history unchecked, since pull requests merge with a merge commit, and
// a base the instruments never saw.
//
// - No commit subject in <base>..<head> is an unfinished one: "WIP" or "[WIP]", "Draft:" or
//   "[Draft]", "fixup" in any form, "squash!", "squash:" or "[squash]", "amend!".
// - No commit message, and not the pull request's title or body, holds CJK text: the repository's
//   text is English (CLAUDE.md, invariant 3). CJK_RANGES are `is_cjk`'s in
//   compiler/mente-cli/tests/gates.rs, and a test keeps the two equal.
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

/// The CLI: checks the event's commits in the repository at `cwd` and returns the exit code.
export async function main(env, cwd) {
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
    pullRequest = { title: env.PR_TITLE ?? "", body: env.PR_BODY ?? "" };
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
