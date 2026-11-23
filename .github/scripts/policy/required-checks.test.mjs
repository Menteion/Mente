// The workflow policy: the required checks (README.md in this directory). Every check main requires
// is one job that runs everywhere and cannot pass without checking.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { FACTS, read } from "./repo.mjs";
import { SANDBOX_SOURCES, TESTING_JOBS, runnersOf } from "./sandbox.mjs";
import { CLEAN_TREE } from "./toolchain.mjs";
import { REPORT_FORMS, REQUIRED_EVENTS, REQUIRED_FILE, WATCH_FORMS, WORKFLOWS, allJobs, hasWatch, isPlain, isReport, logicalLines, needsOf, normalize, requiredChecks, requiredJobs, scriptTexts, stepKey, steps, triggers, workflows } from "./workflows.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 11;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of required-checks.test.mjs ran: a skip or a filter left the rest out`);
});
/// The one job-level condition a required job may carry: it holds on both events a required check
/// reports on, so the job is never skipped there.
const REQUIRED_JOB_IF = "github.event_name == 'pull_request' || github.event_name == 'merge_group'";
/// The conditions steps of required jobs without a watch may carry, each by workflow, job and step.
const STEP_CONDITIONS = new Map([
  // One platform formats and lints for all three.
  ["ci.yml#rust#rustfmt", "matrix.os == 'ubuntu-24.04'"],
  ["ci.yml#rust#clippy", "matrix.os == 'ubuntu-24.04'"],
  // Windows' runner has cdb; Linux's needs gdb installed.
  ["ci.yml#debuggers#Install gdb", "runner.os == 'Linux'"],
  // A shard keeps its outcomes and logs whether it passed or not.
  ["mutants.yml#shard#Keep this shard's outcomes and logs", "always()"],
]);
/// The steps of required jobs that may fail without failing the job, each with its reason.
const CONTINUE_ON_ERROR = new Map([
  // x86-64's runner exposes no counters, so its probe fails there; AArch64's job requires the
  // counters through perf/run.mjs --require-instructions instead.
  ["perf.yml#kernels#Install perf and let it count", "the counters are required by run.mjs where they exist"],
]);

check("the required checks are listed once each", () => {
  const names = requiredChecks();
  assert.ok(names.length > 0);
  const twice = names.filter((n, k) => names.indexOf(n) !== k);
  assert.deepEqual(twice, [], `${REQUIRED_FILE} lists these twice`);
});

/// The jobs that run on pull requests, or on pushes to a branch other than main, and are no required
/// check, each with why it gates nothing. A check that runs on a pull request or a branch and is not
/// required looks like a gate and is not one: "Build the site" ran on pull requests through a
/// `paths:` filter for weeks, which kept it from being required, so a change that broke the site
/// could merge. Read job by job, so that a job beside a required one of its workflow is held too,
/// and with a reason each (the review of 7ba7a5c).
const ADVISORY_JOBS = new Map([
  ["ci.yml#coverage", "a report of line coverage, in the run summary and as lcov, of the tests the required Rust jobs run as the gate"],
  ["spike-cranelift.yml#check", "a spike in a workspace of its own (spikes/cranelift-debug-info), whose findings are data for the register; ci.yml's Debuggers checks are the gate"],
  ["spike-error-return.yml#check", "a spike in a workspace of its own (spikes/error-return-registers), whose findings are data for the register"],
  ["spike-tier1.yml#measure", "the Tier-1 latency spike's measurement, data for the register; ci.yml's Tier-1 latency is the gate, and the Rust jobs run the spike's tests"],
]);

/// Whether triggers `on` run a workflow on a pull request, or on a push to a branch other than
/// main: a push with no filter, with paths alone, or with branches other than main exactly. A push
/// of tags alone runs on no branch.
export function runsBeyondMain(on) {
  if ("pull_request" in on || "pull_request_target" in on) return true;
  if (!("push" in on)) return false;
  const push = on.push;
  if (!isPlain(push)) return true;
  const branches = "branches" in push || "branches-ignore" in push;
  if (!branches) return !("tags" in push || "tags-ignore" in push);
  return !(Array.isArray(push.branches) && push.branches.length === 1 && push.branches[0] === "main" && !("branches-ignore" in push));
}

/// Why the jobs `jobs` ({ file, id, on }) are not each a required check, one of `required`
/// ("file#id"), or one `advisory` names with a reason, when they run beyond main; and why
/// `advisory` names what it should not.
export function advisoryProblems(jobs, required, advisory) {
  const found = [];
  const keys = new Set();
  for (const { file, id, on } of jobs) {
    const key = `${file}#${id}`;
    keys.add(key);
    const beyond = runsBeyondMain(on);
    const listed = advisory.has(key);
    const reason = advisory.get(key);
    if (beyond && !required.has(key) && !listed) found.push(`${key} runs on pull requests or on a branch other than main, and is no required check: make it one, or name it in ADVISORY_JOBS with why it gates nothing`);
    if (listed && (typeof reason !== "string" || reason.trim() === "")) found.push(`${key} is in ADVISORY_JOBS with no reason`);
    if (listed && required.has(key)) found.push(`${key} is in ADVISORY_JOBS, and is a required check`);
    if (listed && !beyond) found.push(`${key} is in ADVISORY_JOBS, and runs on no pull request and no branch other than main`);
  }
  for (const key of advisory.keys()) if (!keys.has(key)) found.push(`ADVISORY_JOBS names ${key}, which is no job`);
  return found;
}

