// The workflow policy: how the tests run (README.md in this directory). Through the pinned nextest,
// under the shared limits, from a list read as nextest writes it.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { doctestProblems, problems as testListProblems, overrideTests } from "../lib/test-list.mjs";
import { PINNED_JOBS } from "./pins.mjs";
import { filesUnder, read, tomlSubset, workspace } from "./repo.mjs";
import { MUTATION_JOBS, TESTING_JOBS } from "./sandbox.mjs";
import { logicalLines, stepKey, steps, workflows } from "./workflows.mjs";

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
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of test-runs.test.mjs ran: a skip or a filter left the rest out`);
});
/// What .config/nextest.toml may hold, table by table, with the keys each must set: one set, or
/// for the overrides one set for each kind of override. To give nextest another table (a JUnit
/// report for ci, another test group), add it here with its keys in the same pull request, where
/// review sees what the table changes.
const NEXTEST_TABLES = new Map([
  ["profile.default", { array: false, keys: ["leak-timeout", "slow-timeout"] }],
  [
    "profile.default.overrides",
    {
      array: true,
      kinds: new Map([
        ["slow", ["filter", "slow-timeout"]],
        ["all-cores", ["filter", "test-group", "threads-required"]],
      ]),
    },
  ],
  ["profile.ci", { array: false, keys: ["fail-fast"] }],
  ["profile.mutants", { array: false, keys: ["fail-fast"] }],
  // The mutation runs' order: the tests that use every core after the others.
  ["profile.mutants.overrides", { array: true, kinds: new Map([["last", ["filter", "priority"]]]) }],
  ["test-groups", { array: false, keys: ["all-cores"] }],
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
  ["profile.mutants.overrides/priority", "-100"],
  ["profile.default.overrides/test-group", '"all-cores"'],
  ["profile.default.overrides/threads-required", '"num-test-threads"'],
  ["test-groups/all-cores", "{ max-threads = 1 }"],
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
/// The tests that run work on more than one thread at once (compiler/mente-threads),
/// which the override of the group `all-cores` runs alone, each taking every test thread, so that
/// none starves the tests beside it of time; under nextest, mente-threads fails any other test
/// that does. Each by its package and its whole path.
const ALL_CORES_TESTS = [
  ["mente-cli", "mir_mutants::tests::test_the_command_passes_or_fails_on_the_corpus_it_finds"],
  ["mente-cli", "mir_mutants::tests::test_the_corpus_passes_the_gate"],
  ["mente-cli", "mir_run::differential::tests::test_three_paths_agree_on_generated_programs"],
  ["mente-cli", "mir_run::differential::tests::test_three_paths_agree_on_the_conformance_suite"],
  ["mente-cli", "test_mn_test_mutate_runs_the_demo_gate"],
  ["mente-cli", "test_fuzz_shares_runs_among_jobs_and_keeps_to_its_seconds"],
  ["mente-cli", "mir_run::coverage::test_generated_programs_reach_every_construct_of_the_grammar"],
  ["mente-cli", "mir_run::differential::tests::test_generated_programs_of_seeds_0_to_29_are_well_typed_and_replayable"],
  ["mente-cli", "mir_run::differential::tests::test_generated_programs_of_seeds_30_to_59_are_well_typed_and_replayable"],
  ["mente-cli", "mir_run::differential::tests::test_generated_programs_of_seeds_60_to_89_are_well_typed_and_replayable"],
];
/// Where the tests read the group's name: mente-threads fails a test outside the group named
/// there, so a group renamed in .config/nextest.toml and here alone would fail every test of the
/// group.
const CORES_GROUP = ["compiler/mente-threads/src/lib.rs", 'pub const GROUP: &str = "all-cores";'];

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
  const kindOf = new Map();
  for (const t of tables) {
    const allowed = NEXTEST_TABLES.get(t.name);
    const shown = t.array ? `[[${t.name}]]` : `[${t.name}]`;
    assert.ok(allowed && allowed.array === t.array, `.config/nextest.toml has ${shown}: to give nextest another table, add it to NEXTEST_TABLES in .github/scripts/policy/test-runs.test.mjs with the keys it may hold`);
    const sets = allowed.kinds ?? new Map([["", allowed.keys]]);
    const keys = [...t.keys.keys()].sort().join(", ");
    const kind = [...sets].find(([, k]) => [...k].sort().join(", ") === keys);
    assert.ok(kind, `${shown} sets ${keys || "nothing"}, not ${[...sets.values()].map((k) => k.join(", ")).join(" or ")}`);
    kindOf.set(t, kind[0]);
    for (const [key, value] of t.keys) {
      const held = NEXTEST_VALUES.get(`${t.name}/${key}`);
      if (held === undefined) continue;
      assert.equal(value.replace(/\s+/g, ""), held.replace(/\s+/g, ""), `${shown} ${key} = ${value}, where the policy holds ${held}: a limit changes in NEXTEST_VALUES too`);
    }
  }
  for (const name of ["profile.default", "profile.ci", "profile.mutants", "test-groups"]) assert.ok(tables.some((t) => t.name === name), `.config/nextest.toml has no [${name}]`);
  const namedBy = (kind) => tables.filter((t) => t.array && kindOf.get(t) === kind).flatMap((t) => filterTests(t.keys.get("filter")));
  const shown = (list) => list.map(([p, t]) => `${p} ${t}`).sort();
  assert.deepEqual(shown(namedBy("slow")), shown(SLOW_TESTS), "the overrides give their longer limit to other tests than SLOW_TESTS, or one twice: more time for a test is a change to the policy too");
  assert.deepEqual(shown(namedBy("all-cores")), shown(ALL_CORES_TESTS), "the overrides run alone other tests than ALL_CORES_TESTS, or one twice: a test that uses every core is a change to the policy too");
  // The mutation runs take exactly those tests last: a caught mutant stops at its first failing
  // test, and these hold the whole machine (the measure of PR 191's run 44959802577).
  assert.deepEqual(shown(namedBy("last")), shown(ALL_CORES_TESTS), "the mutation runs take other tests last than ALL_CORES_TESTS, or one twice");
  const named = [...namedBy("slow"), ...namedBy("all-cores")];
  const [coresFile, groupLine] = CORES_GROUP;
  const groupName = /"([^"]*)"/.exec(NEXTEST_VALUES.get("profile.default.overrides/test-group"))[1];
  const groupLines = read(coresFile).split(/\r?\n/).filter((l) => l === groupLine);
  assert.equal(groupLines.length, 1, `${coresFile} does not hold ${groupLine} once: the group its tests are held to is the group .config/nextest.toml names`);
  assert.equal(groupLine, `pub const GROUP: &str = "${groupName}";`, "CORES_GROUP names another group than the override's test-group");
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
/// The step of the required Rust jobs that runs the tests: the shared limits first, then nextest's
/// own binary under the ci profile, with nothing after either that could filter, retry or forgive a
/// failure, then the check that the tests ran in the sandbox, and nothing on the step but its name
/// and this script.
const NEXTEST_STEP = ". .github/scripts/limits.sh\ncargo-nextest nextest run --locked --workspace --profile ci\n/usr/bin/bash .github/scripts/test-sandbox.sh ran\n/usr/bin/bash .github/scripts/tools.sh verify\n";

check("the tests run through the pinned nextest under the shared limits, which no setting can lift", () => {
  for (const key of TESTING_JOBS) {
    assert.ok(PINNED_JOBS.has(key), `${key}, a testing job, is not pinned`);
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
    const record = all.findIndex((s) => String(s.run ?? "").trim() === "/usr/bin/bash .github/scripts/tools.sh record");
    assert.ok(install >= 0 && record === install + 1, `${key}: the tools are not recorded right after nextest is installed`);
    for (const s of all.slice(record + 1).filter((x) => typeof x.run === "string")) {
      const lines = s.run.trimEnd().split("\n");
      const last = lines.at(-1) === 'exit "$failed"' ? lines.at(-2) : lines.at(-1);
      assert.equal(last, "/usr/bin/bash .github/scripts/tools.sh verify", `${stepKey(file, id, s)} runs code of the change and does not check the tools at its end`);
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
  assert.equal(tests[0].run, NEXTEST_STEP, "the Rust jobs' tests do not run as NEXTEST_STEP in .github/scripts/policy/test-runs.test.mjs reads them: the limits, then nextest under the ci profile; a change to the step changes NEXTEST_STEP too");
  assert.deepEqual(Object.keys(tests[0]).sort(), ["name", "run"], "the Rust jobs' step of the tests carries more than its name and script");
  // The mutation runs and their plan: their tests under the mutants profile, inside the same
  // limits.
  for (const key of TESTING_JOBS.filter((k) => !k.startsWith("ci.yml#"))) {
    const w = workflows().find((x) => x.file === key.split("#")[0]);
    assert.equal(w.doc.env?.NEXTEST_PROFILE, "mutants", `${key} runs its tests under another profile`);
  }
  for (const key of MUTATION_JOBS) {
    const [file, id] = key.split("#");
    const w = workflows().find((x) => x.file === file);
    const mutate = steps(w.doc.jobs[id]).filter((s) => /^cargo mutants (?!.*--list)/m.test(String(s.run ?? "")));
    assert.ok(mutate.length > 0, `${file}: no step runs cargo mutants`);
    for (const s of mutate) {
      const body = String(s.run).split("\n");
      const source = body.indexOf(". .github/scripts/limits.sh");
      const first = body.findIndex((l) => /^cargo mutants (?!.*--list)/.test(l));
      assert.ok(source >= 0 && source < first, `${stepKey(file, id, s)}: cargo mutants runs outside the shared limits`);
    }
  }
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
  // Every override's tests, the mutation runs' order naming the all-cores group's again.
  assert.deepEqual(overrideTests(read(".config/nextest.toml")).sort(), [...SLOW_TESTS, ...ALL_CORES_TESTS, ...ALL_CORES_TESTS].map(([p, t]) => `${p} ${t}`).sort(), "lib/test-list.mjs reads the overrides' tests as the policy does");
  // The doctests' list, as `cargo test --doc -- --list` prints it.
  const none = "   Doc-tests mente_ast\n0 tests, 0 benchmarks\n   Doc-tests mente_mir\n0 tests, 0 benchmarks\n";
  assert.deepEqual(doctestProblems(none), []);
  // As the jobs print it, under CARGO_TERM_COLOR=always: the header coloured.
  assert.deepEqual(doctestProblems("\x1b[1m\x1b[32m   Doc-tests\x1b[0m mente_ast\n0 tests, 0 benchmarks\n"), []);
  assert.equal(doctestProblems(`${none}   Doc-tests mente_sema\ncompiler/mente-sema/src/lib.rs - f (line 3): test\n1 test, 0 benchmarks\n`).length, 1, "a library with a doctest");
  assert.equal(doctestProblems("   Doc-tests mente_ast\n   Doc-tests mente_mir\n0 tests, 0 benchmarks\n").length, 1, "a library without its count");
  assert.equal(doctestProblems("   Compiling mente-ast\n").length, 1, "no library at all");
});
