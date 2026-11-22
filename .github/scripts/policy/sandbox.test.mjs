// The workflow policy: the tests' sandbox (README.md in this directory). Every job that runs code of
// a change runs it in the sandbox, or says why it does not.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import os from "node:os";
import { SANDBOX_PROGRAM, deletedRegexes, evidence, exactRegex, expected, missing, parseDiff, readList, touched } from "../lib/mutants-floor.mjs";
import { PINNED_FILES, PINNED_JOBS } from "./pins.mjs";
import { read, tracked } from "./repo.mjs";
import { GATE_ONLY, LEFT_OUTSIDE, RUNNERS, SANDBOX_SOURCES, TESTING_JOBS, runnersOf } from "./sandbox.mjs";
import { allJobs, logicalLines, requiredJobs, stepKey, steps, workflows } from "./workflows.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 5;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of sandbox.test.mjs ran: a skip or a filter left the rest out`);
});
/// Every job whose code of the change runs in the sandbox on Linux is pinned whole: the testing
/// jobs, and the jobs that run the release tests through cargo alone. A rule that read their
/// steps line by line and no further let through a runner unset after its source, a check inside
/// `if false`, and cargo's shorter spellings (`cargo t`).
const SANDBOXED_JOBS = ["ci.yml#rust", "mutants.yml#plan", "mutants.yml#shard", "mutants-full.yml#full", "ci.yml#execution-paths", "generated.yml#paths"];
/// The steps of each sandboxed job that run code of the change, at the least, as the policy reads
/// them: fewer means the reading lost one. The plan of a pull request's mutation run has one, its
/// baseline, where each other job had two at the least.
const CODE_STEPS = new Map([
  ["ci.yml#rust", 6],
  ["mutants.yml#plan", 1],
  ["mutants.yml#shard", 2],
  ["mutants-full.yml#full", 2],
  ["ci.yml#execution-paths", 2],
  ["generated.yml#paths", 3],
]);

/// The step that sets up the sandbox the code of a change runs in on Linux, and proves it on that
/// runner with the canary: exactly these two commands, and, in a testing job, which records its
/// tools, the check of the tools after the canary, whose probe makes and removes a file among them
/// as the runner's user.
const SANDBOX_SETUP = "/usr/bin/bash .github/scripts/test-sandbox.sh setup\n/usr/bin/bash .github/scripts/test-sandbox.sh check\n";
const VERIFY_TOOLS = "/usr/bin/bash .github/scripts/tools.sh verify\n";
/// What a step that ran code of the change ends with, before the check of the tools: at least one
/// command ran in the sandbox, or, after cargo-mutants, at least one for each mutant caught.
const RAN = "/usr/bin/bash .github/scripts/test-sandbox.sh ran";
/// What the job's last step that runs code of the change holds right after its `ran`: the sandbox
/// added under a tenth to the job's time.
const BOUND = "/usr/bin/bash .github/scripts/test-sandbox.sh bound";
/// What is wrong with where a sandboxed job holds its bound: once, as BOUND, right after the `ran`
/// of its last step that runs code of the change, when every sandboxed command has been counted;
/// held earlier, the canary's fixed cost would outweigh the time so far.
export function boundPlacementProblems(key, jobSteps) {
  const bounds = [];
  let last = null;
  jobSteps.forEach((s, k) => {
    if (typeof s.run !== "string") return;
    const lines = logicalLines(s.run).map((l) => l.trim());
    lines.forEach((l, at) => {
      if (/test-sandbox\.sh bound\b/.test(l)) bounds.push({ k, at, l });
    });
    if (lines.some((l) => runsCode(l))) last = { k, lines };
  });
  if (last === null) return [`${key} runs no code of the change, so it holds no bound`];
  const ranAt = last.lines.findIndex((l) => l === RAN || l.startsWith(`${RAN} `));
  const held = bounds.length === 1 && bounds[0].k === last.k && bounds[0].l === BOUND && ranAt >= 0 && bounds[0].at === ranAt + 1;
  return held ? [] : [`${key} does not hold the sandbox's bound once, as \`${BOUND}\` right after the \`ran\` of its last step that runs code`];
}
/// A cap of the lints of cargo-mutants' own, on its line: the mutation runs take theirs from
/// limits.sh, for the baseline and every mutant alike, and a second cap would change the flags of
/// every mutant's build from the baseline's, which would then rebuild everything.
const MUTANTS_CAP = /\s--cap-lints\b/;
/// The argument, on each line that runs mutants, by which cargo-mutants has nextest forward cargo's
/// JSON messages, rustc's diagnostics among them, into each mutant's build log, where the floor reads
/// them (the review of 75e5eeb: rustc's rendering for people placed an error where it was not).
const MUTANTS_JSON = " --cargo-arg=--cargo-message-format=json ";
/// Whether a line of a step runs code of the change: tests through nextest or cargo, under
/// `timeout` or not, nextest's list, which runs every test executable, a binary through
/// `cargo run`, mutants, and the proof of the overflow checks, which runs a test of every crate. A
/// build (`--no-run`) and a list of mutants or of doctests run none.
const runsCode = (line) =>
  /^(timeout \S+ )?(cargo-nextest nextest (run|list)\b|cargo nextest run\b|cargo run\b|cargo mutants (?!.*--list)|cargo test (?!.*(--no-run|--doc\b.*--list))|bash \.github\/scripts\/overflow-proof\.sh$)/.test(line);
