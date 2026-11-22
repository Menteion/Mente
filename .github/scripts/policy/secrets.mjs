// The jobs that hold a secret, and their steps (README.md in this directory): the held-out set's
// and the broker's, and the shell they run in.

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
export const SECRET_JOBS = new Map([
  ["benchmark.yml#bench", { builtBy: "build", agents: true }],
  ["prelim.yml#prelim", { builtBy: "build", agents: true }],
  ["probe.yml#write", { builtBy: null, agents: false }],
]);
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
/// The one step of a secret job that holds the API key: it starts the broker in the background
/// and ends once the broker listens (benchmark/harness/start-broker.sh, whose node line
/// BROKER_LINE holds).
export const BROKER_STEP = {
  name: "The broker, the one process that holds the API key",
  env: { ANTHROPIC_API_KEY: "${{ secrets.ANTHROPIC_API_KEY }}", BUDGET: "${{ inputs.budget }}" },
  shell: SECRET_SHELL,
  run: "/usr/bin/bash -p benchmark/harness/start-broker.sh",
};
/// The one place each secret may stand in a secret job, everything else refused: the key in the
/// broker's step, the held-out set's token in the held-out step's.
export const HELD_OUT_TOKEN = "${{ secrets.MENTE_BENCH_HELDOUT_TOKEN }}";
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

/// Whether a step is `want` exactly, but for the watch's condition, which a watched job's steps
/// may carry.
export const isStep = (s, want) => {
  const { if: cond, ...rest } = s;
  const { if: wantCond, ...wantRest } = want;
  return JSON.stringify(rest) === JSON.stringify(wantRest) && (cond === wantCond || (wantCond === undefined && cond === "steps.watch.outputs.run != 'false'"));
};

/// Where a job's secret steps end: past its last step that is HELD_OUT_STEP or BROKER_STEP, or 0.
export const secretStepsEnd = (all) => all.reduce((end, s, i) => (isStep(s, HELD_OUT_STEP) || isStep(s, BROKER_STEP) ? i + 1 : end), 0);
