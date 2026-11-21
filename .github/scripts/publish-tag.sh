#!/usr/bin/env bash
# Checks out the milestone tag a run of pages-publish.yml was given, once its commit is proven in
# main's history (.github/scripts/commit-on-main.sh), so that only a tag on a commit main took is
# built and published. The tag comes in TAG, from the dispatch's input through the step's env, never
# spliced into a script; it must be a milestone's name, `m` and a number with at most one letter
# after it (m0a, m1), before git reads it at all. The run tests plant a tag outside main, a name of
# another shape and none at all.
#
#   TAG=m1 bash .github/scripts/publish-tag.sh
set -euo pipefail

tag="${TAG-}"
if ! [[ "$tag" =~ ^m[0-9]+[a-z]?$ ]]; then
  echo "::error::the tag to publish is $(printf '%q' "$tag"), no milestone's name: give one such as m1"
  exit 1
fi
"$BASH" .github/scripts/commit-on-main.sh "refs/tags/$tag"
git -c advice.detachedHead=false checkout --quiet --detach "refs/tags/$tag"
echo "checked out $tag at $(git rev-parse HEAD)"
