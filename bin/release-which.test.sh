#!/bin/sh
# bin/release-which.sh over its cases, `gh` stood in by a script that says a
# draft exists only for the versions listed in $DRAFTS. Run by tests.yml.
set -u
script="$(cd "$(dirname "$0")" && pwd)/release-which.sh"
work=$(mktemp -d) || exit 1
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/bin" "$work/repo/rust/crates/server/src"
cat > "$work/bin/gh" <<'GH'
#!/bin/sh
# gh release view vX --repo R --json isDraft --jq .isDraft
for d in $DRAFTS; do [ "$3" = "$d" ] && { echo true; exit 0; }; done
exit 1
GH
chmod +x "$work/bin/gh"
cd "$work/repo" || exit 1
git init -q && git config user.email t@t && git config user.name t
commit() { printf 'pub const APP_VERSION: &str = "%s";\n' "$1" > rust/crates/server/src/app.rs; git add -A && git commit -qm "$1" && git rev-parse HEAD; }
v1=$(commit 1.0.0)
other=$(git commit -q --allow-empty -m same && git rev-parse HEAD)
v2=$(commit 1.1.0)
rm rust/crates/server/src/app.rs && git add -A && git commit -qm gone
gone=$(git rev-parse HEAD)
failed=0
case_() { # name, before, sha, drafts, asked, want exit, want tag
  out="$work/out"; : > "$out"
  PATH="$work/bin:$PATH" GITHUB_OUTPUT="$out" GITHUB_REPOSITORY=o/r BEFORE="$2" GITHUB_SHA="$3" DRAFTS="$4" ASKED="$5" sh "$script" > /dev/null 2>&1
  code=$?
  tag=$(sed -n 's/^tag=//p' "$out")
  if [ "$code" != "$6" ] || [ "$tag" != "$7" ]; then
    echo "release-which: $1: exit $code, tag '$tag'; wanted exit $6, tag '$7'"
    failed=1
  fi
}
case_ "the same version: no release" "$v1" "$other" "v1.0.0" "" 0 ""
case_ "changed with its draft: that tag" "$other" "$v2" "v1.1.0" "" 0 "v1.1.0"
case_ "changed with no draft: fails" "$other" "$v2" "" "" 1 ""
case_ "unreadable now: fails" "$v2" "$gone" "v1.1.0" "" 1 ""
case_ "unreadable before: fails" "$gone" "$v2" "v1.1.0" "" 1 ""
case_ "by hand: the tag named" "$v1" "$v2" "" "v1.0.0" 0 "v1.0.0"
[ "$failed" = 0 ] && echo "release-which: every case holds"
exit "$failed"
