# Runs `timeout` with the arguments given, in bash, where the steps run it: the workflow policy's
# test of the form its rule pins runs this script rather than hand bash its text (the review of
# 56c29cf: no -c in a call of the policy's).
timeout "$@"