check("every job that runs on pull requests or on a branch other than main is a gate, or says why it gates nothing", () => {
  const jobs = allJobs().map(({ file, doc, id }) => ({ file, id, on: triggers(doc) }));
  const required = new Set(requiredJobs().map(({ file, id }) => `${file}#${id}`));
  assert.deepEqual(advisoryProblems(jobs, required, ADVISORY_JOBS), []);
  assert.ok(required.has("pages.yml#build"), "the site's build is no required check");
  const refused = (list, req = required, advisory = ADVISORY_JOBS) => advisoryProblems(list, req, advisory);
  const pages = jobs.find((j) => j.file === "pages.yml" && j.id === "build");
  // Planted: the site's build as it was, required by nothing; a job beside it in its required
  // workflow; a pull_request_target workflow; and each push that reaches a branch other than main.
  assert.ok(refused(jobs, new Set([...required].filter((k) => k !== "pages.yml#build"))).some((p) => p.startsWith("pages.yml#build runs on")), "the site's build, required by nothing, passes");
  assert.ok(refused([...jobs, { ...pages, id: "extra" }]).some((p) => p.startsWith("pages.yml#extra runs on")), "a job beside a required one passes");
  assert.ok(refused([{ file: "x.yml", id: "j", on: { pull_request_target: null } }]).some((p) => p.startsWith("x.yml#j runs on")), "a job on pull_request_target passes");
  for (const push of [null, { paths: ["src/**"] }, { branches: ["main", "dev"] }, { branches: ["*"] }, { "branches-ignore": ["main"] }, { branches: ["main"], "branches-ignore": ["x"] }, { tags: ["m*"], branches: ["dev"] }]) {
    assert.ok(refused([{ file: "x.yml", id: "j", on: { push } }]).some((p) => p.startsWith("x.yml#j runs on")), `a job on push ${JSON.stringify(push)} passes`);
  }
  // What is no gate's business: main's pushes, tags alone, a schedule, a run by hand.
  for (const on of [{ push: { branches: ["main"] } }, { push: { tags: ["m*"] } }, { schedule: [{ cron: "0 0 * * *" }] }, { workflow_dispatch: null }]) {
    assert.deepEqual(refused([{ file: "x.yml", id: "j", on }], required, new Map()), [], JSON.stringify(on));
  }
  // A reason that is empty or blank; a listed job that is required, that runs on no pull request,
  // or that is no job.
  for (const reason of ["", "   ", undefined]) {
    assert.ok(refused(jobs, required, new Map([...ADVISORY_JOBS, ["spike-tier1.yml#measure", reason]])).includes("spike-tier1.yml#measure is in ADVISORY_JOBS with no reason"), `the reason ${JSON.stringify(reason)} passes`);
  }
  assert.ok(refused(jobs, new Set([...required, "spike-tier1.yml#measure"])).includes("spike-tier1.yml#measure is in ADVISORY_JOBS, and is a required check"), "a listed required check passes");
  const byHand = jobs.map((j) => (j.file === "spike-tier1.yml" ? { ...j, on: { workflow_dispatch: null } } : j));
  assert.ok(refused(byHand).includes("spike-tier1.yml#measure is in ADVISORY_JOBS, and runs on no pull request and no branch other than main"), "a listed job that runs on no pull request passes");
  assert.ok(refused(jobs, required, new Map([...ADVISORY_JOBS, ["gone.yml#j", "x"]])).includes("ADVISORY_JOBS names gone.yml#j, which is no job"), "a listed name of no job passes");
});

check("a workflow that holds a required job runs only on pull requests, the queue and main", () => {
  for (const file of new Set(requiredJobs().map((j) => j.file))) {
    const on = triggers(workflows().find((w) => w.file === file).doc);
    for (const event of Object.keys(on)) assert.ok(REQUIRED_EVENTS.has(event), `${file} runs on ${event}, beside a required job`);
    if ("push" in on) assert.deepEqual(on.push, { branches: ["main"] }, `${file} runs on pushes other than to main`);
    assert.doesNotMatch(read(`${WORKFLOWS}/${file}`), /\binputs\.|github\.event\.inputs/, `${file} reads inputs`);
  }
});

/// The commands each required job exists to run (.github/gate-commands.txt), by workflow and job.
function gateCommands() {
  const out = new Map();
  for (const [n, raw] of read(".github/gate-commands.txt").split(/\r?\n/).entries()) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    const m = /^([\w.-]+\.yml)#([\w-]+): (.+)$/.exec(line);
    assert.ok(m, `.github/gate-commands.txt:${n + 1}: not <workflow>#<job>: <command>`);
    const key = `${m[1]}#${m[2]}`;
    if (!out.has(key)) out.set(key, []);
    assert.ok(!out.get(key).includes(m[3]), `.github/gate-commands.txt:${n + 1}: twice`);
    out.get(key).push(m[3]);
  }
  return out;
}

/// The gate commands that run inside a compound command of their step, each with its reason: none
/// since the plan of "Mutants in the diff" checks the floor of the whole diff, which shard 0 alone
/// did inside an `if`.
const NESTED_GATES = new Map([]);

/// Why a step's script could pass though a gate command in it failed, or run it other than as
/// declared: no line of the script is the command exactly (so nothing is added to it, no argument,
/// no `|| true`, no `&&`, and it is neither commented out nor negated); the line is nested in a
/// compound command (rustfmt-like indentation: a top-level command starts its line); the command
/// itself is a list whose failure the shell forgives; it holds a pipe in a shell without pipefail,
/// which GitHub's bash has only when a step names it (`shell: bash`, here or in the defaults); or
/// the step turns errexit or pipefail off. The one suffix allowed is `|| status=$?`, with
/// `exit "$status"` on a later line of the script.
export function gateLineProblems(script, command, pipefail, nested = false) {
  const found = [];
  const text = String(script);
  if (/\bset\s+(\+[a-z]*e[a-z]*\b|\+o\s+(errexit|pipefail)\b)/.test(text)) found.push("the script turns off errexit or pipefail");
  if (/\bset\s+-[a-z]*o\s+pipefail\b/.test(text)) pipefail = true;
  if (/\|\||&&|;|&\s*$/.test(command)) found.push(`${command} is itself a list whose failure the shell forgives`);
  const lines = logicalLines(text);
  const holding = lines.flatMap((l, k) => (l.trim() === command || l.trim() === `${command} || status=$?` ? [[l, k]] : []));
  if (holding.length === 0) found.push(`no line of the script is exactly ${command}`);
  // Before the command, nothing that could end the script, or change how it ends, while the step
  // still passed: an exit with no failing literal, return, exec, a trap, a kill, eval, builtin or
  // command (which run any word), a source of anything but the shared limits, an alias or shopt,
  // or a function named as the command's first word (the fourth review of PR 137: `exit 0` before
  // a gate command passed).
  const first = holding.length > 0 ? holding[0][1] : lines.length;
  const word = command.split(/\s+/)[0].replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const start = String.raw`(?:^|[;&|({]\s*|\b(?:then|do|else|elif|if|while|until)\s+|!\s+)`;
  for (const line of lines.slice(0, first).map((l) => l.trim())) {
    if (line === "" || line.startsWith("#")) continue;
    if (new RegExp(`${start}exit(?!\\s+[1-9][0-9]*\\s*(?:[;&|)}]|$))\\b`).test(line)) found.push(`it may exit before ${command}: ${line}`);
    if (new RegExp(`${start}(return|exec|trap|kill|eval|builtin|command|source|alias|shopt|enable)\\b`).test(line)) found.push(`it runs a word that could end or bend the script before ${command}: ${line}`);
    if (new RegExp(`${start}\\.\\s`).test(line) && !SANDBOX_SOURCES.includes(line)) found.push(`it sources a script before ${command}: ${line}`);
    if (new RegExp(`(?:^|[;&|({]\\s*)(?:function\\s+${word}\\b|${word}\\s*\\(\\s*\\))`).test(line)) found.push(`it defines a function named ${command.split(/\s+/)[0]} before it`);
  }
  for (const [line, k] of holding) {
    if (line.trim() !== command && !lines.slice(k + 1).some((l) => /\bexit "\$status"/.test(l))) found.push(`keeps the status of ${command} and never exits with it`);
    if (!nested && /^\s/.test(line)) found.push(`${command} is nested in a compound command`);
    if (!pipefail && /(^|[^|])\|([^|]|$)/.test(command)) found.push(`${command} is piped in a shell without pipefail`);
  }
  return found;
}

