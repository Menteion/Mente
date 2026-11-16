# Sources test-sandbox-env.sh and says whether the step went on after it, and with what runner.
# The run tests (.github/scripts/policy-run.test.mjs) run this script rather than hand bash a text.
. .github/scripts/test-sandbox-env.sh
echo "the step went on, its runner ${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER:-none}"
