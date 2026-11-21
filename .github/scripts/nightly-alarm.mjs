// The nightly alarm: the last job of each nightly workflow runs this after a scheduled run, with the
// workflow's own token, which may write issues and read the run's jobs and nothing else
// (.github/scripts/lib/nightly-alarm.mjs holds the logic, which the workflow policy tests).
//
//   GITHUB_TOKEN=<the job's token> RESULTS=<toJSON(needs)> node .github/scripts/nightly-alarm.mjs
//
// It reads GitHub's own variables for the run: GITHUB_API_URL, GITHUB_REPOSITORY, GITHUB_RUN_ID,
// GITHUB_SERVER_URL, GITHUB_SHA, GITHUB_WORKFLOW and GITHUB_WORKFLOW_REF.
//
// The alarm itself comes first, from what the run says of its jobs: the issue opened, commented on
// or closed. What it asks of GitHub after that, each failing job's error lines and whether the other
// nightlies have run, is best effort, and its failure leaves the alarm as it is (the review of
// 4ae3c27: a 502 on the jobs' listing, or a log that stalled, left a failing night with no issue).
// Every call has a time limit and is tried again on an answer that may pass on a second try.
import { NIGHTLIES, detailsBody, errorLines, failingJobs, failureBody, keepAlarm, keepRunning, verdict } from "./lib/nightly-alarm.mjs";

const env = process.env;
for (const name of ["GITHUB_TOKEN", "RESULTS", "GITHUB_API_URL", "GITHUB_REPOSITORY", "GITHUB_RUN_ID", "GITHUB_SERVER_URL", "GITHUB_SHA", "GITHUB_WORKFLOW", "GITHUB_WORKFLOW_REF"]) {
  if (!env[name]) {
    console.error(`nightly-alarm: ${name} is not set`);
    process.exit(1);
  }
}
// The time a call may take, and the waits before its second and third tries. ALARM_TIMEOUT_MS and
// ALARM_BACKOFF_MS shorten them for the run tests; the workflow's step sets neither, which the
// policy holds.
const TIMEOUT_MS = Number(env.ALARM_TIMEOUT_MS ?? 20_000);
const BACKOFF_MS = Number(env.ALARM_BACKOFF_MS ?? 2_000);
const TRIES = 3;
// The time the best-effort details may take in all.
const DETAILS_MS = 10 * TIMEOUT_MS;

const api = env.GITHUB_API_URL;
const repo = env.GITHUB_REPOSITORY;
const headers = { authorization: `Bearer ${env.GITHUB_TOKEN}`, accept: "application/vnd.github+json", "x-github-api-version": "2022-11-28" };
const wait = (ms) => new Promise((done) => setTimeout(done, ms));
// An answer worth a second try: no answer at all, a time out, too many requests, or GitHub's own
// trouble.
const transient = (status) => status === 0 || status === 429 || status >= 500;
class CallError extends Error {
  constructor(message, status) {
    super(message);
    this.status = status;
  }
}
/// One call to GitHub's API, within TIMEOUT_MS, tried TRIES times while its answer is transient.
async function call(method, route, body) {
  let last;
  for (let attempt = 1; attempt <= TRIES; attempt++) {
    try {
      const res = await fetch(`${api}/${route}`, { method, headers: body ? { ...headers, "content-type": "application/json" } : headers, body: body ? JSON.stringify(body) : undefined, signal: AbortSignal.timeout(TIMEOUT_MS) });
      if (res.ok) return res.status === 204 ? null : res.json();
      last = new CallError(`${method} ${route}: ${res.status} ${(await res.text()).slice(0, 300)}`, res.status);
    } catch (e) {
      last = e instanceof CallError ? e : new CallError(`${method} ${route}: ${e.message}`, 0);
    }
    if (!transient(last.status) || attempt === TRIES) break;
    await wait(BACKOFF_MS * attempt);
  }
  throw last;
}
// Every page of a listing.
async function all(route) {
  const out = [];
  for (let page = 1; ; page++) {
    const part = await call("GET", `${route}${route.includes("?") ? "&" : "?"}per_page=100&page=${page}`);
    const items = Array.isArray(part) ? part : (part.jobs ?? []);
    out.push(...items);
    if (items.length < 100) return out;
  }
}
const issues = {
  // Every open issue of the repository: which are an alarm's, the logic reads from their author and
  // marker, not from a filter of the server's.
  openIssues: () => all(`repos/${repo}/issues?state=open`),
  create: async (issue) => {
    // Once: a creation that failed is the logic's to try again, after it reads what stands.
    const res = await fetch(`${api}/repos/${repo}/issues`, { method: "POST", headers: { ...headers, "content-type": "application/json" }, body: JSON.stringify(issue), signal: AbortSignal.timeout(TIMEOUT_MS) });
    if (!res.ok) throw new CallError(`POST issues: ${res.status}`, res.status);
    return res.json();
  },
  comment: (number, text) => call("POST", `repos/${repo}/issues/${number}/comments`, { body: text }),
  close: (number) => call("PATCH", `repos/${repo}/issues/${number}`, { state: "closed", state_reason: "completed" }),
  // The latest scheduled run of a workflow, by its file's name: its time, or null when it has none.
  latestRun: async (file) => {
    const runs = await call("GET", `repos/${repo}/actions/workflows/${file.split("/").pop()}/runs?event=schedule&per_page=1`);
    return runs.workflow_runs?.[0]?.created_at ?? null;
  },
};

