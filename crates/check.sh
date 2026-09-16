#!/bin/sh
# Every differential harness, on fresh builds of both profiles. The harnesses
# read a copy of the database named by BAGHOLDER_DB and never the live file;
# the live-source ones read the real sources.
#
#   BAGHOLDER_DB=/path/to/copy.db sh crates/check.sh
set -e
cd "$(dirname "$0")/.."
cargo build -q
cargo build -q --release
set +e
fail=0
for t in crates/model/*.py crates/store/*.py crates/ws/*.py crates/market/*.py crates/server/*.py; do
  out=$(python3 "$t" 2>&1)
  code=$?
  printf '%-32s %s\n' "$t" "$(printf '%s\n' "$out" | tail -1)"
  if [ $code -ne 0 ]; then
    fail=1
    printf '%s\n' "$out" | tail -25 | sed 's/^/    /'
  fi
done
exit $fail
