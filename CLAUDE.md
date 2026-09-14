# Working on Bagholder

Read this first, then `SPEC.md`. Bagholder is a local-first trading journal for Wealthsimple users. On the desktop: a Python standard-library server (`bagholder.py`), the derived model (`model.py`), market data (`market.py`), the SQLite store (`store.py`), CSV import (`csvimport.py`) and one static page (`ledger.html`, drawing charts with the bundled `lightweight-charts.js`). On the phone: an iOS app (`ios/`, SwiftUI) and an Android app (`android/`, Compose), each with its own implementation of the model; `MOBILE.md` is their build-and-test guide. The repository is public.

## What is authoritative

- `SPEC.md` defines every figure and every screen on every platform: meaning, formula, source, currency, format, layout, refresh cadence, and the verification steps; its §8 is the phone's presentation of the same figures. Check every change against it. When a change needs a definition to differ, change `SPEC.md` in the same commit and say why.
- The user's standing rules, all recorded in the spec: only what was asked, no captions, tooltips, notes or helper text; per-instrument figures in the instrument's own currency; aggregates in CAD, never labelled; payout frequency verified from the fund's record, never assumed; raw Wealthsimple rows never rewritten; nothing synthetic on a chart.

## How changes land

1. Never commit to `master`, and never run `git checkout` in the user's checkout: their live app serves from it and they test PRs there. Work in a detached worktree under the session scratchpad (`git worktree add --detach <dir> origin/master`), commit there, and push the commit straight to its topic branch without creating a local one: `git push origin HEAD:refs/heads/<topic>`. A branch checked out in any worktree locks that name, and the user's own checkout of it then fails.
2. Verify before committing (see below). If something is wrong, stop and report before committing; do not commit and mention it afterwards.
3. Commit with a message that says what changed and why, ending with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Open the PR from the pushed branch: `gh pr create --head <topic> --fill` (PR bodies end with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`).
4. Anything the user has to do is one fenced `bash` block per step; a step never exists in prose ("build the branch", "install it" are not instructions). Before writing a block, run the pre-flight in their checkout in the same turn and write the block for what it shows: `git fetch -q; git branch --show-current; git worktree list; git branch --list <target>; git status --short`, and `git diff --stat <their-branch> origin/<target> -- <each modified file>` for every file the status lists (a file that differs between the branches blocks the checkout: say so and give the `git stash` line). No worktree or local branch may hold the target; if one does, push the same commit under another name. The block is then `git fetch origin && git checkout <target>`, plus a restart of `python3 bagholder.py` when the server changed, and `git checkout master && git pull` to come back. Whatever can be run here is run here, never handed over: the simulator build, the install and seed, the APK, the demo server; the user is left only what cannot be a command from this machine (an Xcode Run on a cabled phone, a sign-in), and that one action is named alone.
5. Merge only when the user says so, from outside their checkout so `gh` cannot switch it: `gh pr merge <n> --merge --delete-branch -R <owner>/<repo>` with the scratch worktree as the working directory (run inside a checkout sitting on the PR branch, `gh` checks out the default branch, pulls and deletes the local branch itself). Confirm the PR is MERGED before any branch deletion, then give the user `git checkout master && git pull` and remove the scratch worktree.
6. A change to the apps goes to both platforms in the same PR. It is not "the same on both" until both apps have been captured on the same rows, screen by screen, and compared (`MOBILE.md` has the recipe); what cannot be the same is found on the simulator and the emulator and reported before the user meets it. Speed on a phone is judged from a home-screen launch, never from an Xcode run: the debugger makes the web engine ten times slower.

## Verifying a change

- Tests: `python3 -m unittest discover tests`. Add a test for every behaviour change; the suites are the contract for the model, store and server. `tests/cases` are the shared model cases every implementation (Python, Swift, Kotlin) runs; a model change on any platform comes with a case, regenerated with `python3 tests/make_cases.py` and reviewed as a diff (`tests/README.md`), and the Swift and Kotlin suites are run on it too (`MOBILE.md`).
- The page has no automated tests, so render it. Run a second instance on a copy of the user's data, never on the live database:
  ```
  mkdir -p /tmp/bh-scratch && cp ~/.bagholder/bagholder.db /tmp/bh-scratch/
  BAGHOLDER_NO_BROWSER=1 BAGHOLDER_DRY_ORDERS=1 BAGHOLDER_HOME=/tmp/bh-scratch BAGHOLDER_PORT=8799 python3 bagholder.py
  ```
  `BAGHOLDER_DRY_ORDERS=1` is not optional either: orders are live by default, and a scratch copy that ever carries a login must never place one. `BAGHOLDER_NO_BROWSER=1` is not optional: the app opens a browser tab at start, and without it every scratch start puts the scratch data in the user's own browser, where it reads as their app being disconnected. Open `http://127.0.0.1:8799/` yourself (`localhost` is refused by design). The user's own instance runs on 8765; do not restart or write to it.
- What to check is listed in `SPEC.md` §7: every displayed figure traced to its model field and meaning; every page at 1200, 1340, 1440 and 1680 px with no table overflowing or clipping at 1340 and above; headers level; lookups by id, exercised with a synthetic duplicate symbol in a second account. Take screenshots; measure with JavaScript rather than eyeballing. Hash-only navigation does not reload the page; use `location.reload()` after editing `ledger.html`.
- Server-side changes need the user to restart their app; say so. When the page and the server change together, bump `PROTOCOL` in both `bagholder.py` and `ledger.html` (a test keeps them equal) so the page shows "Restart Bagholder to finish the update" rather than degrading quietly.

## Handoffs from Claude Design

The file served from the design URL carries Design's preview harness on line 4 (`data-omelette-injected`, about 20 KB of script that hooks `fetch`, `postMessage` and cookies). Strip that line, its closing `</script>` and the blank line after it; keep everything else byte for byte. Then diff against `master`: the diff must be only what the handoff describes and based on the current page. Review it as a change, not as a file swap: a handoff can carry a wrong lookup or a wrong formula as easily as a colour. Verify as above before committing, and report anything wrong before committing.

## Releases

Versions are GitHub releases tagged `vMAJOR.MINOR.PATCH` (semantic versioning); commits are not versions. Which part to bump:

- PATCH (1.1.0 → 1.1.1): fixes only, nothing new to use.
- MINOR (1.1.0 → 1.2.0): anything new a user can see or do (a column, a card, a toggle, a data source), with existing data and behaviour intact.
- MAJOR (1.1.0 → 2.0.0): a change that breaks existing installs (a database that must be migrated by hand, a removed page, a changed protocol with the page that requires more than a restart).

The apps carry the product version: `APP_VERSION` in `bagholder.py`, `MARKETING_VERSION` in `ios/Bagholder.xcodeproj/project.pbxproj` and `versionName` in `android/app/build.gradle.kts` are one number, bumped together in the last PR going into the release. A release page carries only what its tag contains; a phone build from an unmerged branch is never attached to a release (it goes out as a pre-release pointing at its branch, tagged outside the `vX.Y.Z` scheme, which the updater ignores).

To release: bump the version, merge, then build the archive the in-app updater installs and publish it with the release:
  ```
  git archive --format=zip -o bagholder-vX.Y.Z-web.zip vX.Y.Z   # after tagging, or use origin/master and tag on create
  shasum -a 256 bagholder-vX.Y.Z-web.zip > bagholder-vX.Y.Z-web.zip.sha256
  gh release create vX.Y.Z bagholder-vX.Y.Z-web.zip bagholder-vX.Y.Z-web.zip.sha256 --target master --title vX.Y.Z --notes "..."
  ```
  with notes listing the merged PRs. Assets say what they are: the web archive is named exactly `bagholder-vX.Y.Z-web.zip` with its `.sha256` beside it, or running copies fall back to an "Update available" link; the Android build goes on the same page as `bagholder-vX.Y.Z-android.apk`, built from the same tag (`cd android && ./gradlew :app:assembleDebug`, then rename `app/build/outputs/apk/debug/app-debug.apk`). Copies older than the release that taught the updater the `-web` name look for `bagholder-vX.Y.Z.zip`, which is why the release carrying that change ships under the old name and the next one takes `-web`. Master may carry unreleased features between releases; a release collects everything merged since the last tag, so the bump is decided by the biggest change in that set, not by the last PR alone. Running copies check the latest release at start and hourly, and show an "Update to vX.Y.Z" button when it is newer.

## Mobile

`MOBILE.md`: why the apps are native and local-first, how each is built, run, seeded and tested, and the platform limits that cannot be worked around. Their screens are specified in `SPEC.md` §8; the working rules are in "How changes land" above.

## Do not

- Commit `.env`, `session.json`, the database, backups, or `.claude/` (all in `.gitignore`); the repository is public.
- Reach for a dependency without weighing it. The app runs on Python 3.9+ with the standard library and stays close to it on purpose — a dependency is a real cost (install, packaging, the container, dependency drift), so add one only when the job genuinely needs it and the standard library cannot do it well. It is not a hard ban: `tzdata` (Windows time zones), `curl_cffi` (the SEDAR+ provider's browser TLS fingerprint, which the standard library cannot present) and `pdfminer.six` (reading text from issuer PDFs whose subsetted fonts a naive reader cannot decode) are all optional and carry their weight. Two of them are provisioned so the user installs nothing: `curl_cffi` where absent simply drops SEDAR+ from the pipeline (`disclosures.py`), the rest of the app unchanged; `pdfminer.six` is auto-installed into `~/.bagholder/pylibs` on first use (`pdftext.py`), a system `pdftotext` used first when present. When you add or lean on a dependency, say why here and put it in `requirements.txt` (and the container gets it from there).
- Reformat or "clean up" code you were not asked to change.
