// Judges each test's time against the times main's own green runs took on the same runner, so that
// a test that grows past its limit by a change fails that change, not a later one. A test of the
// conformance corpus quadrupled with pull request 183 and passed, 36 s under nextest's stop of 40 s,
// and the queue's next runs failed on it; nextest marks a test past 20 s SLOW, which fails nothing.
//
// The required Rust jobs upload nextest's JUnit report of every run (`test-times-<runner>`), and
// after their tests run this over it (.github/scripts/test-times.mjs):
// - The baseline of a test is the median of its times in the latest green runs of CI on main,
//   pushes to main alone, that recorded this runner's times: never the change's own run, a run of
//   the merge queue, or one of another branch or repository.
// - Each runner's speed varies from run to run, by up to twice between the machines GitHub gives a
//   job, and every test with it: the run's speed against the baseline is the median ratio of every
//   test that took a second or more in both, and each test is judged against its baseline at that
//   speed.
// - A test fails when it takes more than FLOOR seconds and more than RATIO times its baseline at
//   the run's speed. A test main has no time for, a new one, fails past NEW_LIMIT seconds.
// - .config/slower-tests.txt accepts a test's new time, which review sees: the baseline is then at
//   least that time.
// - When none of main's recent green runs left this runner's times, the job fails, unless main's
//   workflow at its latest green run had no step that records them, which is the guard's own first
//   run: nothing then passes without a baseline more than once on main.
// - The Windows runner's times are recorded and not judged: from run to run on main, one test took
//   five times as long and came back, and a test that starts native processes swung by 1.8 times
//   beside the run's speed, past what one run can tell from a regression; the two Linux runners
//   kept every test within 1.17 times its previous time at the run's speed over twenty runs of main,
//   but for real changes.

/// The least time, in seconds, a test is judged at: below it, a ratio says more of the runner than
/// of the test.
export const FLOOR = 10;
/// The most a test may take against its baseline at the run's speed.
export const RATIO = 1.5;
/// The most a test main has no time for may take: nextest's mark of a slow test.
export const NEW_LIMIT = 20;
/// The green runs of main whose times are a test's baseline, the median of them.
export const BASELINE_RUNS = 3;
/// The runs of main searched for them, newest first.
export const RUNS_SEARCHED = 30;
/// The tests a run's speed is read from must be at least these many, or it is taken as 1.
export const MIN_COMMON = 20;
/// The workflow and the step that record the times, which the policy holds ci.yml to.
export const WORKFLOW = "ci.yml";
export const RECORD_STEP = "Upload the test times";
/// The runners whose times are judged, and those recorded alone, by the label the Rust jobs run on.
export const JUDGED_RUNNERS = ["ubuntu-24.04", "ubuntu-24.04-arm"];
export const RECORDED_RUNNERS = ["windows-2025"];
/// nextest's JUnit report of the ci profile, as .config/nextest.toml names it, which the policy
/// holds equal: read here, so that no line of the job names a path into target/.
export const REPORT = "target/nextest/ci/junit.xml";
/// The artifact of a runner's times.
export const artifactName = (runner) => `test-times-${runner}`;