/// The lines of cargo that run no code of the change: the formatter, the lints, the install of
/// cargo-mutants, a build of the tests, and the lists of doctests and of mutants.
const BUILDS_ONLY = /^cargo (fmt|clippy|install cargo-mutants|test [^|;&]*--no-run|test [^|;&]*--doc -- --list|mutants [^|;&]*--list)\b/;
/// Whether a line of a pinned job could start code of the change in a form the policy does not
/// read. The pin holds every line of these jobs, and a pin is rewritten only with the code
/// owner's approval; this rule names, for that review, the commonest ways past the sandbox: cargo
/// in any form but those above (its shorter spellings `cargo t` and `cargo r`, `cargo bench`, an
/// option before its subcommand, a command after a variable or inside another), a `--config` on
/// cargo's own line, which can name a runner or a wrapper, the proof of the overflow checks with
/// anything after it, a path into target/, where the built binaries are, cargo's runner named or
/// unset, and the count of sandboxed commands written by hand.
const runsCodeUnread = (line) => {
  if (line.startsWith("#")) return false;
  const cargo = /(^|[\s;&|(])cargo(-nextest)?\s/.test(line);
  if (cargo && !(runsCode(line) || BUILDS_ONLY.test(line))) return true;
  if (cargo && /\s--config[\s=]/.test(line)) return true;
  if (/overflow-proof\.sh/.test(line) && !runsCode(line)) return true;
  return /(^|[^\w.-])target\//.test(line) || /CARGO_TARGET_|_RUNNER\b|test-sandbox-(runs\.log|cost)\b/.test(line);
};

check("on Linux the testing jobs prove the tests' sandbox first, and every step that runs code of the change ran it there", () => {
  assert.deepEqual([...CODE_STEPS.keys()], SANDBOXED_JOBS, "CODE_STEPS holds other jobs than SANDBOXED_JOBS");
  for (const key of SANDBOXED_JOBS) {
    const [file, id] = key.split("#");
    const w = workflows().find((x) => x.file === file);
    assert.ok(w?.doc.jobs?.[id], `${file} has no job ${id}`);
    const job = w.doc.jobs[id];
    const all = steps(job);
    const bash = (s) => {
      const shell = s.shell ?? job.defaults?.run?.shell ?? w.doc.defaults?.run?.shell;
      return shell === "bash" || (shell === undefined && /^ubuntu-/.test(String(job["runs-on"])));
    };
    // One step sets the sandbox up and proves it. In a testing job it is the step right after the
    // tools are recorded, before the tests are built, and so before any runs.
    const naming = all.flatMap((s, k) => (/test-sandbox\.sh (setup|check)\b/.test(String(s.run ?? "")) ? [k] : []));
    assert.equal(naming.length, 1, `${key}: ${naming.length} steps set up or prove the sandbox, not one`);
    const sandbox = all[naming[0]];
    const where = stepKey(file, id, sandbox);
    assert.equal(sandbox.run, TESTING_JOBS.includes(key) ? SANDBOX_SETUP + VERIFY_TOOLS : SANDBOX_SETUP, `${where} does not set up and prove the sandbox as SANDBOX_SETUP in .github/scripts/policy/sandbox.test.mjs reads it`);
    assert.deepEqual(Object.keys(sandbox).filter((k) => k !== "if").sort(), ["name", "run"], `${where} carries more than its name, its script and the watch's condition`);
    assert.ok(bash(sandbox), `${where} does not run in bash`);
    if (TESTING_JOBS.includes(key)) {
      const record = all.findIndex((s) => String(s.run ?? "").trim() === "/usr/bin/bash .github/scripts/tools.sh record");
      assert.ok(record >= 0 && naming[0] === record + 1, `${key}: the sandbox is not set up and proven right after the tools are recorded`);
    }
    let running = 0;
    for (const [k, s] of all.entries()) {
      if (typeof s.run !== "string") continue;
      const lines = logicalLines(s.run).map((l) => l.trim());
      for (const l of lines) assert.ok(!runsCodeUnread(l), `${stepKey(file, id, s)} starts code of the change in a form the policy does not read, which cargo's runner may never see: ${l}`);
      const runs = lines.flatMap((l, at) => (runsCode(l) ? [at] : []));
      if (runs.length === 0) continue;
      running++;
      const step = stepKey(file, id, s);
      assert.ok(k > naming[0], `${step} runs code of the change before the sandbox is set up and proven`);
      const source = lines.findIndex((l) => SANDBOX_SOURCES.includes(l));
      assert.ok(source >= 0 && source < runs[0], `${step} runs code of the change before it sources the limits or test-sandbox-env.sh, and so outside the sandbox: ${lines[runs[0]]}`);
      assert.ok(bash(s), `${step} sources the sandbox's settings in a shell that is not bash`);
      const mutants = lines.some((l) => /^cargo mutants (?!.*--list)/.test(l));
      // Every run of mutants caps the lints at warnings, so that a lint the change denies cannot
      // make a mutant fail to build, and count as unviable, not missed: limits.sh exports the cap
      // under the mutation runs' profile, for the baseline and every mutant alike, so the step
      // sources limits.sh, the workflow sets that profile, and cargo-mutants adds no cap of its
      // own (docs/design-review.md, untrusted-test-isolation).
      if (mutants) {
        assert.ok(lines.includes(". .github/scripts/limits.sh"), `${step} runs mutants without sourcing limits.sh, which caps the lints`);
        assert.equal(w.doc.env?.NEXTEST_PROFILE, "mutants", `${file} runs mutants outside the mutation runs' profile, under which limits.sh caps the lints`);
        for (const l of lines.filter((x) => /^cargo mutants (?!.*--list)/.test(x))) {
          assert.doesNotMatch(l, MUTANTS_CAP, `${step} gives cargo-mutants a cap of the lints of its own, beside limits.sh's: ${l}`);
          // The floor's evidence reads rustc's JSON diagnostics in each mutant's build log, which
          // nextest forwards only when told to (.github/scripts/lib/mutants-floor.mjs).
          assert.equal(l.split(MUTANTS_JSON).length, 2, `${step} runs mutants without forwarding rustc's JSON diagnostics to their logs, once: ${l}`);
        }
      }
      const ran = lines.flatMap((l, at) => (l === RAN || l.startsWith(`${RAN} `) ? [at] : []));
      assert.ok(ran.length === 1 && ran[0] > runs.at(-1), `${step} runs code of the change and does not check, once and after it, that it ran in the sandbox`);
      assert.equal(lines[ran[0]], mutants ? `${RAN} "$caught"` : RAN, `${step} checks other than that ${mutants ? "each mutant caught" : "a command"} ran in the sandbox`);
    }
    assert.ok(running >= CODE_STEPS.get(key), `${key}: the policy reads ${running} steps that run code of the change, fewer than CODE_STEPS holds`);
    assert.deepEqual(boundPlacementProblems(key, all), []);
  }
  // The planted cases: the bound missing, twice, spelled otherwise, before the `ran`, after the
  // check of the tools, or in a step before the last that runs code.
  const tests = ". .github/scripts/limits.sh\ncargo-nextest nextest run --locked --workspace --profile ci";
  const last = { run: `${tests}\n${RAN}\n${BOUND}\n${VERIFY_TOOLS}` };
  const earlier = { run: `${tests}\n${RAN}\n${VERIFY_TOOLS}` };
  assert.deepEqual(boundPlacementProblems("x.yml#y", [earlier, last]), []);
  for (const [what, list] of [
    ["no bound", [earlier, { run: `${tests}\n${RAN}\n${VERIFY_TOOLS}` }]],
    ["two bounds", [{ run: `${tests}\n${RAN}\n${BOUND}` }, last]],
    ["a bound with an argument", [earlier, { run: `${tests}\n${RAN}\n${BOUND} 0` }]],
    ["a bound before the ran", [earlier, { run: `${tests}\n${BOUND}\n${RAN}` }]],
    ["a bound after the check of the tools", [earlier, { run: `${tests}\n${RAN}\n${VERIFY_TOOLS}${BOUND}` }]],
    ["a bound in an earlier step", [{ run: `${tests}\n${RAN}\n${BOUND}` }, earlier]],
  ]) {
    assert.ok(boundPlacementProblems("x.yml#y", list).length > 0, `the bound's place lets through ${what}`);
  }
  for (const line of ['cargo mutants --in-place --cap-lints=true --timeout 600 || status=$?', "cargo mutants --in-place --cap-lints=false", "cargo mutants --cap-lints true"]) assert.match(line, MUTANTS_CAP, line);
  assert.doesNotMatch('cargo mutants --in-diff pr.diff --in-place --timeout 600 || status=$?', MUTANTS_CAP);
  for (const [line, runs] of [
    ["cargo-nextest nextest run --locked --workspace --profile ci", true],
    ["cargo-nextest nextest list --locked --workspace --run-ignored all --message-format json > x", true],
    ["cargo nextest run --workspace 2>&1 | tee baseline.log", true],
    ["cargo test --locked -p mente-cli --test gates", true],
    ["timeout 15m cargo test --release --locked -p mente-cli -- --ignored matches_the_reference_interpreter", true],
    ["cargo run --locked --bin mn -- spec", true],
    ['cargo mutants "${shard[@]}" --in-place || status=$?', true],
    ["bash .github/scripts/overflow-proof.sh", true],
    ["cargo test --locked --workspace --no-run", false],
    ["cargo test --locked --workspace --doc -- --list > x 2>&1", false],
    ["cargo mutants --in-diff pr.diff --list --json > diff-mutants.json", false],
    ["cargo fmt --all --check", false],
  ]) {
    assert.equal(runsCode(line), runs, line);
    assert.ok(!runsCodeUnread(line), `a line the policy reads counts as unread: ${line}`);
  }
  for (const [line, unread] of [
    ["(cd ../base && timeout 15m cargo test --release --locked -p mente-cli -- --ignored x)", true],
    ["MENTE_X=1 cargo test --locked", true],
    ["if cargo-nextest nextest run; then echo passed; fi", true],
    ["bash .github/scripts/overflow-proof.sh || true", true],
    ["./target/release/mn smt examples --out x", true],
    ["target/release/mn fuzz --seed 1", true],
    ["status=0; target/debug/mn spec || status=$?", true],
    ["mn=target/debug/mn", true],
    ["cargo t --release -p mente-cli", true],
    ["cargo --locked test --release -p mente-cli", true],
    ["cargo bench --locked", true],
    ["cargo r --locked --bin mn -- spec", true],
    ['cargo build --release --locked && "./target/release/mn" spec', true],
    ['"$PWD/target/release/mn" spec', true],
    ["cd target/release && ./mn spec", true],
    ["timeout 15m cargo test --config \"target.x86_64-unknown-linux-gnu.runner='env'\" --release --locked -p mente-cli", true],
    ["unset CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER", true],
    ['echo x >> "$RUNNER_TEMP/test-sandbox-runs.log"', true],
    ['echo "added 0" >> "$RUNNER_TEMP/test-sandbox-cost"', true],
    ['if git show --no-textconv "origin/${GITHUB_BASE_REF}:.cargo/mutants.toml" > base-mutants.toml 2>/dev/null; then', false],
    ['cargo mutants --list --json "${config[@]}" > base-all-mutants.json', false],
    ['node .github/scripts/test-list.mjs nextest "$RUNNER_TEMP/tests.json"', false],
    ['cargo install cargo-mutants --locked --git https://github.com/sourcefrog/cargo-mutants --rev "$CARGO_MUTANTS_REV"', false],
    ['cargo install cargo-mutants --locked --git https://github.com/sourcefrog/cargo-mutants --rev "$CARGO_MUTANTS_REV"', false],
    ["cargo clippy --all-targets --workspace --locked -- -D warnings", false],
    ["# cargo test runs here", false],
    ["rustup toolchain install", false],
  ]) {
    assert.equal(runsCodeUnread(line), unread, line);
  }
  // limits.sh sources test-sandbox-env.sh on Linux, last, and that file takes cargo's runner from
  // the sandbox's script and stops when there is no sandbox.
  assert.match(
    read(".github/scripts/limits.sh").replace(/\r\n/g, "\n"),
    /\n  Linux\)\n(?:(?!\n    ;;\n)[\s\S])*\n    \. \.github\/scripts\/test-sandbox-env\.sh\n    ;;\n/,
    "limits.sh does not source test-sandbox-env.sh on Linux",
  );
  assert.match(
    read(".github/scripts/test-sandbox-env.sh").replace(/\r\n/g, "\n"),
    /\nif ! test_sandbox_env=\$\(\/usr\/bin\/bash \.github\/scripts\/test-sandbox\.sh env\); then\n  echo "\$test_sandbox_env"\n  exit 1\nfi\neval "\$test_sandbox_env"\n/,
    "test-sandbox-env.sh does not take cargo's runner from test-sandbox.sh env, or goes on without it",
  );
  // A trap on EXIT runs when the script ends, after every function has returned and its local
  // variables are gone. One that reads them then fails, under `set -u`, a canary that had passed,
  // as the first such trap did on every runner. So in the pinned scripts a trap on EXIT names what
  // it needs when it is set, in double quotes.
  const lateExitTrap = /^\s*trap '[^'\n]*\$[^'\n]*' EXIT\b/m;
  for (const f of [...PINNED_FILES.keys()].filter((f) => f.endsWith(".sh"))) {
    assert.doesNotMatch(read(f), lateExitTrap, `${f} sets a trap on EXIT that reads its variables only when the script ends, where a function's are gone`);
  }
  assert.match(`  trap 'kill "$decoy" "$listener" 2> /dev/null || true' EXIT`, lateExitTrap);
  assert.doesNotMatch('  trap "kill $decoy $listener 2> /dev/null || true" EXIT', lateExitTrap);
  assert.doesNotMatch("  trap 'signal=15' TERM", lateExitTrap);
});

