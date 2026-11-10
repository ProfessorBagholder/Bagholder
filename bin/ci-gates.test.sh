#!/bin/sh
# bin/ci-gates.sh over the changes a job must and must not run for. Run by tests.yml.
set -u
script="$(cd "$(dirname "$0")" && pwd)/ci-gates.sh"
failed=0
gate() { # event, job, paths (newline-separated), want
  got=$(printf '%s\n' "$3" | CI_EVENT="$1" sh "$script" "$2")
  if [ "$got" != "$4" ]; then
    echo "ci-gates: $1 $2 on [$3] was $got, wanted $4"
    failed=1
  fi
}

# a sign-in: the feed step calls load_session, which returns before this file runs
gate pull_request capacity "rust/crates/server/src/session.rs" false
gate pull_request capacity "rust/crates/server/src/login.rs" false
gate pull_request capacity "rust/crates/server/src/http/session.rs" false
# a hot path, and the measurement itself
gate pull_request capacity "rust/crates/engine/src/engine.rs" true
gate pull_request capacity "rust/crates/server/src/figures.rs" true
gate pull_request capacity "rust/crates/server/src/capacity.rs" true
gate pull_request capacity "rust/crates/server/src/events.rs" true
gate pull_request capacity "rust/crates/sources/src/cache.rs" true
# a reader the probe never calls, and a test
gate pull_request capacity "rust/crates/sources/src/adapters/yahoo.rs" false
gate pull_request capacity "rust/crates/engine/tests/cases.rs" false
gate pull_request capacity "rust/crates/server/src/orders/ticket.rs" false
gate pull_request capacity "web/src/lib/live.svelte.ts" false
gate pull_request capacity "docs/plans/a.md" false
# master, a release's commit, runs them whatever the paths
gate push capacity "rust/crates/server/src/session.rs" true
gate push capacity "CLAUDE.md" true

# the suite reads SPEC.md and the page's sources; a plan does not
gate pull_request rust "SPEC.md" true
gate pull_request rust "CLAUDE.md" true
gate pull_request rust "docs/architecture.md" true
gate pull_request rust "web/src/lib/generated/wire.ts" true
gate pull_request rust ".github/workflows/capacity.yml" true
gate pull_request rust "docs/plans/a.md" false
gate pull_request rust "bin/plants.py" false

# clippy does not read the page
gate pull_request rust-checks "web/src/App.svelte" false
gate pull_request rust-checks "rust/crates/core/src/dec.rs" true
gate pull_request rust-checks "bin/ci-gates.sh" true
gate pull_request rust-checks "docs/plans/a.md" false

gate pull_request web "web/src/App.svelte" true
gate pull_request web "rust/crates/server/src/session.rs" false

# the helper is not the server the browser suite drives; the engine is
gate pull_request browser "rust/crates/browser/src/main.rs" false
gate pull_request browser "rust/crates/engine/src/engine.rs" true
gate pull_request browser "rust/crates/server/src/session.rs" true
gate pull_request browser "rust/crates/model/src/wire.rs" true
gate pull_request browser "rust/crates/server/src/tests_session.rs" false
gate pull_request browser "rust/crates/server/src/http/tests.rs" false
gate pull_request browser "docs/plans/a.md" false
gate pull_request browser "web/e2e/dryorder.spec.ts" true

# the image builds the helper; a plan changes neither
gate pull_request container "rust/crates/browser/src/main.rs" true
gate pull_request container "rust/Dockerfile" true
gate pull_request container "docs/plans/a.md" false
gate pull_request container "bin/plants.py" false

# a run by hand runs the job it was given
gate workflow_dispatch web "" true

[ "$failed" -eq 0 ] && echo "ci-gates: ok"
exit "$failed"
