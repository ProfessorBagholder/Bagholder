# Plan: a change to the record does work in proportion to the change, never to the book

## For the owner to decide

Nothing open.

## Scope

This is the second plan of brief 15's Stage 3 ("light at the owner's size"). It takes on issues #420 (an added trade does work that grows with the whole book) and #419 (an import links every row any file ever imported against the whole ledger). Both are listed over budget in `capacity.rs` (`OVER_BUDGET`).

Every write to the record reaches the figures the same way today, whichever path made it (an added trade, an import, a pull, a removed row, a link, a trade's journal):

1. `Figures::record_changed` → `Figures::pulled` (`figures.rs:411–461`) reads the whole record from the book: `engine_inputs::ledger` (`engine_inputs.rs:24–58`) reads every account, every instrument with its names and terms, every live record, every problem, every transaction, every transfer link, every trade, group and journal entry.
2. It compares that whole ledger with the engine's (`e.inputs().ledger != ledger`) and hands it over whole (`Change::Ledger`).
3. The engine matches every lot of every holding again (`Engine::rematch`, `engine.rs`; `match_lots`, `ledger.rs:2135`) and rebuilds every trade, position, cash row, payer and broker check (`Engine::derive`), then compares every figure with the one before (`Engine::compare`).
4. `settle_trades` (`figures.rs:74`) reads every trade from the book (`book.trades()`) to give new round trips their ids.

An import then links each of the file's rows to the broker's own row for the same movement (`csv_import::link`, `csv_import.rs:327–398`): every live row any file ever imported, against every transaction in the ledger, a product of the two. Every pull runs the same linking again (`broker_reads.rs:421`), whether or not it stored a row.

All of this grows with the book: on master `d6824aa7` an added trade takes 3.9 million SQLite steps at four times the owner's size against 0.94 million at the owner's size, and about 730 ms on a Pi against a 100 ms budget (the table is under Verification).

The rule both issues break is `docs/architecture.md` §13: work is done because something changed, and only that work. The places it applies, by reading the code, and all fixed here:

| Where | What it does now | What it does after |
| --- | --- | --- |
| `Figures::pulled` / `record_changed` (every write to the record) | reads and compares the whole record | reads what the book's change log names since the engine last read it, by key |
| `Engine::apply(Change::Ledger)` | matches every holding again, rebuilds every figure | matches again only the holdings the change can reach, rebuilds their figures |
| `Engine::apply(Change::Adjustments)` | the same, for a corporate event's values | the same, for the holdings the event's legs name |
| `Engine::apply(Change::Closes)` on an underlying | matches every holding again | matches again only the underlying's family |
| `Engine::apply(Change::Trades / Groups / Journal)` | rebuilds every trade and position | rebuilds the ones the change names |
| `settle_trades` | reads every trade from the book | reads the trades of the round trips it settles, by key |
| `csv_import::link` after an import | every file row ever imported × every transaction | each row the import stored, looked up by its movement in an index |
| `csv_import::link` after a pull | the same, on every pull | each broker row the pull stored, looked up the same way |
| `entries::enter` (`entries.rs:167–171`, `:106`) | finds a transaction by scanning the ledger; finds an instrument by symbol by scanning every instrument | by key; by an index of current symbols |

Nothing on screen changes. `SPEC.md` does not change: every figure keeps its definition, and the engine's figures after any change stay exactly those of a fresh build (held below).

Out of scope, each its own issue: the day turning (`Change::Clock`) still matches everything once a day, since every contract's expiry is decided by it (not an operation that answers a person, and not in the growth rule's list); the stream step's whole views (#426); one signal waking every loop (#421); the book opened per call (#422).

## The old app here

The old app recomputed everything from its rows on every change, and the Rust build's engine carried that over: `docs/architecture.md` §13 says "a change to the record re-derives the book's figures in full, which takes milliseconds on a real book". That sentence was a guess, and it was wrong at the owner's size: the capacity job measures an added trade at far more than milliseconds on a Pi (Verification). It was also never the rule the same section sets ("only that work is done"). This plan changes that sentence in `docs/architecture.md` to the rule built here, and adds the mistake to `docs/old-app-mistakes.md` with the tests that guard it.

What is carried over, and why it is right:

- **First in, first out per account and instrument, in the record's order** (`ledger.rs`, its header): `SPEC.md` defines the lots that way. The plan does not change the match; it changes which holdings are matched.
- **A full build at a start and when the day turns**: nothing is held across a start but the book, and the day turning decides every contract's expiry, so a full match is the work those changes need.
- **The figures compared field by field to say what moved** (`Engine::compare`): the page's element-level updates read it. Kept, but run over the figures recomputed only.

