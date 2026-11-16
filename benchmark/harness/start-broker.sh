#!/usr/bin/env bash
# Starts the broker, the one process of a benchmark run that holds the API key
# (benchmark/harness/broker.ts; docs/design-review.md, untrusted-test-isolation), in the
# background, and ends once it listens, so that the shell that held the key ends with this step.
#
#   ANTHROPIC_API_KEY=<key> BUDGET=<dollars> bash benchmark/harness/start-broker.sh
#
# The key reaches the broker on its standard input and in no environment: the broker runs with an
# environment cleared but for PATH (env -i, which also drops NODE_OPTIONS, whose
# --allow-child-process would widen the permission model), under the flags of
# benchmark/harness/permission.json, which the workflow policy holds the node line below to. It
# may write no core file at any size, a limit no process of the runner's user can raise again,
# since `ulimit -c 0` sets the hard limit too; and its directory, its socket, its log and its pid
# file are the runner's user's alone (mode 0700 and 0600). The sandbox covers the directory, so no
# agent code reaches the socket.
set -euo pipefail
: "${ANTHROPIC_API_KEY:?the key, from the env of the step}" "${BUDGET:?the budget of the run, in US dollars}"
: "${RUNNER_TEMP:?the temporary directory of the runner}" "${GITHUB_WORKSPACE:?the workspace}"
dir="$RUNNER_TEMP/broker"
ulimit -c 0
umask 0177
mkdir -m 0700 "$dir"
printf '%s\n' "$ANTHROPIC_API_KEY" | env -i PATH="$PATH" node --disallow-code-generation-from-strings --no-experimental-sqlite --permission --allow-fs-read="$GITHUB_WORKSPACE/benchmark/harness/broker.ts" --allow-fs-read="$GITHUB_WORKSPACE/benchmark/harness/broker-limits.json" "$GITHUB_WORKSPACE/benchmark/harness/broker.ts" --socket "$dir/socket" --budget "$BUDGET" > "$dir/log" 2>&1 &
echo "$!" > "$dir/pid"
for _ in $(seq 100); do
  if [ -S "$dir/socket" ]; then break; fi
  sleep 0.1
done
cat "$dir/log"
if [ ! -S "$dir/socket" ]; then
  echo "::error::the broker did not start"
  exit 1
fi
if [ "$(stat -c %a "$dir")" != 700 ] || [ "$(stat -c %a "$dir/socket")" != 600 ]; then
  echo "::error::the broker's directory or socket is open to more than the runner's user: $(stat -c '%n %a' "$dir" "$dir/socket" | tr '\n' ' ')"
  exit 1
fi
