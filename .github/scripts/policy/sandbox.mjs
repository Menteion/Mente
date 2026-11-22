// How the workflow policy classes jobs for the tests' sandbox (README.md in this directory): the
// testing jobs, the jobs left outside it with their reasons, and the runners a job may name.

import { NIGHTLY_ALARM_JOBS, PAGES_DEPLOY_JOB, isPlain } from "./workflows.mjs";
/// The mutation runs: the jobs that run cargo-mutants over the change's mutants, a pull request's
/// shards and the full run's.
export const MUTATION_JOBS = ["mutants.yml#shard", "mutants-full.yml#full"];
/// The pinned jobs that run the workspace's tests through nextest, under the shared limits, and
/// record their tools: the required Rust jobs, the plan of a pull request's mutation run, which
/// times its tests, and the mutation runs.
export const TESTING_JOBS = ["ci.yml#rust", "mutants.yml#plan", ...MUTATION_JOBS];
/// What names the sandbox as cargo's runner in a step, and starts the count of what ran in it: the
/// testing jobs' limits, which source the second, or the second alone.
export const SANDBOX_SOURCES = [". .github/scripts/limits.sh", ". .github/scripts/test-sandbox-env.sh"];

/// Every required job that runs on a Linux runner is in exactly one of three lists, each with its
/// reason: SANDBOXED_JOBS, whose code of the change runs in the sandbox; LEFT_OUTSIDE, which runs code
/// of the change outside it; and GATE_ONLY, which runs none but the gate's own scripts. A job in
/// none of them fails the check below, so that a job that leaves the sandboxed ones, or a required job
/// added later, is classified where review sees it, not by a pattern of what its steps run.
///
/// LEFT_OUTSIDE: in each, the change's own code produces the evidence its step reads, so a sandbox
/// would keep that code from the runner but not from its own result; what bringing each in would
/// take is in ROADMAP.md, M2, the guards of the audit.
export const LEFT_OUTSIDE = new Map([
  ["ci.yml#bench", "its tests read the base branch's medians from a file of the runner's and write the job's summary"],
  ["ci.yml#tier1-latency", "its test writes the job's summary"],
  ["ci.yml#debuggers", "a script of the change drives the debugger over the binary it built"],
  ["ci.yml#docs", "it runs the change's own node code: the benchmark's checker and its tests, the harness's tests and the documentation tools"],
  ["smt.yml#z3", "it runs mn by its path, which cargo's runner never sees, and writes outside the workspace"],
  ["fuzz.yml#fuzz", "it runs mn by its path, which cargo's runner never sees, and writes outside the workspace"],
  ["mir-mutants.yml#gate", "it runs mn by its path, which cargo's runner never sees, and writes outside the workspace"],
  ["perf.yml#kernels", "it measures mn and its kernels through perf, with the base branch's compiler beside the change's"],
  ["harness.yml#fixtures", "its own sandbox confines agent code, not mn"],
  ["pages.yml#build", "it runs the change's site generator, its tests and `mn explain`, whose output is the site its link check reads"],
]);
/// GATE_ONLY: the required or pinned jobs that run no code of the change but the gate's own scripts
/// under .github/. Each is pinned whole, with every script it runs: an allowlist of exact steps, where a
/// rule that read the steps for tools by their names let through every spelling it did not know
/// (`sh -c`, a quoted name, a path to the tool, node's child_process; the third review of 980965e).
export const GATE_ONLY = new Map([
  ["commits.yml#commits", "it runs .github/scripts/commits.mjs alone, over the commits' text and the pull request's"],
  [PAGES_DEPLOY_JOB, "it deploys the site its build job made from a tag the build proved in main's history, and runs nothing else"],
  ...NIGHTLY_ALARM_JOBS.map((key) => [key, "it runs .github/scripts/nightly-alarm.mjs alone, with the results of the jobs it needs"]),
  ["mutants.yml#aggregate", "it checks out nothing: one step reads the plan's outputs and the shards' result"],
]);
/// The labels a job may run on: each a literal, or a matrix value that is one.
export const RUNNERS = new Set(["ubuntu-24.04", "ubuntu-24.04-arm", "windows-2025"]);
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
