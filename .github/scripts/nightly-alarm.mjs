// The nightly alarm: the last job of each nightly workflow runs this after a scheduled run, with the
// workflow's own token, which may write issues and read the run's jobs and nothing else
// (.github/scripts/lib/nightly-alarm.mjs holds the logic, which the workflow policy tests).
//
//   GITHUB_TOKEN=<the job's token> RESULTS=<toJSON(needs)> node .github/scripts/nightly-alarm.mjs
//
// It reads GitHub's own variables for the run: GITHUB_API_URL, GITHUB_REPOSITORY, GITHUB_RUN_ID,
// GITHUB_SERVER_URL, GITHUB_SHA and GITHUB_WORKFLOW.
import { errorLines, failureBody, keepAlarm, verdict } from "./lib/nightly-alarm.mjs";

const env = process.env;
for (const name of ["GITHUB_TOKEN", "RESULTS", "GITHUB_API_URL", "GITHUB_REPOSITORY", "GITHUB_RUN_ID", "GITHUB_SERVER_URL", "GITHUB_SHA", "GITHUB_WORKFLOW"]) {
  if (!env[name]) {
    console.error(`nightly-alarm: ${name} is not set`);
    process.exit(1);
  }
}
const api = env.GITHUB_API_URL;
const repo = env.GITHUB_REPOSITORY;
const headers = { authorization: `Bearer ${env.GITHUB_TOKEN}`, accept: "application/vnd.github+json", "x-github-api-version": "2022-11-28" };
async function call(method, route, body) {
  const res = await fetch(`${api}/${route}`, { method, headers: body ? { ...headers, "content-type": "application/json" } : headers, body: body ? JSON.stringify(body) : undefined });
  if (!res.ok) throw new Error(`${method} ${route}: ${res.status} ${(await res.text()).slice(0, 300)}`);
  return res.status === 204 ? null : res.json();
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

const run = verdict(JSON.parse(env.RESULTS));
const runUrl = `${env.GITHUB_SERVER_URL}/${repo}/actions/runs/${env.GITHUB_RUN_ID}`;
let body = "";
if (run.verdict === "failure") {
  // The run's failing jobs, each matrix leg its own, with the first error lines of its log; this
  // job itself, still running, is none of them.
  const jobs = (await all(`repos/${repo}/actions/runs/${env.GITHUB_RUN_ID}/jobs`)).filter((j) => j.status === "completed" && !["success", "skipped"].includes(j.conclusion));
  const shown = [];
  for (const j of jobs) {
    let errors = [];
    try {
      // GitHub answers with a redirect to the log's storage, which is fetched without the token.
      const at = await fetch(`${api}/repos/${repo}/actions/jobs/${j.id}/logs`, { headers, redirect: "manual" });
      const where = at.headers.get("location");
      const res = where ? await fetch(where) : at;
      if (res.ok) errors = errorLines(await res.text());
    } catch {
      errors = [];
    }
    shown.push({ name: j.name, result: j.conclusion, errors });
  }
  if (shown.length === 0) for (const f of run.failing) shown.push({ name: f.job, result: f.result, errors: [] });
  body = failureBody({ runUrl, sha: env.GITHUB_SHA, jobs: shown });
}
const done = await keepAlarm({
  verdict: run.verdict,
  workflow: env.GITHUB_WORKFLOW,
  sha: env.GITHUB_SHA,
  runUrl,
  body,
  api: {
    openIssues: () => all(`repos/${repo}/issues?state=open&creator=${encodeURIComponent("github-actions[bot]")}`),
    create: (issue) => call("POST", `repos/${repo}/issues`, issue),
    comment: (number, text) => call("POST", `repos/${repo}/issues/${number}/comments`, { body: text }),
    close: (number) => call("PATCH", `repos/${repo}/issues/${number}`, { state: "closed", state_reason: "completed" }),
  },
});
console.log(`nightly-alarm: ${run.verdict}: ${JSON.stringify(done)}`);
