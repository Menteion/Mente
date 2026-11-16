#!/usr/bin/env bash
# Starts the broker, the one process of a benchmark run that holds the API key
# (benchmark/harness/broker.ts; docs/design-review.md, untrusted-test-isolation), in the
# background, and ends once it listens, so that the shell that held the key ends with this step.
#
#   ANTHROPIC_API_KEY=<key> BUDGET=<dollars> /usr/bin/bash -p benchmark/harness/start-broker.sh
#
# Its step runs it in a privileged bash, which imports no exported function: one named `printf`
# would otherwise run in place of the builtin, with the key (the final pass on 89c7691).
#
# The key reaches the broker on its standard input and in no environment: the broker runs with an
# environment cleared but for PATH (env -i, which also drops NODE_OPTIONS, whose
# --allow-child-process would widen the permission model), under the flags of
# benchmark/harness/permission.json, which the workflow policy holds the node line below to. It
# may write no core file at any size, a limit no process of the runner's user can raise again,
# since `ulimit -c 0` sets the hard limit too; and its directory, its socket, its log and its pid
# file are the runner's user's alone (mode 0700 and 0600). The sandbox covers the directory, so no
# agent code reaches the socket. Every command it runs while the key is in its environment is named
# by its path, Node by the one setup-node put in the runner's tool cache at the version
# permission.json names, so that no program first on PATH receives the key (the re-check of c55cf8f).
set -euo pipefail
: "${ANTHROPIC_API_KEY:?the key, from the env of the step}" "${BUDGET:?the budget of the run, in US dollars}"
: "${RUNNER_TEMP:?the temporary directory of the runner}" "${GITHUB_WORKSPACE:?the workspace}" "${RUNNER_TOOL_CACHE:?the tool cache of the runner, where setup-node puts Node}"
dir="$RUNNER_TEMP/broker"
ulimit -c 0
umask 0177
/usr/bin/mkdir -m 0700 "$dir"
printf '%s\n' "$ANTHROPIC_API_KEY" | /usr/bin/env -i PATH="$PATH" "$RUNNER_TOOL_CACHE/node/24.22.1/x64/bin/node" --disallow-code-generation-from-strings --no-experimental-sqlite --permission --allow-fs-read="$GITHUB_WORKSPACE/benchmark/harness/broker.ts" --allow-fs-read="$GITHUB_WORKSPACE/benchmark/harness/broker-limits.json" "$GITHUB_WORKSPACE/benchmark/harness/broker.ts" --socket "$dir/socket" --budget "$BUDGET" > "$dir/log" 2>&1 &
echo "$!" > "$dir/pid"
for _ in $(/usr/bin/seq 100); do
  if [ -S "$dir/socket" ]; then break; fi
  /usr/bin/sleep 0.1
done
/usr/bin/cat "$dir/log"
if [ ! -S "$dir/socket" ]; then
  echo "::error::the broker did not start"
  exit 1
fi
if [ "$(/usr/bin/stat -c %a "$dir")" != 700 ] || [ "$(/usr/bin/stat -c %a "$dir/socket")" != 600 ]; then
  echo "::error::the broker's directory or socket is open to more than the runner's user: $(/usr/bin/stat -c '%n %a' "$dir" "$dir/socket" | /usr/bin/tr '\n' ' ')"
  exit 1
fi
