# Nothing of the earlier app, every account read one way, the numbers shown agreeing

Clone: `/tmp/claude-1005/-home-md-dev/12a8decb-6b2e-51e6-96e2-82398e1475b1/scratchpad/r571` at `2255a88` (v2.9.0 + 2). Every path below is relative to it; line numbers are the clone's.

## 0. Verdict

One book migration deletes the earlier app's rows outright and forgets every full read; the next sync, due at once, reads every account whole from Wealthsimple; the pull goes back to the shape it had before PR #575, with the closed-account exception gone and two small fixes; the header gains one positive, dated sentence. Two releases: 2.10.0 does all of that and deletes every reader and writer of the earlier app's file; 3.0.0 removes the migration history behind a stated minimum upgrade path. Net, by estimate: about 44,900 lines removed (13,400 hand-written code and tests, 9,800 recorded fixtures, 21,700 migration SQL and schema snapshots), about 290 hand-written lines added, plus one generated schema snapshot per migration (about 1,250 lines) and one 970-line base schema that is moved, not written.

Nothing added is a standing condition. The migration runs once per book and is recorded (`rust/crates/sqlite/src/migrate.rs:216` runs only `&schema.migrations[from as usize..]`). The pull's rule is one rule for every account. The header's sentence is computed from checks the engine already makes.

## 1. What is deleted (release 2.10.0 unless marked 3.0.0)

### 1a. The earlier app's data: one migration, number 37

