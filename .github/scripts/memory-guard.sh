# The guard on the runner's memory that .github/scripts/limits.sh starts beside the tests on Linux.
# Shards 11 and 12 of the full mutation runs lost their runners when a mutant made several tests at
# once take all 16 GB within a minute, each under the limit per process. Once a second, below 2 GB
# available, the process taking the most memory, the runaway test, is stopped, so that its test
# fails. Memory and disk go to the job's log once a minute.
#
# No command's failure stops the guard: errexit and pipefail are off here, though the step that
# starts it runs with both, so a process that ends between `ps` and `kill` costs one round, not
# the guard. When the guard was a subshell of that step, the first such `kill` ended it silently.
# The workflow policy (.github/scripts/policy.test.mjs) sources it under errexit against a process
# that is gone and sees it go on; its two arguments, the meminfo file and the seconds between
# rounds, exist for that test and are never given in a workflow.
set +e +o pipefail
meminfo=${1:-/proc/meminfo}
interval=${2:-1}
tick=0
while sleep "$interval"; do
  avail=$(awk '/^MemAvailable:/ {print int($2 / 1024)}' "$meminfo")
  if [ "$avail" -lt 2048 ]; then
    pid=$(ps -eo pid,rss --sort=-rss --no-headers | awk 'NR == 1 {print $1}')
    echo "memory guard: $avail MB available, stopping $(ps -o comm= -p "$pid") ($pid)"
    kill -9 "$pid"
  fi
  tick=$((tick + 1))
  if [ $((tick % 60)) -eq 0 ]; then
    echo "resources: $(free -m | awk '/^Mem:/ {print $3 " of " $2 " MB used"}'), $(df -h / | awk 'NR == 2 {print $4 " of disk free"}')"
  fi
done
