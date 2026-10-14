# Brief 30: the deep-history build as built (#514, `deep-history` at `1a550d63`): not yet, merge after the fixes

**Reviewed:** the build of plan #511 against brief 28's fifteen changes and brief 27's six open defects. Three readers took part A (names as rows), part B (the continued match), and parts C and D with the `oracles` feature and brief 27's fixes. They ran the suites on the PR build with the oracles on: `bagholder-book`; engine `continued` and `by_change`; server `capacity::tests` (8 of 8, the growth test and both plants among them, 11.5 min) and `figures::` (13 of 13), all passing. Each defect was then planted or undone in a throwaway copy, one at a time. I checked every finding below against the code myself. The copies and their build folders are deleted.

## Verdict

Not yet. The design is built as planned and most of brief 28 is in. But four defects stand:
- one record shape gives a round trip and a position different from a fresh build (B1);
- every book open now takes the write lock (A1);
- one unreadable join stops every write and every start (A2);
- a large import's cost grows with the history again (A3), the cost this PR exists to remove.

Several tests also hold less than the PR's table says. Merge #515 first. Then merge master into this branch, which conflicts in one file, in two places (brief 29). Then make the fixes below. The verdict will be on that merged head.

## Brief 28's changes, as built

| Item | Verdict |
|---|---|
| 1 every change leaves a due row | Done |
| 2 the instant's stated-Open holdings kept | Done; the test partly (B3) |
| 3 positions on a trade or journal change | Done |
| 4 identity from trades on any fill | Done |
| 5 guard 6 | Done differently, as well: any change to an existing account blocks its families |
| 6 setup over the added rows | Done |
| 7 rolls span two holdings | Partly (C3) |
| 8 due rows by head; `current_symbol` checked | Done, at a cost (A3) |
| 9 Clear | Done in the code; the test partly (A6) |
| 10 the oracles where the tests run | Done, with one leak into the measured window (A5) |
| 11 rates | Done; closes #513 |
| 12 groups | Done |
| 13 wiring | Done differently, as well |
| 14 the growth test | Partly (C4) |
| 15 settle per head; nested `atomically`; "after"; `regroup` | Partly (A2); done differently, as well; done; done |

Brief 27's open defects:
- Done: the import plant is caught; names after a failed pull; the names cache lock; `same_as_fresh` compares the match; fix 2's generator (16 of 60 books catch it); fix 7's test.
- Fix 1's test was done differently, as well.
- The race test is partly done (C1).

## Part A, names kept as rows

The kept runs equal the fold in every case tried.

A1. **Every book open now takes the write lock.** `Book::open` ends with `book.atomically(|| Ok(()))?` (`book/src/lib.rs:110`), which begins IMMEDIATE (:137). The server opens a book per call (`Figures::book`, `figures.rs:333–334`, used throughout). Measured: an open waited 3.08 s behind another connection's 3 s write; behind Clear or any long write, every request waits up to the 30 s busy timeout and then fails, and a read-only file no longer opens. On master an open at the current schema takes no lock. Required: settle at open only when `names_due` holds a row, or only after a migration ran.

A2. **One unreadable join fails every write and every open.** The settle resolves each due row's head with `self.canonical(id)?` (`names.rs:237`) before the per-head savepoint (:264), and the oracle does the same (`lib.rs:145`). Confirmed: a join whose `into_id` does not parse makes a write of an unrelated instrument, and `Book::open`, fail with "cannot read in instrument_joins.into_id"; Clear fails the same way. This is the failure brief 28 15.1 asked to contain to one instrument. Required: resolve the head inside the per-head handling; on failure log it, keep its rows and go on.

A3. **`current_symbol` folds the whole history whenever anything of the group is due, including rows the caller's own open transaction just wrote.** It now goes through `names()` (`names.rs:202–204`), which folds when due (:159); master sought the newest sighting through an index. A CSV import of more than 100 rows commits in groups of two or more rows (`csv_import.rs:265`), and each row looks its instrument up through `Book::name` → `current_symbol` (`csv_import.rs:226`, `person.rs:280`), so every row after the first in a group folds. Measured: 1,542 steps at depth 50 against 11,692 at depth 400; with nothing due, a constant 75. The gate's 20-row import commits a row at a time and never sees it. Required: when due, `current_symbol` reads the newest sighting per member through the index, as master did; add a probe importing more than 100 rows on a deep listing.

A4. **`recursive_triggers` is on for the whole connection, unstated.** `lib.rs:105` sets it; the plan (line 71) and the comment at `identity.rs:865–866` still say no connection sets it. Every REPLACE on the book's connection now fires delete triggers (for example `transfer_links_changed_delete` on `statements.rs:543`), and the property test's REPLACE step runs on that connection, so it never tests the upsert alone. Required: state the pragma in the plan or drop it; correct the comment; run the REPLACE step on a connection without it.

A5. **The due-row assertion runs inside a probe's measured window.** `lib.rs:141–149` queries `names_due` on every commit under `oracles` with no `measuring()` check, so the in-suite growth test counts test-only steps, against plan lines 87 and 127. Required: gate it as `names.rs:161` is.

A6. **The property test holds less than the criterion.** It clears broker-alone and all, never entries alone (`tests/names.rs:141`); it has no raw plain INSERT; it only joins `ids[0]` into `ids[1]` (:136). Required: person or CSV records sighting a listing the broker's records also sight, then a Clear of entries alone; a raw INSERT step.

## Part B, the continued match

Guards 3 to 7 hold, and the guard 3 plants fail `by_change` as the PR's table says.

