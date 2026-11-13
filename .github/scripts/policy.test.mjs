// The workflow policy (docs/design-review.md, guard-per-problem): what makes the instruments
// merge gates of `main` that cannot pass without checking, and what keeps the workflows safe,
// checked over .github/workflows. It runs in the Documentation job, a required check, as
// `npm run test:ci`; test_the_documentation_job_runs_the_workflow_policy in compiler/mente-cli
// keeps it there.
//
// - Every check .github/required-checks.txt names is produced by exactly one job, whose workflow
//   runs on every pull request and in the merge queue with no `paths:` or `branches:` filter, and
//   which nothing can skip, let fail, or give more than read access.
// - Every instrument workflow decides at its first step after the checkout whether it has anything
//   to check (.github/scripts/watch.mjs); every later step runs unless the decision was "false",
//   so a decision never written runs the instrument.
// - Each instrument's list in .github/instrument-paths.txt names Cargo.toml, Cargo.lock,
//   rust-toolchain.toml, its workflow, every file of every crate its jobs build, every repository
//   path its scripts name with what they import, and the corpora the compiler reads for it.
// - No script a required job runs gates its main() on the path it was started by.
// - Every action is pinned to a full commit SHA, every checkout drops its credentials, no workflow
//   runs on `pull_request_target` or `workflow_run`, no `run:` script and no `script:` input holds
//   an expression, overflow checks stay on in every build, and a measured benchmark run proves its
//   sandbox before any agent code runs.
// - Every milestone ROADMAP.md marks done on the base has its tag, and every instrument
//   docs/design-brief.md section 7.7 schedules for a finished milestone has its checks required.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import http from "node:http";
import { fileURLToPath } from "node:url";
import { parseYaml } from "./lib/yaml.mjs";
import { ALWAYS, PATHS_FILE, decide, globToRegExp, matches, parsePaths } from "./lib/watch.mjs";
import { CJK_RANGES, badSubject, baseChangedSincePush, cjkLines, problems, pullRequestOf } from "./lib/commits.mjs";
import { deletedRegexes, evidence, exactRegex, expected, missing, parseDiff, readList, touched } from "./lib/mutants-floor.mjs";
import { doctestProblems, problems as testListProblems, slowTests } from "./lib/test-list.mjs";
import { pins as actionPins, problems as actionPinProblems, taggedCommit } from "./lib/action-pins.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));

/// Every check of the policy counts itself when it runs, and the suite fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// FLOOR equal to the checks this file declares.
const FLOOR = 52;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks ran: a skip or a filter left the rest out`);
});
const ROOT = path.resolve(HERE, "../..");
const read = (file) => fs.readFileSync(path.join(ROOT, file), "utf8");
const git = (args) => execFileSync("git", args, { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
const WORKFLOWS = ".github/workflows";
const REQUIRED_FILE = ".github/required-checks.txt";
const WATCH = "steps.watch.outputs.run != 'false'";
/// The conditions a step after the watch may carry: the watch's decision, alone or after
/// `!cancelled()`, which still runs a check when an earlier one failed.
const WATCH_FORMS = [WATCH, `!cancelled() && ${WATCH}`];
/// The conditions that run a step only when another failed, or even when the run was cancelled:
/// for a step that keeps or reports what the others found, never for a check, which they would
/// skip whenever the job is passing.
const REPORT_FORMS = [`always() && ${WATCH}`, `failure() && ${WATCH}`];
/// The steps of watch jobs, beside the uploads of artifacts, that only report: workflow, job, step.
const REPORT_STEPS = new Set(["generated.yml#paths#Summary"]);
const isReport = (file, id, s) => String(s.uses ?? "").startsWith("actions/upload-artifact@") || REPORT_STEPS.has(stepKey(file, id, s));
/// The shells a step may run in, named bare: a shell given as a template (`true {0}`) could run
/// nothing and pass.
const SHELLS = new Set(["bash", "sh", "pwsh"]);
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
]);
/// The steps of required jobs that may fail without failing the job, each with its reason.
const CONTINUE_ON_ERROR = new Map([
  // x86-64's runner exposes no counters, so its probe fails there; AArch64's job requires the
  // counters through perf/run.mjs --require-instructions instead.
  ["perf.yml#kernels#Install perf and let it count", "the counters are required by run.mjs where they exist"],
]);

const normalize = (expression) =>
  String(expression)
    .trim()
    .replace(/^\$\{\{\s*([\s\S]*?)\s*\}\}$/, "$1")
    .replace(/\s+/g, " ");

let trackedCache;
function tracked() {
  trackedCache ??= new Set(git(["ls-files", "-z"]).split("\0").filter(Boolean));
  return trackedCache;
}
const isDir = (dir) => [...tracked()].some((f) => f.startsWith(`${dir}/`));
const filesUnder = (dir) => [...tracked()].filter((f) => f.startsWith(`${dir}/`));

function workflows() {
  return fs
    .readdirSync(path.join(ROOT, WORKFLOWS))
    .filter((f) => f.endsWith(".yml") || f.endsWith(".yaml"))
    .sort()
    .map((file) => ({ file, doc: parseYaml(read(`${WORKFLOWS}/${file}`)) }));
}

/// The events a workflow runs on, as an object whatever form `on:` takes.
function triggers(doc) {
  const on = doc.on;
  if (typeof on === "string") return { [on]: null };
  if (Array.isArray(on)) return Object.fromEntries(on.map((e) => [e, null]));
  return on ?? {};
}

/// The names GitHub gives a job's checks: its `name:`, or its id, once for each combination of its
/// matrix, with `${{ matrix.<key> }}` filled in, or, where the name has no matrix expression, the
/// combination's values appended in parentheses.
export function checkNames(id, job) {
  const template = String(job.name ?? id);
  const matrix = job.strategy?.matrix;
  if (!matrix || typeof matrix !== "object") return [template];
  const keys = Object.keys(matrix).filter((k) => k !== "include" && k !== "exclude");
  if (keys.some((k) => !Array.isArray(matrix[k]))) return [template];
  if (matrix.exclude) throw new Error(`${id}: a matrix with exclude is not read here`);
  let combos = [{}];
  for (const k of keys) combos = combos.flatMap((c) => matrix[k].map((v) => ({ ...c, [k]: v })));
  if (matrix.include) {
    if (keys.length > 0) throw new Error(`${id}: a matrix with both keys and include is not read here`);
    combos = matrix.include;
  }
  const expression = () => /\$\{\{\s*matrix\.([\w-]+)\s*\}\}/g;
  return combos.map((c) =>
    expression().test(template)
      ? template.replace(expression(), (all, key) => (key in c ? String(c[key]) : all))
      : `${template} (${Object.values(c).map(String).join(", ")})`,
  );
}

function allJobs() {
  return workflows().flatMap(({ file, doc }) =>
    Object.entries(doc.jobs ?? {}).map(([id, job]) => ({ file, doc, id, job, names: checkNames(id, job) })),
  );
}

function requiredChecks() {
  return read(REQUIRED_FILE)
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l !== "" && !l.startsWith("#"));
}

function requiredJobs() {
  const required = new Set(requiredChecks());
  return allJobs().filter((j) => j.names.some((n) => required.has(n)));
}

const steps = (job) => (Array.isArray(job.steps) ? job.steps : []);
const stepKey = (file, id, s) => `${file}#${id}#${s.name ?? s.uses ?? String(s.run).split("\n")[0]}`;
const hasWatch = (job) => steps(job).some((s) => s.id === "watch" || /watch\.mjs/.test(String(s.run ?? "")));

// ------------------------------------------------------------------- safety of every workflow

check("no workflow runs on pull_request_target or workflow_run", () => {
  for (const { file, doc } of workflows()) {
    for (const event of ["pull_request_target", "workflow_run"]) {
      assert.ok(!(event in triggers(doc)), `${file} runs on ${event}, which runs with the base's secrets and a write token`);
    }
  }
});

const readOnly = (permissions) =>
  permissions === "read-all" ||
  (permissions !== null && typeof permissions === "object" && Object.values(permissions).every((v) => v === "read" || v === "none"));

check("every workflow sets its permissions at the top, read-only wherever a required job runs", () => {
  for (const { file, doc } of workflows()) {
    assert.ok(doc.permissions !== undefined && doc.permissions !== null, `${file} has no top-level permissions:`);
    assert.notEqual(doc.permissions, "write-all", `${file} gives every permission`);
  }
  for (const { file, doc, id, job } of requiredJobs()) {
    assert.ok(readOnly(doc.permissions), `${file}: a required job runs under permissions broader than read: ${JSON.stringify(doc.permissions)}`);
    if ("permissions" in job) {
      assert.ok(readOnly(job.permissions), `${file}, job ${id}: a required job widens its permissions: ${JSON.stringify(job.permissions)}`);
    }
  }
});

check("every action is pinned to a full commit SHA", () => {
  for (const { file, id, job } of allJobs()) {
    const uses = [job.uses, ...steps(job).map((s) => s.uses)].filter((u) => u !== undefined);
    for (const u of uses) {
      assert.match(String(u), /^(\.\/.+|[\w.-]+\/[\w.-]+(\/[\w./-]+)?@[0-9a-f]{40})$/, `${file}, job ${id}: ${u} is not pinned to a full commit SHA`);
    }
  }
});

check("every pinned action names its release, which the Documentation job checks the commit against", () => {
  // Offline, every pin carries its tag; the Documentation job asks GitHub that the tag names the
  // commit (.github/scripts/action-pins.mjs).
  for (const { file } of workflows()) {
    for (const p of actionPins(`${WORKFLOWS}/${file}`, read(`${WORKFLOWS}/${file}`))) assert.notEqual(p.tag, null, `${p.at}: ${p.repo}@${p.sha} names no release in a comment`);
  }
  const sha = (c) => c.repeat(40);
  const read2 = actionPins("w.yml", `      - uses: actions/checkout@${sha("a")} # v7.0.1\n        uses: org/repo/sub/path@${sha("b")}\n      - uses: ./local\n`);
  assert.deepEqual(read2.map((p) => [p.repo, p.tag]), [["actions/checkout", "v7.0.1"], ["org/repo", null]]);
  assert.equal(taggedCommit(`${sha("c")}\trefs/tags/v1\n${sha("d")}\trefs/tags/v1^{}\n`, "v1"), sha("d"), "an annotated tag is its peeled commit");
  assert.equal(taggedCommit(`${sha("c")}\trefs/tags/v1\n`, "v1"), sha("c"));
  assert.equal(taggedCommit("", "v1"), null);
  const remote = (repo, tag) => (tag === "v7.0.1" ? `${sha("a")}\trefs/tags/v7.0.1\n` : "");
  assert.deepEqual(actionPinProblems(read2.slice(0, 1), remote), []);
  assert.equal(actionPinProblems([{ ...read2[0], sha: sha("e") }], remote).length, 1, "a commit other than its tag's");
  assert.equal(actionPinProblems([{ ...read2[0], tag: "v9" }], remote).length, 1, "a tag that is not there");
  assert.equal(actionPinProblems([read2[1]], remote).length, 1, "a pin without its tag");
});

check("every checkout drops its credentials", () => {
  for (const { file, id, job } of allJobs()) {
    for (const s of steps(job)) {
      if (String(s.uses ?? "").startsWith("actions/checkout@")) {
        assert.equal(s.with?.["persist-credentials"], false, `${file}, job ${id}: a checkout without persist-credentials: false`);
      }
    }
  }
});

check("no script holds an expression: values reach scripts through env", () => {
  for (const { file, id, job } of allJobs()) {
    for (const s of steps(job)) {
      const where = `${file}, job ${id}, step "${s.name ?? s.uses ?? String(s.run).split("\n")[0]}"`;
      if (typeof s.run === "string") {
        const found = s.run.match(/\$\{\{[\s\S]*?\}\}/);
        assert.equal(found, null, `${where}: ${found?.[0]} is written into the script; pass it through env:`);
      }
      assert.ok(!(s.with && "script" in s.with), `${where}: a script: input runs code the policy cannot read; use a run: step`);
    }
  }
});

/// Any spelling of the overflow-checks flag rustc and cargo accept: a hyphen or an underscore, in
/// any case (`-C overflow_checks=off` works as well as `overflow-checks`).
const OVERFLOW = /overflow[-_]checks/i;
/// The cargo profile variables a workflow may set, each with its reason.
const CARGO_PROFILE_ENV = new Map([
  // The fuzzer's release build also asserts what a debug build asserts (fuzz.yml).
  ["CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", "the fuzzer's debug assertions"],
]);

