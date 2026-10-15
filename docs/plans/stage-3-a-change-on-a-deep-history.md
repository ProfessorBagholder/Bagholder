# Plan: a change on an instrument with a long history does work in proportion to the change, not to the history

## For the owner to decide

Nothing open.

## Scope

This is the third plan of brief 15's Stage 3 ("light at the owner's size"), and the step brief 27 set under "Depth: the gate's call". It finishes issues #420 (an added trade) and #419 (an import) on the gate's depth pair, where both stay listed in `OVER_BUDGET` (`capacity.rs`) after #507.

The record-by-change build (#507) made a change cost work in proportion to the **families** it reaches (the holdings a change can move: `docs/plans/stage-3-the-record-by-change.md`, B). A family is the whole history of an instrument across accounts, so a trade added to an instrument with a long history still does work in proportion to that history. Measured on master `0bc73874` (this Mac, arm64, 2026-10-13), on the depth pair's books: the owner's book with one listing 358 rows deep, and the same with it 1,406 rows deep. The probe adds one trade, or imports 20 rows, on that listing. The breakdown comes from temporary counters at each step (not committed).

| Step of an added trade | SQLite steps, 358 → 1,406 rows | Allocations, 358 → 1,406 rows |
| --- | --- | --- |
| The write itself (the book) | 5,525 → 5,525 | 354 → 354 |
| Reading the change: the instrument's names (`Book::names`, in `engine_inputs::delta`) | 10,404 → 39,748 | 2,594 → 9,930 |
| Matching the family again (`match_over`) | 0 | 10,137 → 37,766 |
| Working out the figures again (`Engine::rederive`), after the trade | 0 | 11,363 → 44,435 |
| The same again, after `settle_trades` opens the new round trip's trade | 0 | 11,376 → 44,449 |
| **Whole operation** | **16,729 → 46,073 (2.75×)** | **36,161 → 137,604 (3.81×)** |

An import of 20 rows has the same three costs (names 13,482 → 42,826 steps; match 10,561 → 38,191 allocations; the figures twice, 11,959 → 45,032 and 12,076 → 45,150), 52,440 → 81,780 steps and 45,862 → 147,324 allocations in all. The growth rule allows under 2× (`docs/architecture.md` §16). On the runner, scaled to a Pi, an added trade at the deeper size is 109.4 ms against a 100 ms budget (#507's table).

The rule they break is the one #507 built (`docs/architecture.md` §13): only the work a change makes is done. Each place it applies, from the code:

| Where | What it does now | What it does after |
| --- | --- | --- |
| `Book::names` (`book/src/identity.rs:433`), read for every instrument a change names | folds every live sighting of the instrument into its runs of one symbol and venue | reads the runs, which the book keeps as rows, maintained where sightings and records change (A) |
| `Book::current_symbol` (`identity.rs:409`) | finds the newest sighting of each member, then falls back to the whole fold | reads the last kept run (A) |
| `Engine::rematch_families` → `match_over` (`engine.rs:386`, `ledger.rs:2161`) | matches every row of each reached family from its first | for rows placed after everything the family has matched, continues the family's match from its kept state; otherwise matches the family whole, as now (B) |
| `Engine::rederive` (`engine.rs:813`): identity, round trips, groups | every round trip of every reached instrument, and every group | the round trips the match changed or the change names, and the groups holding them (C) |
| `Engine::rederive`: a trade's change (`delta.trades`, `delta.journal`) | reaches the trade's whole instrument | reaches the trade's round trip (C) |
| `Engine::rederive`: the payers' rates | filters every cash row of the book for each payer reached | reads that payer's rows through an index, and only where its payments or its units changed (C) |

Nothing on screen changes. `SPEC.md` does not change: every figure keeps its definition, and the engine after any change stays equal to a fresh build (held below).

Out of scope, each with its reason:
- A row placed **before** rows the family has already matched (a back-dated trade, a correction, a removal) still matches the family whole. Every later lot can change with it, so the work from that row onward is the change's own; starting from the family's first row instead of from that row is the part over. Doing that would need the family's state kept at past points, and the gate's probes add today's rows. It is issue #512, for the gate to schedule.
- The day turning (`Change::Clock`) still matches everything once a day, as #507 left it.
- Issues #508, #509, #426, #421 and #422.

## The old app here

The old app recomputed everything on every change; #507 replaced that with recomputing the families a change reaches. Nothing of the old app is carried over here. What this plan carries over from #507, and why it is right:

- **The match itself** (first in, first out per account and instrument, in the record's order; `ledger.rs`, its header): `SPEC.md` defines the lots that way. Resuming it changes where the walk starts, never what it does at each row.
- **The whole-family match as the fallback**: it is what #507 proved equal to a fresh build. Every case this plan does not continue goes through it unchanged.
- **The fold of sightings into names** (`Book::names`, `book/src/identity.rs:401–470`): a name is each unbroken run of live records' sightings under one symbol and venue, from its first day to its last, so a ticker that changes and changes back is three names. That is what a renamed listing is (what it was called, and when), and the page reads it that way. The kept runs are that definition stored; the fold stays as the test oracle.

## How the leading products do it

- **Inventory systems keep lots per item and recompute forward from a change.** A trading journal's lots are inventory layers. ERPNext's FIFO valuation says inserting or editing a past entry means it must "recompute the quantity, rate, value, and GL impact of every subsequent stock ledger entry for that item-warehouse" ([ERPNext, FIFO and Moving Average](https://docs.frappe.io/erpnext/fifo-and-moving-average), read 2026-10-13). The work is per item, and it covers the entries after the change, never the ones before it. NetSuite does the same per item: "A back-dated inventory adjustment recalculates inventory costing only for the items on the adjustment" ([NetSuite, Inventory Costing Recalculations on Inventory Adjustments](https://docs.oracle.com/en/cloud/saas/netsuite/ns-online-help/bridgehead_N2198841.html), read 2026-10-13). A new receipt or sale after every existing entry recomputes nothing before it. That is case B. A back-dated one recomputes forward from its date; that is the out-of-scope case above, where this plan recomputes from the family's first row.
- **Event-sourced systems keep the state at a position and apply only later events.** "A snapshot captures the state of an entity at a specific position in the event stream", and the snapshot is a cache: snapshots "do not replace events and are not required to reconstruct the entity's state" ([Axon Framework reference, Snapshotting](https://docs.axoniq.io/axon-framework-reference/5.3/snapshotting/), read 2026-10-13). B keeps the family's match state at its last row in the same way. That state is never the source: whenever it cannot be used, the family is matched whole from the record.
- **Derived results maintained by key.** #507's plan cites the incremental-view-maintenance literature (DBSP, arXiv:2203.16684) for partitioning work by key. A and C apply it one level finer: a name run is keyed by its instrument and its first sighting, a figure by its round trip.
- **The journals.** Tradervue, TraderSync and TradeZella document trades and executions, not how they compute them. What they show is per trade: adding an execution changes the trade it belongs to (Tradervue merges executions into a trade by symbol and time, [Tradervue FAQ](https://app.tradervue.com/help/faq), read 2026-10-12). C makes the engine's work follow the same unit.

## Open questions

None. The objective is that an added trade and an import cost the same however long the instrument's history is. What settles the approach:
- the measured breakdown in Scope (each step's counts at both depths);
- brief 27's recommendation and the guards it set for continuing the match;
- the code read in full: `Matcher` and `match_over` (`ledger.rs:707–760`, `2161–2300`), `rematch_families` and `rederive` (`engine.rs:386–425`, `813–900`), `identify_in` (`identity.rs:82–135`), `Book::names` and `current_symbol` (`book/src/identity.rs:401–470`), `settle_trades` (`server/src/figures.rs:165`).

## Approach

### A. The book keeps each instrument's names as rows (book migration 029)

A table `instrument_names` holds one row per run of one symbol and venue, under the instrument the runs are read as (the head of its succession), keyed by the run's first sighting. Each row carries the first and last sighting's key (day, symbol, row number), and the run's `Name` fields: symbol, venue code, venue name, name, first and last day seen, and source. Those are the fields `Book::names` builds today: the latest venue name and name stated in the run, the source of its last sighting. Neither new table has a foreign key: `Book::clear` deletes sightings and instruments in its own order (`clear.rs:171`, `:255`), and the triggers and the settle at the end of Clear's transaction keep the runs right for whatever Clear leaves (brief 28, 9).

**Every change to what a run is built from leaves a due row, by trigger.** A table `names_due(instrument_id, day, symbol, sighting)` takes a row from `AFTER INSERT`, `AFTER UPDATE` and `AFTER DELETE` triggers on:
- `instrument_sightings`, keyed by the sighting: an update logs its old key and its new one. `record_sighting` writes `INSERT … ON CONFLICT DO UPDATE` instead of `INSERT OR REPLACE` (`identity.rs:939`), because no connection sets `recursive_triggers`, so a REPLACE fires no delete trigger (brief 28, 1);
- `source_records`, on a change of `state` (a record made live, revised or removed): one row for each of its sightings, old and new, through the sightings' primary key, which leads with the record;
- `instrument_joins`: a row with no sighting for each side, old and new, meaning "fold this head whole". A succession is rare, and it re-points every row anyway.

The row holds the instrument the sighting is kept under: the member's own id, not its head (`identity.rs:562`, `:609`).

**The runs are re-formed only around each due sighting.** `Book::settle_names` reads the due rows, resolves each to the head it is read as now, and works one head at a time, in sighting order, under a savepoint of its own: a head whose rows cannot be read fails alone, with its error logged and its due rows kept, as one instrument's names fail alone on master (brief 28, 15). For each due sighting, the runs that can change are the kept run holding it and the runs either side of it. They are re-formed from what is already known plus a few index reads:
- The kept runs, cut at each due sighting into pieces. Each piece keeps its symbol and venue (compared with `IS`, so an absent venue matches only an absent one), and is kept only if a live sighting remains in it (one indexed read).
- Each due sighting, where it is live now.
- Adjacent pieces of one symbol and venue joined into one run.
- Each run's first and last day, its latest venue name and name, and its source read at its ends. Two partial indexes on the sightings make these reads seeks: one over the sightings that state a name, one over those that state a venue name.

The work per due sighting is a handful of index reads, whatever the instrument's history. A head marked whole is folded whole (today's fold). Runs kept under an id that is no longer a head (a member joined into another) are dropped.

`settle_names` runs at the end of the outermost `Book::atomically` (`book/src/lib.rs:123`), before its commit, so every write through the book's own transactions leaves nothing due. `atomically` returns early inside a transaction opened elsewhere (`bagholder_sqlite::atomically`, `Book::snapshot`; `lib.rs:124`), so a write there commits its due rows unsettled. Those rows are settled by the next outermost `atomically`, and the read below stays right meanwhile. In test builds, a commit hook asserts that nothing is due after a commit through `Book::atomically`.

**A read is correct whatever has settled.** `Book::names` and `current_symbol` read the kept runs only where nothing is due for any instrument of the head's group (its members as well as itself). Otherwise they fold the sightings as today, which is always right, only slower. So correctness never depends on every write path settling, only the cost does, and the capacity probes measure that. Under the dev-only feature below, every read compares the kept runs with the fold and fails on a difference, except inside a probe's measured window.

`current_symbol` reads the last kept run, with its fallback unchanged: the last of `names`, which ends in a source's own description of the instrument (`instruments_described`; `identity.rs:426–430`).

`names_due` and `instrument_names` are named in the clear-all test's classification. The authorizer test of #507, which holds every table the ledger reads to its change-log triggers, sees `instrument_names` read. `instrument_names` gets the three triggers. `names_due` is stated as exempt: it is the queue the runs are settled from, and a change to it changes nothing the engine reads.

The migration creates the tables, the indexes and the triggers, and marks every head whole. `Book::open` settles them once, which on an existing book is one whole fold, the cost of today's start.

### B. The match continues from the family's state for rows placed after it

**What is kept.** After a family is matched whole, the engine keeps the matcher's state for it, taken before the final `expire_before(today)` and the cash sums (brief 27, depth 2). That is every field of `Matcher` but its inputs and output: the expiry calendar, the three taints, `consumed`, the transfer links, `event_groups`, `lieu_fraction`, `ended_on_record`, `places` and `rounding_held`. Alongside it the engine keeps:
- the largest `order_key` walked (`ledger.rs`; the walk breaks ties by ledger place, so the transaction id is not part of it);
- the holdings of the stated-Open rows walked at that key's instant, gathered over every walk at that instant and emptied when a walk moves past it (brief 28, 2);
- whether the final `expire_before(today)` changed anything in the family (closed or tainted a contract).

The match's output is not copied: the family's books, round trips and units are already in the engine's `Matched`, keyed by holding and by round trip.

**When it continues.** A change continues a family's match only if every one of these holds. Otherwise the family is matched whole, exactly as now:

1. The change only adds records. Nothing is revised or removed among the family's records, and nothing among their problems, links or adjustments changes.
2. Every leg of each added record has an `order_key` strictly greater than the largest walked. Where a leg falls at that key's instant, it is not a single-leg stated Close on a holding in the kept set of stated-Open holdings. That set is all the look-ahead reads: it fires only from a stated Open row and takes only single-leg stated Close rows on the same holding (`ledger.rs:2255–2260`). Hand-entered trades state no effect (`person.rs:204–208`), so the probes' ordinary case continues.
3. The change leaves the families as they were. This is decided from the added rows alone: each added row's instrument, account and the couplings it carries (an underlying, a transfer's link, an event's legs, `leg-unstated`) fall within one existing family, so no family is joined or split. The family reads none of its other rows to decide it (`Families::regroup` reads every row of the family today, `families.rs:194`; brief 28, 15).
4. No added row is an expiry, an assignment or an exercise (`ended_on_record` decides earlier expiries), or a corporate event (`event_groups`, `lieu_fraction`). No added row is a transfer linked to a row already walked, or one that names an adjustment.
5. No added coin row states its units to a finer place than the family's rows from the same source did (`places`, per instrument and source, rounds every earlier row).
6. Nothing the match reads of an instrument or an account changed: the instrument's kind, currency and terms, and each account's smallest coin order (`coin_minimum`, which `dust.rs:32` reads), compared before and after for every account that held a row of the family before the change. A new account, such as the Manual account an added trade or an import creates (`entries.rs:138`, `csv_import.rs:168`), held none and does not stop it (brief 28, 5).
7. The final `expire_before(today)` of the family's last match changed nothing, and the day has not turned since. Turning the day matches everything whole, as now.

A change of exchange rates (`Change::Rates`) drops the kept state of every coin family with a row on or after the first day whose rate changed, and matches those families again, since dust is judged at the row's day's rate (`dust.rs:38`). This also closes #513, the same fault on master between a rates change and the next trade (brief 28, 11).

**How it continues.** `Matcher` takes its output as `&mut Matched` instead of owning it: the whole match passes a fresh one, the continued match passes the engine's. The continued match:
1. restores the family's kept state;
2. runs `match_over`'s setup over the added rows (the transfer links each sends and receives, `places`, `ended_on_record`, `event_groups`; `ledger.rs:2196–2217`) and merges it into the kept state. By guard 4, `ended_on_record` and `event_groups` gain nothing, and a debug assertion says so (brief 28, 6);
3. walks the added rows by `match_over`'s own loop;
4. keeps the new state;
5. runs `expire_before(today)`, and adds the rows' cash to the family's share of each account (`Engine::shares`).

Keyed entries (books, round trips, units, roles, what waits on the person) are written in place. Each list entry the walk adds (beyond what was held, dust, arrived, disagreements, stock dividends, what was not applied) is put in its place in the match's order with `partition_point`, as `Matched::replace` puts rows. `trips_of` and `held_by` are updated for the holdings walked. The walk returns the round trips it wrote and the holdings it walked, the reach C works from.

**Wired so a continued family is not wiped** (brief 28, 13). `rematch_families` tells an unchanged family from a changed one by comparing its members before and after, since `regroup` lists every family of the region as gone even when it comes back unchanged (`families.rs:272–276`). It passes `Matched::replace` only the families matched whole, keeps the shares (`Engine::shares`) of the continued ones, and drops the kept state of every family that went.

**Held as an oracle, where the tests run** (brief 28, 10). `#[cfg(test)]` compiles only into a crate's own unit tests, and `engine/tests/by_change.rs`, `book/tests/*.rs` and the server's tests link the libraries without it. So the oracles and the counter sit behind a dev-only cargo feature, `oracles`, on `bagholder-book` and `bagholder-engine`, as `book` already does with `pinned-tzdb`. Each consumer's dev-dependencies enable it, and it is off inside a probe's measured window. Under it, every continued match is checked against matching the family whole on a copy: the engine's `Matched` within the family and the kept state must be equal. A counter records each family's path at each step. The property test compares it with the path the generator's own label for that step expects, never with what the guard decided, and asserts a minimum number of continued steps.

### C. The figures are worked out for the round trips that changed

`Reach` (`engine.rs`) gains the round trips to derive. They come from:
- after a continued match, the round trips its walk wrote;
- for a trade or journal change, the trade's round trip before and after (`Identity::trip_of`, and the round trip its anchor's fill belongs to), in place of its instrument.

After a whole-family match, the reach stays every round trip of the family's instruments, as now. `rederive` then works on that reach:

- **Identity** for the round trips reached (`identify_trips`). Two indexes beside the ranked trades (`Ranked<Trade>`) and the match replace scanning every trade on the instrument (`anchors`, `identity.rs:51`): anchor transaction → trades, and transaction → round trips holding it as a fill. `identify_trips` takes every trade anchored on any fill of a reached round trip, as well as every trade the change names. A trade anchored on a fill that is not an opening moves to the one round trip holding that fill if it has no trade yet (`identity.rs:115–117`), so an opening index alone would miss it. `Identity::replace` updates `moved` and `unclaimed` for those trades (brief 28, 4).
- **Round-trip figures** (`trip_fig`) for the round trips reached. A round trip's figure reads only its own round trip.
- **Groups.** `groups_shown` runs when a round trip reached is held by a group, when a group changed, or when a trade the change names, before or after, is a member of a group. It claims round trips through each member's `trip_of` in the book's order (`trades.rs:344`). Otherwise every group's figure is unchanged (brief 28, 12).
- **Positions** for every holding with lots of a round trip written or reached: a rolled round trip spans the old contract's holding and the new one's (`ledger.rs:1554`, `:893–894`; a position's fills come from every lot's round trip, `positions.rs:239–241`). On a trade or journal change, also every position whose first lot's round trip is reached, before and after, since a position carries that trade and its journal (`positions.rs:235–236`). That is the probes' second step: `settle_trades` opening the new round trip's trade (brief 28, 3 and 7).
- **Cash rows**, as now: by transaction.
- **Payers' rates** for an instrument only where one of its payment rows or its held units changed. Its payments are read through an index of cash rows by instrument, over every account's rows (today it filters every cash row of the book: `engine.rs:884`).
- **Broker checks** for every account holding a reached instrument, as now.

This also stops the second whole pass in the Scope table. `settle_trades` opening the new round trip's trade is a trade change, and it now reaches that round trip, its position and its group only.

### D. The gate

`OVER_BUDGET` loses add-trade and import on the depth pair, and #419 and #420 close. The capacity run must pass both pairs with nothing listed for these two operations; its own rule fails a listed operation that meets its budget. The in-suite growth test's depth pair becomes `deep: 20` against `deep: 320` (rows 39 and 639, sixteen times as deep), so a planted whole fold of names, at about 29 steps a row, is over 2× and fails it. It also asserts that the continued counter ran for the probe (brief 28, 14).

### E. What brief 28 found left of brief 27's fixes, built in this PR

- **The in-suite growth test catches one whole read in the import.** Its plant becomes exactly one `live_records(csv)` read in the import's window. The test catches it on steps at the deeper size, or counts bytes read where the platform gives them, instead of filtering them out (`capacity.rs:1103`). The PR states what the test then proves.
- **The member-write test holds its ref and removal steps.** It deletes the head's existing refs before adding the member's broker id, and stores a record under the member, so each step changes what is shown.
- **A pull that fails after its first catch-up keeps its names.** On any error after the first `catch_up` in `pulled`, the gathered names are reloaded (or the names cache is reset) before the error returns. A test drives it through `pulled` with an error after the catch-up.
- **The names cache is never written stale.** `names()` keeps the engine's read lock across the cache write (`figures.rs:471–472`).
- **The race test races the read-back.** Its hook fires inside `settle_trades`' catch-up, so the stale-anchor guards run against `fills_give_way`.
- **The server's fresh-build check compares order, the match and identity.** `same_as_fresh` (`figures.rs:728–742`) calls `match_disagrees` too.
- **Fix 2's generator** is widened until several generated books catch its plant. **Fix 7** (a failed log prune leaves the pull's work standing) gets its test.

### What stays the same

What every figure is; the order the match applies rows in; the whole-family match and every case that goes to it; the families; the change log and how it is read; the full build at a start and when the day turns; `SPEC.md`.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green; `cargo clippy --workspace --lib --bins` clean.
- [ ] **The kept names equal the fold.** A property test over generated books writes sightings in any order, under several symbols and venues, changing back and forth, with names and venue names stated on some and absent on others. It writes through the book and also with raw `INSERT`, `UPDATE`, `REPLACE` and `DELETE` outside `atomically` (as `figures.rs:783` renames a sighting). It revises and removes records, and joins and parts successions with records under the member. It clears entries alone and clears all. After every step `Book::names` and `current_symbol` equal the fold for every instrument. Under the `oracles` feature, the check on every read in the suite holds the same.
- [ ] **A write through the book leaves nothing due.** Under the feature, a commit hook asserts it on every commit through `Book::atomically` in the suite.
- [ ] **A head that cannot be settled fails alone.** A test plants an unreadable sighting under one instrument. Writes and `Book::open` still succeed, other instruments' names settle, and that one's error is logged and its due rows kept.
- [ ] **The continued match equals a whole one, and is used.** `engine/tests/by_change.rs` gains sequences that add rows after the last (today's trades, an import, a first trade into a new account, a linked transfer pair both added, a first hand-entered row on a coin). They are interleaved with every case the guards send whole: a back-dated row; a revision and a removal; a row at the last key's instant, on a holding with a walked stated Open and on another (the three-step case of brief 28, 2); an expiry and an assignment row; a contract passing its expiry with lots open; a corporate event; a transfer linked to an earlier row; a coin row with a finer place from the same source; a change of terms; a smallest coin order raised with a coin trade; a family joined by a new record; a rates change on a coin's day; and the day turning. Also a partial roll, then a continued sale of the new contract. After every step the engine equals a fresh build (`match_disagrees`: `Matched`, `Identity`, families, the trades, positions and cash in order). The counter matches the generator's label on every step, with at least a stated number of continued steps.
- [ ] **The figures by round trip equal a fresh build's.** The same sequences, plus trade, group and journal changes (a trade anchored on an add-on fill, a removed trade freeing a round trip, a group member reattached), hold `Moved` equal to the difference between fresh builds before and after each step.
- [ ] **Each guard bites.** For each of guards 1–7, the rates rule and the wiring of a continued family, planting its removal fails the property test (shown in the PR).
- [ ] **The capacity run, breadth and depth:** add-trade and import meet the growth rule on both pairs (steps, allocations and bytes read under 2×) and their budgets on a Pi (100 ms; 1 s), with nothing listed for them in `OVER_BUDGET`. The runner's table is pasted in the PR.
- [ ] **The in-suite growth test** at `deep: 20` against `deep: 320` holds steps and allocations under 2× for both probes, and asserts the continued counter ran. Its planted whole fold of names fails it, and the import's planted `live_records(csv)` read fails it.
- [ ] **Brief 27's leftovers (E):** each has its test, and each test fails with its fix undone (shown in the PR).
- [ ] **The owner's book:** a copy migrates (029); the figures digest is equal before and after; an added trade and a 20-row import on the copy are timed and counted, and pasted in the PR.
- [ ] `docs/architecture.md` §13's sentence on the engine says a change to the record continues a family's match where it can, and keeps names as runs; the back-dated case stays under #512; #513 closes.

## Surfaces to check beyond the diff

- Book migration 029 and its `schema/v29.sql`; `record_sighting`'s upsert; `Book::clear` with both clears; the clear-all test's classification; the authorizer test of #507 with `instrument_names` and the `names_due` exemption.
- `Book::atomically`: `settle_names` before commit, the savepoint per head, nested calls, and writes inside `bagholder_sqlite::atomically` or `snapshot`.
- Every caller of `Book::names` and `current_symbol`: `engine_inputs`, `watched.rs:132`, the wire's names (`wire/build.rs`).
- `Matcher` taking `&mut Matched`: `match_lots`, `match_over`, `Engine::past` (it matches afresh).
- `rematch_families`, `Families::regroup` and `Engine::shares`; `Change::Rates` (`engine.rs:671–676`).
- `settle_trades` (`figures.rs:165`), `catch_up` and `pulled`; `names()` and its lock.
- The `oracles` feature in every crate's dev-dependencies that tests the book or the engine.
- `capacity.rs`: `OVER_BUDGET`, the in-suite growth test and both plants.

## Right to refuse

If the property test finds a coupling inside a family that continuing the match breaks, beyond the guards here, the case gets a guard that sends the family whole, never a workaround. If guards then send the probes' ordinary case (today's trade on a held listing) whole, the plan stops and brings the measurement back to the gate. If the kept runs cannot equal the fold for some input, the read falls back to the fold for that head, and the case is reported here.

## Anti-stub self-check

To initial at build: no kept state the continued match does not read; no guard without a step in the property test that trips it; no trigger whose rows nobody settles; no oracle compiled out of the tests that cite it; the suite, the capacity run and the owner's copy run, not asserted.

## Verification

Before the build: the breakdown in Scope, master `0bc73874`, this Mac, the depth pair's books built by the capacity run of 2026-10-12 (358 and 1,406 rows), the probes run with temporary counters at each step.

## Handoff

Brief 28 (Go with changes) applied. Build order: A (the names; measured alone first, as brief 27 asked), then B behind the property test and the oracles, then C, then D and E.

**Nothing left running.**
