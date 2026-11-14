# The sandbox the code of a change runs in on a Linux runner, where a job tests it: every test
# executable, and every binary `cargo run` starts, with whatever they start in turn
# (docs/design-review.md, untrusted-test-isolation). The tools that judge that code, cargo, nextest,
# cargo-mutants and the step's shell, stay outside it. Before, a test ran with the job's rights,
# the runner's user with sudo, and could fake every later step of its job.
#
#   /usr/bin/bash .github/scripts/test-sandbox.sh setup    once a job, before any code of the change runs
#   /usr/bin/bash .github/scripts/test-sandbox.sh check    the canary: proves the sandbox on this runner
#   /usr/bin/bash .github/scripts/test-sandbox.sh env      what test-sandbox-env.sh evaluates: cargo's runner
#   /usr/bin/bash .github/scripts/test-sandbox.sh ran [n]  at a step's end: n commands ran inside, 1 if not given
#   /usr/bin/bash .github/scripts/test-sandbox.sh uid      the user the commands run as, for the memory guard
#
# How a command gets inside: `env` names this script as cargo's runner for the host's target
# (CARGO_TARGET_<TRIPLE>_RUNNER), which `cargo test`, `cargo run`, nextest and, through nextest,
# cargo-mutants all hand each executable to, as `run <executable> <arguments>`. Building stays
# outside: no package of the workspace may run code at build time, as cargo itself reads the
# manifests (.github/scripts/build-time-code.mjs), and .cargo/mutants.toml may name no command.
#
# What the command gets, using only what a GitHub-hosted Ubuntu runner has (sudo and util-linux):
#   - a root of its own, entered with pivot_root, that holds /usr, /etc and the workspace, all
#     read-only, five device nodes, its own /proc, and an empty /tmp and target/tmp of its own to
#     write. Nothing else exists there: no toolchain, no CARGO_HOME, no $RUNNER_TEMP with the
#     records the checks read, no $GITHUB_ENV or $GITHUB_PATH, no socket of the runner. The
#     workspace's .git is empty there: a test cannot compare the sources with the commit's;
#   - a user of its own, which no account of the runner has, with no group, no capability and no
#     way to gain one (no_new_privs), so sudo and every setuid program refuse it;
#   - PID, network, IPC and UTS namespaces of its own: it sees no process but its own, so it can
#     signal or trace none; it has no network interface but a loopback that is down; and when it
#     ends, the kernel ends every process it started;
#   - an environment of the variables cargo, nextest and the tests set (CARGO_*, NEXTEST*,
#     MENTE_*, RUST_*), and the limits on memory and file size of the shell that started it;
#   - no core dump: `setup` makes the runner's core pattern a file name, since its handler is a
#     pipe to a program that runs as root, and the core limit there is one byte.
#
# Two guards that a user of its own would switch off are kept. nextest stops a test past its time
# by a signal, which the runner's user cannot send to another user's process: the wrapper stays the
# runner's, and its end, by any signal, ends the namespace. And a test that leaves a process
# behind fails, as nextest's leak-timeout has it fail, instead of the namespace ending it unseen.
#
# It fails closed: a sandbox that cannot be set up or entered fails the command, which never runs
# outside it; `env` fails without `setup`; and `ran` fails a step whose tools ran nothing inside.
# Root mounts through no path of the workspace that is a link or lies under one.
#
# What it does not stop: a flaw of the kernel; what the command prints, which reaches the job's
# log; a command that starts processes without end, which fails its own job; and a test that
# reads the sources and fails when they differ from a copy it holds, which would catch every
# mutant: that is outside any sandbox, and the diff a reviewer reads is what sees it. Windows has
# no sandbox yet.
#
# The workflow policy (.github/scripts/policy.test.mjs) pins this script and the steps that run it.
set -euo pipefail

# Root's own: this script's copy, which the wrapper and the namespace run, and the root.
home=/opt/mente-test-sandbox
self=$home/test-sandbox.sh
root=$home/root
# The user and group the commands run as.
uid=4242424
gid=4242424
# Each command that ran inside, one a line, where no command inside can reach it.
log=${RUNNER_TEMP:-/nonexistent}/test-sandbox-runs.log

fail() {
  echo "::error::$1"
  exit 1
}

# Whether a mount is read-only.
read_only() {
  case ",$(/usr/bin/findmnt -n -o VFS-OPTIONS -M "$1")," in
    *,ro,*) return 0 ;;
    *) return 1 ;;
  esac
}

