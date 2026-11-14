# Records, or checks, what the later steps of a testing job run and read: every file and link of
# cargo's bin directory and of every rustup toolchain's bin and lib directories, the cargo
# configuration in CARGO_HOME, and the repository's .github, .config and rust-toolchain.toml, each
# by its SHA-256. A testing job records them once the tools are installed, before any code of the
# change runs, and checks them at the end of every step that ran such code, where the step's own
# $GITHUB_PATH and $GITHUB_ENV, through which it would reach the next step, must also be empty.
# Every command it runs it names by its absolute path: directories before /usr/bin on the path
# (~/.local/bin, ~/.cargo/bin) are where a stub would go.
#
#   /usr/bin/bash .github/scripts/tools.sh record
#   /usr/bin/bash .github/scripts/tools.sh verify
#
# What it stops, and what it does not: it catches a tool damaged by accident, and a stub, a
# setting or a script a step left in place. It does not stop code of the change that sets out to
# defeat it, which runs with the job's rights, and on GitHub's runners with sudo: a process that
# swaps a tool and restores it between two checks, a write to this script, its record or .git
# before they run, a stub that answers as the tool would, or a toolchain or a stub earlier on the
# path that the record already holds. Against that, on Linux, that code runs in a sandbox, where it
# can write none of these (.github/scripts/test-sandbox.sh); on Windows the guard is still the
# review of the change (docs/design-review.md, untrusted-test-isolation).
# The workflow policy (.github/scripts/policy.test.mjs) pins this script and the steps that run
# it, and runs it against a changed tool.
set -euo pipefail
posix() { if [ -x /usr/bin/cygpath ]; then /usr/bin/cygpath -u "$1"; else printf '%s\n' "$1"; fi; }
cargo_home=$(posix "${CARGO_HOME:-$HOME/.cargo}")
toolchains=$(posix "${RUSTUP_HOME:-$HOME/.rustup}")/toolchains
record=$(posix "$RUNNER_TEMP")/tools.sha256
# The paths it reads, of the type the arguments name.
paths() {
  /usr/bin/find "$cargo_home/bin" "$@"
  /usr/bin/find "$cargo_home" -maxdepth 1 -name 'config*' "$@"
  if [ -d "$toolchains" ]; then /usr/bin/find "$toolchains" \( -path '*/bin/*' -o -path '*/lib/*' \) "$@"; fi
  /usr/bin/find .github .config rust-toolchain.toml "$@"
}
# Every link by its target and every file by its digest, in one order on every runner.
listing() {
  paths -type l -print0 | LC_ALL=C /usr/bin/sort -z | while IFS= read -r -d '' link; do
    printf 'link %s -> %s\n' "$link" "$(/usr/bin/readlink "$link")"
  done
  paths -type f -print0 | LC_ALL=C /usr/bin/sort -z | /usr/bin/xargs -0 -r /usr/bin/sha256sum
}
case "${1:-}" in
  record)
    listing > "$record"
    echo "Recorded $(/usr/bin/wc -l < "$record") files, digest $(/usr/bin/sha256sum < "$record" | /usr/bin/cut -c1-64)"
    ;;
  verify)
    if [ ! -s "$record" ]; then
      echo "::error::no record of the tools: tools.sh record runs before the code of the change"
      exit 1
    fi
    failed=0
    if ! listing | /usr/bin/diff "$record" -; then
      echo "::error::a tool, a setting or a script changed since it was recorded: a step that ran code of the change replaced, added or removed it"
      failed=1
    fi
    for file in "${GITHUB_PATH:-}" "${GITHUB_ENV:-}"; do
      if [ -n "$file" ] && [ -s "$(posix "$file")" ]; then
        echo "::error::this step set the path or the environment of the steps after it:"
        /usr/bin/cat "$(posix "$file")"
        failed=1
      fi
    done
    exit "$failed"
    ;;
  *)
    echo "usage: /usr/bin/bash .github/scripts/tools.sh record|verify"
    exit 2
    ;;
esac
