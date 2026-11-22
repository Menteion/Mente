// The workflow policy: actions and permissions (README.md in this directory). What every workflow
// may run and with what token: pinned actions, read-only permissions, no expression in a script.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { parseYaml } from "../lib/yaml.mjs";
import { pins as actionPins, problems as actionPinProblems, taggedCommit } from "../lib/action-pins.mjs";
import { ROOT, read, tracked } from "./repo.mjs";
import { RUNNERS, runnersOf } from "./sandbox.mjs";
import { BROKER_STEP, HELD_OUT_STEP, SECRET_SHELL, isStep } from "./secrets.mjs";
import { REQUIRED_EVENTS, SHELLS, WORKFLOWS, allJobs, checkNames, isPlain, needsOf, permissionProblems, requiredChecks, requiredJobs, stepKey, steps, triggers, workflows } from "./workflows.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 14;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of actions-and-permissions.test.mjs ran: a skip or a filter left the rest out`);
});
// ------------------------------------------------------------------- safety of every workflow

check("every file under .github/ holds only printable ASCII, tab and LF", () => {
  // The first check, before any rule reads a workflow: GitHub's YAML reader breaks a line at NEL
  // (U+0085) and the line separators, where JavaScript's `.` and the readers here did not, so a
  // comment here was a step there, with no pin moved (the fourth review of PR 137). Every gate file
  // is ASCII today; anything else is refused, not read.
  const odd = [];
  for (const f of [...tracked()].filter((x) => x.startsWith(".github/"))) {
    const bytes = fs.readFileSync(path.join(ROOT, f));
    const k = bytes.findIndex((b) => b !== 0x09 && b !== 0x0a && (b < 0x20 || b > 0x7e));
    if (k >= 0) odd.push(`${f}: byte 0x${bytes[k].toString(16)} at offset ${k}`);
  }
  assert.deepEqual(odd, [], "files under .github/ hold bytes outside printable ASCII, tab and LF");
  // The readers refuse such a character too, so that no later reading can depend on this check.
  const nel = String.fromCharCode(0x85);
  assert.throws(() => parseYaml(`jobs:\n  a:\n    steps:\n      - run: x\n      # x${nel}        continue-on-error: true\n`), /outside printable ASCII/);
  assert.match(actionPins("w.yml", `      # x${nel}      - uses: someone/action@main\n`)[0].unreadable, /outside printable ASCII/);
});

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

