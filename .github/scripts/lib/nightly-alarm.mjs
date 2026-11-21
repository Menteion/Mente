// The nightly alarm's logic (.github/scripts/nightly-alarm.mjs runs it): one issue per nightly
// workflow, which the code owner is assigned to. A scheduled run that fails opens it, or comments on
// it when it is open already; a scheduled run that passes says so on it and closes it; and a
// nightly that has not run within its period opens it too. An instrument nobody reads finds
// nothing (Norman, 2026-11-21: the nightlies were red nearly every night from 11-10, unseen). This
// module talks to GitHub only through the `api` it is handed, so the policy's tests run it against
// a stand-in; it starts no process and opens no connection.

/// The issue a workflow's alarm keeps is the one whose body holds this marker, the workflow's file:
/// its title, from the workflow's name, may change with it (the review of 4ae3c27).
export const alarmMarker = (file) => `<!-- nightly-alarm: ${file} -->`;
/// The title a new issue is given, for people: by the workflow's name, or, for one that has not
/// run, its file.
export const alarmTitle = (name) => `Nightly: ${name} failing`;
/// The label on it, and who is assigned it: the code owner.
export const ALARM_LABEL = "nightly";
export const ALARM_OWNER = "Dancraft3746";
/// Who opens it: the workflows' own token, so that an issue opened by anyone else, whatever it
/// holds, is no alarm's and is left alone.
export const ALARM_AUTHOR = "github-actions[bot]";
/// The nightly workflows, each with the hours its scheduled runs may lie apart before the alarm
/// says it has not run: its period and two hours for a daily, a day for the weekly. The policy holds
/// these to the workflows' own schedules.
export const NIGHTLIES = new Map([
  [".github/workflows/fuzz-nightly.yml", 24 + 2],
  [".github/workflows/generated-nightly.yml", 24 + 2],
  [".github/workflows/mutants-full.yml", 7 * 24 + 24],
]);
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

/// The comment of a failing run from what the run itself says, before anything else is asked of
/// GitHub: its link and commit, and each job it needs that did not succeed, with its result.
export function failureBody({ runUrl, sha, failing }) {
  const jobs = failing.map((f) => `- \`${String(f.job).replace(/`/g, "'")}\`: ${f.result}`).join("\n");
  return `The scheduled run ${runUrl} failed, at ${sha}.\n\n${jobs}`;
}

/// The details of a failing run, added after the alarm itself as best it can: each failing job of
/// the run, matrix legs one by one, its result and its first error lines in a fence, where no text of
/// a log is read as markdown or mentions anyone.
export function detailsBody({ runUrl, jobs }) {
  const parts = [`The failing jobs of ${runUrl}:`];
  for (const j of jobs) {
    const lines = j.errors?.length ? j.errors.join("\n") : "(no error line in its log)";
    const f = fence(lines);
    parts.push(`**${j.name}**: ${j.result}\n\n${f}text\n${lines}\n${f}`);
  }
  return parts.join("\n\n");
}

/// The jobs of a run the details name: each completed and neither passed nor skipped. A job still
/// running, the alarm's own among them, is none.
export const failingJobs = (jobs) => jobs.filter((j) => j.status === "completed" && !["success", "skipped"].includes(j.conclusion));

/// The open alarm issues of a workflow's file, oldest first, from the repository's open issues.
const alarmsOf = (issues, file) => issues.filter((i) => i.user?.login === ALARM_AUTHOR && String(i.body ?? "").includes(alarmMarker(file))).sort((a, b) => a.number - b.number);

/// Opens a workflow's alarm issue. A creation whose answer was lost may have made the issue all the
/// same, so a failed one reads the open issues again before it tries once more: never two.
async function openAlarm(api, file, title, body) {
  const issue = { title, body: `${alarmMarker(file)}\n${body}`, labels: [ALARM_LABEL], assignees: [ALARM_OWNER] };
  try {
    return (await api.create(issue)).number;
  } catch (first) {
    const made = alarmsOf(await api.openIssues(), file);
    if (made.length > 0) return made[0].number;
    try {
      return (await api.create(issue)).number;
    } catch {
      throw first;
    }
  }
}

/// Keeps a workflow's one issue for the run's verdict, through `api`:
///   openIssues() -> [{ number, title, body, user: { login } }], the repository's open issues;
///   create({ title, body, labels, assignees }) -> { number };
///   comment(number, body); close(number).
/// A failure comments on the oldest open issue of the workflow's file, opened by ALARM_AUTHOR, or
/// opens one; any other such issue is closed as its duplicate, so that never two are open. A
/// success comments "green again" on each and closes it. Returns what it did.
export async function keepAlarm({ verdict: v, file, name, sha, runUrl, body, api }) {
  const existing = alarmsOf(await api.openIssues(), file);
  if (v === "failure") {
    const kept = existing.length === 0 ? await openAlarm(api, file, alarmTitle(name), body) : existing[0].number;
    if (existing.length > 0) await api.comment(kept, body);
    const duplicates = existing.slice(1).map((i) => i.number);
    for (const n of duplicates) {
      await api.comment(n, `Closed as a duplicate of #${kept}: one issue per nightly workflow.`);
      await api.close(n);
    }
    return { action: existing.length === 0 ? "opened" : "commented", issue: kept, closedDuplicates: duplicates };
  }
  if (v === "success") {
    for (const i of existing) {
      await api.comment(i.number, `Green again at ${sha}: ${runUrl}`);
      await api.close(i.number);
    }
    return { action: existing.length === 0 ? "nothing" : "closed", closed: existing.map((i) => i.number) };
  }
  throw new Error(`no verdict the alarm knows: ${v}`);
}

/// What a nightly that runs nowhere looks like to the others: for each nightly, its latest
/// scheduled run (the alarm's own is the run it stands in, so never late), `api.latestRun(file)` -> an ISO time or null, older than its hours allow,
/// or none, opens that workflow's issue, unless one is open already, which alarms enough. A
/// schedule GitHub disabled, or jobs that never started, raise no alarm of their own.
export async function keepRunning({ now, api }) {
  const opened = [];
  for (const [file, hours] of NIGHTLIES) {
    const last = await api.latestRun(file);
    const age = last === null ? Infinity : (now - Date.parse(last)) / 3_600_000;
    if (age <= hours) continue;
    if (alarmsOf(await api.openIssues(), file).length > 0) continue;
    const since = last === null ? "it has no scheduled run" : `its last scheduled run was at ${last}`;
    opened.push(await openAlarm(api, file, `Nightly: ${file.split("/").pop()} has not run`, `${file} has not run within ${hours} hours: ${since}.`));
  }
  return opened;
}