check("every required job runs the commands it exists for, in steps that cannot be skipped", () => {
  const declared = gateCommands();
  const required = requiredJobs();
  const requiredNames = new Set(requiredChecks());
  for (const key of declared.keys()) {
    assert.ok(required.some(({ file, id }) => `${file}#${id}` === key), `.github/gate-commands.txt names ${key}, which is no required job`);
  }
  for (const { file, doc, id, job, names } of required) {
    const key = `${file}#${id}`;
    // An aggregate, a job of a required check's name that needs others, runs no command of its
    // own: aggregateProblems holds its one step. The jobs it needs run theirs.
    if (needsOf(job).length > 0 && names.some((n) => requiredNames.has(n))) {
      assert.ok(!declared.has(key), `${key}, an aggregate, is named in .github/gate-commands.txt`);
      continue;
    }
    const commands = declared.get(key) ?? [];
    assert.ok(commands.length > 0, `${key} is a required job with no command in .github/gate-commands.txt`);
    const releases = scriptTexts(doc, job).some((t) => /\bcargo\s+(build|test|run)\b[^\n]*--release/.test(t));
    if (releases) assert.ok(commands.includes("bash .github/scripts/overflow-proof.sh"), `${key} builds in release without proving its overflow checks`);
    for (const command of commands) {
      const runs = steps(job).filter((s) => typeof s.run === "string" && logicalLines(s.run).some((l) => l.trim() === command || l.trim() === `${command} || status=$?`));
      assert.ok(runs.length > 0, `${key}: no step runs exactly ${command}`);
      for (const s of runs) {
        const where = `${stepKey(file, id, s)}, which runs ${command}`;
        assert.ok(!("continue-on-error" in s), `${where}, may fail quietly`);
        const shell = [s.shell, job.defaults?.run?.shell, doc.defaults?.run?.shell].find((x) => x !== undefined);
        // bash, named or the default of a Linux runner: pwsh ends with the last native command's
        // code, so a line after the gate command could decide the step (the fourth review).
        assert.ok(shell === "bash" || (shell === undefined && (runnersOf(job) ?? []).every((r) => r.startsWith("ubuntu-"))), `${where}, runs in ${shell ?? "the runner's default shell"}, not bash`);
        const pipefail = shell === "bash";
        assert.deepEqual(gateLineProblems(s.run, command, pipefail, NESTED_GATES.has(`${key}: ${command}`)), [], `${where}: its failure could pass the step`);
        const condition = normalize(s.if ?? "");
        const allowed = hasWatch(job) ? WATCH_FORMS.includes(condition) : !("if" in s);
        assert.ok(allowed, `${where}, runs only if ${s.if}`);
      }
    }
  }
  // What the line rule refuses, and what it lets through.
  const verdict = (script, command = "gate --x", pipefail = true) => gateLineProblems(script, command, pipefail).length > 0;
  for (const script of [
    "gate --x || true\n",
    "gate --x || :\n",
    "gate --x && echo done\nnext\n",
    "gate --x; echo done\n",
    "gate --x &\n",
    "! gate --x\n",
    "# gate --x\nnext\n",
    "echo # gate --x\n",
    "gate --x --more\n",
    "gate --x -- --skip test_\n",
    "gate --x > /dev/null\n",
    "if false; then\n  gate --x\nfi\n",
    "set +e\ngate --x\n",
    "set +o errexit\ngate --x\n",
    "gate --x \\\n  --more || true\n",
    "status=0\ngate --x || status=$?\necho done\n",
    // The fourth review's plants, and the other words that end or bend a script before the command.
    "exit 0\ngate --x\n",
    "exit\ngate --x\n",
    'exit "$ok"\ngate --x\n',
    "if true; then exit 0; fi\ngate --x\n",
    "true && exit\ngate --x\n",
    "trap 'exit 0' EXIT\ngate --x\n",
    "exec true\ngate --x\n",
    "return 0\ngate --x\n",
    "kill -9 $$\ngate --x\n",
    'eval "exit 0"\ngate --x\n',
    "builtin exit 0\ngate --x\n",
    "command exit 0\ngate --x\n",
    "source ./x.sh\ngate --x\n",
    ". ./x.sh\ngate --x\n",
    "alias gate=true\ngate --x\n",
    "gate() { true; }\ngate --x\n",
    "function gate { :; }\ngate --x\n",
  ]) {
    assert.ok(verdict(script), `the line rule lets through: ${script}`);
  }
  for (const script of ["if [ -z \"$x\" ]; then echo none; exit 1; fi\ngate --x\n", ". .github/scripts/limits.sh\ngate --x\n", "count() { :; }\ngate --x\n", "echo exit\ngate --x\n"]) {
    assert.ok(!verdict(script), `the line rule refuses: ${script}`);
  }
  assert.ok(verdict("gate --x | tee log\n", "gate --x | tee log", false), "a pipe in a shell without pipefail");
  assert.ok(verdict("set +o pipefail\ngate --x | tee log\n", "gate --x | tee log"), "pipefail turned off");
  assert.ok(verdict("gate --x || true\n", "gate --x || true"), "a declared command that forgives itself");
  for (const script of ["gate --x\n", "gate \\\n  --x\n", "set +o xtrace\ngate --x\n", "status=0\ngate --x || status=$?\nexit \"$status\"\n"]) {
    assert.ok(!verdict(script), `the line rule refuses: ${script}`);
  }
  assert.ok(!verdict("gate --x | tee log\n", "gate --x | tee log"), "a pipe under pipefail");
  assert.ok(!verdict("set -o pipefail\ngate --x | tee log\n", "gate --x | tee log", false), "pipefail set in the script");
});