check("the sandbox's script, its one program and the floor name the one sandbox: its home, root, user and program", () => {
  // test-sandbox.sh sets the sandbox up and names cargo's runner; test-sandbox.rs is that runner and
  // the namespace's first process; the floor reads nextest's line that names it. A value that
  // differed between them would point cargo at no program, the program at a root or a user setup
  // did not make, or the floor at a runner that no longer runs.
  const sh = read(".github/scripts/test-sandbox.sh").replace(/\r\n/g, "\n");
  const rs = read(".github/scripts/test-sandbox.rs").replace(/\r\n/g, "\n");
  const shell = (name) => {
    const m = new RegExp(`^${name}=(\\S+)$`, "m").exec(sh);
    assert.ok(m, `test-sandbox.sh sets no ${name}`);
    return m[1].replace(/^\$home(?=\/)/, home);
  };
  const rust = (name) => {
    const m = new RegExp(`^const ${name}: [^=]+ = "?([^";]+)"?;$`, "m").exec(rs);
    assert.ok(m, `test-sandbox.rs has no constant ${name}`);
    return m[1];
  };
  const home = /^home=(\/\S+)$/m.exec(sh)?.[1];
  assert.ok(home, "test-sandbox.sh sets no home of the sandbox");
  assert.equal(rust("HOME"), home, "the program's home of the sandbox is not the script's");
  assert.equal(rust("ROOT"), shell("root"), "the program's root of the sandbox is not the script's");
  assert.equal(rust("UID"), shell("uid"), "the program runs the command as another user than the script's");
  assert.equal(rust("GID"), shell("gid"), "the program runs the command in another group than the script's");
  assert.equal(rust("PROGRAM"), shell("program"), "the program starts another program as the namespace's first process than the one setup installs");
  assert.equal(SANDBOX_PROGRAM, shell("program"), "the floor reads nextest's line for another runner than the sandbox's program");
  // cargo's runner, as `env` names it on both Linux targets, is that program's `run`; the wrapper
  // starts that program's `inner` through sudo and unshare.
  for (const triple of ["X86_64_UNKNOWN_LINUX_GNU", "AARCH64_UNKNOWN_LINUX_GNU"]) {
    assert.ok(sh.includes(`  printf "export CARGO_TARGET_${triple}_RUNNER='%s run'\\n" "$program"\n`), `test-sandbox.sh env does not name the program's run as cargo's runner for ${triple}`);
  }
  assert.ok(rs.includes('"--kill-child=SIGKILL", "--", PROGRAM, "inner",'), "the program's wrapper does not start the program's own inner in the namespace");
});

