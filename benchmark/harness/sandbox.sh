#!/usr/bin/env bash
# Runs one agent build or one agent program with least privilege on a GitHub-hosted Ubuntu runner
# (benchmark/README.md, "The harness"). An M1 agent program, unlike Mente at M1, could read files
# and open sockets, and a compiler runs the agent's code at build time (Odin's `#load`, Rust's
# build scripts and proc macros), so the agent's code is untrusted. Anything it printed would land
# in results.json, whose artifacts are public. This wrapper denies it the API key, the network, the
# workspace (where the held-out set is checked out), and any write outside its own directory.
#
#   sandbox.sh [--hide <path>]... <workdir> <command> [args...]
#
# How, using only what a GitHub-hosted Ubuntu runner has (passwordless sudo and util-linux's
# unshare, setpriv and mount):
#   - a network namespace, so the command has only a down loopback and no route out;
#   - a PID namespace with its own /proc, so the driver's process is not in the table and its
#     /proc/<pid>/environ, which holds the key, cannot be opened;
#   - a cleared environment rebuilt from a small allowlist, so the key is not in the command's own
#     environment either;
#   - a mount namespace in which an empty tmpfs covers $GITHUB_WORKSPACE and every --hide path
#     (the held-out checkout, wherever it is), an unreadable empty directory covers each credential
#     directory under the runner's home, /tmp is a private tmpfs, and the root filesystem is
#     read-only, so the toolchains cannot be changed; only <workdir> stays writable;
#   - a drop to the runner's own user with no new privileges, so the command is never root.
# It takes real root through sudo rather than an unprivileged user namespace, which Ubuntu 24.04
# restricts by default. <workdir> must already exist and must lie outside every hidden path. CI
# only: a developer's machine runs the harness without it.
set -euo pipefail

hide=()
while [ "$#" -gt 0 ] && [ "$1" = "--hide" ]; do
  hide+=("$2")
  shift 2
done
if [ "$#" -lt 2 ]; then
  echo "usage: sandbox.sh [--hide <path>]... <workdir> <command> [args...]" >&2
  exit 2
fi
workdir="$(realpath "$1")"
shift

# The paths an empty tmpfs covers: the workspace and those the harness names, as real paths,
# without repeats. The working directory must lie outside all of them, or it would be covered too.
hidden=()
for p in "${GITHUB_WORKSPACE:-}" "${hide[@]}"; do
  [ -n "$p" ] && [ -e "$p" ] || continue
  real="$(realpath "$p")"
  case "$workdir/" in
    "$real"/*)
      echo "sandbox.sh: the working directory $workdir lies inside the hidden $real" >&2
      exit 2
      ;;
  esac
  case " ${hidden[*]:-} " in
    *" $real "*) ;;
    *) hidden+=("$real") ;;
  esac
done
# A path inside another hidden one is covered with it, and is gone once that is covered, as the
# held-out checkout inside the workspace is; only the outermost paths are mounted over.
outermost=()
for p in "${hidden[@]:-}"; do
  [ -n "$p" ] || continue
  inner=false
  for q in "${hidden[@]}"; do
    if [ "$p" != "$q" ]; then
      case "$p/" in "$q"/*) inner=true ;; esac
    fi
  done
  $inner || outermost+=("$p")
done
hidden=("${outermost[@]:-}")

# An unreadable empty directory, bound over each credential directory.
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

# As root, enter fresh namespaces; then, still as root, arrange the mounts; then drop to the
# runner's own user with no new privileges and run the command. The values are this unprivileged
# shell's own, expanded before sudo, which resets the environment; the hidden paths go in as one
# newline-separated variable.
exec sudo -n unshare --net --pid --mount --uts --ipc --fork --mount-proc --kill-child -- \
  /usr/bin/env \
  "BENCH_HOME=$HOME" "BENCH_UID=$(id -u)" "BENCH_GID=$(id -g)" "BENCH_EMPTY=$empty" \
  "BENCH_WORKDIR=$workdir" "BENCH_HIDDEN=$(printf '%s\n' "${hidden[@]:-}")" \
  bash -c '
    set -euo pipefail
    mount --make-rprivate /
    # The working directory as a mount of its own, so it stays writable under a read-only root.
    mount --bind "$BENCH_WORKDIR" "$BENCH_WORKDIR"
    for d in .ssh .config .aws .docker .gnupg .kube .npm; do
      if [ -d "$BENCH_HOME/$d" ]; then
        mount --bind "$BENCH_EMPTY" "$BENCH_HOME/$d"
      fi
    done
    while IFS= read -r p; do
      [ -n "$p" ] && [ -e "$p" ] || continue
      if [ -d "$p" ]; then
        mount -t tmpfs -o size=1m,mode=0755 tmpfs "$p"
      else
        mount --bind /dev/null "$p"
      fi
    done <<< "$BENCH_HIDDEN"
    # A private /tmp, unless the working directory is in it.
    case "$BENCH_WORKDIR/" in
      /tmp/*) ;;
      *) mount -t tmpfs -o mode=1777 tmpfs /tmp ;;
    esac
    mount -o remount,bind,ro /
    cd "$BENCH_WORKDIR"
    exec setpriv --reuid "$BENCH_UID" --regid "$BENCH_GID" --clear-groups --no-new-privs -- "$@"
  ' bench "${clean[@]}" "$@"
