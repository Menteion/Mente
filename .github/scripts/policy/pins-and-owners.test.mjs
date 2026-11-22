// The workflow policy: pins and owners (README.md in this directory). The scripts and jobs the
// policy pins, the gate files CODEOWNERS gives the owner, and the modules the policy itself loads.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { CJK_RANGES, badSubject, baseChangedSincePush, cjkLines, problems, pullRequestOf } from "../lib/commits.mjs";
import { globToRegExp } from "../lib/watch.mjs";
import { JS_MODULE, jsImports, jsTokens } from "./js.mjs";
import { GATE_FILES, PINNED_FILES, PINNED_JOBS, POLICY_FILES, POLICY_LIST, POLICY_LIST_FILE, QUICK_MAP, isPolicy, jobDigest, owns } from "./pins.mjs";
import { FACTS, ROOT, digest, filesUnder, gitHead, read, readFacts, tracked, workspace } from "./repo.mjs";
import { LEFT_OUTSIDE } from "./sandbox.mjs";
import { BROKER_STEP, SECRET_JOBS, isStep, secretStepsEnd } from "./secrets.mjs";
import { PERMISSION, TOOLCHAIN_STEP } from "./toolchain.mjs";
import { repoPaths, scriptTexts, steps, workflows } from "./workflows.mjs";

/// Every check of this module counts itself when it runs, and the module fails when fewer ran than
/// FLOOR, the number of checks below: a skip or a filter of the test runner cannot leave the
/// policy passing with less of it checked. test_the_documentation_job_runs_the_workflow_policy keeps
/// each module's FLOOR equal to the checks it declares.
const FLOOR = 4;
let ran = 0;
function check(name, fn) {
  test(name, (...args) => {
    ran++;
    return fn(...args);
  });
}
after(() => {
  assert.ok(ran >= FLOOR, `${ran} of the policy's ${FLOOR} checks of pins-and-owners.test.mjs ran: a skip or a filter left the rest out`);
});
/// A file that runs as a script: a module by its name, or a shell or PowerShell script.
const isScript = (f) => JS_MODULE.test(f) || /\.(sh|ps1)$/.test(f);
/// A script a pinned job runs that PINNED_FILES must hold: every one under .github/scripts, whatever
/// its name, but the policy's own files, exactly those its list names (the review of f217d1e: a
/// directory prefix exempted every script under it, and a name with a dot in it, such as a
/// dotfile's, passed the pattern of names this read).
const toPin = (f) => f.startsWith(".github/scripts/") && isScript(f) && !isPolicy(f);
/// The directory of the policy's files.
const POLICY_DIR = ".github/scripts/policy/";

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
  assert.deepEqual(changed, [], "gates the policy pins changed; their digests now are the ones listed, which PINNED_FILES and PINNED_JOBS in .github/scripts/policy/pins.mjs take in the same pull request");
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
    for (const f of repoPaths(scriptTexts(w.doc, read))) if (toPin(f)) pending.push([key, f]);
  }
  for (const f of PINNED_FILES.keys()) pending.push(["PINNED_FILES", f]);
  // The policy's own files, exactly those its list names, are exempt; any other script under its
  // directory that a pinned job runs is pinned like every other (the review of f217d1e: `bash
  // .github/scripts/policy/helper.sh` in a pinned job passed unpinned).
  for (const f of POLICY_FILES) assert.ok(isPolicy(f) && !toPin(f), `${f}, a file of the policy, taken for a script to pin`);
  for (const f of [`${POLICY_DIR}helper.sh`, `${POLICY_DIR}x.mjs`, `${POLICY_DIR}.secrets-and-broker.test.mjs`, `${POLICY_DIR}sub/a.ps1`, `${POLICY_DIR}repo.mjs.sh`]) {
    assert.ok(toPin(f) && !isPolicy(f), `${f}, run by a pinned job, escapes its pin`);
  }
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
/// The files a secret job runs up to the end of its secret steps, the held-out step and the
/// broker's, with every module those import: the scripts and modules, not the data they read.
export function secretPrefixFiles(doc, job) {
  const prefix = { ...job, steps: steps(job).slice(0, secretStepsEnd(steps(job))) };
  const files = [...repoPaths(scriptTexts(doc, prefix))].filter((f) => isScript(f) && !isPolicy(f));
  // The broker, whose path start-broker.sh builds from the workspace's, which no reading of a path
  // follows.
  return prefix.steps.some((s) => isStep(s, BROKER_STEP)) ? [...files, PERMISSION.broker.script] : files;
}

/// The owner of the gate files, and the gate files, as the patterns of .github/CODEOWNERS: what
/// decides whether a change may merge, and the rules and philosophy it enforces, which a pull
/// request could otherwise weaken unseen.
const OWNER = "@Dancraft3746";

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

