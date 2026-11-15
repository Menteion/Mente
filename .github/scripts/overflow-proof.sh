#!/usr/bin/env bash
# Proves, in the job that runs it and under that job's own environment, that every compiler crate
# traps on an integer overflow in a release build: each crate holds
# overflow_traps::test_arithmetic_overflow_panics_in_this_crate, which fails where a profile, a
# package override, a flag or a variable of the job builds that crate without the checks
# Cargo.toml's [profile.release] turns on. Each crate's tests run on their own and their output is
# read apart, so that what one crate's tests print never counts for another: every crate must show
# its own test passing, once. Every required job that builds in release runs it
# (.github/gate-commands.txt, .github/scripts/policy.test.mjs).
set -euo pipefail
test=overflow_traps::test_arithmetic_overflow_panics_in_this_crate
log="${RUNNER_TEMP:-/tmp}/overflow-proof.txt"
expected=0
passed=0
while IFS= read -r file; do
  dir=${file%%/src/*}
  name=$(sed -n 's/^name = "\([A-Za-z0-9_-]*\)"$/\1/p' "$dir/Cargo.toml" | head -n 1)
  if [ -z "$name" ]; then
    echo "::error::$dir/Cargo.toml names no package"
    exit 1
  fi
  expected=$((expected + 1))
  # The targets that hold unit tests: the library, the binaries, or both.
  targets=()
  if [ -f "$dir/src/lib.rs" ]; then targets+=(--lib); fi
  if [ -f "$dir/src/main.rs" ] || [ -d "$dir/src/bin" ]; then targets+=(--bins); fi
  status=0
  cargo test --release --locked -p "$name" "${targets[@]}" -- --exact "$test" > "$log" 2>&1 || status=$?
  cat "$log"
  if [ "$status" -ne 0 ]; then
    echo "::error::the tests of $name failed in this job's release build"
    exit 1
  fi
  if [ "$(grep -c "^test $test \.\.\. ok" "$log" || true)" -eq 1 ]; then
    passed=$((passed + 1))
  else
    echo "::error::$name does not show its own overflow test passing, once, in this job's release build"
  fi
done < <(grep -rl "fn test_arithmetic_overflow_panics_in_this_crate" compiler/*/src | LC_ALL=C sort)
echo "Crates whose release build traps on an overflow: $passed of $expected"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  echo "Crates whose release build traps on an overflow: $passed of $expected" >> "$GITHUB_STEP_SUMMARY"
fi
if [ "$expected" -eq 0 ] || [ "$passed" -ne "$expected" ]; then
  echo "::error::$passed of $expected crates showed overflow checks in this job's release build"
  exit 1
fi
