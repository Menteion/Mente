# Runs `timeout` with the arguments given, in bash, where the steps run it. The run tests (.github/scripts/policy-run.test.mjs) run this script rather than hand bash a text.
timeout "$@"
