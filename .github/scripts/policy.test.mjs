// The workflow policy (docs/design-review.md, guard-per-problem): what makes the instruments
// merge gates of `main`, and what keeps the workflows safe, checked over .github/workflows. It runs
// in the Documentation job, a required check: `npm run test:ci`.
//
// - Every check .github/required-checks.txt names is produced by exactly one job, whose workflow
//   runs on every pull request and in the merge queue with no `paths:` filter, and which nothing
//   can skip at the job level.
// - Every instrument workflow decides at its first step after the checkout whether it has anything
//   to check (.github/scripts/watch.mjs), every later step obeys that decision, and its path list
//   in .github/instrument-paths.txt names Cargo.toml, Cargo.lock, rust-toolchain.toml, its own
//   workflow, and every crate its jobs build.
// - Every action is pinned to a full commit SHA, every workflow sets its permissions, every
//   checkout drops its credentials, no workflow runs on `pull_request_target` or `workflow_run`,
//   and no `run:` script interpolates text an event or a dispatch supplies.
// - A measured benchmark run proves its sandbox with sandbox-check.ts before any agent code runs.
// - Every milestone ROADMAP.md marks done has its tag, and every instrument docs/design-brief.md
//   section 7.7 schedules for a finished milestone has its checks required.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { parseYaml } from "./yaml.mjs";
import { ALWAYS, PATHS_FILE, ROOT, decide, globToRegExp, matches, parsePaths } from "./watch.mjs";
import { CJK_RANGES, badSubject, cjkLines, problems } from "./commits.mjs";
import { addedLines, touched } from "./mutants-floor.mjs";

const read = (file) => fs.readFileSync(path.join(ROOT, file), "utf8");
const WORKFLOWS = ".github/workflows";
const REQUIRED_FILE = ".github/required-checks.txt";
const WATCH_IF = "steps.watch.outputs.run == 'true'";
/// The one job-level condition a required job may carry: it runs on both events a required check
/// reports on, and so is never skipped there.
const REQUIRED_JOB_IF = "github.event_name == 'pull_request' || github.event_name == 'merge_group'";

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

