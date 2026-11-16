# Sources limits.sh on a runner it knows no limits for, with `echo` standing in to say, as the script
# reaches its refusal of that runner, the flags it left for cargo by then; an earlier refusal says
# none. The run tests (.github/scripts/policy-run.test.mjs) run this script rather than hand bash a text.
echo() {
  case "$*" in *"defines no limits"*) builtin echo "flags=[${CARGO_ENCODED_RUSTFLAGS-unset}]" ;; esac
  builtin echo "$@"
}
. .github/scripts/limits.sh
