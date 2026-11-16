#!/usr/bin/env bash
# Runs one agent build or one agent program with least privilege on a GitHub-hosted Ubuntu runner
# (benchmark/README.md, "The harness"). An M1 agent program, unlike Mente at M1, could read files
# and open sockets, and a compiler runs the agent's code at build time (Odin's `#load`, Rust's
# build scripts and proc macros), so the agent's code is untrusted. Anything it printed would land
# in results.json, whose artifacts are public. This wrapper denies it the API key, the network, the
# workspace (where the held-out set is checked out), the broker's socket, every other run's folder,
# and any write outside its own directory.
#
#   sandbox.sh [--hide <path>]... <workdir> <command> [args...]
#
# This is the unprivileged half: it builds the only environment that crosses into the sandbox, a
# cleared one rebuilt from a small allowlist, so that no secret is in the command's environment, and
# hands the command to the root half, benchmark/harness/sandbox-root.sh, through sudo. That half
# runs from the root-owned copy benchmark/harness/lock-sudo.sh installs, the one command sudo still
# runs for the runner's user once the job is set up, and decides as root what the command sees:
# a network namespace with no route out, a PID namespace with its own /proc, an empty tmpfs over
# the workspace, the broker's directory $RUNNER_TEMP/broker and every --hide path (the held-out
# checkout, wherever it is, and the runs' output directory), an unreadable directory over each
# credential directory, a private /tmp and a read-only root, with <workdir> alone writable, put
# back at its own path inside the tmpfs when it lies inside a hidden path, and a drop to the
# runner's own user with no new privileges, so the command is never root. It takes real root
# through sudo rather than an unprivileged user namespace, which Ubuntu 24.04 restricts by default.
# <workdir> must already exist and be the runner's user's; it must not be a hidden path nor contain
# one, since showing it would show what is hidden. CI only: a developer's machine runs the harness
# without it.
set -euo pipefail

ENTRY=/usr/local/sbin/mente-sandbox

hide=()
while [ "$#" -gt 0 ] && [ "$1" = "--hide" ]; do
  hide+=(--hide "$2")
  shift 2
done
if [ "$#" -lt 2 ]; then
  echo "usage: sandbox.sh [--hide <path>]... <workdir> <command> [args...]" >&2
  exit 2
fi
workdir="$(realpath "$1")"
shift

# The only environment that crosses into the sandbox: never a secret. The harness exports the
# toolchain variables the build needs; they carry no secret.
clean=(env -i "PATH=$PATH" "HOME=$workdir" "TMPDIR=$workdir" "LANG=${LANG:-C.UTF-8}" "LC_ALL=${LC_ALL:-C.UTF-8}")
for name in RUSTUP_HOME CARGO_HOME RUSTUP_TOOLCHAIN; do
  if [ -n "${!name:-}" ]; then
    clean+=("$name=${!name}")
  fi
done

exec sudo -n "$ENTRY" "${hide[@]}" "$workdir" "${clean[@]}" "$@"
