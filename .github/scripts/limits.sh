# The limits the compiler's tests run under where a required check or a mutation run runs them:
# sourced by the step that runs them (ci.yml's Rust jobs, mutants.yml and mutants-full.yml), so
# that they hold for every process the step starts after. They are defined here once. The workflow
# policy (.github/scripts/policy.test.mjs) pins this file, the scripts it runs and those steps by
# their digests, so that a change to any of them is a change to the policy too.
#
# A test that allocates or writes without end would take the runner down with it, as a mutant
# took two shards of the first full mutation run. Each test process may reserve at most 8 GB and,
# on Linux, write a file of at most 2 GB, so that such a test fails instead. A runner has 16 GB
# and nextest runs a test on each of its 4 CPUs, so each runner also bounds them together:
#
# - Linux: ulimit for each process, and a guard on the runner's memory (memory-guard.sh), which
#   stops the process taking the most memory when less than 2 GB is left.
# - Windows: a job object holding the step's shell and every process it starts after, each
#   committing at most 8 GB and all of them together at most 12 GB (job-limits.ps1).
#
# First, nothing from outside the repository's configuration may change what the tests run: no
# alias of a cargo command, in the environment or in a cargo configuration file from here up or in
# CARGO_HOME, and no nextest setting in the environment but the mutation runs' profile.
limits_home=${CARGO_HOME:-$HOME/.cargo}
if command -v cygpath > /dev/null; then limits_home=$(cygpath -u "$limits_home"); fi
limits_configs=(/dev/null)
limits_dir=$PWD
for limits_file in "$limits_home/config" "$limits_home/config.toml"; do
  if [ -f "$limits_file" ]; then limits_configs+=("$limits_file"); fi
done
while :; do
  for limits_file in "$limits_dir/.cargo/config" "$limits_dir/.cargo/config.toml"; do
    if [ -f "$limits_file" ]; then limits_configs+=("$limits_file"); fi
  done
  if [ "$limits_dir" = / ]; then break; fi
  limits_dir=$(dirname "$limits_dir")
done
if env | grep -E '^(CARGO_ALIAS_|NEXTEST_)' | grep -vx 'NEXTEST_PROFILE=mutants' || grep -E '^[[:space:]]*(\[alias\]|alias\.)' "${limits_configs[@]}"; then
  echo "::error::an alias or a nextest setting from outside the repository's configuration would change what the tests run"
  exit 1
fi
unset limits_home limits_configs limits_dir limits_file
case "$RUNNER_OS" in
  Linux)
    ulimit -v 8388608 -f 2097152
    bash .github/scripts/memory-guard.sh &
    ;;
  Windows)
    pwsh -NoProfile -NonInteractive -File .github/scripts/job-limits.ps1 -ProcessId "$(cat "/proc/$$/winpid")" -ProcessBytes 8589934592 -JobBytes 12884901888
    ;;
  *)
    echo "::error::.github/scripts/limits.sh defines no limits for $RUNNER_OS"
    exit 1
    ;;
esac
