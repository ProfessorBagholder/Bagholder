# The book's records and what hangs off them

Every path below is under R = `/tmp/claude-1005/-home-md-dev/12a8decb-6b2e-51e6-96e2-82398e1475b1/scratchpad/r571` (origin/master, 2255a88). Line numbers are from that clone.

## 1. The record lifecycle

**States.** `source_records.state` is constrained to three values: `state TEXT NOT NULL CHECK (state IN ('live', 'superseded', 'removed'))` (R/rust/crates/book/schema/v36.sql:505). A record is inserted live: `INSERT INTO source_records(... state ...) VALUES (?1, ?2, ?3, ?4, 'live', ...)` (R/rust/crates/book/src/records.rs:160-161). State changes go through one function, `set_state`: `UPDATE source_records SET state = ?, state_changed_at = ? WHERE id = ?` (records.rs:509-515). Two callers: `supersede_counting` sets `RecordState::Superseded` (R/rust/crates/book/src/links.rs:94) and `mark_removed` sets `RecordState::Removed` (records.rs:487). There is no transition back: `supersede_counting` refuses any record on either side that is not live, `if self.record(*r)?.state != RecordState::Live { return Err(... "is not live, so it cannot be linked") }` (links.rs:39-43); `store_superseding` refuses a non-live record unless it is already superseded by this same record (records.rs:136-142); `mark_removed` on a removed record returns `Changes::default()` (records.rs:474-476). A payload arriving again for a superseded or removed record appends a revision but derives nothing: transactions are derived only when `self.record(record)?.state == RecordState::Live` (records.rs:180-184), and `rederive` selects `WHERE source = ? AND state = 'live'` (records.rs:450).

**What a supersede does.** `supersede_counting(from, to, reason, at)` (links.rs:33-113), in one transaction (links.rs:34):
- Collects the `to` records' transactions and sorts them earliest first, "by instant (an unknown instant last), record, leg" (links.rs:47-54).
- For each `from` record, reads the trades anchored on it (`trades_anchored_on`: `SELECT id, anchor_leg, anchor_instrument FROM trades WHERE anchor_record = ?`, records.rs:761-776). For each trade: if the anchor transaction is no longer among that record's transactions, `orphan(trade, "the record it opened on had no such transaction")` (links.rs:59-62); else it looks through the `to` transactions for one that is the `same_opening` (same account, instrument, direction and kind, and the same effect where both state one, records.rs:93-110) with no trade on it yet, and calls `move_anchor` (links.rs:64-69); if none matches, `orphan(trade, "the record it opened on was replaced by records with no opening like it")` (links.rs:71-73).
- `move_anchor` is `UPDATE trades SET anchor_record = ?, anchor_leg = ?, anchor_instrument = ?, orphaned_reason = NULL WHERE id = ?` (R/rust/crates/book/src/trades.rs:218-225). `orphan` is `UPDATE trades SET anchor_record = NULL, anchor_leg = NULL, anchor_instrument = NULL, orphaned_reason = ? WHERE id = ?` (trades.rs:97-103). The trade row, its id and its journal stay either way: "A trade with no counterpart is orphaned with the reason, its journal kept" (links.rs:8-9), held by the test `a_trade_with_no_counterpart_is_orphaned_and_its_note_kept` (R/rust/crates/book/tests/links.rs:82-96).
- Adjustments held by other records on the `from` record's transactions are moved the same way or get an `adjustment-target-gone` problem (links.rs:77-91; R/rust/crates/book/src/facts.rs:526-536).
- Then `self.clear_derived(*r)` and `self.set_state(*r, RecordState::Superseded, at)` (links.rs:92-94), then one `links` row (`kind = 'supersedes'`) and one `link_records` row per record per side (links.rs:96-108), then `settle_successions` (links.rs:110).

**Are a superseded or removed record's transactions kept?** No, they are deleted. `clear_derived` runs `DELETE FROM {table} WHERE record_id = ?` for `transactions`, `record_problems`, `instrument_sightings` and then `clear_adjustments` (records.rs:500-507; facts.rs:479-483 deletes `adjustment_legs` then `adjustments`). Its doc says what stays: "Its revisions stay" (records.rs:501). Not touched by `clear_derived`, so still present on a superseded or removed record: `record_revisions`, `record_refs` (only inserted, records.rs:177-179), `record_holds` and `record_orders` (replaced only by a new derive, records.rs:192 and 282, and deleted only by Clear data, R/rust/crates/book/src/clear.rs:193-194), and `link_records`. `mark_removed` (records.rs:471-498) orphans anchored trades with "the source removed the record it opened on" (records.rs:478-480), flags adjustments (records.rs:481-485), `clear_derived`, sets Removed, and adds a `superseding-record-removed` problem if the record had replaced others (records.rs:488-493). Problems on removed records are still shown, those on superseded records are not: `WHERE r.state IN ('live', 'removed')` (records.rs:659-663). `transactions()` therefore reads "live records only have any" (records.rs:579-583).

## 2. Every table that references a record, and the deleting SQL

