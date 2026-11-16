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
// - On Linux the testing jobs set up and prove the sandbox the code of a change runs in before any
//   of it runs, and every step that runs it checks that it ran there.
// - Every milestone ROADMAP.md marks done on the base has its tag, and every instrument
//   docs/design-brief.md section 7.7 schedules for a finished milestone has its checks required.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseYaml } from "./lib/yaml.mjs";
import { ALWAYS, PATHS_FILE, decide, globToRegExp, matches, parsePaths } from "./lib/watch.mjs";
import { CJK_RANGES, badSubject, baseChangedSincePush, cjkLines, problems, pullRequestOf } from "./lib/commits.mjs";
import { deletedRegexes, evidence, exactRegex, expected, missing, parseDiff, readList, touched } from "./lib/mutants-floor.mjs";
import { doctestProblems, problems as testListProblems, overrideTests } from "./lib/test-list.mjs";
import { pins as actionPins, problems as actionPinProblems, taggedCommit } from "./lib/action-pins.mjs";
import { BUILD_TIME_CODE, CRATES_IO, approvedCrates, listedMembers, lockPackages, lockProblems, main as buildTimeCode, metadataProblems, toolchainProblems } from "./lib/build-time-code.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));

/// Every check of the policy counts itself when it runs, and the suite fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// FLOOR equal to the checks this file declares.
const FLOOR = 66;
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
/// The commit checked out at `root`, read from its .git as git keeps it: a directory or, in a
/// worktree, a file naming one, whose HEAD is a commit or a ref, found loose or packed, in it or in
/// the common directory its `commondir` names. The policy starts no process (policy-runtime-judge),
/// so it reads this from the files.
export function gitHead(root) {
  let gitDir = path.join(root, ".git");
  if (fs.statSync(gitDir).isFile()) gitDir = path.resolve(root, fs.readFileSync(gitDir, "utf8").replace(/^gitdir:\s*/, "").trim());
  const commonFile = path.join(gitDir, "commondir");
  const common = fs.existsSync(commonFile) ? path.resolve(gitDir, fs.readFileSync(commonFile, "utf8").trim()) : gitDir;
  const head = fs.readFileSync(path.join(gitDir, "HEAD"), "utf8").trim();
  const ref = /^ref:\s*(\S+)$/.exec(head)?.[1];
  if (ref === undefined) return head;
  for (const dir of [gitDir, common]) {
    const loose = path.join(dir, ref);
    if (fs.existsSync(loose)) return fs.readFileSync(loose, "utf8").trim();
  }
  const packed = path.join(common, "packed-refs");
  const line = fs.existsSync(packed) ? fs.readFileSync(packed, "utf8").split(/\r?\n/).find((l) => l.endsWith(` ${ref}`)) : undefined;
  if (line === undefined) throw new Error(`${root}: HEAD names ${ref}, which no ref file holds`);
  return line.split(" ")[0];
}

/// What .github/scripts/policy-collect.mjs gave the policy, run just before it (the Documentation
/// job's step, and npm run test:ci): the files git tracks and the milestones' facts. Refused when
/// they are of another commit than the one checked out, so that no stale facts pass.
const FACTS_FILE = path.join(ROOT, "target", "policy-facts.json");
/// The collector's facts from their text, refused unless they are of `head` and whole: an object
/// whose files and tags are names, each a string that is not empty, and whose base is a full commit
/// id, so that nothing the policy reads from them is of another type than it expects (the review of
/// 100e1e5).
export function readFacts(text, head) {
  if (text === null) throw new Error("no facts from the collector: run node .github/scripts/policy-collect.mjs first, as npm run test:ci does");
  const facts = JSON.parse(text);
  if (facts === null || typeof facts !== "object" || Array.isArray(facts)) throw new Error("the collector's facts are not an object: run node .github/scripts/policy-collect.mjs again");
  if (facts.head !== head) throw new Error(`the collector's facts are of ${facts.head}, not of ${head}, the commit checked out: run node .github/scripts/policy-collect.mjs again, as npm run test:ci does`);
  const names = (list) => Array.isArray(list) && list.every((name) => typeof name === "string" && name !== "");
  const whole =
    names(facts.tracked) &&
    facts.tracked.length > 0 &&
    typeof facts.base === "string" &&
    /^[0-9a-f]{40}$/.test(facts.base) &&
    names(facts.tags) &&
    typeof facts.baseRoadmap === "string";
  if (!whole) throw new Error("the collector's facts are not whole: run node .github/scripts/policy-collect.mjs again");
  return facts;
}
const FACTS = readFacts(fs.existsSync(FACTS_FILE) ? fs.readFileSync(FACTS_FILE, "utf8") : null, gitHead(ROOT));
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
/// nothing and pass. The one template allowed, SECRET_SHELL, stands only on a step that is
/// HELD_OUT_STEP or BROKER_STEP exactly, whose script the policy holds too.
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
  trackedCache ??= new Set(FACTS.tracked);
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

/// GitHub's permission scopes, each with the values it admits, from the workflow syntax's
/// `permissions` (read on 2026-11-16): `id-token` and `copilot-requests` take write or none,
/// `vulnerability-alerts` read or none, every other scope read, write or none.
const READ_WRITE_NONE = ["read", "write", "none"];
const PERMISSION_SCOPES = new Map([
  ["actions", READ_WRITE_NONE],
  ["artifact-metadata", READ_WRITE_NONE],
  ["attestations", READ_WRITE_NONE],
  ["checks", READ_WRITE_NONE],
  ["code-quality", READ_WRITE_NONE],
  ["contents", READ_WRITE_NONE],
  ["copilot-requests", ["write", "none"]],
  ["deployments", READ_WRITE_NONE],
  ["discussions", READ_WRITE_NONE],
  ["id-token", ["write", "none"]],
  ["issues", READ_WRITE_NONE],
  ["packages", READ_WRITE_NONE],
  ["pages", READ_WRITE_NONE],
  ["pull-requests", READ_WRITE_NONE],
  ["security-events", READ_WRITE_NONE],
  ["statuses", READ_WRITE_NONE],
  ["vulnerability-alerts", ["read", "none"]],
]);

/**
 * What is wrong with a workflow's permissions, at its top and in each job. They are an allow-list,
 * not a reading of what GitHub might accept: the shorthand `read-all`, or a map whose every key is a
 * scope of PERMISSION_SCOPES and whose every value is one that scope admits, exactly as written.
 * Anything else is refused, other cases and expressions (`${{ }}`) included. And no token may post
 * a check run or a commit status: `write-all`, `checks: write` and `statuses: write` are refused.
 * GitHub matches a required check by its name and its source, and every required check of main
 * comes from GitHub Actions, as such a post would. The probe of policy-runtime-judge (2026-11-16,
 * judge-probe's pull request 3) saw a change's own workflow raise its token so and post a success
 * status and a success check run under the judge's name. The judge posts through its own App, so
 * no workflow needs either permission.
 */
export function permissionProblems(file, doc) {
  const found = [];
  const readPermissions = (where, p) => {
    if (p === "read-all") return;
    if (p === "write-all") {
      found.push(`${where}: write-all, by which its token may post check runs or commit statuses`);
      return;
    }
    if (!isPlain(p)) {
      found.push(`${where}: permissions that are neither read-all nor a map of GitHub's scopes: ${JSON.stringify(p)}`);
      return;
    }
    for (const [scope, value] of Object.entries(p)) {
      const admits = PERMISSION_SCOPES.get(scope);
      if (admits === undefined) found.push(`${where}: ${JSON.stringify(scope)}, no permission scope of GitHub's`);
      else if (!admits.includes(value)) found.push(`${where}: ${scope}: ${JSON.stringify(value)}, none of ${admits.join(", ")}`);
      else if ((scope === "checks" || scope === "statuses") && value === "write") found.push(`${where}: ${scope}: write, by which its token may post check runs or commit statuses`);
    }
  };
  // A workflow's own permissions are required by the check above; a job's may be left out, and
  // then are the workflow's.
  if (doc.permissions !== undefined) readPermissions(file, doc.permissions);
  for (const [id, job] of Object.entries(isPlain(doc.jobs) ? doc.jobs : {})) {
    if (isPlain(job) && "permissions" in job) readPermissions(`${file}, job ${id}`, job.permissions);
  }
  return found;
}

