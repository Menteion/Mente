// Judges each test's time, so that a test that grows toward its limit fails the change that made it
// grow, not a later one. A test of the conformance corpus quadrupled with pull request 183 and passed,
// 36 s under nextest's stop of 40 s, and the queue's next runs failed on it; nextest marks a test past
// 20 s SLOW, which fails nothing.
//
// The required Rust jobs time a fixed workload of their own after the tests, upload it with
// nextest's JUnit report (`test-times-<runner>`), and then run this (.github/scripts/test-times.mjs):
// - Headroom, on every runner: a test past HEADROOM of its own stop, the time nextest ends it at,
//   fails, unless .config/slower-tests.txt accepts it near its limit and names the fix scheduled for
//   it. This holds the Windows runner too, where 183's test met the stop.
// - Growth, on the Linux runners: a test is judged against its baseline, the median of its times in
//   the latest runs of main whose Rust job on this runner passed, pushes to main alone: never the
//   change's own run, a run of the merge queue, or one of another branch or repository.
//   - The run's speed against the baseline is the calibration's, the fixed workload timed in both,
//     which no change can move, held to the band each runner's machines were seen in: GitHub gives
//     the x86-64 jobs two classes of machine, about 1.65 times apart, and the AArch64 jobs one.
//   - A test fails when, at the baseline's speed, it takes more than FLOOR seconds and more than
//     RATIO times its baseline, or, when main has no time for it, more than NEW_LIMIT seconds;
//     .config/slower-tests.txt accepts a larger factor over the baseline for a change that means it.
//   - The Windows runner's growth is not judged: from run to run on main, one test took five times
//     as long and came back, and a test that starts native processes swung by 1.8 times beside the
//     rest, past what one run can tell from a regression.
// - When none of main's recent runs left this runner's times and calibration, the job fails, unless
//   main's workflow at its latest run had no step that records them, which is the guard's own first
//   run: nothing then passes without a baseline more than once on main.
// - An acceptance a run no longer needs, or that names a test the run lacks, fails a pull request
//   or the queue, and is only reported on main, whose runs a stale line must not turn red.

/// The share of a test's stop past which it fails: the stop is nextest's, 40 s or the longer
/// group's 180 s, and the runners' machines vary by up to 1.8 times from run to run.
export const HEADROOM = 0.6;
/// The least time, in seconds at the baseline's speed, a test's growth is judged at.
export const FLOOR = 10;
/// The most a test may take against its baseline at the run's speed.
export const RATIO = 1.5;
/// The most a test main has no time for may take, in seconds at the baseline's speed: nextest's mark
/// of a slow test.
export const NEW_LIMIT = 20;
/// The latest runs of main whose times are a test's baseline, the median of them.
export const BASELINE_RUNS = 3;
/// The runs of main searched for them, newest first.
export const RUNS_SEARCHED = 30;
/// The workflow and the step that record the times, which the policy holds ci.yml to.
export const WORKFLOW = "ci.yml";
export const RECORD_STEP = "Upload the test times";
/// The runners whose growth is judged, with the band their speed was seen in against main's own
/// runs (twenty runs of main, 2026-11-22), and those whose headroom alone is.
export const JUDGED_RUNNERS = new Map([
  ["ubuntu-24.04", [0.55, 1.8]],
  ["ubuntu-24.04-arm", [0.95, 1.05]],
]);
export const HEADROOM_RUNNERS = ["windows-2025"];
/// nextest's JUnit report of the ci profile and the calibration beside it, as .config/nextest.toml
/// names the one, which the policy holds equal: read here, so that no line of the job names a path
/// into target/.
export const REPORT = "target/nextest/ci/junit.xml";
export const CALIBRATION = "target/nextest/ci/calibration.json";
/// The artifact of a runner's times.
export const artifactName = (runner) => `test-times-${runner}`;
/// The stops of .config/nextest.toml, in seconds: the default's and the longer group's, each its
/// period times the periods it ends a test after, which the policy holds equal to the file.
export const DEFAULT_STOP = 40;
export const LONGER_STOP = 180;

