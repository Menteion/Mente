# Installs cargo-nextest at the one version the workflows run, unless `cargo nextest` is that
# version already (restored from the cache), and then proves that `cargo nextest` is that version.
# A binary of another version, from a cache or the runner's image, is replaced, and one that still
# answers in place of the installed one fails the job here instead of running the tests. ci.yml,
# mutants.yml and mutants-full.yml run it, and key their caches of ~/.cargo/bin/cargo-nextest on
# this file's hash, so that the version is written once, here.
set -euo pipefail
version=0.9.149
# The version `cargo nextest` reports: the word after the binary's name on its first line.
running() { cargo nextest --version 2>/dev/null | awk 'NR == 1 && $1 ~ /^cargo-nextest/ {print $2}'; }
if [ "$(running)" != "$version" ]; then
  # nextest's own crates are not held to the workspace's warnings.
  RUSTFLAGS="" cargo install cargo-nextest --locked --force --version "$version"
fi
if [ "$(running)" != "$version" ]; then
  echo "::error::cargo nextest reports version '$(running)', not $version"
  exit 1
fi
echo "cargo-nextest $version"