setup_as_root() {
  local here=$1 workspace=$2 dir node link
  if [ "$(/usr/bin/id -u)" != 0 ]; then fail "the sandbox is set up as root"; fi
  if [ -e "$home" ]; then fail "$home exists: the sandbox is set up once a job, before any code of the change runs"; fi
  if /usr/bin/getent passwd "$uid" > /dev/null || /usr/bin/getent group "$gid" > /dev/null; then
    fail "the user or group $uid of the sandbox is an account of this runner"
  fi
  case "$workspace" in
    /*) if [ ! -d "$workspace" ]; then fail "the workspace $workspace is no directory"; fi ;;
    *) fail "the workspace $workspace is no absolute path" ;;
  esac
  # The script root copies and runs is the workspace's own file, in its own directory.
  if [ "$here" != "$workspace/.github/scripts" ] || [ ! -f "$here/test-sandbox.sh" ]; then
    fail "$here/test-sandbox.sh is not the workspace's own .github/scripts/test-sandbox.sh"
  fi
  /usr/bin/install -d -m 0755 -o 0 -g 0 "$home" "$root"
  /usr/bin/install -m 0644 -o 0 -g 0 "$here/test-sandbox.sh" "$self"
  printf '%s\n' "$workspace" > "$home/workspace"
  # No crash starts a core handler: the runner's is a pipe to a program that runs as root outside
  # every namespace, on the core the crashed process shaped, and the kernel starts a piped handler
  # whatever core limit the process set itself, but for a limit of exactly one byte. A pattern
  # that is a file name starts none; with the sandbox's core limit no file is written either.
  printf 'core\n' > /proc/sys/kernel/core_pattern
  case "$(< /proc/sys/kernel/core_pattern)" in
    core) ;;
    *) fail "the runner's core handler is still $(< /proc/sys/kernel/core_pattern)" ;;
  esac
  /usr/bin/mount -t tmpfs -o size=1m,mode=0755,nosuid,nodev tmpfs "$root"
  /usr/bin/mkdir -p "$root/usr" "$root/etc" "$root/dev" "$root/proc" "$root/tmp" "$root/.old" "$root$workspace"
  # /bin, /lib and the like, links into /usr on a runner.
  for link in /bin /sbin /lib /lib32 /lib64 /libx32; do
    if [ -L "$link" ]; then /usr/bin/cp -P "$link" "$root$link"; fi
  done
  for dir in /usr /etc "$workspace"; do
    /usr/bin/mount --bind "$dir" "$root$dir"
    /usr/bin/mount -o remount,bind,ro,nosuid,nodev "$root$dir"
  done
  for node in null zero full random urandom; do
    : > "$root/dev/$node"
    /usr/bin/mount --bind "/dev/$node" "$root/dev/$node"
  done
  /usr/bin/ln -s /proc/self/fd "$root/dev/fd"
  /usr/bin/ln -s /proc/self/fd/0 "$root/dev/stdin"
  /usr/bin/ln -s /proc/self/fd/1 "$root/dev/stdout"
  /usr/bin/ln -s /proc/self/fd/2 "$root/dev/stderr"
  /usr/bin/mount -o remount,ro,nosuid,nodev "$root"
  # Nothing mounted later on the runner's own tree shows up in the sandbox's.
  /usr/bin/mount --make-rprivate "$root"
  for dir in "$root" "$root/usr" "$root/etc" "$root$workspace"; do
    if ! read_only "$dir"; then fail "$dir of the sandbox is not read-only"; fi
  done
}

setup() {
  local here workspace
  if [ "${RUNNER_OS:-}" != Linux ]; then
    echo "No sandbox on ${RUNNER_OS:-this machine}: the tests run with the job's rights (docs/design-review.md, untrusted-test-isolation)."
    return 0
  fi
  if [ -L "$0" ]; then fail "$0 is a link: root copies and runs the workspace's own file, not what a link names"; fi
  here=$(/usr/bin/dirname "$(/usr/bin/realpath "$0")")
  workspace=$(/usr/bin/realpath "$GITHUB_WORKSPACE")
  /usr/bin/sudo -n -- /usr/bin/bash "$here/test-sandbox.sh" setup-as-root "$here" "$workspace"
  : > "$log"
  echo "The sandbox of $workspace is at $root, for the user $uid."
}

# The processes alive in this namespace beside its first and the watcher, in `left`. A process
# has ended when the kernel says so (State: Z) and no other thread of it runs: its name, which it
# chooses, is never read, and one whose first thread alone has ended still counts.
remaining() {
  local dir pid key value rest state threads
  left=
  for dir in /proc/[0-9]*; do
    pid=${dir#/proc/}
    if [ "$pid" = 1 ] || [ "$pid" = "$watcher" ]; then continue; fi
    state='' threads=''
    {
      while read -r key value rest; do
        case "$key" in
          State:) state=$value ;;
          Threads:) threads=$value ;;
        esac
      done < "$dir/status"
    } 2> /dev/null || continue
    if [ -z "$state" ]; then continue; fi
    if [ "$state" = Z ] && [ "${threads:-1}" -le 1 ]; then continue; fi
    left="$left $pid"
  done
}

# The first process of the namespace, as root: the mounts, the root, then the command as the
# sandbox's user. Its standard input is the wrapper's lifeline.
inner() {
  local cwd=$1 vlimit=$2 flimit=$3 envs=() workspace status=0 watcher left k dir
  shift 3
  while [ "$#" -gt 0 ] && [ "$1" != -- ]; do
    envs+=("$1")
    shift
  done
  if [ "$#" -lt 2 ]; then
    echo "test-sandbox: no command" >&2
    exit 2
  fi
  shift
  if [ "$$" != 1 ]; then
    echo "test-sandbox: not the first process of a namespace of its own" >&2
    exit 2
  fi
  workspace=$(< "$home/workspace")
  exec 8<&0 < /dev/null
  # Memory backs the temporary filesystems, so they are bounded: four tests at once write at most
  # 6 GB.
  /usr/bin/mount -t tmpfs -o nosuid,nodev,mode=1777,size=1g tmpfs "$root/tmp"
  # Root mounts over two directories of the workspace, whose files are the change's own: where
  # cargo sends an integration test's files (CARGO_TARGET_TMPDIR), which the test may write, and
  # .git, which it may not read, so that it cannot tell a mutant from the commit's own sources
  # without holding a copy of them. Each must be that very directory, no link and nothing under a
  # link, or root would mount wherever the change aimed it, over a directory of /usr or /etc.
  # These two are every path under the workspace that root resolves before it changes root.
  for dir in "$workspace/target/tmp" "$workspace/.git"; do
    if [ ! -e "$root$dir" ] && [ ! -L "$root$dir" ]; then continue; fi
    if [ ! -d "$root$dir" ] || [ "$(/usr/bin/realpath -e -- "$root$dir")" != "$root$dir" ]; then
      echo "test-sandbox: $dir is a link, lies under one or is no directory: root mounts nothing through a path the change can aim elsewhere" >&2
      exit 2
    fi
    case "$dir" in
      */.git) /usr/bin/mount -t tmpfs -o ro,nosuid,nodev,noexec,mode=0555,size=4k tmpfs "$root$dir" ;;
      *) /usr/bin/mount -t tmpfs -o nosuid,nodev,mode=1777,size=512m tmpfs "$root$dir" ;;
    esac
  done
  /usr/bin/mount -t proc -o nosuid,nodev,noexec proc "$root/proc"
  cd "$root"
  /usr/sbin/pivot_root . .old
  /usr/bin/umount -l /.old
  # The command starts where cargo started it, which is a directory of the workspace itself.
  if ! cd -P -- "$cwd" 2> /dev/null; then
    echo "test-sandbox: the working directory $cwd is not in the sandbox" >&2
    exit 2
  fi
  case "$PWD/" in
    "$workspace"/*) ;;
    *)
      echo "test-sandbox: the working directory $cwd leads out of the workspace, to $PWD" >&2
      exit 2
      ;;
  esac
  ulimit -v "$vlimit" -f "$flimit"
  # A core limit of one byte, which the command cannot raise: no crash writes a core file.
  /usr/bin/prlimit --pid "$$" --core=1:1
  # When the wrapper ends or asks, every process here but this one ends, and so the command. The
  # watcher goes on ending them for as long as this process lives: a wrapper that ended before the
  # command had started must not leave the command to run on without it.
  {
    read -r -u 8 _ || true
    while :; do kill -s KILL -- -1 || true; done
  } > /dev/null 2>&1 &
  watcher=$!
  /usr/bin/setpriv --reuid "$uid" --regid "$gid" --clear-groups --no-new-privs \
    --inh-caps=-all --ambient-caps=-all --bounding-set=-all -- \
    /usr/bin/env -i "${envs[@]}" PATH=/usr/bin:/bin HOME=/tmp TMPDIR=/tmp "$@" 8<&- || status=$?
  # A process the command left behind ends with the namespace. As under nextest's leak-timeout,
  # 100 ms after the command, that fails it.
  remaining
  for k in 1 2 3 4 5 6 7 8 9 10; do
    if [ -z "$left" ]; then break; fi
    /usr/bin/sleep 0.01
    remaining
  done
  if [ -n "$left" ]; then
    echo "test-sandbox: the command left processes behind, which end with it:$left" >&2
    if [ "$status" = 0 ]; then status=1; fi
  fi
  exit "$status"
}

# The wrapper, as the runner's user: cargo's runner. It records the command, starts the namespace
# through sudo, and waits; its end ends the namespace.
run() {
  local name envs=() child status=0 signal=
  if [ "$#" -lt 1 ]; then
    echo "test-sandbox: no command" >&2
    exit 2
  fi
  case "$1" in
    *=*)
      echo "test-sandbox: a command with = in its path would be read as a variable: $1" >&2
      exit 2
      ;;
  esac
  printf '%s\n' "$1" >> "$log"
  for name in $(compgen -e); do
    case "$name" in
      CARGO | CARGO_* | NEXTEST | NEXTEST_* | MENTE_* | RUST_* | TERM | LANG | LC_* | TZ | CI | NO_COLOR | CLICOLOR | CLICOLOR_FORCE | FORCE_COLOR)
        envs+=("$name=${!name}")
        ;;
    esac
  done
  # The lifeline: a pipe this shell alone writes, which the namespace reads as its standard
  # input. It ends when this shell does, by any signal, SIGKILL too.
  exec 9> >(exec /usr/bin/sudo -n -- /usr/bin/unshare --mount --pid --net --ipc --uts --fork --kill-child=SIGKILL -- \
    /usr/bin/bash "$self" inner "$PWD" "$(ulimit -v)" "$(ulimit -f)" "${envs[@]}" -- "$@")
  child=$!
  trap '' PIPE
  trap 'signal=15; printf "\n" >&9 2> /dev/null' TERM
  trap 'signal=2; printf "\n" >&9 2> /dev/null' INT
  trap 'signal=1; printf "\n" >&9 2> /dev/null' HUP
  wait "$child" || status=$?
  # A signal ends the wait before the namespace has ended.
  while [ -e "/proc/$child" ]; do
    status=0
    wait "$child" || status=$?
  done
  if [ -n "$signal" ]; then exit $((128 + signal)); fi
  exit "$status"
}

env_lines() {
  if [ "${RUNNER_OS:-}" != Linux ]; then return 0; fi
  if [ ! -r "$self" ] || ! read_only "$root"; then
    fail "the sandbox is not set up: test-sandbox.sh setup runs before any code of the change"
  fi
  case "$(< /proc/sys/kernel/core_pattern)" in
    \|*) fail "a crash would start the runner's core handler again: $(< /proc/sys/kernel/core_pattern)" ;;
  esac
  : > "$log"
  printf "export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='/usr/bin/bash %s run'\n" "$self"
  printf "export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GUN_RUNNER='/usr/bin/bash %s run'\n" "$self"
}

# Whether the step's tools handed commands to the sandbox at all: it fails a step in which cargo
# ignored the runner, as it would for a target spelled wrong or after a change of a tool. It is
# no proof for each command: it counts the wrapper's lines, tests and nextest's lists alike, and
# code that had already run outside the sandbox could write them itself.
ran() {
  local need=${1:-1} count
  if [ "${RUNNER_OS:-}" != Linux ]; then
    echo "No sandbox on ${RUNNER_OS:-this machine}."
    return 0
  fi
  case "$need" in
    '' | *[!0-9]*) fail "not a count of commands: '$need'" ;;
  esac
  if [ ! -f "$log" ]; then fail "no record of the commands that ran in the sandbox: test-sandbox-env.sh starts it"; fi
  count=$(/usr/bin/wc -l < "$log")
  if [ "$count" -lt "$need" ]; then
    fail "$count commands of this step ran in the sandbox, where at least $need run the code of the change: cargo, nextest or cargo-mutants did not hand them to it"
  fi
  echo "Commands of this step that ran in the sandbox: $count"
}

# What a command can reach, one line a route: run by the canary once as the runner's user and
# once in the sandbox. Its arguments name what stands for the tools and the records.
probe() {
  local arg key value nnp='' bnd='' eff='' name file dir entry
  declare -A at
  for arg in "$@"; do at[${arg%%=*}]=${arg#*=}; done
  # A file made in a directory, and removed.
  writes() {
    local made="$1/test-sandbox-probe.$BASHPID"
    if { : > "$made"; } 2> /dev/null; then
      /usr/bin/rm -f "$made"
      echo open
    else
      echo closed
    fi
  }
  # A file that exists, opened to write, with nothing written.
  appends() {
    if [ -e "$1" ] && { : >> "$1"; } 2> /dev/null; then echo open; else echo closed; fi
  }
  echo "uid $(/usr/bin/id -u)"
  echo "toolchain $(writes "${at[toolchain]}")"
  echo "cargo-home $(writes "${at[cargo]}")"
  echo "git $(writes .git)"
  if [ -r .git/HEAD ]; then echo "git-history open"; else echo "git-history closed"; fi
  echo "github $(writes .github)"
  echo "config $(writes .config)"
  echo "script $(appends .github/scripts/limits.sh)"
  echo "toolchain-file $(appends rust-toolchain.toml)"
  echo "nextest-settings $(appends .config/nextest.toml)"
  echo "target-file $(appends "${at[tool]}")"
  echo "target $(writes target)"
  echo "github-env $(appends "${at[env]}")"
  echo "github-path $(appends "${at[path]}")"
  echo "github-output $(appends "${at[output]}")"
  echo "step-summary $(appends "${at[summary]}")"
  echo "record $(appends "${at[record]}")"
  if /usr/bin/sudo -n /usr/bin/true 2> /dev/null; then echo "sudo open"; else echo "sudo closed"; fi
  while read -r key value; do
    case "$key" in
      NoNewPrivs:) nnp=$value ;;
      CapBnd:) bnd=$value ;;
      CapEff:) eff=$value ;;
    esac
  done < /proc/self/status
  if [ "$nnp" = 1 ] && [ "$((16#$bnd))" = 0 ] && [ "$((16#$eff))" = 0 ]; then echo "privileges closed"; else echo "privileges open"; fi
  if kill -0 "${at[decoy]}" 2> /dev/null; then echo "signal open"; else echo "signal closed"; fi
  if /usr/bin/grep -q -a "${at[marker]}" "/proc/${at[decoy]}/environ" 2> /dev/null; then echo "trace open"; else echo "trace closed"; fi
  # A shared memory segment of the runner's user, by its id.
  if /usr/bin/ipcs -m 2> /dev/null | /usr/bin/awk '{print $2}' | /usr/bin/grep -x "${at[shm]}" > /dev/null; then echo "ipc open"; else echo "ipc closed"; fi
  # A device beyond the five the sandbox gives, the runner's disks and shared memory among them.
  value=closed
  for entry in /dev/*; do
    case "${entry#/dev/}" in
      null | zero | full | random | urandom | fd | stdin | stdout | stderr) ;;
      *) value=open ;;
    esac
  done
  echo "devices $value"
  value=closed
  while read -r name _; do
    case "$name" in
      lo: | Inter-* | face) ;;
      *:*) value=open ;;
    esac
  done < /proc/net/dev
  echo "network-interface $value"
  if (exec 3<> "/dev/tcp/127.0.0.1/${at[port]}") 2> /dev/null; then echo "network open"; else echo "network closed"; fi
  if [ -n "${GITHUB_ENV:-}${RUNNER_TEMP:-}${TEST_SANDBOX_JOB:-}" ]; then echo "job-variables open"; else echo "job-variables closed"; fi
  # A crash, at the highest core limit the command can set and at a limit lowered to 0, which a
  # piped core handler would not heed: no core file may come of either.
  dir=${TMPDIR:-/tmp}/test-sandbox-core.$BASHPID
  /usr/bin/mkdir -p "$dir"
  (
    cd "$dir"
    ulimit -S -c "$(ulimit -H -c)"
    /usr/bin/sleep 30 &
    kill -s SEGV "$!"
    wait "$!"
  ) > /dev/null 2>&1 || true
  (
    cd "$dir"
    ulimit -S -c 0
    /usr/bin/sleep 30 &
    kill -s SEGV "$!"
    wait "$!"
  ) > /dev/null 2>&1 || true
  if compgen -G "$dir/core*" > /dev/null; then echo "core-file open"; else echo "core-file closed"; fi
  /usr/bin/rm -rf "$dir"
  # A user namespace in which the command would hold root's capabilities: enough to mount.
  dir=${TMPDIR:-/tmp}/test-sandbox-userns.$BASHPID
  /usr/bin/mkdir -p "$dir"
  if /usr/bin/unshare --user --map-root-user --mount /usr/bin/mount -t tmpfs none "$dir" > /dev/null 2>&1; then echo "user-namespace open"; else echo "user-namespace closed"; fi
  /usr/bin/rmdir "$dir" 2> /dev/null || true
  # What the tests need.
  if /usr/bin/grep -q '^\[workspace\]' Cargo.toml 2> /dev/null; then echo "read open"; else echo "read closed"; fi
  file=${TMPDIR:-/tmp}/test-sandbox-probe.$BASHPID
  if printf '#!/bin/sh\nexit 0\n' > "$file" 2> /dev/null && /usr/bin/chmod +x "$file" && "$file"; then echo "temp open"; else echo "temp closed"; fi
  /usr/bin/rm -f "$file"
  echo "target-tmp $(writes target/tmp)"
  if /usr/bin/true; then echo "child open"; else echo "child closed"; fi
  if cc --version > /dev/null 2>&1; then echo "linker open"; else echo "linker closed"; fi
  if [ "${MENTE_TEST_SANDBOX_PROBE:-}" = "${at[marker]}" ]; then echo "variables open"; else echo "variables closed"; fi
  echo "limits $(ulimit -v)/$(ulimit -f)"
  echo "core-limit $(/usr/bin/prlimit --core --noheadings --raw --output SOFT,HARD | /usr/bin/tr ' ' /)"
}

# A process that outlives the command that started it, in a session of its own, holding none of
# its output: `name` is how the canary finds it.
probe_outlive() {
  /usr/bin/setsid /usr/bin/bash -c 'exec -a "$0" /usr/bin/sleep 600' "$1" < /dev/null > /dev/null 2>&1 &
  /usr/bin/sleep 0.2
}

# The canary: before any test, on this runner. Each route a test must not have is open to the
# runner's user, so that the probe proves something, and closed in the sandbox; each the tests
# need is open in both. A process left behind fails its command and ends with it, and the wrapper's
# end, by SIGTERM as nextest stops a test and by SIGKILL, before the command has started or after,
# ends the namespace. No crash in it starts the runner's core handler.
check() {
  local work marker decoy listener shm port tool args control confined route state failed=0 status wrapper start direct inside k started handled delay
  declare -A outside in_sandbox
  if [ "${RUNNER_OS:-}" != Linux ]; then
    echo "No sandbox on ${RUNNER_OS:-this machine}."
    return 0
  fi
  problem() {
    echo "::error::$1"
    failed=1
  }
  # Whether a process whose command line starts with a name is alive.
  alive() { /usr/bin/pgrep -f "^$1" > /dev/null; }
  # Up to a minute for a condition, which holds at once on a runner in good order: one whose disk
  # was slow took over five seconds to start node, and failed the canary for nothing.
  within() {
    local k
    for k in $(/usr/bin/seq 600); do
      if "$@"; then return 0; fi
      /usr/bin/sleep 0.1
    done
    return 1
  }
  gone() { ! /usr/bin/pgrep -f "$1" > /dev/null; }
  started=$(/usr/bin/date +%s)
  work=$RUNNER_TEMP/test-sandbox-check
  /usr/bin/rm -rf "$work"
  /usr/bin/mkdir -p "$work" target/tmp target/test-sandbox-probe
  marker=test-sandbox-$$-$RANDOM
  tool=target/test-sandbox-probe/tool
  printf '#!/bin/sh\n' > "$tool"
  : > "$work/record"
  # A process of the runner's user that stands for cargo and nextest, a shared memory segment of
  # that user, and a listener on the runner's loopback.
  TEST_SANDBOX_DECOY=$marker /usr/bin/sleep 600 &
  decoy=$!
  shm=$(/usr/bin/ipcmk -M 4096 | /usr/bin/tr -d -c 0-9)
  node -e 'require("net").createServer((s) => s.end()).listen(0, "127.0.0.1", function () { console.log(this.address().port); })' > "$work/port" &
  listener=$!
  # None outlives the canary, however it ends. They are named now, in double quotes: when the
  # script ends, this function's variables are gone.
  # shellcheck disable=SC2064
  trap "kill $decoy $listener 2> /dev/null || true; /usr/bin/ipcrm -m $shm 2> /dev/null || true" EXIT
  if ! within test -s "$work/port"; then fail "the canary's listener did not start"; fi
  port=$(< "$work/port")
  args=(
    "toolchain=$(/usr/bin/dirname "$(/usr/bin/realpath "$(rustup which rustc)")")"
    "cargo=${CARGO_HOME:-$HOME/.cargo}/bin"
    "tool=$tool" "env=$GITHUB_ENV" "path=$GITHUB_PATH" "output=$GITHUB_OUTPUT" "summary=$GITHUB_STEP_SUMMARY"
    "record=$work/record" "decoy=$decoy" "marker=$marker" "shm=$shm" "port=$port"
  )
  # A variable the tests read crosses into the sandbox; one of the job does not.
  export MENTE_TEST_SANDBOX_PROBE=$marker TEST_SANDBOX_JOB=$marker
  control=$(ulimit -v 6291456 -f 1048576 && /usr/bin/bash .github/scripts/test-sandbox.sh probe "${args[@]}")
  confined=$(ulimit -v 6291456 -f 1048576 && /usr/bin/bash "$self" run /usr/bin/bash .github/scripts/test-sandbox.sh probe "${args[@]}") \
    || problem "the probe did not run in the sandbox"
  while read -r route state; do outside[$route]=$state; done <<< "$control"
  while read -r route state; do in_sandbox[$route]=$state; done <<< "$confined"
  {
    echo "### The sandbox of the tests, on this runner"
    echo
    echo "| Route | The runner's user | In the sandbox |"
    echo "| :--- | :--- | :--- |"
    while read -r route state; do echo "| $route | $state | ${in_sandbox[$route]:-not reported} |"; done <<< "$control"
  } | /usr/bin/tee -a "$GITHUB_STEP_SUMMARY"
  for route in toolchain cargo-home git git-history github config script toolchain-file nextest-settings target-file target \
    github-env github-path github-output step-summary record sudo privileges signal trace ipc devices \
    network-interface network job-variables core-file; do
    if [ "${outside[$route]:-}" != open ]; then problem "the canary's probe could not reach $route as the runner's user, so its run in the sandbox proves nothing"; fi
    if [ "${in_sandbox[$route]:-}" != closed ]; then problem "the sandbox let the probe reach $route: ${in_sandbox[$route]:-not reported}"; fi
  done
  # Closed in the sandbox whatever the runner's own user may do: Ubuntu keeps capabilities from a
  # user namespace of any unprivileged process, so the probe has no run that shows it open.
  for route in user-namespace; do
    if [ "${in_sandbox[$route]:-}" != closed ]; then problem "the sandbox let the probe reach $route: ${in_sandbox[$route]:-not reported}"; fi
  done
  for route in read temp target-tmp child linker variables; do
    if [ "${in_sandbox[$route]:-}" != open ]; then problem "the sandbox kept the probe from $route, which the tests need"; fi
  done
  if [ "${in_sandbox[uid]:-}" != "$uid" ] || [ "${outside[uid]:-}" = "$uid" ]; then problem "the probe ran in the sandbox as the user ${in_sandbox[uid]:-not reported}, not $uid"; fi
  if [ "${in_sandbox[limits]:-}" != 6291456/1048576 ]; then problem "the sandbox ran the probe under the limits ${in_sandbox[limits]:-not reported}, not its caller's"; fi
  if [ "${in_sandbox[core-limit]:-}" != 1/1 ]; then problem "a crash in the sandbox could write a core file: its core limit is ${in_sandbox[core-limit]:-not reported}, not one byte"; fi

  # A process left behind: the runner's user keeps it; the sandbox ends it and fails the command.
  /usr/bin/bash .github/scripts/test-sandbox.sh probe-outlive "$marker-outlives"
  if ! within alive "$marker-outlives"; then problem "the canary's process did not outlive its command as the runner's user, so its run in the sandbox proves nothing"; fi
  /usr/bin/pkill -f "^$marker-outlives" || true
  status=0
  /usr/bin/bash "$self" run /usr/bin/bash .github/scripts/test-sandbox.sh probe-outlive "$marker-outlives" 2> "$work/outlive.txt" || status=$?
  /usr/bin/cat "$work/outlive.txt"
  if alive "$marker-outlives"; then problem "a process outlived its command in the sandbox"; fi
  if [ "$status" = 0 ] || ! /usr/bin/grep -q "left processes behind" "$work/outlive.txt"; then problem "a command that left a process behind passed in the sandbox (status $status)"; fi

  # The wrapper stopped, as nextest stops a test past its time, and killed.
  for signal in TERM KILL; do
    /usr/bin/bash "$self" run /usr/bin/bash -c 'exec -a "$0" /usr/bin/sleep 600' "$marker-$signal" &
    wrapper=$!
    if ! within alive "$marker-$signal"; then problem "the canary's command did not start in the sandbox"; fi
    kill "-$signal" "$wrapper"
    status=0
    wait "$wrapper" || status=$?
    if ! within gone "$marker-$signal"; then
      problem "SIG$signal to the wrapper left its command running in the sandbox"
      /usr/bin/sudo -n /usr/bin/pkill -KILL -f "$marker-$signal" || true
    fi
    echo "SIG$signal to the wrapper: status $status, and its command ended."
  done
  # The wrapper stopped before its command has started: the command must not run on without it.
  for delay in 0.005 0.015 0.03; do
    /usr/bin/bash "$self" run /usr/bin/bash -c 'exec -a "$0" /usr/bin/sleep 600' "$marker-early" &
    wrapper=$!
    /usr/bin/sleep "$delay"
    kill -TERM "$wrapper" 2> /dev/null || true
    wait "$wrapper" || true
  done
  if ! within gone "$marker-early"; then
    problem "a wrapper stopped before its command had started left the command running in the sandbox"
    /usr/bin/sudo -n /usr/bin/pkill -KILL -f "$marker-early" || true
  fi

  # What the sandbox costs a process.
  start=$(/usr/bin/date +%s%N)
  for k in $(/usr/bin/seq 20); do /usr/bin/true; done
  direct=$((($(/usr/bin/date +%s%N) - start) / 20000))
  start=$(/usr/bin/date +%s%N)
  for k in $(/usr/bin/seq 20); do /usr/bin/bash "$self" run /usr/bin/true; done
  inside=$((($(/usr/bin/date +%s%N) - start) / 20000))
  {
    echo
    echo "A process started and ended, mean of 20: $((direct / 1000)).$(printf '%03d' $((direct % 1000))) ms as the runner's user, $((inside / 1000)).$(printf '%03d' $((inside % 1000))) ms in the sandbox."
  } | /usr/bin/tee -a "$GITHUB_STEP_SUMMARY"

  # The probe crashed two processes in the sandbox, one at a core limit it had lowered to 0. The
  # pattern is a file name, so the kernel started no handler, and the runner's journal holds none.
  case "$(< /proc/sys/kernel/core_pattern)" in
    \|*) problem "a crash would start the runner's core handler: $(< /proc/sys/kernel/core_pattern)" ;;
  esac
  handled=$(/usr/bin/sudo -n /usr/bin/journalctl -q --no-pager -o cat -t systemd-coredump --since "@$started" 2> /dev/null || true)
  if [ -n "$handled" ]; then problem "a crash of the canary started the runner's core handler: $handled"; fi

  kill "$decoy" "$listener" 2> /dev/null || true
  /usr/bin/ipcrm -m "$shm" 2> /dev/null || true
  /usr/bin/rm -rf "$work" target/test-sandbox-probe
  if [ "$failed" != 0 ]; then exit 1; fi
  echo "The sandbox denies the tests every route the canary tries, and gives them what they need."
}


case "${1:-}" in
  setup) setup ;;
  setup-as-root) setup_as_root "$2" "$3" ;;
  run)
    shift
    run "$@"
    ;;
  inner)
    shift
    inner "$@"
    ;;
  env) env_lines ;;
  uid) echo "$uid" ;;
  ran) ran "${2:-}" ;;
  probe)
    shift
    probe "$@"
    ;;
  probe-outlive) probe_outlive "$2" ;;
  check) check ;;
  *)
    echo "usage: /usr/bin/bash .github/scripts/test-sandbox.sh setup|check|env|ran [n]"
    exit 2
    ;;
esac
