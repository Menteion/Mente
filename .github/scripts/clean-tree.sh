#!/usr/bin/env bash
# The check that the tree a job's gate commands judge is the commit's (the brief
# clean-tree-around-every-test-run; the escape plan-downloaded-into-the-checkout): run from the
# checkout's root, before a step's first gate command and after its last, it fails, naming each path,
# when git shows anything in the tree but the commit's files: an untracked file, a tracked file
# changed or staged, or an ignored file. What every walk of the tree leaves out is left out here too,
# read from the walks' own list (compiler/mente-cli/src/walk-skips.txt), with every directory that
# holds a .git of its own. A job writes its working files under $RUNNER_TEMP.
#
# It fails closed: git that fails, or a list that names no directory, fails the step. Each path is
# printed with %, a carriage return and a line feed escaped as a workflow command's data escapes
# them, so that no file's name can end the message or begin a command of its own.
set -euo pipefail

list=compiler/mente-cli/src/walk-skips.txt
skips=()
while IFS= read -r line || [ -n "$line" ]; do
  line=${line%$'\r'}
  case "$line" in '' | '#'*) continue ;; esac
  skips+=("$line")
done < "$list"
if [ "${#skips[@]}" -eq 0 ]; then
  echo "::error::$list names no directory to leave out"
  exit 1
fi

# git's report goes to a file outside the tree, read below and then removed; git that fails ends
# the script here, before any of it is read.
status=$(mktemp)
# No program a repository's configuration names (core.fsmonitor) runs, and no cache of untracked
# files stands in for the tree as it is.
git -c core.fsmonitor= -c core.untrackedCache=false status --porcelain=v1 -z --untracked-files=all --ignored=matching > "$status"

found=0
while IFS= read -r -d '' entry; do
  code=${entry:0:2}
  path=${entry:3}
  # A rename or a copy names its source in the field after it.
  case "$code" in R* | C*) IFS= read -r -d '' _ || true ;; esac
  first=${path%%/*}
  left=0
  for s in "${skips[@]}"; do
    if [ "$first" = "$s" ]; then left=1; fi
  done
  # As the walks do, a directory that holds a .git of its own, and all under it, is left out.
  dir=${path%/}
  while [ "$left" -eq 0 ]; do
    if [ -e "$dir/.git" ]; then left=1; fi
    case "$dir" in */*) dir=${dir%/*} ;; *) break ;; esac
  done
  if [ "$left" -eq 1 ]; then continue; fi
  data=${path//%/%25}
  data=${data//$'\r'/%0D}
  data=${data//$'\n'/%0A}
  echo "::error::the tree the gate commands judge is not the commit's: ${code} ${data}"
  found=$((found + 1))
done < "$status"
rm -f "$status"

if [ "$found" -gt 0 ]; then
  echo "::error::$found path(s) of the tree differ from the commit: a step wrote into the checkout; write a job's working files under \$RUNNER_TEMP"
  exit 1
fi
