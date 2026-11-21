#!/usr/bin/env bash
# The step summary of a run of mn fuzz (fuzz.yml and fuzz-nightly.yml): the campaign's summary, then
# each failure's description and minimized input. GitHub refuses a step summary past 1024k and then
# shows none of it, so every part is capped by lines and by bytes, the whole by bytes, and each cut
# is said with what it left out (2026-11-21: a minimized input of 3 MB on one line, which a cap of
# 40 lines let through, aborted the nightly's summary). The whole of every file is in the run's
# artifact. The run tests feed it that input.
#
#   bash .github/scripts/fuzz-summary.sh <fuzz output> <title> <mn fuzz's status> >> "$GITHUB_STEP_SUMMARY"
set -euo pipefail
export LC_ALL=C

if [ "$#" -ne 3 ]; then
  echo "usage: fuzz-summary.sh <fuzz output> <title> <mn fuzz's status>" >&2
  exit 2
fi
out=$1 title=$2 status=$3
# What one part may show, and what the whole may, well under GitHub's 1024k for a step's summary.
PART_LINES=40
PART_BYTES=4096
TOTAL_BYTES=$((512 * 1024))
# The fence the parts stand in: tildes, which neither a Mente program nor mn's report writes.
FENCE='~~~~~~~~'
written=0

say() {
  printf '%s\n' "$1"
  written=$((written + ${#1} + 1))
}

# Every file's size, from one run of wc over all of them, since a summary may show hundreds.
declare -A sizes
while read -r size name; do
  if [ "$name" != total ]; then sizes[$name]=$size; fi
done < <(wc -c "$out"/*.txt "$out"/*.mn 2> /dev/null || true)

# A file, at most its first PART_BYTES bytes, read by the shell itself without reading the rest, and
# of those at most PART_LINES lines; and what was cut.
show() {
  local file=$1 size=${sizes[$1]:-0} head='' part='' n
  IFS= read -r -d '' -N "$PART_BYTES" head < "$file" || true
  for ((n = 0; n < PART_LINES && ${#head} > 0; n++)); do
    if [[ $head == *$'\n'* ]]; then
      part+=${head%%$'\n'*}$'\n'
      head=${head#*$'\n'}
    else
      part+=$head
      head=''
    fi
  done
  printf '%s' "$part"
  case "$part" in *$'\n' | '') ;; *) printf '\n' ;; esac
  written=$((written + ${#part} + 1))
  if [ "${#part}" -lt "$size" ]; then
    say "--- cut here: $((size - ${#part})) of its $size bytes are past $PART_LINES lines or $PART_BYTES bytes, and in the run's artifact"
  fi
}

say "## $title"
say "${FENCE}text"
if [ -f "$out/summary.txt" ]; then show "$out/summary.txt"; else say "no summary: mn fuzz exited with $status"; fi
say "$FENCE"
failures=()
for about in "$out"/*.txt; do
  case "$about" in */summary.txt) continue ;; esac
  [ -f "$about" ] || continue
  failures+=("$about")
done
shown=0
for about in "${failures[@]}"; do
  if [ "$written" -ge "$TOTAL_BYTES" ]; then
    say "--- $((${#failures[@]} - shown)) more failures are not shown here, past $TOTAL_BYTES bytes of this summary, and in the run's artifact"
    break
  fi
  say "### ${about#"$out"/}"
  say "${FENCE}text"
  show "$about"
  say "--- minimized input:"
  if [ -f "${about%.txt}.mn" ]; then show "${about%.txt}.mn"; else say "(none)"; fi
  say "$FENCE"
  shown=$((shown + 1))
done