// The workflow's file, from GITHUB_WORKFLOW_REF: <owner>/<repository>/<file>@<ref>.
const file = env.GITHUB_WORKFLOW_REF.replace(/@.*$/, "").split("/").slice(2).join("/");
if (!NIGHTLIES.has(file)) {
  console.error(`nightly-alarm: ${file} is no nightly workflow the alarm knows (NIGHTLIES in lib/nightly-alarm.mjs)`);
  process.exit(1);
}
const run = verdict(JSON.parse(env.RESULTS));
const runUrl = `${env.GITHUB_SERVER_URL}/${repo}/actions/runs/${env.GITHUB_RUN_ID}`;
// The alarm: its failure, past every try, fails this job.
const done = await keepAlarm({
  verdict: run.verdict,
  file,
  name: env.GITHUB_WORKFLOW,
  sha: env.GITHUB_SHA,
  runUrl,
  body: run.verdict === "failure" ? failureBody({ runUrl, sha: env.GITHUB_SHA, failing: run.failing }) : "",
  api: issues,
});
console.log(`nightly-alarm: ${run.verdict}: ${JSON.stringify(done)}`);

// The details, best effort: the failing jobs of the run, matrix legs one by one, with the first error
// lines of each log, which GitHub answers with a redirect to its storage, fetched without the token.
if (run.verdict === "failure") {
  try {
    const until = Date.now() + DETAILS_MS;
    const jobs = failingJobs(await all(`repos/${repo}/actions/runs/${env.GITHUB_RUN_ID}/jobs`));
    const shown = [];
    for (const j of jobs) {
      let errors = [];
      if (Date.now() < until) {
        try {
          const at = await fetch(`${api}/repos/${repo}/actions/jobs/${j.id}/logs`, { headers, redirect: "manual", signal: AbortSignal.timeout(TIMEOUT_MS) });
          const where = at.headers.get("location");
          const res = where ? await fetch(where, { signal: AbortSignal.timeout(TIMEOUT_MS) }) : at;
          if (res.ok) errors = errorLines(await res.text());
        } catch {
          errors = [];
        }
      }
      shown.push({ name: j.name, result: j.conclusion, errors });
    }
    if (shown.length > 0) await issues.comment(done.issue, detailsBody({ runUrl, jobs: shown }));
  } catch (e) {
    console.log(`::warning::nightly-alarm: the details of the run could not be added to the issue: ${e.message}`);
  }
}
// The other nightlies, best effort: one that has not run within its period opens its own issue.
try {
  const opened = await keepRunning({ now: Date.now(), api: issues });
  if (opened.length > 0) console.log(`nightly-alarm: nightlies that have not run, issues ${opened.join(", ")}`);
} catch (e) {
  console.log(`::warning::nightly-alarm: whether the other nightlies have run could not be read: ${e.message}`);
}