From R/rust/crates/book/schema/v36.sql. Columns with `REFERENCES source_records(id)`:
- `adjustments.record_id` (v36.sql:122); its `applies_record`/`applies_leg` are plain TEXT with no FK (v36.sql:124-125).
- `adjustment_legs (record_id, leg)` references `adjustments(record_id, leg)` (v36.sql:116), so it hangs on records transitively.
- `instrument_sightings.record_id` (v36.sql:259).
- `link_records.record_id` (v36.sql:312), and `link_records.link_id REFERENCES links(id)` (v36.sql:311).
- `record_holds.record_id` (primary key, v36.sql:441).
- `record_orders.record_id` (v36.sql:451); its `order_id` is plain TEXT, no FK to `orders` (v36.sql:452).
- `record_problems.record_id` (v36.sql:456).
- `record_refs.record_id` (v36.sql:463).
- `record_revisions.record_id` (v36.sql:467).
- `statement_holds.record_id` (v36.sql:528); the other statement tables (`statements`, `statement_cash`, `statement_units`, `account_values`, `account_days`, `monthly_statements`) reference accounts and `broker_reads`, not records (v36.sql:548-574, 520-525, 537-547, 66-79, 45-53, 336-343).
- `transactions.record_id` (v36.sql:614).
- `transfer_links.out_record` and `in_record` (v36.sql:640, 642).

Columns that hold a record id without a foreign key: `trades.anchor_record` is `anchor_record TEXT,` with no REFERENCES (v36.sql:605; unique index `trades_anchor` at v36.sql:39); `adjustments.applies_record` (v36.sql:124); `ledger_changes.key` where `kind = 'record'` (v36.sql:305-309, written by triggers such as v36.sql:894-897 and 941-943).

**Cascade.** No foreign key in the schema cascades: a grep of v36.sql and every migration for `CASCADE`, `ON DELETE`, `ON UPDATE` returns nothing. The book opens with `conn.pragma_update(None, "foreign_keys", "ON")` (R/rust/crates/book/src/lib.rs:103), so a parent row with children cannot be deleted; children go first. (The `sqlite3` shell has foreign keys off by default; the order below is right either way.)

**How Clear data deletes records** (R/rust/crates/book/src/clear.rs:152-233). It builds a temp table of the chosen ids: `DROP TABLE IF EXISTS temp.cleared; CREATE TEMP TABLE cleared AS SELECT id FROM source_records WHERE {which};` (clear.rs:163). It orphans trades first: `UPDATE trades SET anchor_record = NULL, anchor_leg = NULL, anchor_instrument = NULL, orphaned_reason = ? WHERE anchor_record IN (SELECT id FROM temp.cleared)` with reason "the record it opened on was cleared" (clear.rs:165-169). It flags adjustments other records hold on cleared transactions via `adjustment_target_gone(... "was cleared")` (clear.rs:170-179). Then one batch (clear.rs:180-198):
```
DELETE FROM transfer_links WHERE out_record IN (SELECT id FROM temp.cleared) OR in_record IN (SELECT id FROM temp.cleared);
CREATE TEMP TABLE cleared_links AS SELECT DISTINCT link_id FROM link_records WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM link_records WHERE link_id IN (SELECT link_id FROM temp.cleared_links);
DELETE FROM links WHERE id IN (SELECT link_id FROM temp.cleared_links);
DROP TABLE temp.cleared_links;
DELETE FROM instrument_sightings WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM adjustment_legs WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM adjustments WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM record_problems WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM record_refs WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM record_revisions WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM transactions WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM record_holds WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM record_orders WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM statement_holds WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM source_records WHERE id IN (SELECT id FROM temp.cleared);
DROP TABLE temp.cleared;
```
After that it runs `clear_unnamed` (accounts, instruments, issuers, connections nothing names, clear.rs:236-298) and `settle_successions` (clear.rs:228-230). Clear data selects by source: `source NOT IN ('person', 'csv')` for the broker's, `source IN ('person', 'csv')` for the person's (clear.rs:142, 156-161), so today a Clear of "Broker" already deletes `bagholder-import` records with Wealthsimple's.

**The SQL for source 'bagholder-import', in the order it must run.** Modelled on clear.rs; the two differences from clear.rs are marked.
```sql
PRAGMA foreign_keys = ON;
BEGIN IMMEDIATE;
CREATE TEMP TABLE cleared AS SELECT id FROM source_records WHERE source = 'bagholder-import';

-- 1. trades first (no FK, but clear.rs:165-169 does it before the records go): orphaned, journal kept
UPDATE trades
   SET anchor_record = NULL, anchor_leg = NULL, anchor_instrument = NULL,
       orphaned_reason = 'the record it opened on was cleared'
 WHERE anchor_record IN (SELECT id FROM temp.cleared);

-- 2. adjustments other records hold on a cleared transaction say it is gone
--    (clear.rs:170-179; the wording is facts.rs:527 with TransactionId printed as record/leg, core/src/ids.rs:198-201)
INSERT INTO record_problems (record_id, code, detail)
SELECT a.record_id, 'adjustment-target-gone',
       'the transaction it explains, ' || a.applies_record || '/' || a.applies_leg || ', was cleared'
  FROM adjustments a
 WHERE a.applies_record IN (SELECT id FROM temp.cleared)
   AND a.record_id NOT IN (SELECT id FROM temp.cleared);

-- 3. rows that point at a record, children before parents
DELETE FROM transfer_links WHERE out_record IN (SELECT id FROM temp.cleared) OR in_record IN (SELECT id FROM temp.cleared);

-- links (DIFFERENCE 1, see note): only the import's own link_records rows go, then any link left with no 'from' side
DELETE FROM link_records WHERE record_id IN (SELECT id FROM temp.cleared);
CREATE TEMP TABLE empty_links AS
  SELECT id FROM links WHERE id NOT IN (SELECT link_id FROM link_records WHERE side = 'from');
DELETE FROM link_records WHERE link_id IN (SELECT id FROM temp.empty_links);
DELETE FROM links WHERE id IN (SELECT id FROM temp.empty_links);
DROP TABLE temp.empty_links;

DELETE FROM instrument_sightings WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM adjustment_legs     WHERE record_id IN (SELECT id FROM temp.cleared);   -- before adjustments (FK v36.sql:116)
DELETE FROM adjustments         WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM record_problems     WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM record_refs         WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM record_revisions    WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM transactions        WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM record_holds        WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM record_orders       WHERE record_id IN (SELECT id FROM temp.cleared);
DELETE FROM statement_holds     WHERE record_id IN (SELECT id FROM temp.cleared);

-- 4. the records last: every table above references source_records(id) and nothing cascades
DELETE FROM source_records WHERE id IN (SELECT id FROM temp.cleared);
DROP TABLE temp.cleared;
COMMIT;
```
Order rationale: step 1 and 2 before any delete because they read `trades.anchor_record` and `adjustments.applies_record`, which point at the ids about to go; `adjustment_legs` before `adjustments` (FK v36.sql:116); `link_records` before `links` (FK v36.sql:311); every table with `REFERENCES source_records(id)` before `source_records` (v36.sql:122, 259, 312, 441, 451, 456, 463, 467, 528, 614, 640, 642). Without `foreign_keys = ON` the same order still leaves no dangling row.

