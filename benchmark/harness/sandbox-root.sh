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
#   - what it covers comes from /etc/mente-sandbox.conf alone, which the lock writes and only root
#     can change: the directories it hides, and the runs root, under which every working directory
#     lies and which it covers but for that directory; it takes no option and no other path;
#   - the working directory, its one path argument and the one directory the command may write,
#     must be a directory of the caller's strictly under the runs root;
#   - it opens nothing as root but its configuration: it looks paths up to check and mount them,
#     which follows no device, FIFO or file it could be pointed at, and builds every directory it
#     makes on scratch of its own;
#   - the command starts with descriptors 0, 1 and 2 alone, whatever ran before it;
#   - everything after the working directory is the command, which runs only after the drop.
#
#   sudo -n /usr/local/sbin/mente-sandbox <workdir> <command> [args...]
#
# Inside: a network namespace, so the command has only a down loopback and no route out; a PID
# namespace with its own /proc, so no process outside, the driver and the broker among them, is in
# its table; a mount namespace in which an empty tmpfs covers each hidden directory and the runs
# root, an unreadable empty directory covers each credential directory under the home, /tmp is a
# private tmpfs and the root is read-only, so the toolchains cannot be changed; <workdir> alone
# stays visible and writable, put back at its own path inside the cover of the runs root.
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
[ "$#" -ge 2 ] || refuse "usage: mente-sandbox <workdir> <command> [args...]"
case "$1" in -*) refuse "it takes no option: what it hides comes from $CONF alone" ;; esac
[ -f "$CONF" ] && [ "$(stat -c %u:%a -- "$CONF")" = "0:644" ] || refuse "$CONF is missing or not root's alone; benchmark/harness/lock-sudo.sh writes it"

# The configuration: `hide <directory>` lines and one `runs <directory>` line, each an absolute
# path written as the real path it is.
hides=()
runs=""
while read -r kind path rest; do
  [ -n "$kind" ] || continue
  [ -z "$rest" ] && [[ "$path" == /* ]] || refuse "$CONF holds a line it does not read: $kind $path $rest"
  case "$kind" in
    hide) hides+=("$path") ;;
    runs)
      [ -z "$runs" ] || refuse "$CONF names two runs roots"
      runs="$path"
      ;;
    *) refuse "$CONF holds a line it does not read: $kind" ;;
  esac
done < "$CONF"
[ -n "$runs" ] && [ -d "$runs" ] && [ "$(realpath -e -- "$runs")" = "$runs" ] || refuse "$CONF names no runs root that is a real directory"

# The working directory: a directory of the caller's, strictly under the runs root.
[ -d "$1" ] || refuse "the working directory $1 is no directory"
workdir="$(realpath -e -- "$1")"
shift
case "$workdir/" in "$runs"/?*) ;; *) refuse "the working directory $workdir does not lie under the runs root $runs" ;; esac
[ "$(stat -c %u -- "$workdir")" = "$uid" ] || refuse "the working directory $workdir is not the caller's"

# The directories an empty tmpfs covers: the configured ones that are there, as real paths, and
# the runs root, without repeats, and of nested ones only the outermost, since an inner one is
# covered with its outer one and is gone once that is covered.
hidden=()
for p in "${hides[@]}" "$runs"; do
  [ -d "$p" ] || continue
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

# The working directory lies inside one hidden directory, the runs root or one holding it: that one
# is its parent here, and it is put back inside the tmpfs that covers that. It must contain none.
parent=""
for h in "${hidden[@]:-}"; do
  [ -n "$h" ] || continue
  case "$h/" in "$workdir"/*) refuse "the working directory $workdir is, or contains, the hidden $h" ;; esac
  case "$workdir/" in "$h"/*) parent="$h" ;; esac
done
[ -n "$parent" ] || refuse "the working directory $workdir lies in no covered directory"

# Scratch directories, root's own, in /tmp, whose sticky bit lets no one else rename them: an
# unreadable empty one, bound over each credential directory; one to hold the working directory
# while its parent is covered; and one where the tmpfs that covers the parent is built before it is
# moved there whole, so that nothing is ever made on a path the caller owns.
empty="$(mktemp -d)"
chmod 000 "$empty"
stash="$(mktemp -d)"
cover="$(mktemp -d)"

# Enter fresh namespaces; arrange the mounts, still as root, by looking paths up and opening none;
# close every descriptor above 2; then drop to the caller with no new privileges and run the
# command. A path the caller renames while this runs moves a mount within this namespace alone,
# which changes what the command, running as the caller, sees of the caller's own files, and
# nothing on the host.
exec unshare --net --pid --mount --uts --ipc --fork --mount-proc --kill-child -- \
  /usr/bin/env -i "PATH=$PATH" \
  "BENCH_HOME=$home" "BENCH_UID=$uid" "BENCH_GID=$gid" "BENCH_EMPTY=$empty" \
  "BENCH_WORKDIR=$workdir" "BENCH_STASH=$stash" "BENCH_COVER=$cover" "BENCH_PARENT=$parent" \
  "BENCH_REL=${workdir#"$parent"/}" "BENCH_HIDDEN=$(printf '%s\n' "${hidden[@]:-}")" \
  /bin/bash -c '
    set -euo pipefail
    mount --make-rprivate /
    # The working directory as a mount of its own, so it stays writable under a read-only root,
    # held aside while its parent is covered.
    mount --bind "$BENCH_WORKDIR" "$BENCH_STASH"
    for d in .ssh .config .aws .docker .gnupg .kube .npm; do
      if [ -d "$BENCH_HOME/$d" ]; then
        mount --bind "$BENCH_EMPTY" "$BENCH_HOME/$d"
      fi
    done
    while IFS= read -r p; do
      [ -n "$p" ] && [ -d "$p" ] || continue
      if [ "$p" = "$BENCH_PARENT" ]; then
        # The tmpfs that covers the parent: built at root scratch, the working directory put back
        # at its own place inside it, then moved over the parent whole.
        mount -t tmpfs -o size=1m,mode=0755 tmpfs "$BENCH_COVER"
        mkdir -p "$BENCH_COVER/$BENCH_REL"
        mount --move "$BENCH_STASH" "$BENCH_COVER/$BENCH_REL"
        mount --move "$BENCH_COVER" "$p"
      else
        mount -t tmpfs -o size=1m,mode=0755 tmpfs "$p"
      fi
    done <<< "$BENCH_HIDDEN"
    # A private /tmp, unless the working directory is in it.
    case "$BENCH_WORKDIR/" in
      /tmp/*) ;;
      *) mount -t tmpfs -o mode=1777 tmpfs /tmp ;;
    esac
    mount -o remount,bind,ro /
    cd "$BENCH_WORKDIR"
    # The command starts with descriptors 0, 1 and 2 alone: every other this shell holds, whatever
    # opened it, is closed here, last.
    for f in "/proc/$BASHPID/fd/"*; do
      n="${f##*/}"
      if [[ "$n" =~ ^[0-9]+$ ]] && [ "$n" -gt 2 ] && [ -e "$f" ]; then exec {n}<&-; fi
    done
    exec setpriv --reuid "$BENCH_UID" --regid "$BENCH_GID" --clear-groups --no-new-privs -- "$@"
  ' bench "$@"