const ENTITIES = { "&lt;": "<", "&gt;": ">", "&quot;": '"', "&apos;": "'", "&amp;": "&" };
const unescape = (s) => s.replace(/&(lt|gt|quot|apos|amp);/g, (e) => ENTITIES[e]);
const attributes = (text) => {
  const out = new Map();
  for (const m of text.matchAll(/([A-Za-z_:][\w.:-]*)\s*=\s*("([^"]*)"|'([^']*)')/g)) out.set(m[1], unescape(m[3] ?? m[4]));
  return out;
};

/// The time of every test that passed in a JUnit report as nextest writes it, by
/// `<binary id> <test name>`, the names nextest prints; a test that failed, errored or was skipped
/// has no time of its own here. The count read must be the count the report states, so that a
/// report misread passes nothing.
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
  const head = /<testsuites\b([^>]*)>/.exec(xml);
  if (head === null) throw new Error("a JUnit report without <testsuites>");
  const counts = attributes(head[1]);
  const count = (k) => Number(counts.get(k) ?? 0);
  const passed = count("tests") - count("skipped") - count("failures") - count("errors");
  if (!(passed > 0) || out.size !== passed) throw new Error(`the JUnit report states ${passed} tests that passed, and ${out.size} were read`);
  return out;
}

/// The fixed workload whose time is a runner's speed: the same arithmetic, sorting and hashing on
/// every run, which no change of the repository touches.
export function workload() {
  let x = 0x9e3779b9;
  const values = new Float64Array(1 << 18);
  for (let i = 0; i < values.length; i++) {
    x ^= x << 13;
    x >>>= 0;
    x ^= x >>> 17;
    x ^= x << 5;
    x >>>= 0;
    values[i] = x / 4294967296;
  }
  values.sort();
  const map = new Map();
  for (let i = 0; i < 100000; i++) map.set(`k${Math.imul(i, 2654435761) >>> 0}`, i);
  let h = 0;
  for (const [k, v] of map) h = (Math.imul(h, 31) + k.length + v) >>> 0;
  return (h + Math.floor(values[12345] * 1e6)) >>> 0;
}

/// The calibration: the median time, in milliseconds, of RUNS of the workload, after one to warm up.
export function calibrate(now, runs = 9) {
  workload();
  const times = [];
  for (let k = 0; k < runs; k++) {
    const start = now();
    workload();
    times.push(now() - start);
  }
  return { ms: median(times), runs: times };
}

/// The lines of .config/slower-tests.txt: `<kind> <runner> <binary id> <test name> <value>  # why`,
/// the test as nextest prints it. \`slower\` accepts a factor over main's baseline on a judged
/// runner; \`near-limit\` accepts a share of the test's stop, between HEADROOM and 1, and names in
/// its comment the fix scheduled for it. Each needs its reason.
export function acceptedTimes(text) {
  const out = [];
  for (const [k, raw] of text.split(/\r?\n/).entries()) {
    const hash = raw.indexOf("#");
    const line = (hash < 0 ? raw : raw.slice(0, hash)).trim();
    if (line === "") continue;
    const why = hash < 0 ? "" : raw.slice(hash + 1).trim();
    const parts = line.split(/\s+/);
    const value = Number(parts[4]);
    const [kind, runner] = parts;
    const where = `.config/slower-tests.txt:${k + 1}`;
    if (parts.length !== 5 || !(value > 0)) throw new Error(`${where}: not \`<kind> <runner> <binary id> <test name> <value>  # why\`: ${raw}`);
    if (why === "") throw new Error(`${where}: an acceptance without its reason`);
    if (kind === "slower") {
      if (!JUDGED_RUNNERS.has(runner)) throw new Error(`${where}: a factor over the baseline on ${runner}, whose growth is not judged`);
      if (!(value > RATIO)) throw new Error(`${where}: a factor of ${value}, which RATIO already allows`);
    } else if (kind === "near-limit") {
      if (!JUDGED_RUNNERS.has(runner) && !HEADROOM_RUNNERS.includes(runner)) throw new Error(`${where}: ${runner} is no runner of the Rust jobs`);
      if (!(value > HEADROOM && value < 1)) throw new Error(`${where}: a share of ${value}, not between HEADROOM and 1`);
    } else {
      throw new Error(`${where}: ${kind} is neither slower nor near-limit`);
    }
    out.push({ kind, runner, id: `${parts[2]} ${parts[3]}`, value, why, line: k + 1 });
  }
  return out;
}

/// The tests .config/nextest.toml gives the longer stop, as \`<package> <test path>\`: those an
/// override names with a slow-timeout of its own.
export function longerTests(nextestToml) {
  const out = new Set();
  for (const block of nextestToml.split("[[profile.default.overrides]]").slice(1)) {
    const body = block.split(/\n\[/)[0];
    if (!/^slow-timeout\s*=/m.test(body)) continue;
    for (const m of body.matchAll(/\(package\(([a-z][a-z0-9_-]*)\) & test\(=([^)]*)\)\)/g)) out.add(`${m[1]} ${m[2]}`);
  }
  return out;
}

