#!/usr/bin/env bash
# Installs mdBook, the documentation site's builder (docs/design-review.md, `docs-site`), into
# $HOME/.local/mdbook: the release binary of MDBOOK_VERSION, checked against the SHA-256 GitHub
# records for it, on every run. mdBook is a CI tool, not a workspace dependency. Raise both together.
# Run by pages.yml and pages-publish.yml.
#
#   bash .github/scripts/install-mdbook.sh
set -euo pipefail

MDBOOK_VERSION="0.5.4"
MDBOOK_SHA256="3f28de05dafca9d0f2eab99c662116b0e37b89b1d96a08f8f430b9eeae958cd7"

archive="$RUNNER_TEMP/mdbook.tar.gz"
curl -sSfL -o "$archive" \
  "https://github.com/rust-lang/mdBook/releases/download/v${MDBOOK_VERSION}/mdbook-v${MDBOOK_VERSION}-x86_64-unknown-linux-gnu.tar.gz"
echo "${MDBOOK_SHA256}  ${archive}" | sha256sum -c -
rm -rf "$HOME/.local/mdbook"
mkdir -p "$HOME/.local/mdbook"
tar -xzf "$archive" -C "$HOME/.local/mdbook"
"$HOME/.local/mdbook/mdbook" --version
