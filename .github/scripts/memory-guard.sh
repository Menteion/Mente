# The guard on the runner's memory that .github/scripts/limits.sh starts beside the tests on Linux.
# Shards 11 and 12 of the full mutation runs lost their runners when a mutant made several tests at
# once take all 16 GB within a minute, each under the limit per process. Once a second, below 2 GB
# available, the process taking the most memory, the runaway test, is stopped, so that its test
# fails. Memory and disk go to the job's log once a minute.
#
# A test runs in the sandbox as a user of its own (test-sandbox.sh), whose processes the runner's
# user cannot stop: for those, and only those, the guard stops the process through sudo.
#
# No command's failure stops the guard: errexit and pipefail are off here, though the step that
# starts it runs with both, so a process that ends between `ps` and `kill` costs one round, not
# the guard. When the guard was a subshell of that step, the first such `kill` ended it silently.
# The workflow policy (.github/scripts/policy.test.mjs) sources it under errexit, with `ps` naming
# a process that is gone and `sleep` counting the rounds, and requires every round to run; its
# argument, the meminfo file, exists for that check and is never given in a workflow.
set +e +o pipefail
meminfo=${1:-/proc/meminfo}
sandbox_uid=$(/usr/bin/bash .github/scripts/test-sandbox.sh uid)
tick=0
while sleep 1; do
  avail=$(awk '/^MemAvailable:/ {print int($2 / 1024)}' "$meminfo")
  if [ "$avail" -lt 2048 ]; then
    pid=$(ps -eo pid,rss --sort=-rss --no-headers | awk 'NR == 1 {print $1}')
    echo "memory guard: $avail MB available, stopping $(ps -o comm= -p "$pid") ($pid)"
    if [ "$(ps -o uid= -p "$pid" | tr -d ' ')" = "$sandbox_uid" ]; then sudo -n kill -9 "$pid"; else kill -9 "$pid"; fi
  fi
  tick=$((tick + 1))
  if [ $((tick % 60)) -eq 0 ]; then
    echo "resources: $(free -m | awk '/^Mem:/ {print $3 " of " $2 " MB used"}'), $(df -h / | awk 'NR == 2 {print $4 " of disk free"}')"
  fi
done
