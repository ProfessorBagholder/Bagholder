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

`Engine::continuable` lets a change with rows placed before the family's last walked place continue as a **placed** change, only if every one of these holds. Otherwise the family is matched whole, as now:

1. Guards 1, 3, 5 and 6 of the deep-history plan hold (only added records; families unchanged; no finer coin places; nothing the match reads of an instrument or an account changed), as they do for a continued change today.
2. Every leg of every added record falls strictly inside one kept stretch: after its start's instant, and before its end's instant. The stretch holds no other row of the family that moves a lot.
3. Nothing in the family is tainted: every book's taint is empty, and so are the three multi-leg taints. Taints never clear, so none at the end means none at any point.
4. No added row is on a contract or a coin, is a transfer, an expiry, an assignment, an exercise or a corporate event, or names an adjustment.
5. Walked from an empty state, the added rows leave every holding they touch flat again, and they write nothing to the match's lists (beyond what was held, dust, arrivals, disagreements, stock dividends, what was not applied).

**How it is placed.** The added rows are walked by `match_over`'s own loop, from a state with no lots, using the kept `MatchState`'s maps unchanged. By guards 3 and 4 nothing in those maps is read by these rows. The walk writes its round trips, roles, units entries and books into the engine's `Matched` in place, as a continued match does. Then:
- each touched holding's `Basis::realized` gains the stretch's realized amount;
- the account shares (`Engine::shares`) gain the rows' cash;
- the stretch is split in two at the added rows;
- `last`, `opens_at_last`, `expired_at_end` and `ended_quiet` are unchanged, since nothing was walked after the family's last place.

If guard 5 fails after the walk, the engine throws the attempt away and matches the family whole. This is decided before anything is written, on a scratch `Matched`.

**The units timeline.** `record_units` writes an entry when the units differ from the timeline's last entry. In a walk in order, that is the entry before the row's day. A placed walk is not at the end of the timeline, so it compares with the entry on or before the row's day. A whole walk is unchanged by this.

**Figures.** The placed walk returns the round trips it wrote and the holdings it walked, like `Continued`, and `rederive` works on that reach (deep-history plan, C). Past days are forgotten from the change's first day (#527, 4).

### C. Held by the oracle

Under the `oracles` feature, every placed change is checked against matching the family whole on a copy. The engine's `Matched` within the family, `MatchState` including the stretches, and the cash shares must be equal, as for a continued match today. The path counter records "placed" as a third path beside "continued" and "whole".

### D. The gate

`backdated-flat` comes off `OVER_BUDGET` when the capacity run shows it within the growth rule at both depths. The in-suite growth test asserts that its probe took the placed path.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace --locked` green, clippy clean, and the book's oracles green.
- [ ] **Placed equals whole, and is used.** `engine/tests/by_change.rs` gains generated families with round trips inserted into each kind of stretch: the first, a middle one, the last open-ended one, a stretch between two accounts' trips, and a family with a share and its contracts flat on the day. They are interleaved with every case that must go whole:
  - a round trip that ends open, or that oversells;
  - a round trip at a stretch's start instant or end instant;
  - one spanning two stretches;
  - a tainted family;
  - a coin or a contract;
  - a transfer, an event, an adjustment;
  - a one-day multi-leg record with a leg outside the family;
  - an expiry or assignment row.

  After every step the engine equals a fresh build (`match_disagrees`). The path counter matches the generator's label on every step, with at least a stated number of placed steps.
- [ ] **Each guard bites.** Planting the removal of each of guards 2–5, and of the units-timeline change, fails the property test. The plants are shown in the PR.
- [ ] **The stretches are right.** After every step of the same sequences, the kept stretches equal those of a whole match.
- [ ] **Capacity:** `backdated-flat` meets the growth rule at both depths and its budget, and its `OVER_BUDGET` entry is removed. The runner's table is pasted in the PR. The in-suite growth test asserts the placed path.
- [ ] `docs/architecture.md` §13: a round trip added inside a flat stretch is placed there; anything else back-dated matches its family whole.

## Surfaces to check beyond the diff

- `ledger.rs`: `MatchState` (the stretches), `match_family`, `continue_family`, the new `place_in_stretch`, `record_units`.
- `engine.rs`: `continuable` (the placed path and its guards), `continue_family`, `Reach`, the `oracles` paths.
- `capacity.rs`: `OVER_BUDGET`, the in-suite growth test.

## Right to refuse

If the property test finds anything else a placed walk reads from the kept maps, or anything else that crosses a stretch, it gets a guard that sends the family whole, never a workaround. If the guards then send `backdated-flat`'s ordinary case (a buy and its sale of a listed share on a past flat day) whole, stop and bring the measurement back to the gate.

## Anti-stub self-check

*Initialled when built.*

## Verification

Before the build: the table in "What brief 58 asked", from a temporary probe in `Matcher::walk` and a test running the capacity job's `backdated-flat` probe at depths 20 and 320 of the test book, and at 174 and 698 (the owner's depth and four times it). Master `551e281a`, the home server, 2026-10-19. Neither the probe nor the test is committed.

## Handoff

Built in the PR that carries this plan, after the gate's verdict.

**Nothing left running.** *Filled when built.*
