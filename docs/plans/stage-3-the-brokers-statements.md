# Plan: what the broker states is kept when it changes and read by account, and a price change reads only its own instrument

## For the owner to decide

Nothing open.

## Scope

This is the first plan of brief 15's Stage 3 ("light at the owner's size"). It takes on the cause behind five of the six operations the capacity job lists over budget: `brokers`, a price change, startup, and the broker read inside every added trade and import (issues #417 and #418).

Today, every price change rebuilds the market inputs whole (`figures.rs:477`, `market()`), and `market()` reads what the broker states about every account (`engine_inputs::brokers`). For each account, `Book::stated` (`book/src/statements.rs:367`) does three things:
- It scans every balances row of every account: the `statements` table has no index on the account.
- It parses every one of that account's cash statements, to find the newest.
- It reads `account_values` and `buying_power` newest-first by row, with no index on the account.

Meanwhile every balances read appends a row, whether or not anything changed. That is about 890 rows a day at the owner's size; a copy of the owner's book holds 10,624 cash statements after eleven days.

The capacity job measured the result (CI run 37966416194, ubuntu-24.04-arm):

| Measure | At the owner's size | At four times it |
| --- | --- | --- |
| SQLite steps for one price change | 66 million | 951 million |
| Bytes read for one price change | 4 GB | 51 GB |
| A price change, scaled to a Pi | | 67 s |

The plan changes three things:

- **A.** A balance the broker states again unchanged extends the period it already covers instead of adding a row.
- **B.** Each account's newest statement is read through an index.
- **C.** Each kind of change reads only what it changed: one instrument's prices, one account's statements.

Nothing on screen changes. `SPEC.md` does not change: it defines what the figures are, not how the statements are stored, and every figure stays the same (held by tests below).

Out of scope, each in its own plan next:
- An added trade's and an import's whole-ledger read (`record_changed` reads `engine_inputs::ledger` whole), issues #419 and #420.
- The stream step's whole views, issue #426.
- One signal waking every loop, issue #421.
- The book opened per call, issue #422.

## The old app here

The old app had no broker statements: it read the cash from its own rows, so there is nothing of its to carry over. The current design is the Rust build's own (`docs/plans/stage-3b-wealthsimple.md`, "The book's new tables"). Its rule is "a later statement is kept beside an earlier one, never over it, and the newest is the one read". That rule was right about history, which must not be overwritten. It was wrong in taking "kept" to mean "a row per read". A read that states what the last one stated adds no fact beyond the moment it was asked, and `broker_reads` already keeps that moment. `docs/old-app-mistakes.md` gains an entry for this build's own mistake, with the test that guards it.

## How the leading products do it

- **History of a value that changes rarely.** The Kimball Group's Type 2 slowly changing dimension adds a new row only when an attribute changes, each with "a row effective date or date/time stamp" and "a row expiration date or date/time stamp" ([Kimball Group, Type 2: Add New Row](https://www.kimballgroup.com/data-warehouse-business-intelligence-resources/kimball-techniques/dimensional-modeling-techniques/type-2/), read 2026-10-10). This plan keeps each statement that way: a row per value, with the first and last moment the broker stated it. The moments in between are in `broker_reads`, as they are now.
- **Finding the newest row for one account.** SQLite's query planner documents a lookup on a column with no index as a full table scan, which "reads every row out of the table". With an index it does a binary search, at a cost "proportional to (K+1)*logN" ([SQLite, Query Planning](https://www.sqlite.org/queryplanner.html), read 2026-10-10). The same page says that every index entry carries the row's rowid, which breaks ties.
- **A balance is current state, read by account.** Broker and aggregator interfaces return an account's balance as one current value per account, not a list of past reads (Wealthsimple's own accounts query states `currentCombined` per account, recorded in `rust/crates/wealthsimple/tests/replies`). Trading journals keep no balance history of their own beyond what the broker's statements give. The app keeps one because its broker check needs the cash as it stood at the last full read of the activity (`SPEC.md` §4, the broker check).

## Open questions

None. The objective is to bring `brokers`, a price change and startup within budget and the growth rule. The approach is settled by:
- the measurements above;
- the code paths named under Scope;
- the decision of 2026-10-09 ("a cash difference only when a statement after the activity read states the same cash"), which the period form answers directly: the period's last moment is after the read.

## Approach

**A. A statement kept as a period (book migration 026).**

