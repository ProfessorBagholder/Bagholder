# Earlier-app code map, the two PRs, and the repository's size

Clone: /tmp/claude-1005/-home-md-dev/12a8decb-6b2e-51e6-96e2-82398e1475b1/scratchpad/r571 at origin/master = `2255a88` (v2.9.0 + 2 commits: `17cb6c7` decisions, `2255a88` its merge). Line counts are `wc -l` over `git ls-files`; ranges are the clone's lines at HEAD.

## 1. Earlier-app code

### 1a. What runs at every start today (the "standing condition")

| Where | What runs | Guard |
|---|---|---|
| rust/crates/server/src/main.rs:234-237 | `legacy_import::adopt_python_database(&home, &python)` on every start whose HOME is non-empty (`python_home`, legacy_import.rs:475-478) | [corrected] legacy_import.rs:501 `if home.join(BOOK_FILE).exists() \|\| !from.is_file() { return Ok(None); }`: `home/book.db` is stat-ed first and, when it exists, the `\|\|` short-circuits, so `~/.bagholder/bagholder.db` is stat-ed only on a start with no book. On a machine with a book the standing cost is one stat of `book.db` per start |
| rust/crates/server/src/main.rs:160 | `carry::from_old_store(a, &f, now)` on every start | carry.rs:280-286: `guard_home`, `path.exists()`; with no file it still calls `carry_market(None, ...)`, which opens the cache and reads its `meta` each start (carry.rs:68-73) |
| rust/crates/server/src/main.rs:184 | `carry::carry_told_by_listing(&b, &cache, now)` on every start | carry.rs:380 `book.setting(TOLD_BY_LISTING)` |
| rust/crates/server/src/figures.rs:394-398 | `Figures::open` checks `home/bagholder.db` and imports it when no `book.db` exists | figures.rs:396 |
| rust/crates/server/src/figures.rs:269-272 | `legacy_import::reattach_carried_notes` on every engine build (`built`, figures.rs:259) | legacy_import.rs:421 `book.repaired(REATTACH)`, a repair record (`record_repair`, L470) |
| rust/crates/server/src/figures.rs:886-887 | `mappings()` lists `ImportMapping`, so `bagholder-import` records are re-derived on open whenever the mapping's version moves (`rederive_all`, figures.rs:893; version 5 at book/src/import/mapping.rs:196-198) | mapping version |
| rust/crates/server/src/clear.rs:170-171 | every Clear data writes the old file through `carry::clear_old_store` | carry.rs:245-249 |
| rust/crates/server/src/update.rs:609-610 | `STORE_FILES` names `OLD_FILE`, so a rolled-back update puts `bagholder.db` back from `snapshots/` when a snapshot since the update exists (update.rs:617-632) | |

(Both main.rs:160 and :184 sit in `figures_opened`, main.rs:157, called from `open_figures` at main.rs:131, which runs at start, main.rs:280, and again whenever the header is read while the figures are not open, main.rs:117-121.)

### 1b. The map, file by file

Columns: path; lines (range); count; verdict (whole = deletable whole; piece = a piece moves, named); notes.

**server crate (rust/crates/server)**

