#!/bin/sh
# Runs a test command with a temp folder of its own (TMPDIR, and TMP and TEMP for
# Windows tools) and fails when the command leaves anything in it. Every test
# removes what it made, whether it passed, failed or panicked: a folder one test
# owns is a `tempfile::TempDir` (or a node `mkdtemp` removed when its server exits),
# one shared by a whole run is `tests_common::folder_for_the_run`.
#
#   bin/check-test-leftovers.sh cargo test --workspace
#   bin/check-test-leftovers.sh npm run e2e
#
# The command's own failure is its exit status; leftovers alone exit 1. Build
# first: a compiler or bundler is not under test, and its own temp files are not
# what this holds.
set -u
parent=${TMPDIR:-/tmp}
fresh=$(mktemp -d "${parent%/}/bh-leftover-check.XXXXXX") || exit 1
# the tools' own caches, kept between runs on purpose, stay where they always are:
# Node's compiled modules and Playwright's transformed specs
NODE_COMPILE_CACHE=${NODE_COMPILE_CACHE:-${parent%/}/node-compile-cache} \
PWTEST_CACHE_DIR=${PWTEST_CACHE_DIR:-${parent%/}/playwright-transform-cache-$(id -u)} \
TMPDIR=$fresh TMP=$fresh TEMP=$fresh "$@"
status=$?
if [ -n "$(ls -A "$fresh")" ]; then
  echo "check-test-leftovers: '$*' left these in its temp folder; each test must remove what it made:" >&2
  ls -A "$fresh" | sed -n 1,100p >&2
  rm -rf "$fresh"
  [ "$status" -ne 0 ] && exit "$status"
  exit 1
fi
rmdir "$fresh"
exit "$status"