check("overflow checks stay on in every build", () => {
  for (const { file, doc } of workflows()) {
    assert.doesNotMatch(read(`${WORKFLOWS}/${file}`), OVERFLOW, `${file} names overflow checks: [profile.release] in Cargo.toml is their one source`);
    const envs = [doc.env, ...Object.values(doc.jobs ?? {}).flatMap((j) => [j.env, ...steps(j).map((s) => s.env)])].filter(Boolean);
    for (const env of envs) {
      for (const [name, value] of Object.entries(env)) {
        if (/^CARGO_PROFILE_/.test(name)) assert.ok(CARGO_PROFILE_ENV.has(name), `${file} sets ${name}, a profile Cargo.toml does not show`);
        if (/RUSTFLAGS$|^CARGO_/.test(name)) assert.doesNotMatch(String(value), /overflow/i, `${file}: ${name} touches overflow checks`);
      }
    }
  }
  for (const f of [...tracked()].filter((f) => /(^|\/)\.cargo\//.test(f))) {
    assert.doesNotMatch(read(f), OVERFLOW, `${f} names overflow checks`);
  }
  const manifests = new Map([...tracked()].filter((f) => f === "Cargo.toml" || f.endsWith("/Cargo.toml")).map((f) => [f, read(f)]));
  assert.deepEqual(overflowProblems(manifests, [...tracked()]), []);
  // What the check stops: a package's own profile, in either spelling and quoted, a release profile
  // without the checks, a .cargo/config anywhere.
  const root = "[profile.release]\noverflow-checks = true\n";
  const one = (text, files = []) => overflowProblems(new Map([["Cargo.toml", text]]), files).length;
  assert.equal(one(root), 0);
  assert.equal(one(`${root}\n[profile.release.package.mente-mir]\noverflow-checks = false\n`), 1);
  assert.equal(one(`${root}\n[profile.release.package.mente-sema]\n"overflow-checks" = false\n`), 1);
  assert.equal(one(`${root}\n[profile.dev]\n'Overflow_Checks' = false\n`), 1);
  assert.equal(one(`${root}\n[profile.release.package]\nmente-mir = { overflow-checks = false }\n`), 1);
  assert.equal(one("[profile.release]\nopt-level = 3\n"), 1);
  assert.equal(one(root, ["compiler/x/.cargo/config.toml"]), 1);
  assert.equal(one(root, ["compiler/x/.cargo/config"]), 1);
});

/// Why some build could run without overflow checks: any setting of the flag in any manifest but
/// `true`, in any spelling or quoting, a root [profile.release] that does not turn it on, or a
/// .cargo/config anywhere in the tree, which can change a profile where Cargo.toml does not show it.
export function overflowProblems(manifests, files) {
  const found = [];
  for (const [file, text] of manifests) {
    for (const m of text.matchAll(/["']?overflow[-_]checks["']?\s*=\s*([^\s,}#]+)/gi)) {
      if (m[1] !== "true") found.push(`${file} sets overflow checks to ${m[1]}`);
    }
  }
  const release = /^\[profile\.release\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(manifests.get("Cargo.toml") ?? "");
  if (!/^overflow-checks\s*=\s*true\s*$/m.test(release?.[1] ?? "")) found.push("Cargo.toml's [profile.release] does not set overflow-checks = true");
  for (const f of files) if (/(^|\/)\.cargo\/config(\.toml)?$/.test(f)) found.push(`${f} can change a profile unseen`);
  return found;
}

check("every compiler crate proves its overflow checks in a release build", () => {
  const crates = [...tracked()].filter((f) => /^compiler\/[^/]+\/Cargo\.toml$/.test(f)).map((f) => path.posix.dirname(f));
  assert.ok(crates.length > 0);
  for (const dir of crates) {
    const has = filesUnder(`${dir}/src`).some((f) => f.endsWith(".rs") && read(f).includes("fn test_arithmetic_overflow_panics_in_this_crate"));
    assert.ok(has, `${dir} has no test_arithmetic_overflow_panics_in_this_crate (src/overflow_traps.rs)`);
  }
  // The proof every release-building required job runs: every crate's test in release, counted.
  const proof = read(".github/scripts/overflow-proof.sh");
  assert.match(proof, /cargo test --release --locked --workspace --lib --bins -- --exact "\$test"/);
  assert.match(proof, /test=overflow_traps::test_arithmetic_overflow_panics_in_this_crate/);
  assert.match(proof, /if \[ "\$expected" -eq 0 \] \|\| \[ "\$passed" -ne "\$expected" \]; then/);
});

/// A line of the TOML subset the policy reads, without its trailing comment and the whitespace
/// around it: everything from the first `#` outside a string on goes.
function uncomment(line) {
  let quote = null;
  for (let k = 0; k < line.length; k++) {
    const c = line[k];
    if (quote !== null) {
      if (c === "\\" && quote === '"') k++;
      else if (c === quote) quote = null;
    } else if (c === '"' || c === "'") quote = c;
    else if (c === "#") return line.slice(0, k).trim();
  }
  return line.trim();
}

/// A file of the TOML subset the policy reads: its tables in order, each with its keys and their
/// values as written. nextest and rustup at most warn about a key they do not know, and TOML has
/// spellings a narrower reader would pass over (quoted and dotted keys, a key before the first
/// table), so every line must be a blank, a comment, a table's header, a bare `key = value`, or a
/// line of a `"""` string, each with an optional trailing comment. Any other line is an error
/// rather than a line skipped, and so is a table or a key twice.
export function tomlSubset(text, file) {
  const tables = [];
  const lines = text.split(/\r?\n/);
  let current = null;
  const fail = (k, why) => {
    throw new Error(`${file}:${k + 1}: ${why}: ${lines[k]}`);
  };
  for (let k = 0; k < lines.length; k++) {
    const line = uncomment(lines[k]);
    if (line === "") continue;
    const header = /^(\[\[?)([a-z][a-z0-9-]*(?:\.[a-z][a-z0-9-]*)*)(\]\]?)$/.exec(line);
    if (header) {
      const array = header[1] === "[[";
      if (array !== (header[3] === "]]")) fail(k, "a header whose brackets do not match");
      if (tables.some((t) => t.name === header[2] && (!array || !t.array))) fail(k, "a table twice");
      current = { name: header[2], array, keys: new Map() };
      tables.push(current);
      continue;
    }
    const entry = /^([a-z][a-z0-9-]*) = (.+)$/.exec(line);
    if (!entry) fail(k, "not a line the policy reads");
    if (current === null) fail(k, "a key outside any table");
    if (current.keys.has(entry[1])) fail(k, "a key twice");
    let value = entry[2];
    if (value.startsWith('"""')) {
      const start = k;
      value = lines[k].slice(lines[k].indexOf('"""'));
      while (value.indexOf('"""', 3) < 0) {
        if (++k >= lines.length) fail(start, "a string never closed");
        value += `\n${lines[k]}`;
      }
      const end = value.indexOf('"""', 3);
      if (uncomment(value.slice(end + 3)) !== "") fail(k, "more after the string's end");
      value = value.slice(0, end + 3);
    }
    current.keys.set(entry[1], value);
  }
  return tables;
}

/// What .config/nextest.toml may hold, table by table, with the keys each must set. To give nextest
/// another table (a JUnit report for ci, a test group), add it here with its keys in the same pull
/// request, where review sees what the table changes.
const NEXTEST_TABLES = new Map([
  ["profile.default", { array: false, keys: ["leak-timeout", "slow-timeout"] }],
  ["profile.default.overrides", { array: true, keys: ["filter", "slow-timeout"] }],
  ["profile.ci", { array: false, keys: ["fail-fast"] }],
  ["profile.mutants", { array: false, keys: ["fail-fast"] }],
]);
/// The limits, as .config/nextest.toml must state them, spaces aside: nextest reads them there, and
/// the policy holds this copy, so that a changed limit is a change to the policy too. A slow-timeout
/// in its table form stops the test; the string form only marks it slow.
const NEXTEST_VALUES = new Map([
  ["profile.default/slow-timeout", '{ period = "20s", terminate-after = 2 }'],
  ["profile.default/leak-timeout", '{ period = "100ms", result = "fail" }'],
  ["profile.default.overrides/slow-timeout", '{ period = "60s", terminate-after = 3 }'],
  ["profile.ci/fail-fast", "false"],
  ["profile.mutants/fail-fast", "true"],
]);
/// The tests the overrides give their longer limit, each by its package and its whole path: more
/// time for another test is a change to the policy too.
const SLOW_TESTS = [
  ["mente-sema", "incremental::tests::test_a_session_reports_what_the_whole_module_check_does"],
  ["mente-cli", "mir_run::differential::tests::test_three_paths_agree_on_the_conformance_suite"],
  ["mente-cli", "mir_run::tests::test_mir_runs_every_agent_program_as_the_reference_interpreter_does"],
  ["mente-cli", "mir_run::coverage::test_generated_programs_reach_every_construct_of_the_grammar"],
  ["mente-interp", "tests::test_a_program_tier1_proves_never_trips_its_checks"],
  ["mente-cli", "mir_run::tests::test_native_code_runs_each_construct_as_the_reference_interpreter_does"],
];

/// The tests an override's filter names, each as [package, path]: `(package(name) & test(=path))`
/// joined by `|`. The package keeps a test of the same path in another crate out, and the exact
/// path a test whose name contains it; anything else in the filter is an error.
export function filterTests(value) {
  const m = /^"""([\s\S]*)"""$/.exec(value) ?? /^"([^"\\]*)"$/.exec(value);
  assert.ok(m, `an override's filter is not a plain string: ${value}`);
  const filter = m[1];
  const term = /\(package\(([a-z][a-z0-9_-]*)\) & test\(=([^)]*)\)\)/g;
  const tests = [...filter.matchAll(term)].map((t) => [t[1], t[2]]);
  assert.ok(tests.length > 0, `an override's filter names no test by its package and path: ${filter}`);
  const rest = filter.replace(term, "").replace(/\s/g, "");
  assert.equal(rest, "|".repeat(tests.length - 1), `an override's filter is more than (package(name) & test(=path)) joined by |: ${filter}`);
  for (const [, p] of tests) assert.match(p, /^[A-Za-z_]\w*(::[A-Za-z_]\w*)*$/, `test(=${p}) is not a test's path`);
  return tests;
}

/// Where a test of a given path is in a package: each function of its name, in a file of the
/// package whose module path, with the inline modules declared in it, is the test's, at the
/// indentation rustfmt gives that depth, with whether it is a #[test] and whether it is ignored
/// ("yes", "no", or "unknown" for an ignore under a cfg_attr the policy does not read). A unit
/// test's path is its module path in its target; an integration test's, its path within its target
/// under tests/, a file or a directory with main.rs.
export function testsAt(dir, testPath, files) {
  const segments = testPath.split("::");
  const name = segments.pop();
  const found = [];
  const escaped = dir.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  for (const [file, text] of files) {
    const m = new RegExp(`^${escaped}/(src|tests)/(.+)\\.rs$`).exec(file);
    if (!m) continue;
    const parts = m[2].split("/");
    let modules;
    if (m[1] === "tests") {
      if (parts.length === 1 || (parts.length === 2 && parts[1] === "main")) modules = [];
      else modules = parts.slice(1).filter((p, k, all) => !(k === all.length - 1 && p === "mod"));
    } else if (parts.length === 1 && (parts[0] === "lib" || parts[0] === "main")) {
      modules = [];
    } else if (parts[0] === "bin" && (parts.length === 2 || (parts.length === 3 && parts[2] === "main"))) {
      modules = [];
    } else {
      modules = parts.filter((p, k, all) => !(k === all.length - 1 && p === "mod"));
    }
    if (modules.length > segments.length || modules.some((s, k) => s !== segments[k])) continue;
    const inline = segments.slice(modules.length);
    const lines = text.split(/\r?\n/);
    const declared = inline.every((mod, depth) => lines.some((l) => l === `${" ".repeat(4 * depth)}mod ${mod} {`));
    if (!declared) continue;
    const head = new RegExp(`^${" ".repeat(4 * inline.length)}(pub(\\([^)]*\\))? )?fn ${name}\\(\\)`);
    for (let k = 0; k < lines.length; k++) {
      if (!head.test(lines[k])) continue;
      const attributes = [];
      for (let j = k - 1; j >= 0 && /^\s*(#\[|\/\/)/.test(lines[j]); j--) if (/^\s*#\[/.test(lines[j])) attributes.push(lines[j].trim());
      let ignored = "no";
      for (const a of attributes) {
        if (/^#\[ignore\b/.test(a)) ignored = "yes";
        else if (/\bignore\b/.test(a) && a !== "#[cfg_attr(miri, ignore)]" && ignored === "no") ignored = "unknown";
      }
      found.push({ at: `${file}:${k + 1}`, test: attributes.includes("#[test]"), ignored });
    }
  }
  return found;
}

check("the tests run under the limits the policy holds, and every test given more time runs", () => {
  const tables = tomlSubset(read(".config/nextest.toml"), ".config/nextest.toml");
  for (const t of tables) {
    const allowed = NEXTEST_TABLES.get(t.name);
    const shown = t.array ? `[[${t.name}]]` : `[${t.name}]`;
    assert.ok(allowed && allowed.array === t.array, `.config/nextest.toml has ${shown}: to give nextest another table, add it to NEXTEST_TABLES in .github/scripts/policy.test.mjs with the keys it may hold`);
    assert.deepEqual([...t.keys.keys()].sort(), [...allowed.keys].sort(), `${shown} sets ${[...t.keys.keys()].join(", ") || "nothing"}, not ${allowed.keys.join(", ")}`);
    for (const [key, value] of t.keys) {
      const held = NEXTEST_VALUES.get(`${t.name}/${key}`);
      if (held === undefined) continue;
      assert.equal(value.replace(/\s+/g, ""), held.replace(/\s+/g, ""), `${shown} ${key} = ${value}, where the policy holds ${held}: a limit changes in NEXTEST_VALUES too`);
    }
  }
  for (const name of ["profile.default", "profile.ci", "profile.mutants"]) assert.ok(tables.some((t) => t.name === name), `.config/nextest.toml has no [${name}]`);
  const named = tables.filter((t) => t.array).flatMap((t) => filterTests(t.keys.get("filter")));
  const shown = (list) => list.map(([p, t]) => `${p} ${t}`).sort();
  assert.deepEqual(shown(named), shown(SLOW_TESTS), "the overrides give their longer limit to other tests than SLOW_TESTS, or one twice: more time for a test is a change to the policy too");
  const crates = workspace();
  for (const [pkg, testPath] of named) {
    const crate = crates.get(pkg);
    assert.ok(crate, `package(${pkg}), named in .config/nextest.toml, is no member of the workspace`);
    const files = new Map(filesUnder(crate.dir).filter((f) => f.endsWith(".rs")).map((f) => [f, read(f)]));
    const found = testsAt(crate.dir, testPath, files);
    assert.equal(found.length, 1, `${pkg} ${testPath}, named in .config/nextest.toml, is ${found.length} functions of ${crate.dir}, not one: ${found.map((f) => f.at).join(", ")}`);
    assert.ok(found[0].test, `${pkg} ${testPath} at ${found[0].at} is no #[test]`);
    assert.equal(found[0].ignored, "no", `${pkg} ${testPath} at ${found[0].at} is ignored, or may be, so its limit holds for nothing the jobs run`);
  }
  // What the reader refuses rather than skips, and what it reads.
  const refused = [
    '[profile.ci]\n"slow-timeout" = 1\n',
    "[profile.ci]\nslow-timeout.period = 1\n",
    "[profile.ci]\nfail-fast = true\nfail-fast = false\n",
    "retries = 3\n[profile.ci]\n",
    "[profile.ci]\n[profile.ci]\n",
    "[profile.ci]\n[profile.ci]] \n",
    '[[profile.default.overrides]]\nfilter = """\ntest(=a)\n',
    '[[profile.default.overrides]]\nfilter = """\ntest(=a)\n""" x\n',
  ];
  for (const text of refused) assert.throws(() => tomlSubset(text, "f.toml"), /f\.toml:\d+/, text);
  const sample = tomlSubset('# a\n[profile.default] # b\n  slow-timeout = 1  # c\n[[profile.default.overrides]]\nfilter = """\n(package(x) & test(=a::b))\n| (package(y) & test(=c))\n""" # d\n[profile.ci]\nfail-fast = "a # b"\n', "f.toml");
  assert.deepEqual(sample.map((t) => t.name), ["profile.default", "profile.default.overrides", "profile.ci"]);
  assert.equal(sample[0].keys.get("slow-timeout"), "1");
  assert.equal(sample[2].keys.get("fail-fast"), '"a # b"');
  assert.deepEqual(filterTests(sample[1].keys.get("filter")), [["x", "a::b"], ["y", "c"]]);
  for (const v of ['"""\ntest(=a)\n"""', '"""\n(package(x) & test(a))\n"""', '"""\n(package(x) & test(=a)) | package(y)\n"""', '"""\n(package(x) & test(=a)) (package(x) & test(=b))\n"""', '"""\n(package(x) & test(=/a/))\n"""']) {
    assert.throws(() => filterTests(v), assert.AssertionError, v);
  }
  const unit = new Map([
    ["c/src/m.rs", "mod tests {\n    #[test]\n    fn t() {}\n\n    fn helper() {}\n\n    #[test]\n    #[ignore]\n    fn slow() {}\n\n    #[test]\n    #[cfg_attr(miri, ignore)]\n    fn native() {}\n\n    #[test]\n    #[cfg_attr(windows, ignore)]\n    fn unix() {}\n}\n"],
    ["c/src/lib.rs", "/// Doc.\n#[test]\nfn top() {}\n"],
    ["c/tests/cli.rs", "#[test]\nfn whole() {}\n"],
    ["c/tests/big/main.rs", "mod part;\n#[test]\nfn corpus() {}\n"],
    ["c/tests/big/part.rs", "#[test]\nfn piece() {}\n"],
  ]);
  const at = (p) => testsAt("c", p, unit);
  assert.deepEqual(at("m::tests::t"), [{ at: "c/src/m.rs:3", test: true, ignored: "no" }]);
  assert.deepEqual(at("m::tests::helper").map((f) => f.test), [false]);
  assert.deepEqual(at("m::tests::slow").map((f) => f.ignored), ["yes"]);
  assert.deepEqual(at("m::tests::native").map((f) => f.ignored), ["no"]);
  assert.deepEqual(at("m::tests::unix").map((f) => f.ignored), ["unknown"]);
  assert.deepEqual(at("m::t"), []);
  assert.deepEqual(at("top").map((f) => f.at), ["c/src/lib.rs:3"]);
  assert.deepEqual(at("whole").map((f) => f.at), ["c/tests/cli.rs:2"]);
  assert.deepEqual(at("corpus").map((f) => f.at), ["c/tests/big/main.rs:3"]);
  assert.deepEqual(at("part::piece").map((f) => f.at), ["c/tests/big/part.rs:2"]);
});

check("the toolchain is the release rust-toolchain.toml pins, from rustup, with nothing else", () => {
  assert.ok(!tracked().has("rust-toolchain"), "a rust-toolchain file beside rust-toolchain.toml: rustup reads it instead");
  const tables = tomlSubset(read("rust-toolchain.toml"), "rust-toolchain.toml");
  assert.deepEqual(tables.map((t) => t.name), ["toolchain"], "rust-toolchain.toml holds more than [toolchain]");
  const keys = tables[0].keys;
  assert.deepEqual([...keys.keys()].sort(), ["channel", "components", "profile"], "rust-toolchain.toml sets more than its channel, components and profile: a path names a toolchain no one pinned");
  assert.match(keys.get("channel"), /^"\d+\.\d+\.\d+"$/, "rust-toolchain.toml's channel is not a release");
  assert.equal(keys.get("components").replace(/\s+/g, ""), '["clippy","rustfmt"]', "rust-toolchain.toml's components changed");
  assert.equal(keys.get("profile"), '"minimal"', "rust-toolchain.toml's profile changed");
});

/// The digest a pin holds: SHA-256 of the text, Windows line ends read as Unix ones.
const digest = (text) => crypto.createHash("sha256").update(String(text).replace(/\r\n/g, "\n")).digest("hex");

/// A job as its pin reads it: the workflow's env and defaults with the whole job, keys in order,
/// each action named without the commit its pin names, which Dependabot moves and the check of
/// pins holds.
export function jobDigest(doc, job) {
  const canon = (v) =>
    Array.isArray(v)
      ? v.map(canon)
      : v !== null && typeof v === "object"
        ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, k === "uses" ? String(v[k]).replace(/@[0-9a-f]{40}$/, "@pinned") : canon(v[k])]))
        : v;
  return digest(JSON.stringify(canon({ env: doc.env ?? null, defaults: doc.defaults ?? null, job })));
}

/// What the tests run under, pinned whole: the scripts the testing jobs run, by their digests, and
/// the testing jobs themselves, every step's name, action, inputs, environment and script. A
/// change to any of them changes this file too, where review sees that a gate changed; the
/// failure names the digest to write here.
const PINNED_FILES = new Map([
  [".github/scripts/limits.sh", "8e51dda051399943b45b15751806fe171be9a5b6088e7ac581888661cba8a4e8"],
  [".github/scripts/memory-guard.sh", "2803d3fdd012923cd6c276c23db0af506596193b42b332903f5b4127f67fcfc1"],
  [".github/scripts/job-limits.ps1", "739fb90a85d859f593080cd32e4f818270afcc8cdb3f296a11caa58510accce4"],
  [".github/scripts/install-nextest.sh", "e031943ffadbe14c7ef5dfbbb249b8e4741f0afe7af824c6fb9a3ff2b7923d55"],
  [".github/scripts/test-list.mjs", "6aa8a0c532adf0a5ccef1e65b9a4a0f05090929db3be40d667d9ebefc828df09"],
  [".github/scripts/lib/test-list.mjs", "340a1b28b3422d2e3ca0fe1b32d5aa74d7f4494c6aef6f1260b5ca89a7d152f8"],
  [".github/scripts/tools.sh", "f0d0bb7e872fb66ee32a8739a3ca5abb5ae2690e1ce72b9c912399057a77b3ad"],
]);
const PINNED_JOBS = new Map([
  ["ci.yml#rust", "cbbdaa48bdc88bb60fcdf6777e2bdb1bf56d590c7278ffd81d49c40e2bf0acc8"],
  ["mutants.yml#in-diff", "20f57f8e9f73484617c9ee3e8fdf3310ffe28c713c9fa868c98e7e03239b970a"],
  ["mutants-full.yml#full", "51fa24fb8015464bf0ebfae542f4c390c88b098939fdce57cde88306e87b201c"],
]);
/// The step of the required Rust jobs that runs the tests: the shared limits first, then nextest's
/// own binary under the ci profile, with nothing after either that could filter, retry or forgive a
/// failure, and nothing on the step but its name and this script.
const NEXTEST_STEP = ". .github/scripts/limits.sh\ncargo-nextest nextest run --locked --workspace --profile ci\nbash .github/scripts/tools.sh verify\n";

/// The shell and PowerShell scripts under .github/scripts a text names by path.
const namedScripts = (text) => [...String(text).matchAll(/\.github\/scripts\/[\w./-]+\.(?:sh|ps1)\b/g)].map((m) => m[0]);

check("the scripts and jobs that run the tests are the ones the policy pins", () => {
  const changed = [];
  for (const [file, held] of PINNED_FILES) {
    const now = digest(read(file));
    if (now !== held) changed.push(`${file}: ${now}`);
  }
  for (const [key, held] of PINNED_JOBS) {
    const [file, id] = key.split("#");
    const w = workflows().find((x) => x.file === file);
    assert.ok(w?.doc.jobs?.[id], `${file} has no job ${id}`);
    const now = jobDigest(w.doc, w.doc.jobs[id]);
    if (now !== held) changed.push(`${key}: ${now}`);
  }
  assert.deepEqual(changed, [], "gates the policy pins changed; their digests now are the ones listed, which PINNED_FILES and PINNED_JOBS in .github/scripts/policy.test.mjs take in the same pull request");
  // Every script a pinned job runs is pinned, and so is every script a pinned script runs in turn.
  const pending = [];
  for (const key of PINNED_JOBS.keys()) {
    const [file, id] = key.split("#");
    const w = workflows().find((x) => x.file === file);
    for (const f of repoPaths(scriptTexts(w.doc, w.doc.jobs[id]))) if (/^\.github\/scripts\/[^/]+\.(sh|ps1)$/.test(f)) pending.push([key, f]);
  }
  for (const f of PINNED_FILES.keys()) pending.push(["PINNED_FILES", f]);
  const seen = new Set();
  while (pending.length > 0) {
    const [from, f] = pending.pop();
    if (seen.has(f)) continue;
    seen.add(f);
    assert.ok(PINNED_FILES.has(f), `${from} runs ${f}, which PINNED_FILES does not hold`);
    for (const g of namedScripts(read(f))) pending.push([f, g]);
    // A pinned module's imports are pinned too.
    if (f.endsWith(".mjs")) for (const g of repoPaths([`node ${f}`])) if (g !== f) pending.push([f, g]);
  }
  // A script the policy cannot name, its path made of a variable, could be any script.
  const variablePath = /\.github\/scripts\/[^\s"'`]*\$/;
  for (const f of PINNED_FILES.keys()) assert.doesNotMatch(read(f), variablePath, `${f} names a script under .github/scripts by a variable`);
  for (const key of PINNED_JOBS.keys()) {
    const [file, id] = key.split("#");
    const w = workflows().find((x) => x.file === file);
    for (const t of scriptTexts(w.doc, w.doc.jobs[id])) assert.doesNotMatch(t, variablePath, `${key} names a script under .github/scripts by a variable`);
  }
  assert.match('s=.github/scripts/$name.sh; bash "$s"', variablePath);
  // What the pin of a job reads and leaves out.
  const job = { steps: [{ uses: `actions/checkout@${"a".repeat(40)}` }, { run: "x" }] };
  assert.equal(jobDigest({}, job), jobDigest({}, { steps: [{ uses: `actions/checkout@${"b".repeat(40)}` }, { run: "x" }] }), "a moved action pin changes the digest");
  assert.notEqual(jobDigest({}, job), jobDigest({}, { steps: [{ uses: `evil/checkout@${"a".repeat(40)}` }, { run: "x" }] }));
  assert.notEqual(jobDigest({}, job), jobDigest({ env: { A: "1" } }, job));
  assert.equal(digest("a\r\nb\n"), digest("a\nb\n"));
  assert.deepEqual(namedScripts("bash .github/scripts/a.sh; pwsh -File .github/scripts/b.ps1 # .github/scripts/c.mjs"), [".github/scripts/a.sh", ".github/scripts/b.ps1"]);
});

check("the tests run through the pinned nextest under the shared limits, which no setting can lift", () => {
  for (const key of PINNED_JOBS.keys()) {
    const [file, id] = key.split("#");
    const w = workflows().find((x) => x.file === file);
    const job = w.doc.jobs[id];
    const all = steps(job);
    const installs = all.filter((s) => /nextest/.test(String(s.run ?? "")) && /install/.test(String(s.run ?? "")));
    assert.deepEqual(installs.map((s) => String(s.run).trim()), ["bash .github/scripts/install-nextest.sh"], `${key}: nextest is installed other than by install-nextest.sh`);
    for (const s of all) assert.doesNotMatch(String(s.run ?? ""), /GITHUB_ENV|GITHUB_PATH/, `${stepKey(file, id, s)} sets the environment of the steps after it`);
    // Every line that runs tests, nextest's or the mutants', comes after the step sources the
    // shared limits.
    for (const s of all.filter((x) => typeof x.run === "string")) {
      const lines = logicalLines(s.run).map((l) => l.trim());
      const source = lines.indexOf(". .github/scripts/limits.sh");
      for (const [k, l] of lines.entries()) {
        if (/^(cargo nextest run|cargo-nextest nextest run|cargo mutants (?!.*--list))\b/.test(l)) assert.ok(source >= 0 && source < k, `${stepKey(file, id, s)} runs tests outside the shared limits: ${l}`);
      }
    }
    // The tools are recorded right after nextest is installed, before any code of the change
    // runs, and every later step that runs a script checks them at its end, before a final exit.
    const install = all.findIndex((s) => String(s.run ?? "").trim() === "bash .github/scripts/install-nextest.sh");
    const record = all.findIndex((s) => String(s.run ?? "").trim() === "bash .github/scripts/tools.sh record");
    assert.ok(install >= 0 && record === install + 1, `${key}: the tools are not recorded right after nextest is installed`);
    for (const s of all.slice(record + 1).filter((x) => typeof x.run === "string")) {
      const lines = s.run.trimEnd().split("\n");
      const last = lines.at(-1) === 'exit "$failed"' ? lines.at(-2) : lines.at(-1);
      assert.equal(last, "bash .github/scripts/tools.sh verify", `${stepKey(file, id, s)} runs code of the change and does not check the tools at its end`);
    }
    // limits.sh is a bash script, sourced: the steps that source it run in bash, named, or by
    // default on a Linux runner.
    for (const s of all.filter((x) => /^\. \.github\/scripts\/limits\.sh$/m.test(String(x.run ?? "")))) {
      const shell = s.shell ?? job.defaults?.run?.shell ?? w.doc.defaults?.run?.shell;
      const linux = /^ubuntu-/.test(String(job["runs-on"]));
      assert.ok(shell === "bash" || (shell === undefined && linux), `${stepKey(file, id, s)} sources limits.sh in ${shell ?? "the runner's default shell"}, not bash`);
    }
  }
  // The required Rust jobs: the one step of the tests, exactly, in bash on every runner.
  const rustJob = workflows().find((w) => w.file === "ci.yml").doc.jobs.rust;
  assert.equal(rustJob.defaults?.run?.shell, "bash", "the Rust jobs do not run their steps in bash on every runner");
  const tests = steps(rustJob).filter((s) => /nextest\s+run/.test(String(s.run ?? "")));
  assert.equal(tests.length, 1, "the Rust jobs do not run the tests through nextest in one step");
  assert.equal(tests[0].run, NEXTEST_STEP, "the Rust jobs' tests do not run as NEXTEST_STEP in .github/scripts/policy.test.mjs reads them: the limits, then nextest under the ci profile; a change to the step changes NEXTEST_STEP too");
  assert.deepEqual(Object.keys(tests[0]).sort(), ["name", "run"], "the Rust jobs' step of the tests carries more than its name and script");
  // The mutation runs: their tests under the mutants profile, inside the same limits.
  for (const key of [...PINNED_JOBS.keys()].filter((k) => !k.startsWith("ci.yml#"))) {
    const [file, id] = key.split("#");
    const w = workflows().find((x) => x.file === file);
    assert.equal(w.doc.env?.NEXTEST_PROFILE, "mutants", `${file} runs its tests under another profile`);
    const mutate = steps(w.doc.jobs[id]).filter((s) => /^cargo mutants /m.test(String(s.run ?? "")));
    assert.ok(mutate.length > 0, `${file}: no step runs cargo mutants`);
    for (const s of mutate) {
      const body = String(s.run).split("\n");
      const source = body.indexOf(". .github/scripts/limits.sh");
      const first = body.findIndex((l) => /^cargo mutants /.test(l));
      assert.ok(source >= 0 && source < first, `${stepKey(file, id, s)}: cargo mutants runs outside the shared limits`);
    }
  }
});

check("limits.sh refuses an alias, a runner or a compiler from outside the repository", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "limits-"));
  try {
    const home = path.join(dir, "home");
    const other = path.join(dir, "other");
    fs.mkdirSync(home);
    fs.mkdirSync(other);
    fs.writeFileSync(path.join(other, "config.toml"), '[ alias ]\nnextest = "test --no-run --"\n');
    // Sourced on a runner the script knows no limits for, it stops either at a refusal or at the
    // case of the runner, after every refusal passed.
    const run = (vars, cargoHome = home) => {
      const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !/^(CARGO_|NEXTEST_|RUSTC|RUSTDOC)/i.test(k)));
      const r = spawnSync(bashPath(), ["-eo", "pipefail", "-c", ". .github/scripts/limits.sh"], { cwd: ROOT, encoding: "utf8", env: { ...env, ...vars, CARGO_HOME: cargoHome.replace(/\\/g, "/"), RUNNER_OS: "Elsewhere" }, timeout: 30000 });
      return `${r.stdout}${r.stderr}`;
    };
    assert.match(run({}), /defines no limits for Elsewhere/, "limits.sh refuses a clean environment");
    assert.match(run({ NEXTEST_PROFILE: "mutants" }), /defines no limits for Elsewhere/, "the mutation runs' profile is refused");
    assert.match(run({ RUSTFLAGS: "-D warnings" }), /defines no limits for Elsewhere/, "the Rust jobs' flags are refused");
    for (const vars of [
      { RUSTFLAGS: "-C overflow-checks=off" },
      { rustflags: "--cap-lints allow" },
      { CARGO_ENCODED_RUSTFLAGS: "x" },
      { CARGO_BUILD_RUSTFLAGS: "x" },
      { CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS: "x" },
      { RUSTDOCFLAGS: "x" },
      { RUSTUP_TOOLCHAIN: "nightly" },
      { BASH_ENV: "/tmp/x" },
      { CARGO_PROFILE_DEV_OVERFLOW_CHECKS: "false" },
      { CARGO_PROFILE_TEST_DEBUG_ASSERTIONS: "false" },
      { CARGO_ALIAS_NEXTEST: "test --no-run --" },
      { CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER: "true" },
      { cargo_target_x86_64_pc_windows_msvc_runner: "true" },
      { CARGO_BUILD_RUSTC_WRAPPER: "x" },
      { RUSTC_WRAPPER: "x" },
      { RUSTC: "x" },
      { NEXTEST_RETRIES: "3" },
      { NEXTEST_PROFILE: "default" },
    ]) {
      assert.match(run(vars), /::error::a variable above would change what the tests run/, `limits.sh lets through ${JSON.stringify(vars)}`);
    }
    assert.match(run({}, other), /::error::a cargo configuration file outside the repository/, "limits.sh lets through a cargo configuration file in CARGO_HOME");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("tools.sh finds a tool replaced or added, and a step that sets the next one's path", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "tools-"));
  try {
    const at = (...p) => path.join(dir, ...p);
    for (const d of [["home", "bin"], ["rustup", "toolchains", "t", "bin"], ["temp"]]) fs.mkdirSync(at(...d), { recursive: true });
    fs.writeFileSync(at("home", "bin", "cargo-nextest"), "nextest");
    fs.writeFileSync(at("rustup", "toolchains", "t", "bin", "cargo"), "cargo");
    fs.writeFileSync(at("path"), "");
    const slash = (p) => p.replace(/\\/g, "/");
    const run = (mode) => {
      const env = { ...process.env, CARGO_HOME: slash(at("home")), RUSTUP_HOME: slash(at("rustup")), RUNNER_TEMP: slash(at("temp")), GITHUB_PATH: slash(at("path")), GITHUB_ENV: "" };
      const r = spawnSync(bashPath(), [".github/scripts/tools.sh", mode], { cwd: ROOT, encoding: "utf8", env, timeout: 30000 });
      return { status: r.status, out: `${r.stdout}${r.stderr}` };
    };
    assert.equal(run("verify").status, 1, "tools.sh verifies without a record");
    assert.equal(run("record").status, 0, "tools.sh does not record");
    assert.equal(run("verify").status, 0, `tools.sh refuses unchanged tools: ${run("verify").out}`);
    fs.writeFileSync(at("home", "bin", "cargo-nextest"), "#!/bin/sh\nexit 1\n");
    assert.match(run("verify").out, /a tool changed since it was recorded/, "tools.sh lets a stub in place of nextest through");
    fs.writeFileSync(at("home", "bin", "cargo-nextest"), "nextest");
    fs.writeFileSync(at("rustup", "toolchains", "t", "bin", "rustc"), "rustc");
    assert.match(run("verify").out, /a tool changed since it was recorded/, "tools.sh lets a tool added to a toolchain through");
    fs.rmSync(at("rustup", "toolchains", "t", "bin", "rustc"));
    fs.writeFileSync(at("path"), "/tmp/stubs\n");
    assert.match(run("verify").out, /set the path or the environment of the steps after it/, "tools.sh lets a step set the path of the next one");
    fs.writeFileSync(at("path"), "");
    assert.equal(run("verify").status, 0, "tools.sh does not return to passing once the tools are as recorded");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("no required job restores a cache, and no job caches nextest", () => {
  // Another job of the same pull request, pinned or not, can save any entry under any key in the
  // pull request's scope, and a later run restores it.
  for (const { file, id, job } of requiredJobs()) {
    for (const s of steps(job)) assert.doesNotMatch(String(s.uses ?? ""), /^actions\/cache(\/restore)?@/, `${stepKey(file, id, s)}: a required job restores a cache another job could have written`);
  }
  for (const { file, id, job } of allJobs()) {
    for (const s of steps(job)) {
      if (/^actions\/cache(\/save|\/restore)?@/.test(String(s.uses ?? ""))) assert.doesNotMatch(String(s.with?.path ?? ""), /cargo-nextest/, `${stepKey(file, id, s)} caches nextest, which install-nextest.sh fetches by its digest`);
    }
  }
});

/// The variables that change how cargo, rustc, rustup, nextest, the shell, node or npm run what a
/// job checks, in any case; and those of them a required or pinned job may set, each with the one
/// value it may take (null: any) and the jobs that may set it (null: any job).
const TOOL_ENV = /^(CARGO_|NEXTEST_|RUSTC|RUSTDOC|RUSTUP_|RUSTFLAGS$|BASH_ENV$|ENV$|PATH$|SHELLOPTS$|BASHOPTS$|NODE_OPTIONS$|NODE_PATH$|NPM_CONFIG_|LD_PRELOAD$|LD_LIBRARY_PATH$)/i;
const TOOL_ENV_ALLOWED = new Map([
  ["CARGO_TERM_COLOR", { value: "always", jobs: null }],
  ["CARGO_MUTANTS_VERSION", { value: null, jobs: ["mutants.yml#in-diff", "mutants-full.yml#full"] }],
  // The fuzzer's release build also asserts what a debug build asserts.
  ["CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", { value: "true", jobs: ["fuzz.yml#fuzz"] }],
  // The base branch's build beside this change's.
  ["CARGO_TARGET_DIR", { value: "${{ github.workspace }}/../target", jobs: ["perf.yml#kernels"] }],
  ["NEXTEST_PROFILE", { value: "mutants", jobs: ["mutants.yml#in-diff", "mutants-full.yml#full"] }],
  // A warning fails the build.
  ["RUSTFLAGS", { value: "-D warnings", jobs: ["ci.yml#rust"] }],
]);

check("no required or pinned job sets a variable that changes how its tools run", () => {
  const keys = new Set([...requiredJobs().map((j) => `${j.file}#${j.id}`), ...PINNED_JOBS.keys()]);
  for (const { file, doc, id, job } of allJobs().filter((j) => keys.has(`${j.file}#${j.id}`))) {
    for (const env of [doc.env, job.env, ...steps(job).map((s) => s.env)].filter(Boolean)) {
      for (const [name, value] of Object.entries(env)) {
        if (!TOOL_ENV.test(name)) continue;
        const held = TOOL_ENV_ALLOWED.get(name);
        const allowed = held !== undefined && (held.value === null || held.value === String(value)) && (held.jobs === null || held.jobs.includes(`${file}#${id}`));
        assert.ok(allowed, `${file}#${id} sets ${name}=${value}, which changes how cargo, rustc, nextest, the shell or node run what the job checks; TOOL_ENV_ALLOWED names the jobs that may set it, and its value`);
      }
    }
  }
  for (const name of ["BASH_ENV", "npm_config_script_shell", "NODE_OPTIONS", "PATH", "Path", "cargo_target_x86_64_unknown_linux_gnu_runner", "RUSTC_WRAPPER", "RUSTUP_TOOLCHAIN"]) assert.match(name, TOOL_ENV, name);
  assert.doesNotMatch("MENTE_GEN_PROGRAMS", TOOL_ENV);
});

/// The workspace members that may run code at build time, a build script or a procedural macro,
/// each with its reason. That code runs before any limit of the tests, with the job's rights.
const BUILD_TIME_CODE = new Map([]);
/// A manifest's `build =` that names a script, the key bare or quoted, and any mention of a
/// procedural macro, `proc-macro = true` or a `crate-type` of one, comments aside.
const buildScriptKey = /^\s*["']?build["']?\s*=(?!\s*false\b)/m;
const procMacro = /proc[-_]macro/;
/// A doc attribute in any spelling, plain, spaced or under a cfg_attr.
const docAttribute = /#!?\[\s*(doc\s*=|cfg_attr\b[^\]]*\bdoc\s*=)/;

check("no workspace member runs code at build time", () => {
  for (const [name, { dir }] of workspace()) {
    const manifest = read(`${dir}/Cargo.toml`);
    const has = [];
    if (tracked().has(`${dir}/build.rs`)) has.push("a build.rs");
    if (buildScriptKey.test(manifest)) has.push("a build script `build =` names");
    if (procMacro.test(manifest.replace(/#.*$/gm, ""))) has.push("a procedural macro");
    if (has.length > 0) assert.ok(BUILD_TIME_CODE.has(name), `${name} has ${has.join(" and ")}, code that runs at build time, before the tests' limits and with the job's rights: BUILD_TIME_CODE in .github/scripts/policy.test.mjs names the members allowed one`);
  }
  for (const m of ['build = "x.rs"', '"build" = "x.rs"', "'build' = 'x.rs'"]) assert.match(m, buildScriptKey, m);
  assert.doesNotMatch("build = false", buildScriptKey);
  for (const m of ["proc-macro = true", 'crate-type = ["proc-macro"]', "proc_macro = true"]) assert.match(m, procMacro, m);
});

/// The code blocks of a library's doc comments rustdoc compiles as doctests, as `file:line`: a
/// fenced block whose info string names no other language than Rust and does not say `ignore`, and
/// an indented block (four spaces after a blank doc line), which rustdoc reads as Rust.
export function doctestBlocks(file, text) {
  const found = [];
  let fence = null;
  let blank = true;
  const ATTRIBUTE = /^(rust|ignore(-.+)?|no_run|compile_fail|should_panic|test_harness|edition\d{4}|standalone_crate|allow_fail|E\d{4})$/;
  for (const [k, raw] of text.split(/\r?\n/).entries()) {
    const doc = /^\s*\/\/[/!](?![/!]) ?(.*)$/.exec(raw);
    if (!doc) {
      fence = null;
      blank = true;
      continue;
    }
    const body = doc[1];
    const marker = /^\s*(`{3,}|~{3,})\s*(.*)$/.exec(body);
    if (fence !== null) {
      if (marker && marker[1][0] === fence[0] && marker[1].length >= fence.length && marker[2] === "") fence = null;
      continue;
    }
    if (marker) {
      fence = marker[1];
      const tokens = marker[2].split(/[\s,]+/).filter(Boolean).map((t) => t.replace(/^\{|\}$/g, ""));
      const rust = tokens.includes("rust") || tokens.every((t) => ATTRIBUTE.test(t));
      if (rust && !tokens.includes("ignore")) found.push(`${file}:${k + 1}`);
    } else if (blank && /^ {4,}\S/.test(body)) {
      found.push(`${file}:${k + 1}`);
    }
    blank = body.trim() === "";
  }
  return found;
}

check("no library's doc comments hold an example rustdoc would compile", () => {
  // nextest runs no doctests, and `cargo test --doc` has no limit per test: an example belongs in a
  // test, or in a block marked `text` or `ignore`.
  // The required Rust jobs list the doctests rustdoc finds, and fail on any; this reads the
  // sources first, so that an example fails here too, and names where it is.
  const found = [];
  for (const [, { dir }] of workspace()) {
    for (const f of filesUnder(dir).filter((x) => x.endsWith(".rs"))) {
      const text = read(f);
      // A module read from another path could be one these lines never read.
      if (/#!?\[\s*path\b/.test(text)) found.push(`${f}: a #[path] module`);
      if (!tracked().has(`${dir}/src/lib.rs`) || !f.startsWith(`${dir}/src/`) || f === `${dir}/src/main.rs` || f.startsWith(`${dir}/src/bin/`)) continue;
      found.push(...doctestBlocks(f, text));
      if (docAttribute.test(text)) found.push(`${f}: a doc attribute, whose text the policy does not read`);
      if (/\/\*[*!](?![*/])/.test(text)) found.push(`${f}: a block doc comment, which the policy does not read`);
    }
  }
  assert.deepEqual(found, [], "doc comments hold examples rustdoc compiles: mark each `text` or `ignore`, or make it a test");
  for (const a of ['#[doc = "x"]', '#![doc = include_str!("a.md")]', '#[ doc = "x"]', '#[cfg_attr(all(), doc = "x")]']) assert.match(a, docAttribute, a);
  assert.doesNotMatch("#[doc(hidden)]", docAttribute);
  const blocks = (text) => doctestBlocks("f.rs", text).map((at) => Number(at.split(":")[1]));
  assert.deepEqual(blocks("/// a\n/// ```\n/// x\n/// ```\n"), [2]);
  assert.deepEqual(blocks("//! ```rust,no_run\n//! x\n//! ```\n"), [1]);
  assert.deepEqual(blocks("/// ```should_panic\n/// ```\n/// ```edition2021\n/// ```\n"), [1, 3]);
  assert.deepEqual(blocks("/// ```text\n/// x\n/// ```\n/// ```ignore\n/// ```\n/// ```mn\n/// ```\n/// ~~~sh\n/// ~~~\n"), []);
  assert.deepEqual(blocks("/// a\n///\n///     let x = 1;\n"), [3]);
  assert.deepEqual(blocks("/// - a\n///   b\n//// ```\n/// ```text\n///     x\n/// ```\n"), []);
});

check("the list of tests is read as nextest writes it", () => {
  const list = (cases) => ({ "rust-suites": Object.fromEntries(cases.map(([pkg, bin, names], k) => [`${bin}${k}`, { "package-name": pkg, "binary-id": bin, testcases: Object.fromEntries(names.map(([n, ignored]) => [n, { ignored, "filter-match": { status: "matches" } }])) }])) });
  const toml = '(package(a) & test(=m::slow))\n| (package(b) & test(=slow))\n';
  const ok = list([["a", "a", [["m::slow", false], ["m::quick", false], ["m::later", true]]], ["b", "b::it", [["slow", false]]]]);
  assert.deepEqual(testListProblems(ok, "# ignored\na m::later\n", toml), []);
  assert.equal(testListProblems(ok, "", toml).length, 1, "an ignored test the list does not name");
  assert.equal(testListProblems(ok, "a m::later\na m::quick\n", toml).length, 1, "a listed test that is not ignored");
  assert.equal(testListProblems(list([["a", "a", [["m::slow", true], ["m::later", true]]], ["b", "b::it", [["slow", false]]]]), "a m::later\na m::slow\n", toml).length, 1, "a slow test that is ignored");
  assert.equal(testListProblems(list([["a", "a", [["m::later", true]]], ["b", "b::it", [["slow", false]]]]), "a m::later\n", toml).length, 1, "a slow test compiled out");
  assert.equal(testListProblems(list([]), "", "").length, 1, "an empty list");
  assert.deepEqual(slowTests(read(".config/nextest.toml")).sort(), SLOW_TESTS.map(([p, t]) => `${p} ${t}`).sort(), "lib/test-list.mjs reads the override's tests as the policy does");
  // The doctests' list, as `cargo test --doc -- --list` prints it.
  const none = "   Doc-tests mente_ast\n0 tests, 0 benchmarks\n   Doc-tests mente_mir\n0 tests, 0 benchmarks\n";
  assert.deepEqual(doctestProblems(none), []);
  // As the jobs print it, under CARGO_TERM_COLOR=always: the header coloured.
  assert.deepEqual(doctestProblems("\x1b[1m\x1b[32m   Doc-tests\x1b[0m mente_ast\n0 tests, 0 benchmarks\n"), []);
  assert.equal(doctestProblems(`${none}   Doc-tests mente_sema\ncompiler/mente-sema/src/lib.rs - f (line 3): test\n1 test, 0 benchmarks\n`).length, 1, "a library with a doctest");
  assert.equal(doctestProblems("   Doc-tests mente_ast\n   Doc-tests mente_mir\n0 tests, 0 benchmarks\n").length, 1, "a library without its count");
  assert.equal(doctestProblems("   Compiling mente-ast\n").length, 1, "no library at all");
});

/// The bash the policy runs scripts in: on Windows, Git's, beside the git the policy runs, since a
/// bare `bash` there may be WSL's.
function bashPath() {
  if (process.platform !== "win32") return "bash";
  const found = path.resolve(git(["--exec-path"]).trim(), "../../../usr/bin/bash.exe");
  assert.ok(fs.existsSync(found), `no bash beside git at ${found}`);
  return found;
}

check("the memory guard goes on when the process it would stop is already gone", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "memory-guard-"));
  try {
    const meminfo = path.join(dir, "meminfo");
    fs.writeFileSync(meminfo, "MemTotal: 16384000 kB\nMemAvailable: 1024 kB\n");
    // Sourced in a subshell of a shell under errexit and pipefail, as the guard ran when it lost
    // itself, with `ps` naming a process that is gone, so that every `kill` fails, and `sleep`
    // counting the rounds instead of waiting: it lets five through and ends the loop at the sixth.
    // No clock decides: the guard either goes on through all five, and the subshell says so after
    // it, or stops at the first failed `kill`.
    const ROUNDS = 5;
    const script = `rounds=0; sleep() { rounds=$((rounds + 1)); [ "$rounds" -le ${ROUNDS} ]; }; ps() { echo "999999 1"; }; (. .github/scripts/memory-guard.sh "$1"; echo "the guard went on through $((rounds - 1)) rounds")`;
    const r = spawnSync(bashPath(), ["-eo", "pipefail", "-c", script, "guard", meminfo.replace(/\\/g, "/")], { cwd: ROOT, encoding: "utf8", timeout: 30000 });
    const rounds = (String(r.stdout).match(/^memory guard: 1 MB available, stopping /gm) ?? []).length;
    assert.equal(rounds, ROUNDS, `the guard stopped after ${rounds} of ${ROUNDS} rounds: ${r.error ?? ""}${r.stdout}${r.stderr}`);
    assert.match(String(r.stdout), new RegExp(`^the guard went on through ${ROUNDS} rounds$`, "m"), `the guard did not go on after its rounds: ${r.stdout}${r.stderr}`);
    assert.equal(r.status, 0, `the guard's shell failed: ${r.error ?? ""}${r.stderr}`);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
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

/// Whether a value hands a secret over: an expression naming `secrets.` other than a test of
/// whether one is set.
const holdsSecret = (value) =>
  [...String(value).matchAll(/\$\{\{([\s\S]*?)\}\}/g)].some(([, e]) => /\bsecrets\./.test(e) && !/^\s*secrets\.\w+\s*[!=]=\s*''\s*$/.test(e));

check("a job that holds a secret runs agent code only in its proven sandbox", () => {
  const crates = workspace();
  let guarded = 0;
  for (const { file, doc, id, job } of allJobs()) {
    const all = steps(job);
    const wide = [doc.env, job.env].some((env) => env && Object.values(env).some(holdsSecret));
    const secretAt = all.findIndex((s) => wide || [s.env, s.with].some((m) => m && Object.values(m).some(holdsSecret)));
    if (secretAt < 0 || !cratesBuilt(scriptTexts(doc, job), crates).names.has("mente-cli")) continue;
    guarded++;
    const where = `${file}, job ${id}`;
    const check = all.findIndex((s) => /benchmark\/harness\/sandbox-check\.ts/.test(String(s.run ?? "")));
    assert.ok(check >= 0 && check < secretAt, `${where}: holds a secret and builds mn, but sandbox-check.ts does not run before the secret is in reach`);
    for (const key of ["if", "continue-on-error"]) assert.ok(!(key in all[check]), `${where}: the sandbox check carries ${key}:`);
    for (const s of all.slice(secretAt).filter((s) => [s.env, s.with].some((m) => m && Object.values(m).some(holdsSecret)) && typeof s.run === "string")) {
      assert.match(s.run, /--sandbox "\$SANDBOX"/, `${where}, step "${s.name}": holds a secret and runs nothing through the sandbox`);
    }
  }
  assert.ok(guarded >= 2, "the benchmark and the preliminary runs hold the key beside mn");
});

check("the Documentation job lints every workflow with actionlint and shellcheck, both pinned by checksum", () => {
  const job = workflows().find((w) => w.file === "ci.yml").doc.jobs.docs;
  const env = Object.assign({}, ...steps(job).map((s) => s.env ?? {}));
  for (const tool of ["ACTIONLINT", "SHELLCHECK"]) {
    assert.match(String(env[`${tool}_VERSION`] ?? ""), /^\d+\.\d+\.\d+$/, `${tool} is not pinned to a version`);
    assert.match(String(env[`${tool}_SHA256`] ?? ""), /^[0-9a-f]{64}$/, `${tool}'s download has no checksum`);
  }
  const runs = steps(job).map((s) => String(s.run ?? "")).join("\n");
  assert.equal((runs.match(/sha256sum -c -/g) ?? []).length, 2, "both downloads are checked against their checksums");
  assert.match(runs, /"\$RUNNER_TEMP\/lint\/actionlint" -color -shellcheck "\$RUNNER_TEMP\/lint\/shellcheck"/, "actionlint runs with the pinned shellcheck");
});

/// The owner of the gate files, and the gate files, as the patterns of .github/CODEOWNERS: what
/// decides whether a change may merge, which a pull request could otherwise weaken unseen.
const OWNER = "@Dancraft3746";
const GATE_FILES = [
  "/.github/",
  "/.cargo/",
  "/Cargo.toml",
  "/Cargo.lock",
  "/rust-toolchain.toml",
  "/rust-toolchain",
  "/.config/",
  "/clippy.toml",
  "/package.json",
  "/package-lock.json",
  "/.spec-guard.json",
  "/.spec-graph.json",
  "/.spec-brief.json",
  "/perf/run.mjs",
  "/perf/lib.mjs",
  "/benchmark/check.mjs",
  "/benchmark/corpus.lock",
  "/benchmark/harness/sandbox.sh",
  "/benchmark/harness/sandbox-check.ts",
  "/benchmark/harness/build.ts",
  "/benchmark/harness/bench.ts",
  "/benchmark/harness/prelim.ts",
  "/benchmark/harness/probe.ts",
  "/benchmark/harness/fixtures.ts",
  "/benchmark/harness/toolchains.json",
  "/compiler/mente-cli/tests/gates.rs",
  "/compiler/*/src/overflow_traps.rs",
  "/compiler/*/Cargo.toml",
  "/spikes/*/Cargo.toml",
];

/// Whether a CODEOWNERS pattern of the forms GATE_FILES uses owns a file: `/dir/` every file under
/// it, `/file` that file, and `*` any name within one directory.
const owns = (pattern, file) => {
  const body = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, "[^/]*");
  return new RegExp(`^${body}${pattern.endsWith("/") ? ".*" : ""}$`).test(`/${file}`);
};

check("CODEOWNERS gives the gate files, and only them, to their owner", () => {
  const entries = read(".github/CODEOWNERS")
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l !== "" && !l.startsWith("#"))
    .map((l) => l.split(/\s+/));
  for (const [pattern, ...owners] of entries) assert.deepEqual(owners, [OWNER], `${pattern} is owned by ${owners.join(" ") || "nobody"}`);
  assert.deepEqual(entries.map(([p]) => p).sort(), [...GATE_FILES].sort(), ".github/CODEOWNERS and GATE_FILES differ");
  // Every file the policy reads as a gate input has an owner: the workflows and their scripts,
  // the lists, the manifests, the lock and its approvals, the CLIs the required jobs run.
  const inputs = [
    ...filesUnder(".github"),
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    ".config/approved-crates.txt",
    ".config/nextest.toml",
    ".cargo/mutants.toml",
    "package.json",
    "package-lock.json",
    "perf/run.mjs",
    "perf/lib.mjs",
    "benchmark/check.mjs",
    "benchmark/harness/sandbox.sh",
    "benchmark/harness/sandbox-check.ts",
    "benchmark/harness/build.ts",
    "benchmark/harness/bench.ts",
    "benchmark/harness/prelim.ts",
    "benchmark/harness/probe.ts",
    "benchmark/harness/toolchains.json",
    "compiler/mente-cli/tests/gates.rs",
    ...[...tracked()].filter((f) => /^compiler\/[^/]+\/src\/overflow_traps\.rs$/.test(f)),
    // Every workspace member's manifest, wherever the member is.
    ...[...workspace().values()].map(({ dir }) => `${dir}/Cargo.toml`),
  ];
  for (const f of inputs) {
    assert.ok(tracked().has(f), `${f}, a gate input, is not in the repository`);
    assert.ok(GATE_FILES.some((p) => owns(p, f)), `${f} is a gate input without an owner in .github/CODEOWNERS`);
  }
  assert.ok(owns("/.github/", ".github/workflows/ci.yml") && !owns("/perf/run.mjs", "perf/run.test.mjs") && owns("/Cargo.toml", "Cargo.toml"));
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

// ------------------------------------------------------------------------- the required checks

check("the required checks are listed once each", () => {
  const names = requiredChecks();
  assert.ok(names.length > 0);
  const twice = names.filter((n, k) => names.indexOf(n) !== k);
  assert.deepEqual(twice, [], `${REQUIRED_FILE} lists these twice`);
});

check("no two jobs of any workflow report a check of the same name", () => {
  // Whatever triggers it: a scheduled or dispatched job of a required check's name could land a
  // green check of that name on the commit.
  const seen = new Map();
  for (const { file, id, names } of allJobs()) {
    for (const name of names) {
      assert.ok(!seen.has(name), `the check "${name}" is reported by ${seen.get(name)} and by ${file}#${id}: a required check of that name would be ambiguous`);
      seen.set(name, `${file}#${id}`);
    }
  }
});

/// The events a workflow holding a required job may run on: a pull request, the merge queue and a
/// push to main. A dispatched or scheduled run, or one with inputs, could report the job's check
/// on a pull request's head with the work it chose.
const REQUIRED_EVENTS = new Set(["pull_request", "merge_group", "push"]);

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

/// The gate commands that run inside a compound command of their step, each with its reason.
const NESTED_GATES = new Map([
  ["mutants.yml#in-diff: node .github/scripts/mutants-floor.mjs missing pr.diff base-all-mutants.json diff-mutants.json deleted-mutants.json > missing.txt", "shard 0 alone checks the floor of the whole diff"],
]);

/// The logical lines of a script: each continued line joined to the next by one space.
const logicalLines = (script) => String(script).replace(/[ \t]*\\\r?\n[ \t]*/g, " ").split("\n");

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
  for (const key of declared.keys()) {
    assert.ok(required.some(({ file, id }) => `${file}#${id}` === key), `.github/gate-commands.txt names ${key}, which is no required job`);
  }
  for (const { file, doc, id, job } of required) {
    const key = `${file}#${id}`;
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
        const pipefail = [s.shell, job.defaults?.run?.shell, doc.defaults?.run?.shell].find((x) => x !== undefined) === "bash";
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
  ]) {
    assert.ok(verdict(script), `the line rule lets through: ${script}`);
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
  ["mutants.yml#in-diff", 360],
  ["generated.yml#paths", 120],
  ["smt.yml#z3", 45],
  ["fuzz.yml#fuzz", 30],
  ["mir-mutants.yml#gate", 30],
  ["perf.yml#kernels", 60],
  ["harness.yml#fixtures", 30],
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
        if (!/\bcargo\s+test\b.*\s--ignored\b/.test(line)) continue;
        found++;
        assert.match(line, new RegExp(`(^|[\\s(&;])timeout ${IGNORED_TEST_LIMIT} cargo test\\b`), `${stepKey(file, id, s)} runs ignored tests without their limit: timeout ${IGNORED_TEST_LIMIT} cargo test ...`);
        // GNU timeout is bash's on every runner; Windows' own shell finds timeout.exe, which only
        // waits. A job's default shell is bash only on a Linux runner named as such.
        const shell = s.shell ?? job.defaults?.run?.shell ?? doc.defaults?.run?.shell;
        const linux = /^ubuntu-/.test(String(job["runs-on"]));
        assert.ok(shell === "bash" || (shell === undefined && linux), `${stepKey(file, id, s)} runs timeout in ${shell ?? "the runner's default shell"}, not bash`);
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

check("every required check is one job that runs on every pull request and in the merge queue", () => {
  const jobs = allJobs();
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
    assert.ok(!("needs" in job), `${where}: a required job with needs: is skipped, and so passes, when a job it needs fails`);
    assert.ok(!("continue-on-error" in job), `${where}: a required job may not continue on error`);
    if ("if" in job) assert.equal(normalize(job.if), REQUIRED_JOB_IF, `${where}: a job-level if: could skip it`);
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

check("every step runs in a shell named bare", () => {
  for (const { file, doc } of workflows()) {
    const shell = doc.defaults?.run?.shell;
    if (shell !== undefined) assert.ok(SHELLS.has(shell), `${file}: defaults.run.shell: ${shell}`);
    for (const [id, job] of Object.entries(doc.jobs ?? {})) {
      const jobShell = job.defaults?.run?.shell;
      if (jobShell !== undefined) assert.ok(SHELLS.has(jobShell), `${file}, job ${id}: defaults.run.shell: ${jobShell}`);
      for (const s of steps(job)) {
        if ("shell" in s) assert.ok(SHELLS.has(s.shell), `${stepKey(file, id, s)}: shell: ${s.shell}`);
      }
    }
  }
});

check("the matrix names a check as GitHub does", () => {
  assert.deepEqual(checkNames("a", { name: "Fuzz", strategy: { matrix: { include: [{ name: "x86-64 Linux" }] } } }), ["Fuzz (x86-64 Linux)"]);
  assert.deepEqual(checkNames("a", { name: "R (${{ matrix.name }})", strategy: { matrix: { include: [{ name: "L", os: "u" }] } } }), ["R (L)"]);
  assert.deepEqual(checkNames("job", { strategy: { matrix: { shard: [0, 1] } } }), ["job (0)", "job (1)"]);
  assert.deepEqual(checkNames("job", {}), ["job"]);
});

// ------------------------------------------------------------------ the watch and its path lists

check("every instrument workflow decides at its first step, and every later step obeys", () => {
  const sections = parsePaths(read(PATHS_FILE));
  const required = new Set(requiredChecks());
  for (const { file, id, job, names } of allJobs()) {
    const all = steps(job);
    if (!hasWatch(job)) {
      assert.ok(!sections.has(file) || !names.some((n) => required.has(n)), `${file}, job ${id}: a required job of an instrument workflow with no watch step`);
      continue;
    }
    const where = `${file}, job ${id}`;
    assert.ok(sections.has(file), `${where}: a watch step, but ${PATHS_FILE} has no [${file}]`);
    assert.match(String(all[0].uses ?? ""), /^actions\/checkout@/, `${where}: the first step is not the checkout`);
    assert.equal(all[0].with?.["fetch-depth"], 0, `${where}: the checkout needs fetch-depth: 0 for the diff`);
    const watch = all[1];
    assert.equal(watch?.id, "watch", `${where}: the step after the checkout is not the watch`);
    assert.equal(String(watch.run).trim(), `node .github/scripts/watch.mjs ${file}`, `${where}: the watch step reads another list`);
    assert.deepEqual(Object.keys(watch).sort(), ["id", "name", "run"], `${where}: the watch step carries more than its name, id and command`);
    for (const s of all.slice(2)) {
      const condition = normalize(s.if ?? "");
      assert.ok(
        WATCH_FORMS.includes(condition) || (REPORT_FORMS.includes(condition) && isReport(file, id, s)),
        `${where}, step "${s.name ?? s.uses ?? s.run}": its if: is not one of the watch's forms`,
      );
    }
  }
  for (const file of sections.keys()) {
    assert.ok(allJobs().some((j) => j.file === file && steps(j.job)[1]?.id === "watch"), `${PATHS_FILE}: no job of ${file} reads [${file}]`);
  }
});

/// The workspace's members, each with the members it depends on by path.
function workspace() {
  const root = read("Cargo.toml");
  // Every member, in either quote, comments aside; a glob is refused, since cargo would expand it
  // to members this list never names.
  const list = (/members\s*=\s*\[([\s\S]*?)\]/.exec(root)?.[1] ?? "").replace(/#.*$/gm, "");
  const members = [...list.matchAll(/"([^"]+)"|'([^']+)'/g)].map((m) => m[1] ?? m[2]);
  assert.ok(members.length > 0, "Cargo.toml lists no workspace members");
  for (const m of members) assert.doesNotMatch(m, /[*?[]/, `Cargo.toml's workspace member ${m} is a glob`);
  const crates = new Map();
  for (const dir of members) {
    const manifest = read(`${dir}/Cargo.toml`);
    const name = /^\s*name\s*=\s*"([^"]+)"/m.exec(manifest)?.[1];
    const deps = [...manifest.matchAll(/^\s*([\w-]+)\s*=\s*\{[^}]*\bpath\s*=\s*"([^"]+)"/gm)].map((m) => path.posix.normalize(path.posix.join(dir, m[2])));
    crates.set(name, { dir, deps });
  }
  return crates;
}

/// The directories of the crates a set of crates builds: themselves and every member they reach.
function closure(crates, names) {
  const byDir = new Map([...crates.values()].map((c) => [c.dir, c]));
  const out = new Set();
  const visit = (dir) => {
    if (out.has(dir)) return;
    out.add(dir);
    for (const d of byDir.get(dir)?.deps ?? []) visit(d);
  };
  for (const n of names) {
    const c = crates.get(n);
    assert.ok(c, `a workflow builds ${n}, which is not a workspace member`);
    visit(c.dir);
  }
  return out;
}

/// The texts a job's scripts read: every run: script, every env value (the workflow's, the job's,
/// each step's), and the package.json script behind every `npm run`, recursively.
function scriptTexts(doc, job) {
  const texts = [];
  const scripts = JSON.parse(read("package.json")).scripts ?? {};
  const addNpm = (text, seen) => {
    for (const m of text.matchAll(/\bnpm run ([\w:-]+)/g)) {
      if (seen.has(m[1]) || !(m[1] in scripts)) continue;
      seen.add(m[1]);
      texts.push(scripts[m[1]], "package.json");
      addNpm(scripts[m[1]], seen);
    }
  };
  const envs = [doc.env, job.env, ...steps(job).map((s) => s.env)].filter(Boolean);
  for (const env of envs) texts.push(...Object.values(env).map(String));
  for (const s of steps(job)) {
    if (typeof s.run !== "string") continue;
    texts.push(s.run);
    addNpm(s.run, new Set());
  }
  return texts;
}

/// The crates a job's scripts build: those `-p` names, `mente-cli` for `--bin mn`, and every
/// member for a cargo command that names neither.
function cratesBuilt(texts, crates) {
  const names = new Set();
  let cargo = false;
  for (const text of texts) {
    for (const command of text.replace(/\\\n/g, " ").split(/\n|&&|\|\||;|\|/)) {
      const m = /\bcargo\s+(build|test|run|nextest|mutants|clippy|llvm-cov|check)\b(.*)$/.exec(command);
      if (!m) continue;
      cargo = true;
      const packages = [...m[2].matchAll(/(?:-p|--package)\s+([\w-]+)/g)].map((p) => p[1]);
      if (packages.length > 0) packages.forEach((p) => names.add(p));
      else if (/--bin\s+mn\b/.test(m[2])) names.add("mente-cli");
      else [...crates.keys()].forEach((c) => names.add(c));
    }
  }
  return { cargo, names };
}

/// The tracked files the texts name: a file, every file under a directory, every file a glob
/// matches, and, for a script, every file it imports, recursively. Shell comments are left out.
export function repoPaths(texts) {
  const files = tracked();
  const out = new Set();
  const addScript = (file) => {
    if (out.has(file)) return;
    out.add(file);
    if (!/\.(mjs|cjs|js|ts)$/.test(file)) return;
    for (const m of read(file).matchAll(/(?:\bfrom\s+|\bimport\s*\(\s*|^\s*import\s+)["'](\.{1,2}\/[^"']+)["']/gm)) {
      const target = path.posix.normalize(path.posix.join(path.posix.dirname(file), m[1]));
      if (files.has(target)) addScript(target);
    }
  };
  for (const text of texts) {
    const code = text
      .split("\n")
      .filter((l) => !/^\s*#/.test(l))
      .map((l) => l.replace(/\s#\s.*$/, ""))
      .join("\n");
    for (let token of code.split(/[\s;|&()<>"'`=,{}]+/)) {
      token = token.replace(/^\.\//, "").replace(/\/+$/, "");
      if (token === "" || token.includes("$") || token.startsWith("-") || token.startsWith("/")) continue;
      if (token.includes("*")) {
        // A glob names paths only with a directory in it: a bare `*)` is a shell case's default.
        if (!/[\w.-]\//.test(token)) continue;
        const re = globToRegExp(token);
        [...files].filter((f) => re.test(f)).forEach(addScript);
      } else if (files.has(token)) addScript(token);
      else filesUnder(token).forEach(addScript);
    }
  }
  return out;
}

/// The files the crates in these directories compile in with `include_str!` or `include_bytes!`,
/// wherever they lie: the language card is part of `mn`.
export function includedFiles(dirs) {
  const out = new Set();
  for (const dir of dirs) {
    for (const f of filesUnder(dir).filter((f) => f.endsWith(".rs"))) {
      for (const m of read(f).matchAll(/include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)/g)) {
        const target = path.posix.normalize(path.posix.join(path.posix.dirname(f), m[1]));
        assert.ok(tracked().has(target), `${f} includes ${m[1]}, which is not in the repository`);
        out.add(target);
      }
    }
  }
  return out;
}

/// The corpora the compiler reads for an instrument, each from the item of the source that names
/// it: their string literals that are directories of the repository, as written, below `../..`,
/// or below `benchmark/`, the most specific of them.
const CODE_CORPORA = {
  "fuzz.yml": [["compiler/mente-cli/src/fuzz.rs", "const SEED_DIRS"]],
  "generated.yml": [["compiler/mente-cli/src/mir_run/differential.rs", "fn conformance_programs"]],
  "mir-mutants.yml": [["compiler/mente-cli/src/mir_mutants.rs", "fn agent_programs"]],
};

function rustItem(file, marker) {
  const text = read(file);
  const at = text.indexOf(marker);
  assert.ok(at >= 0, `${file}: \`${marker}\` is gone; the instrument reading it needs its corpus found anew here`);
  const from = marker.startsWith("const") ? text.indexOf("=", at) : at;
  const open = from + text.slice(from).search(/[[{]/);
  let depth = 0;
  let inString = false;
  for (let k = open; k < text.length; k++) {
    const c = text[k];
    if (inString) {
      if (c === "\\") k++;
      else if (c === '"') inString = false;
    } else if (c === '"') inString = true;
    else if (c === "[" || c === "{") depth++;
    else if ((c === "]" || c === "}") && --depth === 0) return text.slice(open, k + 1);
  }
  throw new Error(`${file}: \`${marker}\` does not close`);
}

export function codeCorpus(file, marker) {
  const dirs = new Set();
  for (const [, literal] of rustItem(file, marker).matchAll(/"((?:[^"\\]|\\.)*)"/g)) {
    for (const candidate of [literal, literal.replace(/^(\.\.\/)+/, ""), `benchmark/${literal}`]) {
      if (candidate !== "" && isDir(candidate)) dirs.add(candidate);
    }
  }
  return [...dirs].filter((d) => ![...dirs].some((e) => e !== d && e.startsWith(`${d}/`)));
}

check("the path lists read as the watch reads them", () => {
  const sections = parsePaths(read(PATHS_FILE));
  assert.ok(sections.size > 0);
  for (const [file, section] of sections) {
    assert.ok(fs.existsSync(path.join(ROOT, WORKFLOWS, file)), `${PATHS_FILE}: [${file}] is not a workflow`);
    for (const p of section.patterns) {
      assert.ok([...tracked()].some((f) => globToRegExp(p).test(f)), `${PATHS_FILE}, [${file}]: ${p} matches no file of the repository`);
    }
  }
});

check("every path list names everything its jobs build, run and read", () => {
  const sections = parsePaths(read(PATHS_FILE));
  const crates = workspace();
  for (const [file, section] of sections) {
    const needed = new Set([`${WORKFLOWS}/${file}`]);
    for (const { doc, job } of allJobs().filter((j) => j.file === file)) {
      const texts = scriptTexts(doc, job);
      const { cargo, names } = cratesBuilt(texts, crates);
      if (cargo) {
        for (const f of ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"]) needed.add(f);
        const dirs = closure(crates, names);
        for (const dir of dirs) filesUnder(dir).forEach((f) => needed.add(f));
        includedFiles(dirs).forEach((f) => needed.add(f));
      }
      repoPaths(texts).forEach((f) => needed.add(f));
    }
    for (const [source, marker] of CODE_CORPORA[file] ?? []) {
      const corpus = codeCorpus(source, marker);
      assert.ok(corpus.length > 0, `${source}: \`${marker}\` names no directory of the repository`);
      corpus.forEach((dir) => filesUnder(dir).forEach((f) => needed.add(f)));
    }
    const missing = [...needed].filter((f) => !matches(section.patterns, f) && !matches(ALWAYS, f));
    assert.deepEqual(missing.slice(0, 20), [], `${PATHS_FILE}, [${file}] does not watch ${missing.length} files its jobs build, run or read`);
  }
});

check("every instrument's corpus is read from the source that defines it", () => {
  assert.deepEqual(codeCorpus("compiler/mente-cli/src/fuzz.rs", "const SEED_DIRS").sort(), [
    "benchmark/prelim",
    "benchmark/probe",
    "compiler/mente-cli/tests/debuggers",
    "conformance",
    "examples",
    "perf/kernels",
  ]);
  assert.deepEqual(codeCorpus("compiler/mente-cli/src/mir_run/differential.rs", "fn conformance_programs"), ["conformance"]);
  const agents = codeCorpus("compiler/mente-cli/src/mir_mutants.rs", "fn agent_programs");
  assert.ok(agents.includes("benchmark/tasks") && agents.some((d) => d.startsWith("benchmark/probe/")), agents.join(", "));
});

check("the crates and paths a job uses are read from its scripts", () => {
  const crates = workspace();
  const built = (run) => cratesBuilt([run], crates);
  assert.deepEqual([...built("cargo build --release --locked --bin mn").names], ["mente-cli"]);
  assert.deepEqual([...built("cargo test --release -p mente-interp -- --ignored").names], ["mente-interp"]);
  assert.equal(built("cargo test --locked").names.size, crates.size);
  assert.equal(built("node perf/run.mjs").cargo, false);
  const cli = closure(crates, ["mente-cli"]);
  for (const c of ["mente-syntax", "mente-ast", "mente-sema", "mente-mir", "mente-codegen", "mente-interp", "mente-cli"]) assert.ok(cli.has(`compiler/${c}`), c);
  assert.deepEqual([...closure(crates, ["mente-mir"])], ["compiler/mente-mir"]);
  const named = repoPaths(['node .github/scripts/mutants-floor.mjs count x.json # perf\nmn smt examples "conformance/*/*.mn" > out.txt']);
  assert.ok(named.has(".github/scripts/mutants-floor.mjs") && named.has(".github/scripts/lib/mutants-floor.mjs"), "a script and what it imports");
  assert.ok([...named].some((f) => f.startsWith("examples/")) && [...named].some((f) => /^conformance\/[^/]+\/[^/]+\.mn$/.test(f)));
  assert.ok(![...named].some((f) => f.startsWith("perf/")), "a comment names nothing");
});

check("no script a required job runs can skip its own main", () => {
  const guard = /process\.argv\[1\][^\n]*import\.meta\.url|import\.meta\.url[^\n]*process\.argv\[1\]/;
  const scripts = new Set([".github/scripts/watch.mjs", ".github/scripts/commits.mjs", ".github/scripts/mutants-floor.mjs"]);
  for (const { doc, job } of requiredJobs()) {
    for (const f of repoPaths(scriptTexts(doc, job))) if (/\.(mjs|cjs|js|ts)$/.test(f) && !f.endsWith(".test.mjs")) scripts.add(f);
  }
  for (const f of scripts) assert.doesNotMatch(read(f), guard, `${f} runs main() only when the path it was started by matches its own`);
  for (const cli of [".github/scripts/watch.mjs", ".github/scripts/commits.mjs", ".github/scripts/mutants-floor.mjs", ".github/scripts/test-list.mjs", ".github/scripts/action-pins.mjs", "perf/run.mjs"]) {
    assert.match(read(cli), /^\s*(?!\/\/).*\bmain\(/m, `${cli} does not call main() at its top level`);
  }
  assert.ok(scripts.has("benchmark/check.mjs") && scripts.has("perf/lib.mjs") && scripts.has(".github/scripts/lib/test-list.mjs"), "the scripts of the required jobs are read");
});

check("the watch reads globs as GitHub's paths filters do", () => {
  assert.ok(matches(["compiler/**"], "compiler/mente-cli/src/main.rs"));
  assert.ok(!matches(["compiler/*"], "compiler/mente-cli/src/main.rs"));
  assert.ok(matches(["compiler/*/Cargo.toml"], "compiler/mente-mir/Cargo.toml"));
  assert.ok(!matches(["compiler/**"], "compilers/x.rs"));
  assert.ok(matches(["Cargo.lock"], "Cargo.lock"));
  assert.ok(!matches(["Cargo.lock"], "spikes/x/Cargo.lock"));
  assert.ok(!matches(["perf/kernels/**"], "perf/kernels.md"));
  assert.ok(matches(["a.b"], "a.b") && !matches(["a.b"], "axb"));
  for (const f of [".cargo/config.toml", ".cargo/mutants.toml", "rust-toolchain", "rust-toolchain.toml", PATHS_FILE]) assert.ok(matches(ALWAYS, f), f);
});

check("the path lists' format is enforced", () => {
  const ok = parsePaths("# c\n[a.yml]\nmerge_group: skip\nx/**\n\n[b.yml]\ny # why\n");
  assert.deepEqual(ok.get("a.yml"), { mergeGroup: "skip", patterns: ["x/**"] });
  assert.deepEqual(ok.get("b.yml"), { mergeGroup: "run", patterns: ["y"] });
  for (const bad of ["x/**\n", "[a.yml]\n", "[a.yml]\nx\n[a.yml]\ny\n", "[a.yml]\nmerge_group: run\nx\n", "[a.yml]\nx\nx\n", "[a.yml]\na b\n"]) {
    assert.throws(() => parsePaths(bad), undefined, JSON.stringify(bad));
  }
});

check("the watch runs an instrument exactly when a path it watches changed", () => {
  const section = { mergeGroup: "run", patterns: ["compiler/**", "Cargo.lock"] };
  const heavy = { mergeGroup: "skip", patterns: ["compiler/**"] };
  const pr = { pull_request: { base: { sha: "b" }, head: { sha: "h" } } };
  const group = { merge_group: { base_sha: "b", head_sha: "h" } };
  const diff = (files) => (base, head) => {
    assert.deepEqual([base, head], ["b", "h"]);
    return files;
  };
  assert.equal(decide(section, "pull_request", pr, diff(["compiler/mente-mir/src/lib.rs"])).run, true);
  assert.equal(decide(section, "pull_request", pr, diff(["Cargo.lock", "docs/x.md"])).run, true);
  assert.equal(decide(section, "pull_request", pr, diff(["docs/x.md", "ROADMAP.md"])).run, false);
  assert.equal(decide(section, "pull_request", pr, diff([])).run, false);
  for (const always of [PATHS_FILE, ".github/scripts/lib/watch.mjs", ".cargo/config.toml", "rust-toolchain"]) {
    assert.equal(decide(section, "pull_request", pr, diff([always])).run, true, always);
  }
  assert.match(decide(section, "pull_request", pr, diff(["compiler/::error::x"])).reason, /"compiler\/::error::x"/);
  assert.equal(decide(section, "merge_group", group, diff(["compiler/a.rs"])).run, true);
  assert.equal(decide(section, "merge_group", group, diff(["docs/a.md"])).run, false);
  assert.equal(decide(heavy, "merge_group", group, () => assert.fail("no diff for a heavy instrument")).run, false);
  assert.equal(decide(heavy, "pull_request", pr, diff(["compiler/a.rs"])).run, true);
  assert.equal(decide(section, "push", { before: "0".repeat(40), after: "h" }, () => assert.fail("no diff")).run, true);
  assert.equal(decide(section, "push", { before: "b", after: "h" }, diff(["docs/a.md"])).run, false);
  for (const event of ["schedule", "workflow_dispatch"]) assert.equal(decide(section, event, {}, () => assert.fail("no diff")).run, true);
  assert.throws(() => decide(section, "pull_request", {}, diff([])), /no base and head/);
});

check("the watch CLI decides on a real history, through any path, and fails rather than say nothing", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-watch-"));
  try {
    const sh = (...args) =>
      execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd: dir, encoding: "utf8" }).trim();
    const write = (file, text) => {
      fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
      fs.writeFileSync(path.join(dir, file), text);
    };
    const commit = (files) => {
      for (const [f, t] of Object.entries(files)) write(f, t);
      sh("add", "-A");
      sh("commit", "-q", "-m", "change");
      return sh("rev-parse", "HEAD");
    };
    sh("init", "-q");
    const base = commit({ [PATHS_FILE]: "[x.yml]\nsrc/**\n", "src/a.rs": "a\n", "docs/a.md": "a\n" });
    const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("GITHUB_")));
    const cli = (script, head, { output = true } = {}) => {
      const event = path.join(dir, "event.json");
      fs.writeFileSync(event, JSON.stringify({ pull_request: { base: { sha: base }, head: { sha: head } } }));
      const out = path.join(dir, `out-${Math.random().toString(36).slice(2)}.txt`);
      const r = spawnSync(process.execPath, [script, "x.yml"], {
        cwd: dir,
        encoding: "utf8",
        env: { ...env, GITHUB_EVENT_NAME: "pull_request", GITHUB_EVENT_PATH: event, ...(output ? { GITHUB_OUTPUT: out } : {}) },
      });
      return { status: r.status, output: fs.existsSync(out) ? fs.readFileSync(out, "utf8") : "", stderr: r.stderr };
    };
    const script = path.join(HERE, "watch.mjs");
    const docs = commit({ "docs/a.md": "b\n" });
    assert.deepEqual(cli(script, docs), { status: 0, output: "run=false\n", stderr: "" });
    const src = commit({ "src/a.rs": "b\n" });
    assert.equal(cli(script, src).output, "run=true\n");
    const cargo = commit({ ".cargo/config.toml": "[build]\n" });
    assert.equal(cli(script, cargo).output, "run=true\n");
    const silent = cli(script, docs, { output: false });
    assert.equal(silent.status, 1);
    assert.match(silent.stderr, /GITHUB_OUTPUT/);
    // Started through a junction, as a checkout under another path would be: main still runs.
    const link = path.join(dir, "linked");
    fs.symlinkSync(HERE, link, "junction");
    assert.equal(cli(path.join(link, "watch.mjs"), src).output, "run=true\n");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

// ------------------------------------------------------------------------------ the Commits job

check("the Commits check refuses unfinished subjects, CJK text and a changed base", () => {
  for (const s of ["WIP: parse enums", "wip", "Wip parse", "[WIP] parse", "(wip) x", "Draft: enums", "[Draft] enums", "fixup! Parse", "fixup parse", "Fixup: parse", "squash! x", "squash: x", "[squash] x", "squash", "amend! y"]) {
    assert.ok(badSubject(s), s);
  }
  for (const s of ["Parse enums", "Wipe the cache before a run", "Fix the fixup! handling", "Draft the M3 plan", "Squash two passes into one", "Amend the claims", "Merge pull request #1 from a/b"]) {
    assert.equal(badSubject(s), null, s);
  }
  const cjk = (...codes) => String.fromCodePoint(...codes);
  assert.deepEqual(cjkLines(`English\n${cjk(0x4e2d, 0x6587)}\nmore\n${cjk(0x3002)}`), [2, 4]);
  for (const c of [0x1100, 0x2e80, 0x3105, 0x3131, 0x31a0, 0x3200, 0x33ff, 0xac00, 0xff01, 0x20000, 0x3ffff]) {
    assert.deepEqual(cjkLines(`x ${cjk(c)}`), [1], c.toString(16));
  }
  assert.deepEqual(cjkLines(`caf${cjk(0xe9)}, ${cjk(0x2014)} ${cjk(0x1f916)}`), []);
  const found = problems(
    [
      { sha: "1234567890", subject: "WIP: x", message: "WIP: x\n" },
      { sha: "abcdef0123", subject: "Fine", message: `Fine\n\n${cjk(0xac00)}\n` },
      { sha: "0000000000", subject: "Fine too", message: "Fine too\n" },
    ],
    { title: "A title", body: `Body\n${cjk(0xff01)}` },
  );
  assert.equal(found.length, 3);
  assert.match(found[0], /1234567 "WIP: x".*"WIP"/);
  assert.match(found[1], /abcdef0.*line 3/);
  assert.match(found[2], /body.*line 2/);
  assert.deepEqual(problems([], null), []);
  assert.deepEqual(problems([], null, true), ["base changed: push a commit to re-run the instruments"]);
  // A subject reaches the log quoted, so it cannot open a workflow command of its own.
  assert.match(problems([{ sha: "1234567", subject: "wip\n::error::x", message: "" }], null)[0], /"wip\\n::error::x"/);
});

check("the Commits check fails on every event while a base change is newer than the head's push", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-commits-"));
  // A stand-in for the GitHub API at GITHUB_API_URL, as the runner sets it: the pull request, its
  // issue events and timeline, and its head commits' workflow runs.
  const api = { events: [], timeline: [], runs: new Map(), head: null };
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://localhost");
    let body;
    if (url.pathname === "/repos/o/r/issues/7/events") body = api.events;
    else if (url.pathname === "/repos/o/r/issues/7/timeline") body = api.timeline;
    else if (url.pathname === "/repos/o/r/pulls/7") body = { number: 7, head: { sha: api.head } };
    else if (url.pathname === "/repos/o/r/actions/runs") body = { workflow_runs: api.runs.get(url.searchParams.get("head_sha")) ?? [] };
    res.writeHead(body ? 200 : 404, { "content-type": "application/json" });
    res.end(JSON.stringify(body ?? {}));
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    const sh = (...args) =>
      execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd: dir, encoding: "utf8" }).trim();
    sh("init", "-q");
    fs.writeFileSync(path.join(dir, "a"), "a\n");
    sh("add", "-A");
    sh("commit", "-q", "-m", "Base");
    const base = sh("rev-parse", "HEAD");
    const push = (text) => {
      fs.writeFileSync(path.join(dir, "a"), text);
      sh("commit", "-q", "-am", "Change a");
      return sh("rev-parse", "HEAD");
    };
    const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("GITHUB_")));
    const spawnWith = (eventName, payload) =>
      new Promise((done) => {
        const event = path.join(dir, "event.json");
        fs.writeFileSync(event, JSON.stringify(payload));
        const child = spawn(process.execPath, [path.join(HERE, "commits.mjs")], {
          cwd: dir,
          env: {
            ...env,
            GITHUB_EVENT_NAME: eventName,
            GITHUB_EVENT_PATH: event,
            GITHUB_REPOSITORY: "o/r",
            GITHUB_API_URL: `http://127.0.0.1:${server.address().port}`,
            GITHUB_TOKEN: "t",
            PR_TITLE: "Title",
            PR_BODY: "Body",
          },
        });
        let stdout = "";
        child.stdout.on("data", (d) => (stdout += d));
        child.on("close", (status) => done({ status, stdout }));
      });
    const run = (head, action, changes) =>
      spawnWith("pull_request", { action, changes, pull_request: { number: 7, base: { sha: base }, head: { sha: head } } });
    const change = (at) => ({ event: "base_ref_changed", created_at: at });
    const first = push("b\n");
    api.runs.set(first, [{ created_at: "2026-11-12T10:00:00Z" }, { created_at: "2026-11-12T12:00:00Z" }]);
    assert.equal((await run(first, "synchronize")).status, 0);
    api.events = [change("2026-11-12T09:00:00Z")];
    assert.equal((await run(first, "edited", { body: { from: "Old" } })).status, 0, "a base change before the push is answered");
    // The retarget's own run fails from its event, though the API does not show the change yet.
    const retargeted = await run(first, "edited", { base: { ref: { from: "feature" } } });
    assert.equal(retargeted.status, 1);
    assert.match(retargeted.stdout, /base changed: push a commit to re-run the instruments/);
    // Once the issue events show it, every later event fails too: an edit, a re-run of the push.
    api.events.push(change("2026-11-12T11:00:00Z"));
    assert.equal((await run(first, "edited", { title: { from: "Old" } })).status, 1);
    assert.equal((await run(first, "synchronize")).status, 1);
    // The timeline alone shows it as well.
    api.events = [];
    api.timeline = [change("2026-11-12T11:00:00Z")];
    assert.equal((await run(first, "synchronize")).status, 1);
    // A new commit answers it.
    const second = push("c\n");
    api.runs.set(second, [{ created_at: "2026-11-12T13:00:00Z" }]);
    assert.equal((await run(second, "synchronize")).status, 0);
    // In the merge queue, the pull request's head comes from the API: the group's branch names the
    // base, and the group's commit merges the head into it.
    api.head = second;
    const group = (sha) => ({ merge_group: { base_sha: base, head_sha: second, head_ref: `refs/heads/gh-readonly-queue/main/pr-7-${sha}` } });
    assert.equal((await spawnWith("merge_group", group(base))).status, 0);
    api.head = first;
    assert.equal((await spawnWith("merge_group", group(base))).status, 1, "the queued head's push is older than the base change");
    // Without the head's runs it fails rather than pass unseen.
    api.runs.delete(second);
    assert.equal((await run(second, "synchronize")).status, 1);
  } finally {
    server.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

check("the base-change rule reads every source and the head's first run", () => {
  const at = (h) => ({ created_at: `2026-11-12T${h}:00:00Z` });
  const change = (h) => ({ event: "base_ref_changed", ...at(h) });
  assert.equal(baseChangedSincePush([], [at("10")]), false);
  assert.equal(baseChangedSincePush([change("09"), { event: "labeled", ...at("11") }], [at("10")]), false);
  assert.equal(baseChangedSincePush([change("09"), change("11")], [at("12"), at("10")]), true, "the first run is the push");
  assert.throws(() => baseChangedSincePush([change("09")], []), /push time is unknown/);
  // A merge group's branch names the pull request and the group's base, never the head.
  assert.deepEqual(pullRequestOf("merge_group", { merge_group: { head_ref: `refs/heads/gh-readonly-queue/main/pr-124-${"a".repeat(40)}` } }), { number: 124, head: null });
  assert.deepEqual(pullRequestOf("pull_request", { pull_request: { number: 9, head: { sha: "h" } } }), { number: 9, head: "h" });
  assert.throws(() => pullRequestOf("merge_group", { merge_group: { head_ref: "refs/heads/x" } }), /names no pull request/);
});

check("the Commits check rejects the code points test_repository_text_is_english rejects", () => {
  const main = read("compiler/mente-cli/tests/gates.rs");
  const body = /fn is_cjk\(c: char\) -> bool \{([\s\S]*?)\n\s*\}/.exec(main)?.[1];
  assert.ok(body, "is_cjk not found in compiler/mente-cli/tests/gates.rs");
  const ranges = [...body.matchAll(/0x([0-9A-Fa-f_]+)\.\.=0x([0-9A-Fa-f_]+)/g)].map((m) => [parseInt(m[1].replace(/_/g, ""), 16), parseInt(m[2].replace(/_/g, ""), 16)]);
  assert.deepEqual(CJK_RANGES, ranges);
});

// ------------------------------------------------------------------------------ floors of work

check("the Mutants floor counts what the diff changes, as cargo-mutants marks it", () => {
  const diff = [
    "diff --git a/compiler/a/src/lib.rs b/compiler/a/src/lib.rs",
    "index 1..2 100644",
    "--- a/compiler/a/src/lib.rs",
    "+++ b/compiler/a/src/lib.rs",
    "@@ -10,3 +10,4 @@ fn f() {",
    " context 10",
    "-removed",
    "+added 11",
    "+added 12",
    " context 13",
    "@@ -40,3 +41,2 @@ fn g() {",
    " context 41",
    "-only removed",
    " context 42",
    "diff --git a/gone.rs b/gone.rs",
    "--- a/gone.rs",
    "+++ /dev/null",
    "@@ -1 +0,0 @@",
    "-x",
    "",
  ].join("\n");
  const { added, deletions } = parseDiff(diff);
  assert.deepEqual([...added.keys()], ["compiler/a/src/lib.rs"]);
  assert.deepEqual([...added.get("compiler/a/src/lib.rs")], [11, 12]);
  assert.deepEqual([...deletions.get("compiler/a/src/lib.rs")], [10, 11, 41, 42], "every hunk that deletes a line, the mixed one too");
  const fnf = { function_name: "f", span: { start: { line: 5 }, end: { line: 20 } } };
  const fng = { function_name: "g", span: { start: { line: 38 }, end: { line: 50 } } };
  const mutant = (name, start, end, fn, file = "compiler/a/src/lib.rs") => ({ name, file, function: fn, span: { start: { line: start, column: 1 }, end: { line: end, column: 9 } } });
  const mutants = [
    mutant("f body", 5, 20, fnf),
    mutant("f at 12", 12, 12, fnf),
    mutant("f at 15", 15, 15, fnf),
    mutant("g at 39", 39, 39, fng),
    mutant("g at 48", 48, 48, fng),
    mutant("elsewhere", 1, 99, null, "compiler/b/src/lib.rs"),
  ];
  assert.deepEqual(touched(mutants, added).map((m) => m.name), ["f body", "f at 12"]);
  assert.equal(expected(mutants, diff), 5, "every mutant of f and g, each of which loses a line");
  assert.deepEqual(deletedRegexes(mutants, diff, [mutants[0], mutants[1], mutants[3]]), ["^f at 15$", "^g at 48$"]);
  // Every expected name must be in the run's lists. A pull request that changes f and g and
  // excludes f in its own .cargo/mutants.toml lists none of f's, under the base's settings
  // expected all the same, and fails.
  const named = (...names) => mutants.filter((m) => names.includes(m.name));
  assert.deepEqual(missing(mutants, diff, named("f body", "f at 12", "g at 39"), named("f at 15", "g at 48")), []);
  assert.deepEqual(missing(mutants, diff, named("g at 39"), named("g at 48")), ["f at 12", "f at 15", "f body"]);
  assert.deepEqual(missing(mutants, diff, named("f body", "f at 12", "f at 15", "g at 39"), []), ["g at 48"], "one left out is enough");
  assert.equal(exactRegex("a.rs:1:2: replace <impl X for Y>::f -> bool with true (x|y)"), "^a\\.rs:1:2: replace <impl X for Y>::f \\-> bool with true \\(x\\|y\\)$");
  assert.throws(() => touched([{ file: "x", span: {} }], added), /no file, name and span/);
  // A mutant outside any function, in a file that loses lines, is weighed by its own span: taken
  // when a deleted line touches it, as main.rs's constant `1 << ...` was on this pull request, and
  // left when none does. One in a file the diff leaves alone ("elsewhere") does not matter.
  const loose = [...mutants, mutant("const at 41", 41, 41, null), mutant("const at 30", 30, 30, null)];
  assert.equal(expected(loose, diff), 6, "g's and f's five, and the constant a deleted line touches");
  assert.deepEqual(missing(loose, diff, loose.filter((m) => m.name !== "const at 41"), []), ["const at 41"]);
  assert.deepEqual(readList(""), []);
  assert.throws(() => readList("{}"), /not a JSON array/);
  const spanless = { ...mutant("f at 12", 12, 12, fnf), function: { function_name: "f" } };
  assert.throws(() => readList(JSON.stringify([spanless])), /names a function without its span/);
  assert.equal(readList(JSON.stringify([mutant("const", 3, 3, null)])).length, 1, "no function at all is a mutant outside any");
  // The evidence of each outcome, as cargo-mutants 27 writes it (the shapes of #120's run): a
  // caught mutant's tests ended as nextest ends a failed run, a test failing in its log; an
  // unviable one's build failed with an error of the compiler. A stub that exits at once shows
  // neither.
  const outcome = (name, summary, phases, log) => ({ scenario: { Mutant: { name } }, summary, log_path: `log/${name}.log`, phase_results: phases, log });
  const build = (status) => ({ phase: "Build", duration: 2.8, process_status: status });
  const test = (status) => ({ phase: "Test", duration: 0.15, process_status: status });
  const FAILED = "\x1b[1;31m        FAIL\x1b[0m [   0.004s] mente-syntax tests::x\n";
  const real = [
    outcome("caught", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `*** cargo nextest run\n${FAILED}*** result: Failure(100)\n`),
    outcome("timeout", "CaughtMutant", [build("Success"), test({ Failure: 100 })], "        TIMEOUT [  40.104s] mente-cli::bin/mn x\n"),
    outcome("unviable", "Unviable", [build({ Failure: 101 })], "\x1b[0m\x1b[1m\x1b[38;5;9merror[E0277]\x1b[0m: the trait bound\n"),
    outcome("missed", "MissedMutant", [build("Success"), test("Success")], ""),
    { scenario: "Baseline", summary: "Success", phase_results: [], log_path: "log/baseline.log", log: "" },
  ];
  const logs = (list) => (p) => list.find((o) => o.log_path === p)?.log ?? "";
  assert.deepEqual(evidence({ outcomes: real }, logs(real)), []);
  const stubbed = [
    outcome("caught by a stub", "CaughtMutant", [build("Success"), test({ Failure: 1 })], "*** cargo nextest run\n*** result: Failure(1)\n"),
    outcome("caught, a failing line but not nextest's exit", "CaughtMutant", [build("Success"), test({ Failure: 1 })], FAILED),
    outcome("caught, no test failed", "CaughtMutant", [build("Success"), test({ Failure: 100 })], "error: no tests to run\n"),
    outcome("unviable, built", "Unviable", [build("Success")], "error[E0277]: x\n"),
    outcome("unviable, no error", "Unviable", [build({ Failure: 101 })], "*** result: Failure(101)\n"),
  ];
  assert.deepEqual(evidence({ outcomes: stubbed }, logs(stubbed)).map((p) => p.split(":")[0]), stubbed.map((o) => o.scenario.Mutant.name));
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

/// The base the pull request or merge group is checked against: the event's, or the merge base
/// with origin/main.
function policyBase() {
  // The event the runner describes: the step sets no environment of its own (the Rust guard), so
  // nothing in a pull request can point this elsewhere.
  const name = process.env.GITHUB_EVENT_NAME;
  if (name && process.env.GITHUB_EVENT_PATH) {
    const payload = JSON.parse(fs.readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
    const base = { pull_request: payload.pull_request?.base?.sha, merge_group: payload.merge_group?.base_sha, push: payload.before }[name];
    if (base && !/^0+$/.test(base)) return base;
  }
  try {
    return git(["merge-base", "HEAD", "origin/main"]).trim();
  } catch {
    return "HEAD";
  }
}

check("every milestone done on the base has its tag", () => {
  assert.ok(milestones(read("ROADMAP.md")).size >= 10, "ROADMAP.md lists fewer milestones than stage 1 has");
  const tags = new Set(git(["tag", "-l"]).split(/\r?\n/).filter(Boolean));
  const base = policyBase();
  assert.deepEqual(untagged(git(["show", `${base}:ROADMAP.md`]), tags), [], `milestones done on ${base} have no tag (milestone-tags)`);
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
  ["Mutation testing at two levels", [...[0, 1, 2, 3].map((s) => `Mutants in the diff (shard ${s} of 4)`), "MIR mutation gate with the agent programs", ...RUST]],
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