| Path | Range | Lines | Verdict | Notes |
|---|---|---|---|---|
| src/carry.rs | 1-321 (code), 425-623 (tests) | 522 | whole | `carry_market` L66-122 ATTACHes the old file (L81); `carry_following` L126-153; `carry_watch_folder` L158-165; `OLD_TABLES` L170-198; `OLD_META_KEPT` L204-213; `old_meta_kind` L217-233; `clear_old_store` L245-270 (the one write besides the carry, per its doc L244; the carry's own writes are `drop_figure_tables` legacy_import.rs:325 and `drop_order_tables` legacy_orders.rs:243); `from_old_store` L279-320 |
| src/carry.rs | 322-423 + test 624-675 | 154 | piece: `carry_told_by_listing` (+`TOLD_BY_LISTING`, `mark_moment`, `told_scopes`, `earlier_form`) moves to notify.rs | reads only the book's `told`/`settings` (L384 `book.notices()`) and the cache's `filings_carried` (L356, L390-394); `told_scopes`/`earlier_form` also use `bagholder_store::feeds::Regulator` (L340, L366); the plan says it moves (docs/plans/remove-the-earlier-app.md:91) |
| src/legacy_import.rs | 1-994 | 994 (593 code, 401 tests L594-994 [corrected]: `#[cfg(test)]` is L594, `mod tests` L595) | whole | `translate` L35-124 runs the old engine (`bagholder_model::book::build_book` L39); `retire_old_figures` L310-327; `import` L331-356; `reattach_carried_notes` L418-471; `python_home` L475-478; `adopt_python_database` L498-544; `cli` L547-562 |
| src/legacy_orders.rs | 1-310 | 310 (246 code, 64 tests L247-310) | whole | `carry_orders` L195-245 reads `bagholder_store::orders::typed` (L200-201), snapshots `bagholder.db` (L206-207), drops old tables (L243) |
| src/compare.rs | 1-612 | 612 (593 code, 19 tests L594-612) | whole | `old_base` L37-79 builds `bagholder_model::base` from the old file; `old_market` L92-197 reads its `quotes`, `price_history`, `fx_rates`, `distributions` (L107, 139, 150, 177); `cli` L543-592 |
| src/demo_facts.rs | 1-264 | 0 | nothing earlier-app | never opens `bagholder.db` (demo.rs:566 asserts it); writes the market cache (L90, L109) through `bagholder_store::bars/market/feeds` at L51, 75-77, 132-133, 147, 175, 184, 193, 213: those 9 call sites re-point when the cache SQL moves |
| src/figures.rs | 33-34, 256-258, 269-272, 388-398, 887 | ≈18 | piece | `OLD_FILE` L34; reattach call L269-272; import-on-open L394-398; `ImportMapping` in `mappings()` L887 |
| src/main.rs | 11, 12, 36, 37, 157-165, 234-238, 272-283, 412-421 | ≈38 deletable, 12 moved | piece | mods; `from_old_store` call; adoption; `match adopted` collapses to `open_figures` (L280); CLI `import-book` L414-416 and `compare-figures` L418-420; L181-192 (`carry_told_by_listing` call) moves with it |
| src/update.rs | 609-610 | 2 (edit) | piece | `STORE_FILES` drops `OLD_FILE`; `bagholder_store::guard_home` at L578, 715, 830 and `tables::set_meta/get_meta/json_text` at L264, 280 re-point when store moves |
| src/engine_inputs.rs | 196-200 | 5 | whole | `old_store_source` (fn L198), used only by compare.rs:29, 135, 194 |
| src/clear.rs | 6-9, 170-171, tests 254-258, 379-396, 474-487, 506-507, 555-575 | ≈66 | piece | doc, the `clear_old_store` call, and the old-store halves of `filled`, `clear_all_...`, `each_kind_alone...` (all `bagholder_store::` references in clear.rs sit after `#[cfg(test)]` at L197) |
| src/following.rs | tests 448-488 | 41 | piece | `old_store` helper and the two carry tests (L454-488; after `#[cfg(test)]` at L407) |
| src/csv_import.rs | tests 908, 1004-1012, 1096-1104, 1137-1145, 1186-1208 | ≈60 rewritten | piece | tests build "earlier import" records with `OldActivity`/`ImportMapping`; they need another mapping as the stand-in, not deletion |
| src/demo.rs | 566 | 1 | whole | the no-`OLD_FILE` assertion |
| src/tests_boundary.rs | 114-122, 124-144, 146-196 | ≈81 | whole | `earlier_quotes`, `earlier_store` and their tests; L11 `EARLIER` list stays until model/store/ws/market go |
| src/tests_failures.rs | 185-213 | 29 | whole | two `carry_watch_folder` tests |
| src/tests_misc.rs | 722 | 1 | whole | the `legacy_import.rs` terminal-lines entry |
| src/tests_types.rs | 16-17, 155-191, 332 (import line), 363 | ≈41 | whole | `book_declarations` (L158) generates web/src/lib/generated/book.ts |
| tests/start_failures.rs | 152-176 | 25 | whole | three "earlier database" start tests |

Server totals: ≈2,786 lines deletable (≈1,824 code, ≈962 tests), 166 lines moved (`carry_told_by_listing` 102 + its test 52 + main.rs 12), ≈60 test lines rewritten in csv_import.rs.

**book crate (rust/crates/book)**

| Path | Range | Lines | Verdict | Notes |
|---|---|---|---|---|
| src/import/mod.rs | 1-402 | 402 | whole, less 35 moved | `Book::import` L93-132, `import_connection` L137-151, `import_accounts` L156-260, `import_groups` L262-281, `import_journal` L283-320, `trade_for_row` L325-352; `wealthsimple_account_type` L370-402 (35 lines) is live: wealthsimple/src/read.rs:5 (used at read.rs:97), adapter.rs:382, engine_inputs.rs:284,355 (tests), wealthsimple/tests/statements.rs:385-386, pull.rs:251,504 → moves to bagholder_wealthsimple (read.rs) or core::account |
| src/import/old.rs | 1-260 | 260 | whole | `copy_database` L22-37 (callers: legacy_import.rs:324,335,396,527; legacy_orders.rs:207; compare.rs:234, all going); `OldDatabase/OldAccount/OldSecurity/OldActivity` L41-104; `read` L141-226; tests L234-260 |
| src/import/mapping.rs | 1-516 | 516 | whole, less 95 moved | `IMPORT_SOURCE`, `WEALTHSIMPLE_RECORD`, `import_source` L31-42; `ImportedRow` L48-60; `ImportMapping` L62, 188-210; `map_row` L212-387; `instrument` L392-429; `option_draft` L434-475; `parse_option_name` L480-501; tests L503-516. The CSV reader's shared rules `Rule`, `Qty`, `Cash`, `rule()` L81-175 (95 lines) are used by broker/src/csv.rs:22, 451-474, the only caller of `rule(` outside mapping.rs → move to bagholder_broker::csv |
| src/trades.rs | 17-24, 29-31, 33-37, 39-41, 46-48, 80-95, 257-290, 317-320, 344/352/361, 463-471, 475-476 | ≈90 | piece | `Reattached`; `open_trade`'s `legacy_key` parameter and column; `orphan_trade`; `reattach` L263 (only caller legacy_import.rs:464); `trade_by_legacy_key`; `add_group`'s `legacy_key` |
| src/notices.rs | 9, 23-28, 30-36, 78-140 | ≈78 | piece | `CARRIED`, `OLD_SETTINGS`, `OLD_SEEN`, `CarriedNotices`, `notices_carried`, `carry_notices` (ATTACHes the old file L91) |
| src/watched.rs | 18-19, 224-255 | ≈34 | piece | `CARRIED`, `following_carried`, `carry_following` |
| src/orders.rs | 103-126, 318-325, 547, 668, 727, 844 | ≈72 | piece | `import_orders`, `import_order`, `import_bracket`, and the `Imported` event (de)serialisation; depends on core's `OrderEvent::Imported` (core/src/order.rs:167, 184, 229-240, 290, 482) and `BracketEvent::Imported` (core/src/bracket.rs:247, 275, 340-372, 1235, 1248), ≈60 lines in core; server/src/orders/doc.rs:369 reads `BracketEvent::Imported` |
| src/lib.rs | 15 | 1 | whole | `pub mod import;` |
| migrations/001-the-book.sql | 195, 201, 206, 209 | 0 | new migration needed | `trades.legacy_key`, `trade_groups.legacy_key` and their unique indexes: a migration 037 drops them and the `bagholder-import` records; schema/v1..v36.sql keep their 4 mentions each (historical snapshots) |
| tests/import.rs | 1-500 | 500 | whole | builds a schema-13 `bagholder.db` in code (`fixture`, L111-114 onward); no committed `*.db` fixture exists (`git ls-files` finds none) |
| tests/notices.rs | 1-99 | 99 | whole | carry tests on an in-code old store (L12-32) |
| tests/watched.rs | 124-143 | 20 | piece | `the_earlier_store_s_rows_are_carried_once_all_or_none` |
| tests/facts.rs | 40 | 1 edit | piece | an INSERT naming `legacy_key` in a migration test |

Book totals: ≈1,453 src lines named, ≈1,323 deletable and 130 moved (95 rules + 35 account types); 620 test lines deletable.

**core crate**: journal.rs:16-17 `legacy_key` field (2); names.rs:2, 100 (2 edits); order.rs/bracket.rs `Imported` variants ≈60 lines (see orders.rs row; conditional on no live book holding an `imported` order/bracket event).

**store crate (rust/crates/store, 9,141 Rust lines: 6,691 src, 2,450 tests; 349 sql; 9,434 golden)**

Crates depending on it: market, server, ws (Cargo.toml greps: market/Cargo.toml:13, server/Cargo.toml:16, ws/Cargo.toml:8). The sources crate does not (sources/Cargo.toml lists core, sqlite, net, book only, lines 13-19; book again as a dev-dependency at L25). `open_db`, `atomically`, `pool`, `open_db_hooked` are re-exports of bagholder_sqlite (store/src/lib.rs:5) and stay there.

Serves only the old file (production callers are only carry.rs, legacy_import.rs, legacy_orders.rs, compare.rs, or none):

| Module | Lines | Evidence |
|---|---|---|
| src/schema.rs | 280 | callers: legacy_import.rs (11), legacy_orders.rs (7), compare.rs (4), carry.rs (3), clear.rs tests (after L197), market tests |
| src/relabel.rs | 158 | carry.rs:292, legacy_import.rs, following.rs test, clear.rs test, market tests |
| src/activities.rs | 483 | legacy_import.rs:36,628, compare.rs:42, tests_types.rs:16 |
| src/merge.rs | 296 | no caller outside store |
| src/csvimport.rs | 1,033 | carry.rs:159,162,226 and tests only |
| src/book.rs | 126 | tests_types.rs:165 only |
| src/broker.rs | 85 | tests_types.rs:165 only |
| src/admin.rs | 314 | carry.rs:222, following.rs:462 (test) |
| src/orders/{mod,typed,types}.rs | 618 | legacy_orders.rs only |
| src/lib.rs `connect` L60-63, `running_tests` L25-30 | ≈13 | `connect` opens `home/bagholder.db` (L62); callers legacy, compare.rs:235, tests |
| src/rows.rs all but `exposures` L114-122, `news` L130-150, `universes` L152-175 | ≈203 | the three are read by server/src/market_context.rs:193-197 (in `build`, L142) on the cache (`app.cache()` L76 [corrected]); the rest (securities, accounts, balances, margin, nav, watchlist, distributions, quotes, fx, benchmark, groups, journal, tiles L253-257, `TILES_META` L229) only by compare.rs:43-65 and carry.rs:142 |
| src/tables.rs all but `get_meta`/`set_meta` L19-37 and the `json_text` family L434-528 | ≈396 | `replace_accounts/balances/margin`, `upsert_nav`, `fx_rates`, `fx_last_date`, `upsert_fx_rates`, `benchmark_*`, `trade_groups`, `LegacyNote` L325-379 etc.: callers are market/src/refresh.rs (old), market/src/quotes.rs:413 (old `coinbase_prev_close`), compare.rs, legacy; `get_meta`/`set_meta` are live on the cache (history.rs:119-139, filings.rs:21, update.rs:264,280) and `json_text` on the book (notify.rs:360) |
| src/market.rs L20-210 (`QuoteRecord`, `StoredQuote`, `DistributionRecord`, `distributions`, `upsert_distributions`, `quotes`, `upsert_quote`, `quote_fetched_at`, `distributions_fetched_at`, `mark_distributions_fetched`) and L397-416 (`BENCHMARK_SYMBOLS`, `market_data`) | ≈211 | callers: market/src/refresh.rs:204-222, market/src/lib.rs:29-36, quotes.rs, tmx.rs:17, parse.rs:15, compare.rs:47 |
| src/feeds.rs watchlist L171-240 (`WatchedListing`, `list_watchlist`, `add_watch`, `remove_watch`) | ≈70 | carry.rs:130 and tests only |
| sql/schema_0.sql, schema_1.sql | 349 | the old file's DDL (schema.rs:35-38 includes them) |

Old-only total ≈ 4,286 of 6,691 src lines.

Still called by the market crate and the server on the market cache (market.db) or the book's notices connection, so they move to bagholder-sources beside the cache's migrations (plan line docs/plans/remove-the-earlier-app.md:103), ≈2,405 lines:

| Module | Lines | Callers |
|---|---|---|
| src/feeds.rs (all but the watchlist block) | ≈1,635 | notify.rs (52 refs: `add_notification` L664, `list_notifications` L944, `mark_notifications_seen` L874 on `book.notices()`), filings.rs:20-21, feeds.rs:20-21, http/notifications.rs:28-63, docs.rs:171-172, market/src/news.rs:28, exposure.rs:27,1151,1175, shorts.rs:31, fear.rs:14, edgar.rs:22, sedar.rs:23, disclosures.rs:22, interlisted.rs:11; book/migrations/015-notifications.sql:5 names it; book/src/notices.rs:40-44 documents `notices()` as the connection for `bagholder_store::feeds` |
| src/bars.rs | 160 | history.rs:20, parse.rs:14, quotes.rs:20, feeds.rs:20, demo_facts.rs:51 |
| src/market.rs L216-394 (`price_history`, `upsert_price_history`, `history_fetch`, `mark_history_fetched`, `price_bars`, `upsert_price_bars`, `last_bar_time`, `bar_fetch`, `mark_bars_fetched`) | ≈185 | history.rs:549-576, 608, 996-1245, 1257, 1338; demo_facts.rs:132-133; the cache's own migration 006 creates these tables (sources/migrations/006-market-data-by-listing.sql:11-42) |
| src/gens.rs | 169 | versions.rs:14-21, market_context.rs:78-80 (cache's `gen` table, sources/migrations/004:161) |
| src/tables.rs `get_meta`/`set_meta`/`json_text` family | ≈132 | above |
| src/rows.rs `exposures`/`news`/`universes` | 54 | market_context.rs:193-197 |
| src/lib.rs `guard_home` L36-55 | ≈33 | app.rs:377,403; login.rs:1133; update.rs:578,715,830; ws/src/session.rs:271; market/src/localmodel.rs:168 (and carry.rs:246,280, going) |

Store tests: deletable whole 1,678 (activities.rs 222, durability.rs 131, golden_store.rs 1,047, guard.rs 32, orders.rs 144, relabel.rs 102); 772 to rebuild on the cache's migrations (common/mod.rs 94, gens.rs 145, news.rs 56, pool.rs 113, rows.rs 68, stored_feeds.rs 78, tables.rs 218). Fixtures deletable: tests/golden/store.json 9,218, mapped.json 170, golden/csv/*.csv 46 (9 files) = 9,434; README.md 21.

**market crate (rust/crates/market)**

| Path | Range | Lines | Verdict | Evidence |
|---|---|---|---|---|
| src/refresh.rs | 1-288 | 288 | whole | callers outside tests: none (`refresh::` only in market/tests/failures.rs:92-99); declared `pub mod refresh;` at lib.rs:7; it writes the old `fx_rates`/`benchmark_prices` tables (L16-18, 56, 82, 106) |
| src/quotes.rs | 73-75, 85-88, 170-197, 211-216, 244-278, 302-330, 332-361, 363-376, 378-389, 391-434, 436-454, 456-465, 467-483, 485-510, 512-530, 532-561, 563-566, 581-602 | ≈350 of 602 | piece | `yahoo_get` (L73), `yahoo_get_public` (86), `parse_yahoo_quote` (171), `fetch_yahoo_quote` (213), `quote_source` (246), `markets_open` (303), `last_settled` (311), `can_have_moved` (322), `quote_symbols_needing_refresh` (332), `instant_secs` (364), `fetch_cboe_ca_quote` (378), `fetch_cboe_option_chain` (383), `coinbase_prev_close` (395; reads `tables::fx_rates` L413), `float_repr` (438), `fetch_coinbase_spot` (458), `with_prev_close` (468), `occ_root` (477), `fetch_for` (488), `refresh_quotes` (512), `stale_symbols` (538), `n` (564), `peek_quote` (583) have no production caller outside quotes.rs/refresh.rs (every `quotes::`/`use crate::quotes` reference elsewhere names only the kept set). Kept: `yahoo_get_result` L79, `instant_secs_public` L91, `parse_yahoo_chart` L101, `percent_encode` L199, `venue_mic` L222, `yahoo_forms` L230, `tmx_quote_symbol` L236, `yahoo_may_ask`/`yahoo_back_off` L570/575 (callers history.rs:14,363-364,553,982,1216,1249,1259; news.rs:681,1820; shorts.rs:478,623,684,742,785; exposure.rs:379,972; search.rs:92,190; server app.rs:540), and the `SourceQuote` type (tmx.rs:15, parse.rs:17); private helpers `yahoo_paced` L60, `yahoo_turn` L66, `weekday` L298 follow their users |
| src/lib.rs | 27-36 [corrected: the file is 78 lines; `market_fetched` is L27-31 and `pub mod market` L33-36, not 318-327] | 10 | whole | `market_fetched` (only caller quotes.rs:338) and the `pub mod market` re-export of `distributions_fetched_at`, `quotes`, `upsert_quote` (L35) |
| src/tmx.rs | 17, `fetch_tmx` L215 | ? | piece | `fetch_tmx` is called only by refresh.rs:190; `fetch_tmx_quote` (disclosures.rs:198, quotes.rs:496), `is_canadian_listing`, `tmx_bare`, `tmx_listings`, `TMX_URL` are live; line count of the piece not measured |
| tests/failures.rs, quotes.rs, quote_refresh.rs, quote_records.rs | whole | 539 | whole | test refresh and the old quote functions |
| tests/{bars, exposure, exposure_records, gauges_universes, filings_items, news, news_items, listing_isolation, shorts, shorts_records, edgar, sedar, disclosures}.rs | setup lines | rewrite | piece | build their DB with `bagholder_store::schema::init_schema`/`relabel::ensure`/`connect` (counts in the listing: news.rs 12 refs, listing_isolation.rs 14, exposure_records.rs 14); carry.rs:636-638 shows the replacement (`bagholder_sources::cache::MIGRATIONS`) |

**sources crate**: src/cache.rs:57-80 `carry_filings` (fn L62-80, its doc L57-61 [corrected]; L43-48 is the doc of `FILER_OF_ROW`, the rule migration 6 shares) deletable; its `filings_carried` table (sources/migrations/006-market-data-by-listing.sql:129, filled at :145) is also read by `carry_told_by_listing` (carry.rs:356) and cleared by Clear data (clear.rs:84), so a new migration drops it once both go; tests/cache.rs:437-450 (14) deletable; migrations/004-the-earlier-readers.sql:4-5 comment names `bagholder.db` and `bagholder_store::feeds`.

**web**: web/src/lib/generated/book.ts (27 lines) whole: generated by tests_types.rs:158-191 from store's `ActivityRow`, `LegacyNote`, `broker::Account`, `book::*`; nothing in web/src imports it except model_api.ts:5 (`import type { LegacyNote } from './book'`, written by tests_types.rs:332), and `LegacyNote` appears nowhere else in model_api.ts.

**docs**: docs/plans/stage-3c-switch.md (281), stage-6-cutover.md (128), old-app-mistakes.md (120): the plan marks them historical rather than deleting (docs/plans/remove-the-earlier-app.md:108); README.md:189 (the `~/.bagholder/bagholder.db` adoption sentence); rust/crates/store/README.md (21); .gitignore:8.

### 1c. Totals

| | Deletable code (non-test) | Deletable tests | Deletable fixtures | Moved |
|---|---|---|---|---|
| server | ≈1,824 | ≈962 | 0 | 166 (+≈60 test lines rewritten) |
| book | ≈1,323 | 620 | 0 | 130 |
| core | ≈60 (Imported variants, legacy_key) | 0 | 0 | 0 |
| store | ≈4,286 | 1,678 (+772 rebuilt) | 9,434 golden + 349 sql | ≈2,405 to sources |
| market | ≈648 (refresh 288, quotes ≈350, lib 10) | 539 (+13 files' setup rewritten) | 0 | tmx `fetch_tmx` piece |
| sources | ≈20 | 14 | 0 | 0 |
| web | 28 | 0 | 0 | 0 |
| **total** | **≈8,190** | **≈3,813** | **≈9,783** | **≈2,700** |

### 1d. Beyond the list, but earlier-app by the repository's own words

- rust/crates/model: `fifo`, `base`, `view`, `trades`, `positions`, `nav`, `cashflow`, `filters`, `stats`, `fold`, `normalize`, `synth`, `fx`, `book`, `markets`, `cases`, `testing`, `bin/make_cases` = 5,109 src lines [corrected: `wc -l` over those 18 files; the report said 5,127] with no production user outside model, store, compare.rs and legacy_import.rs (grep over non-test paths finds none, and model/src/lib.rs re-exports none of them; legacy_import.rs:7-8 calls this "the old engine, which lives here until the cutover"; docs/plans/stage-3c-switch.md:181 listed base/view/fifo/fx and the old cases runner as "what goes"). Not deletable whole: the live `context.rs` (MarketBase, market_context.rs:189-199), `wire.rs` and `symbols_of.rs` reference `crate::base` (context.rs:62), `crate::nav` (wire.rs:809), `crate::filters` (wire.rs:837), `crate::fifo` (wire.rs:855, symbols_of.rs:11), `crate::trades` (symbols_of.rs:13). Model tests 2,070 lines.
- tests/cases (14,318 lines) and tests/wire (108,621) are "the old model's cases ... generated from the old Python model, mistakes included" (tests/README.md:1-3), read by model/tests/cases.rs:12, diff/src/lib.rs:494 (a test in that crate's src), and the iOS/Android ports (MOBILE.md:3; ios/Bagholder/Model.swift 2,032 lines; android/model 2,701). These 122,939 lines are constant across v2.0.0, v2.7.0 and v2.9.0.

## 2. PR #575 and PR #586

**PR #575**, merge `bfedbae` "A move's leaving side the feed never lists is booked from its arrival (plan #573, Part 1)", branch move-out-from-the-arrival, 6 commits (`11af3d1`, `1d05795`, `d11ece2` plus three dependency bumps `def78c4`, `81d4827`, `69cbf7e`). 9 files, +832 −43 (`git diff --numstat bfedbae^1 bfedbae`).

| File | + | − | Added (function/method/field) |
|---|---|---|---|
| rust/crates/broker/src/lib.rs | 33 | 0 | field `imported: bool`; `BrokerAdapter` default methods `refreshes`, `refresh`, `watches`, `amend` |
| rust/crates/broker/src/pull.rs | 149 | 25 | `type Watching`; `store_row` gains `watching: &Watching`; new `supersede_copies`; `link_moves` now takes `&[RecordId]` |
| rust/crates/wealthsimple/src/adapter.rs | 140 | 1 | `opposing`, `lacks_opposing`, `watches`, `amend`, `refreshes`, `refresh` |
| rust/crates/wealthsimple/src/assemble.rs | 40 | 1 | field `opposing: Option<String>`; trait method `opposing`; `is_move`, `opposing`, `days_read` |
| rust/crates/wealthsimple/src/mapping.rs | 79 | 2 | `leaving` |
| rust/crates/wealthsimple/src/record.rs | 32 | 1 | field `opposing: Option<Opposing>`; `struct Opposing { account, rows }`; field `imported: bool` |
| rust/crates/wealthsimple/tests/common/mod.rs | 17 | 6 | |
| rust/crates/wealthsimple/tests/mapping.rs | 129 | 4 | |
| rust/crates/wealthsimple/tests/pull.rs | 213 | 3 | tests seed `bagholder-import` records at pull.rs:201, 255, 506, 718, 754, 789 |

**PR #586**, merge `784bfbb` "Store a closed account's late rows; name the broker under Net worth rows; sort Net worth's lists; 2.8.1", branch closed-rows-and-card-names, 5 commits (`2fc809f`, `fd8488f`, `ebaba70`, `ba6308a`, `490dc40`). 17 files, +591 −58.

| File | + | − | Added |
|---|---|---|---|
| rust/crates/broker/src/lib.rs | 20 | 0 | default methods `watched_rows`, `names`, `read_beside` |
| rust/crates/broker/src/pull.rs | 146 | 20 | `held`, `closed_due`, `replaced_by`; `index` gains `replaced: &BTreeSet<RecordId>` |
| rust/crates/engine/src/scope.rs | 38 | 3 | `pub struct HeldAt { holdings, invested_cash, chequing_cash, margin }`, `fn note`, field `held_at: HeldAt` |
| rust/crates/engine/tests/net_worth.rs | 63 | 0 | |
| rust/crates/wealthsimple/src/adapter.rs | 30 | 1 | `watched_rows`, `names`, `read_beside` |
| rust/crates/wealthsimple/tests/pull.rs | 209 | 4 | |
| rust/crates/server/src/app.rs | 1 | 1 | `APP_VERSION` 2.8.0 → 2.9.0 |
| rust/crates/server/src/wire/build.rs | 8 | 5 | |
| rust/crates/server/tests/golden/routes.json | 1 | 1 | |
| web/src/lib/NetWorth.svelte | 15 | 3 | `type Row`, `const sorted` |
| web/src/lib/sort.svelte.ts | 5 | 0 | |
| web/src/lib/sort.svelte.test.ts | 2 | 0 | |
| web/e2e/networth.spec.ts | 32 | 1 | |
| SPEC.md 2/1, docs/spec-ids.md 1/0, rust/Cargo.lock 17/17, rust/Cargo.toml 1/1 | | | |

## 3. Size of the repository

Tags: v2.0.0 = `08d58b1` 2026-09-28; v2.7.0 = `5af1bbc` 2026-10-19; v2.9.0 = `784bfbb` 2026-10-20. Counts via `git archive <tag>` extracted into the scratchpad and `wc -l` (the extracted trees' `.rs` totals equal those computed straight from git). (a) = `.rs` under `src/` minus files named `tests_*.rs`/`tests.rs` minus the lines of `#[cfg(test)]` blocks (attribute line to matching brace); (b) = those blocks; (c) = `.rs` under `tests/` plus the `tests_*.rs` files; (d) = generated or recorded.

| Lines | v2.0.0 | v2.7.0 | v2.9.0 | Δ 2.0→2.9 |
|---|---|---|---|---|
| Rust `.rs` total | 127,842 | 156,294 | 159,843 | +32,001 |
| (a) hand-written non-test src | 88,981 | 104,536 | 106,427 | +17,446 |
| (b) `#[cfg(test)]` inside src | 7,822 | 12,560 | 12,814 | +4,992 |
| (c) `tests/` `.rs` | 25,157 | 30,874 | 32,278 | +7,121 |
| (c) `tests_*.rs` in src | 5,846 | 8,288 | 8,288 | +2,442 |
| (a+b+c remainder: build.rs etc.) | 36 | 36 | 36 | 0 |
| (d) rust/crates/book/schema/*.sql | 8,704 | 17,053 | 19,956 | +11,252 |
| (d) tests/golden/* (crates) | 15,949 | 16,078 | 16,037 | +88 |
| (d) tests/replies/* | 78,181 | 91,023 | 91,024 | +12,843 |
| (d) other recorded test files in crates (json, html) | 11,434 | 14,068 | 14,068 | +2,634 |
| (d) web/src/lib/fixtures/*.json | 9,783 | 9,787 | 20,594 | +10,811 |
| (d) web/src/lib/generated/* | 1,683 | 1,974 | 2,012 | +329 |
| (d) top-level tests/ (old-model cases, wire, fixtures) | 123,986 | 123,986 | 123,986 | 0 |
| web/src hand-written (no fixtures, generated, *.test.ts) | 10,864 | 11,749 | 11,970 | +1,106 |
| web/src unit tests | 2,239 | 2,700 | 2,702 | +463 |
| web/e2e | 6,059 | 7,758 | 8,085 | +2,026 |
| migrations .sql | 1,451 | 2,452 | 2,481 | +1,030 |
| docs/*.md | 2,851 | 6,341 | 6,879 | +4,028 |

Note: (a) counts rust/crates/wealthsimple/src/generated.rs (1,029 at v2.7.0, 1,260 at v2.9.0) as hand-written because its header says it is a made-up stand-in ("A made-up Wealthsimple of any size, answering as the real one does"), not generated code (generated.rs:1-5); rust/crates/sources/src/html/entities.rs (2,240) is a WHATWG table (entities.rs:1-4) counted in (a).

Where the growth went: 100 first-parent commits v2.0.0..v2.9.0, +95,821 −15,416. The ten largest by lines changed (`git diff --numstat <merge>^1 <merge>`, classified by path):

| PR | + | − | logic | tests | fixtures/snapshots/generated | migrations | docs | the fixture lines |
|---|---|---|---|---|---|---|---|---|
| #525 market-data-by-listing (`ddb5a3c`) | 15,457 | 5,071 | +7,738 −4,087 | +2,730 −627 | +4,495 −345 | +238 | +256 −12 | market/tests/replies/sedar-tsx60/sedar-resolve.json 1,154; book/schema/v31.sql 937; filings.rs +2,568 is largely feeds.rs −2,257 moved |
| #577 networth-try (`fdeb466`) | 16,780 | 788 | +1,447 −155 | +903 −36 | +14,346 −591 | +29 | +55 −6 | web/src/lib/fixtures/figures_pulled_month.json +11,372 −565; schema v34-v36 2,903 |
| #540 fund-of-funds-sectors (`c33eaa2`) | 10,765 | 15 | +193 −13 | +294 −1 | +10,278 −1 | 0 | 0 | sources/tests/replies/evolve/breakdown-life.html 4,899, breakdown-easy.html 3,971, topholdings-XEQT.TO.json 236 |
| #507 record-by-change (`4b3b6041`) | 5,221 | 489 | +3,108 −436 | +811 −48 | +906 −2 | +392 | +4 −3 | schema v28 871 |
| #487 stage-p (`2ef4e6d3`) | 4,439 | 495 | +1,141 −82 | +157 −1 | +1,045 −30 | 0 | +2,096 −382 | wealthsimple/src/generated.rs 985 (classified by name; hand-written), SPEC.md +1,045, docs/spec-ids.md 968 |
| #514 deep-history (`a56126d1`) | 3,900 | 310 | +2,019 −298 | +840 −8 | +936 | +101 | +4 −4 | schema v29 935 |
| #557 debts-held-outside (`097e36a`) | 3,260 | 492 | +1,417 −33 | +218 −11 | +1,466 −443 | +24 | +135 −5 | schema v33 961; figures_pulled_month.json ±440 |
| #555 notifications-through-the-browser (`740b3ed`) | 2,807 | 528 | +1,289 −472 | +341 −38 | +975 −5 | +11 | +191 −13 | schema v32 943 |
| #503 plan-stage-3-statements (`b0a8d3cc`) | 1,937 | 233 | +710 −229 | +315 −3 | +574 | +161 | +177 −1 | schema v26 574 |
| #520 orders-name-the-listing (`7e0782f1`) | 1,934 | 361 | +811 −229 | +886 −82 | +86 −44 | 0 | +151 −6 | |

Of the three largest PRs, 29,119 of 43,002 added lines are fixtures or snapshots (#577: 14,346 of 16,780; #540: 10,278 of 10,765; #525: 4,495 of 15,457). Each book migration adds a full schema snapshot of ≈870-970 lines (schema/v26..v36).

## 4. Largest hand-written files (non-test files; inline `#[cfg(test)]` lines in brackets)

| # | File | Lines | What it is |
|---|---|---|---|
| 1 | rust/crates/engine/src/ledger.rs | 3,157 [92] | lots and round trips over the book's transactions (ledger.rs:1-2) |
| 2 | rust/crates/server/src/filings.rs | 2,775 [1,110] | the disclosures and release notices: filers, documents, the sweep, enrichment (filings.rs:1-3) |
| 3 | rust/crates/server/src/feeds.rs | 2,269 [437] | market data, news, filings and their sweeps; watchlist and tile writes, gauges, shorts, universes (feeds.rs:1-3) |
| 4 | rust/crates/sources/src/html/entities.rs | 2,240 | the WHATWG named character reference table, a data table not logic (entities.rs:1-4) |
| 5 | rust/crates/market/src/news.rs | 2,005 | news from every per-symbol public source, merged (news.rs:1-3) |
| 6 | rust/crates/engine/src/engine.rs | 1,936 | the engine's entry point: build from inputs, apply one change, say what moved (engine.rs:1-3) |
| 7 | rust/crates/server/src/figures.rs | 1,786 [919] | the figure path: engine built from book and cache, kept current, journal writes (figures.rs:254-258, 300) |
| 8 | rust/crates/server/src/notify.rs | 1,770 [819] | notifications through the browser (notify.rs:1-2) |
| 9 | rust/crates/server/src/capacity.rs | 1,728 [531] | `bagholder capacity`: the server held to its cost budget (capacity.rs:1-2) |
| 10 | rust/crates/store/src/feeds.rs | 1,705 | store reads/writes for exposure, watchlist, news, filings, shorts, gauges, notifications (feeds.rs:1-3) |
| 11 | rust/crates/engine/src/scope.rs | 1,686 | what one filter set produces: tiles, tables, cards (scope.rs:1-3) |
| 12 | rust/crates/server/src/csv_import.rs | 1,436 [635] | CSV import, folder watching, linking file rows to broker rows (csv_import.rs:354-365, 486) |
| 13 | rust/crates/server/src/login.rs | 1,412 [158] | connecting Wealthsimple through a Chromium window over DevTools (login.rs:1-3) |
| 14 | rust/crates/market/src/exposure.rs | 1,401 | sector and country exposure of holdings (exposure.rs:1-3) |
| 15 | rust/crates/market/src/history.rs | 1,375 | daily bars for the chart from several sources (history.rs:1-3) |
| 16 | rust/crates/wealthsimple/src/mapping.rs | 1,321 | Wealthsimple's records into transactions (mapping.rs:1-2) |

Next: core/src/bracket.rs 1,317 [151] (the bracket state machine), server/src/wire/build.rs 1,313 [343], wealthsimple/src/generated.rs 1,260 (the made-up Wealthsimple stand-in), market/src/sedar.rs 1,229 [178]. Files not in the bracketed list have under 87 lines of inline tests.

## 5. Decision on record

docs/decisions.md:5 `## 2026-10-21`, :7 "Nothing of the earlier app is kept, by a one-time migration, never a standing condition ... No step waits on a condition, runs at every start, or keeps an earlier-app row because it still counts", added by `17cb6c7` (2 insertions, L7-8).

## verdicts
- confirmed: from_old_store runs at every start and carry_told_by_listing at every start | rust/crates/server/src/main.rs:160 `if let Err(e) = carry::from_old_store(a, &f, bagholder_core::jiff::Timestamp::now())` and :184 `carry::carry_told_by_listing(&b, &cache, ...)`, both inside `fn figures_opened` (main.rs
- corrected: adopt_python_database runs at every start, stat-ing ~/.bagholder/bagholder.db | The call runs at every start whose HOME is non-empty: main.rs:234-237 `match legacy_import::python_home(..) { Some(python) => legacy_import::adopt_python_database(&home, &python), None => Ok(None) }`, python_home at lega
- confirmed: Figures::open imports bagholder.db when no book.db exists, on every open | rust/crates/server/src/figures.rs:393 `pub fn open(home: &Path, at: Timestamp)`, :394-398 `let book_path = home.join(bagholder_book::BOOK_FILE); let old = home.join(OLD_FILE); if !book_path.exists() && old.exists() { cra
- confirmed: reattach_carried_notes runs on every engine build, guarded by a repair setting | figures.rs:259 `fn built(home, book, cache, zone, now)` (doc L254-258: the engine built whole), :269-272 `if crate::legacy_import::reattach_carried_notes(home, book, &e, now)? { catch_up(..); settle_trades(..); }`. legac
- confirmed: ImportMapping is in the mappings list the start re-derives under | figures.rs:886-887 `pub fn mappings() -> [&'static dyn Mapping; 5] { [&WealthsimpleMapping, &bagholder_book::import::mapping::ImportMapping, &PersonMapping, &CsvMapping, &FillMapping] }`; `fn rederive_all` (figures.rs:89
- confirmed: clear_old_store is the one write to the old file besides the carry and runs on every Clear | carry.rs:239-244 doc ending `/// The one write to the earlier store besides the carry.`, :245 `pub fn clear_old_store(home: &Path, kinds: &[Kind])`, :251-268 opens the file and DELETEs; clear.rs:171 `crate::carry::clear_
- confirmed: STORE_FILES names OLD_FILE so a rolled-back update restores bagholder.db | update.rs:610 `const STORE_FILES: [&str; 3] = [bagholder_book::BOOK_FILE, crate::figures::CACHE_FILE, crate::figures::OLD_FILE];`, used by `fn restore_stores` (update.rs:617) at :621 `for file in STORE_FILES` with `snaps
- confirmed: carry.rs is 676 lines; carry_told_by_listing and helpers (L322-423) and its test (L624-675) do not read bagholder.db | `wc -l` 676. carry.rs:322-325 TOLD_BY_LISTING, :328 mark_moment, :339 told_scopes (reads `filings_carried` on the cache, L356), :365 earlier_form, :379-423 `pub fn carry_told_by_listing` reading `book.notices()` (L384). 
- corrected: legacy_import.rs 994 lines (tests from L595), legacy_orders.rs 310 (tests from L247), compare.rs 612 (tests from L594) | `wc -l`: 994, 310, 612. The `#[cfg(test)]` attributes are at legacy_import.rs:594 (`mod tests` L595), legacy_orders.rs:247 (`mod tests` L248), compare.rs:594 (`mod tests` L595). So legacy_import's test block is L594-994 
- confirmed: translate runs the old engine bagholder_model::book::build_book | legacy_import.rs:35 `pub fn translate(conn: &Connection, today: &str)`, :39 `let book = bagholder_model::book::build_book(&rows, bagholder_model::securities::Securities::new(&securities), today);`; the module doc L7-8 ca
- confirmed: demo_facts.rs never opens bagholder.db; it writes the market cache through bagholder_store types | demo_facts.rs has no OLD_FILE or bagholder.db reference (grep); :90 `MarketCache::open(&home.join(crate::figures::CACHE_FILE), ..)`, :109 `bagholder_sqlite::open_db(&home.join(crate::figures::CACHE_FILE))`, :132-133 `bag
- confirmed: old_store_source is 5 lines used only by compare.rs | engine_inputs.rs:197-200 (doc L197, `pub fn old_store_source() -> SourceName` L198, `SourceName::named("bagholder-old-store")` L199, `}` L200; L196 blank). Callers (grep): compare.rs:29 `use crate::engine_inputs::{self, 
- confirmed: book/src/import is 1,178 lines (mod.rs 402, old.rs 260, mapping.rs 516) | `wc -l rust/crates/book/src/import/*.rs`: mapping.rs 516, mod.rs 402, old.rs 260, total 1178.
- confirmed: The CSV reader uses import's rule/Qty/Cash, the only live use of the import module's rules | rust/crates/broker/src/csv.rs:22 `use bagholder_book::import::mapping::{rule, Cash, Qty};`, :451 `let Some(r) = rule(ty, sub, cash, &direction) else {..}`, :456-466 match on `Qty::*` and `Cash::*`. Grep for `bagholder_bo
- confirmed: wealthsimple_account_type (import/mod.rs:370-402) is used by the live Wealthsimple adapter | rust/crates/book/src/import/mod.rs:370 `pub fn wealthsimple_account_type(words: &str) -> AccountType` through :402 `}` (file is 402 lines). rust/crates/wealthsimple/src/read.rs:5 `use bagholder_book::import::wealthsimple
- confirmed: Book::reattach's only production caller is legacy_import.rs | rust/crates/book/src/trades.rs:263 `pub fn reattach(&self, orphan: TradeId, to: &Opening) -> Result<Reattached>`; grep `.reattach(` over rust/crates finds only legacy_import.rs:464 `if book.reattach(trade, &to).map_err(e
- confirmed: OrderEvent::Imported and BracketEvent::Imported live in core and are read by orders/doc.rs | rust/crates/core/src/order.rs:167 `Imported { state: OrderState, broker_id: Option<String>, filled: Dec, average: Option<Dec>, why: Option<String>, row: String },` (doc L165-166 "Carried over from the earlier app's store
- confirmed: legacy_key columns are created by migration 001 and appear in every schema snapshot v1-v36 | rust/crates/book/migrations/001-the-book.sql:195 `legacy_key TEXT,`, :201 `CREATE UNIQUE INDEX trades_legacy_key ON trades (legacy_key) WHERE legacy_key IS NOT NULL;`, :206 `legacy_key TEXT,`, :209 `CREATE UNIQUE INDEX t
- confirmed: The store crate is 9,141 Rust lines (6,691 src, 2,450 tests) plus 349 sql lines and 9,434 golden fixture lines | `git ls-files rust/crates/store` piped to `wc -l`: src .rs 6691, tests .rs 2450, all .rs 9141; sql/schema_0.sql + schema_1.sql 349; golden: tests/golden/store.json 9218, mapped.json 170, nine csv files 46 (2+4+9+3+4+15+2
- confirmed: bagholder_store::connect opens home/bagholder.db | rust/crates/store/src/lib.rs:60 `pub fn connect(home: &std::path::Path) -> rusqlite::Result<rusqlite::Connection> {`, :61 `guard_home(home)..`, :62 `open_db(&home.join("bagholder.db"))`, :63 `}` (L57-59 are its doc). The
- confirmed: store's schema, relabel, activities, merge, csvimport, book, broker, admin, orders modules have production callers only in carry.rs, legacy_import.rs, legacy_or | `grep -rn 'bagholder_store::<module>' rust/crates` excluding store, tests/ and tests_*: schema in carry.rs(3), clear.rs(4), compare.rs(4), legacy_import.rs(11), legacy_orders.rs(7); relabel in carry.rs, clear.rs, followi
- confirmed: store::feeds notification functions are called by notify.rs on the book's notices connection | rust/crates/server/src/notify.rs:664 `bagholder_store::feeds::add_notification(book.notices(), kind, key, title, body, ..)`, :874 `bagholder_store::feeds::mark_notifications_seen(b.notices(), &[row.id], &now_iso())`, :94
- confirmed: store::market price_history/price_bars functions are called by market/src/history.rs on the cache | rust/crates/market/src/history.rs:549 `bagholder_store::market::history_fetch(conn, &sym)`, :570 `upsert_price_history`, :576 `mark_history_fetched`, :608 `price_history`, :996 `price_bars`, :1018/:1212 `bar_fetch`, :122
- corrected: store::gens is read by versions.rs and market_context.rs on the cache connection | rust/crates/server/src/versions.rs:14 `use bagholder_store::gens;`, :17-22 `gens::all(conn)`, `gens::names()`, `gens::key(..)`. rust/crates/server/src/market_context.rs:22 `use bagholder_store::{gens, rows};`, the reads 
- confirmed: store::rows exposures/news/universes are read by market_context.rs on the cache | market_context.rs:193 `exposures: Arc::new(rows::exposures(conn).map_err(e)?)`, :195 `news: Arc::new(rows::news(conn)..)`, :197 `universes: Arc::new(rows::universes(conn)..)` inside `fn build(f, following, conn: &rusqlit
- confirmed: store::guard_home is used by app.rs, login.rs, update.rs, ws/session.rs, market/localmodel.rs | grep `bagholder_store::guard_home`: rust/crates/server/src/app.rs:377, :403; login.rs:1133; update.rs:578, :715, :830; rust/crates/ws/src/session.rs:271; rust/crates/market/src/localmodel.rs:168; plus carry.rs:246, :280 
- confirmed: The sources crate does not depend on bagholder-store | rust/crates/sources/Cargo.toml:12-19 `[dependencies]` lists only bagholder-core (L13), bagholder-sqlite (L15), bagholder-net (L17), bagholder-book (L19), then rusqlite (L21); `[dev-dependencies]` L23-26 adds bagholder-bo
- confirmed: market/src/refresh.rs (288 lines) has no production caller; only market/tests/failures.rs calls it | `wc -l` 288. `grep -rn 'refresh::' rust` outside refresh.rs: rust/crates/market/tests/failures.rs:92 `refresh::refresh_all(&conn, &[])`, :97 `refresh::refresh_fx(&conn)`, :99 `refresh::series_stale(&conn, "2026-09-16")` 
- confirmed: 22 functions of market/src/quotes.rs (about 350 lines) have no production caller outside quotes.rs and refresh.rs | quotes.rs is 602 lines; the 22 named functions are at L73 yahoo_get, 86 yahoo_get_public, 171 parse_yahoo_quote, 213 fetch_yahoo_quote, 246 quote_source, 303 markets_open, 311 last_settled, 322 can_have_moved, 332 quote_
- corrected: market_fetched in market/src/lib.rs is called only by quotes.rs:338 | The caller is right: rust/crates/market/src/quotes.rs:338 `let fetched = crate::market_fetched(conn)?;` is the only one (grep). The location is wrong: market/src/lib.rs is 78 lines; `pub fn market_fetched(conn)` is at li
- confirmed: carry_filings lives in sources/src/cache.rs and its table filings_carried is also read by carry_told_by_listing | rust/crates/sources/src/cache.rs:62 `pub fn carry_filings(conn: &Connection, from: &str) -> rusqlite::Result<(usize, usize)>` (doc L57-61; L43-48 is `FILER_OF_ROW`'s doc, which the report mislabels as carry_filings' doc 
- confirmed: web/src/lib/generated/book.ts (27 lines) is imported only by model_api.ts:5 for LegacyNote, which model_api.ts does not otherwise use | `wc -l web/src/lib/generated/book.ts` 27. grep over web/src: the only import of './book' is model_api.ts:5 `import type { LegacyNote } from './book'`; `LegacyNote` otherwise appears only in book.ts:9 (its definition) and
- confirmed: No *.db fixture is committed; book/tests/import.rs builds its old database in code | `git ls-files | grep -c '\.db$'` = 0. rust/crates/book/tests/import.rs:111 `fn fixture(dir: &Path) -> std::path::PathBuf {`, :112 `let path = dir.join("bagholder.db");`, :114 `conn.execute_batch(OLD_SCHEMA).unwrap();` (4
- confirmed: PR #575 merge bfedbae changed 9 files, +832 -43; PR #586 merge 784bfbb changed 17 files, +591 -58 | `git diff --shortstat bfedbae^1 bfedbae`: 9 files changed, 832 insertions(+), 43 deletions(-) (bfedbae = "Merge pull request #575 from ProfessorBagholder/move-out-from-the-arrival"); `git diff --shortstat 784bfbb^1 784bf
- confirmed: Rust .rs lines: 127,842 at v2.0.0, 156,294 at v2.7.0, 159,843 at v2.9.0 | `git ls-tree -r --name-only <tag> | grep '\.rs$'` each piped through `git cat-file -p` and `wc -l`: v2.0.0 (08d58b1, 2026-09-28) 127842; v2.7.0 (5af1bbc, 2026-10-19) 156294; v2.9.0 (784bfbb, 2026-10-20) 159843. The extra
- confirmed: Hand-written non-test Rust (src minus tests_*.rs minus cfg(test) blocks): 88,981 / 104,536 / 106,427 | Re-run with `python3 -I scratchpad/tools/cfgtest.py` over `find sizes/<tag>/rust -name '*.rs' -path '*/src/*' ! -name 'tests_*.rs' ! -name 'tests.rs'`: v2.0.0 ALL=96803 CFGTEST=7822 NONTEST=88981; v2.7.0 ALL=117096 CFGTE
- confirmed: book schema snapshots: 8,704 / 17,053 / 19,956 lines; tests/replies: 78,181 / 91,023 / 91,024; web fixtures: 9,783 / 9,787 / 20,594 | Per tag via `git ls-tree -r` + `git cat-file -p` + `wc -l`: rust/crates/book/schema/*.sql 8704 / 17053 / 19956; every path containing /tests/replies/ 78181 / 91023 / 91024; web/src/lib/fixtures/*.json 9783 / 9787 / 20594
- confirmed: 100 first-parent commits between v2.0.0 and v2.9.0, +95,821 -15,416 | `git log --first-parent --oneline v2.0.0..v2.9.0 | wc -l` = 100; summing `git log --first-parent --shortstat --format= v2.0.0..v2.9.0` gives +95821 -15416.
- confirmed: PR #577 added 11,372 lines to web/src/lib/fixtures/figures_pulled_month.json; PR #540 added 8,870 lines of evolve HTML replies | `git diff --numstat fdeb466^1 fdeb466` (Merge pull request #577 .. networth-try): `11372 565 web/src/lib/fixtures/figures_pulled_month.json`. `git diff --numstat c33eaa2^1 c33eaa2` (Merge pull request #540 .. fund-of-fun
- confirmed: tests/cases and tests/wire are the old Python model's recorded cases, 122,939 lines, read by model tests, the diff crate and the mobile ports | `git ls-files tests/cases | xargs cat | wc -l` 14318, tests/wire 108621 (sum 122939; all of top-level tests/ 123986). tests/README.md:1-3 `# The old model's cases` .. `These cases were generated from the old Python model
- corrected: 17 model crate modules (5,127 src lines) have no production user outside model, store, compare.rs and legacy_import.rs, but context.rs, wire.rs and symbols_of.r | `wc -l` over the 18 files named (fifo 1029, base 324, view 518, trades 235, positions 152, nav 312, cashflow 60, filters 247, stats 174, fold 139, normalize 248, synth 151, fx 72, book 60, markets 426, cases 888, testing
- confirmed: The owner decided on 2026-10-21 that the earlier app goes by one migration, never a standing condition | docs/decisions.md:5 `## 2026-10-21`, :7 `- **Nothing of the earlier app is kept, by a one-time migration, never a standing condition.** The earlier app's rows are deleted outright in one migration on every book, ... No s
- confirmed: wealthsimple/src/generated.rs is a hand-written stand-in despite its name | rust/crates/wealthsimple/src/generated.rs:1-5 `//! A made-up Wealthsimple of any size, answering as the real one does: every reply is one of the recorded replies (tests/replies) with its fields set, so the adapter, the m

## open
- Do any live books (the owner's two machines) hold `imported` order or bracket events in their event logs (legacy_orders.rs:93, 176 write them)? If so, removing OrderEvent::Imported / BracketEvent::Imported from core needs a migration of those rows; otherwise the variants and book/src/orders.rs:103-126, 318-325 go whole.
- The exact line count of market/src/tmx.rs's `fetch_tmx` and distribution parsing (called only by refresh.rs:190) and of parse.rs's BoC/FRED/Stooq/TMX-history parsers (used by refresh.rs:14) was not measured; they look deletable with refresh.rs but need a caller check in parse.rs and tmx.rs.
- Whether `carry_told_by_listing` (carry.rs:322-423) should move to notify.rs as the plan says or be retired as a repair already done on the owner's machines; retiring it lets the `filings_carried` table (sources/migrations/006:129) go with `carry_filings`.
- The model crate: the old engine modules (5,127 lines) are entangled with the live MarketBase and wire types (model/src/context.rs, wire.rs, symbols_of.rs); the task did not name the model crate, but legacy_import.rs:8-9 and stage-3c §8 (docs/plans/stage-3c-switch.md:179-185) call it earlier-app code. Is it in scope for this removal?
- tests/cases and tests/wire (122,939 lines) are the old Python model's recorded answers; the iOS and Android ports (MOBILE.md:3) are held to them. Are the mobile ports and these fixtures part of 'every trace of the earlier app'?
- store/src/feeds.rs functions with no production caller (StoredNews, news_ids, filings_all, sold_since, position_quantity, balances_count, all_exposures, stored_universes, TOLD_KEPT_DAYS, NOTIFICATIONS_KEPT at L18) may be used by store's own tests or internally; they were not classified old-only to stay conservative.
- The csv_import.rs tests (L908, 1004-1012, 1096-1104, 1137-1145, 1186-1208) and wealthsimple/tests/pull.rs (L201, 255, 506, 718, 754, 789) use `bagholder-import` records as the stand-in for 'a record another source already holds'; which live mapping should replace them in those tests?