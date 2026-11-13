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
# First, nothing from outside the repository may change what the tests run. No cargo
# configuration file may exist from here up or in CARGO_HOME: one can hold an alias, a runner for
# the test binaries or an [env] table, in more spellings than a check could read, and the
# repository keeps none (the workflow policy refuses a tracked one). No variable may name an alias
# of a cargo command, a runner, the compiler, a wrapper around it or the target, nor set nextest
# but the mutation runs' profile.
limits_home=${CARGO_HOME:-$HOME/.cargo}
limits_dir=$PWD
# On Windows, as this shell names paths: `C:/a` would walk up to `.`, never to `/`.
if command -v cygpath > /dev/null; then
  limits_home=$(cygpath -u "$limits_home")
  limits_dir=$(cygpath -u "$limits_dir")
fi
limits_found=()
for limits_file in "$limits_home/config" "$limits_home/config.toml"; do
  if [ -e "$limits_file" ]; then limits_found+=("$limits_file"); fi
done
while :; do
  # `${dir%/}`: at the root, `/.cargo`, never `//.cargo`, which Windows' shells read as a network path.
  for limits_file in "${limits_dir%/}/.cargo/config" "${limits_dir%/}/.cargo/config.toml"; do
    if [ -e "$limits_file" ]; then limits_found+=("$limits_file"); fi
  done
  limits_up=$(dirname "$limits_dir")
  if [ "$limits_up" = "$limits_dir" ]; then break; fi
  limits_dir=$limits_up
done
if [ "${#limits_found[@]}" -gt 0 ]; then
  echo "::error::a cargo configuration file outside the repository could change what the tests run: ${limits_found[*]}"
  exit 1
fi
# Nor may one change how the tests are built: flags beyond the Rust jobs' `-D warnings`, another
# toolchain, a profile's debug assertions or overflow checks, or a script every bash runs first.
if env | grep -Ei '^(CARGO_ALIAS_|NEXTEST_|CARGO_TARGET_[A-Z0-9_]*_(RUNNER|LINKER|RUSTFLAGS)=|CARGO_BUILD_(RUSTC|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|RUSTDOC|TARGET|RUSTFLAGS|RUSTDOCFLAGS)=|RUSTC=|RUSTDOC=|RUSTC_WRAPPER=|RUSTC_WORKSPACE_WRAPPER=|RUSTFLAGS=|CARGO_ENCODED_RUSTFLAGS=|RUSTDOCFLAGS=|CARGO_ENCODED_RUSTDOCFLAGS=|RUSTUP_TOOLCHAIN=|BASH_ENV=|CARGO_PROFILE_[A-Z0-9_]*_(DEBUG_ASSERTIONS|OVERFLOW_CHECKS)=)' | grep -vx -e 'NEXTEST_PROFILE=mutants' -e 'RUSTFLAGS=-D warnings'; then
  echo "::error::a variable above would change what the tests run"
  exit 1
fi
unset limits_home limits_found limits_dir limits_file limits_up
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
