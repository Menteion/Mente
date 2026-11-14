# Sourced by a step that runs code of the change, in a job that set up the sandbox
# (.github/scripts/test-sandbox.sh setup): from here on, on Linux, cargo, nextest and
# cargo-mutants hand every executable they run to the sandbox, and the count of what ran in it,
# which the step checks at its end (test-sandbox.sh ran), starts at none. Without a sandbox set up
# the step fails: nothing runs outside one. On Windows it changes nothing, since Windows has no
# sandbox yet (docs/design-review.md, untrusted-test-isolation).
# limits.sh sources it for the testing jobs; a job that runs its tests through cargo alone sources
# it by itself. The workflow policy (.github/scripts/policy.test.mjs) pins it.
if ! test_sandbox_env=$(/usr/bin/bash .github/scripts/test-sandbox.sh env); then
  echo "$test_sandbox_env"
  exit 1
fi
eval "$test_sandbox_env"
unset test_sandbox_env