Difference 1, the links. clear.rs:182-185 deletes every link a cleared record is on, both sides. [corrected] The pull's `store_row` finds what a broker row replaces by `book.records_by_ref(scheme, &row.key)` (R/rust/crates/broker/src/pull.rs:701-706), and `records_by_ref` reads `record_refs` alone: `SELECT record_id FROM record_refs WHERE scheme = ? AND value = ?` (records.rs:561-565), under the scheme `broker-record:wealthsimple` (R/rust/crates/broker/src/lib.rs:206-208). Only the import stores a ref under that scheme (`refs.push((WEALTHSIMPLE_RECORD.to_string(), cid.to_string()))`, R/rust/crates/book/src/import/mod.rs:108); csv rows are stored with `refs: vec![]` (R/rust/crates/server/src/csv_import.rs:248), person records with `refs: vec![]` (R/rust/crates/book/src/person.rs:448), statement records with `refs: vec![]` (R/rust/crates/broker/src/statements.rs:317, 335, 420, 538). So `store_superseding(..., &replaces, "the broker's own row", now)` (pull.rs:712) never puts a csv or person record on a link beside an import record. The one place a csv or person record can share a link's `from` side with an import record is `supersede_copies` (pull.rs:751-784): its `copies` are live records of any source but the broker's (`if rec.state != RecordState::Live || rec.source == source { continue; }`, pull.rs:772-775) whose every transaction looks like a leg the broker's row books in another account, and they are superseded together by one `book.supersede(&copies, &[record], "the broker's own row, booking the move leaving this account", now)` (pull.rs:784). The csv linker's own links are one row to one broker record (csv_import.rs:517-519). Where such a shared link exists, deleting the whole link would make `superseded_by` (links.rs:160-167), `standing_for` (links.rs:119-136) and `supersedes_from` (links.rs:140-147, the linker's production check that a broker record has already taken a file row's place, csv_import.rs:499) answer nothing for that csv record; `superseding(&csv::source())` (records.rs:749-758) is read only by the test-only `whole_plan` (csv_import.rs:621-627). Deleting only the import's `link_records` rows and then links with no `from` row left (the plan's wording, R/docs/plans/remove-the-earlier-app.md:56-57) avoids that either way. A link left with only `to` rows must not be kept either: `has_superseded` counts `side = 'to'` rows (links.rs:150-157), and `mark_removed` would then add a "superseding-record-removed" problem on the broker's record (records.rs:488-493).

Difference 2, what the book does after a delete that raw SQL does not: `settle_successions` (links.rs:110, clear.rs:230) and `settle_names` inside every `atomically` (lib.rs:143-158). The triggers still queue the work: deleting sightings inserts into `names_due` (v36.sql:770-772), deleting a corporate-event transaction bumps `successions_due` (v36.sql:951-953), and every delete and the trade update are logged to `ledger_changes` (v36.sql:894-897, 941-943, 845-847, 875-877, 960-962, 937-940), which the engine's catch-up reads. These are settled at the book's next write. `clear_unnamed` (clear.rs:236-298) is not run, so an instrument or account named only by import rows stays as a harmless row; the import's accounts are the broker's own connection anyway (R/rust/crates/book/src/import/mod.rs:134-151).

## 3. The person's own data

| What | Table | Hangs on |
|---|---|---|
| Journal entry (thesis, grade) | `journal` | `trade_id TEXT UNIQUE REFERENCES trades(id)` or `group_id ... REFERENCES trade_groups(id)`, exactly one (v36.sql:290-298) |
| Tags | `journal_tags` | `journal_id REFERENCES journal(id)` (v36.sql:299-304) |
| Groups | `trade_groups`, `trade_group_members` | members `trade_id REFERENCES trades(id)` (v36.sql:590-602) |
| Trades | `trades` | `anchor_record`/`anchor_leg` text, no FK; `anchor_instrument REFERENCES instruments(id)`; `(anchor_record IS NULL) = (orphaned_reason IS NOT NULL)` (v36.sql:603-612) |
| Orders and brackets | `orders`, `order_events`, `brackets`, `bracket_events` | `orders.bracket_id REFERENCES brackets(id)` (v36.sql:394); no column names a record (v36.sql:380-412, 144-156, 370-379, 134-143). Carried from the old file by `legacy_orders::carry_orders` (R/rust/crates/server/src/legacy_orders.rs:195, called at R/rust/crates/server/src/carry.rs:299) |
| Record to order link | `record_orders` | `record_id REFERENCES source_records(id)`, `order_id` plain text (v36.sql:450-454); written by `derive` from the mapping (records.rs:282-285) |
| Notifications | `notifications`, `told`, settings `notify.%` | no FK at all (v36.sql:350-360, 584-589); carried by `carry_notices` (R/rust/crates/book/src/notices.rs:87, carry.rs:312) |
| Watchlist and tiles | `watched`, `tiles`, `listings_named` | `instrument_id REFERENCES instruments(id)` (v36.sql:646-649, 580-583, 322-329); carried by `carry_following` (carry.rs:126, R/rust/crates/book/src/watched.rs:235) |
| Repayment answers | `repayment_answers` | `payee_key`, no FK (v36.sql:473-477) |

