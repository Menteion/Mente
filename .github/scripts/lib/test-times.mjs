// Holds every test well within its stop, the time nextest ends it at, on every runner, so that a
// test that grows toward its limit fails the change that made it grow, not a later one. A test of
// the conformance corpus quadrupled with pull request 183 and passed, 36 s under nextest's stop of
// 40 s, and the queue's next runs failed on it; nextest marks a test past 20 s SLOW, which fails
// nothing.
//
// The required Rust jobs run this after their tests (.github/scripts/test-times.mjs), over
// nextest's JUnit report. A test past HEADROOM of its stop fails, judged in the run's own seconds,
// unless .config/near-limit-tests.txt accepts it on that runner and names the owner and the fix
// scheduled for it. GitHub gives a job one of two classes of machine, about twice apart, so a test
// within HEADROOM on the faster stays within 90% of its stop on the slower; the same holds for every
// runner, Windows included, with no baseline from another run and nothing to normalize.
//
// This is the net for what the compiler's own counts of work cannot see, linking, starting
// processes and I/O: the cost of the compiler's work itself is held by those counts, which are the
// same on every machine (docs/design-review.md, the design of pull request 209 revisited).

/// The share of a test's stop past which it fails.
export const HEADROOM = 0.45;
/// The share under which an acceptance is no longer needed, below HEADROOM so that a test that
/// moves between the two classes of machine does not make its acceptance lapse and return.
export const STALE = 0.3;
/// The runners of the Rust jobs, by the label each runs on, all held alike.
export const RUNNERS = ["ubuntu-24.04", "windows-2025", "ubuntu-24.04-arm"];
/// nextest's JUnit report of the ci profile, as .config/nextest.toml names it, which the policy holds
/// equal: read here, so that no line of the job names a path into target/.
export const REPORT = "target/nextest/ci/junit.xml";
/// The acceptances, which CODEOWNERS gives to the owner.
export const ACCEPTANCES = ".config/near-limit-tests.txt";
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

/// The lines of the acceptances: `<runner> <binary id> <test name> <share>  # owner: fix`, the test
/// as nextest prints it and the runner by its label, the share of the test's stop it may take on
/// that runner, past HEADROOM and under 1. Each names who owns the fix and what it is.
export function acceptedShares(text) {
  const out = [];
  for (const [k, raw] of text.split(/\r?\n/).entries()) {
    const hash = raw.indexOf("#");
    const line = (hash < 0 ? raw : raw.slice(0, hash)).trim();
    if (line === "") continue;
    const why = hash < 0 ? "" : raw.slice(hash + 1).trim();
    const parts = line.split(/\s+/);
    const share = Number(parts[3]);
    const where = `${ACCEPTANCES}:${k + 1}`;
    if (parts.length !== 4) throw new Error(`${where}: not \`<runner> <binary id> <test name> <share>  # owner: fix\`: ${raw}`);
    if (!RUNNERS.includes(parts[0])) throw new Error(`${where}: ${parts[0]} is no runner of the Rust jobs`);
    if (!(share > HEADROOM && share < 1)) throw new Error(`${where}: a share of ${parts[3]}, not past HEADROOM and under 1`);
    if (!/^[^:]+:\s*\S/.test(why)) throw new Error(`${where}: an acceptance that does not name its owner and its fix, as \`# owner: fix\``);
    out.push({ runner: parts[0], id: `${parts[1]} ${parts[2]}`, share, why, line: k + 1 });
  }
  return out;
}

/// The tests .config/nextest.toml gives the longer stop, as `<package> <test path>`: those an
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

/// The verdict on a run's times on `runner`: each test past HEADROOM of its stop fails unless an
/// acceptance of this runner allows its share; an acceptance whose test now takes under STALE of its
/// stop is no longer needed, and one whose test the run lacks names nothing.
export function judge(current, longer, accepted, runner) {
  const mine = new Map(accepted.filter((a) => a.runner === runner).map((a) => [a.id, a]));
  const rows = [];
  for (const [id, t] of [...current].sort(([a], [b]) => a.localeCompare(b))) {
    const stop = stopOf(id, longer);
    const share = t / stop;
    const allowed = mine.get(id)?.share ?? HEADROOM;
    if (share > HEADROOM || mine.has(id)) rows.push({ id, time: t, stop, share, allowed, failed: share > allowed });
  }
  const stale = [...mine.values()].filter((a) => current.has(a.id) && current.get(a.id) / stopOf(a.id, longer) < STALE);
  const missing = [...mine.values()].filter((a) => !current.has(a.id));
  return { rows, failed: rows.filter((r) => r.failed), stale, missing };
}

const pct = (x) => `${(x * 100).toFixed(0)}%`;

/// What the step prints of a verdict, and its summary table.
export function report(runner, count, verdict) {
  const lines = [`test times on ${runner}: ${count} tests, each held to ${pct(HEADROOM)} of its stop`];
  for (const r of verdict.failed) {
    lines.push(`${r.id}: ${r.time.toFixed(1)} s, ${pct(r.share)} of its stop of ${r.stop} s, past ${pct(r.allowed)}: make it faster or split it, or accept it in ${ACCEPTANCES} with its owner and its fix`);
  }
  const table = ["| Test | Time (s) | Stop (s) | Share | Allowed | Verdict |", "| :--- | ---: | ---: | ---: | ---: | :--- |"];
  for (const r of verdict.rows) table.push(`| \`${r.id}\` | ${r.time.toFixed(1)} | ${r.stop} | ${pct(r.share)} | ${pct(r.allowed)} | ${r.failed ? "**fails**" : "ok"} |`);
  return { lines, summary: `### ${lines[0]}\n\n${table.join("\n")}\n` };
}

/// Runs the check over REPORT on TEST_TIMES_RUNNER, with GITHUB_EVENT_NAME in `env`; `io` reads
/// files and writes the summary. A stale acceptance, or one naming a test the run lacks, fails a
/// pull request and the queue, and is reported on a push to main, whose runs it must not turn red.
/// The exit code.
export function main(args, env, io) {
  const runner = env.TEST_TIMES_RUNNER;
  if (args.length !== 0) {
    io.err("usage: node .github/scripts/test-times.mjs, which reads nextest's JUnit report of the ci profile");
    return 2;
  }
  if (!RUNNERS.includes(runner)) {
    io.err(`test times: ${runner} is no runner of the Rust jobs the check knows: name it in .github/scripts/lib/test-times.mjs`);
    return 1;
  }
  const current = junitTimes(io.read(REPORT));
  const accepted = acceptedShares(io.exists(ACCEPTANCES) ? io.read(ACCEPTANCES) : "");
  const verdict = judge(current, longerTests(io.read(".config/nextest.toml")), accepted, runner);
  const { lines, summary } = report(runner, current.size, verdict);
  io.out(lines.join("\n"));
  io.summary(summary);
  const strict = env.GITHUB_EVENT_NAME !== "push";
  const lapsed = [
    ...verdict.stale.map((a) => `${ACCEPTANCES}:${a.line}: ${a.id} takes under ${pct(STALE)} of its stop on ${runner}, so its acceptance is no longer needed: remove the line`),
    ...verdict.missing.map((a) => `${ACCEPTANCES}:${a.line}: ${a.id} is no test of this run: remove or rename the line`),
  ];
  for (const l of lapsed) (strict ? io.err : io.out)(l);
  return verdict.failed.length > 0 || (strict && lapsed.length > 0) ? 1 : 0;
}
