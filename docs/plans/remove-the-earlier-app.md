# Plan: nothing of the earlier app is kept, and every holding still adds up

Amended with brief 74's four changes (8cf981ac), each marked "(brief 74, n)".

## For the owner to decide

Nothing open. The owner decided on 2026-10-20:
- "obviously nothing should ever be happening with some old database. this is ridiculous."
- "It is not acceptable that any part of this design would force us to keep anything related to an old database. OBVIOUSLY."
- "make sure you fix whatever is needed so Bagholder can handle these properly and be accurate"

## What the owner's book shows

The owner ran a count on their book on 2026-10-20 (counts and kinds only). There are 5,861 imported rows (`source_records.source = 'bagholder-import'`) from 2023-09-08 to 2026-09-25:
- 5,573 are `superseded`: Bagholder read the same activity from Wealthsimple and uses that instead;
- 288 are `removed`: credit-card rows no longer in Wealthsimple's list;
- **1 is `live`**: the leaving side (`ASSET_MOVEMENT`/`SOURCE`, a debit) of a holding moved on 2025-06-16 from a **managed RESP**, since closed, into the self-directed RESP.

Its arriving side was read from Wealthsimple (`ASSET_MOVEMENT/DESTINATION`, 2025-06-16) and replaced the imported copy. Its leaving side has no row among what Bagholder read from Wealthsimple. The 2025-07-04 move has both sides from Wealthsimple.

So deleting the imported rows as they stand would drop that one move out. The holding would then count in both RESPs at once. The plan makes Bagholder read that move from Wealthsimple first.

The two manual LIRA withdrawals the owner named (2025-06-26 and 2026-01-12) are a different case. They're cash, missing from the feed, and already booked from the statements (`docs/plans/statement-gaps.md`). Nothing about them changes.

## Part 1: the move out of an account the feed leaves out

**What Wealthsimple states.** The sync reads closed accounts. It records Wealthsimple's closed status, and reads a closed account's activity in full once (`broker/src/pull.rs:125, :162`). Wealthsimple's mapping of a holding move (`wealthsimple/src/mapping.rs`, `transfer`, from L857) already books both sides of a move from one row when the units each account held that day are known: `mirrored` pairs a rise in one account with the same fall in the other, using `opposingAccountId` and the holdings by day. Statement rows carry cash only (`broker::StatementRow`), so the statements can't supply a units move.

**Step 1, a check on the owner's book that names the cause.** A command, `bagholder explain-move <day>`, runs once and is deleted in Part 2 (brief 74, 4). It prints the shape of that day's move rows and nothing that identifies the person:
- the kinds of rows;
- whether `opposingAccountId` is present;
- whether the opposing account is in the book, and its status and kind;
- whether its activity was read in full;
- whether its holdings on the day before and the day of the move were read;
- the units change per side, as signs only.

The owner runs it once, and the PR states the cause it shows. These are the possible causes, each with its fix:
- **The managed RESP isn't in the book.** Wealthsimple's accounts list (`FetchAllAccounts`, default `filter: {}`) doesn't name it. Fix: ask with the filter that includes closed and managed accounts, found the way the other documents were found (`docs/plans/stage-3b-wealthsimple.md`, from Wealthsimple's web app). Then read its activity in full once, as every closed account is.
- **It's in the book, but its holdings on those days weren't read,** so `mirrored` couldn't pair the sides. Fix: for a move whose `opposingAccountId` is one of the person's accounts, read that account's holdings on the day before and the day of the move. This is the read the mapping already uses for its own account.
- **It's in the book and its holdings were read, but the single arriving row didn't book the leaving side.** Fix: the mapping books the opposing account's matched fall (`matched` holds both accounts) as that account's own leg, from the arriving row.

In every case the leaving side is booked from what Wealthsimple states: its row, its `opposingAccountId`, and the units each account held. Never from the imported row.

**Test:** recorded replies with made-up figures, shaped like the owner's move:
- an arriving `ASSET_MOVEMENT` row naming a closed managed RESP;
- that account's activity without a leaving row;
- both accounts' holdings around the day.

The book must hold both legs, the closed RESP at zero after the move, and the self-directed RESP with the units.

**Then on the owner's book:** the next sync books the leaving side, and the one live imported row is superseded like the others. The PR's check is the count, run again: zero `live` imported rows.

## Part 2: the earlier app's data and code removed

**The data.** One book upkeep step (brief 74, 2). It isn't a schema migration: it changes no table, it runs only when its condition holds, and once done it's recorded in the book's settings (`upkeep.earlier-app-rows-removed`), as the book's other repairs are recorded. The step:
- deletes every `bagholder-import` record, its revisions, refs, problems, holds, order links and `link_records`, and the transactions derived from them;
- deletes the `links` left with no records;
- clears `trades.legacy_key`.

