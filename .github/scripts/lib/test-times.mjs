// Reports each test's share of its stop, the time nextest ends it at, on each runner of the
// required Rust jobs, so that a test growing toward its limit is seen in the change that made it
// grow. A test of the conformance corpus quadrupled with pull request 183 and passed, 36 s under
// nextest's stop of 40 s, and the queue's next runs failed on it; nextest marks a test past 20 s
// SLOW, which nobody reads.
//
// The required Rust jobs run this after their tests (.github/scripts/test-times.mjs), over
// nextest's JUnit report. It writes the tests with the highest shares into the job's summary, and a
// notice for each past NOTICE. It fails nothing on a time: the same code runs two to three times
// apart between the classes of machine a runner may give a job, so seconds are reported, and the
// gate on the compiler's cost is its own counts of work, which are the same on every machine. It
// fails only when the report cannot be read in full, so that the report cannot vanish unseen.

/// The share of a test's stop past which the report gives a notice.
export const NOTICE = 0.45;
/// How many tests the summary lists at least, highest shares first.
export const LISTED = 15;
/// nextest's JUnit report of the ci profile, as .config/nextest.toml names it, which the policy holds
/// equal: read here, so that no line of the job names a path into target/.
export const REPORT = "target/nextest/ci/junit.xml";
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
/// report misread reports nothing.
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

/// Each test's time as a share of its stop, highest first.
export function shares(current, longer) {
  return [...current]
    .map(([id, time]) => {
      const stop = stopOf(id, longer);
      return { id, time, stop, share: time / stop };
    })
    .sort((a, b) => b.share - a.share || a.id.localeCompare(b.id));
}

/// A share as a percentage, rounded up to a tenth of one, so that a share past NOTICE never reads
/// as NOTICE itself. The product in tenths of a percent is first cut to six decimals, below any
/// time JUnit writes, so that a share that is a whole tenth reads as it: 1.04 s of 40 is 26 tenths,
/// which the product gives as 26.000000000000004, and rounding that up would read 2.7%.
export const pct = (x) => `${(Math.ceil(Number((x * 1000).toFixed(6))) / 10).toFixed(1)}%`;
/// A message of a workflow command, as GitHub reads one: `%`, CR and LF escaped.
export const commandData = (s) => s.replace(/%/g, "%25").replace(/\r/g, "%0D").replace(/\n/g, "%0A");
/// The annotations of one type GitHub keeps for a step; past them it drops the rest unseen.
export const ANNOTATIONS = 10;

/// What the step prints of the shares on `runner`: a line, a notice for each test past NOTICE,
/// highest first, after one notice of how many there are when they are more than GitHub keeps,
/// and the summary's table of the LISTED highest shares, or of every test past NOTICE if more.
export function report(runner, rows) {
  const near = rows.filter((r) => r.share > NOTICE);
  const head = `test times on ${runner}: ${rows.length} tests, ${near.length} past ${pct(NOTICE)} of their stop`;
  const each = near.map((r) => `::notice title=A test near its stop::${commandData(`${r.id} took ${r.time.toFixed(1)} s on ${runner}, ${pct(r.share)} of its stop of ${r.stop} s`)}`);
  const many = near.length > ANNOTATIONS ? [`::notice title=Tests near their stop::${commandData(`${near.length} tests took past ${pct(NOTICE)} of their stop on ${runner}, more than the ${ANNOTATIONS} notices GitHub shows: the job's summary lists them all`)}`] : [];
  const notices = [...many, ...each];
  const table = ["| Test | Time (s) | Stop (s) | Share |", "| :--- | ---: | ---: | ---: |"];
  for (const r of rows.slice(0, Math.max(LISTED, near.length))) table.push(`| \`${r.id}\` | ${r.time.toFixed(1)} | ${r.stop} | ${r.share > NOTICE ? `**${pct(r.share)}**` : pct(r.share)} |`);
  return { lines: [head, ...notices], summary: `### ${head}\n\nA report, which fails nothing: the gate on the compiler's cost is its counts of work.\n\n${table.join("\n")}\n` };
}

/// Reports REPORT on TEST_TIMES_RUNNER; `io` reads files and writes the summary. The exit code: 0
/// whatever the times, 1 when the report cannot be read in full, 2 on an argument.
export function main(args, env, io) {
  if (args.length !== 0) {
    io.err("usage: node .github/scripts/test-times.mjs, which reads nextest's JUnit report of the ci profile");
    return 2;
  }
  const runner = env.TEST_TIMES_RUNNER || "this runner";
  let current;
  try {
    current = junitTimes(io.read(REPORT));
  } catch (e) {
    io.err(`::error title=No report of the tests' times::${commandData(`${REPORT} could not be read in full: ${e.message}`)}`);
    return 1;
  }
  const { lines, summary } = report(runner, shares(current, longerTests(io.read(".config/nextest.toml"))));
  io.out(lines.join("\n"));
  io.summary(summary);
  return 0;
}
