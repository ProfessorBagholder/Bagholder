#!/bin/sh
# Runs a test command with a temp folder of its own (TMPDIR, and TMP and TEMP for
# Windows) and fails when the command leaves anything in it. Every test removes
# what it made, whether it passed, failed or panicked: a folder one test owns is a
# `tempfile::TempDir`, one shared by a whole run is `tests_common::run_folder`.
#
#   bin/check-test-leftovers.sh cargo test --workspace --locked
#
# The command's own failure is its exit status; leftovers alone exit 1. Build
# first (`--no-run`): a compiler is not under test, and its own temp files are not
# what this holds.
set -u
parent=${TMPDIR:-/tmp}
fresh=$(mktemp -d "${parent%/}/bh-leftover-check.XXXXXX") || exit 1
# Windows reads TMP and TEMP as Windows paths
native=$fresh
if command -v cygpath >/dev/null 2>&1; then
  native=$(cygpath -w "$fresh")
fi
TMPDIR=$fresh TMP=$native TEMP=$native "$@"
status=$?
if [ -n "$(ls -A "$fresh")" ]; then
  echo "check-test-leftovers: '$*' left these in its temp folder; each test must remove what it made:" >&2
  # every path left, so its names say which test made it
  find "$fresh" -mindepth 1 | sed "s|^$fresh/||" | sed -n 1,100p >&2
  rm -rf "$fresh"
  [ "$status" -ne 0 ] && exit "$status"
  exit 1
fi
rmdir "$fresh"
exit "$status"
