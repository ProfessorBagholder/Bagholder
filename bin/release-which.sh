#!/bin/sh
# Which release a run of release.yml makes, written to $GITHUB_OUTPUT as `tag=`:
# the tag a run by hand names ($ASKED); else, on a push to master, the draft
# release of the app's version when this push changes it ($BEFORE to
# $GITHUB_SHA). A push that leaves the version as it was makes none. A version
# that changes with no draft release written for it, or that cannot be read,
# fails the run: a bump never ships nothing without saying so. Held by
# bin/release-which.test.sh.
set -u
if [ -n "${ASKED:-}" ]; then
  echo "tag=$ASKED" >> "$GITHUB_OUTPUT"
  exit 0
fi
version() { git show "$1:rust/crates/server/src/app.rs" 2>/dev/null | sed -n 's/^pub const APP_VERSION: &str = "\(.*\)";$/\1/p'; }
now="$(version "$GITHUB_SHA")"
was="$(version "$BEFORE")"
if [ -z "$now" ]; then
  echo "the app's version cannot be read at $GITHUB_SHA" >&2
  exit 1
fi
if [ -z "$was" ]; then
  echo "the app's version cannot be read at $BEFORE, the commit this push follows" >&2
  exit 1
fi
if [ "$now" = "$was" ]; then
  echo "the version stays $now: no release"
  exit 0
fi
if [ "$(gh release view "v$now" --repo "$GITHUB_REPOSITORY" --json isDraft --jq .isDraft 2>/dev/null)" = "true" ]; then
  echo "v$now: this merge publishes its draft release"
  echo "tag=v$now" >> "$GITHUB_OUTPUT"
  exit 0
fi
echo "the version is now $now, and the draft release v$now is missing: write it, then run this again by hand" >&2
exit 1
