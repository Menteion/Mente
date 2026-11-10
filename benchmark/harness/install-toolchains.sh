#!/usr/bin/env bash
# Installs the agent benchmark's Rust and Odin, each pinned in toolchains.json (benchmark/README.md,
# "The harness"): Rust through the runner's rustup, Odin from its GitHub release, checked against
# the release asset's SHA-256 before it is unpacked. It runs in CI on x86-64 Linux, never on a
# developer's machine.
#
#   bash benchmark/harness/install-toolchains.sh <directory for Odin>
#
# It prints the path of `odin`, and writes it as the step output `odin` when GITHUB_OUTPUT is set.
set -euo pipefail

dest="${1:?usage: install-toolchains.sh <directory for Odin>}"
here="$(cd "$(dirname "$0")" && pwd)"
pin() { node -e 'const pins = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")); console.log(process.argv[2].split(".").reduce((o, k) => o[k], pins));' "$here/toolchains.json" "$1"; }

rust="$(pin rust.toolchain)"
rustup toolchain install "$rust" --profile minimal
RUSTUP_TOOLCHAIN="$rust" rustc --version

release="$(pin odin.release)"
asset="$(pin odin.linux-amd64.asset)"
sha="$(pin odin.linux-amd64.sha256)"
archive="$(mktemp -d)/$asset"
curl --fail --silent --show-error --location --retry 3 --output "$archive" \
  "https://github.com/odin-lang/Odin/releases/download/$release/$asset"
echo "$sha  $archive" | sha256sum --check --strict
mkdir -p "$dest"
tar -xzf "$archive" -C "$dest"
odin="$(find "$dest" -maxdepth 3 -type f -name odin -perm -u+x | head -n 1)"
if [ -z "$odin" ]; then
  echo "no odin executable in $asset" >&2
  exit 1
fi
"$odin" version
echo "$odin"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  echo "odin=$odin" >> "$GITHUB_OUTPUT"
fi