check("every required job on Linux runs the change's code in the sandbox, or says why it does not", () => {
  const lists = [
    ["SANDBOXED_JOBS", new Set(SANDBOXED_JOBS)],
    ["LEFT_OUTSIDE", new Set(LEFT_OUTSIDE.keys())],
    ["GATE_ONLY", new Set(GATE_ONLY.keys())],
  ];
  const required = new Set();
  for (const { file, id, job } of requiredJobs()) {
    const key = `${file}#${id}`;
    required.add(key);
    const runners = runnersOf(job);
    assert.ok(runners && runners.every((r) => RUNNERS.has(r)), `${key} runs on ${JSON.stringify(job["runs-on"])}, which is no literal of RUNNERS nor a matrix value that is one`);
    const linux = runners.some((r) => r.startsWith("ubuntu-"));
    const holding = lists.filter(([, set]) => set.has(key)).map(([name]) => name);
    if (!linux) {
      assert.deepEqual(holding.filter((n) => n !== "SANDBOXED_JOBS"), [], `${key} runs on no Linux runner, and is classified for one in ${holding.join(", ")}`);
      continue;
    }
    assert.equal(holding.length, 1, `${key}, a required job on a Linux runner, is in ${holding.length === 0 ? "none" : holding.join(" and ")} of SANDBOXED_JOBS, LEFT_OUTSIDE and GATE_ONLY in .github/scripts/policy/sandbox.test.mjs and sandbox.mjs, not exactly one: it says, with its reason, whether its code of the change runs in the sandbox, outside it, or not at all`);
    if (GATE_ONLY.has(key)) assert.ok(PINNED_JOBS.has(key), `${key}, a job of GATE_ONLY, is not pinned whole: its exact steps are what it may run`);
  }
  for (const [name, set] of lists) {
    for (const key of set) assert.ok(required.has(key) || (name === "SANDBOXED_JOBS" && key === "mutants-full.yml#full") || (name === "GATE_ONLY" && PINNED_JOBS.has(key)), `${name} names ${key}, which is no required job${name === "GATE_ONLY" ? " and no pinned one" : ""}`);
  }
  // Nothing of the change lies under target/, where the tests are built: root mounts the tests'
  // own directory there, and refuses to through a link the change tracks (test-sandbox.sh); this
  // refuses the tracked file before any job runs.
  const underTarget = [...tracked()].filter((f) => f === "target" || f.startsWith("target/"));
  assert.deepEqual(underTarget, [], "files under target/ are tracked: the sandbox mounts the tests' own directory there, and a tracked link could aim it elsewhere");
  // The runners a job names, as the classification reads them: the third review's P7c, a label
  // spelled by an expression, is no runner at all.
  assert.deepEqual(runnersOf({ "runs-on": "ubuntu-24.04" }), ["ubuntu-24.04"]);
  assert.deepEqual(runnersOf({ "runs-on": "${{ matrix.os }}", strategy: { matrix: { include: [{ os: "ubuntu-24.04" }, { os: "windows-2025" }] } } }), ["ubuntu-24.04", "windows-2025"]);
  for (const on of ["${{ format('{0}-24.04', 'ubuntu') }}", ["ubuntu-24.04"], { group: "x" }]) assert.equal(runnersOf({ "runs-on": on }), null, JSON.stringify(on));
  assert.equal(runnersOf({ "runs-on": "${{ matrix.os }}", strategy: { matrix: "${{ fromJSON('[]') }}" } }), null);
});