- `statements` (cash), `buying_power` and `account_values` each gain `first_us INTEGER NOT NULL` and `last_us INTEGER NOT NULL`: the first and last moment the broker stated that value, in microseconds since the epoch. These sort as time, unlike the text instants (issue #442, for these tables). They also gain `last_read_id`, the read that last stated it. `stated_at` stays as written: the first moment, as text.
- `Book::store_cash`, `store_buying_power` and `store_net_value` compare the new statement with the account's newest one:
  - A cash statement compares the cash in every currency and the holds it carries (`statement_holds`).
  - Buying power compares the amount or the reason it is unavailable.
  - Net value compares the amount.
  - Equal: the newest row's `last_us` and `last_read_id` move forward.
  - Different: a new row, with `first_us = last_us` = this moment.
- **Migration 026:**
  - fills `first_us` and `last_us` from the text instants;
  - folds each run of consecutive equal statements of one account, in the order written, into its first row, whose `last_us` and `last_read_id` become the run's last;
  - deletes the repeats' rows, with their `statement_cash` and `statement_holds` rows.
  No value is lost: each distinct statement stays with its period, and every read stays in `broker_reads`. On a copy of the owner's book, 10,624 cash statements fold to 5,738.
- **Indexes:** `statements (account_id, kind, first_us)`, `buying_power (account_id, first_us)`, `account_values (account_id, first_us)` and `broker_reads (connection_id, part, at_us)` (at_us added the same way).

**B. `Book::stated(account)` read by index.**

- The newest cash: the account's cash row with the greatest `first_us`, ties broken by the row written last (the module's existing rule), one indexed lookup.
- The cash at the last full read of the activity: the period whose `first_us` is at or before the read, the latest such, one indexed lookup. Its moment is the latest moment at or before the read that the broker stated it: `last_us` if that is at or before the read, else the latest cash read of that connection between `first_us` and the read, from `broker_reads` (indexed). This is the same moment the scan finds today.
- Net value now and buying power: the newest period, one indexed lookup each.
- `account_days` is already keyed by account and day (its primary key). It is read whole for an account only when that account's statements change (C).
- `Stated` and `BrokerAccount` keep their fields and meanings. No engine type changes.

**C. Each change reads only what it changed.**

- `Figures::price_changed(instrument)` reads that instrument's quote, ex-dividend date and closes by key, through new keyed readers on `MarketCache`: `quote_of`, `ex_dividend_of` and `closes_of`, beside the existing whole readers (`sources/src/cache.rs:242, :280, :324`), which startup keeps. It no longer reads the book or any broker statement.
- `Figures::benchmark_changed(key)` reads that one series the same way.
- `Figures::broker_changed(account)` reads `stated(account)` alone, not every account's (`figures.rs:456` today builds every account's and keeps one).
- `Figures::record_changed` no longer reads the broker's statements. A pull that stores a statement or notes an activity read calls `broker_changed` for each account it touched (`broker_reads.rs:499` already does for balances; the pull's activity reads join it). A test holds that every writer of a statement or an activity read is followed by `broker_changed` for that account.
- `market()` stays as the whole read at startup only. A test (`tests_boundary`) fails if any change handler calls it.

What stays the same:
- every figure, every screen, every cadence;
- the engine's inputs and `Change` kinds;
- `account_days`;
- the order of statements written at one instant.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green; `cargo clippy --workspace --lib --bins` clean.
- [ ] **Migration 026 on populated books:** a book at version 025, built by `capacity build` at a small and the owner's size, migrates. Then, for every account, `Book::stated` returns exactly what it returned before, every field equal. So do the engine's figures (the determinism digest), before and after.
- [ ] **Folding:** a run of N equal balance reads stores one row, whose period spans the first to the last read; a read that differs adds one. The test uses generated sequences, including equal values with different holds, and two statements at one instant.
- [ ] **The broker check (decision 2026-10-09):** the existing tests for a cash difference stated after the activity read pass unchanged.
- [ ] **No change handler reads the whole market or every account:** `price_changed`, `benchmark_changed`, `broker_changed` and `record_changed` held by a boundary test.
- [ ] **The `Capacity` workflow:** `brokers` and `price-changed` meet their budgets and the growth rule, and are removed from `OVER_BUDGET`. `startup` meets its 5 s budget on the Pi scale or stays listed with the issue naming what remains. `add-trade` and `import` stay listed under #419/#420 with their remaining cost shown. The job's table is pasted in the PR.
- [ ] **The owner's book:** a copy migrates; its folded row count is stated in the PR, and `stated` agrees for every account before and after.

## Surfaces to check beyond the diff

- `book/tests/schema.rs` and the schema snapshot;
- `broker_reads.rs` (every writer of a statement);
- `capacity.rs` (`build` writes balances through the same store calls, so its books fold too);
- `docs/old-app-mistakes.md`;
- `docs/decisions.md` 2026-10-09 (the broker check's rule);
- `server/src/clear.rs` (Clear of Wealthsimple records deletes these tables whole; unchanged).

## Right to refuse

If folding changes any figure or any broker check on a populated book, the migration is wrong and nothing is built until it isn't.

## Anti-stub self-check

_To be initialled at build._

## Verification

_To be filled at build._

## Handoff

_To be filled at build._

**Nothing left running.**