/// A test's stop, by its JUnit name: its package is its binary id's first part.
export const stopOf = (id, longer) => {
  const [binary, name] = id.split(" ");
  return longer.has(`${binary.split("::")[0]} ${name}`) ? LONGER_STOP : DEFAULT_STOP;
};

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

/// The run's speed against the baseline's: the calibration's ratio, held to the runner's band.
export function speedOf(calibration, baselineCalibrations, band) {
  const raw = calibration / median(baselineCalibrations);
  return { raw, speed: Math.min(band[1], Math.max(band[0], raw)) };
}

/// The headroom verdict on a run's times, on any runner: each test past HEADROOM of its stop fails
/// unless a near-limit acceptance of this runner allows its share; an acceptance the run no longer
/// needs, the test under half the headroom, is stale.
export function headroom(current, longer, accepted, runner) {
  const mine = new Map(accepted.filter((a) => a.kind === "near-limit" && a.runner === runner).map((a) => [a.id, a]));
  const rows = [];
  for (const [id, t] of [...current].sort(([a], [b]) => a.localeCompare(b))) {
    const stop = stopOf(id, longer);
    const share = t / stop;
    const allowed = mine.get(id)?.value ?? HEADROOM;
    if (share > HEADROOM || mine.has(id)) rows.push({ id, time: t, stop, share, allowed, failed: share > allowed });
  }
  const stale = [...mine.values()].filter((a) => current.has(a.id) && current.get(a.id) / stopOf(a.id, longer) <= HEADROOM / 2);
  const missing = [...mine.values()].filter((a) => !current.has(a.id));
  return { rows, failed: rows.filter((r) => r.failed), stale, missing };
}

/// The growth verdict on a run's times: each test past FLOOR seconds at the baseline's speed against
/// its baseline, or against NEW_LIMIT when main has no time for it; a slower acceptance of this
/// runner raises the factor allowed. An acceptance the run no longer needs is stale.
export function growth(current, baseline, speed, accepted, runner) {
  const mine = new Map(accepted.filter((a) => a.kind === "slower" && a.runner === runner).map((a) => [a.id, a]));
  const rows = [];
  for (const [id, t] of [...current].sort(([a], [b]) => a.localeCompare(b))) {
    const atBase = t / speed;
    const base = baseline.get(id);
    if (base === undefined) {
      if (atBase > FLOOR) rows.push({ id, time: t, atBase, base: null, ratio: null, failed: atBase > NEW_LIMIT });
      continue;
    }
    if (atBase <= FLOOR) continue;
    const ratio = atBase / base;
    rows.push({ id, time: t, atBase, base, ratio, failed: ratio > (mine.get(id)?.value ?? RATIO) });
  }
  const ratioOf = (id) => current.get(id) / speed / baseline.get(id);
  const stale = [...mine.values()].filter((a) => current.has(a.id) && baseline.has(a.id) && !(ratioOf(a.id) > RATIO));
  const missing = [...mine.values()].filter((a) => !current.has(a.id));
  return { rows, failed: rows.filter((r) => r.failed), stale, missing };
}

/// The runs of main a baseline may come from, newest first: completed pushes to main of this
/// repository, never the run judged or one of its commit. Whether this runner's Rust job passed is
/// read from each run's jobs.
export function baselineRuns(runs, { runId, sha, repository }) {
  return runs
    .filter((r) => r.event === "push" && r.head_branch === "main" && r.status === "completed")
    .filter((r) => String(r.id) !== String(runId) && r.head_sha !== sha)
    .filter((r) => r.head_repository?.full_name === repository && r.repository?.full_name === repository)
    .sort((a, b) => String(b.created_at).localeCompare(String(a.created_at)));
}

/// Whether a workflow's text records the times: a step named RECORD_STEP.
export const recordsTimes = (workflow) => new RegExp(`^\\s*- name: ${RECORD_STEP}\\s*$`, "m").test(workflow);

