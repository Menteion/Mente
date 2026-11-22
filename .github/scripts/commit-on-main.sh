#!/usr/bin/env bash
# Fails unless the commit named by its argument is one of main's own: a commit of main's first-parent
# chain, as the checkout fetched it (refs/remotes/origin/main, from a checkout with fetch-depth: 0),
# each a tree main itself stood at, which the merge queue made and tested whole. An ancestor is not
# enough: every commit inside a pull request main merged is one too, yet none of them was ever main,
# and its tree was never reviewed or tested as a whole (the re-review of 40bb29a). The site's
# publication runs it before anything of the tag (publish-tag.sh, pages-publish.yml). The run tests
# plant a tag on a pull request's head, on a commit inside a merged pull request, and on main.
#
#   bash .github/scripts/commit-on-main.sh <commit or tag>
set -euo pipefail

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
  echo "::error::usage: commit-on-main.sh <commit or tag>"
  exit 2
fi
commit=$(git rev-parse --verify --quiet "$1^{commit}") || { echo "::error::$1 names no commit"; exit 1; }
main=$(git rev-parse --verify --quiet "refs/remotes/origin/main^{commit}") || { echo "::error::no origin/main to check $1 against: check out with fetch-depth: 0"; exit 1; }
# The whole chain is read, and searched for the commit by its full id: grep reads all of it, so no
# early end of the pipe can fail rev-list, and a commit is no prefix of another's id.
found=$(git rev-list --first-parent "$main" | grep -Fx -- "$commit" || true)
if [ "$found" = "$commit" ]; then
  echo "$1 ($commit) is a commit of main's own history, its first-parent chain, at $main"
else
  echo "::error::$1 ($commit) is no commit of main's own history, its first-parent chain: tag a commit main itself stood at, as the merge queue made it, not one inside a pull request"
  exit 1
fi
