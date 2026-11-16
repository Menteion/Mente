#!/usr/bin/env bash
# Fetches the commit of the held-out set that benchmark/corpus.lock names into held-out-set and
# checks it out, printing nothing of the held-out repository: no commit subject, file or name. The
# job's log is public on this public repository, and actions/checkout printed "HEAD is now at <commit>
# <subject>" there (run 3 of the plan, 2026-11-16). Every line it prints is its own words.
#
# Its step starts it under `env -i`, with PATH, a fresh HOME, the token and the commit alone, so that
# nothing the job's environment holds (SHELLOPTS, BASH_ENV, git's own settings) reaches it; and git
# reads no system or global configuration either. The token reaches git as an extra header through
# git's config environment, masked first; it is never on a command line, in held-out-set/.git/config,
# or in any line this script prints. held-out-set must not be there before it, in any form: a link
# there would have taken the checkout wherever it pointed (the review of aafe59b).
set -euo pipefail
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
: "${HELD_OUT_TOKEN:?the held-out token is not set}"
: "${HELD_OUT_COMMIT:?no held-out commit from corpus.lock}"
if ! [[ "$HELD_OUT_COMMIT" =~ ^[0-9a-f]{40}$ ]]; then
  echo "the held-out commit corpus.lock names is no full commit id"
  exit 1
fi
if [ -e held-out-set ] || [ -L held-out-set ]; then
  echo "held-out-set is already there: the held-out set goes into a directory this script makes"
  exit 1
fi
mkdir held-out-set
auth="$(printf 'x-access-token:%s' "$HELD_OUT_TOKEN" | base64 -w0)"
echo "::add-mask::$auth"
git init -q held-out-set > /dev/null 2>&1 || { echo "the held-out set's repository could not be made"; exit 1; }
GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0="http.https://github.com/.extraheader" GIT_CONFIG_VALUE_0="AUTHORIZATION: basic $auth" \
  git -C held-out-set fetch -q --no-tags --depth=1 https://github.com/Menteion/mente-bench-heldout "$HELD_OUT_COMMIT" > /dev/null 2>&1 || { echo "the held-out set could not be fetched"; exit 1; }
unset auth HELD_OUT_TOKEN
git -C held-out-set -c advice.detachedHead=false checkout -q --detach FETCH_HEAD > /dev/null 2>&1 || { echo "the held-out set could not be checked out"; exit 1; }
if [ "$(git -C held-out-set rev-parse HEAD 2> /dev/null)" != "$HELD_OUT_COMMIT" ]; then
  echo "the held-out set is not at the commit corpus.lock names"
  exit 1
fi
echo "the held-out set is at the commit corpus.lock names"
