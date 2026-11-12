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

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { parseYaml } from "./lib/yaml.mjs";
import { ALWAYS, PATHS_FILE, decide, globToRegExp, matches, parsePaths } from "./lib/watch.mjs";
import { CJK_RANGES, badSubject, cjkLines, problems } from "./lib/commits.mjs";
import { deletedRegexes, exactRegex, expected, parseDiff, readList, touched } from "./lib/mutants-floor.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "../..");
const read = (file) => fs.readFileSync(path.join(ROOT, file), "utf8");
const git = (args) => execFileSync("git", args, { cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
const WORKFLOWS = ".github/workflows";
const REQUIRED_FILE = ".github/required-checks.txt";
const WATCH = "steps.watch.outputs.run != 'false'";
/// The conditions a step after the watch may carry: the watch's decision, alone or after one of the
/// status functions that still run it when an earlier step failed.
const WATCH_FORMS = [WATCH, `always() && ${WATCH}`, `!cancelled() && ${WATCH}`, `failure() && ${WATCH}`];
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

test("no workflow runs on pull_request_target or workflow_run", () => {
  for (const { file, doc } of workflows()) {
    for (const event of ["pull_request_target", "workflow_run"]) {
      assert.ok(!(event in triggers(doc)), `${file} runs on ${event}, which runs with the base's secrets and a write token`);
    }
  }
});

const readOnly = (permissions) =>
  permissions === "read-all" ||
  (permissions !== null && typeof permissions === "object" && Object.values(permissions).every((v) => v === "read" || v === "none"));

test("every workflow sets its permissions at the top, read-only wherever a required job runs", () => {
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

test("every action is pinned to a full commit SHA", () => {
  for (const { file, id, job } of allJobs()) {
    const uses = [job.uses, ...steps(job).map((s) => s.uses)].filter((u) => u !== undefined);
    for (const u of uses) {
      assert.match(String(u), /^(\.\/.+|[\w.-]+\/[\w.-]+(\/[\w./-]+)?@[0-9a-f]{40})$/, `${file}, job ${id}: ${u} is not pinned to a full commit SHA`);
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

test("no script holds an expression: values reach scripts through env", () => {
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

test("overflow checks stay on in every build", () => {
  for (const { file } of workflows()) {
    assert.doesNotMatch(
      read(`${WORKFLOWS}/${file}`),
      /CARGO_PROFILE_\w+_OVERFLOW_CHECKS|overflow-checks/,
      `${file} sets overflow checks itself: [profile.release] in Cargo.toml is their one source`,
    );
  }
  const manifests = new Map([...tracked()].filter((f) => f === "Cargo.toml" || f.endsWith("/Cargo.toml")).map((f) => [f, read(f)]));
  assert.deepEqual(overflowProblems(manifests, [...tracked()]), []);
  // What the check stops: a package's own profile, a release profile without the checks, a
  // .cargo/config anywhere.
  const root = "[profile.release]\noverflow-checks = true\n";
  assert.deepEqual(overflowProblems(new Map([["Cargo.toml", root]]), []), []);
  assert.equal(overflowProblems(new Map([["Cargo.toml", `${root}\n[profile.release.package.mente-mir]\noverflow-checks = false\n`]]), []).length, 1);
  assert.equal(overflowProblems(new Map([["Cargo.toml", "[profile.release]\nopt-level = 3\n"]]), []).length, 1);
  assert.equal(overflowProblems(new Map([["Cargo.toml", root]]), ["compiler/x/.cargo/config.toml"]).length, 1);
});

/// Why some build could run without overflow checks: a profile of any manifest that turns them off,
/// a root [profile.release] that does not turn them on, or a .cargo/config anywhere in the tree,
/// which can change a profile where Cargo.toml does not show it.
export function overflowProblems(manifests, files) {
  const found = [];
  for (const [file, text] of manifests) {
    if (/overflow-checks\s*=\s*false/.test(text)) found.push(`${file} turns overflow checks off in a profile`);
  }
  const release = /^\[profile\.release\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(manifests.get("Cargo.toml") ?? "");
  if (!/^overflow-checks\s*=\s*true\s*$/m.test(release?.[1] ?? "")) found.push("Cargo.toml's [profile.release] does not set overflow-checks = true");
  for (const f of files) if (/(^|\/)\.cargo\/config(\.toml)?$/.test(f)) found.push(`${f} can change a profile unseen`);
  return found;
}

test("every compiler crate proves its overflow checks in a release build", () => {
  const crates = [...tracked()].filter((f) => /^compiler\/[^/]+\/Cargo\.toml$/.test(f)).map((f) => path.posix.dirname(f));
  assert.ok(crates.length > 0);
  for (const dir of crates) {
    const has = filesUnder(`${dir}/src`).some((f) => f.endsWith(".rs") && read(f).includes("fn test_arithmetic_overflow_panics_in_this_crate"));
    assert.ok(has, `${dir} has no test_arithmetic_overflow_panics_in_this_crate (src/overflow_checks.rs)`);
  }
  const job = workflows().find((w) => w.file === "ci.yml").doc.jobs["tier1-latency"];
  assert.ok(
    steps(job).some((s) => /cargo test --release .*--workspace/.test(String(s.run)) && String(s.run).includes("test_arithmetic_overflow_panics_in_this_crate") && !("if" in s)),
    "ci.yml's Tier-1 latency job does not run every crate's overflow test in a release build",
  );
  assert.ok(requiredChecks().includes("Tier-1 latency"));
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

/// Whether a value hands a secret over: an expression naming `secrets.` other than a test of
/// whether one is set.
const holdsSecret = (value) =>
  [...String(value).matchAll(/\$\{\{([\s\S]*?)\}\}/g)].some(([, e]) => /\bsecrets\./.test(e) && !/^\s*secrets\.\w+\s*[!=]=\s*''\s*$/.test(e));

test("a job that holds a secret runs agent code only in its proven sandbox", () => {
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

test("the Documentation job lints every workflow with actionlint and shellcheck, both pinned by checksum", () => {
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
];

/// Whether a CODEOWNERS pattern of the forms GATE_FILES uses owns a file: `/dir/` every file under
/// it, `/file` that file.
const owns = (pattern, file) => (pattern.endsWith("/") ? `/${file}`.startsWith(pattern) : `/${file}` === pattern);

test("CODEOWNERS gives the gate files, and only them, to their owner", () => {
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
  ];
  for (const f of inputs) {
    assert.ok(tracked().has(f), `${f}, a gate input, is not in the repository`);
    assert.ok(GATE_FILES.some((p) => owns(p, f)), `${f} is a gate input without an owner in .github/CODEOWNERS`);
  }
  assert.ok(owns("/.github/", ".github/workflows/ci.yml") && !owns("/perf/run.mjs", "perf/run.test.mjs") && owns("/Cargo.toml", "Cargo.toml"));
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
      assert.ok(!seen.has(name), `the check "${name}" is reported by ${seen.get(name)} and by ${file}#${id}: a required check of that name would be ambiguous`);
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

test("no step of a required job can let it pass without checking", () => {
  for (const { file, id, job } of requiredJobs()) {
    const watched = hasWatch(job);
    for (const [k, s] of steps(job).entries()) {
      const key = stepKey(file, id, s);
      if ("continue-on-error" in s) assert.ok(CONTINUE_ON_ERROR.has(key), `${key}: continue-on-error on a step of a required job`);
      if (watched && k <= 1) continue;
      if (watched) {
        assert.ok(WATCH_FORMS.includes(normalize(s.if ?? "")), `${key}: if: ${s.if ?? "(none)"} is not one of ${WATCH_FORMS.join(" | ")}`);
      } else if ("if" in s) {
        assert.equal(normalize(s.if), STEP_CONDITIONS.get(key), `${key}: if: ${s.if} could skip a step of a required job`);
      }
    }
  }
});

test("the matrix names a check as GitHub does", () => {
  assert.deepEqual(checkNames("a", { name: "Fuzz", strategy: { matrix: { include: [{ name: "x86-64 Linux" }] } } }), ["Fuzz (x86-64 Linux)"]);
  assert.deepEqual(checkNames("a", { name: "R (${{ matrix.name }})", strategy: { matrix: { include: [{ name: "L", os: "u" }] } } }), ["R (L)"]);
  assert.deepEqual(checkNames("job", { strategy: { matrix: { shard: [0, 1] } } }), ["job (0)", "job (1)"]);
  assert.deepEqual(checkNames("job", {}), ["job"]);
});

// ------------------------------------------------------------------ the watch and its path lists

test("every instrument workflow decides at its first step, and every later step obeys", () => {
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
    for (const key of ["if", "continue-on-error", "env"]) assert.ok(!(key in watch), `${where}: the watch step carries ${key}:`);
    for (const s of all.slice(2)) {
      assert.ok(WATCH_FORMS.includes(normalize(s.if ?? "")), `${where}, step "${s.name ?? s.uses ?? s.run}": its if: is not one of the watch's forms`);
    }
  }
  for (const file of sections.keys()) {
    assert.ok(allJobs().some((j) => j.file === file && steps(j.job)[1]?.id === "watch"), `${PATHS_FILE}: no job of ${file} reads [${file}]`);
  }
});

/// The workspace's members, each with the members it depends on by path.
function workspace() {
  const root = read("Cargo.toml");
  const members = [...(/members\s*=\s*\[([\s\S]*?)\]/.exec(root)?.[1] ?? "").matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  assert.ok(members.length > 0, "Cargo.toml lists no workspace members");
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

test("the path lists read as the watch reads them", () => {
  const sections = parsePaths(read(PATHS_FILE));
  assert.ok(sections.size > 0);
  for (const [file, section] of sections) {
    assert.ok(fs.existsSync(path.join(ROOT, WORKFLOWS, file)), `${PATHS_FILE}: [${file}] is not a workflow`);
    for (const p of section.patterns) {
      assert.ok([...tracked()].some((f) => globToRegExp(p).test(f)), `${PATHS_FILE}, [${file}]: ${p} matches no file of the repository`);
    }
  }
});

test("every path list names everything its jobs build, run and read", () => {
  const sections = parsePaths(read(PATHS_FILE));
  const crates = workspace();
  for (const [file, section] of sections) {
    const needed = new Set([`${WORKFLOWS}/${file}`]);
    for (const { doc, job } of allJobs().filter((j) => j.file === file)) {
      const texts = scriptTexts(doc, job);
      const { cargo, names } = cratesBuilt(texts, crates);
      if (cargo) {
        for (const f of ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"]) needed.add(f);
        for (const dir of closure(crates, names)) filesUnder(dir).forEach((f) => needed.add(f));
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

test("every instrument's corpus is read from the source that defines it", () => {
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

test("the crates and paths a job uses are read from its scripts", () => {
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

test("no script a required job runs can skip its own main", () => {
  const guard = /process\.argv\[1\][^\n]*import\.meta\.url|import\.meta\.url[^\n]*process\.argv\[1\]/;
  const scripts = new Set([".github/scripts/watch.mjs", ".github/scripts/commits.mjs", ".github/scripts/mutants-floor.mjs"]);
  for (const { doc, job } of requiredJobs()) {
    for (const f of repoPaths(scriptTexts(doc, job))) if (/\.(mjs|cjs|js|ts)$/.test(f) && !f.endsWith(".test.mjs")) scripts.add(f);
  }
  for (const f of scripts) assert.doesNotMatch(read(f), guard, `${f} runs main() only when the path it was started by matches its own`);
  for (const cli of [".github/scripts/watch.mjs", ".github/scripts/commits.mjs", ".github/scripts/mutants-floor.mjs", "perf/run.mjs"]) {
    assert.match(read(cli), /^\s*(?!\/\/).*\bmain\(/m, `${cli} does not call main() at its top level`);
  }
  assert.ok(scripts.has("benchmark/check.mjs") && scripts.has("perf/lib.mjs"), "the scripts of the required jobs are read");
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
  for (const f of [".cargo/config.toml", ".cargo/mutants.toml", "rust-toolchain", "rust-toolchain.toml", PATHS_FILE]) assert.ok(matches(ALWAYS, f), f);
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

test("the watch CLI decides on a real history, through any path, and fails rather than say nothing", () => {
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

test("the Commits check refuses unfinished subjects, CJK text and a changed base", () => {
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

test("the Commits check fails a pull request whose base was changed", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mente-commits-"));
  try {
    const sh = (...args) =>
      execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", ...args], { cwd: dir, encoding: "utf8" }).trim();
    sh("init", "-q");
    fs.writeFileSync(path.join(dir, "a"), "a\n");
    sh("add", "-A");
    sh("commit", "-q", "-m", "Base");
    const base = sh("rev-parse", "HEAD");
    fs.writeFileSync(path.join(dir, "a"), "b\n");
    sh("commit", "-q", "-am", "Change a");
    const head = sh("rev-parse", "HEAD");
    const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("GITHUB_")));
    const run = (payload) => {
      const event = path.join(dir, "event.json");
      fs.writeFileSync(event, JSON.stringify({ pull_request: { base: { sha: base }, head: { sha: head } }, ...payload }));
      return spawnSync(process.execPath, [path.join(HERE, "commits.mjs")], {
        cwd: dir,
        encoding: "utf8",
        env: { ...env, GITHUB_EVENT_NAME: "pull_request", GITHUB_EVENT_PATH: event, PR_TITLE: "Title", PR_BODY: "Body" },
      });
    };
    assert.equal(run({ action: "synchronize" }).status, 0);
    assert.equal(run({ action: "edited", changes: { body: { from: "Old" } } }).status, 0);
    const retargeted = run({ action: "edited", changes: { base: { ref: { from: "feature" }, sha: { from: base } } } });
    assert.equal(retargeted.status, 1);
    assert.match(retargeted.stdout, /base changed: push a commit to re-run the instruments/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("the Commits check rejects the code points test_repository_text_is_english rejects", () => {
  const main = read("compiler/mente-cli/src/main.rs");
  const body = /fn is_cjk\(c: char\) -> bool \{([\s\S]*?)\n\s*\}/.exec(main)?.[1];
  assert.ok(body, "is_cjk not found in compiler/mente-cli/src/main.rs");
  const ranges = [...body.matchAll(/0x([0-9A-Fa-f_]+)\.\.=0x([0-9A-Fa-f_]+)/g)].map((m) => [parseInt(m[1].replace(/_/g, ""), 16), parseInt(m[2].replace(/_/g, ""), 16)]);
  assert.deepEqual(CJK_RANGES, ranges);
});

// ------------------------------------------------------------------------------ floors of work

test("the Mutants floor counts what the diff changes, as cargo-mutants marks it", () => {
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
  assert.deepEqual([...deletions.get("compiler/a/src/lib.rs")], [41, 42]);
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
  assert.equal(expected(mutants, diff), 4, "the added lines' two, and both of g's around its deletion-only hunk");
  assert.deepEqual(deletedRegexes(mutants, diff, [mutants[0], mutants[1], mutants[3]]), ["^g at 48$"]);
  assert.equal(exactRegex("a.rs:1:2: replace <impl X for Y>::f -> bool with true (x|y)"), "^a\\.rs:1:2: replace <impl X for Y>::f \\-> bool with true \\(x\\|y\\)$");
  assert.throws(() => touched([{ file: "x", span: {} }], added), /no file, name and span/);
  assert.deepEqual(readList(""), []);
  assert.throws(() => readList("{}"), /not a JSON array/);
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

/// The base the pull request or merge group is checked against: POLICY_BASE, which the
/// Documentation job sets from the event, or the merge base with origin/main.
function policyBase() {
  if (process.env.POLICY_BASE) return process.env.POLICY_BASE;
  try {
    return git(["merge-base", "HEAD", "origin/main"]).trim();
  } catch {
    return "HEAD";
  }
}

test("every milestone done on the base has its tag", () => {
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

test("every instrument scheduled for a finished milestone is a required check", () => {
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
