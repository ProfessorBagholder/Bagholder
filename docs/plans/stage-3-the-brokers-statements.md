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

## Brief 22's required changes, applied

The reviewer's verdict (brief 22, Go with changes) is applied throughout the build; each item below replaces what the approach above says where they differ.

1. **The engine's moment for a statement is its period's last moment.** `Stated.cash`, and so `BrokerAccount::as_of`, is the newest period's `last_us`, as the newest row's instant is today. Held by a unit test of the brief's sequence (100 at t1, a full read at t2, 100 again at t3, the book at 90) through `stated` and `broker_checks`, and the generated books contain folded runs that straddle an activity read.
2. **Periods are runs in (instant, write) order, never in write order alone.** A period's `first_us` is its least instant and `last_us` its greatest. A statement whose instant is not after the newest period's last moment is placed by its instant: it extends the period it falls in or beside where it states the same, else it is a row of its own; a period it falls inside with another value is split at it, the moments either side read from that connection's cash reads (`broker_reads`). Nothing moves a moment backwards. Two different statements at one instant are both kept, the one written last the newer, which is what today's rows give (net value's `INSERT OR REPLACE` gives the same answer). Held by a property test: random non-monotone sequences with ties, the period store's `stated` equal at every read instant to a reference model of today's rows (sorted by instant, then write order).
3. **Equal means the same cash in every currency and the same holds after `store_cash`'s nulling**, compared field by field with a missing amount equal only to a missing amount. The folding test includes a run whose only change is a hold's amount becoming unknown.
4. **A balances read recomputes one account's check, in work that does not grow with the book.** `Change::Broker` recomputes the check of its account only, from a per-account cash total the engine keeps when the ledger changes, and the account's units through a range of `matched.units`. The `brokers` probe times `broker_changed(account)`, read and apply; acceptance is the growth rule, then the budget.
5. **A pull's rows and its statements reach the engine in one write.** The pull reports the accounts it stated anything for; `record_changed(now, touched)` reads those accounts' statements and applies them with the ledger under one lock and one version. `broker_changed(account)` stays for the balances read alone. Held by a test that a pull writing rows and statements moves `Figures::version` by one, listed under the 2026-10-09 decision.
6. **The writers are held by an oracle, not a list.** After each of `pull`, `pull_activity` and `balances` in the server's tests, the engine's `inputs.market.brokers` equals `engine_inputs::brokers(&book)`.
7. **Units statements:** `first_us` and `last_us` are nullable with the same check `stated_at` has (cash only). A units statement is read newest through an index on `(account_id, kind, as_of_day)`. It is not folded: its lines carry the broker's value of each holding, which moves with the price every day, so two in a row are rarely equal. The PR states the count on the owner's copy.
8. **Every instant the check hinges on is read as an integer.** `broker_reads` and `activity_reads` gain `at_us`; `last_read` and `activity_read_at` read `MAX(at_us)` through indexes; the text columns are written from the same instant and never read again, held equal on the migrated books by the migration test.
9. **A balances read does not read the account's day history.** `broker_changed(account)` reads cash, buying power, net value now and the activity read; the days are read only when a pull touched the account (5), and the engine keeps the ones it has otherwise.
10. **The migration rebuilds the tables** through new tables and a rename (the runner turns foreign keys off for it and checks them after), commits `schema/v26.sql`, deletes the folded rows' `statement_cash` and `statement_holds` with them, and the PR states what it took on the owner's copy.
11. **The boundary test names the whole readers' only callers:** `figures::build`, `read_sources::engine`, `compare.rs` and the capacity `startup` probe; any other caller of `market`, `market_from_cache` or `engine_inputs::brokers` fails it.

`Stated.cash_read` keeps only the cash and the holds: its moment is read nowhere (`engine_inputs.rs` drops it), so the field's moment is removed.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green; `cargo clippy --workspace --lib --bins` clean.
- [ ] **Migration 026 on populated books:** a book at version 025, built by `capacity build` at a small and the owner's size, migrates. Then, for every account, `Book::stated` returns exactly what it returned before, every field equal. So do the engine's figures (the determinism digest), before and after.
- [ ] **Folding:** a run of N equal balance reads stores one row, whose period spans the first to the last read; a read that differs adds one. The test uses generated sequences, including equal values with different holds, and two statements at one instant.
- [ ] **The broker check (decision 2026-10-09):** the existing tests for a cash difference stated after the activity read pass unchanged.
- [ ] **No change handler reads the whole market or every account:** `price_changed`, `benchmark_changed`, `broker_changed` and `record_changed` held by a boundary test.
- [ ] **The `Capacity` workflow:** `brokers` and `price-changed` meet the growth rule, then their budgets, and are removed from `OVER_BUDGET`. `startup` meets its 5 s budget on the Pi scale or stays listed with the issue naming what remains. `add-trade` and `import` stay listed under #419/#420 with their remaining cost shown. The job's table is pasted in the PR.
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

Initialled at build (Claude): no definition nobody references; no field written and never read (`Stated.cash_read`'s moment, read nowhere, is removed); no branch only the switch knows. The suite was run, the migration was run on a copy of the owner's book and compared with it account by account, and the capacity run was made at the owner's size and four times it.

## Verification

### Built differently from the approach above, and why

- **Moments are nanoseconds and the read's place, not microseconds.** Instants are kept as nanoseconds (`first_ns`, `last_ns`, `at_ns`), the precision the book writes them at, so no two instants it holds apart become one. Two statements at one instant are told apart by their reads' places in `broker_reads` (`first_seq`, `last_seq`), never by the row that holds them: the property test showed a split period's later half, written as a new row, overtaking statements made at the same instant before it.
- **Buying power and value now are ordered by instant too.** Today's rows read the newest of these two by the row written last; cash by instant. One rule now holds for all three: by the read's instant, then the order the reads were written in. They differ only when the machine's clock is set back.
- **`Book::stated_latest(account)`** reads the newest of each kind without the day history (`Book::stated` adds the days); the pull's check for cash moved since the activity read uses it.
- **A quote's change rechecks only the accounts that hold the instrument** (the book's positions in it, or the broker's statement of it), as a statement's change rechecks only its account: the quote change recomputed every account's check, the same class as brief 22's change 4, and the capacity run showed its allocations growing with the book.
- **The capacity build writes a year of balances at the owner's rates of change.** With statements kept as periods, a made-up broker whose balances never move folds to one period per account and stops resembling the owner's book; and storing every unchanged read for a year at four times the owner's size took over half an hour. The build now keeps every read (as the app does) and stores a statement where an account's cash, value or buying power changed, at the rates counted on a copy of the owner's book (`BALANCES_READS`, `CASH_CHANGES`, `VALUE_CHANGES`, `POWER_CHANGES` in `capacity.rs`); the last read states every account as the broker does. The owner-size build takes 17 s.
- **Units statements are indexed, not folded** (brief 22, 7): the owner's copy holds 161 over 12.6 days, one per account per full pull, each with the broker's value of every holding, which moves with prices.

### Commands and numbers

- `RUSTFLAGS="-D warnings" cargo test --workspace` (in `rust/`): every test binary passed. `cargo clippy --workspace --lib --bins`: clean.
- New tests, each shown failing without its change where it holds one: `book/tests/statements.rs` `statements_kept_as_periods_answer_what_a_row_per_read_answered_for_any_order_of_instants` (60 seeds, checked after every statement; it found the tie ordering above), `a_cash_stated_again_unchanged_extends_its_period_and_is_stated_at_its_last_moment`, `a_statement_whose_only_change_is_a_hold_becoming_unknown_is_a_period_of_its_own`; `book/tests/schema.rs` `migration_26_keeps_what_every_account_s_statements_answer` (40 generated version-25 books); `server/src/capacity.rs` `test_a_pull_reaches_the_figures_in_one_write_and_every_read_leaves_them_as_the_book_states`, `test_a_difference_the_broker_states_again_after_the_read_is_confirmed_through_a_folded_period` (fails with the period's first moment: `pending: true`); `server/src/tests_boundary.rs` `only_a_start_reads_the_market_and_every_account_whole`.
- **The owner's book** (a copy of `~/.bagholder-rust`, opened by the release build): migration 26 and the first figures in 0.64 s. Cash statements 10,656 → 87 periods; value now 3,994 → 824; buying power 664 → 535; units statements 161, unchanged. Compared by SQL before and after, for every account: the newest cash and its moment, the cash and holds at the last full read of the activity, the newest buying power and value now: identical.
- **`bagholder capacity run`** on an M-series Mac (the CI job's arm64 table is in the pull request), the owner's size and four times it:

  | Operation | Here at ×4 (ms) | Pi (ms) | Budget (ms) | VM steps ×1 → ×4 | Allocations ×1 → ×4 | Before this plan, here at ×4 (ms) |
  | --- | --- | --- | --- | --- | --- | --- |
  | startup | 1,103 | 2,316 | 5,000 | 2.0M → 8.3M | 0.85M → 3.3M | 25,135 |
  | add-trade | 333 | 698 | 100 | 0.94M → 3.9M | 0.81M → 3.2M | 25,448 |
  | import | 639 | 1,341 | 1,000 | 1.3M → 5.5M | 0.84M → 3.3M | 23,345 |
  | brokers | 2.4 | 5.1 | 2,586 | 1,259 → 1,321 | 2,862 → 1,084 | 20,874 |
  | price-changed | 1.0 | 2.0 | 95 | 366 → 366 | 87 → 86 | 19,513 |
  | feed-step | 48 | 101 | 95 | 1,606 → 1,606 | 148K → 712K | 49 |

  `startup`, `brokers` and `price-changed` are off `OVER_BUDGET`. `add-trade` (#420) and `import` (#419) stay listed: each rebuilds the ledger whole, the next plan. `feed-step` (#426) stays listed.

## Handoff

- Issues #417 and #418 are closed by this change. `OVER_BUDGET` holds `import` (#419), `add-trade` (#420) and `feed-step` (#426), each the next plan in brief 15's Stage 3.

**Nothing left running.**