B1. **Guard 2 compares each leg only with its own family's last row; a record is applied at its first leg, whichever family that is in.** The guard checks `order_key_of(t, …) <= last` for the leg's own family (`engine.rs:1008–1009`); the walk applies a record whose legs share a day at its earliest leg (`ledger.rs:2346–2353`). Reproduced: Y held from 01-05; `l` buys 10 X on 03-02 at 15:00; add record `r`, untimed on 03-02, selling 10 X and buying 5 Y. Each leg is after its own family's last row, so both continue. A fresh build applies the X sale at the Y leg's place, before `l`: the sale goes beyond what was held and `l` stays open long 10. The continued match closes `l`. `match_disagrees` reports the trips, books, beyond and units, and the oracle panics (`engine.rs:1102`). This is the plan's "every leg", which brief 28 accepted and the build does not carry. Required: for each family a record touches, the record's earliest leg, on any node, must be after that family's last row; a labelled step for this case; non-event two-leg records in the `by_change` generator, whose only cross-family records today are corporate events, which guard 4 stops.

B2. **Guard 1's "revised or removed" clause fails no test.** With `engine.rs:962–965` commented out, `continued` and `by_change` both pass, while the PR's table says `by_change` catches it. Scenario: one change revises `s1` (buy 10 → 20) and adds a later sale of 15 on the same instrument; without the clause the family continues and differs from a fresh build. Each `by_change` delta carries one record, and `continued.rs` has no removal step. Required: a step revising or removing a record of the family in the same change as an added row of it; correct the PR's table.

B3. **Brief 28 item 2's three-step case has no labelled step.** After `o3` the next step (`o4`) goes whole at once (`continued.rs:171–173`); resetting `opens_at_last` each walk is caught only by the state oracle at `o5`. Required: a step between them at the same instant on another holding, which continues.

B4. **Two plan cases have no labelled step:** an assignment or exercise row, and an adjustment whose legs name only another family (the path that replaced guard 4's dropped clause, sound by reading: `engine.rs:658–661`, :695, `families.rs:197–200`, `engine.rs:711`).

## Part C, figures by round trip; part D, the gate

The identity's reach is closed over the trades it recomputes (`engine.rs:1134–1157`), a defect the generator found during the build. Undoing it, or the positions rebuilt for the round trips reached, or the group check on a named member, each fails `by_change`.

C1. **The race test never reaches the stale-anchor guards.** Its hook fires inside the read-back's snapshot (`figures.rs:1029`). At that point the engine names nothing, so `settle_trades` returns, and the next `record_changed` applies the give-way consistently. With `anchored_as` (`book/src/trades.rs:110`) forced to true, this test and every other `figures::` test still pass. Required:
   - Put the hook between `engine.identity()` and the writes in `settle_trades` (`figures.rs:186–205`), where the other writer re-anchors a trade that the identity names in `moved` or `unclaimed`.
   - Assert the write is refused, and that the test fails with the guard off.

C2. **A group's journal changed alone is never applied.** `JournalSubject::Group(_) => {}` (`engine.rs:652`) reaches nothing. A delta holding only a group's journal leaves that group's figure stale; master rebuilt every group on every ledger change. The server is safe today only because the journal triggers also log the group. Required: a group subject reaches the groups; add the case to `changes.rs`.

C3. **The roll's other holding is rebuilt by a line no test holds.** Removing `shown.extend(t.instruments…)` (`engine.rs:1211`) fails nothing. After the roll, a trade opened on the old round trip, or a journal written on it, is shown on the new contract's position, which only that line rebuilds. Required: `TradeOn("r1")`, or a journal on it, after the roll in `continued.rs`.

C4. **The growth test checks the continued path for one book only, and only the last entry.** The paths are taken after both probes, and only `paths.last()` is read (`capacity.rs:1136`, :1146). So the smaller book's probe could go whole unseen, and so could a whole match before a continued one. Required:
   - Take the paths after each probe.
   - Assert that the window's entries for the target family are exactly one continued match, on both books.

C5. **Payers are still found by scanning every cash row** (`engine.rs:1251`, as on master), against the plan's index. This is CPU the gate cannot see. Required: build the index, or name the leftover in the plan.

C6. **A pull that fails after its catch-up drops what moved.** On an error, `pulled` returns at `figures.rs:619–621`, before `self.moved(moved)` and the wake. What the applied catch-up changed then reaches no page until the next change. The same was true on master, but this PR rewrote the function. Required: record what moved and wake before returning the error. Brief 29's fix 1 (wake with no engine) lands in the same function.

## What is sound

- **Part A.** Due rows are written by trigger on every input the fold reads: sightings, record state and joins. Runs are re-formed around each due sighting with the venue compared exactly. The direct path for a key after the last run compares the full (day, symbol, rowid) tuple. Clear needs no foreign keys. A failing head's rows are kept and its reads fall back to the fold.
- **Part B.**
  - The kept state is taken before the final expiry, and the setup is extended over the added rows.
  - Guard 5 is per (instrument, source).
  - On a rates change, coin families are matched again.
  - The wiring keeps continued families out of the regroup.
  - Ties are broken without the id, and lists are placed by `partition_point`.
  - The counter takes each step's path from the test's own labels and asserts at least 300 continued steps.
  - Dropping guard 4's "a row naming an adjustment" holds for everything the book can produce.
- **Part D.** The depth runs at 320 (639 rows). Both plants fail the growth test. `OVER_BUDGET` keeps only the feed step. The `oracles` feature stays out of release builds (resolver 2).
