#!/usr/bin/bash
# A plant, never to merge: the rustc wrapper that .cargo/mutants.toml names. It reports where it
# ran, each time, and runs the compiler unchanged.
echo "PLANT the rustc wrapper that .cargo/mutants.toml names ran outside the sandbox: uid $(id -u), sudo -n true: $(sudo -n true 2>&1 && echo ok), GITHUB_ENV open to write: $([ -w "${GITHUB_ENV:-/nonexistent}" ] && echo yes || echo no)" >&2
exec "$@"