Nothing of the person's references a record directly; only a trade's anchor does, and a journal reaches a record only through `journal.trade_id -> trades.anchor_record`.

**Deleting a superseded (or removed) import record.** It cannot detach a trade or a journal entry, because by the time a record becomes superseded no trade is anchored on it: `supersede_counting` moves or orphans every trade from `trades_anchored_on(*r)` (links.rs:58-74) before `set_state(*r, RecordState::Superseded, at)` (links.rs:94), and `mark_removed` orphans them (records.rs:478-480) before `set_state(record, RecordState::Removed, at)` (records.rs:487). Nothing anchors a trade on a non-live record afterwards: `open_trade` and `anchor_on` go through `check_opening`, which requires `self.transaction(tx)` to exist (trades.rs:57-61), and a superseded record has no transactions (records.rs:502-505). A trade that supersede moved sits on the broker's record (trades.rs:218-225, reason cleared to NULL); a trade it orphaned already has `anchor_record = NULL` and keeps its journal (trades.rs:97-103; tests/links.rs:82-96). The `UPDATE trades ... WHERE anchor_record IN (cleared)` therefore matches no row for superseded or removed records; query 2 in section 4 proves it on the owner's book (no row for `superseded` or `removed`). Orders, brackets, notifications, told marks, watchlist, tiles and groups have no column naming a record, so no delete of a record can detach them. The only rows of the person's kind that go are `record_orders` rows of import records (clear.rs:194 does the same); the orders themselves stay. Whether the import mapping produced any such rows is answered by query 4.

**Deleting the live import record.** If a trade is anchored on it, step 1 orphans that trade with "the record it opened on was cleared" and its journal stays (clear.rs:165-169; thesis kept, held by the test `clearing_the_broker_s_records_keeps_the_journal_orphaned_and_the_next_pull_reads_in_full`, R/rust/crates/server/src/clear.rs:579-600). The transaction itself is gone, so the move leaves the figures: the plan says so, "deleting the imported rows as they stand would drop that one move out. The holding would then count in both RESPs at once" (R/docs/plans/remove-the-earlier-app.md:21). If the broker's own row later books that leg (pull.rs:751-785 `supersede_copies` books "the move leaving this account"), the engine's `settle_trades` gives the round trip a new trade with `open_trade(..., None, at)` (R/rust/crates/server/src/figures.rs:229-232 [corrected: was cited as 228-231; the call is at line 230]), not the orphaned one; the orphaned note is then only reachable through `orphaned_journal` (trades.rs:445-457), whose only production caller is `reattach_carried_notes` (R/rust/crates/server/src/legacy_import.rs:424), and no page lists orphaned notes (grep of `rust/crates/server/src/http` and `web/src` for "orphaned" returns nothing; the engine maps an orphaned anchor to nothing, R/rust/crates/engine/src/input.rs:345, 351). A units move out of an account has direction -1 (records.rs:80-91); whether a trade is anchored on it is answered by the `live` row of query 2.

## 4. A read-only counting query