/// The files of a zip archive as upload-artifact writes one, stored or deflated, by name.
export function unzip(buf, inflateRaw) {
  const end = buf.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (end < 0) throw new Error("an artifact that is no zip archive");
  const entries = buf.readUInt16LE(end + 10);
  let at = buf.readUInt32LE(end + 16);
  const out = new Map();
  for (let k = 0; k < entries; k++) {
    if (buf.readUInt32LE(at) !== 0x02014b50) throw new Error("an artifact whose central directory is not where it says");
    const method = buf.readUInt16LE(at + 10);
    const size = buf.readUInt32LE(at + 20);
    const nameLength = buf.readUInt16LE(at + 28);
    const local = buf.readUInt32LE(at + 42);
    const name = buf.subarray(at + 46, at + 46 + nameLength).toString("utf8");
    at += 46 + nameLength + buf.readUInt16LE(at + 30) + buf.readUInt16LE(at + 32);
    if (buf.readUInt32LE(local) !== 0x04034b50) throw new Error(`an artifact whose ${name} is not where it says`);
    const start = local + 30 + buf.readUInt16LE(local + 26) + buf.readUInt16LE(local + 28);
    const data = buf.subarray(start, start + size);
    if (method === 0) out.set(name, data.toString("utf8"));
    else if (method === 8) out.set(name, inflateRaw(data).toString("utf8"));
    else throw new Error(`an artifact's ${name} compressed by method ${method}, neither stored nor deflated`);
  }
  return out;
}

const fmt = (x) => (x === null ? "none" : x.toFixed(2));

/// What the step prints of the verdicts, and its summary table.
export function report(runner, room, grown, context) {
  const lines = [`test times on ${runner}: ${context}`];
  for (const r of room.failed) lines.push(`${r.id}: ${r.time.toFixed(1)} s, ${(r.share * 100).toFixed(0)}% of its stop of ${r.stop} s, past ${(r.allowed * 100).toFixed(0)}%: make it faster, or accept it near its limit in .config/slower-tests.txt with the fix scheduled for it`);
  for (const r of grown?.failed ?? []) {
    lines.push(
      r.base === null
        ? `${r.id}: ${r.atBase.toFixed(1)} s at the baseline's speed, a test main has no time for, past ${NEW_LIMIT} s: make it faster, or split it`
        : `${r.id}: ${r.atBase.toFixed(1)} s at the baseline's speed, ${fmt(r.ratio)} times its baseline of ${r.base.toFixed(1)} s: make it faster, or accept the factor in .config/slower-tests.txt`,
    );
  }
  const table = ["| Test | Time (s) | Share of its stop | At the baseline's speed (s) | Baseline (s) | Factor | Verdict |", "| :--- | ---: | ---: | ---: | ---: | ---: | :--- |"];
  const byId = new Map();
  for (const r of room.rows) byId.set(r.id, { ...byId.get(r.id), room: r });
  for (const r of grown?.rows ?? []) byId.set(r.id, { ...byId.get(r.id), grown: r });
  for (const [id, { room: a, grown: g }] of [...byId].sort(([x], [y]) => x.localeCompare(y))) {
    const t = (a ?? g).time;
    const failed = (a?.failed ?? false) || (g?.failed ?? false);
    table.push(`| \`${id}\` | ${t.toFixed(1)} | ${a ? `${(a.share * 100).toFixed(0)}%` : ""} | ${g ? g.atBase.toFixed(1) : ""} | ${g?.base === null || !g ? "" : g.base.toFixed(1)} | ${g ? fmt(g.ratio) : ""} | ${failed ? "**fails**" : "ok"} |`);
  }
  return { lines, summary: `### ${lines[0]}\n\n${table.join("\n")}\n` };
}

const API = "https://api.github.com";

/// The step that times the workload: writes CALIBRATION.
export function calibrateStep(io) {
  const result = calibrate(io.now);
  io.write(CALIBRATION, JSON.stringify(result));
  io.out(`calibration: ${result.ms.toFixed(1)} ms, the median of ${result.runs.length} runs of the workload`);
  return 0;
}

