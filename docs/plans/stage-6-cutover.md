# Plan: stage 6 — the cutover: the Rust server and the Svelte page become the app

A done-contract (`PLAN.template.md`). Stage 6 of the order of work in `docs/design-review.md`. This is a migration (`docs/decisions.md`, 2026-09-26): nothing new is built. Built in two parts, each merged when its criteria are green: **6a**, nothing the app keeps stays in the old store; **6b**, the Python and Go apps and `ledger.html` go, and releases, the container and the updater carry the Rust build alone.

## For the owner to decide

Nothing open. Decided on the migration rule, and recorded in `docs/decisions.md` with this plan:
- The design review's "shared components, one overlay manager, accessibility" for stage 6 is a rework of the page, not the migration; it goes to `docs/future.md`.
- The Rust build keeps its own data folder (`~/.bagholder-rust`; `/data` in the container). A first start with no book there takes the Python app's database from the Python app's folder (`~/.bagholder`), as a first start already takes one from its own folder: a copy, the Python app's folder untouched.

## Scope

**6a.** What the Rust server still reads and writes in the old store (`bagholder.db`, the Python app's schema) moves to the two stores `docs/architecture.md` §6 names, each carried over once: the notification history, the told marks, the notification settings and the seen marks to the book; news, filings and their read text, short interest, exposures, universes, gauges, chart history and bars, the update check and the readers' memory of which source answered (the old store's `meta` keys) to the market cache. Clear data follows each table to its new place. After it, the server opens the old store only to carry it over. Four failures the move brings to light are fixed with it, each a regression of the rewrite: the exposure refresh waits on change counters nothing writes any more (a new holding or watched row is classified six hours late); the short interest's days-to-cover reads a benchmark table the switch dropped (always blank on a Canadian listing); a chart in another currency than its bars converts with a rate table the switch dropped (empty); the `synced_at` read of a key nothing writes.
**6b.** `python/`, the root `bagholder.py`, `go/`, `ledger.html`, `lightweight-charts.js` and the root `favicon.png` are removed, with everything that exists only for them: their CI jobs, release jobs and images, the server's `/ledger.html` and `/v2` routes, the tests that read them. The container's `:latest` and `:X.Y.Z` tags become the Rust image; `docker-compose.yml` runs it on `./data`. The server finds its root and the notification icon without `ledger.html`. Docs name one build.
Out of scope, on purpose: every screen, figure and word (`SPEC.md` changes only where it names the Python app, its updater or its image); the phones (`tests/cases` stay as frozen files, since the phones read them); `bagholder_market`'s readers (news, filings, short interest, exposures, universes, gauges, charts), which are the app's only implementation of those features and stay, now on the market cache; the Wealthsimple sign-in and order transport (`bagholder_ws`); the page's structure.

## The old app here

- The Rust server opens `bagholder.db` at every start (`server/src/app.rs`), with the old schema's migrations and relabels, and keeps in it the notification history (`notifications`, `told`, meta `notify_settings`, `notify_seen:*`), everything the market readers read (`news`, `filings`, `exposures`, `gauges`, `shorts`, `universes`, `price_history`, `history_fetches`, `price_bars`, `bar_fetches`, and meta `tmx_form:*`, `bars_source:*`, `coinbase_product:*`, `yahoo_miss:*`, `bars_miss:*`, `news_source_fetched:*`, `filings_sources:*`, `filings:held-since`, `update_check`), and the `gen` counters that rebuild the markets context. `docs/architecture.md` §6 places the first group in the book and the second in the market cache; neither has a place for them yet.
- Three readers still read tables the switch dropped from the old store: `tables::benchmark_days` (`benchmark_prices`, for days-to-cover), `history::read_fx` (`fx_rates`, for a chart in another currency), and `feeds::exposure_loop`'s wait on the `gen` counters of `activities`, `watchlist` and `securities`. Each reads nothing now and fails quietly: entries added to `docs/old-app-mistakes.md` "The cutover (stage 6)" with their tests.
- Removing the old builds breaks, by the survey of 2026-09-26: `main.rs` `root_dir()` finds the checkout by `ledger.html`, and the updater's git mode and the debug page loading depend on it; `http/assets.rs` serves `/ledger.html`, `/v2` and `/lightweight-charts.js`; `notify.rs` uses the root `favicon.png` as the notification icon (byte-identical to `web/public/favicon.png`); the Rust tests in `main.rs` and `tests_misc.rs` that parse the page or compare its `PROTOCOL`; the CI's Python and Go jobs, `release.yml`'s Python and Go jobs and the Rust package step copying the page; `docker.yml`, where the Python image owns `:latest`; `rust/Dockerfile`'s copy of the page; `docker-compose.yml`; `.dockerignore`, `.gitignore`; `README.md`, `SPEC.md` (§2 Versions, the container, §7), `MOBILE.md`, `DISCLOSURES.md`, `CLAUDE.md`, `tests/README.md`.
- **A Python install and the cutover.** A Python release copy looks for `bagholder-vX.Y.Z-web.zip` and, finding none, shows its "Update available" link (`python/bagholder.py` 7140-7154): it goes to the release page, where the Rust archive is. A Python git checkout on a clean `master` pulls and restarts through its supervisor, which runs `python/bagholder.py` again (7367-7370): after 6b there is none, the child fails, and the supervisor exits. 6b keeps the root `bagholder.py` as what that restart runs: it starts the Rust server from the checkout (the built release binary, else `cargo run --release`, else it says what to run), so a checkout that updated itself comes back as the Rust app. The Python folder's `python/bagholder.py` is kept as the same few lines for the supervisor's path.

**Carried over, and why:** the feed and market functions in `bagholder_store` and `bagholder_market` keep their SQL (the tables move whole, with their indexes and triggers), since the migration moves where the data lives, not what the readers do; the `meta` key names the readers remember sources by, kept as a table of the market cache for the same reason; the one-time import of an old database (`legacy_import`), which is how a Python user's data reaches the Rust app.

## How the leading products do it

Nothing here is a figure or a behaviour a trading journal defines: it moves where the app keeps what it already has. The one practice that bears on it, carrying the person's data forward on the first start of the new build and leaving the old copy untouched, is what the import already does and what this keeps.

## Open questions

None.

## Approach

### 6a
- **Book migration 015** (`rust/crates/book/migrations/015-notifications.sql`, schema snapshot `v15.sql`): `notifications` and `told` as the old store has them (columns and meaning kept), in the book. Notification settings and seen marks become book `settings` keys (`notify.settings`, `notify.seen.<stream>`), through `Book::setting`/`set_setting`. Carried once from the old store, in one transaction, marked by a book setting; the old tables are not read after.
- **Market cache migration 004** (`rust/crates/sources/migrations/004-the-earlier-readers.sql`): the old store's market tables, their indexes, the `gen` counters and triggers for them, and a `meta` table for the readers' memory keys and `update_check`. Carried once from the old store with `ATTACH` and `INSERT … SELECT`, marked in the cache; then the old tables are not read.
- **One connection pool on the market cache** replaces `App.store` for everything above: `feeds.rs`, `market_context.rs`, `notify.rs` (settings and history go to the book), `update.rs`, `search.rs`, `history`'s background reads, the exposure context. `bagholder.db` is opened at start only to carry, then closed.
- **The four regressions**: the exposure pass wakes on the book's own change (the engine's report of a holding or a watched row added) instead of the old counters; days-to-cover counts the market's trading days from the market cache's benchmark tracker closes (SPEC's definition unchanged: days that index traded); a chart in another currency converts with the book's Bank rates (`Book` facts, the rate on or before each day, as the figures use); the `synced_at` read goes.
- **Clear data** (`server/src/clear.rs`): `OLD_TABLES` and `META_KEPT` replaced by the kinds' tables in the book and the cache; its tests follow.
- **The e2e harness and `demo-book`** keep writing an old-store database, since importing one is the path a Python user's data takes; the carry runs on it at start as on a real one.

### 6b
- Remove `python/`, `go/`, `ledger.html`, `lightweight-charts.js`, root `favicon.png` (the icon moves to the embedded `web/dist` copy or `web/public/favicon.png`), their CI, release and image jobs. `root_dir()` finds the checkout by `rust/Cargo.toml`. `/v2`, `/ledger.html`, `/lightweight-charts.js` go.
- Root `bagholder.py` and `python/bagholder.py`: the few lines above.
- `docker.yml`: the Rust image is `:latest` and `:X.Y.Z` (and keeps `:rust`, `:rust-X.Y.Z` for installs that follow those tags); compose runs it on `./data`, which holds the Python app's database, carried on first start.
- First start with no book: the database written last, the app's own folder's or the Python app's (`~/.bagholder/bagholder.db`, copied); an own one the Python app's is newer than (an earlier Rust build's, never made into a book) is set aside in `snapshots/`.
- Docs: one build; the version trio becomes `APP_VERSION` in `rust/crates/server/src/app.rs`, the iOS and Android numbers; `CLAUDE.md` and `README.md` rewritten for one build; `SPEC.md` where it names the Python supervisor, `-web.zip` or the Python image.

**Stays the same:** every screen, figure and word; `PROTOCOL` unless the page and the server change together; the stores' existing tables; `legacy_import`.

## Acceptance criteria

Every part:
- [x] `cargo test --workspace` green in `rust/`, a test for each behaviour change, `RUSTFLAGS="-D warnings"` clean, `cargo clippy --workspace --lib --bins` clean.
- [x] `npm run check`, `npm test` and `npm run e2e` green in `web/`.
- [x] Rendered on the Rust scratch server on a copy of the owner's data (orders dry): the header's error line names nothing but sources out of reach; News, Disclosures, short interest, the heatmap, a chart, the notification history and settings show what the old store held.

**6a**
- [x] **Carried once:** on a copy of the owner's data, every row of each moved table and each moved `meta` key is in its new store after the first start, the same count and the same values (a test on a made-up old store, and the counts on the owner's copy written into Verification); a second start carries nothing; a write after the carry lands in the new store and nothing writes the old one (a test that makes `bagholder.db` read-only after the carry and runs a news pass, a filings pass, a notification and a chart read).
- [x] **Nothing reads the old store after the carry:** a boundary test fails when a server file other than the carry and `legacy_import` names the old store's pool or its tables.
- [x] **The four regressions:** a new holding or watched row starts its exposure read at once (test); days-to-cover on a Canadian listing counts the tracker's trading days (test on recorded closes); a chart in another currency converts at the Bank's rate on or before each day (test); no read of `synced_at`.
- [x] **Clear data** empties each kind in its new place and keeps the rest (the existing tests, re-pointed).

**6b**
- [x] No `python/`, `go/`, `ledger.html`, `lightweight-charts.js`, root `favicon.png` in the tree; no CI, release or image job for them; `git grep` finds no reference outside the history docs (`docs/plans/`, `docs/decisions.md`), the launcher that replaces them, and the phones' own files (out of scope).
- [x] The release workflow's steps, run by hand here, build the Rust archive with the page embedded and nothing else; the image builds from `rust/Dockerfile` (CI's `image` job, on every pull request, since Docker does not run in this sandbox and `docker.yml` builds only on a tag).
- [x] A first start with no book and a `bagholder.db` in the Python folder carries it, and takes it over an older one of its own (tests with `HOME` pointed at a scratch folder).
- [x] Root `bagholder.py` starts the Rust server (test: run it with a stand-in binary).
- [x] The notification icon and the root are found without `ledger.html` (tests).

## Surfaces to check beyond the diff

`book/schema/v15.sql`; `sources/migrations/004-*` and the cache's schema test; `clear.rs`; `tests_boundary.rs`; `tests_misc.rs` `TIMED_WAITS`; `web/e2e/serve.mjs`; `release.yml`, `docker.yml`, `tests.yml`; `.dockerignore`; `README.md`, `SPEC.md`, `CLAUDE.md`, `MOBILE.md`, `DISCLOSURES.md`; `docs/old-app-mistakes.md`.

## Right to refuse

If a moved table cannot be carried with its values unchanged, that table is named and the part stops for it.

## Anti-stub self-check

Initialled at the gate: the carry run on a copy of the owner's data and the counts compared; no old-store read left behind a flag; the page rendered.

## Verification

**6a** (2026-09-26):
- Written to `bagholder.db` after the carry: only by Clear data, emptying a cleared kind (`carry::clear_old_store`); the boundary test holds every other server file to not naming it.
- `RUSTFLAGS="-D warnings" cargo test -q --workspace` green; `cargo clippy --workspace --lib --bins` clean. `web/`: `npm run check` 0 errors, `npx vitest run` 140 passed, `npx vite build` built. `npx playwright test`: 276 passed.
- Tests added: `book/tests/notices.rs` (carried once with every value, the next id after the earlier store's, cleared with Settings, the mark kept); `server/src/carry.rs` (every row and key of a made-up earlier store carried with its values, a second start carries nothing; after the carry, with `bagholder.db` read-only, a news pass, a disclosures pass, a notification and a chart read go through and the file is byte for byte as it was; a cache made with no earlier store never takes one in later; a carry that fails part way carries nothing and names the column); `tests_boundary.rs` `nothing_reads_the_earlier_store_but_the_carry`; the four regressions' tests named in `docs/old-app-mistakes.md` "The cutover (stage 6)"; `clear.rs` tests re-pointed to the cache's and the book's tables, and extended to the earlier store: Clear data still clears everything (`docs/decisions.md`, 2026-09-25), so each kind cleared is also emptied from `bagholder.db` (`carry::clear_old_store`, its carry flags and marks kept), the tests show the file holds none of a cleared kind's rows or keys afterwards and that a start after the clear carries nothing back. The exposure-wake test waits on the listing watched itself (`events::park_until`), no clock.
- The owner's data (a copy of `~/.bagholder/bagholder.db`, the Python app's, on a fresh Rust home; release build, orders dry, offline): the first start imported it into a book, retired its figure tables, carried 10 orders and 1 bracket, then:

  | carried to | table / keys | old store | new store |
  |---|---|---|---|
  | market cache | news | 1102 | 1102 |
  | | filings | 4754 | 4754 |
  | | exposures | 76 | 76 |
  | | gauges | 2 | 2 |
  | | shorts | 26 | 26 |
  | | universes | 260 | 260 |
  | | price_history | 8296 | 8296 |
  | | history_fetches | 27 | 27 |
  | | price_bars | 110236 | 110236 |
  | | bar_fetches | 114 | 114 |
  | | readers' `meta` keys and `update_check` | 395 | 395 |
  | book | notifications | 51 | 51 |
  | | told | 1393 | 1393 |
  | | `notify_settings` and non-empty `notify_seen:*` → `notify.*` settings | 37 | 37 |

  Values compared with `EXCEPT` across the two files for news, price_bars and filings: no row differs. `/api/status` error line empty; `/api/notifications` answers the carried history. Rendered from the same copy (offline, so no quotes): Markets shows the carried Fear & Greed readings, the heatmap's sectors from the carried exposures and the carried news, newest first; the header's error line is empty. Found in the render and fixed: Clear data left `snapshots/` (the files as they were before a migration or a carry, each holding every kind); any clear now removes them (`clear.rs` `a_clear_removes_the_copies_kept_of_the_files_as_they_were`).

**6b** (2026-09-26, on bfe46430):
- Removed: `go/` (all of it), `python/` but for the launcher, `ledger.html`, `lightweight-charts.js`, the root `favicon.png` (byte-identical to `web/public/favicon.png`, `cmp`); `tests.yml`'s `python` and `go` jobs, `release.yml`'s `python` and `go` jobs and the package step's copies of the page files and of `web/dist` (the page is in the binary), `docker.yml`'s `python`, `go` and `go-image` jobs; the server's `/v2`, `/ledger.html`, `/lightweight-charts.js` routes, the `ledger.html` fallback for `/` and `feeds::ledger_path`. `tests/cases`, `tests/fixtures` and `tests/wire` kept: `rust/crates/model` (cases, wire), `bagholder-diff` (wire), the market crate (fixtures) and the phones (cases) read them.
- `git grep -n -I -e 'ledger.html' -e 'python/' -e 'go/' -e 'bagholder.py'` outside the history docs finds: the launcher (`bagholder.py`, `python/bagholder.py`), its tests (`rust/crates/server/src/main.rs`) and the lines naming it (`README.md`, `CLAUDE.md`, `tests.yml`); the phones' comments and `ios/DESKTOP-API.md`, `ios/README.md`, `ios/project.yml` (the phones are out of scope; `project.yml` is an XcodeGen spec the checked-in `Bagholder.xcodeproj` does not follow, and the project does not reference the removed files); `/wp-json/wp/v2/` in recorded fund pages (`rust/crates/sources/tests/replies/harvest`), a match of the pattern `go/` inside another word.
- `RUSTFLAGS="-D warnings" cargo test -q --workspace`: 1,370 passed, 0 failed (with `web/dist` built); the three tests that depend on a built page run again with `web/dist` moved away: 3 passed. `cargo clippy --workspace --lib --bins`: no output, exit 0.
- `web/`: `npm run check` 0 errors (19 warnings, as before); `npx vitest run` 28 files, 140 tests passed; `npx vite build` built, `dist/favicon.png` byte-identical to `public/favicon.png`.
- The release package for `aarch64-apple-darwin`, the workflow's steps by hand: `npm ci && npm run build`, `cargo build --release --locked` of the four binaries, the package step: the archive lists `bagholder`, `bagholder-browser`, `disclosures-mcp`, `sedar` and nothing else (15.2 MB, with its `.sha256`). Unpacked into an empty folder and started there (scratch `HOME` and data folder, offline, dry orders): `--version` answers `bagholder 1.47.0`; `/` 200 (817 bytes), its script 200, `/favicon.png` 200 (8,009 bytes), `/v2`, `/ledger.html`, `/lightweight-charts.js` 404.
- `docker build -f rust/Dockerfile .` not run: the Docker daemon is not running on this machine. The Dockerfile's lines are held by `test_the_image_builds_this_workspace_and_the_page_from_the_repository_root` and `.dockerignore` by `test_nothing_the_image_needs_is_kept_out_of_it`.
- The page of the same commit as the server: a checkout's update (`update.rs` `pull`) builds the page (`npm ci`, `npm run build` in `web/`) before `cargo build`, and a page that does not build, or no npm, fails the update and puts the previous commit back as a failed server build does (`build_checkout`, tested with stand-in npm and cargo: order, each step's failure stopping what follows, npm missing). The launcher builds the page when `web/dist/index.html` is missing or older than any file under `web/src`, `web/public`, or `web/index.html`, `package.json`, `package-lock.json`, `vite.config.*` (tested for each). Suite after: 1,372 passed, 0 failed; clippy clean.
- Tests added: `legacy_import`: `the_python_app_s_folder_is_found_in_the_person_s_home`, `a_first_start_with_no_book_takes_a_copy_of_the_python_app_s_database` (the copy imported, the Python folder byte for byte and file for file unchanged, a second start takes nothing), `a_python_app_that_is_running_is_copied_with_what_its_log_holds`, `a_folder_with_a_database_or_a_book_of_its_own_takes_nothing`; `main.rs`: `test_the_root_is_the_checkout_the_server_was_built_in`, `test_the_launcher_is_one_file_in_both_places`, and under `launcher` (Unix, python3 required): the built server takes the launcher's process (its parent is the test), cargo builds and runs in `rust/`, a page not built is built with npm first, neither says what to run and exits 1; `notify.rs`: `test_the_notification_icon_is_the_pages_own`; `http/tests.rs`: the removed routes answer 404.