const ENTITIES = { "&lt;": "<", "&gt;": ">", "&quot;": '"', "&apos;": "'", "&amp;": "&" };
const unescape = (s) => s.replace(/&(lt|gt|quot|apos|amp);/g, (e) => ENTITIES[e]);
const attributes = (text) => {
  const out = new Map();
  for (const m of text.matchAll(/([A-Za-z_:][\w.:-]*)\s*=\s*("([^"]*)"|'([^']*)')/g)) out.set(m[1], unescape(m[3] ?? m[4]));
  return out;
};

/// The time of every test that passed in a JUnit report as nextest writes it, by
/// `<binary id> <test name>`, the names nextest prints; a test that failed, errored or was skipped
/// has no time of its own here.
export function junitTimes(xml) {
  const out = new Map();
  for (const suite of xml.matchAll(/<testsuite\b([^>]*)>([\s\S]*?)<\/testsuite>/g)) {
    const suiteName = attributes(suite[1]).get("name");
    for (const c of suite[2].matchAll(/<testcase\b([^>]*?)(\/>|>([\s\S]*?)<\/testcase>)/g)) {
      const a = attributes(c[1]);
      if (/<(failure|error|skipped)\b/.test(c[3] ?? "")) continue;
      const binary = a.get("classname") ?? suiteName;
      const name = a.get("name");
      const time = Number(a.get("time"));
      if (binary === undefined || name === undefined || !Number.isFinite(time)) {
        throw new Error(`a test case the JUnit report does not name with its binary, name and time: <testcase${c[1]}>`);
      }
      out.set(`${binary} ${name}`, time);
    }
  }
  return out;
}

/// The times .config/slower-tests.txt accepts: `<binary id> <test name> <seconds>` a line, as
/// nextest prints the test, comments and blanks aside.
export function acceptedTimes(text) {
  const out = new Map();
  for (const [k, raw] of text.split(/\r?\n/).entries()) {
    const line = raw.replace(/#.*/, "").trim();
    if (line === "") continue;
    const parts = line.split(/\s+/);
    const seconds = Number(parts[2]);
    if (parts.length !== 3 || !(seconds > 0)) throw new Error(`.config/slower-tests.txt:${k + 1}: not \`<binary id> <test name> <seconds>\`: ${raw}`);
    out.set(`${parts[0]} ${parts[1]}`, seconds);
  }
  return out;
}

const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  return s.length % 2 === 1 ? s[(s.length - 1) / 2] : (s[s.length / 2 - 1] + s[s.length / 2]) / 2;
};

/// Each test's baseline: the median of its times in the runs given.
export function baselineTimes(runs) {
  const all = new Map();
  for (const run of runs) for (const [id, t] of run) all.set(id, [...(all.get(id) ?? []), t]);
  return new Map([...all].map(([id, ts]) => [id, median(ts)]));
}

/// The run's speed against the baseline: the median ratio of the tests that took a second or more
/// in both, or 1 when fewer than MIN_COMMON did.
export function speedOf(current, baseline) {
  const ratios = [...current].filter(([id, t]) => t >= 1 && (baseline.get(id) ?? 0) >= 1).map(([id, t]) => t / baseline.get(id));
  return ratios.length >= MIN_COMMON ? { speed: median(ratios), over: ratios.length } : { speed: 1, over: ratios.length };
}

/// The verdict on a run's times: each test past FLOOR seconds against its baseline at the run's
/// speed, or against NEW_LIMIT when main has no time for it; `accepted` raises a test's baseline.
export function judge(current, baseline, accepted = new Map()) {
  const { speed, over } = speedOf(current, baseline);
  const rows = [];
  for (const [id, t] of [...current].sort(([a], [b]) => a.localeCompare(b))) {
    if (t <= FLOOR) continue;
    const base = baseline.get(id);
    const allowed = accepted.get(id);
    if (base === undefined) {
      const limit = allowed === undefined ? NEW_LIMIT : RATIO * allowed;
      rows.push({ id, time: t, base: null, ratio: null, limit, failed: t > limit });
      continue;
    }
    const expected = Math.max(base * speed, allowed ?? 0);
    const ratio = t / expected;
    rows.push({ id, time: t, base, ratio, limit: RATIO * expected, failed: ratio > RATIO });
  }
  return { speed, over, rows, failed: rows.filter((r) => r.failed) };
}

/// The runs of main a baseline may come from, newest first: green pushes to main of this
/// repository, never the run judged or one of its commit.
export function baselineRuns(runs, { runId, sha, repository }) {
  return runs
    .filter((r) => r.event === "push" && r.head_branch === "main" && r.status === "completed" && r.conclusion === "success")
    .filter((r) => String(r.id) !== String(runId) && r.head_sha !== sha)
    .filter((r) => r.head_repository?.full_name === repository && r.repository?.full_name === repository)
    .sort((a, b) => String(b.created_at).localeCompare(String(a.created_at)));
}

/// Whether a workflow's text records the times: a step named RECORD_STEP.
export const recordsTimes = (workflow) => new RegExp(`^\\s*- name: ${RECORD_STEP}\\s*$`, "m").test(workflow);

/// The one file of a zip archive, as upload-artifact writes it: stored or deflated.
export function unzipOne(buf, inflateRaw) {
  const end = buf.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (end < 0) throw new Error("an artifact that is no zip archive");
  const entries = buf.readUInt16LE(end + 10);
  if (entries !== 1) throw new Error(`an artifact of ${entries} files, not the one report`);
  const dir = buf.readUInt32LE(end + 16);
  if (buf.readUInt32LE(dir) !== 0x02014b50) throw new Error("an artifact whose central directory is not where it says");
  const method = buf.readUInt16LE(dir + 10);
  const size = buf.readUInt32LE(dir + 20);
  const local = buf.readUInt32LE(dir + 42);
  if (buf.readUInt32LE(local) !== 0x04034b50) throw new Error("an artifact whose file is not where it says");
  const start = local + 30 + buf.readUInt16LE(local + 26) + buf.readUInt16LE(local + 28);
  const data = buf.subarray(start, start + size);
  if (method === 0) return data.toString("utf8");
  if (method === 8) return inflateRaw(data).toString("utf8");
  throw new Error(`an artifact compressed by method ${method}, neither stored nor deflated`);
}

/// What the step prints of a verdict, and its summary table.
export function report({ speed, over, rows, failed }, runner, baselines) {
  const fmt = (x) => (x === null ? "none" : x.toFixed(2));
  const lines = [
    `test times on ${runner}: ${rows.length} tests past ${FLOOR} s, against the median of main's runs ${baselines.join(", ")}, at the run's speed ${speed.toFixed(2)} (from ${over} tests)`,
  ];
  for (const r of failed) {
    lines.push(
      r.base === null
        ? `${r.id}: ${r.time.toFixed(1)} s, a test main has no time for, past ${r.limit.toFixed(1)} s: make it faster, or accept its time in .config/slower-tests.txt`
        : `${r.id}: ${r.time.toFixed(1)} s, ${fmt(r.ratio)} times its baseline of ${r.base.toFixed(1)} s at the run's speed, past ${RATIO}: make it faster, or accept its time in .config/slower-tests.txt`,
    );
  }
  const table = ["| Test | Time (s) | Baseline (s) | At the run's speed | Verdict |", "| :--- | ---: | ---: | ---: | :--- |"];
  for (const r of rows) table.push(`| \`${r.id}\` | ${r.time.toFixed(1)} | ${r.base === null ? "none" : r.base.toFixed(1)} | ${fmt(r.ratio)} | ${r.failed ? "**too slow**" : "ok"} |`);
  return { lines, summary: `### ${lines[0]}\n\n${table.join("\n")}\n` };
}

const API = "https://api.github.com";

/// Runs the check over REPORT, with GH_TOKEN, GITHUB_REPOSITORY, GITHUB_RUN_ID, GITHUB_SHA and
/// TEST_TIMES_RUNNER in `env`; `io` reads files, writes the summary and fetches. The exit code.
export async function main(args, env, io) {
  const runner = env.TEST_TIMES_RUNNER;
  if (args.length !== 0) {
    io.err("usage: node .github/scripts/test-times.mjs, which reads nextest's JUnit report of the ci profile");
    return 2;
  }
  const current = junitTimes(io.read(REPORT));
  if (RECORDED_RUNNERS.includes(runner)) {
    io.out(`test times on ${runner}: ${current.size} tests recorded, not judged (.github/scripts/lib/test-times.mjs says why)`);
    return 0;
  }
  if (!JUDGED_RUNNERS.includes(runner)) {
    io.err(`test times: ${runner} is a runner neither judged nor recorded alone: name it in .github/scripts/lib/test-times.mjs`);
    return 1;
  }
  const repository = env.GITHUB_REPOSITORY;
  const headers = { Authorization: `Bearer ${env.GH_TOKEN}`, Accept: "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "mente-test-times" };
  const get = async (path, binary = false) => {
    for (let attempt = 1; ; attempt++) {
      const res = await io.fetch(`${API}/${path}`, { headers });
      if (res.ok) return binary ? Buffer.from(await res.arrayBuffer()) : res.json();
      if (attempt === 3) throw new Error(`GET ${path}: ${res.status}`);
      await io.sleep(2000 * attempt);
    }
  };
  const listed = await get(`repos/${repository}/actions/workflows/${WORKFLOW}/runs?branch=main&event=push&status=success&per_page=${RUNS_SEARCHED}`);
  const candidates = baselineRuns(listed.workflow_runs ?? [], { runId: env.GITHUB_RUN_ID, sha: env.GITHUB_SHA, repository });
  const runs = [];
  const used = [];
  for (const run of candidates) {
    if (runs.length === BASELINE_RUNS) break;
    const found = await get(`repos/${repository}/actions/runs/${run.id}/artifacts?name=${artifactName(runner)}`);
    const artifact = (found.artifacts ?? []).find((a) => a.name === artifactName(runner) && !a.expired);
    if (!artifact) continue;
    runs.push(junitTimes(unzipOne(await get(`repos/${repository}/actions/artifacts/${artifact.id}/zip`, true), io.inflateRaw)));
    used.push(`${run.id} (${String(run.head_sha).slice(0, 7)})`);
  }
  if (runs.length === 0) {
    const latest = candidates[0];
    if (latest === undefined) {
      io.err(`test times on ${runner}: no green run of ${WORKFLOW} on main among the latest ${RUNS_SEARCHED}, so nothing to judge against`);
      return 1;
    }
    const file = await get(`repos/${repository}/contents/.github/workflows/${WORKFLOW}?ref=${latest.head_sha}`);
    if (!recordsTimes(Buffer.from(file.content, "base64").toString("utf8"))) {
      io.out(`test times on ${runner}: main's workflow at ${String(latest.head_sha).slice(0, 7)} records no test times yet, so there is no baseline; main's next run records them`);
      return 0;
    }
    io.err(`test times on ${runner}: main records the test times, but none of its latest ${RUNS_SEARCHED} green runs left this runner's: they expired or failed to upload; run CI on main again`);
    return 1;
  }
  const accepted = acceptedTimes(io.exists(".config/slower-tests.txt") ? io.read(".config/slower-tests.txt") : "");
  const verdict = judge(current, baselineTimes(runs), accepted);
  const { lines, summary } = report(verdict, runner, used);
  io.out(lines.join("\n"));
  io.summary(summary);
  return verdict.failed.length > 0 ? 1 : 0;
}