/// Runs the check over REPORT and CALIBRATION, with TEST_TIMES_RUNNER, TEST_TIMES_JOB (the Rust
/// job's name on this runner), GITHUB_EVENT_NAME and, on a judged runner, GH_TOKEN,
/// GITHUB_REPOSITORY, GITHUB_RUN_ID and GITHUB_SHA in \`env\`; \`io\` reads files, writes the summary and
/// fetches. The exit code.
export async function main(args, env, io) {
  if (args.length === 1 && args[0] === "calibrate") return calibrateStep(io);
  const runner = env.TEST_TIMES_RUNNER;
  if (args.length !== 0) {
    io.err("usage: node .github/scripts/test-times.mjs [calibrate]");
    return 2;
  }
  if (!JUDGED_RUNNERS.has(runner) && !HEADROOM_RUNNERS.includes(runner)) {
    io.err(`test times: ${runner} is no runner the check knows: name it in .github/scripts/lib/test-times.mjs`);
    return 1;
  }
  const current = junitTimes(io.read(REPORT));
  const calibration = JSON.parse(io.read(CALIBRATION)).ms;
  const accepted = acceptedTimes(io.exists(".config/slower-tests.txt") ? io.read(".config/slower-tests.txt") : "");
  const longer = longerTests(io.read(".config/nextest.toml"));
  // On main a stale or missing acceptance is reported, not failed: a pull request removes it.
  const strict = env.GITHUB_EVENT_NAME !== "push";
  const lapsed = (verdicts) => {
    const found = verdicts.flatMap((v) => [
      ...v.stale.map((a) => `.config/slower-tests.txt:${a.line}: ${a.kind} ${a.id} on ${a.runner} is no longer needed: remove the line`),
      ...v.missing.map((a) => `.config/slower-tests.txt:${a.line}: ${a.id} is no test of this run: remove the line`),
    ]);
    for (const f of found) (strict ? io.err : io.out)(f);
    return strict ? found.length : 0;
  };
  const room = headroom(current, longer, accepted, runner);
  if (HEADROOM_RUNNERS.includes(runner)) {
    const { lines, summary } = report(runner, room, null, `${current.size} tests against their stops; their growth is recorded, not judged`);
    io.out(lines.join("\n"));
    io.summary(summary);
    return room.failed.length + lapsed([room]) > 0 ? 1 : 0;
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
  const listed = await get(`repos/${repository}/actions/workflows/${WORKFLOW}/runs?branch=main&event=push&status=completed&per_page=${RUNS_SEARCHED}`);
  const candidates = baselineRuns(listed.workflow_runs ?? [], { runId: env.GITHUB_RUN_ID, sha: env.GITHUB_SHA, repository });
  const runs = [];
  const calibrations = [];
  const used = [];
  for (const run of candidates) {
    if (runs.length === BASELINE_RUNS) break;
    const jobs = await get(`repos/${repository}/actions/runs/${run.id}/jobs?per_page=100`);
    if (!(jobs.jobs ?? []).some((j) => j.name === env.TEST_TIMES_JOB && j.conclusion === "success")) continue;
    const found = await get(`repos/${repository}/actions/runs/${run.id}/artifacts?name=${artifactName(runner)}`);
    const artifact = (found.artifacts ?? []).find((a) => a.name === artifactName(runner) && !a.expired);
    if (!artifact) continue;
    const files = unzip(await get(`repos/${repository}/actions/artifacts/${artifact.id}/zip`, true), io.inflateRaw);
    if (!files.has("junit.xml") || !files.has("calibration.json")) continue;
    runs.push(junitTimes(files.get("junit.xml")));
    calibrations.push(JSON.parse(files.get("calibration.json")).ms);
    used.push(`${run.id} (${String(run.head_sha).slice(0, 7)})`);
  }
  if (runs.length === 0) {
    const latest = candidates[0];
    if (latest === undefined) {
      io.err(`test times on ${runner}: no completed run of ${WORKFLOW} on main among the latest ${RUNS_SEARCHED}, so nothing to judge against`);
      return 1;
    }
    const file = await get(`repos/${repository}/contents/.github/workflows/${WORKFLOW}?ref=${latest.head_sha}`);
    if (!recordsTimes(Buffer.from(file.content, "base64").toString("utf8"))) {
      const { lines, summary } = report(runner, room, null, `main's workflow at ${String(latest.head_sha).slice(0, 7)} records no test times yet, so their growth has no baseline; main's next run records them`);
      io.out(lines.join("\n"));
      io.summary(summary);
      return room.failed.length + lapsed([room]) > 0 ? 1 : 0;
    }
    io.err(`test times on ${runner}: main records the test times, but no ${env.TEST_TIMES_JOB} job of its latest ${RUNS_SEARCHED} runs that passed left them with their calibration: they expired or failed to upload; run CI on main again`);
    return 1;
  }
  const { raw, speed } = speedOf(calibration, calibrations, JUDGED_RUNNERS.get(runner));
  const grown = growth(current, baselineTimes(runs), speed, accepted, runner);
  const context = `${current.size} tests against their stops, and against the median of main's runs ${used.join(", ")} at the run's speed ${speed.toFixed(2)} (the calibration's ${raw.toFixed(2)}, held to ${JUDGED_RUNNERS.get(runner).join(" to ")})`;
  const { lines, summary } = report(runner, room, grown, context);
  io.out(lines.join("\n"));
  io.summary(summary);
  return room.failed.length + grown.failed.length + lapsed([room, grown]) > 0 ? 1 : 0;
}
