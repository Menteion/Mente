// The nightly alarm's logic (.github/scripts/nightly-alarm.mjs runs it): one issue per nightly
// workflow, which the code owner is assigned to. A scheduled run that fails opens it, or comments on
// it when it is open already, with the run's link and each failing job's first error lines; a
// scheduled run that passes says so on it and closes it. An instrument nobody reads finds nothing
// (Norman, 2026-11-21: the nightlies were red nearly every night from 11-10, unseen). This module
// talks to GitHub only through the `api` it is handed, so the policy's tests run it against a
// stand-in; it starts no process and opens no connection.

/// The one issue's title for a workflow, by its name as GitHub shows it.
export const alarmTitle = (workflow) => `Nightly: ${workflow} failing`;
/// The label on it, and who is assigned it: the code owner.
export const ALARM_LABEL = "nightly";
export const ALARM_OWNER = "Dancraft3746";
/// Who opens it: the workflow's own token, so that an issue of the same title opened by anyone
/// else is no alarm's and is left alone.
export const ALARM_AUTHOR = "github-actions[bot]";
/// The error lines of a failing job a comment shows, at most, and the characters of each.
const ERROR_LINES = 5;
const LINE_MAX = 300;

/// What a run says, from its jobs' results as `toJSON(needs)` gives them: "success" when every job
/// it needs succeeded, else "failure", the jobs that did not with their results. No job at all, or
/// one whose result is no word GitHub uses, is a failure: a run that cannot be read never reads as
/// green.
export function verdict(needs) {
  const entries = needs !== null && typeof needs === "object" && !Array.isArray(needs) ? Object.entries(needs) : [];
  if (entries.length === 0) return { verdict: "failure", failing: [{ job: "(the run's jobs)", result: "unread" }] };
  const failing = entries.filter(([, v]) => v?.result !== "success").map(([job, v]) => ({ job, result: typeof v?.result === "string" ? v.result : "unread" }));
  return failing.length === 0 ? { verdict: "success", failing: [] } : { verdict: "failure", failing };
}

/// The lines of a job's log that GitHub marks as errors, the first `max`, each without its time and
/// mark and cut to LINE_MAX characters.
export function errorLines(log, max = ERROR_LINES) {
  const out = [];
  for (const line of String(log).split(/\r?\n/)) {
    const m = /##\[error\](.*)$/.exec(line);
    if (!m) continue;
    const text = m[1].trim();
    out.push(text.length > LINE_MAX ? `${text.slice(0, LINE_MAX - 3)}...` : text);
    if (out.length >= max) break;
  }
  return out;
}

/// A fence no line of `text` can close: one backtick more than the longest run of them in it.
const fence = (text) => "`".repeat(Math.max(3, ...[...String(text).matchAll(/`+/g)].map((m) => m[0].length + 1)));

/// The comment of a failing run: its link and commit, then each failing job, its result and its
/// first error lines in a fence, where no text of a log is read as markdown or mentions anyone.
export function failureBody({ runUrl, sha, jobs }) {
  const parts = [`The scheduled run ${runUrl} failed, at ${sha}.`];
  for (const j of jobs) {
    const lines = j.errors?.length ? j.errors.join("\n") : "(no error line in its log)";
    const f = fence(lines);
    parts.push(`**${j.name}**: ${j.result}\n\n${f}text\n${lines}\n${f}`);
  }
  return parts.join("\n\n");
}

/// Keeps the workflow's one issue for the run's verdict, through `api`:
///   openIssues() -> [{ number, title, user: { login } }], the repository's open issues;
///   create({ title, body, labels, assignees }) -> { number };
///   comment(number, body); close(number).
/// A failure comments on the oldest open issue of the title, opened by ALARM_AUTHOR, or opens one;
/// any other such issue is closed as its duplicate, so that never two are open. A success comments
/// "green again" on each and closes it. Returns what it did.
export async function keepAlarm({ verdict: v, workflow, sha, runUrl, body, api }) {
  const title = alarmTitle(workflow);
  const open = (await api.openIssues()).filter((i) => i.title === title && i.user?.login === ALARM_AUTHOR).sort((a, b) => a.number - b.number);
  if (v === "failure") {
    let kept;
    if (open.length === 0) {
      kept = (await api.create({ title, body, labels: [ALARM_LABEL], assignees: [ALARM_OWNER] })).number;
    } else {
      kept = open[0].number;
      await api.comment(kept, body);
    }
    const duplicates = open.slice(1).map((i) => i.number);
    for (const n of duplicates) {
      await api.comment(n, `Closed as a duplicate of #${kept}: one issue per nightly workflow.`);
      await api.close(n);
    }
    return { action: open.length === 0 ? "opened" : "commented", issue: kept, closedDuplicates: duplicates };
  }
  if (v === "success") {
    for (const i of open) {
      await api.comment(i.number, `Green again at ${sha}: ${runUrl}`);
      await api.close(i.number);
    }
    return { action: open.length === 0 ? "nothing" : "closed", closed: open.map((i) => i.number) };
  }
  throw new Error(`no verdict the alarm knows: ${v}`);
}