Run as `sqlite3 -readonly book.db < counts.sql`. Only states, table names, the app's source labels and counts are printed. Column names checked against v36.sql: `source_records(id, source, state)` (500-511), `trades(id, anchor_record, legacy_key)` (603-612), `journal(id, trade_id)` (290-298), `transactions/record_revisions/record_refs/record_problems/record_holds/record_orders/statement_holds/instrument_sightings.record_id` (614, 467, 463, 456, 441, 451, 528, 259), `adjustments(record_id, applies_record)` (122-124), `transfer_links(out_record, in_record)` (640-642), `link_records(link_id, record_id, side)` (310-315), `links(id)` (316-321).
```sql
.headers on
.mode column

-- 1. import records by state
SELECT 'import records' AS what, state, COUNT(*) AS n
  FROM source_records
 WHERE source = 'bagholder-import'
 GROUP BY state;

-- 2. trades anchored on an import record, by that record's state
--    (expected: only a 'live' row, or none; supersede and removal orphan or move anchors first)
SELECT 'trades anchored on an import record' AS what, r.state, COUNT(*) AS n
  FROM trades t
  JOIN source_records r ON r.id = t.anchor_record
 WHERE r.source = 'bagholder-import'
 GROUP BY r.state;

-- 3. journal entries on those trades, by the record's state
SELECT 'journal entries on those trades' AS what, r.state, COUNT(*) AS n
  FROM journal j
  JOIN trades t ON t.id = j.trade_id
  JOIN source_records r ON r.id = t.anchor_record
 WHERE r.source = 'bagholder-import'
 GROUP BY r.state;

-- 4. rows hanging off import records, by table
WITH imp AS (SELECT id FROM source_records WHERE source = 'bagholder-import')
SELECT 'transactions' AS tbl, COUNT(*) AS n FROM transactions WHERE record_id IN imp
UNION ALL SELECT 'record_revisions', COUNT(*) FROM record_revisions WHERE record_id IN imp
UNION ALL SELECT 'record_refs', COUNT(*) FROM record_refs WHERE record_id IN imp
UNION ALL SELECT 'record_problems', COUNT(*) FROM record_problems WHERE record_id IN imp
UNION ALL SELECT 'record_holds', COUNT(*) FROM record_holds WHERE record_id IN imp
UNION ALL SELECT 'record_orders', COUNT(*) FROM record_orders WHERE record_id IN imp
UNION ALL SELECT 'statement_holds', COUNT(*) FROM statement_holds WHERE record_id IN imp
UNION ALL SELECT 'instrument_sightings', COUNT(*) FROM instrument_sightings WHERE record_id IN imp
UNION ALL SELECT 'adjustments held by import records', COUNT(*) FROM adjustments WHERE record_id IN imp
UNION ALL SELECT 'adjustments of other records applying to an import transaction', COUNT(*) FROM adjustments WHERE applies_record IN imp AND record_id NOT IN imp
UNION ALL SELECT 'transfer_links', COUNT(*) FROM transfer_links WHERE out_record IN imp OR in_record IN imp
UNION ALL SELECT 'link_records rows of import records', COUNT(*) FROM link_records WHERE record_id IN imp
UNION ALL SELECT 'links with an import record on the from side', COUNT(DISTINCT link_id) FROM link_records WHERE side = 'from' AND record_id IN imp
UNION ALL SELECT 'links whose from side is only import records',
       COUNT(*) FROM links l
        WHERE EXISTS (SELECT 1 FROM link_records f WHERE f.link_id = l.id AND f.side = 'from')
          AND NOT EXISTS (SELECT 1 FROM link_records f JOIN source_records r ON r.id = f.record_id
                           WHERE f.link_id = l.id AND f.side = 'from' AND r.source <> 'bagholder-import');

-- 5. orphaned trades, with and without an earlier-app key, and how many carry a journal entry
SELECT 'orphaned trades' AS what,
       COALESCE(SUM(t.legacy_key IS NOT NULL), 0)                        AS with_legacy_key,
       COALESCE(SUM(t.legacy_key IS NOT NULL AND j.id IS NOT NULL), 0)   AS with_legacy_key_and_journal,
       COALESCE(SUM(t.legacy_key IS NULL), 0)                            AS without_legacy_key,
       COALESCE(SUM(t.legacy_key IS NULL AND j.id IS NOT NULL), 0)       AS without_legacy_key_with_journal
  FROM trades t LEFT JOIN journal j ON j.trade_id = t.id
 WHERE t.anchor_record IS NULL;

-- 6. anchored trades that still carry an earlier-app key, by the source of the record they sit on
SELECT 'anchored trades with a legacy key' AS what, r.source, COUNT(*) AS n
  FROM trades t JOIN source_records r ON r.id = t.anchor_record
 WHERE t.legacy_key IS NOT NULL
 GROUP BY r.source;
```

## 5. legacy_key, orphan_trade, LegacyNote, reattach_carried_notes, orphaned_journal

- **`trades.legacy_key`** (v36.sql:608; unique partial index `trades_legacy_key ... WHERE legacy_key IS NOT NULL`, v36.sql:41; both from migration 1, R/rust/crates/book/migrations/001-the-book.sql:195, 201). Purpose: "the key an earlier version of the app knew the trade by, where it was imported" (trades.rs:29-30). `open_trade(opening, legacy_key, at)` returns the trade found by key, or stamps the key on the trade already on the opening, or inserts it with the key (trades.rs:31-52); `trade_by_legacy_key` reads it (trades.rs:317-320); `Trade.legacy_key` is carried on the core type (R/rust/crates/core/src/journal.rs:17). The only production caller passing a key is the import, `self.open_trade(&Opening {...}, Some(key), at)` (R/rust/crates/book/src/import/mod.rs:350); the engine's settling passes `None` (figures.rs:230). `trade_groups.legacy_key` is the same for groups (v36.sql:600, index v36.sql:38; `add_group(..., legacy_key, ...)` trades.rs:465-486; only caller with a key import/mod.rs:277). Nothing in the engine, the HTTP layer or the web reads `legacy_key` (grep of `rust/crates/engine/src`, `rust/crates/server/src/http`, `mcp.rs`, `views.rs`, `web/src` returns nothing; the engine's tests only fill the field with `None`, R/rust/crates/engine/tests/common/mod.rs:198, 384). Earlier-app code: yes, and the plan lists "the `legacy_key` parameters" for removal (R/docs/plans/remove-the-earlier-app.md:98); the column itself would stay unless a migration drops it.
- **`orphan_trade(legacy_key, reason, at)`** (trades.rs:80-95): inserts a trade with no anchor, `INSERT INTO trades(id, orphaned_reason, legacy_key, created_at)`, "A trade an earlier version of the app knew by `legacy_key` and whose opening cannot be found: kept orphaned, with the reason, so what the person wrote on it is not lost" (trades.rs:80-83). Callers: the import only, import/mod.rs:290, 309, 328, 332, 346. Earlier-app code: yes (plan line 98 removes it).
- **`LegacyNote`** (R/rust/crates/store/src/tables.rs:322-334): "A note kept on a trade by an older version of the page, before the journal", fields `thesis`, `tag`, `grade`, `trade_id`; `LegacyNotes` map at tables.rs:337, read from the old file's `meta` by `trade_notes` (tables.rs:360-373); carried on the old store's `Book` struct (R/rust/crates/store/src/book.rs:43-52, filled at book.rs:106, whose builder `book::book` is called only from store/tests/golden_store.rs:939, 1000). It reaches the web as a generated type (R/web/src/lib/generated/book.ts:9, 27; model_api.ts:5) through the type-generation test (R/rust/crates/server/src/tests_types.rs:17, 164, 363; `mod tests_types` is under `#[cfg(test)]`, main.rs:651-652). No production code in the server reads it (grep of `rust/crates/server/src` finds only tests_types.rs). Earlier-app code: yes, in the `bagholder-store` crate the plan removes entirely (remove-the-earlier-app.md:89, 99-103).
- **`reattach_carried_notes(home, book, engine, at)`** (R/rust/crates/server/src/legacy_import.rs:407-471): puts back, once per book, the notes carried from the earlier app that an earlier build orphaned. Called from `built` at every whole engine build, `if crate::legacy_import::reattach_carried_notes(home, book, &e, now)? { catch_up(...); settle_trades(...); }` (figures.rs:269-272; `built` is called at figures.rs:414, 440, 456, 675). It returns at once once the repair is recorded, `if book.repaired(REATTACH)` with `REATTACH = "carried-notes-reattached"` (legacy_import.rs:359, 421-423; `repaired`/`record_repair` keep a `repair.<name>` row in `settings`, trades.rs:287-300). Until then, when any orphaned trade with a `legacy_key` has a note (legacy_import.rs:424), it reads the earlier app's file [corrected: the read happens only inside `if !orphans.is_empty()`, legacy_import.rs:426; with no such orphan the repair is recorded at once, legacy_import.rs:469, and nothing is read]: `original_database` returns `home/bagholder.db` (`crate::figures::OLD_FILE`, figures.rs:34) or the newest `snapshots/bagholder-before-the-book-*.db` (legacy_import.rs:365-389), `translated_from` copies it and runs the old engine's `translate` (legacy_import.rs:393-405, 35), and each note goes back through `Book::reattach` (trades.rs:263-285; legacy_import.rs:464). Earlier-app code: yes (plan line 84).
- **`orphaned_journal()`** (trades.rs:445-457): "Journal entries on orphaned trades, for the person to re-attach, with why"; joins `journal_entries` with `trade(id)` and keeps `Anchor::Orphaned`. It is the book's own orphan model, not import-specific (lib.rs:4-5; trades.rs:4-6), but its only production caller is `reattach_carried_notes` (legacy_import.rs:424); every other use is a test (figures.rs:1722, R/rust/crates/server/src/demo.rs:570, tests/links.rs:94, tests/records.rs:216, tests/import.rs:356, 482, legacy_import.rs:706, 730, 755). No page or route lists it (grep above).

