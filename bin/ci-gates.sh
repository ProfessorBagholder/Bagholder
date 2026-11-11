#!/bin/sh
# Whether a change can affect a CI job. The changed paths are read on stdin, one
# a line; the job's name is the argument; the script prints true or false.
#
# A pull request runs a job only when a changed path is one the job's work
# actually reads. A master push runs the capacity measurements whatever the
# paths (CI_EVENT=push), and so does a run by hand or a release's rebuild
# (workflow_dispatch, workflow_call): a filter that was too narrow is still
# caught there. Any other event with no list runs nothing.
#
# The lists are what the jobs compile or what the tests open, worked out from
# the code. A path not named here does not run that job.
set -u
job=${1:?a job name}
event=${CI_EVENT:-pull_request}

case "$event" in
  workflow_dispatch|workflow_call) printf '%s\n' true; exit 0;;
esac
# the capacity measurements, on a master push and so on the commit a release is cut from
if [ "$job" = capacity ] && [ "$event" = push ]; then
  printf '%s\n' true
  exit 0
fi

# a test, or a small command of its own: not what a job builds and runs
not_the_running_code() {
  case "$1" in
    */tests/*|*/tests.rs|*/src/tests*.rs|*/src/bin/*|*/agents/tests.rs) return 0;;
  esac
  return 1
}

# markdown a Rust test opens (tests_misc.rs reads this list back)
md_a_rust_test_reads() {
  case "$1" in
    SPEC.md|docs/spec-ids.md|docs/architecture.md|CLAUDE.md|rust/crates/wealthsimple/tests/replies/wealthsimple-statements/README.md) return 0;;
  esac
  return 1
}

