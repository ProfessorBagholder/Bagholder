#!/bin/sh
# nextest's wrapper for every test (`rust/.config/nextest.toml`): runs one test
# with a temp folder of its own and fails it when it leaves anything there, naming
# the test, so a leftover is never a hunt through the whole suite
# (`check-test-leftovers.sh` holds the run as a whole the same way).
set -u
parent=${TMPDIR:-/tmp}
fresh=$(mktemp -d "${parent%/}/bh-one-test.XXXXXX") || exit 1
native=$fresh
if command -v cygpath >/dev/null 2>&1; then
  native=$(cygpath -w "$fresh")
fi
TMPDIR=$fresh TMP=$native TEMP=$native "$@"
status=$?
if [ -n "$(ls -A "$fresh")" ]; then
  echo "one-test-leftovers: this test left these in its temp folder; each test must remove what it made:" >&2
  find "$fresh" -mindepth 1 | sed "s|^$fresh/||" | sed -n 1,100p >&2
  rm -rf "$fresh"
  [ "$status" -ne 0 ] && exit "$status"
  exit 1
fi
rmdir "$fresh"
exit "$status"
