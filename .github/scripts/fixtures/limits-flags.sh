# Sources limits.sh on a runner it knows no limits for, with `echo` standing in to say, as the script
# reaches its refusal of that runner, the flags it left for cargo by then; an earlier refusal says
# none. The workflow policy's test of the cap of the lints under the mutation runs' profile runs this
# script rather than hand bash its text (the review of 56c29cf: no -c in a call of the policy's).
echo() {
  case "$*" in *"defines no limits"*) builtin echo "flags=[${CARGO_ENCODED_RUSTFLAGS-unset}]" ;; esac
  builtin echo "$@"
}
. .github/scripts/limits.sh
