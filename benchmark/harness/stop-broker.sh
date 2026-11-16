#!/usr/bin/env bash
# Stops the broker benchmark/harness/start-broker.sh started, whatever came before, and prints its
# log, which holds no key: the broker takes the key out of every byte it writes.
#
#   bash benchmark/harness/stop-broker.sh
set -euo pipefail
dir="${RUNNER_TEMP:?the temporary directory of the runner}/broker"
if [ -f "$dir/pid" ]; then kill "$(cat "$dir/pid")" 2> /dev/null || true; fi
if [ -f "$dir/log" ]; then cat "$dir/log"; fi
