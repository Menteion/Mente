#!/usr/bin/env bash
# Runs one agent build or one agent program with least privilege on a GitHub-hosted Ubuntu runner
# (benchmark/README.md, "The harness"). An M1 agent program, unlike Mente at M1, could read files
# and open sockets, and a compiler runs the agent's code at build time (Odin's `#load`, Rust's
# build scripts and proc macros), so the agent's code is untrusted. Anything it printed would land
# in results.json, whose artifacts are public. This wrapper denies it the two things that matter:
# the API key, which the driver holds in its environment, and the network.
#
#   sandbox.sh <workdir> <command> [args...]
#
# How, using only what a GitHub-hosted Ubuntu runner has (passwordless sudo and util-linux's
# unshare and setpriv):
#   - a network namespace, so the command has only a down loopback and no route out;
#   - a PID namespace with its own /proc, so the driver's process is not in the table and its
#     /proc/<pid>/environ, which holds the key, cannot be opened;
#   - a cleared environment rebuilt from a small allowlist, so the key is not in the command's own
#     environment either;
#   - a mount namespace that hides the credential directories under the runner's home;
#   - a drop to the runner's own user with no new privileges, so the command is never root.
# It takes real root through sudo rather than an unprivileged user namespace, which Ubuntu 24.04
# restricts by default. The working directory is <workdir>, which must already exist and which the
# command may write; nothing else is writable that the command did not make. CI only: a developer's
# machine runs the harness without it.
set -euo pipefail

if [ "$#" -lt 2 ]; then
  echo "usage: sandbox.sh <workdir> <command> [args...]" >&2
  exit 2
fi

workdir="$1"
shift

# An unreadable empty directory, bound over each credential directory below.
empty="$(mktemp -d)"
chmod 000 "$empty"

# The only environment that crosses into the sandbox: never a secret. The harness exports the
# toolchain variables the build needs; they carry no secret.
clean=(env -i "PATH=$PATH" "HOME=$workdir" "TMPDIR=$workdir" "LANG=${LANG:-C.UTF-8}" "LC_ALL=${LC_ALL:-C.UTF-8}")
for name in RUSTUP_HOME CARGO_HOME RUSTUP_TOOLCHAIN; do
  if [ -n "${!name:-}" ]; then
    clean+=("$name=${!name}")
  fi
done

# As root, enter fresh namespaces; then, still as root, hide the credential directories under the
# runner's home; then drop to the runner's own user with no new privileges and run the command.
# The home, uid, gid, empty directory and working directory are this (unprivileged) shell's own,
# expanded before sudo; sudo resets the environment, so they are passed in explicitly.
exec sudo -n unshare --net --pid --mount --uts --ipc --fork --mount-proc --kill-child -- \
  /usr/bin/env \
  "BENCH_HOME=$HOME" "BENCH_UID=$(id -u)" "BENCH_GID=$(id -g)" "BENCH_EMPTY=$empty" "BENCH_WORKDIR=$workdir" \
  bash -c '
    set -euo pipefail
    mount --make-rprivate /
    for d in .ssh .config .aws .docker .gnupg .kube .npm; do
      if [ -d "$BENCH_HOME/$d" ]; then
        mount --bind "$BENCH_EMPTY" "$BENCH_HOME/$d"
      fi
    done
    cd "$BENCH_WORKDIR"
    exec setpriv --reuid "$BENCH_UID" --regid "$BENCH_GID" --clear-groups --no-new-privs -- "$@"
  ' bench "${clean[@]}" "$@"