## How the leading products do it

- **A trade added in a portfolio tracker updates that holding.** Sharesight files every trade under its holding ("Trades & Income" tab of the holding) and computes performance from each trade's traded date: "This is the date the buy or sell transaction occurred (and is the date used for performance and tax calculations)" ([Sharesight help, Setting up your holdings](https://help.sharesight.com/setting-up-your-holdings/), read 2026-10-12). A trade belongs to one holding; the holding is what its figures are worked out over.
- **An import matches only the rows it brings.** Tradervue flags an incoming execution as a duplicate when "another trade execution exists (from prior to the import) for the same symbol at exactly the same timestamp" ([Tradervue FAQ](https://app.tradervue.com/help/faq), read 2026-10-12): each new row is looked up against what is there, not every past row against every other.
- **Incremental view maintenance.** The database field's standard for keeping derived results current is to process the change, not the whole input: DBSP gives "efficient incremental view maintenance techniques" for rich query languages ([Budiu, McSherry, Ryzhyk, Tannen, DBSP, arXiv:2203.16684](https://arxiv.org/abs/2203.16684), read 2026-10-12). Its building block applies here directly: a computation partitioned by a key needs to be redone only for the keys a change touches.
- **Change capture in the store.** SQLite's own change capture, the session extension, works per connection and must be attached before the writes it records: "A single session object monitors changes made to a single database ... via a single sqlite3* database handle" ([SQLite, The Session Extension](https://www.sqlite.org/sessionintro.html), read 2026-10-12). The book is written by several connections (the server's, a pull's, an import's), so a session per connection would miss writes it was not attached to. A trigger fires as part of the statement that makes the write, on whichever connection ([SQLite, CREATE TRIGGER](https://www.sqlite.org/lang_createtrigger.html), read 2026-10-12), so a change log kept by triggers records every write, in the same transaction, from every writer the book has or will have. This is the change-table (outbox) form of change data capture, chosen for that reason.

## Open questions

None. The objective is that an added trade and an import do work in proportion to what they change. What settles the approach:
- the code paths in the Scope table, read in full;
- the owner's book: 7,151 live transactions on 191 instruments, 3,296 of them with no instrument (cash moved); the largest single instrument has 349 (a copy of `~/.bagholder-rust/book.db`, counted with `sqlite3` on 2026-10-12). Matching one instrument's holdings again is about a twentieth of the book at worst, and far less for most;
- the growth rule's list in `docs/architecture.md` §16, which names "an added trade's write" and "an import's linking".

## Approach

### A. The book keeps a log of what changed in the record (book migration 028)

A table `ledger_changes (seq INTEGER PRIMARY KEY, kind TEXT NOT NULL, key TEXT NOT NULL)` and, on each table the engine's ledger is read from, `AFTER INSERT / UPDATE / DELETE` triggers that append the key the row belongs to:

| Kind | Key | Tables whose triggers write it |
| --- | --- | --- |
| `record` | the record's id | `source_records`, `transactions`, `record_problems`, `transfer_links` (both sides' records) |
| `instrument` | the instrument's id | `instruments`, `instrument_joins` (one merged into another), `instrument_sightings` (its names), `option_terms` |
| `account` | the account's id | `accounts`, `broker_connections` (its accounts) |
| `trade` | the trade's id | `trades` |
| `group` | the group's id | `trade_groups`, `trade_group_members` |
| `journal` | the subject | `journal`, `journal_tags` |
| `adjustment` | the transaction it applies to | `adjustments`, `adjustment_legs` |

The tables are listed by the test, not by hand: a test reads every table `engine_inputs::ledger` and `facts().adjustments` read (through `work::statements_naming`, the counter built for brief 25) and fails on one with no trigger.

The engine's holder (`Figures`) keeps the highest `seq` it has applied. At a start the engine is built whole, as now, from the book as it stands at `MAX(seq)`, and rows at or below that are deleted (the server is the log's only reader; a tool that opens the book reads it whole and does not consume the log). A change reads `SELECT kind, key FROM ledger_changes WHERE seq > ?`, deduplicated, and then reads each named thing by its key through indexed readers the book gains beside its whole ones (`Book::record_transactions(&[RecordId])`, `Book::instruments_by_id`, `Book::trades_by_id`, …). One write to the record is then a handful of indexed reads, whatever the size of the book.

Why a log and not the writer naming what it wrote: the record has many writers (`records.rs`, `identity.rs`, `statements.rs`, `trades.rs`, `links.rs`, `clear.rs`, the import, the pull), and more will come with another broker. A writer that forgets to name a change leaves the figures wrong with nothing to show it. A trigger cannot be forgotten, and it is written in the transaction that makes the change, so a write that rolls back leaves no entry (held by a test).

### B. The engine is told the change, keyed, and matches again only what it reaches

`Change::Ledger(Ledger)` becomes `Change::Ledger(LedgerDelta)`: for each keyed collection of the ledger, the entries put (new or revised) and the keys removed. `Change::Trades`, `Groups` and `Journal` become parts of the same delta (they are read in the same write already, `figures.rs:431–446`), so every part of `Inputs` is still changed by exactly one variant. The ledger's transactions are held by record (`BTreeMap<RecordId, Vec<Transaction>>`) with an index by transaction id, so a delta is applied by key; the 33 readers of `ledger.transactions` (engine and server) read it through an iterator in the record's order, which is what they read now.

**What a change can reach.** The match couples holdings in a known, finite set of ways, each read from `ledger.rs`:
- every account's holdings of one instrument share its coin places and its last stated price (`Matcher.places`, `Matched.last_price`);
- a contract and its underlying (assignment, exercise, multi-leg orders tainting by underlying: `underlying_taint`);
- the instruments a corporate event's legs name (`event_groups`, `adjustments`);
- the two sides of a transfer between accounts (`transfer_links`);
- every holding of an account, where a multi-leg order names no instrument (`account_taint`).

The engine keeps these as a disjoint-set over instruments, plus the accounts made whole by an account-wide taint: a **family** is a set of instruments joined by any of the first three, extended by the fourth across accounts. Transactions with no instrument (cash moved) form one family per account; they affect the cash totals and cash rows only. A change reaches the families of every transaction, instrument and adjustment it names, before and after the change (a revision can move a transaction from one family to another). The families are recomputed for what changed and merged or split as links come and go.

**Matching again.** `match_lots` takes the set of families to match and runs exactly as today over their transactions only, in the same order; its result replaces those families' entries in `Matched`. To make that a replacement by key, every collection in `Matched` that is a list today (`beyond`, `dust`, `arrived`, `stock_dividends`, `disagreements`, `unit_disagreements`, `unapplied`) becomes a map by the transaction (or holding) it is about, and the engine keeps, per family, the keys it produced. Each account's cash totals (`Matched.cash`) are kept per (account, family) and summed per account, so a family's change moves its own share.

**Deriving again.** `build_trades`, `build_positions` (which already takes an instrument), `build_cashflow`, `payer_rates`, `identify` and `broker_checks` (which already takes an account) take the families (or their trips, holdings, transactions and accounts) to build; the engine's collections (`trades`, `positions`, `cash`) are held by key, and only the recomputed entries are compared to say what moved. `Entity::Book`'s figure (accounts, instruments, what waits on the person, the count of transactions) is updated by key.

**The same for the other changes that matched everything:** `Change::Adjustments` reaches the families of the transactions the changed adjustments apply to; `Change::Closes` of an underlying reaches that underlying's family.

### C. Trade ids are settled by key

`settle_trades` reads the trades of the round trips the change produced or moved (`Book::trades_by_id`, `Book::trades_on`), not every trade, and `Identity` is updated for the recomputed families only.

### D. An import, and a pull, link only the rows they stored

The engine keeps an index of the record's movements: for each fill or movement on an instrument, the key `csv_import::candidates` matches on (account, instrument, kind, quantity, trade day, and for a movement its cash and settle day), to the records holding it, maintained by the same delta. `link` then takes the records the operation stored:
- after an import, each row the file stored is looked up in the index for broker rows of the same movement (the feed's before the statement's, as now);
- after a pull, each broker row the pull stored is looked up for live file rows of the same movement.

The matching rule itself (the price to the places the file states, the tiers, a broker row taking one file row only, ambiguity reported) is unchanged: it is `SPEC.md`'s, and only which rows are compared changes. A pull that stored nothing links nothing.

### E. An added trade finds what it names by key

`entries::enter` reads a transaction by its id through the ledger's index, and an instrument by symbol through an index of current symbols the engine keeps with the instruments (`held_by_symbol`, `contract_by_terms`), maintained by the same delta.

### What stays the same

What the figures are, the order the match applies transactions in, the brief-22 rule that a pull's rows and statements are applied in one write under one version (`Figures::pulled` keeps its shape: it reads the log instead of the whole record), and the full build at a start.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green; `cargo clippy --workspace --lib --bins` clean.
- [ ] **Equal to a fresh build, for every kind of change.** A property test over generated books (accounts of several kinds; shares, coins, contracts on underlyings; transfers between accounts; corporate events with their adjustments; a multi-leg order naming no instrument) applies random sequences of writes through the book's own writers (insert, revise and remove a record, back-dated and today; add and remove a transfer link; set and clear an adjustment; open, move and orphan a trade; a journal entry), reads the change log after each, and holds `Engine::differences` against `Engine::build` from the book empty after every step. Also held with the existing `each_writer_leaves_the_engine_as_a_fresh_build_would`.
- [ ] **What moved is what changed.** For the same sequences, the `Moved` an incremental apply reports equals the difference between the fresh builds before and after (no figure reported that did not change, none missed).
- [ ] **The log records every writer.** A test reads, through `work::statements_naming`, every table the whole ledger read touches, and fails on one without a trigger; a write rolled back leaves no entry; a write through a second connection is in the log.
- [ ] **No change handler reads the record whole.** The boundary test of brief 22 (`only_a_start_reads_the_market_and_every_account_whole`) gains `ledger`, `transactions`, `trades` (the whole readers) with only the start's build and the comparison tool allowed to call them.
- [ ] **Linking reads only what was stored.** A test counts the rows `link` compares: an import of N rows into a book of any size compares N rows with their index lookups; a pull that stored nothing compares none.
- [ ] **The `Capacity` workflow:** `add-trade` and `import` meet the growth rule, then their budgets (100 ms answered; 1 s for a small import, on a Pi), and are removed from `OVER_BUDGET`; #419 and #420 closed by the PR. The job's table pasted in the PR.
- [ ] **The owner's book:** a copy migrates (028); `Engine::build` before and after the migration gives the same figures (the determinism digest); an added trade and a 20-row import on the copy, timed and counted, pasted in the PR.
- [ ] `docs/architecture.md` §13's sentence on a change to the record replaced by the rule built here; `docs/old-app-mistakes.md` gains the entry.

## Surfaces to check beyond the diff

- Book migration 028 and its committed `schema/v28.sql`; `Book::clear` (`clear.rs`) empties the log with the record.
- Every reader of `ledger.transactions` (33 sites across `engine/src` and `server/src`: `ledger.rs`, `needs.rs`, `scope.rs`, `cashflow.rs`, `engine.rs`, `clear.rs`, `status.rs`, `entries.rs`, `csv_import.rs`, `wire/build.rs`).
- `Engine::past` builds a past day's holdings from a filtered ledger; it reads the new shape.
- `compare.rs` and `read_sources.rs` build engines whole; they keep doing so.
- The capacity probes `add-trade` and `import` (`capacity.rs:584–606`) measure the whole operation, write included; unchanged.
- `docs/decisions.md` 2026-10-09 (a pull's rows and statements in one write): its test still holds.

## Right to refuse

If matching a family alone cannot equal a full match for some coupling not listed above (the property test would show it), that coupling is added to the families, never worked around; if one makes every family one (a coupling across all instruments), the plan stops and says so before building further.

## Anti-stub self-check

To initial at build: no delta field nobody applies; no trigger whose kind nobody reads; no family the engine keeps that the match does not use; the suite run, the capacity job run, the owner's copy migrated and measured, not asserted.

## Verification

Before the build: master `d6824aa7`, `target/release/bagholder capacity run` on this Mac (arm64), at the owner's size and four times it, 2026-10-12.

| Operation | This Mac (ms) | Pi (ms) | Budget (ms) | VM steps ×1 → ×4 | Allocations ×1 → ×4 |
| --- | --- | --- | --- | --- | --- |
| add-trade | 347.1 | 729.0 | 100 | 938,375 → 3,896,785 (4.15×) | 814,754 → 3,220,728 (3.95×) |
| import (20 rows) | 638.8 | 1,341.5 | 1,000 | 1,343,091 → 5,503,460 (4.10×) | 841,420 → 3,288,393 (3.91×) |

Both grow as the book does (the growth rule allows under 2×), and both miss their wall-time budgets on a Pi. The step and allocation counts do not depend on the machine (`docs/architecture.md` §16); the milliseconds are this Mac's, scaled by `PI_SLOWDOWN`. The same run on this Mac also reported `startup` and `brokers` over budget, while the Linux runner passes both on the same commit; neither is this plan's, and the build's PR reads the runner's table for them.

## Handoff

Not started; this is the plan. To resume: build A first (the log and its tests), then B behind the equality property test, then C–E.

**Nothing left running.**
