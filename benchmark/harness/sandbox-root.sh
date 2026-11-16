#!/bin/bash
# The benchmark sandbox's root half (benchmark/README.md, "The harness"). benchmark/harness/
# sandbox.sh, which runs as the runner's user, hands it one command through sudo; it enters fresh
# namespaces, covers what the command must not see, and drops to that user with no new privileges
# before the command runs. benchmark/harness/lock-sudo.sh installs a root-owned copy of it at
# /usr/local/sbin/mente-sandbox, the one command sudo still runs for the runner's user once a job
# is set up (docs/design-review.md, untrusted-test-isolation). So it takes nothing from its caller
# that would let the caller act as root:
#   - the user it drops to is the one sudo names as the caller (SUDO_UID, SUDO_GID), never root,
#     and the home whose credential directories it masks is that user's in the password database;
#   - the paths it always covers come from /etc/mente-sandbox.conf, which the lock writes and only
#     root can change; a --hide path only adds a cover, in the command's own mount namespace;
#   - the working directory, which alone the command may write, must be a directory the caller
#     owns;
#   - its scratch directories are made here, as root;
#   - everything after the working directory is the command, which runs only after the drop.
#
#   sudo -n /usr/local/sbin/mente-sandbox [--hide <path>]... <workdir> <command> [args...]
#
# Inside: a network namespace, so the command has only a down loopback and no route out; a PID
# namespace with its own /proc, so no process outside, the driver and the broker among them, is in
# its table; a mount namespace in which an empty tmpfs covers each hidden path, an unreadable empty
# directory covers each credential directory under the home, /tmp is a private tmpfs and the root
# is read-only, so the toolchains cannot be changed; <workdir> alone stays visible and writable,
# put back at its own path when it lies inside a hidden one.
set -euo pipefail
PATH=/usr/sbin:/usr/bin:/sbin:/bin
CONF=/etc/mente-sandbox.conf

refuse() {
  echo "mente-sandbox: $1" >&2
  exit 2
}

uid="${SUDO_UID:-}"
gid="${SUDO_GID:-}"
if [ "$(id -u)" != 0 ] || ! [[ "$uid" =~ ^[0-9]+$ && "$gid" =~ ^[0-9]+$ ]] || [ "$uid" = 0 ] || [ "$gid" = 0 ]; then
  refuse "run it through sudo as the runner's user; it never runs a command as root"
fi
home="$(getent passwd "$uid" | cut -d: -f6)"
[ -n "$home" ] || refuse "no home for user $uid"

hide=()
while [ "$#" -gt 0 ] && [ "$1" = "--hide" ]; do
  [ "$#" -ge 2 ] || refuse "--hide needs a path"
  hide+=("$2")
  shift 2
done
[ "$#" -ge 2 ] || refuse "usage: mente-sandbox [--hide <path>]... <workdir> <command> [args...]"
[ -d "$1" ] || refuse "the working directory $1 is no directory"
workdir="$(realpath -e -- "$1")"
shift
[ "$(stat -c %u -- "$workdir")" = "$uid" ] || refuse "the working directory $workdir is not the caller's"
[ -f "$CONF" ] && [ "$(stat -c %u:%a -- "$CONF")" = "0:644" ] || refuse "$CONF is missing or not root's alone; benchmark/harness/lock-sudo.sh writes it"
mapfile -t configured < "$CONF"

# The paths an empty tmpfs covers: those the configuration names and those the caller adds, as
# real paths, without repeats, and of nested ones only the outermost, since an inner path is
# covered with its outer one and is gone once that is covered.
hidden=()
for p in "${configured[@]}" "${hide[@]}"; do
  [ -n "$p" ] && [ -e "$p" ] || continue
  real="$(realpath -e -- "$p")"
  case " ${hidden[*]:-} " in
    *" $real "*) ;;
    *) hidden+=("$real") ;;
  esac
done
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

