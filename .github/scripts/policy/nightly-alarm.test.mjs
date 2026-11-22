// The workflow policy: the nightly alarm (README.md in this directory). Every workflow that runs on
// a schedule raises its one alarm, which keeps one issue per workflow, and no other job writes
// issues.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { ALARM_AUTHOR, ALARM_LABEL, ALARM_OWNER, NIGHTLIES, alarmMarker, detailsBody, errorLines, failingJobs, failureBody, keepAlarm, keepRunning, lastRealRun, verdict as alarmVerdict } from "../lib/nightly-alarm.mjs";
import { SANDBOX_PROGRAM, deletedRegexes, evidence, exactRegex, expected, missing, parseDiff, readList, touched } from "../lib/mutants-floor.mjs";
import { PINNED_JOBS } from "./pins.mjs";
import { GATE_ONLY } from "./sandbox.mjs";
import { NIGHTLY_ALARM_JOBS, WORKFLOWS, isPlain, normalize, permissionProblems, steps, triggers, workflows } from "./workflows.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 3;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of nightly-alarm.test.mjs ran: a skip or a filter left the rest out`);
});
/// A nightly alarm's last step, exactly, and its job's permissions: the issues it writes, the run's
/// jobs it reads, the repository it checks out, and nothing more.
const ALARM_STEP = { name: "Open, comment on or close the workflow's one issue", env: { GITHUB_TOKEN: "${{ github.token }}", RESULTS: "${{ toJSON(needs) }}" }, run: "node .github/scripts/nightly-alarm.mjs" };
const ALARM_PERMISSIONS = { contents: "read", actions: "read", issues: "write" };
/// A value with its keys in order, so that two equal maps compare equal however they were written.
const canonical = (v) => (Array.isArray(v) ? v.map(canonical) : isPlain(v) ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canonical(v[k])])) : v);
const sameValue = (a, b) => JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));

/// What is wrong with a workflow's nightly alarm (Norman, 2026-11-21): a workflow that runs on a
/// schedule ends in a job alarm that needs every other job of it, runs on every scheduled run
/// whatever they ended in, holds ALARM_PERMISSIONS, ends with ALARM_STEP, and is pinned whole and
/// gate-only. The nightlies were red nearly every night from 11-10, and nobody saw.
export function alarmProblems(file, doc) {
  if (!("schedule" in triggers(doc))) return [];
  const jobs = isPlain(doc.jobs) ? doc.jobs : {};
  const alarm = jobs.alarm;
  if (!isPlain(alarm)) return [`${file} runs on a schedule and has no job alarm: a scheduled run that fails would go unseen`];
  const found = [];
  const others = Object.keys(jobs).filter((id) => id !== "alarm").sort();
  const needs = (Array.isArray(alarm.needs) ? alarm.needs : [alarm.needs]).filter((n) => n !== undefined).map(String).sort();
  if (!sameValue(needs, others)) found.push(`${file}, job alarm, needs ${needs.join(", ") || "nothing"}, not every other job: ${others.join(", ")}`);
  if (normalize(String(alarm.if ?? "")) !== "always() && github.event_name == 'schedule'") found.push(`${file}, job alarm, runs if: ${alarm.if ?? "(none)"}, not always() on a scheduled run: a run that timed out, was cancelled or passed would say nothing`);
  if (!sameValue(alarm.permissions, ALARM_PERMISSIONS)) found.push(`${file}, job alarm, holds the permissions ${JSON.stringify(alarm.permissions ?? null)}, not ALARM_PERMISSIONS`);
  const all = steps(alarm);
  if (all.length === 0 || !sameValue(all[all.length - 1], ALARM_STEP)) found.push(`${file}, job alarm, does not end with ALARM_STEP: ${JSON.stringify(all[all.length - 1] ?? null)}`);
  if (!PINNED_JOBS.has(`${file}#alarm`)) found.push(`${file}, job alarm, is not pinned whole in PINNED_JOBS`);
  if (!GATE_ONLY.has(`${file}#alarm`)) found.push(`${file}, job alarm, is not in GATE_ONLY`);
  return found;
}

