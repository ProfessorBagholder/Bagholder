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

A table `instrument_names` holds one row per run of one symbol and venue, under the instrument the runs are read as (the head of its succession), keyed by the run's first sighting. Each row carries the first and last sighting's key (day, symbol, row number), and the run's `Name` fields: symbol, venue code, venue name, name, first and last day seen, and source. Those are the fields `Book::names` builds today: the latest venue name and name stated in the run, the source of its last sighting.

**What changes a run is logged by triggers, as #507's change log is.** A table `names_due(instrument_id, day, symbol, sighting)` takes a row from triggers on:
- `instrument_sightings`: insert and delete, keyed by the sighting;
- `source_records`: a change of `state` (a record made live, revised or removed), one row for each of its sightings, through the sightings' primary key, which leads with the record;
- `instrument_joins`: insert and delete, a row with no sighting for both sides' heads, meaning "fold this head whole". A succession is rare, and it re-points every row anyway.

**The runs are re-formed only around each due sighting.** `Book::settle_names` reads the due rows by head, in sighting order. For each due sighting, the runs that can change are the kept run holding it and the runs either side of it. Those runs are re-formed from what is already known plus a few index reads:
- The kept runs, cut at each due sighting into pieces. Each piece keeps its symbol and venue, and is kept only if a live sighting remains in it (one indexed read).
- Each due sighting, where it is live now.
- Adjacent pieces of one symbol and venue joined into one run.
- Each run's first and last day, its latest venue name and name, and its source read at its ends. Two partial indexes on the sightings make these reads seeks: one over the sightings that state a name, one over those that state a venue name.

The work per due sighting is a handful of index reads, whatever the instrument's history. A head marked whole is folded whole (today's fold).

`settle_names` runs at the end of the outermost `Book::atomically` (`book/src/lib.rs:123`), before its commit. Every write that goes through the book's own transactions therefore leaves no due rows.

**A read is correct whatever settled.** `Book::names` reads the kept runs only where the head has no due rows. Otherwise, after a write made outside `atomically`, it folds the sightings as today; that is always right, only slower. So correctness never depends on every write path settling: only the cost does, and the capacity probes measure that. In test builds every read compares the kept runs with the fold and fails on a difference, as #507's linking oracle does, and it does not compare inside a probe's measured window.

`current_symbol` reads the last kept run. Its fallbacks (the person's chosen listing, a source's own description: `listing_named`, `instruments_described`) are unchanged.

The migration creates the tables, the indexes and the triggers, and marks every head whole. `Book::open` settles them once, which on an existing book is one whole fold, the cost of today's start.

### B. The match continues from the family's state for rows placed after it

**What is kept.** After a family is matched whole, the engine keeps the matcher's state for it, taken before the final `expire_before(today)` and the cash sums (brief 27, depth 2). That is every field of `Matcher` but its inputs and output: the expiry calendar, the three taints, `consumed`, the transfer links, `event_groups`, `lieu_fraction`, `ended_on_record`, `places` and `rounding_held`. Alongside it the engine keeps the family's last walked row: its full place in the match (`place_in_match`) and its instant, with the holdings walked at that instant. It also records whether the final `expire_before(today)` changed anything in the family (it closed or tainted a contract). The match's output is not copied: the family's books, round trips and units are already in the engine's `Matched`, keyed by holding and by round trip.

**When it continues.** A change continues a family's match only if every one of these holds; otherwise the family is matched whole, exactly as now:

1. The change only adds records. Nothing is revised or removed among the family's records, and nothing among their problems, links or adjustments changes.
2. Every leg of each added record is placed strictly after the family's last walked row. Where a leg falls at the same instant as that row, no row walked at that instant is on the leg's holding: the rule that a close goes before an open at one instant (`match_over`, `ledger.rs:2251–2266`) looks ahead within an instant.
3. The change leaves the families as they were: no family is joined or split (`Families::regroup` returns the families the change reached, unchanged).
4. No added row is an expiry, an assignment or an exercise (`ended_on_record` decides earlier expiries), or a corporate event (`event_groups`, `lieu_fraction`). No added row is a transfer linked to a row already walked, or one that names an adjustment.
5. No added coin row states its units to a finer place than the family's rows did (`places` rounds every earlier row).
6. No instrument or account the family reads changed: terms, kind, currency, the account's type.
7. The final `expire_before(today)` of the family's last match changed nothing, and the day has not turned since. Turning the day matches everything whole, as now.

**How it continues.** `Matcher` takes its output as `&mut Matched` instead of owning it. The whole match passes a fresh one; the continued match passes the engine's. It restores the family's kept state, walks the added rows by `match_over`'s own loop, keeps the new state, then runs `expire_before(today)` and adds the rows' cash to the family's share of each account (`Engine::shares`). Keyed entries (books, round trips, units, roles, what waits on the person) are written in place. Each list entry the walk adds (beyond what was held, dust, arrived, disagreements, stock dividends, what was not applied) is put in its place in the match's order with `partition_point`, as `Matched::replace` puts rows. `trips_of` and `held_by` are updated for the holdings walked. The walk returns the round trips and holdings it wrote: the reach C works from.

**Held as an oracle.** In test builds, every continued match is checked against matching the family whole on a copy: the engine's `Matched` within the family and the kept state must be equal. It is not checked inside a probe's measured window. A counter of continued matches lets the tests assert that the path ran, so a guard that always falls back cannot pass as continued.

### C. The figures are worked out for the round trips that changed

`Reach` (`engine.rs`) gains the round trips to derive. They come from:
- after a continued match, the round trips its walk wrote;
- for a trade or journal change, the trade's round trip before and after (`Identity::trip_of`, and the round trip its anchor opens), in place of its instrument.