For completeness, the other earlier-app code on the start path: `adopt_python_database` copies `~/.bagholder/bagholder.db` into the data folder on a first start (R/rust/crates/server/src/main.rs:235-238; legacy_import.rs:492-544); `Figures::open` imports `bagholder.db` when no `book.db` exists (figures.rs:393-398); `carry::from_old_store` runs at every start (main.rs:157-160) and, when `bagholder.db` is present, opens it, retires its figure tables with a copy (legacy_import.rs:310-327), carries orders, the watchlist, the watch folder and notifications (carry.rs:279-321); Clear data writes to it (R/rust/crates/server/src/clear.rs:170-171); `bagholder import-book` and `compare-figures` are CLI subcommands (main.rs:414-420). The import itself: `IMPORT_SOURCE = "bagholder-import"` and `WEALTHSIMPLE_RECORD = "broker-record:wealthsimple"` (R/rust/crates/book/src/import/mapping.rs:32, 36); the pull supersedes an import record by that ref (pull.rs:701-713 [corrected: was 702-713], 790-800; records.rs:720-732 used at pull.rs:667). The owner's decision of 2026-10-21: "Nothing of the earlier app is kept, by a one-time migration, never a standing condition ... the migration's own code goes one release later behind a stated minimum upgrade path" (R/docs/decisions.md:7), and "A closed account is read like any other" (docs/decisions.md:8); the older plan with the conditional upkeep step is R/docs/plans/remove-the-earlier-app.md:53-75.

## 6. Schema snapshots and migrations

- **Declaration.** 36 migrations, each `include_str!` of `migrations/NNN-*.sql`, in `MIGRATIONS: [Migration; 36]` (R/rust/crates/book/src/schema.rs:7-44), under `SCHEMA { name: "book", application_id: 0x4248_424B ("BHBK") }` (schema.rs:46-51). "A released migration is never edited: each version's schema is committed in `schema/v<N>.sql` and the tests compare" (schema.rs:1-3); the architecture rule is R/docs/architecture.md:103.
- **How one is applied and recorded** (R/rust/crates/sqlite/src/migrate.rs). `open` checks numbering is 1..N with no gaps (migrate.rs:122, 134-142), looks at the file read-only and refuses one that is another store's or newer (`identify`, migrate.rs:149-168; `MigrateError::Newer` 62, 164-166), then `migrate` (170-197): if `from > 0` it first copies the whole file with `VACUUM INTO` to `snapshots/<stem>-v<from>-<stamp>.db` beside it (178-180, 249-267) keeping the newest of the two latest versions (`SNAPSHOTS_KEPT = 2`, 51-55, 301-315); it turns `foreign_keys` off for the run and back after (191-194). `run_pending` applies each pending migration in its own transaction: migration 1 also sets `PRAGMA application_id` and creates `schema_migrations (number, name, applied_at, app_version)` (218-229); then `conn.execute_batch(m.sql)`, `references_hold` (`PRAGMA foreign_key_check`, 199-213), `INSERT INTO schema_migrations(number, name, applied_at, app_version)`, and `PRAGMA user_version = <n>` (230-237); a failure leaves the file at the version before and names the migration (238, 85-89). The book then turns `foreign_keys` and `recursive_triggers` on (lib.rs:103-106).
- **The snapshots.** `schema/v1.sql` to `v36.sql`; `v36.sql` is 969 lines and `v35.sql`/`v34.sql` 967 (`wc -l`); each starts `-- user_version N` followed by every `sqlite_master` entry in type and name order, as `schema_text` prints it (migrate.rs:317-331). The files are test fixtures only; nothing in migrate.rs or the book reads them at runtime.
- **The tests that hold them** (R/rust/crates/book/tests/schema.rs): `each_version_is_exactly_the_schema_committed_for_it` builds every prefix `MIGRATIONS[..n]` in a fresh file and compares to `schema/v{n}.sql`, regenerating with `BAGHOLDER_BLESS` (schema.rs:12-33); `every_earlier_version_reaches_the_latest_with_its_rows` opens a file at each version 1..35 with a row and checks `(done.from, done.to) == (n, 36)`, a snapshot taken, the row kept (schema.rs:35-52); `the_header_and_the_record_of_migrations_agree` checks `user_version`, `schema_migrations` 1..36 and the application id (schema.rs:54-64); `an_earlier_apps_database_is_not_opened_as_a_book` (schema.rs:66-74); and migration-specific tests for 25, 26 and 30 (schema.rs:114-147, 161-266, 268-301).
- **Can the app drop old migrations and start from a snapshot ("a book older than vN must go through release X first")?** Not as written. `check_numbering` requires the list to start at 1 (migrate.rs:134-142); `run_pending` slices `&schema.migrations[from as usize..]`, so migration k must sit at index k-1 (migrate.rs:216); migration 1 is the one that creates `schema_migrations` (migrate.rs:218-229); `identify` refuses a version-0 file that has tables (migrate.rs:157-160) and a newer file (164-166) but has no "too old" refusal, and `MigrateError` has no such variant (migrate.rs:57-70). Nothing reads `schema/vNN.sql` at runtime. Doing it would need a base version on `Schema`, an `Older { file, minimum }` refusal in `identify` naming the release to go through, the slice in `run_pending` offset by that base, a way to create a fresh file from the base (a base SQL or the first kept migration carrying the whole schema), and the two schema tests rewritten to build from the base. The decision at docs/decisions.md:7 ("the migration's own code goes one release later behind a stated minimum upgrade path") asks for exactly this mechanism, which does not exist yet.

