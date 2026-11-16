#!/usr/bin/env bash
# Fails unless the commit named by its argument is in the history of main, as the checkout fetched
# it (refs/remotes/origin/main, from a checkout with fetch-depth: 0): the Pages deployment runs it
# before it publishes, so that a milestone tag on a commit main never took, a pull request's head
# say, publishes nothing (pages-publish.yml). The run tests plant both.
#
#   bash .github/scripts/commit-on-main.sh <commit or tag>
set -euo pipefail

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo "::error::usage: commit-on-main.sh <commit or tag>"
  exit 2
fi
commit=$(git rev-parse --verify --quiet "$1^{commit}") || { echo "::error::$1 names no commit"; exit 1; }
main=$(git rev-parse --verify --quiet "refs/remotes/origin/main^{commit}") || { echo "::error::no origin/main to check $1 against: check out with fetch-depth: 0"; exit 1; }
if git merge-base --is-ancestor "$commit" "$main"; then
  echo "$1 ($commit) is in main's history, at $main"
else
  echo "::error::$1 ($commit) is not in main's history: tag a commit of main, as the merge queue merged it"
  exit 1
fi