After a whole-family match, the reach stays every round trip of the family's instruments, as now. `rederive` then works on that reach:

- **Identity** for the round trips reached and the trades the change names (`identify_trips`). An index from opening to trade beside the ranked trades (`Ranked<Trade>`) replaces scanning every trade on the instrument (`anchors`, `identity.rs:51`). Adding rows after a family's last row never stops an existing opening from being one, so the trades a continued match can leave without a round trip are only those the change itself names.
- **Round-trip figures** (`trip_fig`) for the round trips reached.
- **Groups.** `groups_shown` runs only when a round trip reached is held by a group, or a group changed. Otherwise every group's figure is unchanged, because none of its round trips changed and the claims between groups did not either.
- **Positions** for the holdings walked, through `build_positions(instrument)` as now.
- **Cash rows**, as now: by transaction.
- **Payers' rates** for an instrument only where one of its payment rows or its held units changed, reading its payments through an index of cash rows by instrument (today it filters every cash row of the book: `engine.rs:884`).
- **Broker checks** for the accounts reached, as now.

This also stops the second whole pass in the Scope table. `settle_trades` opening the new round trip's trade is a trade change, and it now reaches that round trip only.

### D. The gate

`OVER_BUDGET` loses add-trade and import on the depth pair, and #419 and #420 close. The capacity run must pass both pairs with nothing listed for these two operations. Its own rule already enforces this: a listed operation that meets its budget fails the run.

### What stays the same

What every figure is; the order the match applies rows in; the whole-family match and every case that goes to it; the families; the change log and how it is read; the full build at a start and when the day turns; `SPEC.md`.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green; `cargo clippy --workspace --lib --bins` clean.
- [ ] **The kept names equal the fold.** A property test over generated books writes sightings in any order, under several symbols and venues, changing back and forth, with names and venue names stated on some and absent on others. It revises and removes records, joins and parts successions, and writes outside `atomically`. After every step `Book::names` and `current_symbol` equal the fold for every instrument. The test oracle on every read in the suite holds the same.
- [ ] **A write through the book leaves nothing due.** A test: after every write path the suite drives through `atomically`, `names_due` is empty.
- [ ] **The continued match equals a whole one, and is used.** `engine/tests/by_change.rs` gains sequences that add rows after the last (today's trades, an import), interleaved with every case the guards send whole: a back-dated row, a revision and a removal, a row at the last row's instant on the same holding and on another, an expiry and an assignment row, a contract passing its expiry with lots open, a corporate event, a transfer linked to an earlier row, a coin row with a finer place, a change of terms, a family joined by a new record, and the day turning. After every step the engine equals a fresh build (`match_disagrees`: `Matched`, `Identity`, families, the trades, positions and cash in order). The counter shows the continued path ran on every step a guard allows and on no step one forbids.
- [ ] **The figures by round trip equal a fresh build's.** The same sequences, plus trade, group and journal changes, hold `Moved` equal to the difference between fresh builds before and after each step.
- [ ] **Each guard bites.** For each of guards 1–7, planting its removal fails the property test (shown in the PR).
- [ ] **The capacity run, breadth and depth:** add-trade and import meet the growth rule on both pairs (steps, allocations and bytes read under 2×), and their budgets on a Pi (100 ms; 1 s), with nothing listed for them in `OVER_BUDGET`. The runner's table is pasted in the PR.
- [ ] **The in-suite growth test** compares an added trade on a small book's deepest listing with the same listing sixteen times as deep, and holds steps and allocations under 2×. Its planted whole fold of names (`Book::names` read through the fold inside the window) fails it.
- [ ] **The owner's book:** a copy migrates (029); the figures digest is equal before and after; an added trade and a 20-row import on the copy are timed and counted, and pasted in the PR.
- [ ] `docs/architecture.md` §13's sentence on the engine says a change to the record continues a family's match where it can, and keeps names as runs; the out-of-scope back-dated case stays under #512.

## Surfaces to check beyond the diff

- Book migration 029 and its `schema/v29.sql`; `Book::clear` empties `instrument_names` and `names_due` with the record; the authorizer test of #507 (`ledger_changes` triggers) still finds every table the ledger reads, now including `instrument_names`.
- `Book::atomically`: `settle_names` before commit, and nested calls.
- Every caller of `Book::names` and `current_symbol`: `engine_inputs`, `watched.rs:132`, the wire's names (`wire/build.rs`).
- `Matcher` taking `&mut Matched`: `match_lots`, `match_over`, `Engine::past` (a past day's holdings built from a filtered ledger).
- `settle_trades` (`figures.rs:165`) and `catch_up`: the second pass is now a trade change.
- `capacity.rs`: `OVER_BUDGET`, the in-suite growth test and its plant.

## Right to refuse

If the property test finds a coupling inside a family that continuing the match breaks, beyond guards 1–7, the case gets a guard that sends the family whole, never a workaround. If guards then send the probes' ordinary case (today's trade on a held listing) whole, the plan stops and brings the measurement back to the gate. If the kept runs cannot equal the fold for some input, the read falls back to the fold for that head and the case is reported here.

## Anti-stub self-check

To initial at build: no kept state the continued match does not read; no guard without a step in the property test that trips it; no trigger whose rows nobody settles; the suite, the capacity run and the owner's copy run, not asserted.

## Verification

Before the build: the breakdown in Scope, master `0bc73874`, this Mac, the depth pair's books built by the capacity run of 2026-10-12 (358 and 1,406 rows), the probes run with temporary counters at each step.

## Handoff

Not started. Build order: A (the names; measured alone first, as brief 27 asked), then B behind the property test and the oracle, then C, then D.

**Nothing left running.**