check("every workflow's permissions are GitHub's scopes and values exactly, and no token can post a check run or a commit status", () => {
  for (const { file, doc } of workflows()) assert.deepEqual(permissionProblems(file, doc), []);
  // What passes: read alone, read-all, none at all, and the Pages job's pages and id-token write.
  const base = { permissions: { contents: "read" }, jobs: { j: { "runs-on": "ubuntu-24.04" } } };
  const job = (permissions) => ({ ...base, jobs: { j: { "runs-on": "ubuntu-24.04", permissions } } });
  for (const [what, doc] of [
    ["contents read", base],
    ["read-all", { ...base, permissions: "read-all" }],
    ["no permission at all", { ...base, permissions: {} }],
    ["vulnerability-alerts read", job({ "vulnerability-alerts": "read" })],
    ["a job with none of its own", { ...base, jobs: { j: { "runs-on": "ubuntu-24.04" } } }],
  ]) {
    assert.deepEqual(permissionProblems("x.yml", doc), [], what);
  }
  const deploy = (permissions, id = "deploy", top = { contents: "read" }) => ({ permissions: top, jobs: { [id]: { "runs-on": "ubuntu-24.04", permissions } } });
  assert.deepEqual(permissionProblems("pages-publish.yml", deploy({ contents: "read", pages: "write", "id-token": "write" })), [], "the Pages deployment's");
  // Planted: Pages or an OIDC token written anywhere else: another job, another workflow, the
  // publishing workflow's top, and its build job.
  for (const [what, file, doc] of [
    ["pages write in another job", "pages-publish.yml", deploy({ pages: "write" }, "build")],
    ["id-token write in another job", "pages-publish.yml", deploy({ "id-token": "write" }, "build")],
    ["pages write at the publishing workflow's top", "pages-publish.yml", deploy({ contents: "read" }, "deploy", { contents: "read", pages: "write" })],
    ["pages write in another workflow's job of that name", "ci.yml", deploy({ pages: "write" })],
    ["id-token write at another workflow's top", "x.yml", deploy({ contents: "read" }, "j", { "id-token": "write" })],
  ]) {
    assert.ok(permissionProblems(file, doc).some((p) => p.includes("which only pages-publish.yml#deploy may hold")), what);
  }
  // Planted, at the top and in a job: each way to post one, a key or a value GitHub does not name
  // as written, an expression, and the probe's own forging job.
  const forger = parseYaml(`name: Spoof
on:
  pull_request:
    branches: [main]
permissions:
  contents: read
jobs:
  forger:
    runs-on: ubuntu-24.04
    permissions:
      contents: read
      statuses: write
      checks: write
    steps:
      - run: gh api "repos/$GITHUB_REPOSITORY/check-runs" -f name="Workflow policy" -f head_sha="$SHA" -f status=completed -f conclusion=success
`);
  for (const [what, doc] of [
    ["checks write at the top", { ...base, permissions: { contents: "read", checks: "write" } }],
    ["statuses write at the top", { ...base, permissions: { contents: "read", statuses: "write" } }],
    ["write-all at the top", { ...base, permissions: "write-all" }],
    ["checks write in a job", job({ checks: "write" })],
    ["statuses write in a job", job({ statuses: "write" })],
    ["write-all in a job", job("write-all")],
    ["Checks: write", job({ Checks: "write" })],
    ["checks: Write", job({ checks: "Write" })],
    ["Write-All", job("Write-All")],
    ["Read-All", { ...base, permissions: "Read-All" }],
    ["an expression for the whole block", job("${{ inputs.permissions }}")],
    ["an expression for a value", job({ contents: "${{ inputs.access }}" })],
    ["a scope GitHub does not name", job({ models: "read" })],
    ["id-token read, which it does not admit", job({ "id-token": "read" })],
    ["vulnerability-alerts write, which it does not admit", job({ "vulnerability-alerts": "write" })],
    ["a value that is no string", job({ contents: true })],
    ["permissions with no value", job(null)],
    ["a list", job(["contents"])],
    ["the probe's forging job", forger],
  ]) {
    assert.ok(permissionProblems("x.yml", doc).length > 0, `a workflow with ${what} passes`);
  }
});

/// What is wrong with the files of .github/workflows: anything but a workflow the policy reads,
/// a file directly in it named .yml or .yaml, exactly so. A file GitHub might run that workflows()
/// does not read (x.YML) would escape every rule.
export const workflowFileProblems = (paths) =>
  [...paths].filter((f) => f.startsWith(`${WORKFLOWS}/`) && !/^[^/]+\.(yml|yaml)$/.test(f.slice(WORKFLOWS.length + 1))).map((f) => `${f}: a file of ${WORKFLOWS} the policy does not read as a workflow`);

