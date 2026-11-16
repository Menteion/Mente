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
# its table; an IPC namespace; a mount namespace in which an empty tmpfs covers each hidden
# directory and the runs root, an unreadable empty directory covers each credential directory under
# the home, /tmp, /dev/shm, /dev/mqueue and the caller runtime directory under /run/user are its
# own, and every other mount the table lists is read-only, so that no run leaves anything for
# another to find and the toolchains cannot be changed; <workdir> alone stays visible and writable
# outside those, put back at its own path inside the cover of the runs root.
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
# The sandbox has a /tmp of its own, which it mounts first: a runs root under the host one would be
# gone by then.
case "$runs/" in /tmp/*) refuse "$CONF names a runs root under /tmp, which the sandbox replaces with its own" ;; esac

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

# Enter fresh namespaces; arrange the mounts, still as root, by looking paths up and opening none;
# close every descriptor above 2; then drop to the caller with no new privileges and run the
# command. Everything the mounts need is made inside the namespace, on its private /tmp, so that
# nothing is left on the host whatever happens to this call, a kill among it. A path the caller
# renames while this runs moves a mount within this namespace alone, which changes what the
# command, running as the caller, sees of the caller's own files, and nothing on the host.
exec unshare --net --pid --mount --uts --ipc --fork --mount-proc --kill-child -- \
  /usr/bin/env -i "PATH=$PATH" \
  "BENCH_HOME=$home" "BENCH_UID=$uid" "BENCH_GID=$gid" \
  "BENCH_WORKDIR=$workdir" "BENCH_PARENT=$parent" \
  "BENCH_REL=${workdir#"$parent"/}" "BENCH_HIDDEN=$(printf '%s\n' "${hidden[@]:-}")" \
  /bin/bash -c '
    set -euo pipefail
    mount --make-rprivate /
    # A private /tmp first, and on it the scratch: an unreadable empty directory, bound over each
    # credential directory; one to hold the working directory while its parent is covered; and one
    # where the tmpfs that covers the parent is built before it is moved there whole, so that
    # nothing is ever made on a path the caller owns.
    mount -t tmpfs -o mode=1777 tmpfs /tmp
    scratch=/tmp/.mente-sandbox
    mkdir -m 0700 "$scratch" "$scratch/stash" "$scratch/cover"
    mkdir -m 0000 "$scratch/empty"
    # The working directory as a mount of its own, so it stays writable under a read-only root,
    # held aside while its parent is covered.
    mount --bind "$BENCH_WORKDIR" "$scratch/stash"
    for d in .ssh .config .aws .docker .gnupg .kube .npm; do
      if [ -d "$BENCH_HOME/$d" ]; then
        mount --bind "$scratch/empty" "$BENCH_HOME/$d"
      fi
    done
    while IFS= read -r p; do
      [ -n "$p" ] && [ -d "$p" ] || continue
      if [ "$p" = "$BENCH_PARENT" ]; then
        # The tmpfs that covers the parent: built on the scratch, the working directory put back at
        # its own place inside it, then moved over the parent whole.
        mount -t tmpfs -o size=1m,mode=0755 tmpfs "$scratch/cover"
        mkdir -p "$scratch/cover/$BENCH_REL"
        mount --move "$scratch/stash" "$scratch/cover/$BENCH_REL"
        mount --move "$scratch/cover" "$p"
      else
        mount -t tmpfs -o size=1m,mode=0755 tmpfs "$p"
      fi
    done <<< "$BENCH_HIDDEN"
    rmdir "$scratch/stash" "$scratch/cover" "$scratch/empty" "$scratch"
    # Shared memory, message queues and the caller own runtime directory, which programs of other
    # runs and the runner could also write: private to this sandbox.
    mount -t tmpfs -o nosuid,nodev,mode=1777 tmpfs /dev/shm
    mount -t mqueue -o nosuid,nodev,noexec mqueue /dev/mqueue
    private=("$BENCH_WORKDIR" /tmp /dev/shm /dev/mqueue)
    if [ -d "/run/user/$BENCH_UID" ]; then
      mount -t tmpfs -o nosuid,nodev,mode=0700,uid="$BENCH_UID",gid="$BENCH_GID" tmpfs "/run/user/$BENCH_UID"
      private+=("/run/user/$BENCH_UID")
    fi
    # Every other mount read-only, each one the mount table lists, so that one this does not know
    # of, a new shared mount among them, is read-only too; a remount that fails stops the sandbox.
    keep=$(printf "%s\n" "${private[@]}")
    while read -r target majmin; do
      target="$(printf "%b" "$target")"
      grep -qxF -- "$target" <<< "$keep" && continue
      # A mount that another now lies over, a cover or the new /proc, is no longer reachable by its
      # path, nor by the command: what its path reaches is the mount above, a filesystem of another
      # device. The canary, which writes into every writable mount and is looked for from outside,
      # finds any this would leave writable.
      [ "$(stat -c "%Hd:%Ld" -- "$target" 2> /dev/null)" = "$majmin" ] || continue
      mount -o remount,bind,ro -- "$target"
    done < <(findmnt -rn -o TARGET,MAJ:MIN | sort -u)
    cd "$BENCH_WORKDIR"
    # The command starts with descriptors 0, 1 and 2 alone: every other this shell holds, whatever
    # opened it, is closed here, last.
    for f in "/proc/$BASHPID/fd/"*; do
      n="${f##*/}"
      if [[ "$n" =~ ^[0-9]+$ ]] && [ "$n" -gt 2 ] && [ -e "$f" ]; then exec {n}<&-; fi
    done
    exec setpriv --reuid "$BENCH_UID" --regid "$BENCH_GID" --clear-groups --no-new-privs -- "$@"
  ' bench "$@"