check("every workflow that runs on a schedule raises its one alarm, and no other job writes issues", () => {
  const scheduled = workflows().filter((w) => "schedule" in triggers(w.doc));
  assert.deepEqual(scheduled.map((w) => `${w.file}#alarm`).sort(), [...NIGHTLY_ALARM_JOBS].sort(), "NIGHTLY_ALARM_JOBS names other jobs than the alarms of the workflows that run on a schedule");
  for (const { file, doc } of workflows()) assert.deepEqual(alarmProblems(file, doc), []);
  // Planted on a nightly: each way its failure could go unseen, or its token reach further.
  const fuzz = workflows().find((w) => w.file === "fuzz-nightly.yml").doc;
  const planted = (change) => {
    const d = JSON.parse(JSON.stringify(fuzz));
    change(d);
    return alarmProblems("fuzz-nightly.yml", d);
  };
  for (const [what, change] of [
    ["no alarm", (d) => delete d.jobs.alarm],
    ["an alarm that needs no job", (d) => (d.jobs.alarm.needs = [])],
    ["a job the alarm does not need", (d) => (d.jobs.extra = { "runs-on": "ubuntu-24.04", steps: [] })],
    ["an alarm on failure alone", (d) => (d.jobs.alarm.if = "failure()")],
    ["an alarm on every run, by hand too", (d) => (d.jobs.alarm.if = "always()")],
    ["an alarm that writes the repository", (d) => (d.jobs.alarm.permissions = { ...d.jobs.alarm.permissions, contents: "write" })],
    ["an alarm that runs another script", (d) => (d.jobs.alarm.steps[d.jobs.alarm.steps.length - 1].run = "node x.mjs")],
    ["an alarm handed a secret", (d) => (d.jobs.alarm.steps[d.jobs.alarm.steps.length - 1].env.X = "${{ secrets.X }}")],
    ["a step after the alarm's", (d) => d.jobs.alarm.steps.push({ run: "echo done" })],
  ]) {
    assert.ok(planted(change).length > 0, `${what} passes`);
  }
  // issues: write anywhere but a nightly alarm: a job of that name in a workflow on no schedule,
  // another job of a nightly, and a nightly's top.
  const writes = (file, doc) => permissionProblems(file, doc).some((p) => p.includes("issues: write, which only a nightly alarm may hold"));
  assert.ok(writes("ci.yml", { on: { pull_request: null }, permissions: { contents: "read" }, jobs: { alarm: { permissions: { issues: "write" } } } }), "an alarm of a workflow on no schedule writes issues");
  assert.ok(writes("fuzz-nightly.yml", { ...fuzz, jobs: { fuzz: { permissions: { issues: "write" } } } }), "another job of a nightly writes issues");
  assert.ok(writes("fuzz-nightly.yml", { ...fuzz, permissions: { contents: "read", issues: "write" } }), "a nightly's top writes issues");
  assert.ok(!writes("fuzz-nightly.yml", fuzz), "the nightly's own alarm may not write issues");
});