It runs only when no `bagholder-import` record is `live`. A book where one still is keeps them and says which, in words, until a sync supersedes it, so nothing that counts is ever dropped. On the owner's book it runs after Part 1's sync.

**The person's own data survives it** (brief 74, 1). Before deleting anything, the step checks that none of these depends on an imported record or a transaction derived from one, and refuses, naming each one, if any does:
- every trade's anchor: the round trip a journal entry is written on;
- the journal itself: thesis, grade and tags;
- the orders and brackets carried from the earlier app;
- the notifications and what was told;
- the watchlist and tiles.

An anchor that points at an imported row is re-pointed first, to the row that superseded it, through the existing `links` (`supersede`). Only then does the step run.

**Held by:**
- every account's units and cash after the step equal what they were before, on a book holding superseded and removed imported rows (`book/tests`);
- the step refuses while a `live` imported row exists;
- the person's data survives (brief 74, 1): on a book where journal entries, trade anchors, carried orders and brackets, notifications and the watchlist sit on trades built from imported rows that were superseded, every one of them is equal, field for field, after the step. A book where one can't be re-pointed is refused, named, and left as it was;
- once done, it's recorded and never runs again.

**The code, all of it.** Every reader and writer of `bagholder.db`, every carry, and every old-format type, from the deletion map of 2026-10-20:
- **server:**
  - `carry.rs`;
  - `legacy_import.rs`;
  - `legacy_orders.rs`;
  - `compare.rs` (the `compare-figures` and `import-book` commands);
  - the start's import and adoption of the Python app's file (`figures.rs` L393-398, `main.rs` adoption);
  - `reattach_carried_notes`;
  - Clear data's write to the old file;
  - `update.rs`'s `OLD_FILE`;
  - `engine_inputs::old_store_source`;
  - `demo_facts.rs`'s old-file parts;
  - the generated `book.ts` and `LegacyNote`.

  `carry_told_by_listing` doesn't touch the old file. It moves into `notify.rs` as the book-only repair it is.
- **book:**
  - `carry_notices` and `carry_following`;
  - `Book::import` and its types;
  - `import/old.rs`, including `OldActivity` and `OldSecurity`, which nothing reads once Part 2's migration has run;
  - `import/mapping.rs`'s `ImportMapping`.

  The rules the CSV reader shares (`rule`, `Qty`, `Cash`, `wealthsimple_account_type`, `row_leg`) move out of `import` to the module that uses them, under names that say what they are. Also removed: `import_orders`, `orphan_trade`, the `legacy_key` parameters and `reattach`.
- **store:** everything that serves only the old file:
  - `relabel`, `schema`, `activities`, `merge`, `csvimport`, `book`, `broker`, `admin`, `orders`, `connect`;
  - the old-only functions in `rows`, `tables`, `market`, `feeds` and `gens`.

  What remains is the market cache's SQL. It moves into `bagholder-sources` beside the cache's migrations, and the `bagholder-store` crate is removed.
- **market:** `refresh.rs` and the old quote functions in `quotes.rs`, which have no production caller.
- **sources:** `carry_filings`.
- **Tests and fixtures** for all of the above. The market tests are rebuilt on the cache's own migrations.
- **Docs:**
  - stage-3c and stage-6 sections marked historical;
  - `decisions.md` 2026-10-20 records the owner's words;
  - the 2026-09-25 Clear entry loses its old-file clause.

**Held by:**
- a source scan for exact identifiers, not stems (brief 74, 3): `"bagholder.db"`, `OLD_FILE`, `"bagholder-import"`, `IMPORT_SOURCE`, `ImportMapping`, `ImportedRow`, `OldActivity`, `OldSecurity`, `legacy_import`, `legacy_orders`, `from_old_store`, `carry_notices`, `carry_following`, `carry_market`, `carry_watch_folder`, `clear_old_store`, `relabel::`, `bagholder_store`, `explain_move`. It fails on any of them in `rust/`, `web/` or `.github/`, apart from this plan and the docs marked historical;
- the start never opening any file but `book.db` and the cache: a test lists the files a start opens.

**The old file itself** stays where it is, untouched, and Bagholder never opens it. The owner may delete it whenever they like.

## Order

1. **The demo written straight into the book** (in progress, its own PR): the e2e suite green on it.
2. **Part 1,** the check and its fix, then the owner's sync, and the count shows zero `live`.
3. **Part 2,** one PR: the migration and the deletion.

## Not included

Nothing else about statements or the cash withdrawals, which are already booked from statements.