check("every file of .github/workflows is a workflow the policy reads", () => {
  assert.deepEqual(workflowFileProblems(tracked()), []);
  assert.deepEqual(workflowFileProblems(new Set([`${WORKFLOWS}/ci.yml`, `${WORKFLOWS}/x.yaml`, "docs/x.YML"])), []);
  for (const f of ["x.YML", "x.Yaml", "x.yml.bak", "x", "sub/x.yml", ".x.json"]) {
    assert.ok(workflowFileProblems(new Set([`${WORKFLOWS}/${f}`])).length > 0, `${f} in ${WORKFLOWS} passes`);
  }
  // And workflows() reads exactly the files this admits.
  const read = workflows().map(({ file }) => `${WORKFLOWS}/${file}`).sort();
  const admitted = [...tracked()].filter((f) => f.startsWith(`${WORKFLOWS}/`)).sort();
  assert.deepEqual(read, admitted);
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
    for (const p of actionPins(`${WORKFLOWS}/${file}`, read(`${WORKFLOWS}/${file}`))) {
      assert.equal(p.unreadable, undefined, `${p.at}: a uses: line the check of pins cannot read: ${p.unreadable}`);
      assert.notEqual(p.tag, null, `${p.at}: ${p.repo}@${p.sha} names no release in a comment`);
    }
  }
  const sha = (c) => c.repeat(40);
  const read2 = actionPins("w.yml", `      - uses: actions/checkout@${sha("a")} # v7.0.1\n        uses: org/repo/sub/path@${sha("b")}\n      - uses: ./local\n`);
  assert.deepEqual(read2.map((p) => [p.repo, p.tag]), [["actions/checkout", "v7.0.1"], ["org/repo", null]]);
  for (const line of [`      - "uses": actions/checkout@${sha("a")} # v7`, `      - { uses: actions/checkout@${sha("a")} }`, `      - uses: actions/checkout@${sha("a")} # v7 and more`, "      - uses: actions/checkout@v7", "      - uses: docker://alpine"]) {
    assert.equal(actionPins("w.yml", line).filter((p) => p.unreadable !== undefined).length, 1, `the check of pins passes over: ${line}`);
  }
  assert.equal(taggedCommit(`${sha("c")}\trefs/tags/v1\n${sha("d")}\trefs/tags/v1^{}\n`, "v1"), sha("d"), "an annotated tag is its peeled commit");
  assert.equal(taggedCommit(`${sha("c")}\trefs/tags/v1\n`, "v1"), sha("c"));
  assert.equal(taggedCommit("", "v1"), null);
  const remote = (repo, tag) => (tag === "v7.0.1" ? `${sha("a")}\trefs/tags/v7.0.1\n` : "");
  assert.deepEqual(actionPinProblems(read2.slice(0, 1), remote), []);
  const moved = actionPinProblems([{ ...read2[0], sha: sha("e") }], remote);
  assert.equal(moved.length, 1, "a commit other than its tag's");
  // A tag that names another commit never suggests following the tag: the pin stays.
  assert.match(moved[0], /keep the pin/, `the check of pins suggests following the tag: ${moved[0]}`);
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

/// The jobs whose matrix a dispatch's input gives, each with its reason; every other matrix is a
/// literal. Such a job's workflow runs on no pull request, push or queue, and its name's literal
/// start is the start of no required check.
const MATRIX_FROM_INPUTS = new Map([["mutants-full.yml#full", "the shards to run are the dispatch's choice"]]);
/// The jobs whose matrix the plan of their workflow makes, each with its reason, by one key the name
/// reads: `${{ fromJSON(needs.<plan>.outputs.<key>s) }}` for a plan it needs, every leg run
/// (fail-fast off). Its name's literal start is the start of no required check, and its name, its
/// matrix value read as any text, is no required check, so no leg can report one; the aggregate
/// its plan feeds reads every leg's result.
const MATRIX_FROM_PLAN = new Map([["mutants.yml#shard", "the shards the plan made, as many as the diff's mutants need, which only the plan can count"]]);
/// Why a job of MATRIX_FROM_PLAN takes its matrix otherwise than from its plan, or could report a
/// required check.
export function planMatrixProblems(key, job, required) {
  const found = [];
  const strategy = job?.strategy;
  const name = String(job?.name ?? "");
  const refs = [...name.matchAll(/\$\{\{\s*matrix\.([\w-]+)\s*\}\}/g)].map((m) => m[1]);
  const matrixKey = refs[0];
  if (refs.length !== 1) found.push(`${key}: its name reads ${refs.length} matrix values, not one`);
  if (!isPlain(strategy) || Object.keys(strategy).some((k) => k !== "fail-fast" && k !== "matrix") || strategy["fail-fast"] !== false) found.push(`${key}: its strategy is more than its matrix with fail-fast: false`);
  const matrix = strategy?.matrix;
  const value = isPlain(matrix) && Object.keys(matrix).length === 1 ? matrix[matrixKey] : undefined;
  const from = /^\$\{\{ fromJSON\(needs\.([\w-]+)\.outputs\.([\w-]+)\) \}\}$/.exec(String(value));
  if (!from || !needsOf(job).includes(from[1]) || from[2] !== `${matrixKey}s`) found.push(`${key}: its matrix is not \${{ fromJSON(needs.<plan>.outputs.${matrixKey}s) }} for a plan it needs`);
  const prefix = name.split("${{")[0];
  const pattern = new RegExp(`^${name.split(/\$\{\{\s*matrix\.[\w-]+\s*\}\}/).map((p) => p.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("[\\s\\S]*")}$`);
  if (prefix.trim() === "" || required.some((c) => c.startsWith(prefix) || pattern.test(c))) found.push(`${key}: its name, "${name}", could report a required check`);
  return found;
}
/// Why a workflow's jobs could be named, run or set otherwise than the policy reads them: a name
/// spelled by an expression other than a matrix value, a runner that is no literal of RUNNERS, a
/// matrix, strategy or defaults holding an expression, an env or a step's inputs that are an
/// expression as a whole. The policy reads each as written; GitHub would evaluate it (the third
/// review of 980965e: `env: ${{ fromJSON(...) }}` set NODE_OPTIONS past TOOL_ENV, and a job named
/// by format() reported a required check's name).
/// The values a matrix gives a key in every combination, or null when some combination lacks the key
/// or the matrix is no literal; and whether a value renders in a name as the policy reads it: an
/// integer, or a word that starts with a letter and is none YAML or GitHub reads otherwise (the
/// fourth review of PR 137: `matrix.Name` for a key `name`, a key absent, `~` and `0.0` each rendered
/// otherwise than the policy read them).
export function matrixValuesOf(matrix, key) {
  if (!isPlain(matrix)) return null;
  if (Array.isArray(matrix[key])) return matrix[key];
  if (key in matrix && key !== "include" && key !== "exclude") return null;
  const include = matrix.include;
  if (!Array.isArray(include) || include.length === 0 || Object.keys(matrix).some((k) => k !== "include")) return null;
  return include.every((c) => isPlain(c) && Object.hasOwn(c, key)) ? include.map((c) => c[key]) : null;
}
const YAML_WORDS = /^(null|true|false|yes|no|on|off|y|n)$/i;
export const plainNameValue = (v) => (typeof v === "number" && Number.isInteger(v) && v >= 0) || (typeof v === "string" && /^[A-Za-z][A-Za-z0-9 _.,()+/-]*$/.test(v) && !YAML_WORDS.test(v));
export function expressionProblems(file, doc) {
  const found = [];
  const expression = (v) => /\$\{\{/.test(JSON.stringify(v ?? null));
  const mapping = (where, v) => {
    if (v !== undefined && !isPlain(v)) found.push(`${where} is an expression or no mapping: ${JSON.stringify(v)}`);
  };
  mapping(`${file}: env`, doc.env);
  if (doc.defaults !== undefined && (!isPlain(doc.defaults) || expression(doc.defaults))) found.push(`${file}: defaults are or hold an expression`);
  const runsOnPush = Object.keys(triggers(doc)).some((e) => REQUIRED_EVENTS.has(e));
  for (const [id, job] of Object.entries(doc.jobs ?? {})) {
    const key = `${file}#${id}`;
    const name = String(job?.name ?? id);
    if (/\$\{\{/.test(name.replace(/\$\{\{\s*matrix\.[\w-]+\s*\}\}/g, ""))) found.push(`${key}: its name holds an expression other than a matrix value: ${name}`);
    const planned = MATRIX_FROM_PLAN.has(key) && runsOnPush;
    if (planned) found.push(...planMatrixProblems(key, job, requiredChecks()));
    if (!MATRIX_FROM_INPUTS.has(key) && !planned) {
      const matrix = job?.strategy?.matrix;
      for (const [, ref] of name.matchAll(/\$\{\{\s*matrix\.([\w-]+)\s*\}\}/g)) {
        const values = matrixValuesOf(matrix, ref);
        if (!values) found.push(`${key}: its name names matrix.${ref}, which its matrix does not give, in that very spelling, to every combination`);
        else if (!values.every(plainNameValue)) found.push(`${key}: its name names matrix.${ref}, whose values ${JSON.stringify(values)} GitHub could render otherwise than the policy reads them`);
      }
      if (isPlain(matrix)) {
        const keys = [...Object.keys(matrix), ...(Array.isArray(matrix.include) ? matrix.include.flatMap((c) => (isPlain(c) ? Object.keys(c) : [])) : [])];
        const byCase = new Map();
        for (const k of keys) {
          const held = byCase.get(k.toLowerCase());
          if (held !== undefined && held !== k) found.push(`${key}: its matrix holds the keys ${held} and ${k}, which GitHub reads as one`);
          byCase.set(k.toLowerCase(), k);
        }
      }
    }
    if (job?.strategy !== undefined && (!isPlain(job.strategy) || expression(job.strategy))) {
      const prefix = name.split("${{")[0];
      const allowed = (MATRIX_FROM_INPUTS.has(key) && !runsOnPush && prefix.trim() !== "" && ![...requiredChecks()].some((c) => c.startsWith(prefix))) || (planned && planMatrixProblems(key, job, requiredChecks()).length === 0);
      if (!allowed) found.push(`${key}: its strategy or matrix is or holds an expression, so the checks it reports and its runners cannot be read`);
    }
    const runners = runnersOf(job);
    if (!MATRIX_FROM_INPUTS.has(key) && (!runners || !runners.every((r) => RUNNERS.has(r)))) found.push(`${key}: runs-on ${JSON.stringify(job?.["runs-on"])} is no literal of RUNNERS nor a matrix value that is one`);
    mapping(`${key}: env`, job?.env);
    if (job?.defaults !== undefined && (!isPlain(job.defaults) || expression(job.defaults))) found.push(`${key}: its defaults are or hold an expression`);
    for (const s of steps(job ?? {})) {
      mapping(`${stepKey(file, id, s)}: env`, s.env);
      mapping(`${stepKey(file, id, s)}: with`, s.with);
    }
  }
  return found;
}

check("no workflow names a job, its runners, its matrix, its defaults or a whole env by an expression", () => {
  for (const { file, doc } of workflows()) assert.deepEqual(expressionProblems(file, doc), [], file);
  // The plants of the third review of 980965e.
  const job = (extra) => ({ name: "Rust (x86-64 Linux)", "runs-on": "ubuntu-24.04", steps: [{ run: "x" }], ...extra });
  const doc = (jobs, extra = {}) => ({ on: { pull_request: null }, jobs, ...extra });
  for (const [what, plant] of [
    ["a name by format()", doc({ shadow: job({ name: "${{ format('Rust ({0})', 'x86-64 Linux') }}" }) })],
    ["a matrix by fromJSON", doc({ shadow: job({ name: "Rust (${{ matrix.name }})", strategy: { matrix: "${{ fromJSON('{\"name\":[\"x86-64 Linux\"]}') }}" } }) })],
    ["a matrix value by an expression", doc({ shadow: job({ name: "Rust (${{ matrix.name }})", strategy: { matrix: { name: ["${{ format('x86-64 Linux') }}"] } } }) })],
    ["a runner by format()", doc({ extra: job({ "runs-on": "${{ format('{0}-24.04', 'ubuntu') }}" }) })],
    ["a runner of another label", doc({ extra: job({ "runs-on": "self-hosted" }) })],
    ["a job env by fromJSON", doc({ z3: job({ env: "${{ fromJSON('{\"NODE_OPTIONS\":\"--require ./x.js\"}') }}" }) })],
    ["a workflow env by fromJSON", doc({ z3: job() }, { env: "${{ fromJSON('{}') }}" })],
    ["a step env by fromJSON", doc({ z3: job({ steps: [{ run: "x", env: "${{ fromJSON('{}') }}" }] }) })],
    ["a step's inputs by fromJSON", doc({ z3: job({ steps: [{ uses: "actions/checkout@x", with: "${{ fromJSON('{}') }}" }] }) })],
    ["defaults by an expression", doc({ z3: job({ defaults: { run: { shell: "${{ 'bash' }}" } } }) })],
    // The fourth review's plants: names GitHub renders otherwise than the policy reads them.
    ["a matrix key named in another case", doc({ shadow: job({ name: "Rust (${{ matrix.Name }})", strategy: { matrix: { name: ["x86-64 Linux"] } } }) })],
    ["a matrix key absent", doc({ shadow: job({ name: "Documentation${{ matrix.nope }}", strategy: { matrix: { os: ["ubuntu-24.04"] } } }) })],
    ["a null matrix value", doc({ shadow: job({ name: "Fuzz${{ matrix.x }}", strategy: { matrix: { x: [null] } } }) })],
    ["a matrix value written as a float", doc({ shadow: job({ name: "Mutants in the diff (shard ${{ matrix.s }} of 4)", strategy: { matrix: { s: ["0.0"] } } }) })],
    ["a matrix value YAML reads as a word", doc({ shadow: job({ name: "Fuzz ${{ matrix.x }}", strategy: { matrix: { x: ["Null"] } } }) })],
    ["a key in some include entries only", doc({ shadow: job({ name: "Rust (${{ matrix.name }})", strategy: { matrix: { include: [{ name: "x86-64 Linux" }, { os: "ubuntu-24.04" }] } } }) })],
    ["two keys GitHub reads as one", doc({ shadow: job({ name: "Rust (${{ matrix.name }})", strategy: { matrix: { name: ["a"], Name: ["b"] } } }) })],
  ]) {
    assert.ok(expressionProblems("plant.yml", plant).length > 0, `the expression rule lets through ${what}`);
  }
  // The shards' matrix, which only the plan can count, and the ways past it: a name that could
  // report "Mutants in the diff", a matrix from elsewhere or from a job it does not need, a leg that
  // stops the others, a name with no literal start.
  const shard = workflows().find((w) => w.file === "mutants.yml").doc.jobs.shard;
  const required = requiredChecks();
  assert.deepEqual(planMatrixProblems("mutants.yml#shard", shard, required), []);
  for (const [what, plant] of [
    ["a name that could be a required check", { ...shard, name: "Mutants in the diff${{ matrix.shard }}" }],
    ["a name with no literal start", { ...shard, name: "${{ matrix.shard }}" }],
    ["a matrix from an input", { ...shard, strategy: { "fail-fast": false, matrix: { shard: "${{ fromJSON(inputs.shards) }}" } } }],
    ["a matrix from a job it does not need", { ...shard, needs: [], strategy: { "fail-fast": false, matrix: { shard: "${{ fromJSON(needs.plan.outputs.shards) }}" } } }],
    ["a matrix of another output", { ...shard, strategy: { "fail-fast": false, matrix: { shard: "${{ fromJSON(needs.plan.outputs.count) }}" } } }],
    ["fail-fast left on", { ...shard, strategy: { matrix: shard.strategy.matrix } }],
    ["a second matrix key", { ...shard, strategy: { "fail-fast": false, matrix: { ...shard.strategy.matrix, os: ["ubuntu-24.04"] } } }],
  ]) {
    assert.ok(planMatrixProblems("mutants.yml#shard", plant, required).length > 0, `the plan's matrix rule lets through ${what}`);
  }
  assert.deepEqual(expressionProblems("fine.yml", doc({ rust: job({ name: "Rust (${{ matrix.name }})", "runs-on": "${{ matrix.os }}", strategy: { matrix: { include: [{ name: "x86-64 Linux", os: "ubuntu-24.04" }] } }, env: { MENTE_X: "${{ matrix.name }}" } }) })), []);
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

/// Whether a step's shell is one SHELLS names, or SECRET_SHELL on a secret step exactly.
const stepShellOk = (s) => SHELLS.has(s.shell) || (s.shell === SECRET_SHELL && (isStep(s, HELD_OUT_STEP) || isStep(s, BROKER_STEP)));

check("every step runs in a shell named bare", () => {
  // Planted: a template that could run nothing, and SECRET_SHELL on a step that is no secret step.
  assert.ok(stepShellOk(HELD_OUT_STEP) && stepShellOk(BROKER_STEP) && stepShellOk({ run: "x", shell: "bash" }));
  for (const s of [{ run: "x", shell: "true {0}" }, { run: "x", shell: SECRET_SHELL }, { ...BROKER_STEP, run: "true" }]) assert.ok(!stepShellOk(s), `a step with shell ${s.shell} and run ${s.run} passes`);
  for (const { file, doc } of workflows()) {
    const shell = doc.defaults?.run?.shell;
    if (shell !== undefined) assert.ok(SHELLS.has(shell), `${file}: defaults.run.shell: ${shell}`);
    for (const [id, job] of Object.entries(doc.jobs ?? {})) {
      const jobShell = job.defaults?.run?.shell;
      if (jobShell !== undefined) assert.ok(SHELLS.has(jobShell), `${file}, job ${id}: defaults.run.shell: ${jobShell}`);
      for (const s of steps(job)) {
        if ("shell" in s) assert.ok(stepShellOk(s), `${stepKey(file, id, s)}: shell: ${s.shell}`);
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