check("the nightly alarm keeps one issue per workflow: it opens it, comments on it and closes it", async () => {
  // A stand-in for GitHub's issues, as .github/scripts/nightly-alarm.mjs reaches them. `lost` makes
  // the next creation fail after it made the issue, as an answer lost on the way would; `fail`, before
  // it made anything.
  const github = () => {
    const issues = [];
    const comments = [];
    let next = 1;
    const state = { lost: 0, fail: 0 };
    const api = {
      openIssues: async () => issues.filter((i) => i.state === "open").map((i) => ({ number: i.number, title: i.title, body: i.body, user: { login: i.user } })),
      create: async ({ title, body, labels, assignees }) => {
        if (state.fail > 0) {
          state.fail--;
          throw new Error("502: nothing made");
        }
        const i = { number: next++, title, body, labels, assignees, state: "open", user: ALARM_AUTHOR };
        issues.push(i);
        if (state.lost > 0) {
          state.lost--;
          throw new Error("502: the answer was lost");
        }
        return { number: i.number };
      },
      comment: async (n, body) => void comments.push({ n, body }),
      close: async (n) => void (issues.find((i) => i.number === n).state = "closed"),
    };
    return { issues, comments, api, state };
  };
  const FUZZ = ".github/workflows/fuzz-nightly.yml";
  const g = github();
  const run = (v, body = "it failed", name = "Fuzz, nightly") => keepAlarm({ verdict: v, file: FUZZ, name, sha: "a".repeat(40), runUrl: "https://example.invalid/run", body, api: g.api });
  assert.deepEqual(await run("success"), { action: "nothing", closed: [] }, "a green run with no issue open");
  const first = await run("failure", "first");
  assert.equal(first.action, "opened");
  assert.deepEqual(g.issues.map((i) => [i.title, i.labels, i.assignees, i.state, i.body]), [["Nightly: Fuzz, nightly failing", [ALARM_LABEL], [ALARM_OWNER], "open", `${alarmMarker(FUZZ)}\nfirst`]]);
  const second = await run("failure", "second");
  assert.equal(second.action, "commented");
  assert.equal(g.issues.length, 1, "a later failure opened a second issue");
  assert.deepEqual(g.comments.at(-1), { n: first.issue, body: "second" });
  // The workflow renamed: its file is the same, and so is its issue (the review of 4ae3c27).
  const renamed = await run("failure", "renamed", "Fuzz, every night");
  assert.deepEqual([renamed.action, renamed.issue, g.issues.length], ["commented", first.issue, 1], "a renamed workflow opened a second issue");
  const green = await run("success");
  assert.deepEqual(green, { action: "closed", closed: [first.issue] });
  assert.equal(g.comments.at(-1).body, `Green again at ${"a".repeat(40)}: https://example.invalid/run`);
  assert.equal(g.issues[0].state, "closed");
  const again = await run("failure", "again");
  assert.equal(again.action, "opened", "a failure after the issue closed opens a new one");
  assert.equal(g.issues.filter((i) => i.state === "open").length, 1);
  // A creation whose answer was lost made the issue all the same: read again, never made twice.
  const lost = github();
  lost.state.lost = 1;
  const once = await keepAlarm({ verdict: "failure", file: FUZZ, name: "Fuzz, nightly", sha: "d".repeat(40), runUrl: "u", body: "x", api: lost.api });
  assert.deepEqual([once.action, once.issue, lost.issues.length], ["opened", 1, 1], "a lost answer made a second issue");
  // A creation that failed and made nothing is tried once more.
  const flaky = github();
  flaky.state.fail = 1;
  const retried = await keepAlarm({ verdict: "failure", file: FUZZ, name: "Fuzz, nightly", sha: "d".repeat(40), runUrl: "u", body: "x", api: flaky.api });
  assert.deepEqual([retried.action, retried.issue, flaky.issues.length], ["opened", 1, 1], "a creation that failed once left the night without an issue");
  // Never two open: of two open issues of the workflow, the oldest is kept and the other closed.
  const two = github();
  for (const body of ["x", "y"]) await two.api.create({ title: "anything", body: `${alarmMarker(FUZZ)}\n${body}`, labels: [], assignees: [] });
  const kept = await keepAlarm({ verdict: "failure", file: FUZZ, name: "Fuzz, nightly", sha: "b".repeat(40), runUrl: "u", body: "z", api: two.api });
  assert.deepEqual([kept.issue, kept.closedDuplicates], [1, [2]]);
  assert.deepEqual(two.issues.map((i) => i.state), ["open", "closed"]);
  // Anyone's issue that holds the marker, an issue of the title without it, and the alarm of another
  // workflow, are no alarm's of this one: left alone.
  const others = github();
  others.issues.push({ number: 7, title: "Nightly: Fuzz, nightly failing", body: alarmMarker(FUZZ), state: "open", user: "someone" });
  others.issues.push({ number: 8, title: "Nightly: Fuzz, nightly failing", body: "no marker", state: "open", user: ALARM_AUTHOR });
  others.issues.push({ number: 9, title: "Nightly: Mutants, full run failing", body: alarmMarker(".github/workflows/mutants-full.yml"), state: "open", user: ALARM_AUTHOR });
  const own = await keepAlarm({ verdict: "failure", file: FUZZ, name: "Fuzz, nightly", sha: "c".repeat(40), runUrl: "u", body: "w", api: others.api });
  assert.equal(own.action, "opened");
  assert.deepEqual(others.comments, []);
  await assert.rejects(() => keepAlarm({ verdict: "maybe", file: FUZZ, name: "x", sha: "s", runUrl: "u", body: "", api: g.api }), /no verdict the alarm knows/);
  // A workflow's last run that ran: a run that ended in startup_failure, or that has no job, never
  // started its jobs, its alarm among them, and is passed over; the jobs of a startup_failure are
  // not even asked for (the review of d15fe30).
  const jobsOf = new Map([[11, 0], [12, 3], [13, 1]]);
  const asked = [];
  const countJobs = async (run) => (asked.push(run.id), jobsOf.get(run.id) ?? 1);
  const listed = [
    { id: 10, created_at: "2026-11-21T11:00:00Z", conclusion: "startup_failure" },
    { id: 11, created_at: "2026-11-21T10:00:00Z", conclusion: "failure" },
    { id: 12, created_at: "2026-11-20T09:00:00Z", conclusion: "success" },
    { id: 13, created_at: "2026-11-19T09:00:00Z", conclusion: "success" },
  ];
  assert.equal(await lastRealRun(listed, countJobs), "2026-11-20T09:00:00Z");
  assert.deepEqual(asked, [11, 12], "the jobs of a startup_failure were asked for, or of runs past the one found");
  assert.equal(await lastRealRun([], countJobs), null);
  assert.equal(await lastRealRun([listed[0], listed[1]], countJobs), null, "runs that never started a job count as runs");
  // A nightly that has not run within its period, or whose latest runs never started a job, opens
  // its own issue, unless one is open; one that ran in time does not. Generated programs' latest run
  // is an hour-old startup_failure, behind which it last ran 27 hours ago.
  const now = Date.parse("2026-11-21T12:00:00Z");
  const runs = new Map([
    [".github/workflows/generated-nightly.yml", listed],
    [".github/workflows/mutants-full.yml", [listed[0], listed[1]]],
  ]);
  const stale = github();
  const latestRun = async (file) => lastRealRun(runs.get(file) ?? [{ id: 20, created_at: "2026-11-21T11:00:00Z", conclusion: "failure" }], countJobs);
  const opened = await keepRunning({ now, api: { ...stale.api, latestRun } });
  assert.deepEqual(stale.issues.map((i) => [i.title, i.body]), [
    ["Nightly: generated-nightly.yml has not run", `${alarmMarker(".github/workflows/generated-nightly.yml")}\n.github/workflows/generated-nightly.yml has not run within 26 hours: its last scheduled run that started a job was at 2026-11-20T09:00:00Z.`],
    ["Nightly: mutants-full.yml has not run", `${alarmMarker(".github/workflows/mutants-full.yml")}\n.github/workflows/mutants-full.yml has not run within 192 hours: none of its latest scheduled runs started a job.`],
  ]);
  assert.deepEqual(opened, [1, 2]);
  assert.deepEqual(await keepRunning({ now, api: { ...stale.api, latestRun } }), [], "a nightly already alarmed opened a second issue");
  assert.deepEqual(await keepRunning({ now, api: { ...github().api, latestRun: async () => "2026-11-21T00:00:00Z" } }), [], "nightlies that ran in time opened an issue");
  // A run's verdict, from its jobs' results: green only when every job succeeded; no job, a job of
  // no result, a cancellation, a skip and a failure are each a failure.
  assert.equal(alarmVerdict({ fuzz: { result: "success" } }).verdict, "success");
  for (const needs of [{ fuzz: { result: "failure" } }, { full: { result: "cancelled" } }, { paths: { result: "skipped" } }, { fuzz: { result: "success" }, full: { result: "failure" } }, {}, null, { fuzz: {} }]) {
    assert.equal(alarmVerdict(needs).verdict, "failure", JSON.stringify(needs));
  }
  // The first comment names the jobs from the run's own results; a job's name cannot open code.
  assert.equal(failureBody({ runUrl: "u", sha: "s", failing: [{ job: "fu`zz", result: "cancelled" }] }), "The scheduled run u failed, at s.\n\n- `fu'zz`: cancelled");
  // The details name the failing jobs: completed, neither passed nor skipped; the alarm's own job,
  // still running, is none of them (the review of 4ae3c27).
  assert.deepEqual(failingJobs([{ name: "a", status: "completed", conclusion: "failure" }, { name: "b", status: "completed", conclusion: "success" }, { name: "c", status: "completed", conclusion: "skipped" }, { name: "Nightly alarm (Fuzz)", status: "in_progress", conclusion: null }, { name: "d", status: "completed", conclusion: "timed_out" }]).map((j) => j.name), ["a", "d"]);
  // The error lines of a log, and a comment no line of a log can leave its fence in.
  assert.deepEqual(errorLines("2026-11-21T00:00:00.0Z ##[error]boom\nok\n##[error]  second  \n"), ["boom", "second"]);
  assert.equal(errorLines(Array.from({ length: 9 }, (_, k) => `##[error]e${k}`).join("\n")).length, 5);
  const body = detailsBody({ runUrl: "u", jobs: [{ name: "fuzz", result: "failure", errors: ["````", "@someone ping"] }] });
  const opening = /\n(`{5,})text\n/.exec(body);
  assert.ok(opening && body.endsWith(opening[1]), `a log's own fence closed the comment's: ${body}`);
});

check("the nightly alarm knows each nightly workflow and its schedule", () => {
  // NIGHTLIES holds every workflow that runs on a schedule, each with its period and the slack the
  // alarm gives it: a cron of every day, 24 hours and 2; of one day of the week, 168 and 24.
  const scheduled = workflows().filter((w) => "schedule" in triggers(w.doc));
  const period = (cron) => {
    const fields = String(cron).trim().split(/\s+/);
    assert.equal(fields.length, 5, `a cron of another form: ${cron}`);
    const [, , day, month, weekday] = fields;
    if (day === "*" && month === "*" && weekday === "*") return 24 + 2;
    if (day === "*" && month === "*" && /^[0-6]$/.test(weekday)) return 7 * 24 + 24;
    throw new Error(`a schedule the alarm has no period for: ${cron}`);
  };
  const expected = new Map(
    scheduled.map((w) => {
      const crons = triggers(w.doc).schedule.map((s) => s.cron);
      assert.equal(crons.length, 1, `${w.file} runs on ${crons.length} schedules`);
      return [`${WORKFLOWS}/${w.file}`, period(crons[0])];
    }),
  );
  assert.deepEqual(new Map([...NIGHTLIES].sort()), new Map([...expected].sort()), "NIGHTLIES in lib/nightly-alarm.mjs is not the workflows that run on a schedule, with their periods");
});