## verdicts
- confirmed: source_records.state is constrained to 'live', 'superseded', 'removed'; a record is inserted 'live' and only set_state changes it, called by supersede_counting  | rust/crates/book/schema/v36.sql:505 `state TEXT NOT NULL CHECK (state IN ('live', 'superseded', 'removed'))`. records.rs:160-161 `INSERT INTO source_records(... state ...) VALUES (?1, ?2, ?3, ?4, 'live', ?5, ?6, ?6)`. re
- confirmed: A superseded or removed record's transactions, problems, sightings and adjustments are deleted by clear_derived; its revisions, refs, holds, order links and lin | records.rs:500-507: doc `Its revisions stay` (501), `for table in ["transactions", "record_problems", "instrument_sightings"] { ... DELETE FROM {table} WHERE record_id = ? }` (503-505), then `self.clear_adjustments(recor
- confirmed: supersede_counting moves each trade anchored on a from record to the earliest same_opening transaction of the to records with no trade on it (move_anchor), else | links.rs:47-54 collects and sorts `targets` (`the earliest first: by instant (an unknown instant last), record, leg`, 51); 58-74 loops `trades_anchored_on(*r)`, orphans with `the record it opened on had no such transacti
- confirmed: Twelve columns reference source_records(id) with a foreign key (adjustments, instrument_sightings, link_records, record_holds, record_orders, record_problems, r | `grep -n 'REFERENCES source_records' v36.sql` returns exactly lines 122, 259, 312, 441, 451, 456, 463, 467, 528, 614, 640, 642 (twelve). v36.sql:605 `anchor_record TEXT,` and 124 `applies_record TEXT NOT NULL,` carry no 
- confirmed: The book opens with PRAGMA foreign_keys ON, so children must be deleted before source_records. | rust/crates/book/src/lib.rs:103 `conn.pragma_update(None, "foreign_keys", "ON")?;` under the comment `every reference between the book's tables is held by SQLite itself` (102).
- confirmed: Clear data orphans trades on cleared records with 'the record it opened on was cleared', flags adjustments, then deletes transfer_links, whole links, sightings, | rust/crates/book/src/clear.rs:163 temp table `cleared`; 165-169 `UPDATE trades SET anchor_record = NULL ... WHERE anchor_record IN (SELECT id FROM temp.cleared)` with `"the record it opened on was cleared"` (168); 170-17
- corrected: A broker row's store_superseding puts every live record of another source carrying its id on the from side of one link, so a csv/person record can share a link  | The mechanism named is wrong; the consequence holds through another one. store_row collects `replaces` from `book.records_by_ref(scheme, &row.key)` (pull.rs:701, not 702), and `records_by_ref` reads `record_refs` alone: 
- confirmed: Journal entries hang on trades.id or trade_groups.id, tags on journal.id, group members on trades.id; orders, brackets, notifications, told, watched, tiles and  | v36.sql:290-298 journal `trade_id TEXT UNIQUE REFERENCES trades(id)`, `group_id TEXT UNIQUE REFERENCES trade_groups(id)`, `CHECK ((trade_id IS NULL) <> (group_id IS NULL))`; 299-304 journal_tags `journal_id INTEGER NOT N
- confirmed: Deleting a superseded or removed import record cannot detach a trade or journal entry, because no trade is anchored on a non-live record (supersede and removal  | links.rs:58-74 moves or orphans every `trades_anchored_on(*r)` before `set_state(*r, RecordState::Superseded, at)` at 94; records.rs:478-480 `for (trade, _) in self.trades_anchored_on(record)? { self.orphan(trade, "the s
- corrected: Deleting the live import record orphans any trade anchored on it (journal kept) and drops its transaction from the figures; the engine later opens a new trade w | Substance right, one location off by one. clear.rs:165-169 orphans with `the record it opened on was cleared`; server/src/clear.rs:578-600 test `clearing_the_broker_s_records_keeps_the_journal_orphaned_and_the_next_pull_
- confirmed: trades.legacy_key and trade_groups.legacy_key date from migration 1; the only production callers passing a key are the import's open_trade and add_group; orphan | migrations/001-the-book.sql:195 `legacy_key TEXT,` in trades, 201 `CREATE UNIQUE INDEX trades_legacy_key`, 206 `legacy_key TEXT,` in trade_groups, 209 `CREATE UNIQUE INDEX trade_groups_legacy_key`. trades.rs:31 `open_tra
- confirmed: LegacyNote is the old store's note type in bagholder_store::tables, exported to the web's generated book.ts, with no production reader in the server. | rust/crates/store/src/tables.rs:322-334 doc `A note kept on a trade by an older version of the page, before the journal`, `pub struct LegacyNote` (325) with thesis, tag, grade, trade_id; 337 `pub type LegacyNotes = BTree
- corrected: reattach_carried_notes is called from figures::built at every whole engine build, returns at once once the 'carried-notes-reattached' repair is recorded, and un | Call and early return confirmed: figures.rs:269-272 `if crate::legacy_import::reattach_carried_notes(home, book, &e, now)? { catch_up(...); settle_trades(...); }` inside `built` (259), which is called at figures.rs:414, 
- confirmed: orphaned_journal's only production caller is reattach_carried_notes; all other uses are tests. | `grep -rn orphaned_journal` over rust/crates and web/src: definition trades.rs:446; production call legacy_import.rs:424; the rest are figures.rs:1722 (after `#[cfg(test)]` at figures.rs:905), demo.rs:570 (after `#[cfg(t
- confirmed: Each migration runs in its own transaction: migration 1 creates schema_migrations and sets application_id; each then executes its SQL, checks PRAGMA foreign_key | rust/crates/sqlite/src/migrate.rs:216-240 `for m in &schema.migrations[from as usize..] { crate::atomically(conn, || { if m.number == 1 { PRAGMA application_id = ...; CREATE TABLE schema_migrations (number, name, applied
- confirmed: Dropping old migrations is not supported as written: numbering must start at 1 with no gaps, run_pending indexes migrations by from, migration 1 creates schema_ | migrate.rs:134-142 `check_numbering`: `let expected = i as u32 + 1; if m.number != expected { return Err(MigrateError::Numbering {...}) }`. 216 `for m in &schema.migrations[from as usize..]`. 218-229 migration 1 creates 
- confirmed: The schema snapshots are held by book/tests/schema.rs: every prefix of MIGRATIONS must equal schema/v{n}.sql (BAGHOLDER_BLESS regenerates), every earlier versio | rust/crates/book/tests/schema.rs:12-20 `built(n)` opens `&MIGRATIONS[..n]` and returns `schema_text`; 22-33 `each_version_is_exactly_the_schema_committed_for_it` compares with `schema/v{n}.sql` (25), writes it when `BAGH
- confirmed: v36.sql is 969 lines and v34/v35 are 967; earlier snapshots are shorter. | `wc -l rust/crates/book/schema/v*.sql`: v36.sql 969, v35.sql 967, v34.sql 967, v33.sql 961, v32.sql 943, v31.sql 937, v30.sql 935, v29.sql 935, v28.sql 871, down to v1.sql 198; every file before v34 is shorter than 967.
- confirmed: The owner's decision of 2026-10-21 replaces the conditional upkeep step with a one-time migration and a stated minimum upgrade path, and reads closed accounts l | docs/decisions.md:7 `Nothing of the earlier app is kept, by a one-time migration, never a standing condition ... the migration's own code goes one release later behind a stated minimum upgrade path ... Replaces brief 74'
- confirmed: Other start-path earlier-app code: adopt_python_database on first start, Figures::open importing bagholder.db when no book exists, carry::from_old_store at ever | server/src/main.rs:235-237 `legacy_import::adopt_python_database(&home, &python)` on start (legacy_import.rs:498-544, only when no book.db and a Python `bagholder.db` exists, 501-503); figures.rs:393-398 `if !book_path.e

## open
- Whether a trade is anchored on the one live import record (the leaving side of the 2025-06-16 RESP move): query 2's 'live' row answers it; if one is, deleting that record orphans it and its note becomes reachable only through orphaned_journal, which no page lists.
- Whether any link puts a csv or person record on the same 'from' side as an import record on the owner's book (query 4's last two rows): if the counts differ, deleting whole links as clear.rs does would strip the csv record's supersession, so the narrower link deletion in section 2 should be used.
- Whether the import mapping produced any record_orders rows (query 4): those rows go with the records, though the orders themselves stay.
- After deletion, should the orphaned trades with a legacy_key (query 5) be kept, have their key cleared (the plan at docs/plans/remove-the-earlier-app.md:58 clears trades.legacy_key), or be dropped where they carry no journal entry; and should the orphaned-journal list get a page, since nothing shows it once reattach_carried_notes is gone.
- The one-time migration the decision asks for (docs/decisions.md:7) and the 'minimum upgrade path' need a mechanism migrate.rs does not have (no too-old refusal, numbering must start at 1); whether to add a base version to Schema or to keep the full migration list is undecided in the code.
- A raw SQL delete outside the book leaves names_due, successions_due and ledger_changes rows for the book's next write to settle, and does not run clear_unnamed; if the deletion is done as a migration, it runs with foreign keys off (migrate.rs:191-194) and before the book's connection exists, so settle_names/settle_successions run at the first write after it; whether that first write happens before the first page read was not checked.