# The working directory must not be a hidden path nor contain one, since it stays visible; when it
# lies inside one, that hidden path is its parent here, and it is put back inside the tmpfs that
# covers it.
parent=""
for h in "${hidden[@]:-}"; do
  [ -n "$h" ] || continue
  case "$h/" in "$workdir"/*) refuse "the working directory $workdir is, or contains, the hidden $h" ;; esac
  case "$workdir/" in "$h"/*) parent="$h" ;; esac
done

# Scratch directories, root's own, in /tmp, whose sticky bit lets no one else rename them: an
# unreadable empty one, bound over each credential directory; and, when the working directory lies
# inside a hidden path, one to hold it while that path is covered, and one where the tmpfs that
# covers it is built before it is moved there whole.
empty="$(mktemp -d)"
chmod 000 "$empty"
stash=""
cover=""
if [ -n "$parent" ]; then
  stash="$(mktemp -d)"
  cover="$(mktemp -d)"
fi

# Enter fresh namespaces; arrange the mounts, still as root; then drop to the caller with no new
# privileges and run the command. Inside, each path the caller could rename is opened once, and
# every mount on it goes through that descriptor (/proc/self/fd/N), never through its path again,
# so that what was checked is what is mounted; a descriptor that no longer names the path it was
# checked as is a path moved since, and is refused. Nothing is created on a path the caller owns:
# the working directory is put back inside the cover, built at root's own scratch path. What still
# goes by path is masking alone, in this namespace: the credential directories, and the last cd,
# which, renamed under it, would only start the command, as the caller, somewhere else.
exec unshare --net --pid --mount --uts --ipc --fork --mount-proc --kill-child -- \
  /usr/bin/env -i "PATH=$PATH" \
  "BENCH_HOME=$home" "BENCH_UID=$uid" "BENCH_GID=$gid" "BENCH_EMPTY=$empty" \
  "BENCH_WORKDIR=$workdir" "BENCH_STASH=$stash" "BENCH_COVER=$cover" "BENCH_PARENT=$parent" \
  "BENCH_REL=${workdir#"$parent"/}" "BENCH_HIDDEN=$(printf '%s\n' "${hidden[@]:-}")" \
  /bin/bash -c '
    set -euo pipefail
    refuse() {
      echo "mente-sandbox: $1" >&2
      exit 2
    }
    opened() {
      [ "$(readlink "/proc/self/fd/$1")" = "$2" ] || refuse "$2 moved while the sandbox was set up"
    }
    mount --make-rprivate /
    # The working directory, opened once and checked through its descriptor, then bound from it as
    # a mount of its own, so it stays writable under a read-only root: held aside while the hidden
    # path above it is covered, or bound onto itself.
    exec {wd}<"$BENCH_WORKDIR"
    opened "$wd" "$BENCH_WORKDIR"
    if [ ! -d "/proc/self/fd/$wd" ] || [ "$(stat -L -c %u "/proc/self/fd/$wd")" != "$BENCH_UID" ]; then
      refuse "the working directory $BENCH_WORKDIR is no longer a directory of the caller"
    fi
    if [ -n "$BENCH_STASH" ]; then
      mount -c --bind "/proc/self/fd/$wd" "$BENCH_STASH"
    else
      mount -c --bind "/proc/self/fd/$wd" "/proc/self/fd/$wd"
    fi
    for d in .ssh .config .aws .docker .gnupg .kube .npm; do
      if [ -d "$BENCH_HOME/$d" ]; then
        exec {c}<"$BENCH_HOME/$d"
        mount -c --bind "$BENCH_EMPTY" "/proc/self/fd/$c"
      fi
    done
    while IFS= read -r p; do
      [ -n "$p" ] && [ -e "$p" ] || continue
      exec {h}<"$p"
      opened "$h" "$p"
      if [ "$p" = "$BENCH_PARENT" ]; then
        # The tmpfs that covers the hidden path the working directory lies in: built at root scratch,
        # the working directory put back at its own place inside it, then moved over the path whole.
        mount -t tmpfs -o size=1m,mode=0755 tmpfs "$BENCH_COVER"
        mkdir -p "$BENCH_COVER/$BENCH_REL"
        mount --move "$BENCH_STASH" "$BENCH_COVER/$BENCH_REL"
        mount -c --move "$BENCH_COVER" "/proc/self/fd/$h"
      elif [ -d "/proc/self/fd/$h" ]; then
        mount -c -t tmpfs -o size=1m,mode=0755 tmpfs "/proc/self/fd/$h"
      else
        mount -c --bind /dev/null "/proc/self/fd/$h"
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
  ' bench "$@"