Everything of source `bagholder-import` (5,867 superseded, 288 removed and 1 live record on the owner's book) and everything hanging off those records, in the order the foreign keys require (no key cascades: no `CASCADE` anywhere in `rust/crates/book/schema/v36.sql` or the migrations; children first, then `source_records`). The SQL is section 2a. Also deleted by it: the two `legacy_key` columns and their indexes (`v36.sql:38`, `:41`, `:600`, `:608`), and orphaned trades carrying no journal entry and no group membership (an orphaned trade exists only "so what the person wrote on it is not lost", `rust/crates/book/src/trades.rs:80-83`; one with nothing written holds nothing).

### 1b. The code that reads or writes the earlier app's file

From the deletion map, whole files unless marked piece:

- server (`rust/crates/server/src`): `carry.rs` (whole, 522 + the `carry_told_by_listing` block 322-423 and its test 624-675, retired rather than moved, section 9), `legacy_import.rs` (994), `legacy_orders.rs` (310), `compare.rs` (612), `engine_inputs.rs:196-200` (`old_store_source`), `figures.rs:33-34, 269-272, 394-398, 887` (`OLD_FILE`, the reattach call, the import on open, `ImportMapping` in `mappings()`), `main.rs` (the mods, `from_old_store`, `adopt_python_database`, `match adopted` collapsing to `open_figures`, the `import-book` and `compare-figures` subcommands at 412-421, the `carry_told_by_listing` call at 181-192), `update.rs:610` (`OLD_FILE` out of `STORE_FILES`), `clear.rs:170-171` and the old-store halves of its tests, `demo.rs:566`, `tests_boundary.rs:114-196`, `tests_failures.rs:185-213`, `tests_misc.rs:722`, `tests_types.rs:16-17, 155-191, 332, 363`, `tests/start_failures.rs:152-176`, `following.rs:448-488`. About 2,950 lines.
- book (`rust/crates/book/src`): `import/mod.rs` (less `wealthsimple_account_type` L370-402, 35 lines, which moves to `bagholder_wealthsimple::read`, its only production reader `rust/crates/wealthsimple/src/read.rs:5`), `import/old.rs` (260), `import/mapping.rs` (less the CSV reader's shared `Rule`/`Qty`/`Cash`/`rule()` L81-175, 95 lines, which move to `rust/crates/broker/src/csv.rs`, their only caller at `csv.rs:22, 451-474`), `lib.rs:15`, `trades.rs` (`Reattached`, `reattach`, `orphan_trade`, `trade_by_legacy_key`, the `legacy_key` parameters of `open_trade` and `add_group`, about 90 lines), `notices.rs:9, 23-36, 78-140` (`carry_notices`), `watched.rs:18-19, 224-255` (`carry_following`), `orders.rs:103-126, 318-325` (`import_orders`, `import_order`, `import_bracket`); tests `tests/import.rs` (500), `tests/notices.rs` (99), `tests/watched.rs:124-143`. About 1,940 lines.
- core: `journal.rs:16-17` (`legacy_key`). The `OrderEvent::Imported` and `BracketEvent::Imported` variants stay (section 9).
- store (`rust/crates/store`): the modules that serve only the old file, deleted in place: `schema.rs` (280), `relabel.rs` (158), `activities.rs` (483), `merge.rs` (296), `csvimport.rs` (1,033), `book.rs` (126), `broker.rs` (85), `admin.rs` (314), `orders/*` (618), `lib.rs` `connect`/`running_tests` (13), `rows.rs` all but `exposures`/`news`/`universes` (203), `tables.rs` all but `get_meta`/`set_meta`/`json_text` (396), `market.rs:20-210, 397-416` (211), `feeds.rs:171-240` (70), `sql/schema_0.sql` and `schema_1.sql` (349); tests `activities.rs`, `durability.rs`, `golden_store.rs`, `guard.rs`, `orders.rs`, `relabel.rs` (1,678); fixtures `tests/golden/store.json`, `mapped.json`, `golden/csv/*` (9,434); `README.md` (21). About 4,290 code + 1,680 tests + 9,780 fixtures. The live modules (feeds, bars, the cache's `market.rs:216-394`, `gens.rs`, the meta functions, `guard_home`) stay where they are: moving them is a rename with no behaviour, listed under not included.
- market (`rust/crates/market/src`): `refresh.rs` (288), the old quote functions of `quotes.rs` (about 350: `yahoo_get` L73 to `peek_quote` L583 as the map lists them; `yahoo_get_result`, `instant_secs_public`, `parse_yahoo_chart`, `percent_encode`, `venue_mic`, `yahoo_forms`, `tmx_quote_symbol`, `yahoo_may_ask`/`yahoo_back_off` and `SourceQuote` stay), `lib.rs:27-36`, `tmx.rs` `fetch_tmx` (called only by `refresh.rs:190`); tests `failures.rs`, `quotes.rs`, `quote_refresh.rs`, `quote_records.rs` (539). About 650 + 540.
- sources: `src/cache.rs:57-80` (`carry_filings`), `tests/cache.rs:437-450`; a cache migration drops `filings_carried` (section 2).
- web: `web/src/lib/generated/book.ts` (27) and the `LegacyNote` import at `web/src/lib/model_api.ts:5`.
- docs: `README.md:189` (the adoption sentence); `docs/plans/remove-the-earlier-app.md` is superseded by this design and `docs/decisions.md:7` and is deleted; `stage-3c-switch.md`, `stage-6-cutover.md`, `old-app-mistakes.md` stay as history with one line saying so.

### 1c. PR #575 and PR #586, reverted on the pull side

What goes (every item is a reason to read one account differently from another, or a way of keeping an import counted):

- `rust/crates/broker/src/lib.rs`: `Moved.imported` (119-123); trait methods `refreshes`, `refresh`, `watches`, `amend` (233-259), `watched_rows`, `names`, `read_beside` (262-279).
- `rust/crates/broker/src/pull.rs`: `Index`'s `imported` flag (66-85) and `index`'s `replaced`/`imported` work (805-856); `stale` and the refresh loop (145-159, 282-309); `watching`/`Watching` and the amend loop in `store_row` (148-150, 166, 681-683, 720-740); `unheld`, the `closed`/`to_read` loop with `closed_read` and the `since` lowering (151-153, 160-165, 184-209, 220-221); `closed_due` (557-597); the `read_beside` loop with `beside`/`beside_linked` (310-342, 358); `store_row`'s `replaces` lookup and `store_superseding` branch and `Report.superseded` (33-34, 699-713); `supersede_copies` (745-786); `replaced_by` (790-802); the import-removal loop in `unlisted` (656-677) with its doc (609-612). What stays: `held` (549-555), `store_row`'s `book.atomically` wrapper, `link_moves(&[RecordId])` (863-879), the unlisted guard (641-655).
- `rust/crates/wealthsimple/src/adapter.rs`: `stated`, `read_from`, `read_again`, `given` (79-88), `Replies::opposing` (264-292), `book_moves`' `imported` (332-346), `lacks_opposing` (349-361), `watches`, `amend`, `watched_rows`, `names`, `read_beside`, `refreshes`, `refresh` (483-586).
- `rust/crates/wealthsimple/src/assemble.rs`: `Needs.opposing` (37-40, 144-148), `Replies::opposing` (76-80), `is_move`, `opposing()`, `days_read` (83-107), the assemble step (256-259).
- `rust/crates/wealthsimple/src/mapping.rs`: `fn leaving` (1007-1077) and its call (289-292); the version moves 13 to 14 (95-98) so every stored Wealthsimple record is derived again without it (`rust/crates/server/src/figures.rs:893`, `rederive_all`; the rule is `docs/architecture.md:98`).
- `rust/crates/wealthsimple/src/record.rs`: `Record.opposing`, `struct Opposing`, `BookMove.imported` and their serialization (69-80, 106-108, 175-188).
- Tests: `rust/crates/wealthsimple/tests/pull.rs:246, 484, 498, 517, 566, 710, 737, 771`, helpers `assert_moved_once` (463), the `Imported` test mapping (197-245), `LateMove.import`; `tests/mapping.rs:617, 680, 700`, helpers `set_row`, `mapped_after`, `legs_in`, and `record_all` in `tests/common/mod.rs:73` where nothing else uses them. `tests/mapping.rs:659` stays less its `opposing` assertion. The RESP fixtures (`tests/pull.rs:401-463`) and `LateMove` (606-700) stay as the fixtures of the replacement test. About 720 (#575) + 340 (#586) + 95 (the import loop, its test, the `Imported` mapping) lines.

The Net worth parts of #586 (`rust/crates/engine/src/scope.rs` `HeldAt`, `build.rs`, `NetWorth.svelte`, the sort, `net_worth.rs:388`) are unrelated to the pull and stay.

### 1d. Release 3.0.0: the migration history squashed

Migrations 001-036 (`rust/crates/book/migrations/*.sql`, 1,574 lines), migration 037's data SQL, and snapshots `schema/v1.sql` to `v36.sql` (19,956 lines), the migration-specific tests for 25, 26 and 30 (`rust/crates/book/tests/schema.rs:114-301`, about 190 lines) and the migration-37 test from 2.10.0. The base becomes the schema as 2.10.0 leaves it (section 2d). About 21,700 lines.

### 1e. Totals (estimates)

| | Hand-written code and tests | Recorded fixtures | Migration SQL and schema snapshots |
|---|---|---|---|
| 2.10.0 | about 13,400 | about 9,780 | 0 |
| 3.0.0 | about 250 | 0 | about 21,530 |
| total removed | about 13,650 | about 9,780 | about 21,530 |

## 2. What is added

### 2a. Migration 037, `rust/crates/book/migrations/037-the-broker-s-records-alone.sql` (about 32 lines of SQL, one line in `rust/crates/book/src/schema.rs:7-44`, and `schema/v37.sql`, about 970 generated lines regenerated with `BAGHOLDER_BLESS`, `tests/schema.rs:26-28`)

Modelled on Clear data's own deletion (`rust/crates/server/src/clear.rs:152-233`), with the narrower link deletion the book readers worked out (a csv record can share a link's `from` side with an import record only through `supersede_copies`, `pull.rs:751-784`; deleting whole links would strip that csv record's supersession, `rust/crates/book/src/links.rs:119-167`):

```sql
CREATE TEMP TABLE gone AS SELECT id FROM source_records WHERE source = 'bagholder-import';
UPDATE trades SET anchor_record = NULL, anchor_leg = NULL, anchor_instrument = NULL,
       orphaned_reason = 'the record it opened on was removed'
 WHERE anchor_record IN (SELECT id FROM temp.gone);
INSERT INTO record_problems (record_id, code, detail)
 SELECT a.record_id, 'adjustment-target-gone', 'the transaction it explains, ' || a.applies_record || '/' || a.applies_leg || ', was removed'
   FROM adjustments a WHERE a.applies_record IN (SELECT id FROM temp.gone) AND a.record_id NOT IN (SELECT id FROM temp.gone);
DELETE FROM transfer_links WHERE out_record IN (SELECT id FROM temp.gone) OR in_record IN (SELECT id FROM temp.gone);
DELETE FROM link_records WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM link_records WHERE link_id NOT IN (SELECT link_id FROM link_records WHERE side = 'from');
DELETE FROM links WHERE id NOT IN (SELECT link_id FROM link_records);
DELETE FROM instrument_sightings WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM adjustment_legs WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM adjustments WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM record_problems WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM record_refs WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM record_revisions WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM transactions WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM record_holds WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM record_orders WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM statement_holds WHERE record_id IN (SELECT id FROM temp.gone);
DELETE FROM source_records WHERE id IN (SELECT id FROM temp.gone);
DROP TABLE temp.gone;
DELETE FROM trades WHERE anchor_record IS NULL
   AND id NOT IN (SELECT trade_id FROM journal WHERE trade_id IS NOT NULL)
   AND id NOT IN (SELECT trade_id FROM trade_group_members);
DROP INDEX trades_legacy_key;        ALTER TABLE trades DROP COLUMN legacy_key;
DROP INDEX trade_groups_legacy_key;  ALTER TABLE trade_groups DROP COLUMN legacy_key;
UPDATE activity_reads SET complete = 0;
```

Why each part is safe: every table above references `source_records(id)` (`v36.sql:122, 259, 312, 441, 451, 456, 463, 467, 528, 614, 640, 642`) and `adjustment_legs` references `adjustments` (`v36.sql:116`), so children go first; the migration runner checks every reference after the batch and fails the migration, leaving the file at version 36, on any dangling row (`migrate.rs:199-213, 230-233`); a snapshot is taken first (`migrate.rs:178-180`). `complete = 0` is what makes the next read of every account whole: the last full read is the newest row with `complete = 1` (`rust/crates/book/src/statements.rs:535-538`), and an account with none is read from nothing (`pull.rs:190-193, :215`). The book's own change triggers fire on these deletes (`v36.sql:894-897, 941-943` and the others), so the engine's next build sees the change; `names_due` and `successions_due` rows settle at the first write (`rust/crates/book/src/lib.rs:143-158`, `links.rs:110`).

The person's data cannot be touched by it: a superseded or removed record anchors no trade, because supersede moves or orphans every trade first (`links.rs:58-74` before `:94`) and removal orphans them first (`rust/crates/book/src/records.rs:478-480` before `:487`), and nothing anchors a trade on a record with no transactions (`trades.rs:57-61`, `records.rs:502-505`). The one live import record is the leaving side of a move out of the managed RESP, a `TransferOut` (direction −1, `records.rs:80-91`), which opens nothing, so no trade sits on it. Journal, tags, groups, orders, brackets, notifications, watchlist and tiles name no record (`v36.sql:290-304, 590-612, 380-412, 350-360, 584-589, 646-649, 580-583`).

### 2b. The pull (`rust/crates/broker/src/pull.rs`), net a deletion, with two fixes

1. The read loop collapses to: for every account the broker states, open or closed, read from its last full read, lowered to the first day of any row of it not yet final, whole where there is none (the code already there at `pull.rs:215-219` once the `closed`/`to_read` split and `since` go). 0 lines added.
2. A row whose record cannot be put together (`store_row`'s `Err` branch, `pull.rs:689-696`) no longer leaves its account's read complete: the account is kept out of `read_accounts` and `spans` and its read noted incomplete, exactly as a row without an id does today (`pull.rs:229-234`), so the row is listed again by the next read and nothing of the account is removed on this one. About 8 lines. Prevents the one path the readers found that loses a listed row of a complete read.
3. `keys_of` holds every stated account, not only the open ones (`pull.rs:365`, the `filter(|a| a.open)` goes; `margin` keeps `a.open`). A closed account's cash and holdings are then stated and checked like an open one's, which is what makes the managed RESP's leaving leg checkable (section 5). −1 line; see risk 1 for the fallback.
4. `rust/crates/server/src/broker_reads.rs:289-290`: a pull is also due while an account of the connection has no complete activity read. About 5 lines. `pull_due(None, …)` is already true for a connection never pulled (`broker_reads.rs:85-91`), but `last_daily` falls back to the last `accounts` read (`:281, :289`), which other tables reference (`v36.sql:19, 51, 57, 72`), so the book's own state has to say it: this rule is what makes the first start after the update sync at once instead of waiting for the day's window.

### 2c. The header's positive sentence (about 25 server lines, 2 page lines, 3 SPEC lines)

After each pull's figures are applied (`broker_reads.rs:409` `f.pulled`), the server reads `figures.checks` (`rust/crates/engine/src/equity.rs:130-214`): when no account has a difference that is not pending, and the sales beyond what was held are none (`status.rs:225-236`), it keeps the day every account's holdings were compared as of (the earliest `held_as_of`, `equity.rs:181`) as the book setting `broker.agreed_as_of`, beside the existing `statements.said` (`broker_reads.rs:416`, `status.rs:400`, reloaded at start `main.rs:166-169`). `Status` (`status.rs:19-52`) gains `agreed_as_of: String`; `web/src/lib/generated/status.ts` is regenerated; `web/src/App.svelte:314` reads `'Synced ' + relTime(s.lastSync) + ' · Agrees with Wealthsimple as of ' + day` when the day is set. A confirmed difference already takes the line (`App.svelte:312`, `s.error`), and the day waits until a later check agrees, so nothing shows for a moment and goes (`SPEC.md` pages-8). `SPEC.md` §4 (pages-16) gains the sentence and names the model: YNAB's reconciled state and Quicken's `R`.

### 2d. Release 3.0.0: the base version (about 50 lines in `rust/crates/sqlite/src/migrate.rs`, about 30 in `rust/crates/book/tests/schema.rs`, 1 line each in `docs/architecture.md:103` and `CLAUDE.md` Releases)

`Schema` gains `base: u32` (the version the first listed migration follows); `check_numbering` starts at `base + 1` (`migrate.rs:134-142`); `run_pending` slices from `from - base` (`:216`); `identify` refuses a file at `0 < from < base` with `MigrateError::Older { file, minimum: "2.10.0" }` beside `Newer` (`:149-168`, `:57-70`); a fresh file runs the base first. The base SQL is `schema/v37.sql`'s text (every `CREATE` in `sqlite_master`, which is what `schema_text` prints, `migrate.rs:317-331`), moved to `migrations/037-the-book.sql`, plus `PRAGMA user_version = 37` and the `schema_migrations` row, which the runner already writes (`:218-237`). `MIGRATIONS` becomes `[Migration; 1]` until the next migration. This is the squash Django documents ("squash, keeping the old files, commit and release, wait until all systems are upgraded with the new release… and then remove the old files, commit and do a second release"), the baseline Rails keeps ("the authoritative source for rebuilding that database… makes it possible to delete or prune old migration files") and GitLab's squash at each required stop.

### 2e. Smaller edits

- `rust/crates/sources/migrations/009-filings-carried-dropped.sql` (1 line) and its line in `MIGRATIONS` (`rust/crates/sources/src/cache.rs:25`), with `schema/v9.sql` regenerated (about 284 lines). The table is read only by `carry_told_by_listing` and `carry_filings` and cleared by Clear data (`clear.rs:84`), all gone.
- `docs/decisions.md:7-8`: the "Held by" lines name the tests of section 7 instead of the identifier scan; `docs/plans/stage-3b-wealthsimple.md:63` loses "nothing for a closed account with no new activity"; `SPEC.md` market-data-11 says "every account, open or closed".
- `figures.rs:886-887`: `mappings()` is four.
- `tests/csv_import.rs:908, 1004-1012, 1096-1104, 1137-1145, 1186-1208`: the stand-in for "a record another source already holds" becomes a person's entered record (`bagholder_book::person::PersonMapping`), about 60 lines rewritten; the store and market tests build their database with `bagholder_sources::cache::MIGRATIONS` instead of `schema::init_schema` (`carry.rs:636-638` shows the form), a few lines per file.

### 2f. Totals (estimates)

| | Hand-written | Generated or moved |
|---|---|---|
| 2.10.0 | migration 32, pull 13, due 5, header 30, tests about 120 (section 7), docs 8: about 210 | `v37.sql` 970, `v9.sql` 284, `status.ts` 1 |
| 3.0.0 | migrate.rs 50, tests 30, docs 2: about 85 | the base, 970, moved from `v37.sql` |
| total added | about 295 | about 2,225 |

## 3. The data step: what happens to your book

Written for you, in order. Your book today: 6,156 rows of the earlier app's import, of which 5,867 are superseded (Wealthsimple's own row of each is what counts), 288 removed (card rows Wealthsimple no longer lists), and 1 live: the leaving side of the cash move on 2025-06-16 out of the managed RESP, since closed, into the self-directed RESP. Its arriving side is Wealthsimple's own row. Wealthsimple's feed now lists the leaving row, but your book never stored it, because a closed account was read whole once and never again (`pull.rs:184-209`).

1. **You install 2.10.0.** The updater snapshots the book before the migration (`docs/architecture.md:103`; `migrate.rs:178-180`, kept under `snapshots/`). The migration runs once, in one transaction: the 6,156 import rows and everything derived from them are deleted; the two `legacy_key` columns go; every account's "read in full" mark is cleared. If anything in it fails, the book stays at version 36 and the page says the migration failed (`docs/architecture.md:216`); nothing is half done.
2. **The app starts.** The figures are built from the migrated book. For the minute that follows, the leaving side of the 2025-06-16 move is not in the book at all: the cash that left the managed RESP is still counted there as well as in the self-directed RESP. The header does not show a difference for this: a difference is said only once confirmed (`SPEC.md` pages-17), and the managed RESP has no statement yet to confirm one.
3. **The first sync starts at once**, because every account now has no full read (section 2b, item 4). The header reads the sync's steps: the accounts, then each account's activity by name, `n of N` (`pull.rs:211`), the closed managed RESP among them read whole like the others, then the balances and the holdings. On your book the feed alone is about 60 requests of 100 rows, 150 ms apart (`rust/crates/wealthsimple/src/client.rs:18, 100-105, 241`), under a minute; a row whose stored record already holds it as it is costs nothing more (`pull.rs:250-256`, `held`). The 2025-06-16 leaving row is listed in the managed RESP's feed and, having no record, is stored as Wealthsimple's own record, and its leg is booked from that row (`mapping.rs`, the `SOURCE` side of an `ASSET_MOVEMENT`), in the managed RESP, on 2025-06-16. Every Wealthsimple record is also derived again under mapping version 14, so any leg the 2.8/2.9 builds inferred from an arrival (`fn leaving`) is gone and only what Wealthsimple states remains.
4. **The sync ends.** The managed RESP's cash and holdings, asked like any account's, are stated (zero, for a closed account), and the engine compares every account: cash against what Wealthsimple stated at this read, holdings against what it stated as of yesterday (`equity.rs:147-209`; `SPEC.md` pages-16). The header reads `Synced just now · Agrees with Wealthsimple as of Oct 20`. If instead the managed RESP's feed had not listed the leaving row, the header would read `CAD cash in the managed RESP: Wealthsimple states $0.00, the book holds $4,321.10.` (the form at `status.rs:216`), and stay until a read shows it resolved.
5. **Every later sync** reads each account from its last full read, open or closed, the same way (section 4). Nothing of the earlier app runs, at start or ever: the file `bagholder.db` is never opened (the test of section 7, item 8 holds that), and you may delete it when you like.
6. **You install 3.0.0** (after both your machines have run 2.10.0). Nothing happens to the book: it is at version 37 and the base is version 37. A book never opened by 2.10.0 is refused with `This book was last opened by an earlier version; open it with 2.10.0 first`, and the page says so instead of loading.

## 4. The read and reconciliation rule

**One rule for every account the broker states** (`docs/decisions.md:8`, 2026-10-21): its activity is read from its last full read, lowered to the first day of any row of it stored before it was final, and whole where it has no full read (`pull.rs:215-219`, the only lowering left); its cash and buying power are asked now and its holdings as of yesterday (`pull.rs:369-371, 388-407`); its monthly statements are read only while its book cash disagrees with its stated cash (`rust/crates/broker/src/statements.rs:153-157, 565-580`), walking back to a month that reconciles and booking the movements the feed leaves out (`:656-714`). A closed account is on no separate path: Wealthsimple's closing is the broker's statement that it holds nothing, and the check says so if the book disagrees.

**A full read is the one-time step after the migration**, never a standing one. What a whole read removes is held by the guard that exists: nothing of an account is removed when a final row dated before its last full read is gone, or when more than three final rows are (`pull.rs:641-655`, `FEW` at `:547`); a read with no earlier full read (every account, once, after the migration) is held by the count alone and reported suspect past it (`broker_reads.rs:432-445`). A row no longer listed inside the span read is removed as today; an import is no longer a case.

**A row that fails to assemble** leaves the read incomplete (section 2b, item 2), is said as a failed part of the pull (`SPEC.md` pages-11), and is listed again by the next read from the last complete one.

**What this rule cannot do, said plainly.** Whether Wealthsimple applies the feed's `startDate` to a row's `occurredAt` or to when it was posted is not in the repository (`rust/crates/wealthsimple/graphql/FetchActivityFeedItems.graphql:2`; no recorded reply carries a `startDate`; `docs/plans/stage-3b-wealthsimple.md:63` assumes the former). Under the code's reading, a row Wealthsimple adds later with an older date, in any account, is not asked for by an incremental read. It is caught where the vendors' guidance puts it: by the statement of balances (Plaid: a removed transaction "will no longer appear in /transactions/get", so a date window is not enough; Akahu: "expect to handle addition, modification, and deletion"; Teller: match on stable ids with an overlapping window). Here that is the broker check: the cash difference is confirmed by the next statement and said; the units difference is said as of the day read (`equity.rs:159-172, 179-209`; `status.rs:201-237`). The person's way to read everything again is the one the app has: `Clear data` → `Wealthsimple records`, "so the next pull reads everything again", journal kept (`SPEC.md` pages-24, pages-25; `rust/crates/server/src/clear.rs:579-600`). A re-read triggered by a reason is not added (the owner, of such a rule: "That is not a fix").

## 5. How you are assured the numbers are right, on the page

What exists: the engine compares, per account, the book's cash per currency with the cash Wealthsimple stated at the last full read, net of its holds, and the book's units with Wealthsimple's units as of the day that read covers; a difference is said in the header, exactly, once confirmed (`equity.rs:130-214`; `status.rs:201-237`; `SPEC.md` pages-14 to pages-20), and so is a month whose statement does not reconcile (`status.rs:405-437`) and every sale beyond what was held. What is missing is the positive half that YNAB and Quicken show: YNAB marks reconciled transactions with a lock and calls it "assurance that the numbers in your plan are correct"; Quicken marks each reconciled row `R`; Beancount keeps a dated balance assertion in the ledger itself and "will report an error" on a different balance. The app's equivalent is one dated sentence, section 2c: `Agrees with Wealthsimple as of <day>`, true only when every account's cash and holdings were compared and none differs, and replaced by the differences, named with figures, whenever one is confirmed. It costs no new read: the checks are made with every change to the figures (`docs/architecture.md` §13), and the sentence is written when a pull's figures land and read with the header.

For the one case that started this: the managed RESP's cash and holdings are now stated and compared like any account's, so a leaving side missing from the book is a difference the header says, with the account's name and the two figures, and a book that agrees says so with a date.

## 6. Releases and versions

- **2.10.0 (MINOR):** sections 1a-1c, 2a-2c, 2e. Something new a user can see (the header's sentence); no install breaks: every book is migrated with a snapshot. Bumped in `rust/crates/server/src/app.rs:14` and `rust/Cargo.toml` in the last change (`CLAUDE.md:91`).
- **3.0.0 (MAJOR):** sections 1d and 2d. A book last opened by 2.9.x or earlier is refused until 2.10.0 has opened it, which is a break for an install that skips 2.10.0, so the rule at `CLAUDE.md:91` makes it MAJOR. The stated minimum upgrade path, written in the Releases section of `CLAUDE.md` and in 3.0.0's release notes: "A book from before 2.10.0 is opened with 2.10.0 first." About two machines download a release; both run 2.9.0 today; the cost is nil, as Home Assistant's six-release migration window and GitLab's required stops cost nothing to an installation that keeps up.

## 7. Tests (each named against the rule it holds)

2.10.0:
1. `rust/crates/wealthsimple/tests/pull.rs` `every_account_the_broker_states_is_read_from_its_last_full_read_closed_or_not`: on the RESP fixtures (`:401-463`) with `late_cash_move` and `late_units_move` (`:621-664`), the first pull reads the closed account whole; the second, from the first's day, stores the leaving row dated between them and books the leg from that row alone (`assert_left_from_its_own_row` less its import line); the third reads nothing new. Holds `docs/decisions.md:8`.
2. `pull.rs` `a_row_whose_record_cannot_be_put_together_leaves_its_account_s_read_incomplete_and_is_listed_again`: one positions reply made unreadable; the account's read is incomplete, nothing of it removed, the failure said; the next pull lists the row and stores it once the reply reads. Holds section 2b, item 2.
3. `pull.rs:101` `a_pull_with_nothing_new_asks_only_the_accounts_their_activity_and_their_cash`, updated: a closed account's cash and holdings are asked like an open one's, and nothing else is.
4. `rust/crates/book/tests/schema.rs` `migration_37_leaves_only_the_broker_s_and_the_person_s_records_and_forgets_every_full_read`: a version-36 file holding records of three sources, a csv record sharing a link's `from` side with an import, a trade with a note on a Wealthsimple record, an orphaned trade with a note and one without, and a complete read; after open, the import's rows and their derived rows are gone, the csv record's supersession stands, the note's trade is still anchored, the orphan with a note is kept and the one without is gone, both `legacy_key` columns are gone, and `activity_read_at` is `None` for every account. Temporary: it goes with the migration in 3.0.0. (It names the source label once, to prove the migration, and is not a scan.)
5. `rust/crates/server/src/broker_reads.rs` `a_pull_is_due_while_an_account_has_no_full_read`. Holds section 2b, item 4.
6. `rust/crates/server/src/status.rs` `the_header_says_the_day_every_account_agreed_with_the_broker_and_keeps_it_while_a_check_is_pending` and `a_confirmed_difference_takes_the_line_and_the_agreed_day_waits_for_the_next_agreement`. Hold section 2c and `SPEC.md` pages-8, pages-16, pages-17.
7. `web/e2e/header.spec.ts` `the header says the book agrees with Wealthsimple as of a day`: on the made-up book, after the sync, the line carries the sentence and the day (`CLAUDE.md:73`: a behaviour in `SPEC.md` is not done until a browser test drives it).
8. `rust/crates/server/tests/start_failures.rs` `a_start_opens_only_the_book_and_the_cache`: a start on an empty data folder beside a file of another name leaves exactly the book, the cache and their journals in the folder, and that file's bytes and modification time unchanged. This is what holds `docs/decisions.md:7` in place of the identifier scan the owner declined: it states what the app does, not names it must not use.
9. Existing tests that keep holding the guard and the rest of the pull: `pull.rs:351, :370` (the unlisted guard), `:305` (a row the adapter cannot read), `:332` (a row with no id), `tests/mapping.rs:659` (a two-sided move books one leg in each account), `rust/crates/book/tests/links.rs:82-96` (an orphaned trade keeps its note), `clear.rs:579-600` (Clear data keeps the journal and the next pull reads in full).

3.0.0:
10. `tests/schema.rs` `each_version_is_exactly_the_schema_committed_for_it` and `every_earlier_version_reaches_the_latest_with_its_rows` run from the base; `the_header_and_the_record_of_migrations_agree` accepts numbers from the base; new `a_book_older_than_the_base_is_refused_naming_the_release_to_open_it_with`; `an_earlier_apps_database_is_not_opened_as_a_book` (`:66-74`) keeps its body under the name `a_file_that_is_not_a_book_is_not_opened_as_one`.

## 8. Order

1. PR 1, the pull: section 1c and 2b (the loop collapses, the failure fix, closed accounts asked, mapping version 14), with tests 1-3 and 9. No data changes; import rows are untouched until PR 2.
2. PR 2, the earlier app gone: sections 1a, 1b, 2a, 2e, with tests 4, 5, 8. The migration and the code deletion land together, as `docs/decisions.md:7` asks ("every reader and writer of the earlier app's format is deleted in the same release").
3. PR 3, the header: section 2c, with tests 6, 7. Then the bump to 2.10.0 and the release (`CLAUDE.md:91`).
4. Both machines take 2.10.0 (the updater offers it; the header's `Update to v2.10.0`). The owner's one check, once: the header reads the agreement sentence, or the differences it names.
5. PR 4, the squash, its own gate (a change to how stored data is opened, `CLAUDE.md:30`): sections 1d, 2d, test 10. Bump to 3.0.0 and release, after step 4.

## 9. Risks

1. **Wealthsimple may refuse a closed account's balances or positions.** `FetchAccountsWithBalance` takes any ids and nothing restricts it to open accounts (`rust/crates/wealthsimple/graphql/FetchAccountsWithBalance.graphql:2-3`), but no recorded reply shows a closed account's balance (`tests/replies/` holds 20 `open` and no `closed`). The acceptance criterion of PR 1 is one recorded reply, figures made up, for a closed id. If it refuses: closed accounts are asked in a call of their own (about 6 lines in `store_balances`, `pull.rs:1037-1057`, and the holdings loop) so the refusal is said as its own part (`SPEC.md` pages-11) and the open accounts' statements still store; the closed account's check then rests on its cash alone, which `statements.rs:921-941` already takes as zero for a closed account with no statement rows.
2. **The minute between the update's restart and the first sync's end** shows figures without the leaving side of the 2025-06-16 move (section 3, step 2). Said here so it is expected; the header shows the sync's steps throughout.
3. **The whole read after the migration removes nothing before its own day's guard**: `read_before` is unset, so only the count guard holds (at most three final rows per account, else suspect and nothing removed, `pull.rs:645-652`). Any row removed is listed in the pull's log and report (`broker_reads.rs:432-445`).
4. **`ALTER TABLE … DROP COLUMN`** needs SQLite 3.35 or later and a column no index or constraint names: the indexes are dropped first; if the bundled SQLite refuses, the table is built again by the procedure the runner already documents (`migrate.rs:181-190`), about 12 more lines of SQL.
5. **`carry_told_by_listing` is retired, not moved.** It landed on 2026-10-16 (`777161e`), before v2.7.0 (2026-10-19), so every book opened since has run it (`main.rs:181-192`, once per book by `book.setting(TOLD_BY_LISTING)`, `carry.rs:380`); a book that never did sets its marks without telling (`main.rs:182-183`), so nothing is told twice. The minimum path for it is the same one: 2.10.0 after 2.7.0 or later.
6. **`reattach_carried_notes` is retired outright**: it recorded itself done at the first build after 2026-09-26 (`legacy_import.rs:421, 470`), and every book is past that.
7. **The `Imported` order and bracket events stay** (`rust/crates/core/src/order.rs:229-240`, `bracket.rs:340-350`): they are the first event of every order and bracket carried in, and `OrderFold::start` and the bracket fold read them. Deleting the variants would make those logs unreadable; a migration rewriting them would invent history. Nothing writes them once `legacy_orders.rs` is gone; they are the vocabulary of logs the book keeps.
8. **The squash refuses a book older than version 37** (section 6). An install that skips 2.10.0 sees the refusal in the header and installs 2.10.0 by hand from the releases page.
9. **Open questions that stay open and change nothing here:** which field `startDate` filters on; whether the default `accountScope: OWN` hides rows of a family RESP; whether `FetchAllAccounts`' default filter lists every closed account (the managed RESP was listed, `pull.rs:125`). Each is a probe on the owner's account, none a reason for code.

## 10. Not included

- Moving the store crate's live cache SQL (about 2,405 lines: `feeds.rs`, `bars.rs`, `market.rs:216-394`, `gens.rs`, the meta functions, `guard_home`) into `bagholder-sources` and removing the crate: a rename with no behaviour and no net lines; done when that SQL is next touched.
- The model crate's old engine (`fifo`, `base`, `nav`, `filters` and the rest, 5,109 lines), `tests/cases` and `tests/wire` (122,939 lines) and the phone ports: the live wire types name them (`rust/crates/model/src/wire.rs:809, 837, 855`; `symbols_of.rs:11, 13`; `context.rs:62`), and the phone apps are on hold and compute the old model by `CLAUDE.md:25`. Whether the phone apps go is a decision of its own, one line to the owner.
- A re-read of an account triggered by a confirmed difference (`docs/plans/stage-3b-wealthsimple.md:63` states it; nothing builds it): declined by the owner's words on reason-driven re-reads.
- The book's account status following the broker's on later pulls (`pull.rs:121-128` sets it only when adding; the five-minute balances read keys off it, `pull.rs:909-911`): a one-line proposal, not part of this.
- A page listing orphaned notes (`trades.rs:445-457` has no production reader once `reattach_carried_notes` goes; no page lists them today either).
- The statements, the two LIRA withdrawals, and the Net worth changes of #586.

## 11. Sources leaned on

- Retiring migration code: Django, `docs.djangoproject.com/en/5.2/topics/migrations/` (squash, release, "wait until all systems are upgraded… then remove the old files, commit and do a second release"); Rails, `guides.rubyonrails.org/active_record_migrations.html` ("makes it possible to delete or prune old migration files"); GitLab, `docs.gitlab.com/development/database/migration_squashing/` and `/update/upgrade_paths/` (required stops, squash two stops back); Home Assistant, `home-assistant.io/more-info/unsupported/home_assistant_core_version/` (migration code kept six releases).
- Why a date-window read is not enough and what catches it: Plaid, `plaid.com/docs/api/products/transactions/` and `/docs/transactions/`; Teller, `teller.io/docs/api/account/transactions`; Akahu, `developers.akahu.nz/docs/accessing-transactional-data`; Stripe, `docs.stripe.com/financial-connections/transactions`.
- The positive reconciled state: YNAB, `support.ynab.com/en_us/reconciling-accounts-a-guide-BJFE3fHys` and `/getting-started-with-reconciling-accounts-an-overview-Sy3JWx4Js`; Quicken, `info.quicken.com/win/find-out-if-a-transaction-has-cleared`; Beancount, `beancount.github.io/docs/beancount_language_syntax/` (balance assertions); the accounting term, OpenStax, Principles of Financial Accounting 8.6.
- The book rebuilt from the source of record: Fowler, `martinfowler.com/eaaDev/EventSourcing.html` ("rebuild it by re-running the events"); Kleppmann, `martin.kleppmann.com/2015/03/04/turning-the-database-inside-out.html` ("you could rebuild it from the log at any time").

lines_removed 44900 lines_added 2520

key claims:
- Clearing every activity read's `complete` flag makes the next read of every account whole, and nothing else needs to change for it: the last full read is the newest row with `complete = 1` (rust/crates/book/src/statements.rs:535-538), and an account with none is read from nothing (rust/crates/broker/src/pull.rs:190-193, :215).
- Deleting every superseded, removed and live `bagholder-import` record detaches no journal entry: supersede moves or orphans every anchored trade before it sets the state (rust/crates/book/src/links.rs:58-74 before :94), removal orphans them first (rust/crates/book/src/records.rs:478-480 before :487), nothing anchors a trade on a record with no transactions (rust/crates/book/src/trades.rs:57-61; records.rs:502-505), and the one live record is a TransferOut leg (direction -1, records.rs:80-91), which opens nothing.
- The engine already compares each account's cash at its last full read and its units as of the day read and says a confirmed difference in the header; only the positive dated statement is missing (rust/crates/engine/src/equity.rs:130-214; rust/crates/server/src/status.rs:201-237; web/src/App.svelte:312-314 shows `Synced <time>` and no agreement).
- The first sync after the migration needs a general due rule, not a deletion of read rows: `pull_due(None, …)` is true but `last_daily` falls back to the last `accounts` read (rust/crates/server/src/broker_reads.rs:85-91, :281, :289), and `broker_reads` rows are referenced by other tables (rust/crates/book/schema/v36.sql:19, :51, :57, :72), so they cannot be deleted.
- Removing the 36 earlier migrations and their snapshots needs a base version the runner does not have: numbering must start at 1 and migrations are sliced from `from` (rust/crates/sqlite/src/migrate.rs:134-142, :216), `identify` has no too-old refusal (:149-168), and nothing reads schema/vNN.sql at runtime; the snapshot text is every `CREATE` in `sqlite_master` (:317-331), so v37.sql can serve as the base SQL.
- Asking a closed account's cash is one line (the `filter(|a| a.open)` at rust/crates/broker/src/pull.rs:365) and the document allows it (rust/crates/wealthsimple/graphql/FetchAccountsWithBalance.graphql:2-3 takes any ids), but no recorded reply shows a closed account's balance (tests/replies/ holds 20 `open` and no `closed`), so one recorded reply for a closed id is the acceptance criterion.