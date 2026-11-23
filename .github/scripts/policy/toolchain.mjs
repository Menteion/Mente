// What may run before the toolchain, in every required or pinned job (README.md in this
// directory): the check that nothing of the change can run where cargo builds, and what precedes it.

import { read } from "./repo.mjs";
import { SHELLS, WATCH_FORMS, isPlain, normalize, stepKey } from "./workflows.mjs";
/// The toolchain step of every job that builds: before rustup or cargo reads anything of the
/// workspace, the check that rust-toolchain.toml names a release alone and no cargo configuration
/// is there; then the toolchain; then cargo's reading of the manifests and the lock. So a change
/// whose code would run where cargo builds fails each such job before any of it is built.
export const TOOLCHAIN_STEP = "node .github/scripts/build-time-code.mjs --before-toolchain\nrustup toolchain install\nnode .github/scripts/build-time-code.mjs\n";

/// What may come before TOOLCHAIN_STEP, each step exactly, with no key beyond those named: the
/// checkout, with no input but these; node, at the release the jobs use and with no cache of its
/// own, which setup-node would restore from the pull request's own scope once package.json named
/// a package manager; the watch; and, each in the one job named, the gate's own steps that read
/// nothing of the change's but the scripts the policy pins.
export const CHECKOUT_INPUTS = [{ "persist-credentials": false }, { "fetch-depth": 0, "persist-credentials": false }];
/// How the broker, the one process that holds the API key, is started, and the one Node every job
/// sets up, whose permission model the broker's flags rely on (benchmark/harness/permission.json):
/// read here, and by the harness's test of the escapes.
export const PERMISSION = JSON.parse(read("benchmark/harness/permission.json"));
export const NODE_INPUTS = { "node-version": PERMISSION.node, "package-manager-cache": false };
/// The check that the tree a job's gate commands judge is the commit's (.github/scripts/clean-tree.sh;
/// the brief clean-tree-around-every-test-run), which every step of a gate command runs around it.
export const CLEAN_TREE = "/usr/bin/bash .github/scripts/clean-tree.sh";
/// The Documentation job's first run: the collector, then the policy, its run tests and the YAML
/// reader's (policy-runtime-judge), checked around for the tree they read, as BEFORE_TOOLCHAIN reads
/// a step's script, trimmed.
export const POLICY_RUN = `${CLEAN_TREE}\nnode .github/scripts/policy-collect.mjs\nnode --test ".github/scripts/policy/*.test.mjs" .github/scripts/policy-run.test.mjs .github/scripts/yaml.test.mjs\n${CLEAN_TREE}`;
export const BEFORE_TOOLCHAIN = new Map([
  ["bash .github/scripts/publish-tag.sh", { job: "pages-publish.yml#build", env: { TAG: "${{ inputs.tag }}" }, why: "the milestone tag the run was given, checked out once its commit is proven in main's history, before anything of it runs" }],
  [POLICY_RUN, { job: "ci.yml#docs", why: "the policy, first in the Documentation job: its collector, then its tests" }],
  [`${CLEAN_TREE}\nnode .github/scripts/action-pins.mjs\n${CLEAN_TREE}`, { job: "ci.yml#docs", why: "the check of the actions' pins against their releases" }],
  ['echo "RUSTUP_HOME=$RUNNER_TEMP/rustup" >> "$GITHUB_ENV"', { job: "harness.yml#fixtures", why: "the harness's toolchain, put outside the home its sandbox hides" }],
]);
export const sameKeys = (o, keys) => isPlain(o) && JSON.stringify(Object.keys(o).sort()) === JSON.stringify([...keys].sort());
/// Whether a step holds these keys and no other, the watch's condition aside.
export const stepKeysAre = (s, keys) => sameKeys(Object.fromEntries(Object.entries(s).filter(([k]) => k !== "if")), keys);

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
  if (at < 0) return [`${key}: none of its steps is TOOLCHAIN_STEP in .github/scripts/policy/toolchain.mjs, the check that nothing of the change can run where cargo builds, before rustup installs the toolchain`];
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
    } else {
      // A step of BEFORE_TOOLCHAIN: its name and script alone, or with exactly the variables its
      // entry names, when it names any.
      const allowed = BEFORE_TOOLCHAIN.get(String(s.run ?? "").trim());
      const keysOk = allowed?.env === undefined ? stepKeysAre(s, ["name", "run"]) : stepKeysAre(s, ["name", "env", "run"]) && JSON.stringify(s.env) === JSON.stringify(allowed.env);
      if (!keysOk || allowed?.job !== key) found.push(`${where} is none of the steps BEFORE_TOOLCHAIN names for this job, with its name, its script and the variables its entry names alone: ${JSON.stringify(s)}`);
    }
  }
  if (checkouts !== 1) found.push(`${key} checks out ${checkouts} times before the check, not once`);
  return found;
}
