# Sources test-sandbox-env.sh and says whether the step went on after it, and with what runner: the
# workflow policy's test of it runs this script rather than hand bash its text (the review of
# 56c29cf: no -c in a call of the policy's).
. .github/scripts/test-sandbox-env.sh
echo "the step went on, its runner ${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER:-none}"
