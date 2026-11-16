#!/usr/bin/env bash
# Takes root from the runner's user for the rest of the job, but for the benchmark sandbox's root
# half (docs/design-review.md, untrusted-test-isolation). A GitHub-hosted runner gives its user
# passwordless sudo, and with it any process of that user could read any process's memory: the
# broker's, which holds the API key, or the runner's own, which holds the job's secrets. So, once
# the job is set up and before the sandbox runs anything or the key is in reach, this:
#   - installs a root-owned copy of benchmark/harness/sandbox-root.sh at /usr/local/sbin/
#     mente-sandbox, and writes the paths it always covers to /etc/mente-sandbox.conf, root's alone;
#   - sends every core dump nowhere, since a core of the broker or of the runner would hold a key;
#   - lets no process trace another but one with CAP_SYS_PTRACE (Yama's ptrace_scope 2), which no
#     process of the runner's user has once sudo is gone;
#   - stops the container daemons, whose sockets give whoever reaches them root;
#   - leaves sudo one rule for the runner's user, the sandbox's root half, and takes every other.
# Then it checks that root is out of reach, as --check does again at the job's end.
#
#   bash benchmark/harness/lock-sudo.sh           after setup, before the sandbox and the key
#   bash benchmark/harness/lock-sudo.sh --check   later: root is still out of the user's reach
set -euo pipefail

ENTRY=/usr/local/sbin/mente-sandbox
CONF=/etc/mente-sandbox.conf
RULE=/etc/sudoers.d/00-mente-sandbox
CORE='|/bin/false'
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

fail() {
  echo "::error::$1"
  exit 1
}

# What --check finds wrong, one line each; nothing when root is out of the user's reach.
problems() {
  if sudo -n true 2> /dev/null; then echo "sudo -n true still runs as root"; fi
  if sudo -n cat /etc/shadow > /dev/null 2>&1; then echo "sudo -n cat still reads what only root reads"; fi
  # The one rule sudo lists for this user is the sandbox's root half.
  local listed
  listed="$(sudo -n -l 2> /dev/null | awk '/may run the following commands/ { on = 1; next } on && NF { sub(/^[ \t]+/, ""); print }')"
  if [ "$listed" != "(root) NOPASSWD: $ENTRY" ]; then echo "sudo lists for $(id -un) something other than its one rule, the sandbox: ${listed:-nothing}"; fi
  if [ "$(stat -c %u:%g:%a "$ENTRY" 2> /dev/null)" != "0:0:755" ] || ! cmp -s "$ENTRY" "$here/sandbox-root.sh"; then echo "$ENTRY is not root's copy of sandbox-root.sh"; fi
  if [ "$(stat -c %u:%a "$CONF" 2> /dev/null)" != "0:644" ]; then echo "$CONF is not root's alone"; fi
  case "$(cat /proc/sys/kernel/yama/ptrace_scope 2> /dev/null)" in
    2 | 3) ;;
    *) echo "Yama's ptrace_scope is not 2 or 3, so a process of the user could trace another" ;;
  esac
  if [ "$(cat /proc/sys/kernel/core_pattern)" != "$CORE" ]; then echo "core dumps go somewhere: $(cat /proc/sys/kernel/core_pattern)"; fi
  # No socket that gives root is open to this user.
  local s
  for s in /run/docker.sock /var/run/docker.sock /run/containerd/containerd.sock /run/podman/podman.sock /var/run/libvirt/libvirt-sock /run/libvirt/libvirt-sock; do
    if [ -S "$s" ] && [ -w "$s" ]; then echo "$s, whose daemon gives root, is open to $(id -un)"; fi
  done
  # Nor a group that is root by another name.
  local g
  for g in $(id -Gn); do
    case "$g" in
      root | sudo | admin | wheel | disk | lxd | libvirt) echo "$(id -un) is in the group $g, which gives root" ;;
    esac
  done
}

check() {
  local found
  found="$(problems)"
  if [ -n "$found" ]; then
    printf '::error::%s\n' "${found//$'\n'/$'\n'::error::}"
    exit 1
  fi
  echo "root is out of $(id -un)'s reach, but for $ENTRY"
}

lock() {
  [ "$(id -u)" != 0 ] || fail "run it as the runner's user, whose sudo it takes"
  : "${GITHUB_WORKSPACE:?the workspace, which the sandbox covers}" "${RUNNER_TEMP:?the temporary directory of the runner, where the broker listens}"
  sudo -n install -o root -g root -m 0755 "$here/sandbox-root.sh" "$ENTRY"
  printf '%s\n' "$GITHUB_WORKSPACE" "$RUNNER_TEMP/broker" | sudo -n tee "$CONF" > /dev/null
  sudo -n chown root:root "$CONF"
  sudo -n chmod 0644 "$CONF"
  sudo -n sysctl -q -w "kernel.core_pattern=$CORE"
  sudo -n sysctl -q -w kernel.yama.ptrace_scope=2
  # A unit the image lacks fails to stop; problems() below finds any socket left open.
  local unit
  for unit in docker.socket docker.service containerd.service podman.socket libvirtd.service; do
    sudo -n systemctl stop "$unit" 2> /dev/null || true
  done
  sudo -n rm -f /run/docker.sock /var/run/docker.sock
  # Last, in one run of sudo: the one rule, checked before it stands, and every other rule moved
  # out of reach, so that this is the last command sudo runs as root for this user but the sandbox.
  sudo -n /bin/bash -c '
    set -euo pipefail
    user="$1"; entry="$2"; rule="$3"
    printf "%s ALL=(root) NOPASSWD: %s\n" "$user" "$entry" > "$rule.new"
    chmod 0440 "$rule.new"
    visudo -cqf "$rule.new"
    mkdir -p -m 0700 /root/sudoers.d.off
    for f in /etc/sudoers.d/*; do
      if [ "$f" != "$rule" ] && [ "$f" != "$rule.new" ]; then mv "$f" /root/sudoers.d.off/; fi
    done
    mv "$rule.new" "$rule"
    visudo -cq
  ' lock "$(id -un)" "$ENTRY" "$RULE"
  check
}

case "${1:-}" in
  "") lock ;;
  --check) check ;;
  *)
    echo "usage: lock-sudo.sh [--check]" >&2
    exit 2
    ;;
esac
