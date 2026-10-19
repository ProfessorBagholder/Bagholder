# Plan: a round trip added on a past day a holding was flat costs what it changed (#537)

## For the owner to decide

Nothing open. Brief 58 settles the scope as engineering. What crosses a flat point is read from the engine below, and what a kept state costs is measured.

## Scope

When a person adds a round trip of their own on a past day a holding was flat (a buy and its sale, `backdated-flat` in the capacity job), the engine matches the holding's whole family again. That happens because the match keeps its state only at the end of the history (`docs/plans/stage-3-a-change-on-a-deep-history.md`, B), and `Engine::continuable` refuses any row placed before it (guard 2). Its allocations grow with the history: 7.1× at the test sizes, and 3.6× at four times the owner's depth. It is listed on `OVER_BUDGET` as `("backdated-flat", Depth, Allocations)` under #537.

This plan places such a round trip into the flat stretch it falls in, without walking the rest of the family. No figure, screen, wire type or stored table changes.

Out of scope:
- **A back-dated row that keeps a position open** (`backdated-trade`, `backdated-fill`). Every later round trip joins the one it opens, so the work from its day on is the change's own, judged per changed trip (#527). Its request time is #538.
- **A back-dated row on a holding that is not flat on its day.** It still matches the family whole.

## The old app here

The old app re-derived everything on any change. Nothing is carried over.

## What brief 58 asked, answered

### 1. What a state kept at a flat point costs, measured

A temporary probe kept, at every point where all of a family's holdings were flat, a copy of everything the match carries. That is every `Matcher` field that is not a reference or per walk, plus each holding's taint, its average cost and realized total, the last coin price and each holding's last units entry. The probe ran inside the capacity job's `backdated-flat` window, at the test depths, and counted entries and allocations. It was a local hook, not committed, on master `551e281a`.

