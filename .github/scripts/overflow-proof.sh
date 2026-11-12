#!/usr/bin/env bash
# Proves, in the job that runs it and under that job's own environment, that every compiler crate
# traps on an integer overflow in a release build: each crate holds
# overflow_traps::test_arithmetic_overflow_panics_in_this_crate, which fails where a profile, a
# package override, a flag or a variable of the job builds that crate without the checks
# Cargo.toml's [profile.release] turns on. The count proves each crate's test ran. Every required
# job that builds in release runs it (.github/gate-commands.txt, .github/scripts/policy.test.mjs).
set -euo pipefail
test=overflow_traps::test_arithmetic_overflow_panics_in_this_crate
log="${RUNNER_TEMP:-/tmp}/overflow-proof.txt"
expected=$(grep -rl "fn test_arithmetic_overflow_panics_in_this_crate" compiler/*/src | wc -l)
cargo test --release --locked --workspace --lib --bins -- --exact "$test" 2>&1 | tee "$log"
passed=$(grep -c "^test $test \.\.\. ok" "$log" || true)
echo "Crates whose release build traps on an overflow: $passed of $expected"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  echo "Crates whose release build traps on an overflow: $passed of $expected" >> "$GITHUB_STEP_SUMMARY"
fi
if [ "$expected" -eq 0 ] || [ "$passed" -ne "$expected" ]; then
  echo "::error::$passed of $expected crates showed overflow checks in this job's release build"
  exit 1
fi