/// The names GitHub gives a job's checks: its `name:`, or its id, once for each combination of
/// its matrix, with `${{ matrix.<key> }}` filled in.
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
  return combos.map((c) =>
    template.replace(/\$\{\{\s*matrix\.([\w-]+)\s*\}\}/g, (all, key) => (key in c ? String(c[key]) : all)),
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

function steps(job) {
  return Array.isArray(job.steps) ? job.steps : [];
}

// ------------------------------------------------------------------- safety of every workflow

test("no workflow runs on pull_request_target or workflow_run", () => {
  for (const { file, doc } of workflows()) {
    for (const event of ["pull_request_target", "workflow_run"]) {
      assert.ok(!(event in triggers(doc)), `${file} runs on ${event}, which runs with the base's secrets and a write token`);
    }
  }
});

test("every workflow sets its permissions at the top", () => {
  for (const { file, doc } of workflows()) {
    assert.ok(doc.permissions !== undefined && doc.permissions !== null, `${file} has no top-level permissions:`);
    assert.notEqual(doc.permissions, "write-all", `${file} gives every permission`);
  }
});

test("every action is pinned to a full commit SHA", () => {
  for (const { file, id, job } of allJobs()) {
    const uses = [job.uses, ...steps(job).map((s) => s.uses)].filter((u) => u !== undefined);
    for (const u of uses) {
      assert.match(
        String(u),
        /^(\.\/.+|[\w.-]+\/[\w.-]+(\/[\w./-]+)?@[0-9a-f]{40})$/,
        `${file}, job ${id}: ${u} is not pinned to a full commit SHA`,
      );
    }
  }
});

test("every checkout drops its credentials", () => {
  for (const { file, id, job } of allJobs()) {
    for (const s of steps(job)) {
      if (String(s.uses ?? "").startsWith("actions/checkout@")) {
        assert.equal(s.with?.["persist-credentials"], false, `${file}, job ${id}: a checkout without persist-credentials: false`);
      }
    }
  }
});

test("no run script interpolates text an event, a dispatch or a secret supplies", () => {
  for (const { file, id, job } of allJobs()) {
    for (const s of steps(job)) {
      if (typeof s.run !== "string") continue;
      for (const [expression] of s.run.matchAll(/\$\{\{[\s\S]*?\}\}/g)) {
        assert.doesNotMatch(
          expression,
          /\bgithub\.event\b|\bgithub\.head_ref\b|\binputs\.|\bsecrets\./,
          `${file}, job ${id}, step "${s.name ?? s.run.split("\n")[0]}": ${expression} is written into the script; pass it through env:`,
        );
      }
    }
  }
});

test("every measured benchmark run proves its sandbox first", () => {
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

// ------------------------------------------------------------------------- the required checks

test("the required checks are listed once each", () => {
  const names = requiredChecks();
  assert.ok(names.length > 0);
  const twice = names.filter((n, k) => names.indexOf(n) !== k);
  assert.deepEqual(twice, [], `${REQUIRED_FILE} lists these twice`);
});

test("no two jobs that run on pull requests report a check of the same name", () => {
  const seen = new Map();
  for (const { file, doc, id, names } of allJobs()) {
    const on = triggers(doc);
    if (!("pull_request" in on) && !("merge_group" in on)) continue;
    for (const name of names) {
      assert.ok(!seen.has(name),
        `the check "${name}" is reported by ${seen.get(name)} and by ${file}#${id}: a required check of that name would be ambiguous`);
      seen.set(name, `${file}#${id}`);
    }
  }
});

test("every required check is one job that runs on every pull request and in the merge queue", () => {
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
      for (const key of ["paths", "paths-ignore", "branches-ignore"]) {
        assert.ok(!(key in filter), `${where}: its ${event} trigger has ${key}:, so a run it skips never reports`);
      }
    }
    const types = on.pull_request?.types;
    if (types) {
      for (const t of ["opened", "synchronize", "reopened"]) {
        assert.ok(types.includes(t), `${where}: its pull_request trigger leaves out ${t}`);
      }
    }
    assert.ok(!("needs" in job), `${where}: a required job with needs: is skipped, and so passes, when a job it needs fails`);
    if ("if" in job) {
      assert.equal(String(job.if).replace(/^\$\{\{\s*|\s*\}\}$/g, ""), REQUIRED_JOB_IF, `${where}: a job-level if: could skip it`);
    }
  }
});

// ------------------------------------------------------------------ the watch and its path lists

/// The workspace's members, each with the members it depends on by path.
function workspace() {
  const root = read("Cargo.toml");
  const members = [...(/members\s*=\s*\[([\s\S]*?)\]/.exec(root)?.[1] ?? "").matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  assert.ok(members.length > 0, "Cargo.toml lists no workspace members");
  const crates = new Map();
  for (const dir of members) {
    const manifest = read(`${dir}/Cargo.toml`);
    const name = /^\s*name\s*=\s*"([^"]+)"/m.exec(manifest)?.[1];
    const deps = [...manifest.matchAll(/^\s*([\w-]+)\s*=\s*\{[^}]*\bpath\s*=\s*"([^"]+)"/gm)].map((m) =>
      path.posix.normalize(path.posix.join(dir, m[2])),
    );
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

/// The crates a job's scripts build: those `-p` names, `mente-cli` for `--bin mn`, and every
/// member for a cargo command that names neither.
function cratesBuilt(job, crates) {
  const names = new Set();
  let cargo = false;
  for (const s of steps(job)) {
    if (typeof s.run !== "string") continue;
    const commands = s.run.replace(/\\\n/g, " ").split(/\n|&&|\|\||;|\|/);
    for (const command of commands) {
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

test("the path lists read as the watch reads them", () => {
  const sections = parsePaths(read(PATHS_FILE));
  assert.ok(sections.size > 0);
  const tracked = execFileSync("git", ["ls-files", "-z"], { cwd: ROOT, encoding: "utf8" }).split("\0").filter(Boolean);
  for (const [file, section] of sections) {
    assert.ok(fs.existsSync(path.join(ROOT, WORKFLOWS, file)), `${PATHS_FILE}: [${file}] is not a workflow`);
    for (const p of section.patterns) {
      assert.ok(tracked.some((f) => globToRegExp(p).test(f)), `${PATHS_FILE}, [${file}]: ${p} matches no file of the repository`);
    }
  }
});

test("every instrument workflow decides at its first step, and every later step obeys", () => {
  const sections = parsePaths(read(PATHS_FILE));
  const required = new Set(requiredChecks());
  for (const { file, id, job, names } of allJobs()) {
    const all = steps(job);
    const watchAt = all.findIndex((s) => s.id === "watch" || /watch\.mjs/.test(String(s.run ?? "")));
    if (watchAt < 0) {
      assert.ok(!sections.has(file) || !names.some((n) => required.has(n)),
        `${file}, job ${id}: a required job of an instrument workflow with no watch step`);
      continue;
    }
    const where = `${file}, job ${id}`;
    assert.ok(sections.has(file), `${where}: a watch step, but ${PATHS_FILE} has no [${file}]`);
    assert.equal(watchAt, 1, `${where}: the watch step is not the first after the checkout`);
    assert.match(String(all[0].uses ?? ""), /^actions\/checkout@/, `${where}: the first step is not the checkout`);
    assert.equal(all[0].with?.["fetch-depth"], 0, `${where}: the checkout needs fetch-depth: 0 for the diff`);
    assert.equal(all[1].id, "watch", `${where}: the watch step's id is not watch`);
    assert.equal(String(all[1].run).trim(), `node .github/scripts/watch.mjs ${file}`, `${where}: the watch step reads another list`);
    assert.ok(!("if" in all[1]), `${where}: the watch step is conditional`);
    for (const s of all.slice(2)) {
      assert.ok(String(s.if ?? "").includes(WATCH_IF), `${where}, step "${s.name ?? s.uses ?? s.run}": no if: on ${WATCH_IF}`);
    }
  }
  for (const file of sections.keys()) {
    assert.ok(
      allJobs().some((j) => j.file === file && steps(j.job)[1]?.id === "watch"),
      `${PATHS_FILE}: no job of ${file} reads [${file}]`,
    );
  }
});

test("every path list names the crates, the lock, the toolchain and the workflow its jobs use", () => {
  const sections = parsePaths(read(PATHS_FILE));
  const crates = workspace();
  for (const [file, section] of sections) {
    const needed = new Set([`${WORKFLOWS}/${file}`]);
    for (const { job } of allJobs().filter((j) => j.file === file)) {
      const { cargo, names } = cratesBuilt(job, crates);
      if (!cargo) continue;
      for (const f of ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"]) needed.add(f);
      for (const dir of closure(crates, names)) {
        needed.add(`${dir}/Cargo.toml`);
        needed.add(`${dir}/src/lib.rs`);
      }
    }
    const missing = [...needed].filter((f) => !matches(section.patterns, f));
    assert.deepEqual(missing, [], `${PATHS_FILE}, [${file}] does not watch these, which its jobs build or run`);
  }
});

test("the crates a job builds are read from its cargo commands", () => {
  const crates = workspace();
  const job = (run) => ({ steps: [{ run }] });
  assert.deepEqual([...cratesBuilt(job("cargo build --release --locked --bin mn"), crates).names], ["mente-cli"]);
  assert.deepEqual([...cratesBuilt(job("cargo test --release -p mente-interp -- --ignored"), crates).names], ["mente-interp"]);
  assert.equal(cratesBuilt(job("cargo test --locked"), crates).names.size, crates.size);
  assert.equal(cratesBuilt(job("node perf/run.mjs"), crates).cargo, false);
  const cli = closure(crates, ["mente-cli"]);
  for (const c of ["mente-syntax", "mente-ast", "mente-sema", "mente-mir", "mente-codegen", "mente-interp", "mente-cli"]) {
    assert.ok(cli.has(`compiler/${c}`), `mente-cli builds ${c}`);
  }
  assert.deepEqual([...closure(crates, ["mente-mir"])], ["compiler/mente-mir"]);
});

test("the watch reads globs as GitHub's paths filters do", () => {
  assert.ok(matches(["compiler/**"], "compiler/mente-cli/src/main.rs"));
  assert.ok(!matches(["compiler/*"], "compiler/mente-cli/src/main.rs"));
  assert.ok(matches(["compiler/*/Cargo.toml"], "compiler/mente-mir/Cargo.toml"));
  assert.ok(!matches(["compiler/**"], "compilers/x.rs"));
  assert.ok(matches(["Cargo.lock"], "Cargo.lock"));
  assert.ok(!matches(["Cargo.lock"], "spikes/x/Cargo.lock"));
  assert.ok(!matches(["perf/kernels/**"], "perf/kernels.md"));
  assert.ok(matches(["a.b"], "a.b") && !matches(["a.b"], "axb"));
});

test("the path lists' format is enforced", () => {
  const ok = parsePaths("# c\n[a.yml]\nmerge_group: skip\nx/**\n\n[b.yml]\ny # why\n");
  assert.deepEqual(ok.get("a.yml"), { mergeGroup: "skip", patterns: ["x/**"] });
  assert.deepEqual(ok.get("b.yml"), { mergeGroup: "run", patterns: ["y"] });
  for (const bad of ["x/**\n", "[a.yml]\n", "[a.yml]\nx\n[a.yml]\ny\n", "[a.yml]\nmerge_group: run\nx\n", "[a.yml]\nx\nx\n", "[a.yml]\na b\n"]) {
    assert.throws(() => parsePaths(bad), undefined, JSON.stringify(bad));
  }
});

test("the watch runs an instrument exactly when a path it watches changed", () => {
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
  for (const always of ALWAYS) assert.equal(decide(section, "pull_request", pr, diff([always])).run, true);
  assert.equal(decide(section, "merge_group", group, diff(["compiler/a.rs"])).run, true);
  assert.equal(decide(section, "merge_group", group, diff(["docs/a.md"])).run, false);
  assert.equal(decide(heavy, "merge_group", group, () => assert.fail("no diff for a heavy instrument")).run, false);
  assert.equal(decide(heavy, "pull_request", pr, diff(["compiler/a.rs"])).run, true);
  assert.equal(decide(section, "push", { before: "0".repeat(40), after: "h" }, () => assert.fail("no diff")).run, true);
  assert.equal(decide(section, "push", { before: "b", after: "h" }, diff(["docs/a.md"])).run, false);
  for (const event of ["schedule", "workflow_dispatch"]) {
    assert.equal(decide(section, event, {}, () => assert.fail("no diff")).run, true);
  }
  assert.throws(() => decide(section, "pull_request", {}, diff([])), /no base and head/);
});

// ------------------------------------------------------------------------------ the Commits job

test("the Commits check refuses unfinished subjects and CJK text", () => {
  for (const s of ["WIP: parse enums", "wip", "Wip parse", "fixup! Parse enums", "squash! x", "amend! y"]) {
    assert.ok(badSubject(s), s);
  }
  for (const s of ["Parse enums", "Wipe the cache before a run", "Fix the fixup! handling", "Merge pull request #1 from a/b"]) {
    assert.equal(badSubject(s), null, s);
  }
  const cjk = (...codes) => String.fromCodePoint(...codes);
  assert.deepEqual(cjkLines(`English\n${cjk(0x4e2d, 0x6587)}\nmore\n${cjk(0x3002)}`), [2, 4]);
  assert.deepEqual(cjkLines("café, naïve, — \u{1F916}"), []);
  const found = problems(
    [
      { sha: "1234567890", subject: "WIP: x", message: "WIP: x\n" },
      { sha: "abcdef0123", subject: "Fine", message: `Fine\n\n${cjk(0xac00)}\n` },
      { sha: "0000000000", subject: "Fine too", message: "Fine too\n" },
    ],
    { title: "A title", body: `Body\n${cjk(0xff01)}` },
  );
  assert.equal(found.length, 3);
  assert.match(found[0], /1234567.*WIP/);
  assert.match(found[1], /abcdef0.*line 3/);
  assert.match(found[2], /body.*line 2/);
  assert.deepEqual(problems([], null), []);
});

test("the Commits check rejects the code points test_repository_text_is_english rejects", () => {
  const main = read("compiler/mente-cli/src/main.rs");
  const body = /fn is_cjk\(c: char\) -> bool \{([\s\S]*?)\n\s*\}/.exec(main)?.[1];
  assert.ok(body, "is_cjk not found in compiler/mente-cli/src/main.rs");
  const ranges = [...body.matchAll(/0x([0-9A-Fa-f]+)\.\.=0x([0-9A-Fa-f]+)/g)].map((m) => [parseInt(m[1], 16), parseInt(m[2], 16)]);
  assert.deepEqual(CJK_RANGES, ranges);
});

// --------------------------------------------------------------------- milestones and their gates

/// The milestones of ROADMAP.md: `### M<id>: ...` sections, each done when it has items and every
/// one is checked.
function milestones() {
  const out = new Map();
  let current = null;
  for (const line of read("ROADMAP.md").split(/\r?\n/)) {
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

test("every milestone the roadmap marks done has its tag", () => {
  const all = milestones();
  assert.ok(all.size >= 10, "ROADMAP.md lists fewer milestones than stage 1 has");
  const tags = new Set(execFileSync("git", ["tag", "-l"], { cwd: ROOT, encoding: "utf8" }).split(/\r?\n/).filter(Boolean));
  const done = [...all.values()].filter(finished);
  assert.ok(done.length > 0, "no milestone is done");
  for (const m of done) {
    assert.ok(tags.has(m.id.toLowerCase()), `${m.id} is done in ROADMAP.md but has no tag ${m.id.toLowerCase()} (milestone-tags)`);
  }
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

test("every instrument scheduled for a finished milestone is a required check", () => {
  const brief = read("docs/design-brief.md");
  const start = brief.indexOf("| Instrument | What it catches | Active from |");
  assert.ok(start >= 0, "the quality table of docs/design-brief.md section 7.7 is gone");
  const rows = brief
    .slice(start)
    .split(/\r?\n/)
    .slice(2)
    .filter((_, k, all) => all.slice(0, k + 1).every((l) => l.startsWith("|")))
    .map((l) => l.split(" | ").map((c) => c.replace(/^\|\s*|\s*\|$/g, "")));
  assert.ok(rows.length >= 8, "the quality table has fewer rows than it did");
  const all = milestones();
  const isDone = (id) => {
    const parts = [...all.values()].filter((m) => m.id === id || (m.id.startsWith(id) && /^[a-z]$/.test(m.id.slice(id.length))));
    return parts.length > 0 && parts.every(finished);
  };
  const required = new Set(requiredChecks());
  let due = 0;
  for (const row of rows) {
    const instrument = row[0];
    const activeFrom = row[row.length - 1];
    const named = [...activeFrom.matchAll(/\bM\d+[a-z]?\b/g)].map((m) => m[0]);
    assert.ok(named.length > 0, `the row "${instrument.slice(0, 40)}" names no milestone`);
    if (!named.some(isDone)) continue;
    due++;
    const entry = INSTRUMENT_CHECKS.find(([prefix]) => instrument.startsWith(prefix));
    assert.ok(entry, `"${instrument.slice(0, 60)}" is active from a finished milestone, and no required check is named for it here`);
    for (const check of entry[1]) {
      assert.ok(required.has(check), `"${check}", which runs "${entry[0]}", is not in ${REQUIRED_FILE}`);
    }
  }
  assert.ok(due > 0, "no instrument is due, though milestones are done");
});

// ------------------------------------------------------------------------------ floors of work

test("the Mutants floor counts the mutants whose span holds an added line", () => {
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
    "@@ -40 +41,0 @@",
    "-only removed",
    "diff --git a/gone.rs b/gone.rs",
    "--- a/gone.rs",
    "+++ /dev/null",
    "@@ -1 +0,0 @@",
    "-x",
    "",
  ].join("\n");
  const added = addedLines(diff);
  assert.deepEqual([...added.keys()], ["compiler/a/src/lib.rs"]);
  assert.deepEqual([...added.get("compiler/a/src/lib.rs")], [11, 12]);
  const span = (file, start, end) => ({ file, span: { start: { line: start, column: 1 }, end: { line: end, column: 9 } } });
  const mutants = [span("compiler/a/src/lib.rs", 5, 20), span("compiler/a/src/lib.rs", 12, 12), span("compiler/a/src/lib.rs", 13, 30), span("compiler/b/src/lib.rs", 1, 99)];
  assert.equal(touched(mutants, added).length, 2);
  assert.equal(touched(mutants, new Map()).length, 0);
  assert.throws(() => touched([{ file: "x", span: {} }], added), /no file and span/);
  assert.throws(() => touched({}, added), /not a JSON array/);
});
