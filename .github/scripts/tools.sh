# Records, or checks, the tools the later steps of a testing job run: every file of cargo's bin
# directory and of every rustup toolchain's bin directory, by its SHA-256. The tests and the
# compiler are code of the change, and run with the job's rights: one could put a stub in place of
# cargo-nextest, cargo or rustc, or add a directory to the path or a variable to the environment of
# the steps after it, and a later step, or every mutant of a mutation run, would then check
# nothing. A testing job records the tools once they are installed, before any code of the change
# runs, and checks them at the end of every step that ran such code, where the step's own
# $GITHUB_PATH and $GITHUB_ENV, through which it would reach the next step, must also be empty.
#
#   bash .github/scripts/tools.sh record
#   bash .github/scripts/tools.sh verify
#
# This catches code that changes the tools; code that also rewrites this script or its record can
# defeat it, as it can any check that runs with the same rights, and the review of the change is
# the guard there. The workflow policy (.github/scripts/policy.test.mjs) pins this script and the
# steps that run it, and runs it against a changed tool.
set -euo pipefail
posix() { if command -v cygpath > /dev/null; then cygpath -u "$1"; else printf '%s\n' "$1"; fi; }
cargo_bin=$(posix "${CARGO_HOME:-$HOME/.cargo}")/bin
toolchains=$(posix "${RUSTUP_HOME:-$HOME/.rustup}")/toolchains
record=$(posix "$RUNNER_TEMP")/tools.sha256
# Every file of the bin directories, by path and digest, in one order on every runner.
listing() {
  {
    find "$cargo_bin" -type f
    if [ -d "$toolchains" ]; then find "$toolchains" -path '*/bin/*' -type f; fi
  } | LC_ALL=C sort | while IFS= read -r file; do sha256sum "$file"; done
}
case "${1:-}" in
  record)
    listing > "$record"
    echo "Recorded $(wc -l < "$record") tools, digest $(sha256sum < "$record" | cut -c1-64)"
    ;;
  verify)
    if [ ! -s "$record" ]; then
      echo "::error::no record of the tools: tools.sh record runs before the code of the change"
      exit 1
    fi
    failed=0
    if ! listing | diff "$record" -; then
      echo "::error::a tool changed since it was recorded: a step that ran code of the change replaced or added it"
      failed=1
    fi
    for file in "${GITHUB_PATH:-}" "${GITHUB_ENV:-}"; do
      if [ -n "$file" ] && [ -s "$(posix "$file")" ]; then
        echo "::error::this step set the path or the environment of the steps after it:"
        cat "$(posix "$file")"
        failed=1
      fi
    done
    exit "$failed"
    ;;
  *)
    echo "usage: bash .github/scripts/tools.sh record|verify"
    exit 2
    ;;
esac