# crates linked into the server the page and the image run (server/Cargo.toml)
server_crate() {
  case "$1" in
    rust/crates/server/src/*|rust/crates/engine/src/*|rust/crates/book/src/*|rust/crates/core/src/*|rust/crates/model/src/*|rust/crates/broker/src/*|rust/crates/wealthsimple/src/*|rust/crates/sources/src/*|rust/crates/market/src/*|rust/crates/sqlite/src/*|rust/crates/diff/src/*|rust/crates/diff-derive/src/*|rust/crates/ws/src/*|rust/crates/net/src/*) return 0;;
    rust/Cargo.toml|rust/Cargo.lock|rust/rust-toolchain.toml) return 0;;
    rust/crates/server/Cargo.toml|rust/crates/engine/Cargo.toml|rust/crates/book/Cargo.toml|rust/crates/core/Cargo.toml|rust/crates/model/Cargo.toml|rust/crates/broker/Cargo.toml|rust/crates/wealthsimple/Cargo.toml|rust/crates/sources/Cargo.toml|rust/crates/market/Cargo.toml|rust/crates/sqlite/Cargo.toml|rust/crates/diff/Cargo.toml|rust/crates/diff-derive/Cargo.toml|rust/crates/ws/Cargo.toml|rust/crates/net/Cargo.toml) return 0;;
  esac
  return 1
}

# What `capacity.rs` builds and measures. `build` pulls a made-up book through
# the broker and Wealthsimple's adapter and settles it (`figures`, the engine,
# the book, sqlite). `probe` then times one operation: an added trade and an
# import (`entries`, `csv_import`), a balances read (`figures::broker_changed`),
# a price (`figures::price_changed`), net worth (`engine::scope`), a feed step
# (`events::Feed::step`, which rebuilds the page's views and calls `status`),
# and an agent's page (`views`, `http::model`). `status` on that step also reads
# the header's other counts (`notify`, `update`, `orders::open_orders_count`,
# `orders::order_failures`, `http::agents::status`, `http::remote::status`,
# `feeds::feed_failures`), so those files are on the measured path. The sign-in
# parser is not: `load_session` returns at a missing file, which is what the
# made-up book has, before any of `session.rs` or `login.rs` runs.
check_capacity() {
  not_the_running_code "$1" && return 1
  case "$1" in
    rust/crates/engine/*|rust/crates/book/*|rust/crates/sqlite/*|rust/crates/core/*|rust/crates/broker/*|rust/crates/wealthsimple/*|rust/crates/diff/*|rust/crates/diff-derive/*|rust/crates/model/*|rust/crates/market/*) return 0;;
    rust/crates/sources/src/cache.rs|rust/crates/sources/src/contract.rs|rust/crates/sources/src/venue.rs|rust/crates/sources/src/lib.rs|rust/crates/sources/Cargo.toml) return 0;;
    rust/crates/server/src/capacity.rs|rust/crates/server/src/figures.rs|rust/crates/server/src/entries.rs|rust/crates/server/src/csv_import.rs|rust/crates/server/src/events.rs|rust/crates/server/src/views.rs|rust/crates/server/src/status.rs|rust/crates/server/src/engine_inputs.rs|rust/crates/server/src/app.rs|rust/crates/server/src/docs.rs|rust/crates/server/src/feeds.rs|rust/crates/server/src/notify.rs|rust/crates/server/src/update.rs|rust/crates/server/src/due.rs|rust/crates/server/src/broker_reads.rs|rust/crates/server/src/market_context.rs|rust/crates/server/src/main.rs|rust/crates/server/src/remote.rs) return 0;;
    rust/crates/server/src/wire/*|rust/crates/server/src/agents/*) return 0;;
    rust/crates/server/src/orders/mod.rs|rust/crates/server/src/orders/doc.rs|rust/crates/server/src/orders/tools.rs) return 0;;
    rust/crates/server/src/http/mod.rs|rust/crates/server/src/http/model.rs|rust/crates/server/src/http/agents.rs|rust/crates/server/src/http/remote.rs) return 0;;
    rust/crates/server/Cargo.toml|rust/Cargo.toml|rust/Cargo.lock|rust/rust-toolchain.toml) return 0;;
    rust/crates/engine/Cargo.toml|rust/crates/book/Cargo.toml|rust/crates/sqlite/Cargo.toml|rust/crates/core/Cargo.toml|rust/crates/broker/Cargo.toml|rust/crates/wealthsimple/Cargo.toml|rust/crates/diff/Cargo.toml|rust/crates/diff-derive/Cargo.toml|rust/crates/model/Cargo.toml|rust/crates/market/Cargo.toml) return 0;;
    .github/workflows/capacity.yml) return 0;;
  esac
  return 1
}

# The Rust suite. It reads the markdown named above, every workflow (the copy
# that must name BAGHOLDER_EXIT_WITH, the builds that must enable no feature)
# and the page's sources (the generated types, the protocol, the amounts scan).
check_rust() {
  case "$1" in
    rust/*|.github/workflows/*|web/src/*|rust/Dockerfile|.dockerignore|bin/ci-gates.sh) return 0;;
  esac
  md_a_rust_test_reads "$1" && return 0
  return 1
}

# Clippy, the lock, the release decision, the binary with no developer's command.
check_rust_checks() {
  case "$1" in
    rust/*|bin/release-which.sh|bin/release-which.test.sh|bin/ci-gates.sh|bin/ci-gates.test.sh) return 0;;
  esac
  return 1
}

# The page's own check, tests and bundle. They read web/ and nothing in rust/.
check_web() {
  case "$1" in
    web/*) return 0;;
  esac
  return 1
}

# Playwright against the server. The server's crates and the page. Not the
# browser helper (the suite never starts it) and not a test file (it is not in
# the binary the suite runs). An engine change is in: the page shows its figures.
check_browser() {
  not_the_running_code "$1" && return 1
  case "$1" in
    web/*) return 0;;
  esac
  server_crate "$1" && return 0
  return 1
}

# The image: the same crates, the helper and sedar's crate, the page, and the
# files the image and its check are built from.
check_container() {
  not_the_running_code "$1" && return 1
  case "$1" in
    web/*|rust/crates/browser/src/*|rust/Dockerfile|rust/docker-entrypoint.sh|docker-compose.yml|docker-compose.updater.yml|.dockerignore|bin/image-check.sh) return 0;;
  esac
  server_crate "$1" && return 0
  return 1
}

case "$job" in
  rust-checks) check=check_rust_checks;;
  rust|capacity|web|browser|container) check=check_$job;;
  *) echo "ci-gates: no such job: $job" >&2; exit 2;;
esac
found=false
while IFS= read -r f; do
  [ -n "$f" ] || continue
  if "$check" "$f"; then found=true; break; fi
done
printf '%s\n' "$found"
