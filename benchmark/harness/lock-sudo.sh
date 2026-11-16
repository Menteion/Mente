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
#   - lets no unprivileged process watch another through perf (perf_event_paranoid at its highest);
#   - leaves sudo one rule for the runner's user, the sandbox's root half by its SHA-256, so that
#     sudo itself refuses a changed file, with its environment reset and its path fixed, and takes
#     every other rule.
# Then it checks that root is out of reach, as --check does again at the job's end.
#
#   bash benchmark/harness/lock-sudo.sh           after setup, before the sandbox and the key
#   bash benchmark/harness/lock-sudo.sh --check   later: root is still out of the user's reach
set -euo pipefail

ENTRY=/usr/local/sbin/mente-sandbox
CONF=/etc/mente-sandbox.conf
RULE=/etc/sudoers.d/00-mente-sandbox
CORE='|/bin/false'
SECURE_PATH=/usr/sbin:/usr/bin:/sbin:/bin
# The file capabilities a program on the user's path may carry: the network's alone, none of which
# reads another process or the files of another user.
NETWORK_CAPS=" cap_net_raw cap_net_admin cap_net_bind_service "
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
digest="$(sha256sum "$here/sandbox-root.sh" | cut -d' ' -f1)"

fail() {
  echo "::error::$1"
  exit 1
}

# What --check finds wrong, one line each; nothing when root is out of the user's reach.
problems() {
  if sudo -n true 2> /dev/null; then echo "sudo -n true still runs as root"; fi
  if sudo -n cat /etc/shadow > /dev/null 2>&1; then echo "sudo -n cat still reads what only root reads"; fi
  # The one rule sudo lists for this user is the sandbox's root half, by its digest; and the last of
  # the defaults it lists, which win, reset the environment, keep none of it, let no caller set it
  # and fix the path, on which the root half's own shell relies.
  local listing listed defaults
  listing="$(sudo -n -l 2> /dev/null)"
  listed="$(awk '/may run the following commands/ { on = 1; next } on && NF { sub(/^[ \t]+/, ""); print }' <<< "$listing")"
  if [ "$listed" != "(root) NOPASSWD: sha256:$digest $ENTRY" ]; then echo "sudo lists for $(id -un) something other than its one rule, the sandbox by its digest: ${listed:-nothing}"; fi
  defaults="$(awk '/^Matching Defaults entries/ { getline; sub(/^[ \t]+/, ""); print; exit }' <<< "$listing")"
  case "$defaults" in
    *"env_reset, !env_keep, !setenv, secure_path=${SECURE_PATH//:/\\:}") ;;
    *) echo "sudo's defaults for $(id -un) do not end with env_reset, !env_keep, !setenv and secure_path=$SECURE_PATH: ${defaults:-none}" ;;
  esac
  # Root's alone, every directory above the root half, its configuration and sudo's rules, the
  # directories of the path sudo runs commands from, and those files themselves.
  local p d
  for p in "$ENTRY" "$CONF" /etc/sudoers "$RULE" ${SECURE_PATH//:/ }; do
    d="$p"
    while :; do
      local mode
      mode="$(stat -L -c %u:%a "$d" 2> /dev/null || true)"
      if [ -z "$mode" ]; then
        echo "$d, above or on what sudo runs as root, is missing"
      elif [ "${mode%%:*}" != 0 ] || (( (8#${mode#*:} & 8#022) != 0 )); then
        echo "$d, above or on what sudo runs as root, is not root's alone: $mode"
      fi
      [ "$d" = / ] && break
      d="$(dirname "$d")"
    done
  done
  if [ "$(stat -c %u:%g:%a "$ENTRY" 2> /dev/null)" != "0:0:755" ] || ! cmp -s "$ENTRY" "$here/sandbox-root.sh"; then echo "$ENTRY is not root's copy of sandbox-root.sh"; fi
  if [ "$(stat -c %u:%a "$CONF" 2> /dev/null)" != "0:644" ]; then echo "$CONF is not root's alone"; fi
  case "$(cat /proc/sys/kernel/yama/ptrace_scope 2> /dev/null)" in
    2 | 3) ;;
    *) echo "Yama's ptrace_scope is not 2 or 3, so a process of the user could trace another" ;;
  esac
  if [ "$(cat /proc/sys/kernel/core_pattern)" != "$CORE" ]; then echo "core dumps go somewhere: $(cat /proc/sys/kernel/core_pattern)"; fi
  if [ "$(cat /proc/sys/kernel/perf_event_paranoid 2> /dev/null || echo 0)" -lt 3 ]; then echo "perf_event_paranoid is below 3, so a process of the user could watch another through perf"; fi
  # No program on the user's path, or on sudo's, carries a capability that reads another process
  # or another user's files: the network's alone.
  local getcap=/usr/sbin/getcap line cap
  [ -x "$getcap" ] || getcap=/sbin/getcap
  if [ ! -x "$getcap" ]; then
    echo "getcap is not there to show the capabilities of the programs on the path"
  else
    local IFS_SAVED="$IFS" dirs
    IFS=: read -ra dirs <<< "$PATH:$SECURE_PATH"
    IFS="$IFS_SAVED"
    for d in "${dirs[@]}"; do
      [ -d "$d" ] || continue
      while IFS= read -r line; do
        [ -n "$line" ] || continue
        if [[ "$line" == *" ="* ]]; then echo "$line: every capability"; continue; fi
        for cap in $(grep -o 'cap_[a-z_]*' <<< "${line#* }"); do
          [[ "$NETWORK_CAPS" == *" $cap "* ]] || echo "$line: $cap"
        done
      done < <(find "$d" -maxdepth 1 -type f -exec "$getcap" {} + 2> /dev/null)
    done
  fi
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
  # problems() runs to its end whatever fails inside it, and says so last: an end it never reached,
  # as when a command it runs is missing, is a failure, never an empty list of problems.
  local found
  found="$(
    set +e
    problems
    echo "::end"
  )"
  if [ "${found##*$'\n'}" != "::end" ]; then fail "the check of root's reach did not run to its end: $found"; fi
  found="${found%::end}"
  found="${found%$'\n'}"
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
  # The highest level the kernel takes: 4 where the distribution's kernel knows it, else 3.
  sudo -n sysctl -q -w kernel.perf_event_paranoid=4 2> /dev/null || sudo -n sysctl -q -w kernel.perf_event_paranoid=3
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
    user="$1"; entry="$2"; rule="$3"; digest="$4"; path="$5"
    {
      printf "Defaults:%s env_reset, !env_keep, !setenv, secure_path=\"%s\"\n" "$user" "$path"
      printf "%s ALL=(root) NOPASSWD: sha256:%s %s\n" "$user" "$digest" "$entry"
    } > "$rule.new"
    chmod 0440 "$rule.new"
    visudo -cqf "$rule.new"
    mkdir -p -m 0700 /root/sudoers.d.off
    for f in /etc/sudoers.d/*; do
      if [ "$f" != "$rule" ] && [ "$f" != "$rule.new" ]; then mv "$f" /root/sudoers.d.off/; fi
    done
    mv "$rule.new" "$rule"
    visudo -cq
  ' lock "$(id -un)" "$ENTRY" "$RULE" "$digest" "$SECURE_PATH"
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
