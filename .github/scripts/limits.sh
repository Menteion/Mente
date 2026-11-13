# The limits the compiler's tests run under where a required check or a mutation run runs them:
# sourced by the step that runs them (ci.yml's Rust jobs, mutants.yml and mutants-full.yml), so
# that they hold for every process the step starts after. They are defined here once; the
# workflow policy (.github/scripts/policy.test.mjs) checks that each of those steps sources this
# file before its tests, and that the limits stay as they are.
#
# A test that allocates or writes without end would take the runner down with it, as a mutant
# took two shards of the first full mutation run. Each test process may reserve at most 8 GB and,
# on Linux, write a file of at most 2 GB, so that such a test fails instead. A runner has 16 GB
# and nextest runs a test on each of its 4 CPUs, so each runner also bounds them together:
#
# - Linux: ulimit for each process, and a guard on the runner's memory. Shards 11 and 12 of the
#   full runs lost their runners when a mutant made several tests at once take all 16 GB within a
#   minute, each under the limit above. Once a second, below 2 GB available, the process taking
#   the most memory, the runaway test, is stopped, so that its test fails. Memory and disk go to
#   the job's log once a minute.
# - Windows: a job object holding the step's shell and every process it starts after, each
#   committing at most 8 GB and all of them together at most 12 GB (job-limits.ps1).
case "$RUNNER_OS" in
  Linux)
    ulimit -v 8388608 -f 2097152
    (tick=0; while sleep 1; do
      avail=$(awk '/^MemAvailable:/ {print int($2 / 1024)}' /proc/meminfo)
      if [ "$avail" -lt 2048 ]; then
        pid=$(ps -eo pid,rss --sort=-rss --no-headers | awk 'NR == 1 {print $1}')
        echo "memory guard: $avail MB available, stopping $(ps -o comm= -p "$pid") ($pid)"
        kill -9 "$pid"
      fi
      tick=$((tick + 1))
      if [ $((tick % 60)) -eq 0 ]; then
        echo "resources: $(free -m | awk '/^Mem:/ {print $3 " of " $2 " MB used"}'), $(df -h / | awk 'NR == 2 {print $4 " of disk free"}')"
      fi
    done) &
    ;;
  Windows)
    pwsh -NoProfile -NonInteractive -File .github/scripts/job-limits.ps1 -ProcessId "$(cat "/proc/$$/winpid")" -ProcessBytes 8589934592 -JobBytes 12884901888
    ;;
  *)
    echo "::error::.github/scripts/limits.sh defines no limits for $RUNNER_OS"
    exit 1
    ;;
esac