/// The most time each required job may take, by workflow and job: a hang in a step without a
/// limit of its own holds a runner no longer, and a longer time is a change to the policy too.
const TIMEOUTS = new Map([
  ["ci.yml#rust", 45],
  ["ci.yml#bench", 30],
  ["ci.yml#execution-paths", 45],
  ["ci.yml#debuggers", 20],
  ["ci.yml#tier1-latency", 30],
  ["ci.yml#docs", 20],
  ["commits.yml#commits", 10],
  ["mutants.yml#plan", 60],
  ["mutants.yml#shard", 360],
  ["mutants.yml#aggregate", 5],
  ["generated.yml#paths", 120],
  ["smt.yml#z3", 45],
  ["fuzz.yml#fuzz", 30],
  ["mir-mutants.yml#gate", 30],
  ["perf.yml#kernels", 60],
  ["harness.yml#fixtures", 30],
  ["pages.yml#build", 15],
]);

/// The time an ignored test may take where a required job runs it by name, in release: those runs
/// took at most 2.7 minutes with their build, on any runner, in the runs of #120, #127 and main.
/// nextest's limits do not reach them, since they run through `cargo test` to show their output.
const IGNORED_TEST_LIMIT = "15m";

check("every ignored test a required job runs has a time limit of its own", () => {
  let found = 0;
  for (const { file, doc, id, job } of requiredJobs()) {
    for (const s of steps(job)) {
      for (const line of logicalLines(s.run ?? "")) {
        // Every timeout runs in bash, GNU's: Windows' own shell finds timeout.exe, which only
        // waits. A job's default shell is bash only on a Linux runner named as such.
        const shell = s.shell ?? job.defaults?.run?.shell ?? doc.defaults?.run?.shell;
        const linux = /^ubuntu-/.test(String(job["runs-on"]));
        if (/(^|[\s(&;|])timeout\s+\d/.test(line)) {
          assert.ok(shell === "bash" || (shell === undefined && linux), `${stepKey(file, id, s)} runs timeout in ${shell ?? "the runner's default shell"}, not bash`);
        }
        if (!/\bcargo\s+test\b.*\s--ignored\b/.test(line)) continue;
        found++;
        assert.match(line, new RegExp(`(^|[\\s(&;])timeout ${IGNORED_TEST_LIMIT} cargo test\\b`), `${stepKey(file, id, s)} runs ignored tests without their limit: timeout ${IGNORED_TEST_LIMIT} cargo test ...`);
        // Piped, its timeout would fail only the last command of the pipe without pipefail, which
        // GitHub's bash has only when a step names it.
        if (/(^|[^|])\|([^|]|$)/.test(line)) {
          assert.ok(shell === "bash" || /\bset\s+-[a-z]*o\s+pipefail\b/.test(String(s.run)), `${stepKey(file, id, s)} pipes ignored tests under timeout without pipefail`);
        }
      }
    }
  }
  assert.ok(found >= 6, "the policy reads the commands that run ignored tests");
});

check("every required job takes at most the time the policy holds for it", () => {
  const keys = new Set();
  for (const { file, id, job } of requiredJobs()) {
    const key = `${file}#${id}`;
    keys.add(key);
    assert.ok(TIMEOUTS.has(key), `${key} is a required job without a time in TIMEOUTS`);
    assert.equal(job["timeout-minutes"], TIMEOUTS.get(key), `${key}: timeout-minutes is ${job["timeout-minutes"]}, where the policy holds ${TIMEOUTS.get(key)}`);
  }
  assert.deepEqual([...TIMEOUTS.keys()].filter((k) => !keys.has(k)), [], "TIMEOUTS holds a time for a job that is not required");
});

/// The one condition an aggregate carries: it runs whatever the jobs it needs did, since a job
/// skipped, as a job with needs: is when one it needs fails, reports a check GitHub counts as
/// passing; `!cancelled()` would skip it on a cancellation.
const AGGREGATE_IF = "always()";
/// The values an aggregate's step may read: the event, and the result and outputs of a job it
/// needs, each an expression of its own.
const AGGREGATE_ENV = /^\$\{\{ (github\.event_name|needs\.([\w-]+)\.(result|outputs\.[\w-]+)) \}\}$/;

/// The one job-level condition each job an aggregate needs may carry, by workflow and job: a job
/// with none runs whenever the aggregate's workflow does (the review of 560ccd9: the shards'
/// condition was held by the pin alone).
const PART_JOB_IF = new Map([["mutants.yml#shard", "needs.plan.outputs.run == 'true' && needs.plan.outputs.count != '0'"]]);

/// Why a job an aggregate needs could let the aggregate pass with less checked: it may continue on
/// error, which reports success for a failed job, or it runs under a condition other than its
/// PART_JOB_IF entry, or under one where it has none.
export function partProblems(key, job) {
  const found = [];
  if ("continue-on-error" in job) found.push(`${key}: a job a required aggregate needs may continue on error, and so report success when it failed`);
  const held = PART_JOB_IF.get(key);
  if ("if" in job ? normalize(job.if) !== held : held !== undefined) found.push(`${key} runs if: ${job.if ?? "(none)"}, where the policy holds ${held ?? "no condition"} (PART_JOB_IF)`);
  return found;
}

/// Why an output a job gives could say other than a step of it found: each output an aggregate
/// reads is exactly `${{ steps.<id>.outputs.<the same key> }}` of a step of that job, never a value
/// written into the workflow (the review of 560ccd9: `reason: no-mutants` and `count: "0"` written
/// so passed every check but the pin).
export function outputProblems(key, need, outputKey) {
  const value = need?.outputs?.[outputKey];
  const m = /^\$\{\{ steps\.([\w-]+)\.outputs\.([\w-]+) \}\}$/.exec(String(value ?? ""));
  if (!m || m[2] !== outputKey || !steps(need ?? {}).some((s) => s.id === m[1])) return [`${key} gives outputs.${outputKey} as ${JSON.stringify(value ?? null)}, not \${{ steps.<id>.outputs.${outputKey} }} of a step of its own`];
  return [];
}

/// Why a required job that needs others is no aggregate of its workflow, which passes only when
/// they do: it runs if: always(); it needs every other job of its workflow; it is a name, a runner,
/// a time limit and one step, with no matrix, container, services or defaults; and that step is a
/// name, variables and a script, no action and so no checkout, whose every variable is the event or
/// the result or an output of a job it needs, and which reads every such job's result. What its
/// script decides is pinned whole (PINNED_JOBS), and mutation.test.mjs runs the script of "Mutants
/// in the diff" over the results its jobs can give.
export function aggregateProblems(key, doc, job) {
  const found = [];
  const [file, id] = key.split("#");
  const needs = needsOf(job);
  const others = Object.keys(doc.jobs ?? {}).filter((other) => other !== id);
  if (normalize(job.if ?? "") !== AGGREGATE_IF) found.push(`${key} runs if: ${job.if ?? "(none)"}, not ${AGGREGATE_IF}: a skipped aggregate reports a check GitHub counts as passing`);
  if ([...needs].sort().join(",") !== [...others].sort().join(",")) found.push(`${key} needs ${needs.join(", ") || "nothing"}, not every other job of its workflow: ${others.join(", ")}`);
  const keys = Object.keys(job).filter((k) => !["name", "needs", "if", "runs-on", "timeout-minutes", "steps"].includes(k));
  if (keys.length > 0) found.push(`${key} sets ${keys.join(", ")}, beyond a name, its needs, its condition, a runner, a time limit and its step`);
  const all = steps(job);
  if (all.length !== 1) found.push(`${key} has ${all.length} steps, not one`);
  for (const s of all) {
    const extra = Object.keys(s).filter((k) => !["name", "env", "run"].includes(k));
    if (extra.length > 0 || typeof s.run !== "string") found.push(`${stepKey(file, id, s)} is more than a name, variables and a script: ${extra.join(", ")}`);
    const read = new Set();
    for (const [name, value] of Object.entries(isPlain(s.env) ? s.env : {})) {
      const m = AGGREGATE_ENV.exec(String(value));
      if (!m || (m[2] !== undefined && !needs.includes(m[2]))) found.push(`${key} reads ${name}: ${value}, neither the event nor the result or an output of a job it needs`);
      else if (m[3] === "result") read.add(m[2]);
      else if (m[3]?.startsWith("outputs.")) found.push(...outputProblems(`${file}#${m[2]}`, doc.jobs?.[m[2]], m[3].slice("outputs.".length)));
    }
    for (const need of needs) if (!read.has(need)) found.push(`${key} does not read the result of ${need}, which it needs`);
  }
  return found;
}

check("every required check is one job that runs on every pull request and in the merge queue", () => {
  const jobs = allJobs();
  let aggregates = 0;
  for (const name of requiredChecks()) {
    const producers = jobs.filter((j) => j.names.includes(name));
    assert.equal(producers.length, 1, `"${name}" of ${REQUIRED_FILE} is the name of ${producers.length} jobs, not one`);
    const { file, doc, id, job } = producers[0];
    const where = `"${name}" (${file}, job ${id})`;
    const on = triggers(doc);
    for (const event of ["pull_request", "merge_group"]) {
      assert.ok(event in on, `${where}: the workflow does not run on ${event}`);
      const filter = on[event] ?? {};
      for (const key of ["paths", "paths-ignore", "branches", "branches-ignore"]) {
        assert.ok(!(key in filter), `${where}: its ${event} trigger has ${key}:, so a run it skips never reports`);
      }
    }
    const types = on.pull_request?.types;
    if (types) {
      for (const t of ["opened", "synchronize", "reopened"]) assert.ok(types.includes(t), `${where}: its pull_request trigger leaves out ${t}`);
    }
    assert.ok(!("continue-on-error" in job), `${where}: a required job may not continue on error`);
    // A required job with needs: is skipped, and so passes, when a job it needs fails, unless it is
    // an aggregate of its workflow.
    if ("needs" in job) {
      assert.deepEqual(aggregateProblems(`${file}#${id}`, doc, job), [], `${where}: a required job with needs: is no aggregate`);
      aggregates++;
    } else if ("if" in job) assert.equal(normalize(job.if), REQUIRED_JOB_IF, `${where}: a job-level if: could skip it`);
  }
  assert.ok(aggregates > 0, "the policy reads no aggregate, though Mutants in the diff is one");
  // Every job an aggregate needs: no continue-on-error, and its condition the one the policy holds.
  const named = new Set(jobs.filter((j) => j.names.some((n) => requiredChecks().includes(n))).map((j) => `${j.file}#${j.id}`));
  for (const { file, id, job } of requiredJobs().filter((j) => !named.has(`${j.file}#${j.id}`))) assert.deepEqual(partProblems(`${file}#${id}`, job), []);
  // Planted, against the class "a required check that can pass unseen": the aggregate of Mutants
  // with !cancelled(), with no condition, missing a need, with a checkout, as an action, with a
  // matrix, reading a value from elsewhere, and leaving a need's result unread.
  const mutants = workflows().find((w) => w.file === "mutants.yml").doc;
  const aggregate = mutants.jobs.aggregate;
  const step = steps(aggregate)[0];
  assert.deepEqual(aggregateProblems("mutants.yml#aggregate", mutants, aggregate), []);
  for (const [what, job] of [
    ["!cancelled()", { ...aggregate, if: "${{ !cancelled() }}" }],
    ["no condition", Object.fromEntries(Object.entries(aggregate).filter(([k]) => k !== "if"))],
    ["a need left out", { ...aggregate, needs: ["shard"] }],
    ["a checkout", { ...aggregate, steps: [{ uses: "actions/checkout@x" }, step] }],
    ["an action as its step", { ...aggregate, steps: [{ ...step, uses: "actions/github-script@x" }] }],
    ["a matrix", { ...aggregate, strategy: { matrix: { k: [0] } } }],
    ["a value from elsewhere", { ...aggregate, steps: [{ ...step, env: { ...step.env, PLAN: "${{ vars.PLAN }}" } }] }],
    ["a need's result unread", { ...aggregate, steps: [{ ...step, env: Object.fromEntries(Object.entries(step.env).filter(([, v]) => v !== "${{ needs.shard.result }}")) }] }],
  ]) {
    assert.ok(aggregateProblems("mutants.yml#aggregate", mutants, job).length > 0, `an aggregate with ${what} passes`);
  }
  // The review of 560ccd9's plants: the plan's outputs written into the workflow, or bound to a
  // step it does not hold or to another key; the shards continuing on error, or under another
  // condition or none.
  const plan = mutants.jobs.plan;
  for (const [what, outputs] of [
    ["its reason and count written in", { ...plan.outputs, reason: "no-mutants", count: "0" }],
    ["its watch's decision written in", { ...plan.outputs, run: "false" }],
    ["an output of a step it does not hold", { ...plan.outputs, count: "${{ steps.gone.outputs.count }}" }],
    ["an output of another key", { ...plan.outputs, count: "${{ steps.plan.outputs.needed }}" }],
  ]) {
    const planted = { ...mutants, jobs: { ...mutants.jobs, plan: { ...plan, outputs } } };
    assert.ok(aggregateProblems("mutants.yml#aggregate", planted, aggregate).length > 0, `a plan with ${what} passes`);
  }
  const shard = mutants.jobs.shard;
  assert.deepEqual(partProblems("mutants.yml#shard", shard), []);
  assert.deepEqual(partProblems("mutants.yml#plan", plan), []);
  for (const [what, key, job] of [
    ["continue-on-error", "mutants.yml#shard", { ...shard, "continue-on-error": true }],
    ["continue-on-error on the plan", "mutants.yml#plan", { ...plan, "continue-on-error": true }],
    ["another condition", "mutants.yml#shard", { ...shard, if: "needs.plan.outputs.run == 'true'" }],
    ["no condition", "mutants.yml#shard", Object.fromEntries(Object.entries(shard).filter(([k]) => k !== "if"))],
    ["a condition where none is held", "mutants.yml#plan", { ...plan, if: "false" }],
  ]) {
    assert.ok(partProblems(key, job).length > 0, `a job an aggregate needs with ${what} passes`);
  }
});

check("no step of a required job can let it pass without checking", () => {
  for (const { file, id, job } of requiredJobs()) {
    const watched = hasWatch(job);
    for (const [k, s] of steps(job).entries()) {
      const key = stepKey(file, id, s);
      if ("continue-on-error" in s) assert.ok(CONTINUE_ON_ERROR.has(key), `${key}: continue-on-error on a step of a required job`);
      if (String(s.uses ?? "").startsWith("./") || String(s.uses ?? "").startsWith("docker://")) {
        assert.fail(`${key}: a required job runs ${s.uses}, whose steps the policy cannot read`);
      }
      if (watched && k <= 1) continue;
      if (watched) {
        const condition = normalize(s.if ?? "");
        const allowed = WATCH_FORMS.includes(condition) || (REPORT_FORMS.includes(condition) && isReport(file, id, s));
        assert.ok(allowed, `${key}: if: ${s.if ?? "(none)"} is not ${WATCH_FORMS.join(" or ")}${isReport(file, id, s) ? ` or ${REPORT_FORMS.join(" or ")}` : ""}`);
      } else if ("if" in s) {
        assert.equal(normalize(s.if), STEP_CONDITIONS.get(key), `${key}: if: ${s.if} could skip a step of a required job`);
      }
    }
  }
});

// --------------------------------------------------------------------- milestones and their gates

/// The milestones of a ROADMAP.md: `### M<id>: ...` sections, each done when it has items and
/// every one is checked.
export function milestones(roadmap) {
  const out = new Map();
  let current = null;
  for (const line of roadmap.split(/\r?\n/)) {
    const heading = /^### (M\d+[a-z]?):/.exec(line);
    if (heading) {
      current = { id: heading[1], done: 0, open: 0 };
      out.set(current.id, current);
      continue;
    }
    if (/^#{1,3} /.test(line)) current = null;
    if (!current) continue;
    if (/^\s*- \[x\]/i.test(line)) current.done++;
    else if (/^\s*- \[ \]/.test(line)) current.open++;
  }
  return out;
}

const finished = (m) => m.done > 0 && m.open === 0;

/// The milestones done in `baseRoadmap` that have no tag: a pull request that finishes a milestone
/// passes, and the next one fails until the tag is set on that merge (milestone-tags).
export function untagged(baseRoadmap, tags) {
  return [...milestones(baseRoadmap).values()].filter(finished).map((m) => m.id).filter((id) => !tags.has(id.toLowerCase()));
}

check("every milestone done on the base has its tag", () => {
  assert.ok(milestones(read("ROADMAP.md")).size >= 10, "ROADMAP.md lists fewer milestones than stage 1 has");
  // The base, its ROADMAP.md and the tags, as the collector found them.
  const base = FACTS.base;
  assert.deepEqual(untagged(FACTS.baseRoadmap, new Set(FACTS.tags)), [], `milestones done on ${base} have no tag (milestone-tags)`);
  const open = "### M1: a\n- [x] one\n### M2: b\n- [x] two\n- [ ] three\n";
  const closed = open.replace("- [ ] three", "- [x] three");
  assert.deepEqual(untagged(open, new Set(["m1"])), [], "the pull request that closes M2 passes");
  assert.deepEqual(untagged(closed, new Set(["m1"])), ["M2"], "the next one fails until m2 is tagged");
});

/// The checks that run each instrument of the quality table in docs/design-brief.md section 7.7,
/// by the start of its row. A row with no entry here fails the test once its milestone is done.
const RUST = ["Rust (x86-64 Linux)", "Rust (x86-64 Windows)", "Rust (AArch64 Linux)"];
const GENERATED = ["Generated programs (x86-64 Linux)", "Generated programs (x86-64 Windows)", "Generated programs (AArch64 Linux)"];
export const INSTRUMENT_CHECKS = [
  ["A conformance suite", RUST],
  ["An error-code catalogue", RUST],
  ["Three execution paths", [...RUST, ...GENERATED, "Execution paths (x86-64 Linux)", "Execution paths (x86-64 Windows)", "Execution paths (AArch64 Linux)"]],
  ["A generator of well-typed programs", [...GENERATED, "Fuzz"]],
  ["Soundness by execution", [...GENERATED, "Tier 1 against Z3"]],
  ["Mutation testing at two levels", ["Mutants in the diff", "MIR mutation gate with the agent programs", ...RUST]],
  ["A CI matrix", RUST],
  ["Latency, performance and hardware-counter benchmarks", ["Projection benchmark", "Tier-1 latency", "Tight-loop kernels (x86-64 Linux)", "Tight-loop kernels (AArch64 Linux)"]],
  ["The agent benchmark", ["Documentation", "Fixtures in Rust and Odin"]],
  ["Rust hygiene", RUST],
];

check("every instrument scheduled for a finished milestone is a required check", () => {
  const brief = read("docs/design-brief.md");
  const start = brief.indexOf("| Instrument | What it catches | Active from |");
  assert.ok(start >= 0, "the quality table of docs/design-brief.md section 7.7 is gone");
  const lines = brief.slice(start).split(/\r?\n/).slice(2);
  const end = lines.findIndex((l) => !l.startsWith("|"));
  const rows = lines.slice(0, end < 0 ? lines.length : end).map((l) => l.split(" | ").map((c) => c.replace(/^\|\s*|\s*\|$/g, "")));
  assert.ok(rows.length >= 8, "the quality table has fewer rows than it did");
  const all = milestones(read("ROADMAP.md"));
  const isDone = (id) => {
    const parts = [...all.values()].filter((m) => m.id === id || (m.id.startsWith(id) && /^[a-z]$/.test(m.id.slice(id.length))));
    return parts.length > 0 && parts.every(finished);
  };
  const required = new Set(requiredChecks());
  let due = 0;
  for (const row of rows) {
    const instrument = row[0];
    const named = [...row[row.length - 1].matchAll(/\bM\d+[a-z]?\b/g)].map((m) => m[0]);
    assert.ok(named.length > 0, `the row "${instrument.slice(0, 40)}" names no milestone`);
    if (!named.some(isDone)) continue;
    due++;
    const entry = INSTRUMENT_CHECKS.find(([prefix]) => instrument.startsWith(prefix));
    assert.ok(entry, `"${instrument.slice(0, 60)}" is active from a finished milestone, and no required check is named for it here`);
    for (const check of entry[1]) assert.ok(required.has(check), `"${check}", which runs "${entry[0]}", is not in ${REQUIRED_FILE}`);
  }
  assert.ok(due > 0, "no instrument is due, though milestones are done");
});

/// The jobs that run the gate's own script alone, over no file of the tree, and so need no check of
/// it: the Commits check reads commit and pull request text through git and the API.
const CLEAN_TREE_EXEMPT = new Set(["commits.yml#commits"]);
/// The test runs of the testing jobs that gate-commands.txt does not list (the Mutants plan's baseline,
/// and mutants-full.yml, no required check): a run of the workspace's tests through nextest, and
/// cargo-mutants mutating in place.
const MUTATION_RUN = /^(cargo nextest run --workspace\b|cargo mutants .*--in-place\b)/;
/// The gate commands of the gate's own setup, each with why a step of these alone needs no check of
/// the tree: it runs before any code of the change, or reads only files outside the tree. A step
/// that runs one beside another gate command is checked around them all.
export const CLEAN_TREE_SETUP = new Map([
  ["node .github/scripts/build-time-code.mjs --before-toolchain", "the toolchain's step, right after the checkout, before anything else has run in the tree"],
  ["node .github/scripts/build-time-code.mjs", "the toolchain's step, right after the checkout, before anything else has run in the tree"],
  ["/usr/bin/bash .github/scripts/tools.sh record", "it records the digests of the runner's tools, outside the tree"],
  ["/usr/bin/bash .github/scripts/test-sandbox.sh setup", "it builds the sandbox under $RUNNER_TEMP"],
  ["/usr/bin/bash .github/scripts/test-sandbox.sh check", "it proves the sandbox with its canary, under $RUNNER_TEMP"],
  ['/usr/bin/sha256sum -c "$RUNNER_TEMP/cargo-mutants.sha256"', "it checks cargo-mutants' binary under $RUNNER_TEMP"],
  ["node .github/scripts/test-times.mjs", "it reads nextest's report under target/, which every walk leaves out"],
  ["bash .github/scripts/install-mdbook.sh", "it installs mdBook under $HOME"],
]);

/// Every step that runs a gate command of its job (`gate`, as gateCommands() reads it) other than
/// the gate's own setup (CLEAN_TREE_SETUP) without the check of the tree right before its first gate
/// command and right after its last, blank lines and comments aside; and every test run of a testing
/// job (`testingJobs`, TESTING_JOBS) without it so. A step's tests then never judge a tree a step
/// wrote into: before them, after them, or in a step between. And every check right after a
/// shellcheck directive, which then covers the check in place of the command it was written for
/// (pull request 223's first round: actionlint refused smt.yml's `bash -c 'check "$1"'` once the
/// check stood between it and its `# shellcheck disable=SC2016`).
export function cleanTreeProblems(docs, gate, testingJobs) {
  const out = [];
  for (const { file, doc } of docs) {
    for (const [id, job] of Object.entries(doc.jobs ?? {})) {
      const key = `${file}#${id}`;
      for (const s of steps(job)) {
        if (typeof s.run !== "string") continue;
        const raw = logicalLines(s.run).map((l) => l.trim()).filter((l) => l !== "");
        for (const [k, l] of raw.entries()) {
          if (l === CLEAN_TREE && /^#\s*shellcheck\b/.test(raw[k - 1] ?? "")) out.push(`${stepKey(file, id, s)}: the check of the tree stands right after \`${raw[k - 1]}\`, which then covers the check in place of its command: put the check above the directive and its comments`);
        }
      }
      if (CLEAN_TREE_EXEMPT.has(key)) continue;
      const commands = gate.get(key) ?? [];
      const runs = testingJobs.includes(key);
      if (commands.length === 0 && !runs) continue;
      for (const s of steps(job)) {
        if (typeof s.run !== "string") continue;
        const lines = logicalLines(s.run).map((l) => l.trim()).filter((l) => l !== "" && !l.startsWith("#"));
        const isRun = (l) => runs && MUTATION_RUN.test(l);
        const isGate = (l) => commands.some((c) => l === c || l === `${c} || status=$?`) || isRun(l);
        const at = lines.flatMap((l, k) => (isGate(l) ? [k] : []));
        if (!at.some((k) => isRun(lines[k]) || !CLEAN_TREE_SETUP.has(lines[k].replace(/ \|\| status=\$\?$/, "")))) continue;
        const where = stepKey(file, id, s);
        if (lines[at[0] - 1] !== CLEAN_TREE) out.push(`${where}: no check of the tree right before its first gate command, \`${lines[at[0]]}\`: put \`${CLEAN_TREE}\` there`);
        if (lines[at[at.length - 1] + 1] !== CLEAN_TREE) out.push(`${where}: no check of the tree right after its last gate command, \`${lines[at[at.length - 1]]}\`: put \`${CLEAN_TREE}\` there`);
      }
    }
  }
  return out;
}

check("every step of a required job checks, around its gate commands, that the tree is the commit's", () => {
  // The escape plan-downloaded-into-the-checkout: a shard's tests judged a tree a download had
  // written into. Every step that runs a gate command, and every test run of a testing job, checks
  // the tree right before and right after, so that no gate passes on a tree other than the commit's.
  const gate = gateCommands();
  assert.deepEqual(cleanTreeProblems(workflows(), gate, TESTING_JOBS), []);
  // Planted, on a job of its own: the check missing before, missing after, set apart from the command
  // by a writer, and kept only on a line before; a step of the exempt job; and a test run of a
  // mutation job that gate-commands.txt does not list.
  const job = (run) => [{ file: "x.yml", doc: { jobs: { j: { steps: [{ name: "s", run }] } } } }];
  const g = new Map([["x.yml#j", ["cargo test --workspace", "node a.mjs"]]]);
  const C = CLEAN_TREE;
  assert.deepEqual(cleanTreeProblems(job(`${C}\ncargo test --workspace\n${C}\n`), g, []), []);
  assert.deepEqual(cleanTreeProblems(job(`${C}\n# the tests\n\ncargo test --workspace || status=$?\n\n${C}\n`), g, []), [], "a comment or a blank line beside the command is refused");
  assert.deepEqual(cleanTreeProblems(job(`${C}\ncargo test --workspace\necho x\nnode a.mjs\n${C}\n`), g, []), [], "a step of two gate commands bracketed whole is refused");
  assert.deepEqual(cleanTreeProblems(job(`${C}\n# why\n# shellcheck disable=SC2016\ncargo test --workspace\n${C}\n`), g, []), [], "a shellcheck directive right before its command is refused");
  assert.equal(cleanTreeProblems(job(`# why\n# shellcheck disable=SC2016\n\n${C}\ncargo test --workspace\n${C}\n`), g, []).length, 1, "a check between a shellcheck directive and its command passes");
  assert.equal(cleanTreeProblems([{ file: "y.yml", doc: { jobs: { k: { steps: [{ run: `#shellcheck disable=SC2086\n${C}\necho $x\n` }] } } } }], new Map(), []).length, 1, "a check after a directive in a job with no gate command passes");
  for (const [run, why] of [
    ["cargo test --workspace\n" + C + "\n", "the check missing before"],
    [C + "\ncargo test --workspace\n", "the check missing after"],
    [C + "\necho x > a.md\ncargo test --workspace\n" + C + "\n", "a writer between the check and the command"],
    [C + "\ncargo test --workspace\ntouch b.md\n" + C + "\n", "a writer between the command and the check after it"],
    ["cargo test --workspace", "a one-line step with no check"],
    [C + "\nnode a.mjs\n" + C + "\ncargo test --workspace\n", "the check around the first command alone"],
  ]) {
    assert.ok(cleanTreeProblems(job(run), g, []).length > 0, `${why} passes`);
  }
  const exempt = [{ file: "commits.yml", doc: { jobs: { commits: { steps: [{ run: "node .github/scripts/commits.mjs" }] } } } }];
  assert.deepEqual(cleanTreeProblems(exempt, new Map([["commits.yml#commits", ["node .github/scripts/commits.mjs"]]]), []), [], "the Commits job is held to the check");
  const full = [{ file: "f.yml", doc: { jobs: { full: { steps: [{ run: 'cargo nextest run --workspace 2>&1 | tee "$RUNNER_TEMP/b.log"' }, { run: `${C}\ncargo mutants --in-place --shard 0/16 || status=$?\n${C}\n` }] } } } }];
  assert.equal(cleanTreeProblems(full, new Map(), ["f.yml#full"]).length, 2, "a mutation job's test run without the check passes");
  assert.equal(cleanTreeProblems(full, new Map(), []).length, 0, "a job that is no mutation job is held to its test runs");
  // The gate's own setup alone needs no check; beside another gate command, the step is checked
  // around them all.
  const setup = [...CLEAN_TREE_SETUP.keys()];
  const withSetup = new Map([["x.yml#j", [...setup, "cargo test --workspace"]]]);
  assert.deepEqual(cleanTreeProblems(job(`${setup[0]}\nrustup toolchain install\n${setup[1]}\n`), withSetup, []), [], "the toolchain's step is held to the check");
  assert.equal(cleanTreeProblems(job(`${setup[2]}\ncargo test --workspace\n`), withSetup, []).length, 2, "a step of setup and a test with no check passes");
  assert.equal(cleanTreeProblems(job(`${C}\n${setup[2]}\ncargo test --workspace\n${C}\n`), withSetup, []).length, 0, "a step of setup and a test checked around both is refused");
  assert.equal(cleanTreeProblems(job(`${setup[2]}\n${C}\ncargo test --workspace\n${C}\n`), withSetup, []).length, 1, "a check after the step's setup, which is its first gate command, passes");
  for (const [command, why] of CLEAN_TREE_SETUP) assert.ok(why.length > 0 && command.length > 0);
});