check("every workflow's permissions are GitHub's scopes and values exactly, and no token can post a check run or a commit status", () => {
  for (const { file, doc } of workflows()) assert.deepEqual(permissionProblems(file, doc), []);
  // What passes: read alone, read-all, none at all, and the Pages job's pages and id-token write.
  const base = { permissions: { contents: "read" }, jobs: { j: { "runs-on": "ubuntu-24.04" } } };
  const job = (permissions) => ({ ...base, jobs: { j: { "runs-on": "ubuntu-24.04", permissions } } });
  for (const [what, doc] of [
    ["contents read", base],
    ["read-all", { ...base, permissions: "read-all" }],
    ["no permission at all", { ...base, permissions: {} }],
    ["the Pages job's", job({ pages: "write", "id-token": "write" })],
    ["vulnerability-alerts read", job({ "vulnerability-alerts": "read" })],
    ["a job with none of its own", { ...base, jobs: { j: { "runs-on": "ubuntu-24.04" } } }],
  ]) {
    assert.deepEqual(permissionProblems("x.yml", doc), [], what);
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
  // The proof every release-building required job runs: every crate's test in release, each
  // crate's run on its own, counted.
  const proof = read(".github/scripts/overflow-proof.sh");
  assert.match(proof, /cargo test --release --locked -p "\$name" "\$\{targets\[@\]\}" -- --exact "\$test" > "\$log" 2>&1 \|\| status=\$\?/);
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
/// The crate whose functions alone start threads (clippy.toml's disallowed-methods), and which
/// alone does not inherit the workspace's lints.
const THREADS_CRATE = "mente-threads";
/// What clippy.toml bans outside it: the core count, every safe way std starts a thread, and the
/// ways a test could hide from its check the variables the check reads.
const THREAD_BANS = [
  "std::thread::available_parallelism",
  "std::thread::Scope::spawn",
  "std::thread::spawn",
  "std::thread::Builder::spawn",
  "std::thread::Builder::spawn_scoped",
  "std::env::set_var",
  "std::env::remove_var",
  "std::process::Command::env_clear",
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
    assert.ok(allowed && allowed.array === t.array, `.config/nextest.toml has ${shown}: to give nextest another table, add it to NEXTEST_TABLES in .github/scripts/policy.test.mjs with the keys it may hold`);
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

/// .cargo/mutants.toml, as the few forms the policy reads: a bare key at the start of a line, then
/// `true`, `false`, one string, or a list of strings over one line or several, with comments. A
/// string is literal ('...') or holds no escape ("..."). Anything else is an error rather than a
/// line skipped: a quoted or dotted key, a table, an escape, which could each spell a key or a
/// value otherwise to cargo-mutants than to this.
export function mutantsSettings(text, file = ".cargo/mutants.toml") {
  const settings = new Map();
  let k = 0;
  const fail = (why) => {
    throw new Error(`${file}:${text.slice(0, k).split("\n").length}: ${why}`);
  };
  // A carriage return ends a line only before a line feed; alone, TOML refuses it or reads it as a
  // line's end, where this reader would read a space.
  k = text.search(/\r(?!\n)/);
  if (k >= 0) fail("a carriage return that does not end its line");
  k = 0;
  const skip = (lines) => {
    for (;;) {
      const c = text[k];
      if (c === " " || c === "\t" || c === "\r" || (lines && c === "\n")) k++;
      else if (c === "#") while (k < text.length && text[k] !== "\n") k++;
      else return;
    }
  };
  const string = () => {
    const quote = text[k];
    const end = text.indexOf(quote, k + 1);
    const line = text.indexOf("\n", k + 1);
    if (end < 0 || (line >= 0 && line < end)) fail("a string that does not close on its line");
    const body = text.slice(k + 1, end);
    if (quote === '"' && body.includes("\\")) fail("a string with an escape, which could spell anything");
    k = end + 1;
    return body;
  };
  for (;;) {
    skip(true);
    if (k >= text.length) break;
    if (k > 0 && text[k - 1] !== "\n") fail("a key that does not start its line");
    const key = /^[a-z][a-z0-9_]*(?= = )/.exec(text.slice(k, k + 100))?.[0];
    if (!key) fail("not a bare key and its value");
    if (settings.has(key)) fail(`${key} twice`);
    k += key.length + 3;
    let value;
    if (text[k] === "[") {
      k++;
      value = [];
      for (;;) {
        skip(true);
        if (text[k] === "]") {
          k++;
          break;
        }
        if (text[k] !== "'" && text[k] !== '"') fail(`${key} holds something other than strings`);
        value.push(string());
        skip(true);
        if (text[k] === ",") k++;
        else if (text[k] !== "]") fail(`${key}: a list that does not go on with , or end with ]`);
      }
    } else if (text[k] === "'" || text[k] === '"') {
      value = string();
    } else {
      const word = /^(true|false)(?![\w.-])/.exec(text.slice(k, k + 8))?.[1];
      if (!word) fail(`${key} is neither true, false, a string nor a list of strings`);
      value = word === "true";
      k += word.length;
    }
    skip(false);
    if (k < text.length && text[k] !== "\n") fail(`more after the value of ${key}`);
    settings.set(key, value);
  }
  return settings;
}

const strings = (v) => Array.isArray(v) && v.every((s) => typeof s === "string");
/// What .cargo/mutants.toml may set, each key with what its value must be. cargo-mutants reads the
/// file of the change in every mutation run, outside the tests' sandbox, and other keys of it hand
/// cargo or nextest whatever arguments they hold (`additional_cargo_args`,
/// `additional_cargo_test_args`): a wrapper around rustc, a runner, or another nextest
/// configuration, whose setup scripts are no test executables and so never reach the sandbox.
/// These four choose the tests and the mutants, and name no command, no argument and no file. To
/// give cargo-mutants another setting, add its key here with its reason, where review sees it.
const MUTANTS_KEYS = new Map([
  // Every mutant runs against the whole workspace's tests.
  ["test_workspace", (v) => v === true],
  // Through nextest, whose limits and whose runner, the sandbox, the policy holds.
  ["test_tool", (v) => v === "nextest"],
  // The files and the mutants left out, each with its reason in the file.
  ["exclude_globs", strings],
  ["exclude_re", strings],
]);

/// Why a .cargo/mutants.toml could take a command of the change past the sandbox, or run the
/// mutants' tests otherwise than through nextest over the whole workspace.
export function mutantsSettingsProblems(text) {
  let settings;
  try {
    settings = mutantsSettings(text);
  } catch (e) {
    return [e.message];
  }
  const found = [];
  for (const [key, value] of settings) {
    const allowed = MUTANTS_KEYS.get(key);
    if (!allowed) found.push(`.cargo/mutants.toml sets ${key}, which MUTANTS_KEYS in .github/scripts/policy.test.mjs does not hold: cargo-mutants reads this file outside the tests' sandbox, and a key of it can hand cargo or nextest a wrapper, a runner or another configuration`);
    else if (!allowed(value)) found.push(`.cargo/mutants.toml: ${key} = ${JSON.stringify(value)} is not a value the policy holds for it`);
  }
  for (const key of ["test_workspace", "test_tool"]) {
    if (!settings.has(key)) found.push(`.cargo/mutants.toml does not set ${key}, on which the mutation runs' limits and sandbox stand`);
  }
  return found;
}

check("the mutation runs' settings name no command, no argument for cargo and no other file", () => {
  assert.deepEqual(mutantsSettingsProblems(read(".cargo/mutants.toml")), []);
  const base = 'test_workspace = true\ntest_tool = "nextest"\n';
  assert.deepEqual(mutantsSettingsProblems(`# c\n${base}exclude_globs = ["spikes/**"] # why\nexclude_re = [\n  # one\n  'a, [b] # \\(c\\)$',\n  "d",\n]\n`), []);
  assert.deepEqual(mutantsSettings("exclude_re = [\n  'a, [b] # \\(c\\)$', # why\n  \"d\"\n]\n").get("exclude_re"), ["a, [b] # \\(c\\)$", "d"]);
  // The plants of the review of pull request 137, and the other keys and spellings that name a
  // command, an argument or a file.
  for (const plant of [
    'additional_cargo_test_args = ["--config-file", "compiler/mente-cli/tests/nextest-plant.toml"]\n',
    "additional_cargo_args = [\"--config\", \"build.rustc-wrapper='w.sh'\"]\n",
    'additional_cargo_args = ["--config", "target.x86_64-unknown-linux-gnu.runner=\'env\'"]\n',
    '"additional_cargo_args" = ["--all-features"]\n',
    "'additional_cargo_args' = ['--all-features']\n",
    '"\\u0061dditional_cargo_args" = ["--all-features"]\n',
    'output = "/tmp/elsewhere"\n',
    'profile = "plant"\n',
    'features = ["plant"]\n',
    "all_features = true\n",
    "timeout_multiplier = 0.1\n",
    "minimum_test_timeout = 0\n",
    'error_values = ["x"]\n',
    'examine_globs = ["none/**"]\n',
    "skip_calls = ['f']\n",
    "[package]\n",
    "tool.test = true\n",
    'exclude_re = ["a\\\\u0062"]\n',
    'exclude_globs = ["a", 1]\n',
    'exclude_globs = "spikes/**"\n',
    "exclude_globs = ['a'] 'b'\n",
    "  exclude_globs = ['a']\nexclude_globs = ['b']\n",
    // L-4 of the second review of 4e896ca: a carriage return alone, which this reader took for a
    // space and TOML refuses or reads as a line's end.
    "exclude_globs = ['a']\rexclude_re = ['b']\n",
    "exclude_globs = ['a'] # c\rexclude_re = ['b']\n",
  ]) {
    assert.ok(mutantsSettingsProblems(`${base}${plant}`).length > 0, `the settings' reader lets through: ${plant}`);
  }
  for (const text of ['test_workspace = false\ntest_tool = "nextest"\n', 'test_workspace = true\ntest_tool = "cargo"\n', "test_workspace = true\n", 'test_tool = "nextest"\n', `${base}${base}`]) {
    assert.ok(mutantsSettingsProblems(text).length > 0, `the settings' reader lets through: ${text}`);
  }
});

/// The attributes .gitattributes may set: line ends, and the language and documentation GitHub
/// counts a file as. Every other is refused: `linguist-generated`, which folds a file's diff away on
/// GitHub's review page; and above all those that hide a change from a diff, which the Mutants
/// floor reads (H-1 of the second review of 4e896ca: `compiler/** -diff` made git write "Binary
/// files differ", and cargo-mutants read that as a diff with no line): `-diff`, `diff=`, `binary`,
/// a macro, and the filters, merge drivers and export rules that make the tree another than the
/// commit's.
const GIT_ATTRIBUTES = [/^text$/, /^text=auto$/, /^eol=lf$/, /^linguist-language=[A-Za-z][\w+#-]*$/, /^linguist-documentation=true$/];
export function gitattributesProblems(text) {
  const found = [];
  for (const [k, raw] of String(text).split("\n").entries()) {
    const line = raw.replace(/\r$/, "");
    const where = `.gitattributes:${k + 1}`;
    if (line.includes("\r")) found.push(`${where}: a carriage return inside a line`);
    if (line.trim() === "" || line.trim().startsWith("#")) continue;
    if (/^\s/.test(line)) found.push(`${where}: a line that does not start with its pattern`);
    const [pattern, ...attributes] = line.trim().split(/\s+/);
    if (/^["[!]/.test(pattern)) found.push(`${where}: a quoted pattern, a macro or a negation: ${pattern}`);
    if (attributes.length === 0) found.push(`${where}: a pattern with no attribute`);
    for (const a of attributes) if (!GIT_ATTRIBUTES.some((re) => re.test(a))) found.push(`${where}: ${a}, an attribute GIT_ATTRIBUTES in .github/scripts/policy.test.mjs does not allow; -diff, diff=, binary, filter=, merge= and export-ignore hide a change from a diff or make the tree another than the commit's`);
  }
  return found;
}
/// A Rust source as its tokens read, for a rule over them: each comment, nested block comments
/// too, made one space, and each string, raw string or character literal made `""`. One pass, so
/// that no text makes a rule over it slow, and no `"/*"` in a string hides the code after it.
export function rustTokensText(text) {
  const out = [];
  const n = text.length;
  const raw = /[bc]?r(#*)"/y;
  let k = 0;
  let from = 0;
  const cut = (to, put) => {
    out.push(text.slice(from, k), put);
    k = to;
    from = to;
  };
  while (k < n) {
    const c = text[k];
    raw.lastIndex = k;
    const rawMatch = /[bcr]/.test(c) && !/\w/.test(text[k - 1] ?? "") ? raw.exec(text) : null;
    if (c === "/" && text[k + 1] === "/") {
      const end = text.indexOf("\n", k);
      cut(end < 0 ? n : end, " ");
    } else if (c === "/" && text[k + 1] === "*") {
      let depth = 0;
      let j = k;
      while (j < n) {
        if (text[j] === "/" && text[j + 1] === "*") {
          depth++;
          j += 2;
        } else if (text[j] === "*" && text[j + 1] === "/") {
          depth--;
          j += 2;
          if (depth === 0) break;
        } else j++;
      }
      cut(Math.min(j, n), " ");
    } else if (rawMatch) {
      const close = `"${rawMatch[1]}`;
      const end = text.indexOf(close, k + rawMatch[0].length);
      cut(end < 0 ? n : end + close.length, '""');
    } else if (c === '"') {
      let j = k + 1;
      while (j < n && text[j] !== '"') j += text[j] === "\\" ? 2 : 1;
      cut(Math.min(j + 1, n), '""');
    } else if (c === "'" && text[k + 1] === "\\") {
      let j = k + 3;
      while (j < n && text[j] !== "'" && text[j] !== "\n") j++;
      cut(Math.min(j + 1, n), '""');
    } else if (c === "'" && k + 1 < n && text[k + 1] !== "\n") {
      const width = text.codePointAt(k + 1) > 0xffff ? 2 : 1;
      if (text[k + 1 + width] === "'") cut(k + 2 + width, '""');
      else k++;
    } else k++;
  }
  out.push(text.slice(from));
  return out.join("");
}
/// Rust's whitespace beyond ASCII (Pattern_White_Space): NEL, the left-to-right and right-to-left
/// marks, and the line and paragraph separators. Rust and the parser cargo-mutants reads with take
/// each for a space between tokens; JavaScript's \s knows only the last two. None may stand in a
/// Rust source or in a file under compiler/ or spikes/.
const RUST_SPACE_BEYOND_ASCII = new RegExp(`[${String.fromCharCode(0x85, 0x200e, 0x200f, 0x2028, 0x2029)}]`);
/// `mutants::skip` as cargo-mutants 27.1.0 reads it, in its tokens: path segments `mutants` and
/// `skip`, a leading `::` or not, alone or under a cfg_attr, with any space, line or comment
/// between them, and raw identifiers. cargo-mutants has no option that disregards it, so the rule
/// is here. It takes a function out of every mutation run, the floor included.
const RUST_SPACE = `\\s${String.fromCharCode(0x85, 0x200e, 0x200f)}`;
const MUTANTS_SKIP = new RegExp(`\\b(?:r#)?mutants[${RUST_SPACE}]*::[${RUST_SPACE}]*(?:r#)?skip\\b`);
const namesMutantsSkip = (text) => MUTANTS_SKIP.test(rustTokensText(text));
/// The attributes of a Rust source, each the text between `#[` or `#![` and its closing bracket, in
/// the source's tokens (so no bracket in a string or a comment counts), in one pass.
export function rustAttributes(text) {
  const code = rustTokensText(text);
  const found = [];
  const open = /#\s*!?\s*\[/g;
  for (let m = open.exec(code); m; m = open.exec(code)) {
    let depth = 0;
    let k = m.index + m[0].length - 1;
    for (; k < code.length; k++) {
      if (code[k] === "[") depth++;
      else if (code[k] === "]" && --depth === 0) break;
    }
    found.push(code.slice(m.index + m[0].length, k));
    open.lastIndex = Math.min(k + 1, code.length);
    if (k >= code.length) break;
  }
  return found;
}
/// A `path = ...` in an attribute, alone or under a cfg_attr: the module it names is read from a
/// file of any name, wherever it lies (cargo-mutants 27.1.0 refuses only a leading `/`), so a
/// function put there escapes every rule that reads the sources by their names.
const PATH_ATTRIBUTE = /(?<![\w#])(?:r#)?path\s*=/;
const readsModuleFromPath = (text) => rustAttributes(text).some((a) => PATH_ATTRIBUTE.test(a));
/// `include!`, in the source's tokens, by any path (`std::include!`, `::core::include!`), with any
/// space or comment before its `!`, and as a raw identifier: it compiles another file's text into
/// this one, so a test or a function written there stands in a module whose file is not its own,
/// where `mn evidence` maps a test to the file it reads and cargo-mutants reads no module.
/// `include_str!` and `include_bytes!` read data, not code, and stay.
const INCLUDE_MACRO = new RegExp(`(?<![\\w#])(?:r#)?include[${RUST_SPACE}]*!`);
const includesSource = (text) => INCLUDE_MACRO.test(rustTokensText(text));
/// An attribute whose path ends in `test` with segments before it (`rust_analyzer::test`,
/// `miri::test`, `::test`): cargo-mutants 27.1.0 skips any item whose attribute path ends in `test`
/// (`attr_is_test`), as it skips `mutants::skip`, and clippy and rustfmt let some of them through (the
/// fourth review of PR 137: `#[rust_analyzer::test]` took 13 listed mutants to 0). The bare `#[test]`
/// marks a test, which no non-test code can call.
const TEST_SUFFIX = /::\s*(?:r#)?test(?!\w)(?!\s*::)/;
const skipsAsTest = (text) => rustAttributes(text).some((a) => TEST_SUFFIX.test(a));

check("git reads every change as text, and no source names mutants::skip, an attribute that ends in ::test, a module path of its own, or include!", () => {
  assert.deepEqual(gitattributesProblems(read(".gitattributes")), []);
  const nested = [...tracked()].filter((f) => f !== ".gitattributes" && path.posix.basename(f) === ".gitattributes");
  assert.deepEqual(nested, [], "a .gitattributes below the root, which the policy does not read");
  // Every Rust source of the repository, and every file under compiler/ and spikes/ whatever its
  // name: with no #[path], cargo-mutants reads modules only from .rs files under a crate's src/,
  // and a file of another name is read as text here, comments and all.
  const problems = [];
  for (const f of [...tracked()].filter((x) => x.endsWith(".rs") || x.startsWith("compiler/") || x.startsWith("spikes/"))) {
    const text = read(f);
    if (RUST_SPACE_BEYOND_ASCII.test(text)) problems.push(`${f}: a space of Rust's beyond ASCII (U+0085, U+200E, U+200F, U+2028 or U+2029)`);
    if (f.endsWith(".rs") ? namesMutantsSkip(text) : MUTANTS_SKIP.test(text)) problems.push(`${f}: mutants::skip, which takes a function out of every mutation run, the Mutants floor's included`);
    if (f.endsWith(".rs") && readsModuleFromPath(text)) problems.push(`${f}: a #[path] attribute, whose module could be a file of any name the rules here do not read`);
    if (f.endsWith(".rs") && includesSource(text)) problems.push(`${f}: include!, which compiles another file's text into this one, out of reach of the rules that read a module by its file`);
    if (f.endsWith(".rs") && skipsAsTest(text)) problems.push(`${f}: an attribute whose path ends in ::test, which cargo-mutants reads as a test and skips`);
  }
  assert.deepEqual(problems, []);
  // The plants of the third review of 980965e, H1: a module from a .inc file, and the spaces Rust
  // reads that JavaScript's \s does not.
  for (const plant of ['#[path = "plant.inc"] mod plant;', '#[cfg_attr(all(), path = "x.rs")]\nmod plant;', '#[ path="../../docs/x.rs" ]\nmod plant;', "#[r#path = \"x\"] mod plant;", '#[cfg_attr(miri, path = "a.inc")] pub mod plant;']) {
    assert.ok(readsModuleFromPath(`fn f() {}\n${plant}\n`), `the #[path] rule lets through: ${plant}`);
  }
  for (const fine of ["let path = 1;", "fn f(path: &str) {}", '#[doc = "path = x"]\nfn f() {}', '#[serde(rename = "path")]\nstruct S;', "// #[path = \"x.inc\"]\nmod m;", "#[derive(Debug)]\nstruct Path { path: u8 }"]) {
    assert.ok(!readsModuleFromPath(fine), `the #[path] rule refuses: ${fine}`);
  }
  // The plants of the round-5 review of PR 172: a test file that includes another file's tests.
  for (const plant of ['include!("helper.rs");', 'include ! ("x.rs")', 'std::include!("x.rs");', '::core::include!("x.rs")', 'r#include!("x.rs")', 'include/* c */!("x.rs")', `include${String.fromCharCode(0x85)}!("x.rs")`]) {
    assert.ok(includesSource(`mod tests {\n    ${plant}\n}\n`), `the include! rule lets through: ${plant}`);
  }
  for (const fine of ['include_str!("x.txt")', 'include_bytes!("x.bin")', '// include!("x.rs")', 'let s = "include!(x)";', "fn include() {}", "let include = 1;", "fn reinclude() {}"]) {
    assert.ok(!includesSource(fine), `the include! rule refuses: ${fine}`);
  }
  // The fourth review's plants: attributes cargo-mutants reads as a test, and so skips.
  for (const plant of ["#[rust_analyzer::test]", "#[miri::test]", "#[rustfmt::test]", "#[::test]", "#[a::b::test]", "#[tokio :: test]", "#[cfg_attr(miri, rust_analyzer::test)]", "#[rust_analyzer /* c */ :: test]"]) {
    assert.ok(skipsAsTest(`${plant}\npub fn f() -> u8 { 1 }\n`), `the ::test rule lets through: ${plant}`);
  }
  for (const fine of ["#[test]\nfn t() {}", "#[cfg(test)]\nmod tests {}", '#[serde(rename = "a::test")]\nstruct S;', "#[test_case::x]\nfn f() {}", "#[a::testing]\nfn f() {}", "#[a::test::b]\nfn f() {}", "// #[rust_analyzer::test]\nfn f() {}"]) {
    assert.ok(!skipsAsTest(fine), `the ::test rule refuses: ${fine}`);
  }
  // cargo-mutants also skips an `unsafe fn`: no member may write one, since each inherits the
  // workspace's lints and the workspace forbids unsafe code.
  assert.match(read("Cargo.toml"), /^\[workspace\.lints\.rust\]\n(?:#.*\n)*unsafe_code = "forbid"$/m, "the workspace no longer forbids unsafe code, whose functions cargo-mutants skips");
  const lintTables = (text, prefix) => {
    const tables = [];
    let current = null;
    for (const raw of text.split(/\r?\n/)) {
      const line = raw.replace(/\s+#.*$/, "").trim();
      if (line === "" || line.startsWith("#")) continue;
      // An array of tables, `[[bin]]` say, ends a lint table as a table does.
      if (/^\[\[[^\]]+\]\]$/.test(line)) {
        current = null;
        continue;
      }
      const header = /^\[([^\]]+)\]$/.exec(line);
      if (header) {
        current = header[1].startsWith(prefix) ? [header[1].slice(prefix.length), {}] : null;
        if (current) tables.push(current);
        continue;
      }
      const entry = /^([a-z][a-z0-9_-]*) = (.+)$/.exec(line);
      if (current) {
        assert.ok(entry, `a line of [${prefix}${current[0]}] the policy does not read: ${raw}`);
        current[1][entry[1]] = entry[2];
      }
    }
    return tables;
  };
  // Each member's lints, read as tables, not matched as text: a `"""` string could hold a decoy.
  for (const [name, { dir }] of workspace()) {
    const manifest = read(`${dir}/Cargo.toml`);
    assert.ok(!/"""|'''/.test(manifest), `${dir}/Cargo.toml holds a multi-line string, which the policy does not read`);
    if (name === THREADS_CRATE) continue;
    assert.deepEqual(lintTables(manifest, "lints"), [["", { workspace: "true" }]], `${name} does not inherit the workspace's lints, and only them, which forbid unsafe code and clippy's disallowed_methods`);
  }
  // The one crate that may start threads holds the workspace's lints as its own, but for
  // disallowed_methods, which it denies where the workspace forbids it.
  const workspaceLints = lintTables(read("Cargo.toml"), "workspace.lints.");
  const threadsLints = lintTables(read(`compiler/${THREADS_CRATE}/Cargo.toml`), "lints.");
  const expected = workspaceLints.map(([group, keys]) => [group, group === "clippy" ? { ...keys, disallowed_methods: '"deny"' } : keys]);
  assert.equal(Object.fromEntries(workspaceLints).clippy?.disallowed_methods, '"forbid"', "the workspace no longer forbids clippy's disallowed_methods, which keeps threads to mente-threads");
  assert.deepEqual(threadsLints, expected, `${THREADS_CRATE}'s lints are not the workspace's with disallowed_methods denied`);
  // The methods clippy.toml bans outside mente-threads: every way std starts a thread, and the
  // core count. A ban removed, which no #[expect] in mente-threads may notice, fails here.
  const clippy = read("clippy.toml").split(/\r?\n/).filter((l) => !/^\s*#/.test(l)).join("\n");
  const bans = clippy.slice(clippy.indexOf("disallowed-methods = ["), clippy.indexOf("]", clippy.indexOf("disallowed-methods = [") + 22));
  assert.deepEqual([...bans.matchAll(/path = "([^"]+)"/g)].map((m) => m[1]).sort(), [...THREAD_BANS].sort(), "clippy.toml's disallowed-methods are not THREAD_BANS");
  for (const space of [0x85, 0x200e, 0x200f, 0x2028, 0x2029].map((c) => String.fromCharCode(c))) {
    assert.ok(RUST_SPACE_BEYOND_ASCII.test(`#[cfg_attr(miri, mutants${space}::skip)]`));
    assert.ok(namesMutantsSkip(`#[rustfmt::skip]\n#[cfg_attr(miri, mutants${space}::${space}skip)] fn f() {}`), `the mutants::skip rule lets through U+${space.codePointAt(0).toString(16)}`);
  }
  // The plants of the second review of 4e896ca, H-1, and the other spellings.
  for (const plant of ["compiler/** -diff", "*.rs binary", "*.rs linguist-generated=true", "*.rs linguist-generated", "*.rs linguist-vendored", "*.rs diff=plain", "*.rs -text -diff", "*.rs !diff", "*.rs filter=lfs", "*.rs merge=ours", "* export-ignore", "[attr]hide -diff", "*.rs hide", '"compiler/a b.rs" text', "*.rs working-tree-encoding=UTF-16", "*.rs text eol=crlf", "  *.rs -diff", "*.rs", "*.rs text\r-diff"]) {
    assert.ok(gitattributesProblems(`* text=auto eol=lf\n${plant}\n`).length > 0, `the .gitattributes rule lets through: ${JSON.stringify(plant)}`);
  }
  assert.deepEqual(gitattributesProblems("# c\n\n* text=auto eol=lf\r\n*.mn text eol=lf linguist-language=Rust\nspec/*.md linguist-documentation=true\n"), []);
  for (const plant of [
    "#[mutants::skip]",
    "#[mutants :: skip]",
    "#[cfg_attr(test, mutants::skip)]",
    "#[mutants/* c */::/* d */skip]",
    "#[mutants/* a /* nested */ b */::skip]",
    "#[mutants // c\n::skip]",
    "#[::mutants::skip]",
    "#[r#mutants::r#skip]",
    // A string that opens a comment, or closes one, hides nothing after it.
    'const A: &str = "/*";\n#[mutants::skip]\nconst B: &str = "*/";',
    "const A: char = '\"';\n#[mutants::skip]\nconst B: &str = \"\";",
    'const A: &str = r#"/*"#;\n#[mutants::skip]',
    "fn f<'a>(x: &'a str) {}\n#[mutants::skip]",
  ]) {
    assert.ok(namesMutantsSkip(`fn f() {}\n${plant}\nfn g() {}\n`), `the mutants::skip rule lets through: ${plant}`);
  }
  for (const fine of ["mir_mutants::command(&root)", "mutants_skip", "mutants::skipped_count", "// #[mutants::skip]", "/* #[mutants::skip] */", 'const S: &str = "#[mutants::skip]";', 'const S: &str = r##"#[mutants::skip]"##;']) {
    assert.ok(!namesMutantsSkip(fine), `the mutants::skip rule refuses: ${fine}`);
  }
  // In time linear in the text, whatever it holds (CodeQL found the first pattern backtracking on
  // many repetitions of "*//*").
  const started = Date.now();
  for (const bulk of [`mutants${"*//*".repeat(50000)}`, `mutants ${"/* ".repeat(50000)}`, `${"mutants ".repeat(50000)}::`, `"${"\\\\".repeat(50000)}`, "'".repeat(100000)]) namesMutantsSkip(bulk);
  for (const bulk of ["#[".repeat(100000), `#[${"[".repeat(100000)}`, `${"#[a] ".repeat(50000)}`]) readsModuleFromPath(bulk);
  assert.ok(Date.now() - started < 2000, `the mutants::skip and #[path] rules took ${Date.now() - started} ms over 1 MB of text`);
});

/// The settings clippy and rustfmt read: each looks for `clippy.toml` or `.clippy.toml`, and
/// `rustfmt.toml` or `.rustfmt.toml`, from the crate's directory up, and takes the first it finds, so
/// one beside a crate, or a dotted one at the root, would stand in for the root's (the third review
/// of 980965e: an unowned `.clippy.toml` dropped the root's ban on RefCell, and the required clippy
/// step passed). The root's clippy.toml is the one, and code-owned.
const LINT_SETTINGS = /(^|\/)\.?(clippy|rustfmt)\.toml$/;

check("no lint or format settings stand but the root's clippy.toml", () => {
  const found = [...tracked()].filter((f) => LINT_SETTINGS.test(f) && f !== "clippy.toml");
  assert.deepEqual(found, [], "a clippy or rustfmt settings file other than the root's clippy.toml, which clippy or rustfmt would read instead of it for the crates below it");
  for (const f of [".clippy.toml", "compiler/mente-ast/clippy.toml", "compiler/mente-ast/.clippy.toml", "rustfmt.toml", ".rustfmt.toml", "compiler/mente-mir/rustfmt.toml"]) assert.match(f, LINT_SETTINGS, f);
  for (const f of ["clippy.toml.md", "docs/clippy.md", "compiler/mente-ast/Cargo.toml"]) assert.doesNotMatch(f, LINT_SETTINGS, f);
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
  // The workflow's triggers and its token's permissions too: what starts a pinned job, and with what
  // rights, is part of it (the review of 41a54b6: an `issue_comment:` trigger and `contents: write`
  // added to probe.yml left every pin as it was).
  return digest(JSON.stringify(canon({ on: triggers(doc), permissions: doc.permissions ?? null, env: doc.env ?? null, defaults: doc.defaults ?? null, job })));
}

/// What the tests run under, pinned whole: the scripts the testing jobs run, by their digests, and
/// the testing jobs themselves, every step's name, action, inputs, environment and script. A
/// change to any of them changes this file too, where review sees that a gate changed; the
/// failure names the digest to write here.
const PINNED_FILES = new Map([
  [".github/scripts/limits.sh", "ff5fb294d3365b4c5240a4b9f86042d3c9d2767e0c6e5e1fa0adceea359fdee9"],
  [".github/scripts/memory-guard.sh", "0702e624fc6a5d34a84ba1df7d0d3052957636ac1388fd464f9ff5c482416107"],
  [".github/scripts/job-limits.ps1", "739fb90a85d859f593080cd32e4f818270afcc8cdb3f296a11caa58510accce4"],
  [".github/scripts/install-nextest.sh", "e031943ffadbe14c7ef5dfbbb249b8e4741f0afe7af824c6fb9a3ff2b7923d55"],
  [".github/scripts/test-list.mjs", "6aa8a0c532adf0a5ccef1e65b9a4a0f05090929db3be40d667d9ebefc828df09"],
  [".github/scripts/lib/test-list.mjs", "9aab6d21844be4b964bcfa9e19257c4b2c61ce3cf40f03a0319b1a759bc05b6b"],
  [".github/scripts/tools.sh", "8567604ff84d47d657cdd04d1fe55dc8ff5a2ae238f02773c6fa87e31298fe18"],
  [".github/scripts/test-sandbox.sh", "02a8f1312275c5146ddf7950f9d933dab304c5b4470de0ffef792240ffa522d1"],
  [".github/scripts/test-sandbox-env.sh", "b271fb1d97601c393c01e6bb7c590a6dc47a78594acf41f353a0e74cb93b6d10"],
  [".github/scripts/overflow-proof.sh", "72b3760078555145fd95e4124bd0ee8248024819c7dff84b11ef02559646a70a"],
  // The policy's collector, its run tests and the fixtures they run instead of a text handed to bash
  // (policy-runtime-judge): run by the Documentation job, pinned like every script it runs.
  [".github/scripts/policy-collect.mjs", "f87f035aefe245cfea8b3f713d2d44fb5a73f748455ebec847cd980b7721a327"],
  [".github/scripts/policy-run.test.mjs", "be6b14c485b3c531af99e5be06448c455da9b7dcc902c70155ee3c39013a8867"],
  [".github/scripts/fixtures/limits-flags.sh", "6ece2ca2338651617b501bea5c40bf666e497f49d962881c426e6db081c44e09"],
  [".github/scripts/fixtures/memory-guard-rounds.sh", "29cf23124f683670ac11dd3c5cf0bea04f670f077734fa51ff0c4b83aa62b364"],
  [".github/scripts/fixtures/sandbox-env.sh", "e600a29b7080e262ab08f4b7d0f552c193b035bc4db737c950a737d526540c0e"],
  [".github/scripts/fixtures/source-limits.sh", "46892d8cfb2903335e9c6fb689159bba7820e2116a3823161c0e67aa33407988"],
  [".github/scripts/fixtures/timeout.sh", "0697de49d46e74f75710b105bb997630d8e3d0c2c8bd7dfdaca7775c63a5affc"],
  // The node scripts the pinned jobs run, and the modules they import: the build-time check, the
  // Mutants floor, the watch, the actions' pins and the YAML reader the policy reads every
  // workflow with.
  [".github/scripts/build-time-code.mjs", "874280f8818d786a0c6f0bca3a60ced4ae397c0f13169a1244303d4a436487ec"],
  [".github/scripts/lib/build-time-code.mjs", "99b866cb2014b1a97c770819e4707fa06ef203da6bb7e2efc7c6cc3b65517b6a"],
  [".github/scripts/mutants-floor.mjs", "620e5a9329fbddfa0837fad07b7c7c2e918d14bd45b0ba2c80f177edbe722b18"],
  [".github/scripts/lib/mutants-floor.mjs", "20e30ecfcbb91626c8f3ed22f6709d52afc7ca34cf7dd5c6433728a09108babb"],
  [".github/scripts/watch.mjs", "55168acde1c9d16a823db62389483640e24e745dfd39ad6e984cf88c3200a7d3"],
  [".github/scripts/lib/watch.mjs", "895bedf2e57eb8dd3fc327a33d34e2adedf2770df18de599fae304a6819fe502"],
  [".github/scripts/action-pins.mjs", "83f1208532ba0bc01bc19a0546fd49b93520d464c22a3dc3a41f65cfce4db458"],
  [".github/scripts/lib/action-pins.mjs", "f08a9cec14bea0adb5a077a2dc33a52b668868bcd6cc15de2691c57ad8e837dc"],
  [".github/scripts/yaml.test.mjs", "4dc4626a1c6ecb74f5c82f82d3994e6d477d30a7ac63b5e25b2b702665f9eab3"],
  [".github/scripts/lib/yaml.mjs", "ee62bc138d9f19d920de7c2224bdd22b09b821369f40bec8c528b5c54fca3dce"],
  [".github/scripts/lib/commits.mjs", "bd79c94ae1cc142d48e066b22ea559a2f6bbc20c5af877de56fc1b9d9d382a90"],
  [".github/scripts/commits.mjs", "33c4156781145f9c0a0e127fb99c304a85e1ed508e1321c4e56a3992ce6d4322"],
  // Every file on the API key's path and on the lock's, which run beside the key or as root: a line
  // added anywhere in one, before the key's line or after it, is a change to a gate review sees
  // (the re-check of 4ab15da: `set -x`, or the key written to a file, passed every rule of
  // start-broker.sh's shape).
  ["benchmark/harness/start-broker.sh", "9d3255c058d22c79fb2816413eb066d761ccd022a13eebe96c78f1946055853b"],
  ["benchmark/harness/stop-broker.sh", "1f4ec139da512a920c4eeafad893e451c0629c93ea19bd740ba4b6fc5b092ffa"],
  ["benchmark/harness/lock-sudo.sh", "c6562d1115b5561722fc3657860b057cdbf75190c491b46e19b1cf4a6d132cdc"],
  ["benchmark/harness/sandbox-root.sh", "2298f2a5e113cd18c796a1359c54ffb0cd755b783ae0d056d662e06eaf24fbc6"],
  ["benchmark/harness/fetch-held-out.sh", "c16dff5677b17c9d6e9090cf35f93dc9f58782a182b2e417f1d23482fdce9792"],
  // The broker's own module, which start-broker.sh names through the workspace's path.
  ["benchmark/harness/broker.ts", "20ad249db858d152ef201a89e96ba41035d619918764d2b8a6a8b2aa6db8df7e"],
  // Every file a secret job runs before its secret steps, with every module those import: none of
  // them can leave the secret steps an environment, a program first on PATH or a library to preload
  // without an edit review sees (secretPrefixFiles; the re-check of c55cf8f). And the pins of
  // toolchains.json, which choose what install-toolchains.sh installs and runs outside the sandbox.
  ["benchmark/harness/install-toolchains.sh", "db0719972981e0e212a3052c4439239852a75da432a9556f5c1200d4e57ac9f7"],
  ["benchmark/harness/toolchains.json", "cd656ee22820f70bb218457f5941066bde4d3ae758355251830646bcaf4034e8"],
  ["benchmark/harness/sandbox.sh", "e07589a6e45ed458002d77165e37866ac9987098ae705f5a9405ca672317c218"],
  ["benchmark/harness/sandbox-check.ts", "982132df137c367d307fdc3a7d45761b45bd9a214860cf63262f5752d9114a54"],
  ["benchmark/harness/build.ts", "b7cbef2b51fc6c7574dedc96435c2b26e74ced96c20ad0ed189ccb285ba5cf54"],
  ["benchmark/harness/languages.ts", "c20a2cc3728849d6262d207194cc64a28efed6be7421a71da191602e8d668200"],
  ["benchmark/harness/tasks.ts", "b9386ec2e03484721e3e05feb0b86c1ba35710c883f418473e188fe7045b2240"],
  ["benchmark/own.mjs", "ec189cfb8e26ba2394424f1ed7922323786abde7c85c09b0f98eefca08f49330"],
  ["benchmark/check.mjs", "36868660c86c3cea1d663ad9b64b374a5f29581279e21ca452f5e2d5e6f20303"],
]);
/// Every job whose code of the change runs in the sandbox on Linux is pinned whole: the testing
/// jobs, and the jobs that run the release tests through cargo alone. A rule that read their
/// steps line by line and no further let through a runner unset after its source, a check inside
/// `if false`, and cargo's shorter spellings (`cargo t`).
const SANDBOXED_JOBS = ["ci.yml#rust", "mutants.yml#in-diff", "mutants-full.yml#full", "ci.yml#execution-paths", "generated.yml#paths"];
/// The jobs pinned whole: the sandboxed ones, and the Documentation job, which runs the policy and
/// the build-time check before anything else of the change, so that a step put before them, an
/// input of its checkout, or a variable of the job is a change to a gate review sees.
const PINNED_JOBS = new Map([
  ["ci.yml#rust", "43563468f8a2f3871922acb9eaf4531e1cdc097c64f7215166ae890791587819"],
  ["mutants.yml#in-diff", "6b54eaf7d853b04bf84a06306bee5d6d0195bf17477bd7e2c6be4ceeaf9526ad"],
  ["mutants-full.yml#full", "ef3c64177ebd2171cd1fccad046dd3e796dfc9d4294101359fbd0329afa46c88"],
  ["ci.yml#execution-paths", "ffb9b1d7eb4ab57c72f6cb75d3ab6c181f52c749b3a9e6e078ab1475405099cb"],
  ["generated.yml#paths", "e5da13151fa0b007ca0e8f1f226cc44edff46c76b7800d0e77951aae7d4050a4"],
  ["ci.yml#docs", "9c0ef19d2e7875847856480e78182e4fe4c72b715bf88782376851bc4c47d408"],
  ["commits.yml#commits", "c8ea2207fa147bb5092056a0bc68be27770da23fd666855dcce1b79bb432e1bd"],
  ["benchmark.yml#bench", "5fe155416addc5eae5e51174842ae3bdf8e69ffdd1879ba691c482b60a03c971"],
  ["prelim.yml#prelim", "ac6eadf49ad0da4d645891fc15d16450c3dbd0905926e5c9c92b548ed7557bc7"],
  ["probe.yml#write", "0830c255940ceb132a7f00e0426efe6de5c4d1287edb351e8cffd93311c51b34"],
]);
/// The policy itself, which cannot hold its own digest: CODEOWNERS gives it to its owner, and every
/// other script a pinned job runs is pinned here.
const POLICY = ".github/scripts/policy.test.mjs";
/// The pinned jobs that run the workspace's tests through nextest, under the shared limits, and
/// record their tools: the required Rust jobs and the mutation runs.
const TESTING_JOBS = ["ci.yml#rust", "mutants.yml#in-diff", "mutants-full.yml#full"];
/// The step of the required Rust jobs that runs the tests: the shared limits first, then nextest's
/// own binary under the ci profile, with nothing after either that could filter, retry or forgive a
/// failure, then the check that the tests ran in the sandbox, and nothing on the step but its name
/// and this script.
const NEXTEST_STEP = ". .github/scripts/limits.sh\ncargo-nextest nextest run --locked --workspace --profile ci\n/usr/bin/bash .github/scripts/test-sandbox.sh ran\n/usr/bin/bash .github/scripts/tools.sh verify\n";

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
  // Every script a pinned job runs is pinned, and so is every script a pinned script runs in turn,
  // and every module a pinned module imports; the policy alone is held by its owner. A pinned job
  // that runs the change's code outside the sandbox (LEFT_OUTSIDE) is read up to its check that
  // nothing of the change can run where cargo builds: what it runs after that is the change's own.
  const pending = [];
  for (const key of PINNED_JOBS.keys()) {
    const [file, id] = key.split("#");
    const w = workflows().find((x) => x.file === file);
    const job = w.doc.jobs[id];
    const read = LEFT_OUTSIDE.has(key) ? { ...job, steps: steps(job).slice(0, steps(job).findIndex((s) => s.run === TOOLCHAIN_STEP) + 1) } : job;
    assert.ok(steps(read).length > 0, `${key} is read nowhere`);
    for (const f of repoPaths(scriptTexts(w.doc, read))) if (/^\.github\/scripts\/[\w/-]+\.(sh|ps1|mjs)$/.test(f) && f !== POLICY) pending.push([key, f]);
  }
  for (const f of PINNED_FILES.keys()) pending.push(["PINNED_FILES", f]);
  // In a secret job, every file its steps run up to the end of its secret steps, whatever its
  // directory, and every module those import: no earlier step can change the environment the
  // secret steps start in, through $GITHUB_ENV, $GITHUB_PATH or a program it leaves on the runner,
  // without an edit to a pinned file (the re-check of c55cf8f: install-toolchains.sh and
  // sandbox-check.ts, with its modules, were unpinned).
  const unpinnedBeforeSecrets = (pinned) =>
    [...SECRET_JOBS.keys()].flatMap((key) => {
      const [file, id] = key.split("#");
      const w = workflows().find((x) => x.file === file);
      return secretPrefixFiles(w.doc, w.doc.jobs[id]).filter((f) => !pinned.has(f)).map((f) => `${key} runs ${f} up to its secret steps, which PINNED_FILES does not hold`);
    });
  assert.deepEqual(unpinnedBeforeSecrets(PINNED_FILES), [], "files a secret job runs before its secrets are in reach, unpinned");
  for (const f of ["benchmark/harness/install-toolchains.sh", "benchmark/harness/sandbox-check.ts", "benchmark/harness/build.ts", "benchmark/check.mjs", "benchmark/harness/broker.ts"]) {
    const fewer = new Map(PINNED_FILES);
    fewer.delete(f);
    assert.ok(unpinnedBeforeSecrets(fewer).some((p) => p.includes(` runs ${f} `)), `${f}, run before the secrets, left out of the pins passes`);
  }
  // And an edit to one, as the review planted it, changes the digest the pin holds.
  const installer = "benchmark/harness/install-toolchains.sh";
  assert.notEqual(digest(`${read(installer)}echo "BASH_ENV=$RUNNER_TEMP/x.sh" >> "$GITHUB_ENV"\n`), PINNED_FILES.get(installer), "an edit to install-toolchains.sh keeps its pin");
  const seen = new Set();
  while (pending.length > 0) {
    const [from, f] = pending.pop();
    if (seen.has(f)) continue;
    seen.add(f);
    assert.ok(PINNED_FILES.has(f), `${from} runs ${f}, which PINNED_FILES does not hold`);
    for (const g of namedScripts(read(f))) pending.push([f, g]);
    // A pinned module's imports are pinned too.
    if (JS_MODULE.test(f)) for (const g of repoPaths([`node ${f}`])) if (g !== f) pending.push([f, g]);
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
  assert.equal(tests[0].run, NEXTEST_STEP, "the Rust jobs' tests do not run as NEXTEST_STEP in .github/scripts/policy.test.mjs reads them: the limits, then nextest under the ci profile; a change to the step changes NEXTEST_STEP too");
  assert.deepEqual(Object.keys(tests[0]).sort(), ["name", "run"], "the Rust jobs' step of the tests carries more than its name and script");
  // The mutation runs: their tests under the mutants profile, inside the same limits.
  for (const key of TESTING_JOBS.filter((k) => !k.startsWith("ci.yml#"))) {
    const [file, id] = key.split("#");
    const w = workflows().find((x) => x.file === file);
    assert.equal(w.doc.env?.NEXTEST_PROFILE, "mutants", `${file} runs its tests under another profile`);
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

/// The step that sets up the sandbox the code of a change runs in on Linux, and proves it on that
/// runner with the canary: exactly these two commands, and, in a testing job, which records its
/// tools, the check of the tools after the canary, whose probe makes and removes a file among them
/// as the runner's user.
const SANDBOX_SETUP = "/usr/bin/bash .github/scripts/test-sandbox.sh setup\n/usr/bin/bash .github/scripts/test-sandbox.sh check\n";
const VERIFY_TOOLS = "/usr/bin/bash .github/scripts/tools.sh verify\n";
/// What names the sandbox as cargo's runner in a step, and starts the count of what ran in it: the
/// testing jobs' limits, which source the second, or the second alone.
const SANDBOX_SOURCES = [". .github/scripts/limits.sh", ". .github/scripts/test-sandbox-env.sh"];
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
    assert.equal(sandbox.run, TESTING_JOBS.includes(key) ? SANDBOX_SETUP + VERIFY_TOOLS : SANDBOX_SETUP, `${where} does not set up and prove the sandbox as SANDBOX_SETUP in .github/scripts/policy.test.mjs reads it`);
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
    assert.ok(running >= 2, `${key}: the policy reads the steps that run code of the change`);
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
    ['cargo mutants --in-diff pr.diff --shard "$SHARD/4" --in-place || status=$?', true],
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
    ['cargo install cargo-mutants --locked --version "$CARGO_MUTANTS_VERSION"', false],
    ['cargo install cargo-mutants --locked --version "$CARGO_MUTANTS_VERSION"', false],
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

/// Every required job that runs on a Linux runner is in exactly one of three lists, each with its
/// reason: SANDBOXED_JOBS, whose code of the change runs in the sandbox; LEFT_OUTSIDE, which runs code
/// of the change outside it; and GATE_ONLY, which runs none but the gate's own scripts. A job in
/// none of them fails the check below, so that a job that leaves the sandboxed ones, or a required job
/// added later, is classified where review sees it, not by a pattern of what its steps run.
///
/// LEFT_OUTSIDE: in each, the change's own code produces the evidence its step reads, so a sandbox
/// would keep that code from the runner but not from its own result; what bringing each in would
/// take is in ROADMAP.md, M2, the guards of the audit.
const LEFT_OUTSIDE = new Map([
  ["ci.yml#bench", "its tests read the base branch's medians from a file of the runner's and write the job's summary"],
  ["ci.yml#tier1-latency", "its test writes the job's summary"],
  ["ci.yml#debuggers", "a script of the change drives the debugger over the binary it built"],
  ["ci.yml#docs", "it runs the change's own node code: the benchmark's checker and its tests, the harness's tests and the documentation tools"],
  ["smt.yml#z3", "it runs mn by its path, which cargo's runner never sees, and writes outside the workspace"],
  ["fuzz.yml#fuzz", "it runs mn by its path, which cargo's runner never sees, and writes outside the workspace"],
  ["mir-mutants.yml#gate", "it runs mn by its path, which cargo's runner never sees, and writes outside the workspace"],
  ["perf.yml#kernels", "it measures mn and its kernels through perf, with the base branch's compiler beside the change's"],
  ["harness.yml#fixtures", "its own sandbox confines agent code, not mn"],
]);
/// GATE_ONLY: the required jobs that run no code of the change but the gate's own scripts under
/// .github/. Each is pinned whole, with every script it runs: an allowlist of exact steps, where a
/// rule that read the steps for tools by their names let through every spelling it did not know
/// (`sh -c`, a quoted name, a path to the tool, node's child_process; the third review of 980965e).
const GATE_ONLY = new Map([["commits.yml#commits", "it runs .github/scripts/commits.mjs alone, over the commits' text and the pull request's"]]);
/// The labels a job may run on: each a literal, or a matrix value that is one.
const RUNNERS = new Set(["ubuntu-24.04", "ubuntu-24.04-arm", "windows-2025"]);
/// The runners a job names: its literal label, or each value its matrix gives the key `runs-on`
/// names; null when it names them otherwise (an expression, a list, a group).
export function runnersOf(job) {
  const on = job?.["runs-on"];
  if (typeof on !== "string") return null;
  const ref = /^\$\{\{\s*matrix\.([\w-]+)\s*\}\}$/.exec(on);
  if (!ref) return /\$\{\{/.test(on) ? null : [on];
  const matrix = job.strategy?.matrix;
  if (!isPlain(matrix)) return null;
  const values = Array.isArray(matrix[ref[1]]) ? matrix[ref[1]] : Array.isArray(matrix.include) ? matrix.include.map((c) => c?.[ref[1]]) : [];
  return values.length > 0 && values.every((v) => typeof v === "string") ? values : null;
}

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
    assert.equal(holding.length, 1, `${key}, a required job on a Linux runner, is in ${holding.length === 0 ? "none" : holding.join(" and ")} of SANDBOXED_JOBS, LEFT_OUTSIDE and GATE_ONLY in .github/scripts/policy.test.mjs, not exactly one: it says, with its reason, whether its code of the change runs in the sandbox, outside it, or not at all`);
    if (GATE_ONLY.has(key)) assert.ok(PINNED_JOBS.has(key), `${key}, a job of GATE_ONLY, is not pinned whole: its exact steps are what it may run`);
  }
  for (const [name, set] of lists) {
    for (const key of set) assert.ok(required.has(key) || (name === "SANDBOXED_JOBS" && key === "mutants-full.yml#full"), `${name} names ${key}, which is no required job`);
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

check("no required job restores a cache, and no job caches nextest", () => {
  // Another job of the same pull request, pinned or not, can save any entry under any key in the
  // pull request's scope, and a later run restores it.
  for (const { file, id, job } of requiredJobs()) {
    for (const s of steps(job)) assert.doesNotMatch(String(s.uses ?? ""), /^actions\/cache(\/restore)?@/, `${stepKey(file, id, s)}: a required job restores a cache another job could have written`);
  }
  // setup-node restores a cache of its own once package.json names a package manager, unless told
  // not to (the third review of 980965e): every setup-node says no, and names no cache.
  for (const { file, id, job } of allJobs()) {
    for (const s of steps(job).filter((x) => /^actions\/setup-node@/.test(String(x.uses ?? "")))) {
      assert.ok(isPlain(s.with) && s.with["package-manager-cache"] === false && !("cache" in s.with) && !("cache-dependency-path" in s.with), `${stepKey(file, id, s)}: setup-node may restore a cache another job could have written; give it package-manager-cache: false and no cache`);
    }
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
/// The dynamic loader's (LD_AUDIT, LD_PRELOAD, GLIBC_TUNABLES, GCONV_PATH), git's
/// (GIT_CONFIG_COUNT and its pairs run a command on checkout or diff), OpenSSL's (a configuration
/// loads a module), those that send rustup's download elsewhere (a proxy, with a certificate of
/// the change's) and those that move where a tool reads its settings (HOME, XDG_*, TMPDIR) belong
/// here too.
const TOOL_ENV =
  /^(CARGO_|NEXTEST_|RUSTC|RUSTDOC|RUSTUP_|RUSTFLAGS$|RUSTFMT|CLIPPY_|BASH_ENV$|BASH_FUNC_|ENV$|PATH$|SHELLOPTS$|BASHOPTS$|NODE_|NPM_CONFIG_|LD_|DYLD_|GLIBC_TUNABLES$|GCONV_|LOCPATH$|MALLOC_|GIT_|HOME$|XDG_|TMPDIR$|OPENSSL_|SSL_CERT_|CURL_CA_BUNDLE$|HTTPS?_PROXY$|ALL_PROXY$|NO_PROXY$)/i;
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

/// The variables of TOOL_ENV a job sets, in the workflow, the job or a step, that TOOL_ENV_ALLOWED
/// does not hold for it.
function toolEnvProblems(key, doc, job) {
  const found = [];
  for (const env of [doc.env, job.env, ...steps(job).map((s) => s.env)].filter(Boolean)) {
    for (const [name, value] of Object.entries(env)) {
      if (!TOOL_ENV.test(name)) continue;
      const held = TOOL_ENV_ALLOWED.get(name);
      const allowed = held !== undefined && (held.value === null || held.value === String(value)) && (held.jobs === null || held.jobs.includes(key));
      if (!allowed) found.push(`${key} sets ${name}=${value}, which changes how cargo, rustc, nextest, the shell, git, node or the loader run what the job checks; TOOL_ENV_ALLOWED names the jobs that may set it, and its value`);
    }
  }
  return found;
}

check("no required or pinned job sets a variable that changes how its tools run", () => {
  const keys = new Set([...requiredJobs().map((j) => `${j.file}#${j.id}`), ...PINNED_JOBS.keys()]);
  for (const { file, doc, id, job } of allJobs().filter((j) => keys.has(`${j.file}#${j.id}`))) assert.deepEqual(toolEnvProblems(`${file}#${id}`, doc, job), []);
  for (const name of ["BASH_ENV", "npm_config_script_shell", "NODE_OPTIONS", "PATH", "Path", "cargo_target_x86_64_unknown_linux_gnu_runner", "RUSTC_WRAPPER", "RUSTUP_TOOLCHAIN"]) assert.match(name, TOOL_ENV, name);
  // The plants of the second review of 4e896ca: a variable of the Documentation job's.
  for (const name of ["CLIPPY_CONF_DIR", "RUSTFMT", "LD_AUDIT", "LD_PRELOAD", "GIT_CONFIG_COUNT", "GIT_CONFIG_KEY_0", "GIT_EXEC_PATH", "GLIBC_TUNABLES", "GCONV_PATH", "HOME", "XDG_CONFIG_HOME", "TMPDIR", "NODE_EXTRA_CA_CERTS", "BASH_FUNC_cargo%%", "OPENSSL_CONF", "SSL_CERT_FILE", "https_proxy", "ALL_PROXY"]) assert.match(name, TOOL_ENV, name);
  for (const name of ["MENTE_GEN_PROGRAMS", "GITHUB_TOKEN", "ACTIONLINT_VERSION"]) assert.doesNotMatch(name, TOOL_ENV, name);
});

/// A manifest's `build =` that names a script, the key bare or quoted, and any mention of a
/// procedural macro, `proc-macro = true` or a `crate-type` of one, comments aside. This reads the
/// listed members' text, and so sees the plain spellings at once, with no toolchain. It does not
/// hold: a key written with a TOML escape reads as another key here and as `build` to cargo, and a
/// package only a path names is no listed member. What holds is cargo's own reading, in the check
/// after this one.
const buildScriptKey = /^\s*["']?build["']?\s*=(?!\s*false\b)/m;
const procMacro = /proc[-_]macro(?![\w-])/;
/// A doc attribute in any spelling, plain, spaced or under a cfg_attr.
const docAttribute = /#!?\[\s*(doc\s*=|cfg_attr\b[^\]]*\bdoc\s*=)/;

check("no workspace member runs code at build time", () => {
  for (const [name, { dir }] of workspace()) {
    const manifest = read(`${dir}/Cargo.toml`);
    const has = [];
    if (tracked().has(`${dir}/build.rs`)) has.push("a build.rs");
    if (buildScriptKey.test(manifest)) has.push("a build script `build =` names");
    if (procMacro.test(manifest.replace(/#.*$/gm, ""))) has.push("a procedural macro");
    if (has.length > 0) assert.ok(BUILD_TIME_CODE.has(name), `${name} has ${has.join(" and ")}, code that runs at build time, before the tests' limits and with the job's rights: BUILD_TIME_CODE in .github/scripts/lib/build-time-code.mjs names the members allowed one`);
  }
  for (const m of ['build = "x.rs"', '"build" = "x.rs"', "'build' = 'x.rs'"]) assert.match(m, buildScriptKey, m);
  assert.doesNotMatch("build = false", buildScriptKey);
  for (const m of ["proc-macro = true", 'crate-type = ["proc-macro"]', "proc_macro = true"]) assert.match(m, procMacro, m);
  for (const m of ['proc-macro2 = "1"', "proc_macro2 = { workspace = true }"]) assert.doesNotMatch(m, procMacro, m);
});

check("as cargo reads the manifests and the lock, no package of the workspace runs code at build time and every other crate is approved", () => {
  // Where the reading runs, and what may come before it, is the rule of the check after this one,
  // the same for every job that carries it, the Documentation job's too.
  // What the reading refuses, on what cargo reports: the plants of the review of pull request
  // 137, each of which the reader of the manifests' text lets through.
  const member = (name, dir, kinds = ["lib"]) => ({ name, version: "0.1.0", source: null, manifest_path: path.join("/w", dir, "Cargo.toml"), targets: kinds.map((k) => ({ kind: [k] })) });
  const crate = (name, source = CRATES_IO, kinds = ["lib"]) => ({ name, version: "1.0.0", source, manifest_path: `/home/.cargo/registry/src/x/${name}-1.0.0/Cargo.toml`, targets: kinds.map((k) => ({ kind: [k] })) });
  const lockOf = (packages) => packages.map((p) => ({ name: p.name, version: p.version, source: p.source }));
  const found = (packages, listed = ["compiler/a", "compiler/b"]) => metadataProblems({ workspace_root: path.join("/w"), packages }, { listed, lock: lockOf(packages) });
  const own = [member("mente-a", "compiler/a"), member("mente-b", "compiler/b", ["bin"])];
  assert.deepEqual(found([...own, crate("serde_derive", CRATES_IO, ["proc-macro"]), crate("libc", CRATES_IO, ["lib", "custom-build"])]), [], "an approved crate's own build script or macro is refused");
  assert.match(found([member("mente-a", "compiler/a", ["lib", "custom-build"]), own[1]])[0], /mente-a \(compiler\/a\) has a build script/);
  assert.match(found([member("mente-a", "compiler/a", ["proc-macro"]), own[1]])[0], /mente-a \(compiler\/a\) has a procedural macro/);
  const unlisted = found([...own, member("helper", "compiler/b/helper", ["lib", "custom-build"])]);
  assert.equal(unlisted.length, 2, unlisted.join("\n"));
  assert.match(unlisted[0], /cargo builds helper from compiler\/b\/helper, which the members of Cargo\.toml do not list/);
  assert.match(found([...own, crate("forked", "git+https://example.invalid/x#abc")])[0], /forked comes from git\+https/);
  assert.match(found(own, ["compiler/a", "compiler/b", "compiler/c"])[0], /lists the member compiler\/c, which cargo does not read/);
  assert.match(metadataProblems({ workspace_root: path.join("/w"), packages: [...own, crate("unlocked")] }, { listed: ["compiler/a", "compiler/b"], lock: lockOf(own) })[0], /cargo would build unlocked 1\.0\.0, which Cargo\.lock does not hold/);
  // The lock, read as the lines cargo writes: a name written with an escape, a source that is not
  // crates.io and a table cargo does not write stop cargo from being run on it at all.
  const approved = approvedCrates("# c\ntop: top\nleaf: top\n");
  assert.deepEqual([...approved], ["top", "leaf"]);
  assert.throws(() => approvedCrates("top\n"), /not "<crate>: <dependency>"/);
  const lock = (extra) => `# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n\n[[package]]\nname = "mente-a"\nversion = "0.1.0"\ndependencies = [\n "top",\n "leaf 2.0.0",\n "leaf 2.0.0 (registry+https://github.com/rust-lang/crates.io-index)",\n]\n\n[[package]]\nname = "top"\nversion = "1.0.0"\nsource = "${CRATES_IO}"\nchecksum = "${"a".repeat(64)}"\n${extra}`;
  assert.deepEqual(lockProblems(lock(""), approved), { unreadable: [], unapproved: [] });
  assert.equal(lockPackages(lock("")).packages.length, 2);
  const other = lockProblems(lock(`\n[[package]]\nname = "other"\nversion = "1.0.0"\nsource = "${CRATES_IO}"\n`), approved);
  assert.equal(other.unreadable.length, 0);
  assert.match(other.unapproved[0], /Cargo\.lock holds other, which \.config\/approved-crates\.txt does not list/);
  for (const [extra, why] of [
    ['\n[[package]]\nname = "le\\u0061f"\nversion = "2.0.0"\n', /not a line cargo writes/],
    ['\n[[package]]\n"name" = "leaf"\nversion = "2.0.0"\n', /not a line cargo writes/],
    ['\n[[package]]\nname = "leaf"\nversion = "2.0.0"\nsource = "git+https://example.invalid/leaf#abc"\n', /leaf comes from git\+https/],
    ['\n[[package]]\nname = "leaf"\nversion = "2.0.0"\nsource = "registry+https://example.invalid/index"\n', /not crates\.io/],
    ['\n[[patch.unused]]\nname = "leaf"\nversion = "2.0.0"\n', /not a line cargo writes/],
    ['\n[metadata]\n"checksum leaf" = "x"\n', /not a line cargo writes/],
    ['\n[[package]]\nname = "leaf"\nname = "top"\nversion = "2.0.0"\n', /not a line cargo writes/],
    ['\n[[package]]\nversion = "2.0.0"\n', /without its name/],
  ]) {
    const got = lockProblems(lock(extra), approved).unreadable;
    assert.ok(got.length > 0 && why.test(got.join("\n")), `the lock's reader lets through, or misnames: ${JSON.stringify(extra)} -> ${got.join(" | ")}`);
  }
  // The repository's own lock and members read clean, and its approved list reads as a list.
  assert.deepEqual(lockProblems(read("Cargo.lock"), approvedCrates(read(".config/approved-crates.txt"))), { unreadable: [], unapproved: [] });
  assert.deepEqual(listedMembers(read("Cargo.toml")), [...workspace().values()].map((c) => c.dir));
  assert.throws(() => listedMembers('[workspace]\nmembers = ["compiler/*"]\n'), /is a glob/);
  // The CLI itself, with a stand-in for cargo: it runs cargo only on a lock it can read, and
  // reports what cargo read.
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "build-time-"));
  try {
    fs.mkdirSync(path.join(dir, ".config"));
    fs.writeFileSync(path.join(dir, ".config", "approved-crates.txt"), "top: top\n");
    fs.writeFileSync(path.join(dir, "Cargo.toml"), '[workspace]\nmembers = ["compiler/a"]\n');
    const report = (packages) => () => JSON.stringify({ workspace_root: dir, packages });
    const a = (kinds) => ({ name: "mente-a", version: "0.1.0", source: null, manifest_path: path.join(dir, "compiler", "a", "Cargo.toml"), targets: kinds.map((k) => ({ kind: [k] })) });
    // The toolchain's check runs first, in a tree of its own: a cargo home with nothing in it, and a
    // walk up that stops at this directory, so that a configuration of the machine running the
    // policy does not count.
    fs.mkdirSync(path.join(dir, "home"));
    const toolchain = { cargoHome: path.join(dir, "home"), tracked: [], top: dir };
    const run = (metadata, options = {}) => buildTimeCode(dir, metadata, { toolchain, ...options });
    fs.writeFileSync(path.join(dir, "rust-toolchain.toml"), '# c\n[toolchain]\nchannel = "1.97.1"\ncomponents = ["clippy", "rustfmt"]\nprofile = "minimal"\n');
    fs.writeFileSync(path.join(dir, "Cargo.lock"), lock(""));
    const clean = run(report([a(["lib"]), crate("top")]));
    assert.deepEqual(clean.problems, []);
    assert.match(clean.summary, /cargo reads 1 packages of the workspace.* and 1 crates of crates\.io/);
    assert.equal(run(report([a(["lib", "custom-build"]), crate("top")])).problems.length, 1);
    assert.deepEqual(run(() => assert.fail("cargo runs before the toolchain is installed"), { beforeToolchain: true }).problems, []);
    fs.writeFileSync(path.join(dir, "Cargo.lock"), lock('\n[[package]]\nname = "leaf"\nversion = "2.0.0"\nsource = "git+https://example.invalid/leaf#abc"\n'));
    const unread = run(() => assert.fail("cargo runs on a lock that names another source"));
    assert.match(unread.problems[0], /leaf comes from git\+https/);
    fs.writeFileSync(path.join(dir, "Cargo.lock"), lock(""));
    // What the toolchain's check refuses before rustup or cargo runs, cargo itself never asked.
    const valid = fs.readFileSync(path.join(dir, "rust-toolchain.toml"), "utf8");
    const refused = (why) => {
      const r = run(() => assert.fail(`cargo runs though ${why}`), {});
      assert.ok(r.problems.length > 0, `the toolchain's check lets through ${why}`);
    };
    for (const [text, why] of [
      [`${valid}path = "/tmp/toolchain"\n`, "a path toolchain"],
      [`${valid}targets = ["wasm32-unknown-unknown"]\n`, "another target"],
      [valid.replace('"1.97.1"', '"stable"'), "a channel that is no release"],
      [valid.replace('profile = "minimal"', 'profile = "default"'), "another profile"],
      [`${valid}channel = "1.97.1"\n`, "a channel twice"],
      [valid.replace("[toolchain]\n", ""), "no table"],
      [valid.replace("[toolchain]", '"toolchain"'), "a table spelled otherwise"],
    ]) {
      fs.writeFileSync(path.join(dir, "rust-toolchain.toml"), text);
      refused(why);
    }
    fs.writeFileSync(path.join(dir, "rust-toolchain.toml"), valid);
    for (const [file, why] of [
      [path.join(dir, "rust-toolchain"), "a rust-toolchain file beside rust-toolchain.toml"],
      [path.join(dir, ".cargo", "config.toml"), "a cargo configuration file in the workspace"],
      [path.join(dir, "home", "config"), "a cargo configuration file in CARGO_HOME"],
    ]) {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, "x\n");
      refused(why);
      fs.rmSync(file);
    }
    const nested = path.join(dir, "inner");
    fs.mkdirSync(path.join(dir, ".cargo"), { recursive: true });
    fs.writeFileSync(path.join(dir, ".cargo", "config"), "x\n");
    for (const f of ["rust-toolchain.toml", ".config/approved-crates.txt", "Cargo.toml", "Cargo.lock"]) {
      fs.mkdirSync(path.dirname(path.join(nested, f)), { recursive: true });
      fs.copyFileSync(path.join(dir, f), path.join(nested, f));
    }
    assert.ok(toolchainProblems(nested, { ...toolchain }).some((p) => /\.cargo[\\/]config: a cargo configuration file/.test(p)), "the toolchain's check does not walk up to a configuration above the workspace");
    fs.rmSync(path.join(dir, ".cargo"), { recursive: true });
    assert.ok(toolchainProblems(dir, { ...toolchain, tracked: ["compiler/x/.cargo/config.toml"] }).length > 0, "the toolchain's check lets through a tracked configuration deeper in the tree");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

/// The toolchain step of every job that builds: before rustup or cargo reads anything of the
/// workspace, the check that rust-toolchain.toml names a release alone and no cargo configuration
/// is there; then the toolchain; then cargo's reading of the manifests and the lock. So a change
/// whose code would run where cargo builds fails each such job before any of it is built.
const TOOLCHAIN_STEP = "node .github/scripts/build-time-code.mjs --before-toolchain\nrustup toolchain install\nnode .github/scripts/build-time-code.mjs\n";

/// What may come before TOOLCHAIN_STEP, each step exactly, with no key beyond those named: the
/// checkout, with no input but these; node, at the release the jobs use and with no cache of its
/// own, which setup-node would restore from the pull request's own scope once package.json named
/// a package manager; the watch; and, each in the one job named, the gate's own steps that read
/// nothing of the change's but the scripts the policy pins.
const CHECKOUT_INPUTS = [{ "persist-credentials": false }, { "fetch-depth": 0, "persist-credentials": false }];
/// How the broker, the one process that holds the API key, is started, and the one Node every job
/// sets up, whose permission model the broker's flags rely on (benchmark/harness/permission.json):
/// read here, and by the harness's test of the escapes.
const PERMISSION = JSON.parse(read("benchmark/harness/permission.json"));
const NODE_INPUTS = { "node-version": PERMISSION.node, "package-manager-cache": false };
/// The Documentation job's first run: the collector, then the policy, its run tests and the YAML
/// reader's (policy-runtime-judge), as BEFORE_TOOLCHAIN reads a step's script, trimmed.
const POLICY_RUN = "node .github/scripts/policy-collect.mjs\nnode --test .github/scripts/policy.test.mjs .github/scripts/policy-run.test.mjs .github/scripts/yaml.test.mjs";
const BEFORE_TOOLCHAIN = new Map([
  [POLICY_RUN, { job: "ci.yml#docs", why: "the policy, first in the Documentation job: its collector, then its tests" }],
  ["node .github/scripts/action-pins.mjs", { job: "ci.yml#docs", why: "the check of the actions' pins against their releases" }],
  ['echo "RUSTUP_HOME=$RUNNER_TEMP/rustup" >> "$GITHUB_ENV"', { job: "harness.yml#fixtures", why: "the harness's toolchain, put outside the home its sandbox hides" }],
]);
const isPlain = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
const sameKeys = (o, keys) => isPlain(o) && JSON.stringify(Object.keys(o).sort()) === JSON.stringify([...keys].sort());
/// Whether a step holds these keys and no other, the watch's condition aside.
const stepKeysAre = (s, keys) => sameKeys(Object.fromEntries(Object.entries(s).filter(([k]) => k !== "if")), keys);

/// The one rule for every job that carries TOOLCHAIN_STEP: up to and with it, every step is one
/// the rule names, exactly, with no key beyond its own (no working-directory, env, shell,
/// continue-on-error or timeout), and the job and its workflow set no defaults but the shell and
/// run in no container of their own. A variable of the job or the workflow is the rule of
/// TOOL_ENV; a pinned job is held whole besides. The plants of the second review of 4e896ca, P1 to
/// P6, are this rule's fixtures.
export function beforeToolchainProblems(file, id, doc, job) {
  const key = `${file}#${id}`;
  const found = [];
  const all = Array.isArray(job?.steps) ? job.steps : [];
  const at = all.findIndex((s) => s.run === TOOLCHAIN_STEP);
  if (at < 0) return [`${key}: none of its steps is TOOLCHAIN_STEP in .github/scripts/policy.test.mjs, the check that nothing of the change can run where cargo builds, before rustup installs the toolchain`];
  if (all.filter((s) => s.run === TOOLCHAIN_STEP).length !== 1) found.push(`${key} installs the toolchain more than once`);
  for (const [where, defaults] of [
    ["the workflow", doc?.defaults],
    ["the job", job.defaults],
  ]) {
    if (defaults === undefined) continue;
    if (!(sameKeys(defaults, ["run"]) && sameKeys(defaults.run, ["shell"]) && SHELLS.has(defaults.run.shell))) found.push(`${key}: ${where} sets defaults beyond the shell: ${JSON.stringify(defaults)}`);
  }
  for (const k of ["container", "services"]) if (k in job) found.push(`${key} runs in or beside a ${k} of its own`);
  const watched = all.slice(0, at).some((s) => s.id === "watch");
  const condition = (s) => s.if === undefined || (watched && WATCH_FORMS.includes(normalize(s.if)));
  let checkouts = 0;
  for (const [k, s] of all.slice(0, at + 1).entries()) {
    const where = `${stepKey(file, id, s)}, up to the check that nothing of the change can run where cargo builds,`;
    if (!condition(s)) found.push(`${where} runs under a condition other than the watch's: ${s.if}`);
    const keys = Object.keys(s).filter((x) => x !== "if");
    const uses = String(s.uses ?? "");
    if (k === at) {
      if (!stepKeysAre(s, ["name", "run"])) found.push(`${where} carries more than its name, its script and the watch's condition: ${keys.join(", ")}`);
    } else if (/^actions\/checkout@[0-9a-f]{40}$/.test(uses)) {
      checkouts++;
      if (k !== 0) found.push(`${where} checks out after another step`);
      if (!stepKeysAre(s, ["uses", "with"]) || !CHECKOUT_INPUTS.some((w) => sameKeys(s.with, Object.keys(w)) && Object.keys(w).every((x) => s.with[x] === w[x]))) {
        found.push(`${where} checks out with ${JSON.stringify(s.with ?? null)} or more keys than its inputs (${keys.join(", ")}), not one of CHECKOUT_INPUTS`);
      }
    } else if (/^actions\/setup-node@[0-9a-f]{40}$/.test(uses)) {
      if (!stepKeysAre(s, ["uses", "with"]) || JSON.stringify(s.with) !== JSON.stringify(NODE_INPUTS)) found.push(`${where} sets node up with ${JSON.stringify(s.with ?? null)} or more keys (${keys.join(", ")}), not NODE_INPUTS`);
    } else if (s.uses !== undefined) {
      found.push(`${where} runs an action other than the checkout and node: ${uses}`);
    } else if (s.id === "watch") {
      if (!stepKeysAre(s, ["name", "id", "run"]) || String(s.run).trim() !== `node .github/scripts/watch.mjs ${file}`) found.push(`${where} is not the watch exactly: ${JSON.stringify(s)}`);
    } else if (!stepKeysAre(s, ["name", "run"]) || BEFORE_TOOLCHAIN.get(String(s.run ?? "").trim())?.job !== key) {
      found.push(`${where} is none of the steps BEFORE_TOOLCHAIN names for this job, with its name and script alone: ${JSON.stringify(s)}`);
    }
  }
  if (checkouts !== 1) found.push(`${key} checks out ${checkouts} times before the check, not once`);
  return found;
}

check("every required or pinned job but those of GATE_ONLY and SECRET_JOBS checks, before anything of the workspace is built or installed, that nothing of the change can run where cargo builds", () => {
  // Every such job carries the check, whatever its steps run: a rule that asked which jobs build,
  // by the names of the tools in their scripts, missed a tool spelled otherwise (the third review
  // of 980965e). A job of GATE_ONLY is pinned whole instead.
  const keys = new Set([...requiredJobs().map(({ file, id }) => `${file}#${id}`), ...PINNED_JOBS.keys()]);
  let checked = 0;
  for (const { file, doc, id, job } of allJobs().filter((j) => keys.has(`${j.file}#${j.id}`))) {
    const key = `${file}#${id}`;
    const all = steps(job);
    // No step of a required or pinned job runs elsewhere than the workspace's root.
    for (const s of all) assert.ok(!("working-directory" in s), `${stepKey(file, id, s)} runs in a directory of its own`);
    if (GATE_ONLY.has(key) || SECRET_JOBS.has(key)) continue;
    checked++;
    assert.deepEqual(beforeToolchainProblems(file, id, doc, job), []);
    // After it, no step changes the toolchain, but in the base branch's own worktree, whose
    // rust-toolchain.toml is the base's (the performance jobs build the base's compiler there).
    const at = all.findIndex((s) => s.run === TOOLCHAIN_STEP);
    for (const s of all.slice(at + 1)) {
      for (const line of logicalLines(s.run ?? "").map((l) => l.trim())) {
        if (/^\(cd \.\.\/base && /.test(line)) continue;
        assert.doesNotMatch(line, /\brustup\s+(toolchain|default|override|component|target)\b/, `${stepKey(file, id, s)} changes the toolchain after the check: ${line}`);
      }
    }
  }
  assert.equal(checked, keys.size - GATE_ONLY.size - SECRET_JOBS.size, "a required or pinned job was left out of the check");
  // The plants of the second review of 4e896ca, on the Documentation job and on a Rust job: each
  // is refused by this rule, by TOOL_ENV or by the pin.
  const ci = workflows().find((w) => w.file === "ci.yml").doc;
  const clone = (v) => JSON.parse(JSON.stringify(v));
  const index = (job, test) => job.steps.findIndex(test);
  for (const id of ["docs", "rust"]) {
    const plant = (change) => {
      const doc = clone(ci);
      const job = doc.jobs[id];
      change(job, doc);
      return [...beforeToolchainProblems("ci.yml", id, doc, job), ...toolEnvProblems(`ci.yml#${id}`, doc, job), ...(jobDigest(doc, job) === PINNED_JOBS.get(`ci.yml#${id}`) ? [] : ["the pin"])];
    };
    const ruled = (change) => {
      const doc = clone(ci);
      change(doc.jobs[id], doc);
      return [...beforeToolchainProblems("ci.yml", id, doc, doc.jobs[id]), ...toolEnvProblems(`ci.yml#${id}`, doc, doc.jobs[id])];
    };
    const toolchain = (job) => job.steps[index(job, (s) => s.run === TOOLCHAIN_STEP)];
    for (const [name, change] of [
      ["P1, working-directory on the toolchain step", (job) => (toolchain(job)["working-directory"] = "compiler")],
      ["P2, a variable on the toolchain step", (job) => (toolchain(job).env = { MENTE_X: "1" })],
      ["P2, a shell on the toolchain step", (job) => (toolchain(job).shell = "sh")],
      ["P3, setup-node with an input more", (job) => job.steps.splice(1, id === "docs" ? 1 : 0, { uses: `actions/setup-node@${"0".repeat(40)}`, with: { ...NODE_INPUTS, cache: "npm" } })],
      ["P3, the checkout with a path", (job) => (job.steps[0].with = { ...job.steps[0].with, path: "elsewhere" })],
      ["P3, the checkout of another repository", (job) => (job.steps[0].with = { ...job.steps[0].with, repository: "someone/else" })],
      ["P4, a job default of a working directory", (job) => (job.defaults = { run: { shell: "bash", "working-directory": "compiler" } })],
      ["P4, a workflow default of a working directory", (job, doc) => (doc.defaults = { run: { "working-directory": "compiler" } })],
      ["P5, the checkout of main", (job) => (job.steps[0].with = { ...job.steps[0].with, ref: "main" })],
      ["P6, LD_AUDIT on the job", (job) => (job.env = { ...job.env, LD_AUDIT: "./x.so" })],
      ["P6, GIT_CONFIG_COUNT on the workflow", (job, doc) => (doc.env = { ...doc.env, GIT_CONFIG_COUNT: "1", GIT_CONFIG_KEY_0: "core.fsmonitor", GIT_CONFIG_VALUE_0: "./x" })],
      ["a step of the change's before the check", (job) => job.steps.splice(1, 0, { name: "x", run: "bash ./x.sh" })],
      ["a step allowed only in another job, before the check", (job) => job.steps.splice(1, 0, { name: "x", run: 'echo "RUSTUP_HOME=$RUNNER_TEMP/rustup" >> "$GITHUB_ENV"' })],
      ["setup-node with its own cache left on", (job) => job.steps.splice(1, id === "docs" ? 1 : 0, { uses: `actions/setup-node@${"0".repeat(40)}`, with: { "node-version": PERMISSION.node } })],
      ["npm ci before the check", (job) => job.steps.splice(1, 0, { name: "x", run: "npm ci --ignore-scripts" })],
      ["an action before the check", (job) => job.steps.splice(1, 0, { uses: `someone/action@${"0".repeat(40)}` })],
      ["a second checkout", (job) => job.steps.splice(1, 0, clone(job.steps[0]))],
      ["the toolchain step under if: false", (job) => (toolchain(job).if = "false")],
      ["the toolchain step allowed to fail", (job) => (toolchain(job)["continue-on-error"] = true)],
      ["a container", (job) => (job.container = "someone/image")],
      ["the policy step with a variable", (job) => {
        const s = job.steps.find((x) => BEFORE_TOOLCHAIN.has(String(x.run ?? "").trim()));
        if (s) s.env = { MENTE_X: "1" };
        else toolchain(job).env = { MENTE_X: "1" };
      }],
    ]) {
      assert.ok(ruled(change).length > 0, `ci.yml#${id}: the rule lets through ${name}`);
      assert.ok(plant(change).length > 0, `ci.yml#${id}: nothing refuses ${name}`);
    }
    assert.deepEqual(ruled(() => {}), [], `ci.yml#${id}: the rule refuses the job as it is`);
  }
});

/// The code blocks of a library's doc comments rustdoc lists as doctests, as `file:line`: a fenced
/// block whose info string names no other language than Rust, `ignore` included, which rustdoc
/// lists though it does not run it, and an indented block (four spaces after a blank doc line),
/// which rustdoc reads as Rust. A block another language names, `text` say, is no doctest.
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
      if (rust) found.push(`${file}:${k + 1}`);
    } else if (blank && /^ {4,}\S/.test(body)) {
      found.push(`${file}:${k + 1}`);
    }
    blank = body.trim() === "";
  }
  return found;
}

check("no library's doc comments hold an example rustdoc would compile", () => {
  // nextest runs no doctests, and `cargo test --doc` has no limit per test: an example belongs in a
  // test, or in a block marked `text`.
  // The required Rust jobs list the doctests rustdoc finds, and fail on any; this reads the
  // sources first, so that an example fails here too, and names where it is.
  const found = [];
  for (const [, { dir }] of workspace()) {
    for (const f of filesUnder(dir).filter((x) => x.endsWith(".rs"))) {
      const text = read(f);
      if (!tracked().has(`${dir}/src/lib.rs`) || !f.startsWith(`${dir}/src/`) || f === `${dir}/src/main.rs` || f.startsWith(`${dir}/src/bin/`)) continue;
      // A module read from another path, which these lines would never read, is refused in every
      // crate by the check of `#[path]` above.
      found.push(...doctestBlocks(f, text));
      if (docAttribute.test(text)) found.push(`${f}: a doc attribute, whose text the policy does not read`);
      if (/^\s*\/\*[*!](?![*/])/m.test(text)) found.push(`${f}: a block doc comment, which the policy does not read`);
    }
  }
  assert.deepEqual(found, [], "doc comments hold examples rustdoc lists as doctests: mark each `text`, or make it a test");
  for (const a of ['#[doc = "x"]', '#![doc = include_str!("a.md")]', '#[ doc = "x"]', '#[cfg_attr(all(), doc = "x")]']) assert.match(a, docAttribute, a);
  assert.doesNotMatch("#[doc(hidden)]", docAttribute);
  const blocks = (text) => doctestBlocks("f.rs", text).map((at) => Number(at.split(":")[1]));
  assert.deepEqual(blocks("/// a\n/// ```\n/// x\n/// ```\n"), [2]);
  assert.deepEqual(blocks("//! ```rust,no_run\n//! x\n//! ```\n"), [1]);
  assert.deepEqual(blocks("/// ```should_panic\n/// ```\n/// ```edition2021\n/// ```\n"), [1, 3]);
  assert.deepEqual(blocks("/// ```text\n/// x\n/// ```\n/// ```mn\n/// ```\n/// ~~~sh\n/// ~~~\n"), []);
  assert.deepEqual(blocks("/// ```ignore\n/// x\n/// ```\n"), [1], "rustdoc lists an `ignore` block");
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
  assert.deepEqual(overrideTests(read(".config/nextest.toml")).sort(), [...SLOW_TESTS, ...ALL_CORES_TESTS].map(([p, t]) => `${p} ${t}`).sort(), "lib/test-list.mjs reads the overrides' tests as the policy does");
  // The doctests' list, as `cargo test --doc -- --list` prints it.
  const none = "   Doc-tests mente_ast\n0 tests, 0 benchmarks\n   Doc-tests mente_mir\n0 tests, 0 benchmarks\n";
  assert.deepEqual(doctestProblems(none), []);
  // As the jobs print it, under CARGO_TERM_COLOR=always: the header coloured.
  assert.deepEqual(doctestProblems("\x1b[1m\x1b[32m   Doc-tests\x1b[0m mente_ast\n0 tests, 0 benchmarks\n"), []);
  assert.equal(doctestProblems(`${none}   Doc-tests mente_sema\ncompiler/mente-sema/src/lib.rs - f (line 3): test\n1 test, 0 benchmarks\n`).length, 1, "a library with a doctest");
  assert.equal(doctestProblems("   Doc-tests mente_ast\n   Doc-tests mente_mir\n0 tests, 0 benchmarks\n").length, 1, "a library without its count");
  assert.equal(doctestProblems("   Compiling mente-ast\n").length, 1, "no library at all");
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
/// Whether a value, a string or anything in a step or a job, holds a secret: an expression that
/// names `secrets` in any form (`secrets.X`, `secrets['X']`, `toJSON(secrets)`), but the test
/// whether one is empty, which yields no secret. The final review of PR 137 found a secret named
/// without a dot read as none.
/// Every string of a value, keys and values, as written: a JSON string of it would turn a tab or a
/// line break inside an expression into `\t` or `\n`, past `\b` (the review of 35bbe0f).
const stringsOf = (v) => (typeof v === "string" ? [v] : Array.isArray(v) ? v.flatMap(stringsOf) : v !== null && typeof v === "object" ? [...Object.keys(v), ...Object.values(v).flatMap(stringsOf)] : []);
/// GitHub reads a context's name in any case: `SECRETS.X` is a secret too.
/// It reads from a string's first `${{` to its end, not to the first `}}`, which GitHub skips
/// inside a quoted string of the expression (the review of b2be87b: `'}}'` ended it early); the
/// one exception is a string that is the emptiness test and nothing more.
const holdsSecret = (value) =>
  stringsOf(value).some((s) => {
    const k = s.indexOf("${{");
    if (k < 0) return false;
    const rest = s.slice(k);
    return /\bsecrets\b/i.test(rest) && !/^\$\{\{\s*secrets\.\w+\s*[!=]=\s*''\s*\}\}\s*$/i.test(rest);
  });

/// A JavaScript or TypeScript module as tokens, its comments dropped: identifiers, string literals
/// with their values, template literals (an interpolation's code read as code), regular expression
/// literals, numbers and single punctuators, each with its line. A rule over a module reads these,
/// not its text, so that a comment, a string or a regular expression holds no import and no name
/// of it, and spacing changes nothing (the review of b2be87b: `import/**/(x)`, `import"x"` and
/// `from"x"` past the regular expressions before). A literal or a comment that does not close is
/// marked `bad`, so that a misread `/` hides nothing from a rule that refuses those.
export function jsTokens(text) {
  const out = [];
  const n = text.length;
  const starts = [0];
  for (let i = 0; i < n; i++) if (text[i] === "\n") starts.push(i + 1);
  const lineOf = (at) => {
    let lo = 0;
    let hi = starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (starts[mid] <= at) lo = mid;
      else hi = mid - 1;
    }
    return lo + 1;
  };
  const push = (t, v, at, extra = {}) => out.push({ t, v, line: lineOf(at), ...extra });
  let k = text.startsWith("#!") ? Math.max(text.indexOf("\n"), 0) : 0;
  let depth = 0;
  const resume = [];
  const regexAllowed = () => {
    const p = out[out.length - 1];
    if (!p) return true;
    if (p.t === "punct") return !/^[)\]}]$/.test(p.v);
    if (p.t === "tpl") return Boolean(p.open);
    if (p.t === "id") return /^(return|typeof|instanceof|in|of|new|delete|void|throw|case|do|else|yield|await)$/.test(p.v);
    return false;
  };
  const template = (at) => {
    let v = "";
    while (k < n) {
      const c = text[k];
      if (c === "\\") {
        v += text.slice(k, k + 2);
        k += 2;
      } else if (c === "`") {
        k++;
        push("tpl", v, at);
        return;
      } else if (c === "$" && text[k + 1] === "{") {
        k += 2;
        push("tpl", v, at, { open: true });
        resume.push(depth);
        depth = 0;
        return;
      } else {
        v += c;
        k++;
      }
    }
    push("tpl", v, at, { bad: true });
  };
  while (k < n) {
    const c = text[k];
    const at = k;
    if (c === " " || c === "\t" || c === "\n" || c === "\r") {
      k++;
    } else if (c === "/" && text[k + 1] === "/") {
      const e = text.indexOf("\n", k);
      k = e < 0 ? n : e;
    } else if (c === "/" && text[k + 1] === "*") {
      const e = text.indexOf("*/", k + 2);
      if (e < 0) push("punct", "/*", at, { bad: true });
      k = e < 0 ? n : e + 2;
    } else if (c === '"' || c === "'") {
      let j = k + 1;
      let v = "";
      while (j < n && text[j] !== c && text[j] !== "\n") {
        if (text[j] === "\\") {
          v += text.slice(j, j + 2);
          j += 2;
        } else v += text[j++];
      }
      push("str", unescapeJs(v), at, text[j] === c ? {} : { bad: true });
      k = j + 1;
    } else if (c === "`") {
      k++;
      template(at);
    } else if (c === "}" && depth === 0 && resume.length > 0) {
      depth = resume.pop();
      k++;
      template(at);
    } else if (c === "/" && regexAllowed()) {
      let j = k + 1;
      let inClass = false;
      while (j < n && text[j] !== "\n" && (text[j] !== "/" || inClass)) {
        if (text[j] === "\\") j++;
        else if (text[j] === "[") inClass = true;
        else if (text[j] === "]") inClass = false;
        j++;
      }
      const closed = text[j] === "/";
      j++;
      while (j < n && /[a-z]/i.test(text[j])) j++;
      push("re", text.slice(k, j), at, closed ? {} : { bad: true });
      k = j;
    } else if (/[A-Za-z_$]/.test(c)) {
      let j = k + 1;
      while (j < n && /[\w$]/.test(text[j])) j++;
      push("id", text.slice(k, j), at);
      k = j;
    } else if (/[0-9]/.test(c)) {
      let j = k + 1;
      while (j < n && /[\w.]/.test(text[j])) j++;
      push("num", text.slice(k, j), at);
      k = j;
    } else {
      if (c === "{") depth++;
      if (c === "}") depth--;
      push("punct", c, at);
      k++;
    }
  }
  return out;
}

/// A string literal's value from its source between the quotes: the escapes that spell a character
/// read as that character, so that `"ev\x61l"` is "eval".
function unescapeJs(raw) {
  const simple = { n: "\n", t: "\t", r: "\r", b: "\b", f: "\f", v: "\v", 0: "\0" };
  return raw.replace(/\\(u\{([0-9a-fA-F]+)\}|u([0-9a-fA-F]{4})|x([0-9a-fA-F]{2})|\n|.)/g, (_, e, cp, u, x) => {
    const hex = cp ?? u ?? x;
    if (hex !== undefined) return String.fromCodePoint(parseInt(hex, 16));
    if (e === "\n") return "";
    return simple[e] ?? e;
  });
}

/// The specifiers a module's tokens import: each static import, `export ... from` and literal
/// `import("...")`, with its line and whether it is dynamic; and the lines of every `import` the
/// reading cannot follow (a computed `import(`, a form it does not know). The closure of what a
/// job runs follows these, and the rule for a secret job's modules refuses the rest.
export function jsImports(toks) {
  const specifiers = [];
  const unreadable = [];
  const at = (j) => toks[j] ?? { t: "end", v: "" };
  for (let i = 0; i < toks.length; i++) {
    const t = toks[i];
    if (t.t !== "id" || at(i - 1).v === ".") continue;
    if (t.v === "import") {
      if (at(i + 1).v === "." && at(i + 2).v === "meta") continue;
      if (at(i + 1).t === "str") {
        specifiers.push({ v: at(i + 1).v, line: t.line, dynamic: false });
        continue;
      }
      if (at(i + 1).v === "(") {
        if (at(i + 2).t === "str" && at(i + 3).v === ")") specifiers.push({ v: at(i + 2).v, line: t.line, dynamic: true });
        else unreadable.push(t.line);
        continue;
      }
      // `import` [`type`] (name | `{` names `}` | `*` `as` name) [`,` (`{` names `}` | `*` `as` name)] `from` "specifier"
      let j = i + 1;
      if (at(j).v === "type" && at(j + 1).v !== "from" && at(j + 1).v !== ",") j++;
      for (let part = 0; part < 2; part++) {
        if (at(j).t === "id" && at(j).v !== "from") j++;
        else if (at(j).v === "*" && at(j + 1).v === "as" && at(j + 2).t === "id") j += 3;
        else if (at(j).v === "{") {
          j++;
          while (at(j).t === "id" || at(j).v === ",") j++;
          if (at(j).v !== "}") break;
          j++;
        } else break;
        if (at(j).v !== ",") break;
        j++;
      }
      if (at(j).v === "from" && at(j + 1).t === "str") specifiers.push({ v: at(j + 1).v, line: t.line, dynamic: false });
      else unreadable.push(t.line);
    } else if (t.v === "export") {
      // `export` [`type`] (`*` [`as` name] | `{` names `}`) `from` "specifier"
      let j = i + 1;
      if (at(j).v === "type") j++;
      if (at(j).v === "*") {
        j++;
        if (at(j).v === "as") j += 2;
      } else if (at(j).v === "{") {
        j++;
        while (at(j).t === "id" || at(j).v === ",") j++;
        // A list the reading does not understand, a name written as a string among them, fails
        // closed (the final pass of ad38011: `export { "q" as w } from "..."` read as nothing).
        if (at(j).v !== "}") {
          unreadable.push(t.line);
          continue;
        }
        j++;
      } else continue;
      if (at(j).v === "from" && at(j + 1).t === "str") specifiers.push({ v: at(j + 1).v, line: t.line, dynamic: false });
      else if (at(j).v === "from") unreadable.push(t.line);
    }
  }
  return { specifiers, unreadable };
}

/// The one computed load a module of a secret job may make, by its exact line, once, with the files
/// it loads, which must be owned: the corpus checker's, which loads reference.mjs from the corpus
/// and from the held-out set, whose own the private repository holds at the commit corpus.lock
/// names (the review of 35bbe0f: check.mjs loaded an unowned reference.mjs beside the held-out set
/// by a computed path).
const COMPUTED_LOADS = new Map([
  ["benchmark/check.mjs", { line: '  const reference = (await import(pathToFileURL(path.join(dir, "reference.mjs")).href)).tasks;', loads: ["benchmark/reference.mjs"] }],
]);
/// The modules of node a module of a secret job may import, each by `node:`; child_process only in
/// SPAWNERS. None of them loads or runs code: no module, vm, worker_threads or child_process here.
const NODE_MODULES = new Set(["node:fs", "node:path", "node:url", "node:crypto", "node:net"]);
/// The modules of a secret job that may start a process: each by its exact import of
/// child_process, and every use of what it imports a call on a line named or counted, so that what
/// they start is read where they start it: the harness's runProcess and the preliminary run's
/// mnSync, one call each, and the corpus checker's git, whose two calls are named whole, arguments
/// and all (the review of b2be87b: `git -c core.pager=...` would have run anything). No other module
/// of those jobs imports child_process (the review of 35bbe0f: `execSync as sh` in tasks.ts ran mn
/// beside the key).
const SPAWNERS = new Map([
  ["benchmark/harness/build.ts", { line: 'import { spawn } from "node:child_process";', word: "spawn", calls: 1 }],
  ["benchmark/harness/prelim.ts", { line: 'import { spawnSync } from "node:child_process";', word: "spawnSync", calls: 1 }],
  [
    "benchmark/check.mjs",
    {
      line: 'import { execFileSync } from "node:child_process";',
      word: "execFileSync",
      calls: ['function git(dir, ...args) { return execFileSync(process.platform === "win32" ? "git" : "/usr/bin/git", ["-C", dir, ...args], { encoding: "utf8" }).trim(); }'],
      through: { word: "git", calls: ['    const dirty = git(heldOutDir, "status", "--porcelain");', '    const commit = git(heldOutDir, "rev-parse", "HEAD");'] },
    },
  ],
]);
/// The names a module of a secret job may not use, each a way to run or load code that a reading
/// of its imports does not see, or a way to reach one: eval and Function, the loaders and their
/// hooks, the native loaders, workers, WebAssembly, and the objects that reach any global by a
/// computed name.
const REFUSED_NAMES = new Set([
  "eval",
  "Function",
  "AsyncFunction",
  "require",
  "createRequire",
  "register",
  "_load",
  "getBuiltinModule",
  "binding",
  "_linkedBinding",
  "dlopen",
  "Worker",
  "WebAssembly",
  "globalThis",
  "global",
  "Reflect",
  "Proxy",
  "getPrototypeOf",
  "setPrototypeOf",
  "prototype",
  "defineProperty",
  "defineProperties",
  "__proto__",
  "arguments",
  "module",
  "exports",
]);
/// What node runs as a module, by a file's name: what the closure of a job reads for imports.
const JS_MODULE = /\.(mjs|cjs|js|ts|mts|cts|jsx|tsx)$/;
/// The modules a secret job may run or import: ES modules alone, which the root package.json's
/// `"type": "module"` makes of `.js` and `.ts` as of `.mjs`.
const SECRET_MODULE = /\.(mjs|js|ts)$/;
/// The members of `process` a module of a secret job may name, each only as `process.<member>`:
/// its arguments, its environment, its end and its platform. The rest of `process` loads or runs
/// code a reading of the module does not see (`getBuiltinModule`, `binding`, `dlopen`, `execve`,
/// the review of 41a54b6), and `process` by any other spelling, an alias or a computed member,
/// would reach them.
const PROCESS_MEMBERS = new Set(["argv", "env", "exit", "platform"]);
/// The one module through which the modules of a secret job read or write a member by a computed
/// key, by its text: it reads only an object's own property and defines only an own property, so
/// that no key, however it is made, reaches what a prototype holds (`constructor`, and through it
/// Function). Every other computed member access in those modules is refused.
const OWN_FILE = "benchmark/own.mjs";
const OWN_TEXT = [
  "// The one way a module of a job that holds a secret reads or writes a member by a computed key",
  "// (docs/design-review.md, untrusted-test-isolation). It reads only an object's own property and",
  "// defines only an own property, so that no key, however it is made, reaches what a prototype",
  "// holds: `constructor`, and through it Function, above all. The key becomes a property key once,",
  "// and that one is both checked and used: a key whose `toString` answers otherwise the second time",
  "// would otherwise pass the check as one name and read another. The workflow policy refuses every",
  "// other computed member access in those modules, and holds this file's text.",
  "",
  "/** A key as the property key it names, made once. */",
  "const keyOf = (key) => (typeof key === \"symbol\" ? key : String(key));",
  "",
  "/** The own property `key` of `object`, or undefined: never one its prototypes hold. */",
  "export function own(object, key) {",
  "  const k = keyOf(key);",
  "  return object !== null && object !== undefined && Object.hasOwn(Object(object), k) ? object[k] : undefined;",
  "}",
  "",
  "/** Defines `key` as an own property of `object`, whatever its prototypes hold, and returns `value`. */",
  "export function setOwn(object, key, value) {",
  "  Object.defineProperty(object, keyOf(key), { value, writable: true, enumerable: true, configurable: true });",
  "  return value;",
  "}",
  "",
].join("\n");
/// The keywords after which `[` opens an array or a pattern, not a computed member.
const KEYWORDS = /^(return|typeof|instanceof|in|of|new|delete|void|throw|case|do|else|yield|await|extends|const|let|var)$/;
/// Whether the `[` at `i` reads a member by a computed key, or names one: after a name, a
/// literal, a call, a parenthesized expression, another member, `?.`, a block's or an object's
/// `}`, or an expression's non-null `!`; after a keyword that is a property's name (`Array.of[k]`,
/// `m.delete[k]`) or a method's (`o.catch(x)[k]`); and as the computed key of an object, a
/// pattern or a class (`{ [k]: v }`, `const { [k]: f } = x`, `[k]() {}`), which a `]` followed by
/// `:` or `(` shows. Not after a keyword, an operator or `(`, where it opens an array or a
/// pattern, nor right after the condition of `if`, `while`, `for`, `switch`, `catch` or
/// `with`, where it starts a statement. A reading of tokens, this narrows what can be spelled and
/// stays a reading aid: no reading decides what arbitrary code does.
export function computedAccessAt(toks, i) {
  const t = toks[i];
  if (!(t?.t === "punct" && t.v === "[") || toks[i + 1]?.v === "]") return false;
  const p = toks[i - 1];
  if (!p) return false;
  const afterDot = (j) => toks[j - 1]?.t === "punct" && toks[j - 1].v === ".";
  const endsExpression = (x, j) => x && ((x.t === "id" && (!KEYWORDS.test(x.v) || afterDot(j))) || x.t === "str" || x.t === "num" || (x.t === "tpl" && !x.open) || (x.t === "punct" && /^[)\]]$/.test(x.v)));
  // A computed key: `[...]` followed by `:` (a property or a pattern's) or `(` (a method's).
  let close = i + 1;
  for (let depth = 0; close < toks.length; close++) {
    if (toks[close].t === "punct" && /^[[({]$/.test(toks[close].v)) depth++;
    if (toks[close].t === "punct" && /^[\])}]$/.test(toks[close].v) && depth-- === 0) break;
  }
  if (toks[close + 1]?.t === "punct" && /^[:(]$/.test(toks[close + 1].v) && p.t === "punct" && /^[{,;}]$/.test(p.v)) return true;
  if (p.t === "id") return !KEYWORDS.test(p.v) || afterDot(i - 1);
  if (p.t === "str" || p.t === "num" || p.t === "re" || (p.t === "tpl" && !p.open)) return true;
  if (p.t !== "punct") return false;
  if (p.v === "]" || p.v === "}") return true;
  if (p.v === "." && toks[i - 2]?.v === "?") return true;
  if (p.v === "!") return endsExpression(toks[i - 2], i - 2);
  if (p.v === ")") {
    let depth = 0;
    for (let j = i - 1; j >= 0; j--) {
      if (toks[j].t === "punct" && toks[j].v === ")") depth++;
      if (toks[j].t === "punct" && toks[j].v === "(" && --depth === 0) {
        const q = toks[j - 1];
        const control = q?.t === "id" && /^(if|while|for|switch|catch|with)$/.test(q.v) && !afterDot(j - 1);
        return !control;
      }
    }
  }
  return false;
}

/// A module of a secret job, read as an allowlist over its tokens. Its imports are static, each of
/// a relative literal that is a file of the repository or of NODE_MODULES (child_process only in
/// SPAWNERS, used as named there); it uses none of REFUSED_NAMES, as a name or a string,
/// `constructor` only to define a class's, `process` only as `process.<member>` of
/// PROCESS_MEMBERS, and no computed member access, which only OWN_FILE makes; every literal and
/// comment closes, and nothing outside printable ASCII, tab and LF, and no backslash outside a
/// literal (an escaped name), appears. The one computed load allowed is COMPUTED_LOADS's line.
/// Returns the problems and the files its relative imports name. These rules narrow what such a
/// module can spell; they cannot decide what arbitrary code does, so the code owner's review holds
/// what they do not list (docs/design-review.md, trust-from-structure), and each node of a secret
/// job runs with code generation from strings disallowed, which no spelling gets past.
export function readSecretModule(file, text) {
  // OWN_FILE is held by its whole text, which no rule below need read.
  if (file === OWN_FILE) return { problems: text === OWN_TEXT ? [] : [`${file}: its text is not the one the policy holds (OWN_TEXT)`], imports: [] };
  const problems = [];
  // An ES module alone: `.cjs` and `.cts` are CommonJS, whose `arguments`, `require` and
  // `module` load any module by a computed name (the re-check of da58409).
  if (!SECRET_MODULE.test(file)) problems.push(`${file}: no .mjs, .js or .ts module, which the package's "type": "module" makes ES modules`);
  const imports = [];
  const lines = text.split("\n");
  const named = COMPUTED_LOADS.get(file);
  if (named && lines.filter((l) => l === named.line).length !== 1) problems.push(`${file}: its named load is not the one line \`${named.line.trim()}\`, once`);
  if (/[^\t\n -~]/.test(text)) problems.push(`${file}: a character outside printable ASCII, tab and LF`);
  // The named load's line is read as blank, keeping every other line's number.
  const toks = jsTokens(named ? lines.map((l) => (l === named.line ? "" : l)).join("\n") : text);
  const where = (t) => `${file}:${t.line}`;
  const prior = (i) => toks[i - 1] ?? { t: "start", v: "" };
  for (let i = 0; i < toks.length; i++) {
    const t = toks[i];
    if (t.bad) problems.push(`${where(t)}: a literal or a comment that does not close`);
    if (t.t === "punct" && t.v === "\\") problems.push(`${where(t)}: a backslash outside a literal, as in an escaped name`);
    if ((t.t === "id" || t.t === "str" || t.t === "tpl") && REFUSED_NAMES.has(t.v)) problems.push(`${where(t)}: ${t.v}, a way to run or load code that a reading of its imports does not see`);
    if ((t.t === "str" || t.t === "tpl") && t.v === "constructor") problems.push(`${where(t)}: "constructor", the way to Function`);
    if (t.t === "id" && t.v === "constructor" && !(toks[i + 1]?.v === "(" && /^[{};]$/.test(prior(i).v) && prior(i).t === "punct")) problems.push(`${where(t)}: constructor, other than defining a class's, the way to Function`);
    if (t.t === "id" && t.v === "process" && prior(i).v !== "." && !(toks[i + 1]?.v === "." && toks[i + 2]?.t === "id" && PROCESS_MEMBERS.has(toks[i + 2].v))) {
      problems.push(`${where(t)}: process other than as process.${[...PROCESS_MEMBERS].join(", process.")}`);
    }
    // Every computed member access, whatever its key: OWN_FILE's `own` and `setOwn` read and
    // write only own properties (the review of 41a54b6: `(() => 0)["constr" + "uctor"]` and
    // `p["getBuilt" + "inModule"]` after `const p = process`).
    if (computedAccessAt(toks, i)) problems.push(`${where(t)}: a member read by a computed key, which only ${OWN_FILE}'s own() and setOwn() do`);
  }
  const read = jsImports(toks);
  for (const line of read.unreadable) problems.push(`${file}:${line}: an import the policy cannot read as a static import of a literal`);
  for (const s of read.specifiers) {
    if (s.dynamic) problems.push(`${file}:${s.line}: import("${s.v}"), a load at run time, not a static import`);
    else if (/^\.{1,2}\//.test(s.v)) {
      const target = path.posix.normalize(path.posix.join(path.posix.dirname(file), s.v));
      imports.push(target);
      if (!tracked().has(target)) problems.push(`${file}:${s.line}: imports ${s.v}, which is no file of the repository`);
      if (!SECRET_MODULE.test(target)) problems.push(`${file}:${s.line}: imports ${s.v}, which is no .mjs, .js or .ts module`);
    } else if (s.v === "node:child_process") {
      if (!SPAWNERS.has(file)) problems.push(`${file}:${s.line}: imports child_process, which only the modules of SPAWNERS may`);
    } else if (!NODE_MODULES.has(s.v)) problems.push(`${file}:${s.line}: imports ${s.v}, which is neither a relative file nor one of NODE_MODULES`);
  }
  const spawner = SPAWNERS.get(file);
  // No module but SPAWNERS names child_process at all, in any form the reading does not take for
  // an import of it.
  // Read as the rest is read, decoded: the text, and every literal's value, an escape in it spelled
  // out (the review of 75e5eeb: a second import of `"node:child_process"` counted as none in
  // the text).
  const naming = toks.filter((x) => (x.t === "str" || x.t === "tpl") && x.v.includes("child_process"));
  if (!spawner && (text.includes("child_process") || naming.length > 0)) problems.push(`${file}: names child_process, which only the modules of SPAWNERS may`);
  if (spawner) {
    const lineNo = (line) => lines.flatMap((l, k) => (l === line ? [k + 1] : []));
    const importAt = lineNo(spawner.line);
    const imported = read.specifiers.filter((s) => s.v === "node:child_process");
    const alone = importAt.length === 1 && imported.length === 1 && imported[0].line === importAt[0] && naming.length === 1 && (text.match(/child_process/g) ?? []).length === 1;
    if (!alone) problems.push(`${file}: its import of child_process is not \`${spawner.line}\` alone, once, decoded`);
    // Every use of a name is the import (or the function's definition), and each call where it is
    // named or as many as counted, each a call.
    const uses = (word, definedAt, calls) => {
      const found = toks.filter((t) => t.t === "id" && t.v === word && prior(toks.indexOf(t)).v !== ".");
      const rest = found.filter((t) => !definedAt.includes(t.line));
      const callAt = typeof calls === "number" ? null : calls.map(lineNo);
      const isCall = (t) => toks[toks.indexOf(t) + 1]?.v === "(";
      const ok =
        found.length - rest.length === definedAt.length &&
        rest.every(isCall) &&
        (callAt === null ? rest.length === calls : callAt.every((at) => at.length === 1) && rest.length === callAt.length && rest.every((t) => callAt.some((at) => at[0] === t.line)));
      if (!ok) problems.push(`${file}: its uses of ${word} are not ${typeof calls === "number" ? `its import and ${calls} call` : `its import and the named calls: ${calls.map((l) => l.trim()).join(" ")}`}`);
    };
    uses(spawner.word, importAt, spawner.calls);
    if (spawner.through) uses(spawner.through.word, lineNo(spawner.calls[0]), spawner.through.calls);
  }
  return { problems, imports };
}
export const secretModuleProblems = (file, text) => readSecretModule(file, text).problems;

/// Every job that holds a secret, each with the job of its workflow that builds mn for it (or none)
/// and whether it runs agent code. Each names the environment `benchmark`, the one place the
/// secrets are, whose deployment branches are main alone: a dispatch on another ref, or the run of
/// a pull request, never reaches them, so what runs beside them is code the code owner has merged
/// (docs/design-review.md, trust-from-structure). Each is pinned whole, and every file it runs,
/// with every module those import, is the code owner's in CODEOWNERS; it builds nothing, since mn
/// comes from a job without the secret, which carries the build-time check first (the fourth
/// review of PR 137 found the build in the job that later held the key), and mn, which agents
/// write, runs only in the benchmark's own sandbox. The rules below are a reading aid for the code
/// owner, whose review holds what they do not list.
const SECRET_JOBS = new Map([
  ["benchmark.yml#bench", { builtBy: "build", agents: true }],
  ["prelim.yml#prelim", { builtBy: "build", agents: true }],
  ["probe.yml#write", { builtBy: null, agents: false }],
]);
/// The environment that holds the secrets.
const SECRET_ENVIRONMENT = "benchmark";
/// A flag as start-broker.sh writes it: the value of a flag that names the workspace, quoted.
const quoteFlag = (f) => (f.includes("=$") ? f.replace(/=(.*)$/, '="$1"') : f);
/// The shell of every step that holds a secret: env drops what bash reads before the step's first
/// line, BASH_ENV and ENV, which it runs, and SHELLOPTS and BASHOPTS, which can turn tracing on, from
/// an environment any earlier step could have written through $GITHUB_ENV; then bash, by its path,
/// reads no profile, runs privileged, which imports no exported function and reads none of those
/// variables either, and stops at the first failure (the re-check of c55cf8f: the runner's own bash
/// ran a BASH_ENV with the token present, and traced it; the final pass on 89c7691: an exported
/// `printf` printed the token, and the key). Every bash a secret step starts is privileged too.
///
/// What the shell cannot drop: LD_PRELOAD, LD_LIBRARY_PATH and LD_AUDIT load into /usr/bin/env
/// itself, before it runs. Those are held upstream, by what may write the environment of the secret
/// steps at all: the job's digest, the one write ENV_WRITES allows, and the pins of every file an
/// earlier step runs (secretPrefixFiles), which is why those pins matter.
export const SECRET_SHELL = "/usr/bin/env -u BASH_ENV -u ENV -u SHELLOPTS -u BASHOPTS /usr/bin/bash --noprofile --norc -p -eo pipefail {0}";
/// The step that takes root from the runner's user for the rest of a job, but for the sandbox's
/// root-owned half, before the sandbox runs anything and before the key is in reach: so that no
/// process of that user, outside the sandbox, reads the key out of the broker's memory or the
/// runner's (benchmark/harness/lock-sudo.sh; docs/design-review.md, untrusted-test-isolation).
export const LOCK_STEP = { name: "Take root from the runner's user, but for the sandbox", run: "bash benchmark/harness/lock-sudo.sh" };
/// The one step of a secret job that holds the API key: it starts the broker in the background
/// and ends once the broker listens (benchmark/harness/start-broker.sh, whose node line
/// BROKER_LINE holds).
export const BROKER_STEP = {
  name: "The broker, the one process that holds the API key",
  env: { ANTHROPIC_API_KEY: "${{ secrets.ANTHROPIC_API_KEY }}", BUDGET: "${{ inputs.budget }}" },
  shell: SECRET_SHELL,
  run: "/usr/bin/bash -p benchmark/harness/start-broker.sh",
};
/// The last step of a secret job: it stops the broker, whatever came before, and checks that root
/// is still out of the runner's user's reach.
export const STOP_STEP = { name: "Stop the broker", if: "always()", run: "bash benchmark/harness/stop-broker.sh\nbash benchmark/harness/lock-sudo.sh --check\n" };
/// The last step of a job that runs the sandbox and holds no secret: the same check.
export const LOCKED_STEP = { name: "Root is still out of the runner's user's reach", if: "!cancelled() && steps.watch.outputs.run != 'false'", run: "bash benchmark/harness/lock-sudo.sh --check" };
/// The line of start-broker.sh that starts the broker: the key on its standard input and in no
/// environment (`env -i`, which also drops NODE_OPTIONS, whose --allow-child-process would undo
/// the permission model), under permission.json's flags, each command by its path: env, and the
/// Node of permission.json's version that setup-node put in the runner's tool cache.
export const BROKER_LINE = `printf '%s\\n' "$ANTHROPIC_API_KEY" | /usr/bin/env -i PATH="$PATH" "$RUNNER_TOOL_CACHE/node/${PERMISSION.node}/x64/bin/node" ${PERMISSION.broker.flags.map(quoteFlag).join(" ")} "$GITHUB_WORKSPACE/${PERMISSION.broker.script}" --socket "$dir/socket" --budget "$BUDGET" > "$dir/log" 2>&1 &`;
/// The lines start-broker.sh holds before BROKER_LINE: no core file at any size, a limit the
/// runner's user cannot raise again, and a mask and a directory that keep the socket, the log and
/// the pid file the runner's user's alone.
const BROKER_SETUP = ['dir="$RUNNER_TEMP/broker"', "ulimit -c 0", "umask 0177", '/usr/bin/mkdir -m 0700 "$dir"'];
/// The one place each secret may stand in a secret job, everything else refused: the key in the
/// broker's step, the held-out set's token in the held-out step's.
const HELD_OUT_TOKEN = "${{ secrets.MENTE_BENCH_HELDOUT_TOKEN }}";
/// The one step of a secret job that holds the held-out set's token: it fetches the commit
/// corpus.lock names and checks it out, printing nothing of the held-out repository into the
/// public log (benchmark/harness/fetch-held-out.sh; run 3 of the plan, 2026-11-16, found
/// actions/checkout printing the commit's subject there).
export const HELD_OUT_STEP = {
  name: "Check out the held-out set",
  if: "env.HAS_HELD_OUT == 'true'",
  env: { HELD_OUT_TOKEN, HELD_OUT_COMMIT: "${{ steps.lock.outputs.commit }}" },
  shell: SECRET_SHELL,
  // As BROKER_LINE starts the broker: the token on the script's standard input, never on env's
  // command line, readable in /proc; every command by its path; and the script under env -i, with
  // PATH, a fresh HOME and the commit alone, so that SHELLOPTS, BASH_ENV or git's settings in the
  // job's environment reach none of it (the review of aafe59b: tracing printed the token before its
  // mask; the re-check of c55cf8f: a shim first on PATH received it).
  run: "home=\"$(/usr/bin/mktemp -d)\"\nprintf '%s\\n' \"$HELD_OUT_TOKEN\" | /usr/bin/env -i PATH=/usr/bin:/bin HOME=\"$home\" HELD_OUT_COMMIT=\"$HELD_OUT_COMMIT\" /usr/bin/bash --noprofile --norc -p benchmark/harness/fetch-held-out.sh\n",
};
/// The one write to the environment of later steps a secret job may make before its held-out step
/// and its broker's: a path of its own, which holds no secret and turns nothing on. Any other could
/// set what the shells of those steps run with (SHELLOPTS, BASH_ENV) or put a program first on PATH.
const ENV_WRITES = new Set(['echo "RUSTUP_HOME=$RUNNER_TEMP/rustup" >> "$GITHUB_ENV"']);
/// Whether a step writes the environment or the PATH of later steps other than by ENV_WRITES.
const writesEnv = (s) => /GITHUB_ENV|GITHUB_PATH/.test(String(s.run ?? "")) && !ENV_WRITES.has(String(s.run ?? "").trim());
/// What is wrong with the repository's paths for the held-out step: held-out-set, where it checks the
/// set out, tracked as a file, a link or a directory.
export const heldOutPathProblems = (paths) => [...paths].filter((f) => f === "held-out-set" || f.startsWith("held-out-set/")).map((f) => `${f}: a path of the repository where the held-out step checks the set out`);
/// The held-out repository, which no step checks out but HELD_OUT_STEP.
const HELD_OUT_REPOSITORY = "Menteion/mente-bench-heldout";

/// Whether a step is `want` exactly, but for the watch's condition, which a watched job's steps
/// may carry.
const isStep = (s, want) => {
  const { if: cond, ...rest } = s;
  const { if: wantCond, ...wantRest } = want;
  return JSON.stringify(rest) === JSON.stringify(wantRest) && (cond === wantCond || (wantCond === undefined && cond === "steps.watch.outputs.run != 'false'"));
};

/// Where a job's secret steps end: past its last step that is HELD_OUT_STEP or BROKER_STEP, or 0.
const secretStepsEnd = (all) => all.reduce((end, s, i) => (isStep(s, HELD_OUT_STEP) || isStep(s, BROKER_STEP) ? i + 1 : end), 0);
/// The files a secret job runs up to the end of its secret steps, the held-out step and the
/// broker's, with every module those import: the scripts and modules, not the data they read.
export function secretPrefixFiles(doc, job) {
  const prefix = { ...job, steps: steps(job).slice(0, secretStepsEnd(steps(job))) };
  const files = [...repoPaths(scriptTexts(doc, prefix))].filter((f) => (JS_MODULE.test(f) || /\.(sh|ps1)$/.test(f)) && f !== POLICY);
  // The broker, whose path start-broker.sh builds from the workspace's, which no reading of a path
  // follows.
  return prefix.steps.some((s) => isStep(s, BROKER_STEP)) ? [...files, PERMISSION.broker.script] : files;
}

/// What is wrong with start-broker.sh: BROKER_SETUP, then BROKER_LINE, each once, and no other node.
export function startBrokerProblems(text) {
  const found = [];
  const lines = text.split("\n");
  const at = (line) => lines.indexOf(line);
  for (const line of [...BROKER_SETUP, BROKER_LINE]) {
    if (lines.filter((l) => l === line).length !== 1) found.push(`start-broker.sh does not hold, once: ${line}`);
  }
  if (!BROKER_SETUP.every((line, k) => at(line) >= 0 && at(line) < at(BROKER_LINE) && (k === 0 || at(line) > at(BROKER_SETUP[k - 1])))) found.push("start-broker.sh does not set the core limit, the mask and the directory up, in that order, before it starts the broker");
  const nodes = lines.filter((l) => /(^|[\s;&|(\/])node"?(\s|$)/.test(l.replace(/#.*$/, "")));
  if (nodes.length !== 1) found.push(`start-broker.sh starts node ${nodes.length} times, not once`);
  // The key is named twice, in the check that it is set and in the line that hands it to the
  // broker, and the shell never traces what it runs.
  const named = lines.filter((l) => l.includes("ANTHROPIC_API_KEY") && !l.trimStart().startsWith("#"));
  if (named.length !== 2 || !named.includes(BROKER_LINE) || !named.some((l) => l.startsWith(': "${ANTHROPIC_API_KEY:?'))) found.push("start-broker.sh names the key beyond its check and the line that hands it to the broker");
  if (lines.some((l) => /^\s*set\s+(-\w*x|-o\s+xtrace)|BASH_XTRACEFD|\bxtrace\b/.test(l.replace(/#.*$/, "")))) found.push("start-broker.sh traces what it runs, the key among it");
  return found;
}

/// What is wrong with how a secret job holds its key: root taken from the runner's user before the
/// key is in reach (LOCK_STEP); one step holding the key, BROKER_STEP exactly; the one other step
/// that may hold a secret, the held-out checkout, holding its token as its one input secret and
/// keeping it out of .git/config; no secret wider than a step; the harness talking to the broker
/// after it starts, holding none; STOP_STEP last; and permission.json's Node, set up once.
export function brokerProblems(key, job, doc = {}) {
  const found = [];
  const all = steps(job);
  if (holdsSecret(doc.env) || holdsSecret({ ...job, steps: null }) || "secrets" in job) found.push(`${key} holds a secret wider than a step, which would reach every step beside the broker`);
  const locks = all.filter((s) => isStep(s, LOCK_STEP) && s.if === undefined);
  const brokers = all.filter((s) => isStep(s, BROKER_STEP) && s.if === undefined);
  if (brokers.length !== 1) found.push(`${key} does not start the broker once, as BROKER_STEP`);
  if (locks.length !== 1 || all.indexOf(locks[0]) > all.findIndex((s) => holdsSecret(s))) found.push(`${key} does not take root from the runner's user once, as LOCK_STEP, before any secret is in reach`);
  const heldOut = all.filter((s) => isStep(s, HELD_OUT_STEP));
  for (const s of all.filter((x) => holdsSecret(x) && !brokers.includes(x))) {
    if (!heldOut.includes(s)) found.push(`${key}, step "${s.name ?? s.uses}": holds a secret where none belongs (the key belongs in BROKER_STEP alone, ${HELD_OUT_TOKEN} in HELD_OUT_STEP alone)`);
  }
  if (heldOut.length > 1) found.push(`${key} checks the held-out set out more than once`);
  // Nothing before the held-out step or the broker's writes the environment of later steps but
  // ENV_WRITES (the review of aafe59b: SHELLOPTS=xtrace through $GITHUB_ENV traced the token); the
  // scripts those steps run are pinned (secretPrefixFiles).
  const lastHeld = secretStepsEnd(all);
  for (const s of all.slice(0, lastHeld)) if (writesEnv(s)) found.push(`${key}, step "${s.name ?? s.uses}": writes the environment or the PATH of later steps before the secrets are in reach`);
  // No step checks the held-out repository out but HELD_OUT_STEP: actions/checkout prints the
  // commit's subject into the log, and a token as its input reaches .git/config unless told not to.
  for (const s of all) if (/^actions\/checkout@/.test(String(s.uses ?? "")) && String(s.with?.repository ?? "").toLowerCase() === HELD_OUT_REPOSITORY.toLowerCase()) found.push(`${key}, step "${s.name ?? s.uses}": checks the held-out repository out through actions/checkout, which prints its commit's subject into the log`);
  const stops = all.filter((s) => isStep(s, STOP_STEP) && s.if === "always()");
  if (stops.length !== 1 || all.at(-1) !== stops[0]) found.push(`${key} does not stop the broker in its last step, as STOP_STEP`);
  const harness = all.filter((s) => /benchmark\/harness\/(bench|prelim|probe)\.ts/.test(String(s.run ?? "")));
  const talks = harness.length === 1 && String(harness[0].run).includes('--broker "$RUNNER_TEMP/broker/socket"') && all.indexOf(harness[0]) > all.indexOf(brokers[0]);
  if (!talks) found.push(`${key} does not run the harness once, after the broker starts, talking to it`);
  const node = all.filter((s) => /^actions\/setup-node@/.test(String(s.uses ?? "")));
  if (node.length !== 1 || String(node[0].with?.["node-version"]) !== PERMISSION.node) found.push(`${key} does not set up Node ${PERMISSION.node} once, whose permission model the broker's flags rely on`);
  return found;
}

/// What is wrong with a job that runs the benchmark's sandbox: root taken from the runner's user
/// (LOCK_STEP) before the first step that runs it, and checked out of reach in the job's last step
/// (STOP_STEP in a secret job, LOCKED_STEP in another).
export function sudoLockProblems(key, job) {
  const all = steps(job);
  const first = all.findIndex((s) => /"\$SANDBOX"|benchmark\/harness\/sandbox(-check)?\.(sh|ts)/.test(String(s.run ?? "")));
  if (first < 0) return [];
  const found = [];
  const locks = all.filter((s) => isStep(s, LOCK_STEP));
  if (locks.length !== 1 || all.indexOf(locks[0]) > first) found.push(`${key} runs the sandbox, but does not take root from the runner's user once, as LOCK_STEP, before it`);
  const last = all.at(-1);
  if (!last || !(isStep(last, STOP_STEP) || isStep(last, LOCKED_STEP))) found.push(`${key} runs the sandbox, but does not check in its last step that root is still out of reach`);
  return found;
}

/// What is wrong with the Node the jobs set up: each sets up permission.json's, one Node for the
/// broker, for the tests of its permission model and for every other job, read in one place.
export function nodeProblems(jobs) {
  const found = [];
  for (const { file, id, job } of jobs) {
    for (const s of steps(job).filter((x) => /^actions\/setup-node@/.test(String(x.uses ?? "")))) {
      if (String(s.with?.["node-version"]) !== PERMISSION.node) found.push(`${file}#${id} sets up Node ${s.with?.["node-version"]}, not ${PERMISSION.node}, whose permission model the broker's flags rely on`);
    }
  }
  return found;
}

check("the broker, the one process that holds the key, starts under permission.json's flags, root out of every reach, and every job runs its Node", () => {
  // The flags: Node's permission model, no code from strings and no node:sqlite, reads of the
  // broker and its limits alone, and none of the flags that would let it start, load or write
  // anything. What the broker's own code imports or calls guarantees nothing, since
  // process.getBuiltinModule reaches any built-in; the flags do, and the shape of what it sends.
  const flags = PERMISSION.broker.flags;
  for (const f of ["--permission", "--disallow-code-generation-from-strings", "--no-experimental-sqlite"]) assert.ok(flags.includes(f), `the broker's flags lose ${f}`);
  assert.deepEqual(flags.filter((f) => f.startsWith("--allow-")).sort(), ["--allow-fs-read=$GITHUB_WORKSPACE/benchmark/harness/broker-limits.json", "--allow-fs-read=$GITHUB_WORKSPACE/benchmark/harness/broker.ts"]);
  for (const r of PERMISSION.refused) assert.ok(!flags.some((f) => f.startsWith(r)), `the broker's flags hold ${r}`);
  assert.match(PERMISSION.node, /^\d+\.\d+\.\d+$/);
  // A relative import would fail as the broker ran, since its flags let it read no other file.
  assert.deepEqual(readSecretModule(PERMISSION.broker.script, read(PERMISSION.broker.script)).imports, [], "the broker imports a file, which its flags do not let it read");
  // start-broker.sh starts it so, and nothing else.
  const script = read("benchmark/harness/start-broker.sh");
  assert.deepEqual(startBrokerProblems(script), []);
  for (const [what, text] of [
    ["--allow-child-process among the flags", script.replace("--permission", "--permission --allow-child-process")],
    ["no env -i", script.replace('/usr/bin/env -i PATH="$PATH" ', "")],
    ["Node found through PATH", script.replace('"$RUNNER_TOOL_CACHE/node/' + "24.22.1" + '/x64/bin/node"', "node")],
    ["env found through PATH", script.replace('/usr/bin/env -i PATH="$PATH" ', 'env -i PATH="$PATH" ')],
    ["no core limit", script.replace("ulimit -c 0\n", "")],
    ["no mask", script.replace("umask 0177\n", "")],
    ["a mask after the broker starts", script.replace("umask 0177\n", "").replace(/(> "\$dir\/log" 2>&1 &\n)/, "$1umask 0177\n")],
    ["a second node", `${script}node -e 1\n`],
    // The re-check of 4ab15da: lines added before the key's line, each a way to leak the key.
    ["a trace", script.replace("ulimit -c 0\n", "set -x\nulimit -c 0\n")],
    ["the key written to a file", script.replace("ulimit -c 0\n", 'printf %s "$ANTHROPIC_API_KEY" > "$RUNNER_TEMP/k"\nulimit -c 0\n')],
    ["the key handed to another program", script.replace("ulimit -c 0\n", '/usr/bin/printf %s "$ANTHROPIC_API_KEY" | sha256sum\nulimit -c 0\n')],
  ]) {
    assert.ok(startBrokerProblems(text).length > 0, `a start-broker.sh with ${what} passes`);
    // And the pin, which holds the file whole, fails on it whatever the rules above read.
    assert.notEqual(digest(text), PINNED_FILES.get("benchmark/harness/start-broker.sh"), `a start-broker.sh with ${what} keeps its pin`);
  }
  for (const file of ["start-broker.sh", "stop-broker.sh", "lock-sudo.sh", "sandbox-root.sh", "fetch-held-out.sh"]) assert.ok(PINNED_FILES.has(`benchmark/harness/${file}`), `${file}, on the key's path, the held-out token's or the lock's, is not pinned whole`);
  // The held-out step's script is held by its pin, whose digest is its exact text: no rule over its
  // lines stands for that (the review of aafe59b found a denylist of them easy to pass).
  // held-out-set, where it checks the set out, is no path of the repository in any form: a link
  // tracked there would take the checkout wherever it pointed (the review of aafe59b).
  assert.deepEqual(heldOutPathProblems(tracked()), []);
  assert.ok(heldOutPathProblems(new Set(["held-out-set"])).length > 0 && heldOutPathProblems(new Set(["held-out-set/x.md"])).length > 0, "a tracked held-out-set passed");
  // Every job sets up that Node: the secret jobs, the jobs that run the harness's tests and the
  // planted escapes among them, and the rest.
  assert.deepEqual(nodeProblems(allJobs()), []);
  for (const v of [24, "24", "24.x", "lts/*", undefined]) {
    const planted = [{ file: "x.yml", id: "y", job: { steps: [{ uses: "actions/setup-node@0", with: v === undefined ? {} : { "node-version": v } }] } }];
    assert.ok(nodeProblems(planted).length > 0, `a job that sets up Node ${v} passes`);
  }
  // Every job that runs the sandbox takes root from the runner's user before it, and checks at its
  // end that it is still out of reach.
  for (const { file, id, job } of allJobs()) assert.deepEqual(sudoLockProblems(`${file}#${id}`, job), []);
  assert.ok(allJobs().some(({ file, id, job }) => !SECRET_JOBS.has(`${file}#${id}`) && sudoLockProblems(`${file}#${id}`, { steps: steps(job).filter((s) => !isStep(s, LOCK_STEP)) }).length > 0), "no job that runs the sandbox outside the secret jobs proves the lock on every change");
  // Planted: each way a secret job's broker could go wrong.
  const job = {
    steps: [
      { uses: "actions/checkout@0", with: { "persist-credentials": false } },
      { uses: "actions/setup-node@0", with: { "node-version": PERMISSION.node, "package-manager-cache": false } },
      LOCK_STEP,
      HELD_OUT_STEP,
      BROKER_STEP,
      { name: "Run", run: 'node --disallow-code-generation-from-strings benchmark/harness/bench.ts --sandbox "$SANDBOX" --broker "$RUNNER_TEMP/broker/socket"' },
      STOP_STEP,
    ],
  };
  assert.deepEqual(brokerProblems("x.yml#y", job), []);
  assert.deepEqual(sudoLockProblems("x.yml#y", job), []);
  const at = (k, step) => ({ steps: job.steps.map((s, i) => (i === k ? step : s)) });
  const without = (k) => ({ steps: job.steps.filter((_, i) => i !== k) });
  for (const [what, planted] of [
    ["the broker's step changed", at(4, { ...BROKER_STEP, run: "bash benchmark/harness/start-broker.sh --allow-child-process" })],
    ["the key in the harness's step", at(5, { ...job.steps[5], env: { ANTHROPIC_API_KEY: "${{ secrets.ANTHROPIC_API_KEY }}" } })],
    ["a harness that does not talk to the broker", at(5, { name: "Run", run: 'node benchmark/harness/bench.ts --sandbox "$SANDBOX"' })],
    ["the held-out checkout keeping its token", at(3, { uses: "actions/checkout@0", with: { token: HELD_OUT_TOKEN } })],
    // Run 3 of the plan: actions/checkout of the held-out repository printed its commit's subject.
    ["the held-out set checked out by actions/checkout", at(3, { uses: "actions/checkout@0", with: { repository: "Menteion/mente-bench-heldout", token: HELD_OUT_TOKEN, "persist-credentials": false } })],
    ["the held-out repository checked out by actions/checkout with no secret", { steps: [...job.steps.slice(0, 3), { uses: "actions/checkout@0", with: { repository: "menteion/MENTE-BENCH-HELDOUT" } }, ...job.steps.slice(3)] }],
    ["the held-out step changed", at(3, { ...HELD_OUT_STEP, run: "bash benchmark/harness/fetch-held-out.sh; git -C held-out-set log -1" })],
    ["the key in the held-out step", at(3, { ...HELD_OUT_STEP, env: { ...HELD_OUT_STEP.env, ANTHROPIC_API_KEY: "${{ secrets.ANTHROPIC_API_KEY }}" } })],
    ["the held-out set checked out twice", { steps: [...job.steps.slice(0, 4), HELD_OUT_STEP, ...job.steps.slice(4)] }],
    ["the held-out step run in the job's environment", at(3, { ...HELD_OUT_STEP, run: "bash benchmark/harness/fetch-held-out.sh" })],
    ["a trace set for later steps before the secrets", { steps: [...job.steps.slice(0, 2), { run: 'echo "SHELLOPTS=xtrace" >> "$GITHUB_ENV"' }, ...job.steps.slice(2)] }],
    ["a program put first on PATH before the secrets", { steps: [...job.steps.slice(0, 2), { run: 'echo "$RUNNER_TEMP/bin" >> "$GITHUB_PATH"' }, ...job.steps.slice(2)] }],
    // The re-check of c55cf8f: the runner's bash read BASH_ENV and SHELLOPTS before the step's
    // first line, the token already in its environment; env, mktemp and bash came from PATH; and
    // the token stood on env's command line.
    ["BASH_ENV set for later steps before the held-out step", { steps: [...job.steps.slice(0, 3), { run: 'echo "BASH_ENV=$RUNNER_TEMP/x.sh" >> "$GITHUB_ENV"' }, ...job.steps.slice(3)] }],
    ["SHELLOPTS=xtrace set for later steps before the broker's step", { steps: [...job.steps.slice(0, 4), { run: 'echo "SHELLOPTS=xtrace" >> "$GITHUB_ENV"' }, ...job.steps.slice(4)] }],
    ["the held-out step in the runner's shell", at(3, { name: HELD_OUT_STEP.name, if: HELD_OUT_STEP.if, env: HELD_OUT_STEP.env, run: HELD_OUT_STEP.run })],
    ["the broker's step in the runner's shell", at(4, { name: BROKER_STEP.name, env: BROKER_STEP.env, run: BROKER_STEP.run })],
    ["a shell that keeps BASH_ENV", at(3, { ...HELD_OUT_STEP, shell: SECRET_SHELL.replace("-u BASH_ENV ", "") })],
    // The final pass on 89c7691: an exported function in place of printf printed the secret.
    ["a shell whose bash is not privileged", at(3, { ...HELD_OUT_STEP, shell: SECRET_SHELL.replace(" -p ", " ") })],
    ["fetch-held-out.sh in a bash that is not privileged", at(3, { ...HELD_OUT_STEP, run: HELD_OUT_STEP.run.replace(" -p ", " ") })],
    ["start-broker.sh in a bash that is not privileged", at(4, { ...BROKER_STEP, run: BROKER_STEP.run.replace(" -p ", " ") })],
    ["env found through PATH in the held-out step", at(3, { ...HELD_OUT_STEP, run: HELD_OUT_STEP.run.replace("/usr/bin/env ", "env ") })],
    ["mktemp found through PATH", at(3, { ...HELD_OUT_STEP, run: HELD_OUT_STEP.run.replace("/usr/bin/mktemp", "mktemp") })],
    ["the held-out token on env's command line", at(3, { ...HELD_OUT_STEP, run: 'home="$(/usr/bin/mktemp -d)"\n/usr/bin/env -i PATH=/usr/bin:/bin HOME="$home" HELD_OUT_TOKEN="$HELD_OUT_TOKEN" HELD_OUT_COMMIT="$HELD_OUT_COMMIT" /usr/bin/bash --noprofile --norc -p benchmark/harness/fetch-held-out.sh\n' })],
    ["bash found through PATH in the broker's step", at(4, { ...BROKER_STEP, run: "bash benchmark/harness/start-broker.sh" })],
    ["the key as the checkout's token", at(3, { uses: "actions/checkout@0", with: { token: "${{ secrets.ANTHROPIC_API_KEY }}", "persist-credentials": false } })],
    ["another secret as the checkout's token", at(3, { uses: "actions/checkout@0", with: { token: "${{ secrets.OTHER }}", "persist-credentials": false } })],
    ["the held-out token in a step's env", at(5, { ...job.steps[5], env: { HELD_OUT: HELD_OUT_TOKEN } })],
    ["the held-out token in the checkout's env", at(3, { ...job.steps[3], env: { TOKEN: HELD_OUT_TOKEN } })],
    ["the key in the job's env", { ...job, env: { ANTHROPIC_API_KEY: "${{ secrets.ANTHROPIC_API_KEY }}" } }],
    ["no stop", { steps: job.steps.slice(0, 6) }],
    ["a stop that is not last", { steps: [...job.steps.slice(0, 5), STOP_STEP, job.steps[5]] }],
    ["a stop without the lock's check", at(6, { ...STOP_STEP, run: "bash benchmark/harness/stop-broker.sh\n" })],
    ["the broker started twice", { steps: [...job.steps.slice(0, 5), BROKER_STEP, ...job.steps.slice(5)] }],
    ["no lock", without(2)],
    ["the lock after the key is in reach", { steps: [job.steps[0], job.steps[1], job.steps[3], LOCK_STEP, ...job.steps.slice(4)] }],
    ["the lock under a condition", at(2, { ...LOCK_STEP, if: "false" })],
    ["no setup-node", without(1)],
    ["another Node", at(1, { uses: "actions/setup-node@0", with: { "node-version": "24" } })],
  ]) {
    assert.ok(brokerProblems("x.yml#y", planted).length > 0, `a secret job with ${what} passes`);
  }
  assert.ok(brokerProblems("x.yml#y", job, { env: { K: "${{ secrets.ANTHROPIC_API_KEY }}" } }).length > 0, "a secret job whose workflow's env holds the key passes");
  // Planted: a job that runs the sandbox and holds no secret.
  const watched = (s) => ({ ...s, if: "steps.watch.outputs.run != 'false'" });
  const fixtures = { steps: [watched(LOCK_STEP), watched({ name: "Prove", run: 'node benchmark/harness/sandbox-check.ts --sandbox "$SANDBOX"' }), LOCKED_STEP] };
  assert.deepEqual(sudoLockProblems("x.yml#f", fixtures), []);
  for (const [what, planted] of [
    ["no lock", { steps: fixtures.steps.slice(1) }],
    ["the lock after the sandbox", { steps: [fixtures.steps[1], fixtures.steps[0], fixtures.steps[2]] }],
    ["no check at the end", { steps: fixtures.steps.slice(0, 2) }],
    ["a check that is not last", { steps: [fixtures.steps[0], fixtures.steps[2], fixtures.steps[1]] }],
  ]) {
    assert.ok(sudoLockProblems("x.yml#f", planted).length > 0, `a job that runs the sandbox with ${what} passes`);
  }
});
/// A node a job of SECRET_JOBS starts without disallowing code generation from strings.
const NODE_WITHOUT_GUARD = /(^|[\s;&|(])node\s+(?!--disallow-code-generation-from-strings\s)/;

check("a node line without the flag that refuses code from strings is read as one", () => {
  for (const line of ["node benchmark/harness/bench.ts", "x && node -e 1", "(node x.mjs)"]) assert.match(line, NODE_WITHOUT_GUARD, line);
  assert.doesNotMatch("node --disallow-code-generation-from-strings benchmark/harness/bench.ts --budget 1", NODE_WITHOUT_GUARD);
});

/// The other jobs that name an environment, each with its name: the Pages deployment, whose
/// environment holds no secret.
const OTHER_ENVIRONMENTS = new Map([["pages.yml#deploy", "github-pages"]]);

/// What is wrong with a job's environment: a job of SECRET_JOBS names SECRET_ENVIRONMENT, as a
/// literal string, a job of OTHER_ENVIRONMENTS names its own, and no other job names any, so that
/// the secrets reach no job the policy does not hold as one of SECRET_JOBS.
export function environmentProblems(key, job) {
  if (SECRET_JOBS.has(key)) return job.environment === SECRET_ENVIRONMENT ? [] : [`${key} holds a secret and does not name the environment ${SECRET_ENVIRONMENT} as a literal string`];
  const other = OTHER_ENVIRONMENTS.get(key);
  const name = typeof job.environment === "object" && job.environment !== null ? job.environment.name : job.environment;
  if (other !== undefined) return name === other ? [] : [`${key} does not name the environment ${other}, the one OTHER_ENVIRONMENTS gives it`];
  return "environment" in job ? [`${key} names an environment, which only the jobs of SECRET_JOBS and OTHER_ENVIRONMENTS do`] : [];
}

/// What is wrong with how a workflow that holds a job of SECRET_JOBS starts and what its token may
/// do: it starts by a dispatch alone, which only someone with write access can make, and its token
/// reads the repository's contents and nothing more, at the workflow's level and in no job's
/// override. Any other trigger would start the job with ref main, which the environment admits:
/// `issue_comment` on any user's comment, `push` on every merge, `schedule` unasked (the review of
/// 41a54b6 found `issue_comment:` and `contents: write` in probe.yml passing every check).
export function secretWorkflowProblems(file, doc) {
  const found = [];
  const on = Object.keys(triggers(doc));
  if (on.length !== 1 || on[0] !== "workflow_dispatch") found.push(`${file} holds a secret job and starts on ${on.join(", ") || "nothing"}, not on workflow_dispatch alone`);
  if (!isPlain(doc.permissions) || Object.keys(doc.permissions).length !== 1 || doc.permissions.contents !== "read") found.push(`${file} holds a secret job and its token's permissions are not exactly contents: read`);
  for (const [id, job] of Object.entries(doc.jobs ?? {})) if (isPlain(job) && "permissions" in job) found.push(`${file}#${id} sets permissions of its own in a workflow that holds a secret job`);
  // Every other job of it names no environment, so a secret it could see is a repository's or an
  // organization's, which must not exist: each carries LEAK_STEP, which fails then.
  for (const [id, job] of Object.entries(doc.jobs ?? {})) {
    if (SECRET_JOBS.has(`${file}#${id}`)) continue;
    const leak = steps(job).filter((s) => JSON.stringify(s) === JSON.stringify(LEAK_STEP)).length;
    if (leak !== 1) found.push(`${file}#${id} names no environment and does not check, once, as LEAK_STEP, that no secret of the benchmark is set outside it`);
  }
  return found;
}
/// The step that fails when a secret of the benchmark is set outside its environment, as a
/// repository's or an organization's secret of the same name (the review of 41a54b6).
const LEAK_STEP = {
  name: "No secret of the benchmark outside its environment",
  env: { KEY_LEAKED: "${{ secrets.ANTHROPIC_API_KEY != '' }}", HELD_OUT_LEAKED: "${{ secrets.MENTE_BENCH_HELDOUT_TOKEN != '' }}" },
  run: 'if [ "$KEY_LEAKED" != false ] || [ "$HELD_OUT_LEAKED" != false ]; then echo "::error::a secret of the benchmark is set outside the environment benchmark"; exit 1; fi',
};

/// The required job that runs LEAK_STEP on every pull request, so that a secret of the benchmark
/// set again at the repository's or the organization's level is seen at once, not at the next
/// dispatch of a secret workflow (the final pass of ad38011).
const LEAK_CHECKED = "ci.yml#docs";

check("every workflow that holds a secret job starts by a dispatch alone, with a token that only reads", () => {
  const holding = new Set([...SECRET_JOBS.keys()].map((k) => k.split("#")[0]));
  for (const { file, doc } of workflows()) if (holding.has(file)) assert.deepEqual(secretWorkflowProblems(file, doc), []);
  const [leakFile, leakJob] = LEAK_CHECKED.split("#");
  const checked = workflows().find((x) => x.file === leakFile)?.doc.jobs?.[leakJob];
  assert.equal(steps(checked).filter((s) => JSON.stringify(s) === JSON.stringify(LEAK_STEP)).length, 1, `${LEAK_CHECKED} does not run LEAK_STEP once, on every pull request`);
  assert.ok(requiredJobs().some(({ file, id }) => `${file}#${id}` === LEAK_CHECKED), `${LEAK_CHECKED}, which checks on every pull request that no secret leaked, is no required job`);
  const base = { on: { workflow_dispatch: { inputs: { runs: { default: "1" } } } }, permissions: { contents: "read" }, jobs: { write: { "runs-on": "ubuntu-24.04" }, check: { steps: [{ uses: "actions/checkout@0" }, LEAK_STEP] } } };
  assert.deepEqual(secretWorkflowProblems("probe.yml", base), []);
  for (const check of [{ steps: [] }, { steps: [{ ...LEAK_STEP, run: "true" }] }, { steps: [{ ...LEAK_STEP, env: { KEY_LEAKED: LEAK_STEP.env.KEY_LEAKED } }] }, { steps: [LEAK_STEP, LEAK_STEP] }]) {
    assert.ok(secretWorkflowProblems("probe.yml", { ...base, jobs: { ...base.jobs, check } }).length > 0, `a job without the leak check passes: ${JSON.stringify(check)}`);
  }
  // The leak check reads as no secret held: only the test whether one is set.
  assert.ok(!holdsSecret(LEAK_STEP));
  for (const [what, doc] of [
    ["issue_comment", { ...base, on: { ...base.on, issue_comment: { types: ["created"] } } }],
    ["push to main", { ...base, on: { ...base.on, push: { branches: ["main"] } } }],
    ["schedule", { ...base, on: { ...base.on, schedule: [{ cron: "0 0 * * *" }] } }],
    ["workflow_call", { ...base, on: { ...base.on, workflow_call: null } }],
    ["repository_dispatch", { ...base, on: { ...base.on, repository_dispatch: null } }],
    ["pull_request", { ...base, on: "pull_request" }],
    ["a list of triggers", { ...base, on: ["workflow_dispatch", "push"] }],
    ["contents: write", { ...base, permissions: { contents: "write" } }],
    ["write-all", { ...base, permissions: "write-all" }],
    ["id-token beside contents", { ...base, permissions: { contents: "read", "id-token": "write" } }],
    ["no permissions", { ...base, permissions: undefined }],
    ["a job's own permissions", { ...base, jobs: { write: { "runs-on": "ubuntu-24.04", permissions: { contents: "write" } } } }],
  ]) {
    assert.ok(secretWorkflowProblems("probe.yml", doc).length > 0, `a secret workflow with ${what} passes`);
  }
});

check("every job that holds a secret names the environment benchmark, for main alone, and no other job names it", () => {
  for (const { file, id, job } of allJobs()) assert.deepEqual(environmentProblems(`${file}#${id}`, job), []);
  for (const [key, job] of [
    ["benchmark.yml#bench", {}],
    ["benchmark.yml#bench", { environment: { name: SECRET_ENVIRONMENT } }],
    ["prelim.yml#prelim", { environment: "${{ inputs.environment }}" }],
    ["benchmark.yml#build", { environment: SECRET_ENVIRONMENT }],
    ["ci.yml#test", { environment: "staging" }],
    ["pages.yml#deploy", { environment: { name: SECRET_ENVIRONMENT } }],
  ]) {
    assert.ok(environmentProblems(key, job).length > 0, `the environment rule lets through ${key}: ${JSON.stringify(job)}`);
  }
});

/// The names a secret may be handed over as, read from the harness that leaves them out of every
/// child's environment, so that the two cannot drift: one definition, in build.ts.
/// It is read from build.ts's tokens, so that a comment or a string naming it is none, and it must
/// be defined there exactly once, as `const SECRET_NAME = /.../i;`.
export function secretNameOf(text) {
  const toks = jsTokens(text);
  const found = toks.flatMap((x, i) =>
    x.t === "id" && x.v === "const" && toks[i + 1]?.v === "SECRET_NAME" && toks[i + 2]?.v === "=" && toks[i + 3]?.t === "re" && toks[i + 4]?.v === ";" ? [toks[i + 3].v] : [],
  );
  const defined = toks.filter((x, i) => x.t === "id" && x.v === "SECRET_NAME" && /^(const|let|var|function|class)$/.test(toks[i - 1]?.v ?? "")).length;
  if (found.length !== 1 || defined !== 1) return null;
  const m = /^\/(.+)\/i$/.exec(found[0]);
  return m ? new RegExp(m[1], "i") : null;
}
const SECRET_NAME = (() => {
  const name = secretNameOf(read("benchmark/harness/build.ts"));
  assert.ok(name, "benchmark/harness/build.ts does not define SECRET_NAME exactly once, as a case-blind regular expression");
  return name;
})();
/// The action inputs a secret may be handed to, each an action by its repository and the input:
/// the checkout's token, which it keeps from the logs and from what it writes. Any other input of
/// any action may show it, as `actions/cache`'s key does in the list of caches (the review of
/// 41a54b6).
const SECRET_INPUTS = new Set(["actions/checkout#token"]);
/// The one form a secret is handed over in: the whole value, the secret alone, so that no
/// expression turns it into something no name or mask follows (`format('{0}', secrets.X)`).
const SECRET_VALUE = /^\$\{\{ secrets\.[A-Z][A-Z0-9_]* \}\}$/;

/// Where a job holds a secret other than as the value of a step's or the job's environment
/// variable, or of a step's action input, whose name SECRET_NAME matches: a secret in a script, in
/// the workflow's env, under a name that does not look like one (the review of b2be87b: a `*_PAT`
/// would reach a child of the harness, which leaves out only names that look like a secret's).
export function secretPlaceProblems(key, doc, job) {
  const at = [];
  const walk = (v, where) => {
    if (typeof v === "string") {
      if (holdsSecret(v)) at.push(where);
    } else if (Array.isArray(v)) v.forEach((x, i) => walk(x, `${where}[${i}]`));
    else if (v !== null && typeof v === "object") {
      for (const [k, x] of Object.entries(v)) {
        if (holdsSecret(k)) at.push(`${where}.<a key>`);
        walk(x, `${where}.${k}`);
      }
    }
  };
  walk(doc.env ?? {}, "workflow.env");
  walk(job, "job");
  const value = (where) => where.replace(/^job/, "").split(/\.|\[(\d+)\]/).filter(Boolean).reduce((v, k) => v?.[k], job);
  // An expression that reads a secret-named variable back out of the environment, or the whole
  // environment, hands the secret on as `secrets.` would: in an action's input, an artifact's
  // name, a script's output (the final pass of ad38011: `${{ env.ANTHROPIC_API_KEY }}` in a
  // cache's key).
  const fromEnv = [];
  const read = (v, where) => {
    if (typeof v === "string") {
      const k = v.indexOf("${{");
      if (k < 0) return;
      // Allowed, not refused: every `env` of an expression must be `env.<name>` or `env['<name>']`
      // with a name that is no secret's; anything else, `env.*`, `env[<expression>]` or `env`
      // alone, counts as reading the whole environment (the review of 75e5eeb: `join(env.*, '-')`
      // and `env[format(...)]` matched none of the forms refused before).
      const text = v.slice(k);
      for (const m of text.matchAll(/(?<![.\w])env\b/gi)) {
        const after = text.slice(m.index + m[0].length);
        const named = /^\s*\.\s*([A-Za-z_][\w-]*)(?![\w-]*\s*[([*])/.exec(after) ?? /^\s*\[\s*'([A-Za-z_][\w-]*)'\s*\]/.exec(after);
        if (!named || SECRET_NAME.test(named[1])) fromEnv.push(`${key}: ${where} reads ${named ? named[1] : "the whole environment"} out of the environment in an expression`);
      }
    } else if (Array.isArray(v)) v.forEach((x, i) => read(x, `${where}[${i}]`));
    else if (v !== null && typeof v === "object") for (const [k, x] of Object.entries(v)) read(x, `${where}.${k}`);
  };
  read(doc.env ?? {}, "workflow.env");
  read(job, "job");
  return at
    .filter((where) => {
      // Only a step's own environment, which no other step of the job inherits.
      const env = /^job\.steps\[\d+\]\.env\.([A-Za-z_][\w-]*)$/.exec(where);
      if (env) return !(SECRET_NAME.test(env[1]) && SECRET_VALUE.test(String(value(where))));
      const input = /^job\.steps\[(\d+)\]\.with\.([A-Za-z_][\w-]*)$/.exec(where);
      if (input) {
        const action = String(steps(job)[Number(input[1])]?.uses ?? "").replace(/@.*$/, "");
        return !(SECRET_INPUTS.has(`${action}#${input[2]}`) && SECRET_VALUE.test(String(value(where))));
      }
      return true;
    })
    .map((where) => `${key}: a secret at ${where}, not the whole value of a step's environment variable whose name matches ${SECRET_NAME} or of an input of SECRET_INPUTS`)
    .concat(fromEnv);
}

check("every secret is the value of an environment variable or an action input whose name looks like a secret's", () => {
  for (const { file, doc, id, job } of allJobs()) assert.deepEqual(secretPlaceProblems(`${file}#${id}`, doc, job), []);
  const s = "${{ secrets.ANTHROPIC_API_KEY }}";
  for (const [doc, job] of [
    [{}, { steps: [{ env: { BENCH_PAT: s } }] }],
    [{}, { env: { HELD_OUT: s } }],
    [{}, { steps: [{ uses: "actions/checkout@0", with: { ref: s } }] }],
    [{}, { steps: [{ run: `export ANTHROPIC_API_KEY=${s}` }] }],
    [{ env: { ANTHROPIC_API_KEY: s } }, { steps: [] }],
    [{}, { steps: [{ if: "${{ secrets.ANTHROPIC_API_KEY == 'x' }}", run: "true" }] }],
    // The review of 41a54b6: an input of another action, or of the checkout but its token, and a
    // secret turned by an expression.
    [{}, { steps: [{ uses: "actions/cache@0", with: { path: "x", key: s } }] }],
    [{}, { steps: [{ uses: "actions/checkout@0", with: { "ssh-key": s } }] }],
    [{}, { steps: [{ uses: "someone/checkout@0", with: { token: s } }] }],
    [{}, { steps: [{ env: { ANTHROPIC_API_KEY: "${{ format('{0}', secrets.ANTHROPIC_API_KEY) }}" } }] }],
    [{}, { steps: [{ env: { ANTHROPIC_API_KEY: `x${s}` } }] }],
    [{}, { steps: [{ uses: "actions/checkout@0", with: { token: "${{ secrets.A || github.token }}" } }] }],
    // The final pass of ad38011: the secret in the job's env, read back by `env.`.
    [{}, { env: { ANTHROPIC_API_KEY: s }, steps: [] }],
    [{}, { steps: [{ uses: "actions/cache@0", with: { key: "${{ env.ANTHROPIC_API_KEY }}" } }] }],
    [{}, { steps: [{ uses: "actions/upload-artifact@0", with: { name: "x-${{ env['MENTE_BENCH_HELDOUT_TOKEN'] }}" } }] }],
    [{}, { steps: [{ run: "echo ${{ toJSON(env) }} >> $GITHUB_OUTPUT" }] }],
    // The review of 75e5eeb: the whole environment by a filter or a computed name.
    [{}, { steps: [{ uses: "actions/cache@0", with: { key: "${{ join(env.*, '-') }}" } }] }],
    [{}, { steps: [{ uses: "actions/cache@0", with: { key: "${{ toJSON(env.*) }}" } }] }],
    [{}, { steps: [{ uses: "actions/cache@0", with: { key: "${{ env[format('{0}_API_KEY', 'ANTHROPIC')] }}" } }] }],
    [{}, { steps: [{ uses: "actions/cache@0", with: { key: "${{ env[matrix.k] }}" } }] }],
  ]) {
    assert.ok(secretPlaceProblems("x.yml#y", doc, job).length > 0, `a secret in the wrong place let through: ${JSON.stringify({ doc, job })}`);
  }
  assert.deepEqual(secretPlaceProblems("x.yml#y", {}, { env: { HAS: "${{ secrets.A != '' }}" }, steps: [{ env: { ANTHROPIC_API_KEY: s } }, { uses: "actions/checkout@0", with: { token: s } }, { uses: "actions/cache@0", with: { key: "x-${{ env.MDBOOK_VERSION }}-${{ inputs.env }}" } }] }), []);
  // SECRET_NAME is read from tokens, defined once (the review of 41a54b6: the first match of a
  // regular expression could have been a comment).
  assert.equal(String(secretNameOf("// const SECRET_NAME = /x/i;\nconst SECRET_NAME = /key|token/i;")), "/key|token/i");
  for (const text of ["const SECRET_NAME = /key/;", "const SECRET_NAME = /a/i;\nconst SECRET_NAME = /b/i;", "const SECRET_NAME = new RegExp('key', 'i');", '"const SECRET_NAME = /key/i;"', "const SECRET_NAME = /key/i;\nfunction SECRET_NAME() {}"]) {
    assert.equal(secretNameOf(text), null, `SECRET_NAME read from: ${text}`);
  }
});

check("every job that holds a secret is pinned whole, takes mn from a job that checks it first, and runs agent code only in its proven sandbox", () => {
  const found = new Set();
  const holds = (v) => holdsSecret(v);
  for (const { file, doc, id, job } of allJobs()) {
    const key = `${file}#${id}`;
    const all = steps(job);
    // The workflow's env, anything of the job but its steps, and a job that passes secrets on.
    const wide = holds(doc.env) || holds({ ...job, steps: null }) || "secrets" in job;
    const secretAt = wide ? 0 : all.findIndex((s) => holds(s));
    if (secretAt < 0) continue;
    found.add(key);
    const held = SECRET_JOBS.get(key);
    assert.ok(held, `${key} holds a secret and is not in SECRET_JOBS: name the job that builds mn for it, if any`);
    assert.ok(PINNED_JOBS.has(key), `${key} holds a secret and is not pinned whole`);
    // Every file it runs, and every module those import, is the code owner's: the pin holds the
    // job's steps, and CODEOWNERS what they run (the final review of PR 137 found the toolchains'
    // installer and five modules of the process that holds the key in neither).
    const runs = [...repoPaths(scriptTexts(doc, job))];
    const loaded = runs.flatMap((f) => COMPUTED_LOADS.get(f)?.loads ?? []);
    const unowned = [...runs, ...loaded].filter((f) => !GATE_FILES.some((p) => owns(p, f)));
    assert.deepEqual(unowned, [], `${key} holds a secret and runs files .github/CODEOWNERS does not give its owner`);
    // And each module of it loads only what that closure follows, and starts processes only in
    // SPAWNERS.
    assert.deepEqual(runs.filter((f) => JS_MODULE.test(f) || runs.some((g) => JS_MODULE.test(g) && readSecretModule(g, read(g)).imports.includes(f))).flatMap((f) => secretModuleProblems(f, read(f))), [], `${key} holds a secret, and a module it runs loads code or starts a process the policy cannot read`);
    // And every node it starts generates no code from strings, which no spelling of a module gets
    // past: eval, Function and its kin, reached by any route, throw.
    for (const s of all.filter((x) => typeof x.run === "string")) {
      for (const l of logicalLines(s.run)) assert.doesNotMatch(l, NODE_WITHOUT_GUARD, `${stepKey(file, id, s)} starts node without --disallow-code-generation-from-strings: ${l}`);
    }
    if (held.builtBy !== null) {
      assert.ok([job.needs].flat().includes(held.builtBy), `${key} does not need ${held.builtBy}, the job that builds mn for it`);
      const builder = doc.jobs?.[held.builtBy];
      assert.ok(builder, `${file} has no job ${held.builtBy}`);
      assert.deepEqual(beforeToolchainProblems(file, held.builtBy, doc, builder), [], `${file}#${held.builtBy} builds mn for a job that holds a secret without the build-time check first`);
      assert.ok(!holds(builder) && !("secrets" in builder), `${file}#${held.builtBy}, which builds mn, holds a secret`);
      assert.ok(all.some((s) => /^actions\/download-artifact@/.test(String(s.uses ?? ""))), `${key} takes no artifact from ${held.builtBy}`);
    }
    if (held.agents) {
      const check = all.findIndex((s) => /benchmark\/harness\/sandbox-check\.ts/.test(String(s.run ?? "")));
      assert.ok(check >= 0 && check < secretAt, `${key}: runs agent code, but sandbox-check.ts does not run before the secret is in reach`);
      for (const k of ["if", "continue-on-error"]) assert.ok(!(k in all[check]), `${key}: the sandbox check carries ${k}:`);
      // The step that runs agent code, which holds no secret and talks to the broker, runs it all
      // through the sandbox.
      const runs = all.filter((s) => /benchmark\/harness\/(bench|prelim)\.ts/.test(String(s.run ?? "")));
      assert.equal(runs.length, 1, `${key} runs the harness in ${runs.length} steps, not one`);
      assert.match(runs[0].run, /--sandbox "\$SANDBOX"/, `${key}, step "${runs[0].name}": runs agent code, but not through the sandbox`);
    }
    assert.deepEqual(brokerProblems(key, job, doc), []);
  }
  assert.deepEqual([...SECRET_JOBS.keys()].filter((k) => !found.has(k)), [], "SECRET_JOBS names jobs that hold no secret");
  // The final review's plants, and the other spellings of a secret.
  for (const held of ["${{ secrets.ANTHROPIC_API_KEY }}", "${{ secrets['ANTHROPIC_API_KEY'] }}", "${{ toJSON(secrets) }}", "${{ secrets.A || 'x' }}", "${{ fromJSON(toJSON(secrets)).A }}", { env: { K: "${{ secrets['K'] }}" } }]) {
    assert.ok(holdsSecret(held), `a secret read as none: ${JSON.stringify(held)}`);
  }
  for (const none of ["${{ secrets.MENTE_BENCH_HELDOUT_TOKEN != '' }}", "${{ github.token }}", "secrets", "${{ env.SECRETS_DIR }}"]) assert.ok(!holdsSecret(none), `no secret read as one: ${none}`);
  // The review of 35bbe0f: a tab or a line break inside the expression, and the context in another case.
  for (const held of ["${{\tsecrets.ANTHROPIC_API_KEY }}", "${{\n  secrets.ANTHROPIC_API_KEY\n}}", "${{ SECRETS.A }}", "${{ Secrets['A'] }}", { steps: [{ env: { K: "${{\tsecrets.K }}" } }] }]) {
    assert.ok(holdsSecret(held), `a secret read as none: ${JSON.stringify(held)}`);
  }
  // And the modules: a load the policy cannot follow, or a process started outside SPAWNERS.
  for (const [file, text] of [
    ["benchmark/harness/api.ts", 'const m = await import(new URL("../" + "reference.mjs", import.meta.url));'],
    ["benchmark/harness/api.ts", "const r = createRequire(import.meta.url);"],
    ["benchmark/harness/tasks.ts", 'import { execSync as sh } from "child_process";\nsh(`${mn} explain ${code}`);'],
    ["benchmark/harness/tasks.ts", 'const cp = await import("node:child_process");'],
    ["benchmark/harness/build.ts", 'import { spawn, execSync } from "node:child_process";\nspawn(c, a);'],
    ["benchmark/harness/build.ts", 'import { spawn } from "node:child_process";\nspawn(c, a);\nspawn(mn, b);'],
    ["benchmark/check.mjs", 'import { execFileSync } from "node:child_process";\nexecFileSync("node", ["x.mjs"]);'],
    ["benchmark/harness/report.ts", "new Worker(file);"],
    ["benchmark/harness/report.ts", "eval(code);"],
    // The review of b2be87b, run and on paper: a loader's hook, a computed import behind a
    // comment, a built-in module by a computed name, Function and eval by other routes, an import
    // with no space, the loader's internals, a worker by another name, a native binding, and
    // specifiers neither relative nor of NODE_MODULES.
    ["benchmark/harness/api.ts", 'import { register } from "node:module";\nregister("../../docs/hooks.mjs", import.meta.url);'],
    ["benchmark/harness/api.ts", 'register("../../docs/hooks.mjs", import.meta.url);'],
    ["benchmark/harness/api.ts", "const m = await import/**/(process.env.X);"],
    ["benchmark/harness/api.ts", 'process.getBuiltinModule("child_" + "process").spawnSync(mn, ["test"], { env: process.env });'],
    ["benchmark/harness/api.ts", "Function(code)();"],
    ["benchmark/harness/api.ts", "(0, eval)(code);"],
    ["benchmark/harness/api.ts", 'import"../../docs/leak2.mjs";'],
    ["benchmark/harness/api.ts", 'import { x } from"../../docs/leak2.mjs";'],
    ["benchmark/harness/api.ts", "M._load(path, null, false);"],
    ["benchmark/harness/api.ts", "const vm = await import(`node:vm`);"],
    ["benchmark/harness/api.ts", 'import { Worker as W } from "node:worker_threads";\nnew W(code, { eval: true });'],
    ["benchmark/harness/api.ts", 'process.binding("spawn_sync");'],
    ["benchmark/harness/api.ts", 'import { x } from "/home/runner/x.mjs";'],
    ["benchmark/harness/api.ts", 'import { x } from "file:///home/runner/x.mjs";'],
    ["benchmark/harness/api.ts", 'import { runInThisContext } from"node:vm";'],
    ["benchmark/harness/api.ts", 'import * as fs from "fs";'],
    // And the routes to Function and to a computed global that a name alone does not show.
    ["benchmark/harness/api.ts", 'const f = (() => {})["constr" + "uctor"];'],
    ["benchmark/harness/api.ts", "const f = (() => {}).constructor;"],
    ["benchmark/harness/api.ts", "const { constructor: F } = Object.getPrototypeOf(async () => {});"],
    ["benchmark/harness/api.ts", 'const g = process["getBuiltinModule"];'],
    ["benchmark/harness/api.ts", 'const g = process[name];'],
    ["benchmark/harness/api.ts", "\\u0065val(code);"],
    ["benchmark/harness/api.ts", 'const e = "ev\\x61l";'],
    ["benchmark/harness/api.ts", `const e = "${String.fromCharCode(0x435)}val";`],
    ["benchmark/harness/api.ts", "const s = 'unclosed;\nimport(x);"],
    ["benchmark/harness/api.ts", 'const y = await import("./y.ts");'],
    // The review of 41a54b6: process past its allowed members, aliased or destructured, and a
    // member by any computed key, a plain one included.
    ["benchmark/harness/api.ts", 'process.execve("/bin/sh", ["sh", "-c", "id"], process.env);'],
    ["benchmark/harness/api.ts", 'const p = process;\np["getBuilt" + "inModule"]("node:child_" + "process");'],
    ["benchmark/harness/api.ts", "const { [k]: g } = process;"],
    ["benchmark/harness/api.ts", "const { env, getBuiltinModule: g } = process;"],
    ["benchmark/harness/api.ts", "f(process);"],
    ["benchmark/harness/api.ts", "process?.env;"],
    ["benchmark/harness/api.ts", '(() => 0)["constr" + "uctor"]("return 1")();'],
    ["benchmark/harness/api.ts", "const v = xs[i];"],
    ["benchmark/harness/api.ts", "const v = f(x)[0];"],
    ["benchmark/harness/api.ts", "const v = x?.[k];"],
    ["benchmark/harness/api.ts", 'const v = "abc"[k];'],
    ["benchmark/harness/api.ts", "const v = (a || b)[k];"],
    ["benchmark/harness/api.ts", "obj[k] = v;"],
    ["benchmark/harness/api.ts", "Object.defineProperty(Object.prototype, \"shell\", { value: \"/bin/sh\" });"],
    ["benchmark/own.mjs", 'export const own = (o, k) => o[k];\n'],
    // The re-check of da58409: a module of another kind, imported or run, and CommonJS's names.
    ["benchmark/harness/api.ts", 'import { x } from "./tasks.cts";'],
    ["benchmark/harness/api.ts", 'import { x } from "../reference.cjs";'],
    ["benchmark/harness/api.ts", 'import data from "./pins.json" with { type: "json" };'],
    ["benchmark/harness/leak.cjs", "const [, r] = arguments;\nr(\"node:child_\" + \"process\");"],
    ["benchmark/harness/leak.cts", "export const x = 1;"],
    ["benchmark/harness/api.ts", "const [, r] = arguments;"],
    ["benchmark/harness/api.ts", "module.require(name);"],
    ["benchmark/harness/api.ts", "exports.x = 1;"],
    // The review of 75e5eeb: a second import of child_process with an escape in its path.
    ["benchmark/harness/build.ts", `${read("benchmark/harness/build.ts")}\nimport { execSync } from "node:child\\u005fprocess";\nexport const sh = (c) => execSync(c);\n`],
    ["benchmark/harness/api.ts", 'const m = "node:child\\u005fprocess";'],
    // The final pass of ad38011: a name written as a string in an export's list.
    ["benchmark/harness/api.ts", 'export { default as z, "q" as w } from "node:child_process";'],
    ["benchmark/harness/api.ts", 'export { "x" as y } from "../../docs/leak.mjs";'],
    ["benchmark/harness/api.ts", 'const m = "child_process";'],
    // And the computed accesses the reading missed: a computed key in a pattern, an object or a
    // class, a keyword as a property's or a method's name, and an expression's non-null `!`.
    ["benchmark/harness/api.ts", "const { [k]: F } = Array.of;"],
    ["benchmark/harness/api.ts", "const o = { [k]: 1 };"],
    ["benchmark/harness/api.ts", "const o = { a: 1, [k]: 2 };"],
    ["benchmark/harness/api.ts", "class A {\n  x = 1;\n  [k]() {}\n}"],
    ["benchmark/harness/api.ts", "const f = Array.of[k];"],
    ["benchmark/harness/api.ts", "const f = m.delete[k];"],
    ["benchmark/harness/api.ts", "const f = o.catch(x)[k];"],
    ["benchmark/harness/api.ts", "const f = o?.for[k];"],
    ["benchmark/harness/api.ts", "const f = g![k];"],
    // A spawner past its named uses: an alias of what it imports, or the checker's git with other
    // arguments, or by another name.
    ["benchmark/harness/build.ts", 'import { spawn } from "node:child_process";\nconst s = spawn;\ns(mn, b);'],
    ["benchmark/harness/prelim.ts", 'import { spawnSync } from "node:child_process";\nconst r = [spawnSync][0](mn, b);'],
  ]) {
    assert.ok(secretModuleProblems(file, text).length > 0, `the rule for a secret job's modules lets through, in ${file}: ${text}`);
  }
  // The reading follows an import however it is spaced, and a comment or a string holds none.
  assert.deepEqual(
    jsImports(jsTokens('import"./a.mjs";import{b}from"./b.mjs";export*from"./c.mjs";// import "./d.mjs"\nconst s = "import \'./e.mjs\'";')).specifiers.map((s) => s.v),
    ["./a.mjs", "./b.mjs", "./c.mjs"],
  );
  assert.deepEqual(secretModuleProblems("benchmark/harness/api.ts", 'import { x } from "./tasks.ts";\nimport fs from "node:fs";\n// eval(code)\nconst s = "rpn-eval";\nclass A {\n  constructor(t) { this.t = t; }\n}\nconst v = own(xs, i) + own(m, "k") + process.env.HOME + process.argv.length;\nconst [a, b] = xs;\nlet [c] = ys;\nfor (const [k, w] of zs) if (k) [a, b] = [b, a];\nconst t: string[] = [];\nf(a, [b, c]);\nconst n = ![x].length;\nconst o = { a: [1, 2], b };\nx = 1;\n[a, b] = [b, a];'), []);
  assert.deepEqual(secretModuleProblems("benchmark/own.mjs", read("benchmark/own.mjs")), [], "benchmark/own.mjs is not the text the policy holds");
  // The review of b2be87b: a `}}` inside a quoted string of the expression, and the named load of
  // check.mjs moved or joined by another.
  for (const held of ["${{ '}}' && secrets.ANTHROPIC_API_KEY }}", "${{ format('{0}}}', secrets.ANTHROPIC_API_KEY) }}", "${{ secrets.A != '' }} ${{ secrets.B }}"]) {
    assert.ok(holdsSecret(held), `a secret read as none: ${held}`);
  }
  const checker = read("benchmark/check.mjs");
  const named = COMPUTED_LOADS.get("benchmark/check.mjs").line;
  assert.deepEqual(secretModuleProblems("benchmark/check.mjs", checker), []);
  for (const plant of [checker.replace(named, named.replace('path.join(dir, "reference.mjs")', 'path.join(dir, "tasks", "reference.mjs")')), `${checker}\nconst extra = await import(process.env.BENCH_EXTRA ?? "node:os");\n`, `${checker}\n${named}\n`]) {
    assert.ok(secretModuleProblems("benchmark/check.mjs", plant).length > 0, "the named load of check.mjs lets through a moved or a second load");
  }
  // The checker's git, held to its two named calls: other arguments (`-c core.pager=...` runs
  // anything), a third call, an alias, or a named call moved into a string.
  const dirty = '    const dirty = git(heldOutDir, "status", "--porcelain");';
  for (const plant of [
    checker.replace(dirty, '    const dirty = git(heldOutDir, "-c", "core.pager=sh -c id", "log");'),
    `${checker}\ngit(".", "-c", "core.fsmonitor=id", "status");\n`,
    `${checker}\nconst g = git;\ng(".", "status");\n`,
    checker.replace(dirty, `    const dirty = \`\n${dirty}\n\`;\n    const g = git;`),
    checker.replace('"/usr/bin/git", ["-C", dir, ...args]', '"/bin/sh", ["-c", ...args]'),
    checker.replace('"/usr/bin/git", ["-C"', '"git", ["-C"'),
  ]) {
    assert.ok(secretModuleProblems("benchmark/check.mjs", plant).length > 0, "the checker's git lets through a call it does not name");
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
  "/CLAUDE.md",
  "/spec/philosophy.md",
  "/package.json",
  "/package-lock.json",
  "/.spec-guard.json",
  "/.spec-graph.json",
  "/.spec-brief.json",
  "/perf/run.mjs",
  "/perf/lib.mjs",
  "/benchmark/check.mjs",
  "/benchmark/corpus.lock",
  "/benchmark/reference.mjs",
  "/benchmark/own.mjs",
  "/benchmark/harness/",
  "/compiler/mente-cli/tests/gates.rs",
  "/compiler/mente-cli/tests/lints.rs",
  "/compiler/mente-cli/tests/register.rs",
  "/compiler/mente-cli/tests/records.rs",
  "/compiler/mente-cli/tests/escapes.rs",
  "/compiler/mente-cli/src/records.rs",
  "/compiler/mente-cli/src/escapes.rs",
  "/compiler/mente-cli/src/evidence.rs",
  "/compiler/mente-cli/src/json.rs",
  "/compiler/mente-cli/tests/evidence.rs",
  "/tools/evidence-reporter.mjs",
  "/tools/evidence-reporter.test.mjs",
  "/tools/evidence-reporter.jsonl",
  "/compiler/mente-cli/src/front-matter/",
  "/compiler/*/src/overflow_traps.rs",
  "/compiler/mente-threads/",
  "/compiler/*/Cargo.toml",
  "/spikes/*/Cargo.toml",
  "/.gitattributes",
  "/tools/quick.mjs",
  "/tools/quick-checks.json",
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
    // The reference solutions the corpus checker loads beside the held-out set.
    "benchmark/reference.mjs",
    // The harness whole: every script and module the jobs that hold a secret run.
    ...filesUnder("benchmark/harness"),
    "compiler/mente-cli/tests/gates.rs",
    "compiler/mente-cli/tests/lints.rs",
    "compiler/mente-cli/tests/register.rs",
    ...[...tracked()].filter((f) => /^compiler\/[^/]+\/src\/overflow_traps\.rs$/.test(f)),
    // Every workspace member's manifest, wherever the member is.
    ...[...workspace().values()].map(({ dir }) => `${dir}/Cargo.toml`),
    // The quick checks before a push, and their map, which this policy reads.
    "tools/quick.mjs",
    QUICK_MAP,
  ];
  for (const f of inputs) {
    assert.ok(tracked().has(f), `${f}, a gate input, is not in the repository`);
    assert.ok(GATE_FILES.some((p) => owns(p, f)), `${f} is a gate input without an owner in .github/CODEOWNERS`);
  }
  assert.ok(owns("/.github/", ".github/workflows/ci.yml") && !owns("/perf/run.mjs", "perf/run.test.mjs") && owns("/Cargo.toml", "Cargo.toml"));
});

/// The map of the quick checks before a push (tools/quick.mjs), which this policy reads as data.
const QUICK_MAP = "tools/quick-checks.json";

/// The files `npm run test:ci` runs, as package.json, a gate file, names them.
const policyEntries = () => String(JSON.parse(read("package.json")).scripts?.["test:ci"] ?? "").match(/[^\s"']+\.mjs/g) ?? [];

/// Every module the workflow policy's tests load, from `entries` through every relative import,
/// static, `export ... from` or a literal `import("...")`, each text read by `readText`; and what
/// the reading cannot follow: an import it cannot read, a computed `import(`, or a bare specifier
/// other than a Node built-in, which would load code no path of the repository names.
export function policyModules(entries, readText) {
  const modules = [];
  const problems = [];
  const todo = [...entries];
  while (todo.length > 0) {
    const file = todo.shift();
    if (modules.includes(file)) continue;
    modules.push(file);
    const found = jsImports(jsTokens(readText(file)));
    for (const line of found.unreadable) problems.push(`${file}:${line}: an import the policy cannot follow`);
    for (const s of found.specifiers) {
      if (/^\.{1,2}\//.test(s.v)) todo.push(path.posix.normalize(path.posix.join(path.posix.dirname(file), s.v)));
      else if (!s.v.startsWith("node:")) problems.push(`${file}:${s.line}: imports ${s.v}, which is neither a file of the repository nor a Node built-in`);
    }
  }
  return { modules, problems };
}

/// What in the policy's own text could start a process or a thread: an import of a module of node
/// that does (child_process, worker_threads, cluster), static or literal, or a reach for one past
/// the imports (getBuiltinModule). The policy starts none (policy-runtime-judge): the collector
/// gives it git's facts, and the run tests run the tree's scripts. Its runtime enforces this from the
/// next step of that sequence on; until then, this reading of the text holds it.
export function judgeSpawnProblems(text) {
  const toks = jsTokens(text);
  const imports = jsImports(toks).specifiers.filter((s) => /^node:(child_process|worker_threads|cluster)$/.test(s.v)).map((s) => `line ${s.line}: imports ${s.v}`);
  const reaches = toks.filter((t) => t.t === "id" && t.v === "getBuiltinModule").map((t) => `line ${t.line}: getBuiltinModule`);
  return [...imports, ...reaches];
}

check("every module the workflow policy's tests load, directly or through another, is a gate file its owner holds", () => {
  // The policy starts no process, nor does any module it loads: the libraries it shares with the
  // scripts the jobs run take what git and cargo report from those scripts (the review of
  // 100e1e5: three of them still imported child_process).
  const judged = policyModules([POLICY], read).modules;
  assert.ok(judged.includes(".github/scripts/lib/build-time-code.mjs") && judged.includes(".github/scripts/lib/watch.mjs") && judged.includes(".github/scripts/lib/commits.mjs"), judged.join(", "));
  assert.deepEqual(judged.flatMap((f) => judgeSpawnProblems(read(f)).map((p) => `${f}: ${p}`)), [], "the policy, or a module it loads, may start a process");
  for (const plant of ['import { spawnSync } from "node:child_process";\n', 'const cp = await import("node:child_process");\n', 'import { Worker } from "node:worker_threads";\n', 'const c = process.getBuiltinModule("node:child_process");\n']) {
    assert.ok(judgeSpawnProblems(plant).length > 0, `the policy may start a process through ${plant.trim()}`);
  }
  // Planted through a module it loads: a library that spawns, reached by an import of the policy's.
  const spawning = new Map([
    [POLICY, 'import { x } from "./lib/x.mjs";\n'],
    [".github/scripts/lib/x.mjs", 'import { execFileSync } from "node:child_process";\nexport const x = () => execFileSync("git", ["ls-files"]);\n'],
  ]);
  const reached = policyModules([POLICY], (f) => spawning.get(f) ?? "").modules;
  assert.ok(reached.flatMap((f) => judgeSpawnProblems(spawning.get(f) ?? "")).length > 0, "a library the policy loads may start a process");
  // The quick check of d471f7e: policy.test.mjs imported tools/quick.mjs, which no pattern of
  // .github/CODEOWNERS held, and one line there turned the strict assertions into no-ops: this
  // policy passed 70 of 70 with a computed key planted in the harness. No module it loads may be
  // one an edit without the owner's review can change.
  const entries = policyEntries();
  assert.ok(entries.includes(".github/scripts/policy.test.mjs") && entries.includes(".github/scripts/yaml.test.mjs"), `npm run test:ci runs ${entries.join(", ")}`);
  const unowned = (modules) => modules.filter((f) => !GATE_FILES.some((p) => owns(p, f)));
  const { modules, problems } = policyModules(entries, read);
  assert.deepEqual(problems, [], "the workflow policy loads a module in a way it cannot follow");
  assert.ok(modules.includes(".github/scripts/lib/yaml.mjs") && modules.includes(".github/scripts/lib/build-time-code.mjs"), modules.join(", "));
  assert.deepEqual(unowned(modules), [], "modules the workflow policy loads that .github/CODEOWNERS does not give its owner");
  // The reviewer's case, planted: a module the policy loads that imports one no pattern holds,
  // through a held module, by a re-export, and by a literal dynamic import; and the loads it cannot
  // follow.
  const planted = new Map([
    [".github/scripts/policy.test.mjs", 'import assert from "node:assert/strict";\nimport { x } from "./lib/x.mjs";\nconst y = await import("../../tools/y.mjs");\n'],
    [".github/scripts/lib/x.mjs", 'export { x } from "../../../site/markdown.mjs";\n'],
    ["site/markdown.mjs", "export const x = 1;\n"],
    ["tools/y.mjs", 'import z from "some-package";\nconst w = await import(`./${"w"}.mjs`);\n'],
  ]);
  const plant = policyModules([".github/scripts/policy.test.mjs"], (f) => planted.get(f) ?? "");
  assert.deepEqual(unowned(plant.modules).sort(), ["site/markdown.mjs", "tools/y.mjs"], "an import of a module without an owner passed");
  assert.deepEqual(plant.problems.sort(), ["tools/y.mjs:1: imports some-package, which is neither a file of the repository nor a Node built-in", "tools/y.mjs:2: an import the policy cannot follow"]);
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

check("tools/quick.mjs runs this policy before a push for a change to any file, since the policy reads files across the whole tree", () => {
  // PR 161: a change to the benchmark's harness alone failed this policy in CI, where tools/quick.mjs
  // had run it only for the gates' files. The policy reads far more than the gates: every module a
  // job that holds a secret runs, the compiler's sources, the conformance programs, the corpus, the
  // spec and the docs, read on 2026-11-16 as some 2,300 paths in every part of the tree. The one
  // trigger that cannot fall behind what it reads is every path, tracked or new.
  // The map is data, read here and never run (the quick check of d471f7e: imported, it could have
  // turned this policy's assertions off).
  const map = JSON.parse(read(QUICK_MAP));
  const policy = map.checks.filter((c) => Array.isArray(c.run) && c.run.join(" ") === "npm run test:ci");
  assert.equal(policy.length, 1, `${QUICK_MAP} runs the workflow policy in one check`);
  const source = Object.hasOwn(map.patterns, policy[0].when) ? map.patterns[policy[0].when] : null;
  assert.equal(typeof source, "string", `${QUICK_MAP}: the workflow policy's check names no pattern the map defines`);
  const when = new RegExp(source);
  const missed = [...tracked(), "a-new-directory/a-new-file.txt", "a-new-file"].filter((f) => !when.test(f));
  assert.deepEqual(missed.slice(0, 20), [], `${missed.length} files whose change tools/quick.mjs would not check against this policy`);
});

check("the required checks are listed once each", () => {
  const names = requiredChecks();
  assert.ok(names.length > 0);
  const twice = names.filter((n, k) => names.indexOf(n) !== k);
  assert.deepEqual(twice, [], `${REQUIRED_FILE} lists these twice`);
});

/// The jobs whose matrix a dispatch's input gives, each with its reason; every other matrix is a
/// literal. Such a job's workflow runs on no pull request, push or queue, and its name's literal
/// start is the start of no required check.
const MATRIX_FROM_INPUTS = new Map([["mutants-full.yml#full", "the shards to run are the dispatch's choice"]]);
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
    if (!MATRIX_FROM_INPUTS.has(key)) {
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
      const allowed = MATRIX_FROM_INPUTS.has(key) && !runsOnPush && prefix.trim() !== "" && ![...requiredChecks()].some((c) => c.startsWith(prefix));
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
  // Every member the root manifest lists, in either quote, comments aside; a glob is refused,
  // since cargo would expand it to members this list never names (lib/build-time-code.mjs, which
  // also holds the list against the packages cargo itself reads).
  const members = listedMembers(read("Cargo.toml"));
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
  // A file a script names is read for imports when it is a module by its name; a file a module
  // imports is read whatever its name, so that the closure misses no module of another kind (the
  // re-check of da58409: a `.cts` imported by a module of a secret job was never read).
  const addScript = (file, imported = false) => {
    if (out.has(file)) return;
    out.add(file);
    if (!imported && !JS_MODULE.test(file)) return;
    // Read from its tokens, so that no spacing or comment hides an import from the closure (the
    // review of b2be87b: `import"../../docs/leak2.mjs"` was past the regular expression before).
    for (const { v } of jsImports(jsTokens(read(file))).specifiers.filter((s) => /^\.{1,2}\//.test(s.v))) {
      const target = path.posix.normalize(path.posix.join(path.posix.dirname(file), v));
      if (files.has(target)) addScript(target, true);
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
  // H-1 of the second review of 4e896ca: a file git writes as binary is refused, not read as a
  // file with no changed line, and the job reads the diff as text whatever .gitattributes says.
  for (const binary of ["Binary files a/compiler/a/src/lib.rs and b/compiler/a/src/lib.rs differ", "GIT binary patch"]) {
    assert.throws(() => parseDiff(`diff --git a/compiler/a/src/lib.rs b/compiler/a/src/lib.rs\nindex 1..2 100644\n${binary}\n`), /git wrote as binary/, binary);
  }
  // The third review of 980965e: a path git quotes is refused, and one with a space keyed without
  // the tab git ends it with.
  assert.throws(() => parseDiff('diff --git "a/compiler/a/src/\\303\\251.rs" "b/compiler/a/src/\\303\\251.rs"\n--- "a/compiler/a/src/\\303\\251.rs"\n+++ "b/compiler/a/src/\\303\\251.rs"\n@@ -1,1 +1,2 @@\n x\n+y\n'), /git quoted/);
  assert.deepEqual([...parseDiff("diff --git a/compiler/a/src/a b.rs b/compiler/a/src/a b.rs\n--- a/compiler/a/src/a b.rs\t\n+++ b/compiler/a/src/a b.rs\t\n@@ -1,1 +1,2 @@\n x\n+y\n").added.keys()], ["compiler/a/src/a b.rs"]);
  const mutantsJob = steps(workflows().find((w) => w.file === "mutants.yml").doc.jobs["in-diff"]).map((s) => String(s.run ?? "")).join("\n");
  // With quotePath off, git quotes only a path holding a quote, a backslash or a control character,
  // which the floor refuses; a path in any other script stays as it is (the fourth review).
  assert.match(mutantsJob, /^\s*git -c core\.quotePath=false diff --text --no-ext-diff --no-textconv --no-renames "origin\/\$\{GITHUB_BASE_REF\}\.\.\.HEAD" > pr\.diff$/m, "the Mutants job reads the diff in a form .gitattributes or the path's characters can change");
  assert.doesNotMatch(mutantsJob.replace(/git (?:-c core\.quotePath=false )?(diff|show) --(text --no-ext-diff --)?no-textconv /g, ""), /\bgit (?:-c \S+ )*(diff|show|log -p)\b/, "the Mutants job reads a diff or a file without --no-textconv");
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
  const outcome = (name, summary, phases, log, diff = "", file = "compiler/x/src/lib.rs") => ({ scenario: { Mutant: { name, file } }, summary, log_path: `log/${name}.log`, diff_path: `diff/${name}.diff`, phase_results: phases, log, diff });
  // A mutant's diff as cargo-mutants writes it: line 12 of lib.rs, `0` replaced by
  // `Default::default()`, columns 9 to 26 of the new line.
  const DIFF = "--- compiler/x/src/lib.rs\n+++ replace f -> Foo with Default::default()\n@@ -11,3 +11,3 @@\n fn f() -> Foo {\n-        0\n+        Default::default()\n }\n";
  // An error as cargo forwards rustc's JSON diagnostic into a mutant's build log: its primary span
  // (line, first column, last column exclusive), and the spans of its children, as rustc
  // structures them; a span may come from a macro, whose call site is its expansion.
  const span = (line, from, to, file = "compiler/x/src/lib.rs", expansion = undefined) => ({ file_name: file, line_start: line, column_start: from, line_end: line, column_end: to, is_primary: true, ...(expansion ? { expansion: { span: expansion, macro_decl_name: "assert!" } } : {}) });
  const diagnostic = (code, spans, children = [], level = "error") => `${JSON.stringify({ reason: "compiler-message", message: { message: `a diagnostic ${code}`, code: code ? { code } : null, level, spans, children: children.map((s) => ({ message: "note", level: "note", spans: [s], children: [] })) } })}\n`;
  const aborting = `${JSON.stringify({ reason: "compiler-message", message: { message: "aborting due to 1 previous error", code: null, level: "error", spans: [], children: [] } })}\n`;
  const build = (status) => ({ phase: "Build", duration: 2.8, process_status: status });
  const test = (status) => ({ phase: "Test", duration: 0.15, process_status: status });
  const FAILED = "\x1b[1;31m        FAIL\x1b[0m [   0.004s] mente-syntax tests::x\n";
  // nextest's own line, in each mutant's log, that it hands the test executables to the sandbox
  // (the form of the logs of pull requests 139 and 140, coloured as nextest writes it).
  const SANDBOXED = "\x1b[1minfo\x1b[0m: using target runner `\x1b[1m/usr/bin/bash /opt/mente-test-sandbox/test-sandbox.sh run\x1b[0m` defined by environment variable `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER`\n";
  const real = [
    outcome("caught", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `*** cargo nextest run\n${SANDBOXED}${FAILED}*** result: Failure(100)\n`),
    outcome("timeout", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `${SANDBOXED}        TIMEOUT [  40.104s] mente-cli::bin/mn x\n`),
    outcome("unviable", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0277", [span(12, 9, 27)])}${aborting}`, DIFF),
    outcome("unviable, inside lines it rewrote whole", "Unviable", [build({ Failure: 101 })], diagnostic("E0308", [span(13, 3, 9)]), "@@ -12,2 +12,3 @@\n-    a\n-    b\n+    c\n+    d\n+    e\n"),
    outcome("unviable, from a macro called in the change", "Unviable", [build({ Failure: 101 })], diagnostic("E0277", [span(62, 9, 73, "/rustc/x/library/core/src/panic.rs", span(12, 9, 27))]), DIFF),
    outcome("unviable, a constant's error with its own note in the change", "Unviable", [build({ Failure: 101 })], diagnostic("E0080", [span(40, 15, 18)], [span(12, 9, 27)]), DIFF),
    outcome("missed", "MissedMutant", [build("Success"), test("Success")], ""),
    { scenario: "Baseline", summary: "Success", phase_results: [], log_path: "log/baseline.log", log: "" },
  ];
  const logs = (list) => (p) => list.find((o) => o.log_path === p)?.log ?? list.find((o) => o.diff_path === p)?.diff ?? "";
  assert.deepEqual(evidence({ outcomes: real }, logs(real)), []);
  const stubbed = [
    outcome("caught by a stub", "CaughtMutant", [build("Success"), test({ Failure: 1 })], "*** cargo nextest run\n*** result: Failure(1)\n"),
    outcome("caught, a failing line but not nextest's exit", "CaughtMutant", [build("Success"), test({ Failure: 1 })], FAILED),
    outcome("caught, no test failed", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `${SANDBOXED}error: no tests to run\n`),
    outcome("caught, by tests that ran outside the sandbox", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `*** cargo nextest run\n${FAILED}*** result: Failure(100)\n`),
    outcome("caught, by tests another runner took", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `info: using target runner \`env\` defined by \`--config\`\n${FAILED}`),
    outcome("caught, the runner's line printed by a test", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `    Starting 3 tests across 1 binary\n${SANDBOXED}${FAILED}`),
    outcome("caught, the runner's line after the failure", "CaughtMutant", [build("Success"), test({ Failure: 100 })], `${FAILED}${SANDBOXED}`),
    outcome("unviable, built", "Unviable", [build("Success")], diagnostic("E0277", [span(12, 9, 27)]), DIFF),
    outcome("unviable, no error", "Unviable", [build({ Failure: 101 })], "*** result: Failure(101)\n", DIFF),
    // An error only as rendered for people, with no JSON diagnostic: the runs pass the flag that
    // forwards them, and a log without one shows no error the reading can place.
    outcome("unviable, only a rendering", "Unviable", [build({ Failure: 101 })], "error[E0277]: the trait bound `Foo: Default` is not satisfied\n  --> compiler/x/src/lib.rs:12:9\n", DIFF),
    // The reviews of 41a54b6 and da58409: an error of the change's own, a constant assertion over the
    // mutated file's text, fails every mutant of it; and errors beside the change, on its line, in
    // another file, after one inside, with no location, or with no diff to place it.
    outcome("unviable, by a const assert elsewhere", "Unviable", [build({ Failure: 101 })], diagnostic("E0080", [span(62, 9, 73, "/rustc/x/library/core/src/panic.rs", span(40, 15, 22))]), DIFF),
    outcome("unviable, beside the change on its line", "Unviable", [build({ Failure: 101 })], diagnostic("E0277", [span(12, 30, 34)]), DIFF),
    outcome("unviable, in another file", "Unviable", [build({ Failure: 101 })], diagnostic("E0277", [span(12, 9, 27, "compiler/x/src/other.rs")]), DIFF),
    outcome("unviable, one error inside and one outside", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0277", [span(12, 9, 27)])}${diagnostic("E0080", [span(40, 15, 22)])}`, DIFF),
    outcome("unviable, an error with no location", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0277", [span(12, 9, 27)])}${diagnostic(null, [])}`, DIFF),
    outcome("unviable, an error the reading cannot parse", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0277", [span(12, 9, 27)])}{"reason":"compiler-message","message":{\n`, DIFF),
    outcome("unviable, no diff", "Unviable", [build({ Failure: 101 })], diagnostic("E0277", [span(12, 9, 27)])),
    outcome("unviable, a note outside the change", "Unviable", [build({ Failure: 101 })], diagnostic("E0080", [span(40, 15, 18)], [span(30, 9, 27)]), DIFF),
    // The review of 47f423d: an error outside the change, after another line's text on its line, as
    // a log interleaves a build's two streams; it counts, as an error no change can claim.
    outcome("unviable, an error after another line's text", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0277", [span(12, 9, 27)])}   Compiling${diagnostic("E0080", [span(40, 15, 22)])}`, DIFF),
    // A note of its own, not the error's, in the change: the review of 75e5eeb.
    outcome("unviable, a separate note in the change", "Unviable", [build({ Failure: 101 })], `${diagnostic("E0080", [span(3, 64, 70)])}${diagnostic(null, [span(12, 9, 27)], [], "note")}`, DIFF),
  ];
  assert.deepEqual(evidence({ outcomes: stubbed }, logs(stubbed)).map((p) => p.split(":")[0]), stubbed.map((o) => o.scenario.Mutant.name));
  // And the review of 75e5eeb's reproductions, from rustc 1.97.1's own diagnostics over the mutated
  // files (.github/scripts/fixtures/): a long merged line whose assertion is the change's, refused;
  // `x -= y` indented 32 spaces, the mutant's own, accepted; and a constant that fails once the
  // mutant's marker is in the file, whose separate note lies at the mutant's line, refused. And the
  // review of 47f423d's: the change's own assertion inside a macro called around the change,
  // refused; and an error on the mutant's own tokens through three macros, accepted.
  // And end to end, on the tool's own output: one real cargo-mutants 27.1.0 run over a tiny crate,
  // its outcomes, logs and diffs as it wrote them. One mutant fails to build by its own error and
  // counts; four fail by the file's own assertion, which fails once the marker is in it, and are
  // refused (the review of 47f423d).
  const e2e = ".github/scripts/fixtures/cargo-mutants-27.1.0";
  const ran = JSON.parse(read(`${e2e}/crate.json`));
  const refused = evidence(JSON.parse(read(`${e2e}/outcomes.json`)), (p) => read(`${e2e}/${p}`)).map((p) => p.slice(0, p.indexOf(": unviable")));
  assert.deepEqual(refused.sort(), [...ran.expect.refused].sort(), "the floor reads a real cargo-mutants run otherwise than its fixture expects");
  const fixtures = JSON.parse(read(".github/scripts/fixtures/unviable-rustc-1.97.1.json"));
  assert.deepEqual(Object.keys(fixtures.cases), ["merged-assertion", "operator-indented", "erroneous-constant", "macro-check", "macro-three-levels"]);
  for (const [id, c] of Object.entries(fixtures.cases)) {
    const one = [outcome(id, "Unviable", [build({ Failure: 101 })], `*** cargo nextest run --no-run\n${c.log}*** result: Failure(101)\n`, c.diff, c.file)];
    assert.equal(evidence({ outcomes: one }, logs(one)).length, c.expect === "refused" ? 1 : 0, `${id}, from real rustc output, is not ${c.expect}: ${c.why}`);
  }
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

check("the policy reads the commit checked out from .git, in each form git keeps it", () => {
  assert.match(gitHead(ROOT), /^[0-9a-f]{40}$/);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "git-head-"));
  try {
    const at = (...p) => path.join(dir, ...p);
    const A = "a".repeat(40);
    const B = "b".repeat(40);
    // Detached, a loose ref, a packed ref, and a worktree whose .git file names its gitdir and its
    // commondir the repository's.
    fs.mkdirSync(at("plain", ".git", "refs", "heads"), { recursive: true });
    fs.writeFileSync(at("plain", ".git", "HEAD"), `${A}\n`);
    assert.equal(gitHead(at("plain")), A, "a detached HEAD");
    fs.writeFileSync(at("plain", ".git", "HEAD"), "ref: refs/heads/x\n");
    fs.writeFileSync(at("plain", ".git", "refs", "heads", "x"), `${B}\n`);
    assert.equal(gitHead(at("plain")), B, "a loose ref");
    fs.rmSync(at("plain", ".git", "refs", "heads", "x"));
    fs.writeFileSync(at("plain", ".git", "packed-refs"), `# pack-refs with: peeled\n${A} refs/heads/xy\n${B} refs/heads/x\n`);
    assert.equal(gitHead(at("plain")), B, "a packed ref, not the one it is a prefix of");
    fs.mkdirSync(at("plain", ".git", "worktrees", "w"), { recursive: true });
    fs.writeFileSync(at("plain", ".git", "worktrees", "w", "HEAD"), "ref: refs/heads/x\n");
    fs.writeFileSync(at("plain", ".git", "worktrees", "w", "commondir"), "../..\n");
    fs.mkdirSync(at("tree"));
    fs.writeFileSync(at("tree", ".git"), `gitdir: ${at("plain", ".git", "worktrees", "w")}\n`);
    assert.equal(gitHead(at("tree")), B, "a worktree's ref, packed in the common directory");
    fs.writeFileSync(at("plain", ".git", "HEAD"), "ref: refs/heads/none\n");
    assert.throws(() => gitHead(at("plain")), /which no ref file holds/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
  // And the policy reads its facts only at the commit checked out, and whole: facts of another
  // commit, none at all, with a part missing or of another type, are refused.
  assert.equal(FACTS.head, gitHead(ROOT));
  const whole = { head: "a".repeat(40), tracked: ["README.md"], base: "b".repeat(40), tags: ["m0a"], baseRoadmap: "# Roadmap\n" };
  assert.deepEqual(readFacts(JSON.stringify(whole), "a".repeat(40)), whole);
  assert.throws(() => readFacts(JSON.stringify(whole), "c".repeat(40)), /not of c+, the commit checked out/);
  assert.throws(() => readFacts(null, "a".repeat(40)), /no facts from the collector/);
  for (const part of ["tracked", "base", "tags", "baseRoadmap"]) {
    const { [part]: _, ...rest } = whole;
    assert.throws(() => readFacts(JSON.stringify(rest), "a".repeat(40)), /not whole/, part);
  }
  assert.throws(() => readFacts(JSON.stringify({ ...whole, tracked: [] }), "a".repeat(40)), /not whole/, "no tracked file");
  for (const text of ["null", "[]", '"facts"', "7"]) assert.throws(() => readFacts(text, "a".repeat(40)), /not an object/, text);
  const misshapen = {
    "a tracked file that is not a string": { tracked: ["README.md", 7] },
    "a tracked file that is an object": { tracked: [{ toString: "README.md" }] },
    "a tracked file with no name": { tracked: ["README.md", ""] },
    "a tag that is not a string": { tags: ["m0a", null] },
    "a tag with no name": { tags: [""] },
    "a base that is a branch's name": { base: "main" },
    "a base that is a short commit id": { base: "b".repeat(12) },
    "a base in capitals": { base: "B".repeat(40) },
    "a base with more after its id": { base: `${"b".repeat(40)}\n` },
    "a base that is not a string": { base: 7 },
    "a base that is a list holding an id": { base: ["b".repeat(40)] },
    "a ROADMAP.md that is not a string": { baseRoadmap: ["# Roadmap\n"] },
  };
  for (const [what, part] of Object.entries(misshapen)) {
    assert.throws(() => readFacts(JSON.stringify({ ...whole, ...part }), "a".repeat(40)), /not whole/, what);
  }
});

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
