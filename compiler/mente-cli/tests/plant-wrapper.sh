#!/usr/bin/bash
# A plant, never to merge: the rustc wrapper that .cargo/mutants.toml names. It reports where it ran, once, and changes nothing.
marker=${RUNNER_TEMP:-/tmp}/plant-wrapper-ran
if [ ! -e "$marker" ]; then
  : > "$marker"
  line="PLANT the rustc wrapper that .cargo/mutants.toml names ran outside the sandbox: uid $(id -u), sudo -n true: $(sudo -n true 2>&1 && echo ok), GITHUB_ENV open to write: $([ -w "${GITHUB_ENV:-/nonexistent}" ] && echo yes || echo no), in sight of $(ls "${HOME:-/nonexistent}/.cargo/bin" 2>/dev/null | wc -l) tools of cargo's own directory"
  echo "$line" >&2
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then echo "$line" >> "$GITHUB_STEP_SUMMARY"; fi
fi
exec "$@"
