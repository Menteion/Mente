// The workflow policy: the instruments and their path lists (README.md in this directory). Each
// instrument decides at its first step, and its list names everything its jobs read.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { ALWAYS, PATHS_FILE, decide, globToRegExp, matches, parsePaths } from "../lib/watch.mjs";
import { SANDBOX_PROGRAM, deletedRegexes, evidence, exactRegex, expected, missing, parseDiff, readList, touched } from "../lib/mutants-floor.mjs";
import { ROOT, filesUnder, read, tracked, workspace } from "./repo.mjs";
import { REPORT_FORMS, WATCH_FORMS, WORKFLOWS, allJobs, hasWatch, isReport, needsOf, normalize, repoPaths, requiredChecks, requiredJobs, scriptTexts, steps } from "./workflows.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 9;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of instruments.test.mjs ran: a skip or a filter left the rest out`);
});
const isDir = (dir) => [...tracked()].some((f) => f.startsWith(`${dir}/`));

// ------------------------------------------------------------------ the watch and its path lists

/// Why a job of a workflow decides otherwise than an instrument must: a job with a watch step
/// checks out first with the whole history, then runs the watch over its workflow's list, alone,
/// and every later step obeys the decision; a required job of an instrument workflow with no watch
/// step is an aggregate, which decides through a job it needs that holds the watch, its plan
/// (`sections`, the path lists; `required`, the required checks' names; `names`, the job's).
export function watchProblems(file, doc, id, job, names, sections, required) {
  const found = [];
  const all = steps(job);
  const where = `${file}, job ${id}`;
  if (!hasWatch(job)) {
    // It reads the plan's decision, which the plan gives as its watch step's own output.
    const reads = new Set(steps(job).flatMap((s) => Object.values(s.env ?? {}).map(String)));
    const planned = needsOf(job).some((need) => hasWatch(doc.jobs?.[need] ?? {}) && doc.jobs[need].outputs?.run === "${{ steps.watch.outputs.run }}" && reads.has(`\${{ needs.${need}.outputs.run }}`));
    if (sections.has(file) && names.some((n) => required.has(n)) && !planned) found.push(`${where}: a required job of an instrument workflow with no watch step, and no job it needs holds one`);
    return found;
  }
  if (!sections.has(file)) found.push(`${where}: a watch step, but ${PATHS_FILE} has no [${file}]`);
  if (!/^actions\/checkout@/.test(String(all[0]?.uses ?? ""))) found.push(`${where}: the first step is not the checkout`);
  if (all[0]?.with?.["fetch-depth"] !== 0) found.push(`${where}: the checkout needs fetch-depth: 0 for the diff`);
  const watch = all[1];
  if (watch?.id !== "watch") found.push(`${where}: the step after the checkout is not the watch`);
  else {
    if (String(watch.run).trim() !== `node .github/scripts/watch.mjs ${file}`) found.push(`${where}: the watch step reads another list`);
    if (Object.keys(watch).sort().join(",") !== "id,name,run") found.push(`${where}: the watch step carries more than its name, id and command`);
  }
  for (const s of all.slice(2)) {
    const condition = normalize(s.if ?? "");
    if (!(WATCH_FORMS.includes(condition) || (REPORT_FORMS.includes(condition) && isReport(file, id, s)))) found.push(`${where}, step "${s.name ?? s.uses ?? s.run}": its if: is not one of the watch's forms`);
  }
  return found;
}

check("every instrument workflow decides at its first step, and every later step obeys", () => {
  const sections = parsePaths(read(PATHS_FILE));
  const required = new Set(requiredChecks());
  for (const { file, doc, id, job, names } of allJobs()) assert.deepEqual(watchProblems(file, doc, id, job, names, sections, required), []);
  // Planted, around the aggregate's exception: "Mutants in the diff" decides through its plan, and
  // fails once no job it needs holds the watch, once it needs nothing, or once what it needs is no
  // job; a shard, no required check, needs no watch of its own; and a watched job keeps its rules.
  const mutants = allJobs().filter((j) => j.file === "mutants.yml");
  const doc = mutants[0].doc;
  const job = (id) => mutants.find((j) => j.id === id);
  const aggregate = job("aggregate");
  const verdict = (d, id, j, names) => watchProblems("mutants.yml", d, id, j, names, sections, required);
  assert.deepEqual(verdict(doc, "aggregate", aggregate.job, aggregate.names), []);
  assert.deepEqual(verdict(doc, "shard", job("shard").job, job("shard").names), []);
  const unwatched = { ...doc, jobs: { ...doc.jobs, plan: { ...doc.jobs.plan, steps: steps(doc.jobs.plan).filter((s) => s.id !== "watch") } } };
  for (const [what, d, j] of [
    ["a plan that holds no watch", unwatched, aggregate.job],
    ["no needs", doc, Object.fromEntries(Object.entries(aggregate.job).filter(([k]) => k !== "needs"))],
    ["needs that name only the shards", doc, { ...aggregate.job, needs: ["shard"] }],
    ["a need that is no job", doc, { ...aggregate.job, needs: ["gone"] }],
    // The review of 560ccd9: the plan's decision read through an output bound elsewhere, or not
    // read at all.
    ["a plan whose run output is no watch's", { ...doc, jobs: { ...doc.jobs, plan: { ...doc.jobs.plan, outputs: { ...doc.jobs.plan.outputs, run: "true" } } } }, aggregate.job],
    ["the plan's decision unread", doc, { ...aggregate.job, steps: steps(aggregate.job).map((s) => ({ ...s, env: Object.fromEntries(Object.entries(s.env ?? {}).filter(([k]) => k !== "RUN")) })) }],
  ]) {
    assert.ok(verdict(d, "aggregate", j, aggregate.names).length > 0, `a required job with ${what} passes without a watch`);
  }
  const plan = job("plan");
  for (const [what, j] of [
    ["the watch after another step", { ...plan.job, steps: [steps(plan.job)[0], steps(plan.job)[2], steps(plan.job)[1], ...steps(plan.job).slice(3)] }],
    ["a step under another condition", { ...plan.job, steps: [...steps(plan.job).slice(0, 2), { ...steps(plan.job)[2], if: "true" }, ...steps(plan.job).slice(3)] }],
  ]) {
    assert.ok(verdict(doc, "plan", j, plan.names).length > 0, `a watched job with ${what} passes`);
  }
  for (const file of sections.keys()) {
    assert.ok(allJobs().some((j) => j.file === file && steps(j.job)[1]?.id === "watch"), `${PATHS_FILE}: no job of ${file} reads [${file}]`);
  }
});

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
  for (const cli of [".github/scripts/watch.mjs", ".github/scripts/commits.mjs", ".github/scripts/mutants-floor.mjs", ".github/scripts/test-list.mjs", ".github/scripts/action-pins.mjs", ".github/scripts/build-time-code.mjs", "perf/run.mjs"]) {
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