check("every measured benchmark run proves its sandbox first", () => {
  let measured = 0;
  for (const { file, id, job } of allJobs()) {
    const all = steps(job);
    const run = all.findIndex((s) => /benchmark\/harness\/bench\.ts/.test(String(s.run ?? "")));
    if (run < 0) continue;
    measured++;
    const check = all.findIndex((s) => /benchmark\/harness\/sandbox-check\.ts/.test(String(s.run ?? "")));
    assert.ok(check >= 0 && check < run, `${file}, job ${id}: the benchmark runs agent code before sandbox-check.ts proves the sandbox`);
    for (const key of ["if", "continue-on-error"]) {
      assert.ok(!(key in all[check]), `${file}, job ${id}: the sandbox check carries ${key}:, so a run could measure without it`);
    }
  }
  assert.ok(measured > 0, "no workflow runs benchmark/harness/bench.ts");
});

/// The sandbox every job that confines agent code names, and nothing else may change.
const SANDBOX = "benchmark/harness/sandbox.sh";

check("every job that confines agent code names the one sandbox", () => {
  let confining = 0;
  for (const { file, doc, id, job } of allJobs()) {
    if (!steps(job).some((s) => /--sandbox "\$SANDBOX"/.test(String(s.run ?? "")))) continue;
    confining++;
    const where = `${file}, job ${id}`;
    assert.equal(job.env?.SANDBOX, SANDBOX, `${where}: SANDBOX is not ${SANDBOX}: an empty one runs agent code unconfined`);
    assert.ok(!(doc.env && "SANDBOX" in doc.env), `${file} sets SANDBOX for every job`);
    for (const s of steps(job)) {
      assert.ok(!(s.env && "SANDBOX" in s.env), `${stepKey(file, id, s)} sets its own SANDBOX`);
      assert.doesNotMatch(String(s.run ?? ""), /\bSANDBOX=/, `${stepKey(file, id, s)} writes SANDBOX for the steps after it`);
    }
  }
  assert.ok(confining >= 3, "the benchmark, the preliminary runs and the harness confine agent code");
});