| Depth (round trips) | Flat points kept in the window | Entries at the largest point | Allocations without → with the copies |
|---|---|---|---|
| 20 | 44 | 8 | 3,679 → 3,730 (+1.4%) |
| 320 | 644 | 8 | 26,414 → 27,232 (+3.1%) |
| 174 (the owner's depth) | 556 | 31 | 15,371 → 15,810 (+2.9%) |
| 698 (four times it) | 1,507 | 31 | 55,001 → 56,753 (+3.2%) |

A full copy at a flat point is small and does not grow with the history: at most 8 entries on the test book and 31 on the owner's (more accounts and instruments in the family), the same at both depths, and about 1.2 allocations a point. Brief 58's expectation holds. The design below keeps even less: two order keys per flat stretch, because the guards in B rule out every case where the carried state could differ between a flat point and the history's end.

### 2. What reads across a flat point, from `rust/crates/engine`

"Flat" exists per holding (`ledger::Matcher::settle` resets a side's average cost and its round trip once its lots are gone). A family is flat at a point when every holding in it is: every account's units of every instrument in it, including an underlying's contracts and anything a transfer or an event links in. Read from the code (`ledger.rs` and `engine.rs` at `551e281a`), these are the things a row's treatment can depend on from before such a point:

| What | Where | How far back it reads | Held by |
|---|---|---|---|
| A holding's taint (a row it could not apply) | `Book::taint`, merged in `Matcher::book`, copied to every slice | The holding's whole life: never cleared, a flat holding keeps it | Guard 3: no taint anywhere in the family |
| The three multi-leg taints (`underlying_taint`, `instrument_taint`, `account_taint`) | `Matcher` | Whole life, never cleared | Guard 3 |
| The holdings a leg-unstated row taints | `out.books.keys()`, including empty books | Whole life | Guard 3 |
| Realized at average cost | `Basis::realized` | Whole life (`settle` resets only the cost) | Additive: the stretch's own realized is added (C) |
| The last stated coin price | `Matched::last_price` | Whole life, per instrument | Guard 4: no coin rows |
| A coin source's finest place, rounding held, dust | `places`, `rounding_held`, `dust.rs` | Every row of the family, both ways | Guard 4 |
| A broker's expiry, assignment or exercise row | `ended_on_record`, filled from every row before the walk | **Ahead:** a later row decides how an earlier contract ended | Guard 4: no contracts |
| The expiry calendar | `expiring`, entries never removed on close | Until each expiry pops | Guard 4 |
| Transfer links | `links_out`, `links_in`, `consumed` | Both ways, across accounts; a whole move keeps its round trip in another holding | Guard 4: no linked rows; guard 2 places the rows inside one stretch |
| Corporate events, cash in lieu | `event_groups`, `lieu_fraction` | Within one account and day | Guard 4: no events; guard 2: the stretch holds no other row |
| Rolls, one-day multi-leg records | `closed_by`, the walk's record order | Within one record | Guard 4: every added record's legs are in the family, and the stretch holds no other row |
| A stated close before a stated open | `opens_at_last`, the walk's look-ahead | Within one instant | Guard 2: strictly between instants |
| The units timeline | `record_units` compares with the timeline's last entry | The latest entry | Changed (C): compare with the entry on or before the row's day |
| Each account's cash share | `Engine::shares` | Running sums | Additive (C) |
| Instruments, accounts, adjustments, rates, closes | `Inputs` | Any day | Guards 1 and 6 of the deep-history plan, as now |

**Tax windows.** Canada's superficial-loss rule and the US wash-sale rule each reach 30 days either side of a loss, across accounts and instruments. The engine applies neither: neither `SPEC.md` nor `rust/crates` names either rule, and a trade's P&L is matched first in first out with no deferral (`SPEC.md` §1). So nothing reads across a flat point on their account today. If either rule is added, it joins this list: a stretch is then placed into only where no loss within 30 days before it has a purchase within 30 days after it, and otherwise the family is matched whole.

## How the leading products do it

- **Splitting at flat.** Tradervue starts a trade when a position goes flat, and splits trades only where the position is flat ([Split & Merge Trades](https://help.tradervue.com/article/3480-split-merge-trades), read 2026-10-18). The flat point is the category's own boundary between trades, which is why a round trip inside one changes no other trip.
- **Snapshots.** Event-sourced systems keep state at a position in the stream and apply only later events; a snapshot is a cache, never the source ([Axon reference, Snapshotting](https://docs.axoniq.io/axon-framework-reference/5.3/snapshotting/), cited in the deep-history plan). The stretches here are such a cache: whenever one cannot be used, the family is matched whole from the record.

## Approach

### A. The match keeps each family's flat stretches

As it walks, the match records each **flat stretch** of the family: the place of the row after which every holding in the family is flat, and the place of the next row that moves a lot. Rows the walk passes over (`Engine::walk_passes_over`: no instrument, or a distribution no adjustment names) neither end nor start a stretch. The stretches are kept in `MatchState` as a `BTreeMap<OrderKey, OrderKey>`, from start to end. That is two keys a round trip. The stretch before the family's first row has no start, and the last stretch of a family flat at its end has no end.

A whole match rebuilds them, and a continued match (deep-history plan, B) appends to them.

### B. A back-dated round trip inside a stretch is placed there

`Engine::continuable` places a family's added rows, placed before its last walked place, only if every one of these holds. Otherwise the family is matched whole, as now:

1. Guards 1, 3, 5 and 6 of the deep-history plan hold, as they do for a continued change today: only added records, the families unchanged, no finer coin places, and nothing the match reads of an instrument or an account changed. Guard 1 also refuses an adjustment set on a row, so an added row with an adjustment goes whole.
2. Every added row falls on a day strictly between the days its kept stretch starts and ends, and all of the family's added rows fall in one stretch. A row on the stretch's first or last day goes whole, so nothing is placed at an instant the stretch's own rows share.
3. Nothing in the family is tainted: neither the three multi-leg taints in its kept state, nor any book of its holdings. Taints never clear, so none now means none at any point (`Engine::untainted`).
4. Every added row is on a security. A coin's rows read its source's finest place, the rounding held and dust, and a contract's rows read the expiry calendar.
5. **The walk's own check** (`ledger::place_in_stretch`). Walked alone on a scratch match, from no lots and an empty state, the rows must leave every book flat and untainted, and every holding flat. They may write only round trips, roles, units and books: nothing beyond what was held, no dust, arrivals, disagreements, stock dividends, rows not applied or rows waiting on the person, and no expiry, consumed row or rounding left in the state. If this fails, nothing is written and the family is matched whole.

As built, guard 5 also covers what the plan listed separately in its guard 4. A transfer leaves units waiting or beyond what was held. A contract leaves an expiry on the calendar. A record with a leg in another family is walked as the whole match walks it, by its place. The plants show which guard each case needs (the PR's table). A guard that no test could trip was not kept.

**How it is placed.** The added rows are walked by `match_over`'s own loop on the scratch match. Then:
- the round trips, roles and fill indexes are written into the engine's `Matched`;
- each touched holding's book keeps its lots and cost, and its `Basis::realized` gains the stretch's realized amount; a holding the match had no book for takes the scratch's;
- the units entries are added (below);
- an instrument's last stated price (`Matched::last_price`, which `write_off_dust` sets on every row's fill) becomes the placed rows' only where no fill of that instrument after them states one;
- the account shares (`Engine::shares`) gain the rows' cash;
- the stretch is split at the rows: from its start to their first, between their own round trips, and from their last to its end;
- `last`, `opens_at_last`, `expired_at_end` and `ended_quiet` are unchanged, since nothing was walked after the family's last place.

**The units timeline.** `record_units` writes an entry when a holding's units differ from the timeline's last entry. In a walk in order, that is the entry on or before the row's day. A placed walk is not at the timeline's end, so as built the scratch timeline is seeded with the holding's entry on or before the rows' first day, and the seed is dropped after the walk. `record_units` itself is unchanged, and a whole walk is unchanged.

A same-day round trip ends that day at zero whichever entry it is compared with, so it cannot tell the two comparisons apart. The plant of the old comparison (the timeline's last entry) is caught by a round trip over days whose buy equals the units the holding holds at the end.

**Distributions inside the span (brief 60, change 1, withdrawn in its addendum).** The match gives a distribution to no round trip. `Matcher::apply` returns at once for a row naming no instrument, and `apply_distribution` returns for one no adjustment names (`ledger.rs`, `apply` and `apply_distribution`). `Trip` carries no income. Which round trip a payment answers a filter by is read when the scope is worked out (`scope.rs`, `entitled`), from the trades list and the declared distribution's ex-date. So a placed round trip that spans a payment answers for it as after a whole match. The test holds it with payments before, on and between the round trip's days and after it, and with one declared distribution gone ex inside the round trip and one before it.

**Figures.** The placed walk returns the round trips it wrote and the holdings it walked, like `Continued`, and `rederive` works on that reach (deep-history plan, C). Past days are forgotten from the change's first day (#527, 4).

### C. Held by the oracle

Under the `oracles` feature, every placed change is checked against matching the family whole on a copy. The engine's `Matched` within the family, `MatchState` including the stretches, and the cash shares must be equal, as for a continued match today (`Engine::same_as_whole`, shared by both). The path counter (`engine::Path`) records `Placed` as a third path beside `Continued` and `Whole`.

### D. The gate

`backdated-flat` comes off `OVER_BUDGET` when the capacity run shows it within the growth rule at both depths. The in-suite growth test asserts that its probe took the placed path.

## Acceptance criteria

- [x] `RUSTFLAGS="-D warnings" cargo test --workspace --locked` green on CI, clippy clean, and the book's oracles green.
- [x] **Placed equals whole, and is used.** `engine/tests/placed.rs`, over 10 seeds, adds round trips into stretches: the middle of a long one, with a payment before, on, between, on the sale's day and after; across one declared distribution's ex-date; a same-day trip alone and beside another family's row that day; one in a family with a share and its contract, flat on the day; and one after which no fill of its instrument states a price. They are interleaved with the cases that must go whole:
  - a same-day trip on the day its stretch starts, and on the day it ends;
  - one over two stretches;
  - a contract; a transfer; a coin;
  - a one-day record with a leg in another family;
  - a sale of more than is held; a buy that leaves the holding open;
  - a record whose legs are not stated, then a tainted family.

  After every step the engine equals a fresh build (`differences`, `match_disagrees`, the match of the whole ledger, what moved), and so does the cashflow a side filter reads through each payment's round trip. The path counter matches the step's label, with at least 110 placed steps (11 a seed). Every step that places is also held by the oracle.
- [x] **Each guard bites.** Planting the removal of each guard, the units seed, the realized sum and the last-price rule fails the test (the PR's table).
- [x] **The stretches are right.** The oracle compares the kept state, stretches included, with a whole match's after every placed and continued step. `engine/tests/continued.rs` still passes with the stretches in the state.
- [ ] **Capacity:** `backdated-flat` meets the growth rule at both depths and its budget on CI, and its `OVER_BUDGET` entry is removed. The runner's table is pasted in the PR. The in-suite growth test asserts the placed path on both books.
- [x] `docs/architecture.md` §13: a round trip added inside a flat stretch is placed there; anything else back-dated matches its family whole.

## Surfaces to check beyond the diff

- `ledger.rs`: `MatchState` (`open`, `stretches`, `untainted`), `match_family`, `walk` (`stretch_ends`, `stretch_starts`), `record_units` (the open holdings), `passes_over`, the new `place_in_stretch`.
- `engine.rs`: `continuable` and `placeable`, `untainted`, `continue_family`, `place_family`, `add_shares`, `same_as_whole`, `Path`.
- `capacity.rs`: `OVER_BUDGET`, the `backdated-flat` probe's path record, the in-suite growth test.

## Right to refuse

If the property test finds anything else a placed walk reads from the kept maps, or anything else that crosses a stretch, it gets a guard that sends the family whole, never a workaround. If the guards then send `backdated-flat`'s ordinary case (a buy and its sale of a listed share on a past flat day) whole, stop and bring the measurement back to the gate.

## Anti-stub self-check

Initialled at build:
- no kept state the placed walk does not read or write;
- no guard without a step in the test that trips it;
- no oracle compiled out of the test that cites it;
- the test, the plants and the in-suite growth test run, not asserted.

## Verification

Before the build: the table in "What brief 58 asked", from a temporary probe in `Matcher::walk` and a test running the capacity job's `backdated-flat` probe at depths 20 and 320 of the test book, and at 174 and 698 (the owner's depth and four times it). Master `551e281a`, the home server, 2026-10-19. Neither the probe nor the test is committed.

The build, on the home server: `cargo test -p bagholder-engine --test placed --test continued --test by_change --test past`, and `cargo test -p bagholder-server test_every_operation_is_measured_and_the_growth_rule_judged`, green. Clippy is clean on the engine and the server. The plants were run one by one against `placed`.

## Handoff

Built in the PR that carries this plan, after brief 60.

**Nothing left running.**