/// Whether node's glob, as --test reads it, passes over `file` for `pattern`: it matches no name
/// that starts with a dot where the pattern has a wildcard (the review of f217d1e: a module renamed
/// to a dotfile was counted as run, and node ran none of it).
const dotUnderWildcard = (pattern, file) => {
  const parts = file.split("/");
  return pattern.split("/").some((part, k) => part.includes("*") && parts[k]?.startsWith("."));
};

/// The files `npm run test:ci` runs, as package.json, a gate file, names them: a glob, such as the
/// policy's modules, read as the tracked files it matches, as node --test reads it.
const policyEntries = () =>
  (String(JSON.parse(read("package.json")).scripts?.["test:ci"] ?? "").match(/[^\s"']+\.mjs/g) ?? []).flatMap((e) =>
    e.includes("*") ? [...tracked()].filter((f) => globToRegExp(e).test(f) && !dotUnderWildcard(e, f)).sort() : [e],
  );

/// What keeps the policy's directory from holding exactly its listed files (`list`, its
/// modules.json; `files`, the tracked files; `readText`, a file's text): each listed module and
/// library tracked; nothing else there but README.md and the list; no dotfile; every module a
/// .test.mjs and no library one; and the modules' FLOORs, each its module's checks, adding up to the
/// list's total (the review of f217d1e: a module renamed to plain .mjs, or to a dotfile, left every
/// count passing).
function policyDirProblems(list, files, readText) {
  const problems = [];
  const listed = new Set([...list.modules, ...list.libraries, POLICY_LIST_FILE, `${POLICY_DIR}README.md`]);
  const inDir = files.filter((f) => f.startsWith(POLICY_DIR));
  for (const f of inDir) {
    if (f.slice(POLICY_DIR.length).split("/").some((s) => s.startsWith("."))) problems.push(`${f} is a dotfile, which node's glob does not run`);
    else if (!listed.has(f)) problems.push(`${f} lies in the policy's directory but is none of the files ${POLICY_LIST_FILE} lists`);
  }
  for (const f of listed) if (!inDir.includes(f)) problems.push(`${f}, listed, is not tracked`);
  for (const f of list.modules) if (!f.endsWith(".test.mjs")) problems.push(`${f}, listed as a module, is no .test.mjs node runs`);
  for (const f of list.libraries) if (/\.test\.[^/]*$/.test(f)) problems.push(`${f}, listed as a library, is a test file`);
  let total = 0;
  for (const f of list.modules.filter((m) => inDir.includes(m))) {
    const text = readText(f);
    const checks = text.split("\n").filter((l) => l.startsWith('check("')).length;
    const floor = Number(/^const FLOOR = (\d+);$/m.exec(text)?.[1]);
    if (floor !== checks) problems.push(`${f}'s FLOOR is ${floor}, not its ${checks} checks`);
    total += checks;
  }
  if (!(Number.isInteger(list.checks) && list.checks === total)) problems.push(`the policy's modules hold ${total} checks, not the ${list.checks} its list pins`);
  return problems;
}

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
  const judged = policyModules(policyEntries().filter(isPolicy), read).modules;
  assert.ok(judged.includes(".github/scripts/lib/build-time-code.mjs") && judged.includes(".github/scripts/lib/watch.mjs") && judged.includes(".github/scripts/lib/commits.mjs"), judged.join(", "));
  assert.deepEqual(judged.flatMap((f) => judgeSpawnProblems(read(f)).map((p) => `${f}: ${p}`)), [], "the policy, or a module it loads, may start a process");
  for (const plant of ['import { spawnSync } from "node:child_process";\n', 'const cp = await import("node:child_process");\n', 'import { Worker } from "node:worker_threads";\n', 'const c = process.getBuiltinModule("node:child_process");\n']) {
    assert.ok(judgeSpawnProblems(plant).length > 0, `the policy may start a process through ${plant.trim()}`);
  }
  // Planted through a module it loads: a library that spawns, reached by an import of the policy's.
  const spawning = new Map([
    [`${POLICY_DIR}x.test.mjs`, 'import { x } from "../lib/x.mjs";\n'],
    [".github/scripts/lib/x.mjs", 'import { execFileSync } from "node:child_process";\nexport const x = () => execFileSync("git", ["ls-files"]);\n'],
  ]);
  const reached = policyModules([`${POLICY_DIR}x.test.mjs`], (f) => spawning.get(f) ?? "").modules;
  assert.ok(reached.flatMap((f) => judgeSpawnProblems(spawning.get(f) ?? "")).length > 0, "a library the policy loads may start a process");
  // The quick check of d471f7e: the policy imported tools/quick.mjs, which no pattern of
  // .github/CODEOWNERS held, and one line there turned the strict assertions into no-ops: this
  // policy passed 70 of 70 with a computed key planted in the harness. No module it loads may be
  // one an edit without the owner's review can change.
  const entries = policyEntries();
  // The policy's directory holds exactly its listed files, and npm run test:ci runs every listed
  // module, as node reads its glob, and nothing else there.
  const files = [...tracked()];
  assert.deepEqual(policyDirProblems(POLICY_LIST, files, read), []);
  assert.deepEqual(entries.filter((f) => f.startsWith(POLICY_DIR)), [...POLICY_LIST.modules].sort(), `npm run test:ci runs ${entries.join(", ")}`);
  // Planted, as the review of f217d1e found them: each must be refused.
  const moduleA = POLICY_LIST.modules[0];
  const renamed = (to) => files.map((f) => (f === moduleA ? to : f));
  const readAs = (to) => (f) => read(f === to ? moduleA : f);
  for (const [planted, what] of [
    [{ list: POLICY_LIST, files: renamed(moduleA.replace(/\.test\.mjs$/, ".mjs")), text: readAs(moduleA.replace(/\.test\.mjs$/, ".mjs")) }, "a module renamed to plain .mjs"],
    [{ list: POLICY_LIST, files: renamed(`${POLICY_DIR}.${moduleA.slice(POLICY_DIR.length)}`), text: readAs(`${POLICY_DIR}.${moduleA.slice(POLICY_DIR.length)}`) }, "a module renamed to a dotfile"],
    [{ list: POLICY_LIST, files: [...files, `${POLICY_DIR}extra.test.mjs`], text: read }, "a test module no list names"],
    [{ list: { ...POLICY_LIST, modules: POLICY_LIST.modules.slice(1) }, files: files.filter((f) => f !== moduleA), text: read }, "a module dropped from the list and the tree"],
    [{ list: { ...POLICY_LIST, modules: [...POLICY_LIST.modules, `${POLICY_DIR}ghost.test.mjs`] }, files, text: read }, "a module listed that is not there"],
    [{ list: { ...POLICY_LIST, checks: POLICY_LIST.checks - 1 }, files, text: read }, "the total lowered"],
  ]) {
    assert.ok(policyDirProblems(planted.list, planted.files, planted.text).length > 0, `${what} passed`);
  }
  assert.ok(dotUnderWildcard(".github/scripts/policy/*.test.mjs", ".github/scripts/policy/.x.test.mjs") && !dotUnderWildcard(".github/scripts/policy/*.test.mjs", ".github/scripts/policy/x.test.mjs"), "node's glob read as matching a dotfile");
  assert.ok(entries.includes(".github/scripts/yaml.test.mjs"), `npm run test:ci runs ${entries.join(", ")}`);
  const unowned = (modules) => modules.filter((f) => !GATE_FILES.some((p) => owns(p, f)));
  const { modules, problems } = policyModules(entries, read);
  assert.deepEqual(problems, [], "the workflow policy loads a module in a way it cannot follow");
  assert.ok(modules.includes(".github/scripts/lib/yaml.mjs") && modules.includes(".github/scripts/lib/build-time-code.mjs"), modules.join(", "));
  assert.deepEqual(unowned(modules), [], "modules the workflow policy loads that .github/CODEOWNERS does not give its owner");
  // The reviewer's case, planted: a module the policy loads that imports one no pattern holds,
  // through a held module, by a re-export, and by a literal dynamic import; and the loads it cannot
  // follow.
  const planted = new Map([
    [`${POLICY_DIR}x.test.mjs`, 'import assert from "node:assert/strict";\nimport { x } from "../lib/x.mjs";\nconst y = await import("../../../tools/y.mjs");\n'],
    [".github/scripts/lib/x.mjs", 'export { x } from "../../../site/markdown.mjs";\n'],
    ["site/markdown.mjs", "export const x = 1;\n"],
    ["tools/y.mjs", 'import z from "some-package";\nconst w = await import(`./${"w"}.mjs`);\n'],
  ]);
  const plant = policyModules([`${POLICY_DIR}x.test.mjs`], (f) => planted.get(f) ?? "");
  assert.deepEqual(unowned(plant.modules).sort(), ["site/markdown.mjs", "tools/y.mjs"], "an import of a module without an owner passed");
  assert.deepEqual(plant.problems.sort(), ["tools/y.mjs:1: imports some-package, which is neither a file of the repository nor a Node built-in", "tools/y.mjs:2: an import the policy cannot follow"]);
});

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
