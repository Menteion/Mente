# Installs cargo-nextest from its release archive for this runner, checked against the SHA-256
# written here, into cargo's bin directory, and proves that the binary on the path reports the
# version. Nothing comes from a cache: an entry another job of the same pull request saved, or one
# saved after the tests changed the binary, would be trusted on its --version alone. ci.yml runs
# that binary itself; the mutation runs reach it through cargo, whose aliases limits.sh refuses.
# ci.yml, mutants.yml and mutants-full.yml run this script, and the workflow policy
# (.github/scripts/policy.test.mjs) pins it by its digest, so that the version and the archives'
# digests are written once, here, and change with the policy.
set -euo pipefail
version=0.9.149
case "$RUNNER_OS/$RUNNER_ARCH" in
  Linux/X64) target=x86_64-unknown-linux-gnu sha256=32d1d8c0567b7a80bee2bad8bb09125c846da96bc009b2de11ec08557290c770 ;;
  Linux/ARM64) target=aarch64-unknown-linux-gnu sha256=6f82e375187cf3a1f3e336879a51940c315a5d0ce7253f7e517ad28295b74189 ;;
  Windows/X64) target=x86_64-pc-windows-msvc sha256=8e7609fffb5a938c9b48d8788a89699fc83c39d07a9d9dbc61554f661b51d9d8 ;;
  *)
    echo "::error::no cargo-nextest archive is pinned for $RUNNER_OS/$RUNNER_ARCH"
    exit 1
    ;;
esac
# Paths as this shell names them: on Windows, GNU tar reads `D:/...` as a remote host.
posix() { if command -v cygpath > /dev/null; then cygpath -u "$1"; else printf '%s\n' "$1"; fi; }
bin=$(posix "${CARGO_HOME:-$HOME/.cargo}")/bin
archive=$(posix "$RUNNER_TEMP")/cargo-nextest-$version-$target.tar.gz
curl -fsSL --retry 3 -o "$archive" "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-$version/cargo-nextest-$version-$target.tar.gz"
echo "$sha256  $archive" | sha256sum -c -
mkdir -p "$bin"
tar -xzf "$archive" -C "$bin"
# The version the binary on the path reports: the word after its name on the first line.
running=$(cargo-nextest nextest --version 2>/dev/null | awk 'NR == 1 && $1 ~ /^cargo-nextest/ {print $2}')
if [ "$running" != "$version" ]; then
  echo "::error::cargo-nextest on the path reports version '$running', not $version"
  exit 1
fi
echo "cargo-nextest $version from $target, sha256 $sha256"
