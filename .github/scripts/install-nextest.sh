# Installs cargo-nextest at the one version the workflows run, unless the cargo-nextest binary on
# the path is that version already (restored from the cache), and then proves that it is. A binary
# of another version, from a cache or the runner's image, is replaced, and one that still answers
# in place of the installed one fails the job here instead of running the tests. ci.yml runs that
# binary itself, and the mutation runs reach it through cargo, whose aliases limits.sh refuses.
# ci.yml, mutants.yml and mutants-full.yml run this script, and key their caches of
# ~/.cargo/bin/cargo-nextest on its hash, so that the version is written once, here.
set -euo pipefail
version=0.9.149
# The version the binary reports: the word after its name on the first line.
running() { cargo-nextest nextest --version 2>/dev/null | awk 'NR == 1 && $1 ~ /^cargo-nextest/ {print $2}'; }
if [ "$(running)" != "$version" ]; then
  # nextest's own crates are not held to the workspace's warnings.
  RUSTFLAGS="" cargo install cargo-nextest --locked --force --version "$version"
fi
if [ "$(running)" != "$version" ]; then
  echo "::error::cargo-nextest reports version '$(running)', not $version"
  exit 1
fi
echo "cargo-nextest $version"