- On 6a (rebased, with the tile fix): `RUSTFLAGS="-D warnings" cargo test -q --workspace` 1,383 passed, 0 failed; clippy clean; `npx playwright test` 276 passed.
- A first start on the owner's data the way a Python user meets it: `HOME` a scratch folder holding only `.bagholder/bagholder.db` (a copy of the owner's), no `BAGHOLDER_HOME`, release build, offline, orders dry. The start took a copy into `~/.bagholder-rust`, imported it into a book (29 accounts, 6,156 transactions), and carried the orders, notices and market data (the same counts as 6a's table); the Python folder afterwards held only its `bagholder.db`. The second start carried nothing; the Dashboard drew the book's figures; `/` and `/favicon.png` answered 200, `/ledger.html`, `/v2` and `/lightweight-charts.js` 404.
- CI builds the image from `rust/Dockerfile` on every pull request (`tests.yml` `image`, not pushed): Docker does not run in this sandbox, and `docker.yml` builds only on a tag.

- Found on this machine and fixed: the Rust folder held a `bagholder.db` an earlier Rust build left (five days older, never made into a book), and the first start would have imported it rather than the Python app's live one. The newer is taken and the older set aside (`legacy_import` `a_database_of_its_own_older_than_the_python_app_s_is_set_aside_and_the_python_app_s_taken`); on copies of both, the start took the Python app's (6,156 transactions to 2026-09-25) and kept the other in `snapshots/`.

## Handoff
