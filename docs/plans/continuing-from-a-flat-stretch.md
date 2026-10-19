# Plan: a back-dated buy that stays open is matched from the flat stretch before it (#538)

## For the owner to decide

Nothing open. This is engineering scope set by brief 58's addendum and the gate's answer on #538.

## Scope

A buy added on a past day where the holding was open, or that leaves it open, joins every later round trip (`SPEC.md` §2, Trade). Today the engine then matches the holding's whole family from its first row, then works out the figures of every round trip on the instrument. The rows before the last point where the family was flat change nothing, but they are walked again anyway.

Measured on master `be8608fd` (the issue's comment): at four times your book's depth, the request takes 93.6 ms on the home server. Of that, 29 ms is the whole-family match and 8 ms the trades' figures. On CI that is 122 ms on a Pi, against 100 ms. #549 makes the settling's writes set-based (93.6 → 85.0 ms here, about 111 ms on a Pi). This plan takes the rest out of the catch-up.

Out of scope:
- **The settling and its catch-up.** That is #549.
- **A change that also revises or removes records.** It still matches whole.

## The old app here

The old app re-derived everything on any change. Nothing is carried over.

## What the measurement says first

On the depth books, with a temporary print in `Engine::continuable` (not committed), at the probe's buy (2025-04-07):

| Book | Family rows | Flat stretches kept | Latest stretch start before the buy | Rows after it |
|---|---|---|---|---|
| ×1 | 358 | 142 | 2025-01-21 | 208 |
| ×4 | 1,406 | 571 | 2025-01-24 | 800 (57%) |

Matching from that stretch's start walks 57% of the family instead of all of it. It also works out the figures of the round trips after it rather than all of them. Scaled from the breakdown, about 17 ms comes off at ×4, which takes the request to about 89 ms on a Pi.

Where the buy falls is the person's choice. The work is the rows from the flat point before it to the end, which is the change's own: every later round trip changes. A buy dated near the start of a history still costs about a whole match, as it should.

## How the leading products do it

Inventory systems recompute forward from a back-dated entry. ERPNext reposts "every subsequent stock ledger entry for that item-warehouse", never the ones before it ([ERPNext, FIFO and Moving Average](https://docs.frappe.io/erpnext/fifo-and-moving-average), cited in the deep-history plan). This plan does the same, from the flat point before the entry, because a lot can only start fresh where nothing is open.

## Approach

### A. A cut at each flat stretch's start

When the match starts a flat stretch (`Matcher::stretch_starts`, #548), it keeps a **cut**: what the match will need to walk on from that point, as it stands there. The cut holds:
- the matcher's maps (`MatchState` without `last`, `opens_at_last` and the stretches);
- each holding's book as of the cut: its taint and its basis, realized included. The lots are empty at a flat point, and so is each book's current round trip;
- each holding's units entry on or before the cut's day.

As built, a cut does not keep the last stated prices. A change walked on from a cut only adds rows, so every price stated after the cut is stated again by the walk. Planting the price take-out's removal failed no test.

#548 measured a full copy like this at every flat point, inside `backdated-flat`'s window: at most 31 entries, the same at every depth, for about 3% more allocations in that window. The cuts are kept in `MatchState` by the stretch's start.

**What every cut costs, across a whole book (brief 62, 1).** Measured at startup with `capacity probe startup` on the release builds of master `be8608fd` and of this branch. The peak resident memory comes from `time -f %M`. A cut's bytes are its entries' sizes (`Cut::bytes`).

| Book | Cuts kept | Their entries | Their entries' bytes | Peak resident, master → branch |
|---|---|---|---|---|
| Your book's size (breadth ×1) | 239 | 1,518 | 131 KB | 55.1 → 58.0 MB (+5%) |
| Four times it (breadth ×4) | 966 | 6,697 | 574 KB | 159.2 → 168.6 MB (+6%) |
| Your book's depth (depth ×1) | 468 | 2,364 | 206 KB | 58.7 → 62.7 MB (+7%) |
| Four times the depth (depth ×4) | 1,156 | 4,896 | 428 KB | 70.4 → 77.7 MB (+10%) |

These were measured while the cuts still kept the last prices, which the build then dropped (below), so the cuts as built are smaller. The peak's difference also covers #548's stretches and the maps' own nodes, not only the entries. The cuts grow with the history, as the book itself does, and a cut does not grow: about 550 bytes at your book's size and 370 at four times the depth. So every clean cut is kept, rather than spacing them geometrically. Spacing would walk the probe's buy from 1,024 rows back instead of 800, and would give up about half the saving. The in-suite growth test holds both: no more cuts than the history grows by, and no larger cut for a longer one.

A cut is kept only where it is **a clean break**, so that walking on from it is exactly what the whole walk does after it. That means no other row of the family on its day after it, so the units entry of its day is final and no event of that day is half applied.

As built, the plan's two other conditions were not kept, because no test could trip them:
- **A link spanning the cut.** A linked move's units arrive at its send, so a holding would be open across the cut, and there would be no flat point.
- **`lieu_fraction` left over.** The cut keeps the matcher's maps, that one included.

A stretch whose start is not a clean break keeps no cut, and a change before it walks on from an earlier cut.

**The cuts are a cache.** They are left out of comparing two states, and a cut is held to a whole match wherever it is used. Wherever a change could leave a cut stale, it is dropped:
- **a placed round trip** (#548) drops every cut after its stretch's start. Each of those held the match without the placed trip: its realized, its units and its last price;
- **a continued match** drops a cut on the day its rows start, which is no clean break once rows follow it on its day.

A family whose cuts were dropped walks on from the cuts before, or matches whole, until a later match keeps them again.

### B. Matching from the cut

`Engine::continuable` gains a third way for a family, beside continuing at its end and placing into a stretch: **from a cut**. It is used when a change's rows are placed before what the family matched and pass the deep-history plan's guards 1, 3, 5 and 6. It also needs:
1. an added row that is not an expiry, assignment, exercise or corporate event (guard 4 there: they decide earlier rows through `ended_on_record` and `event_groups`, which are computed over the whole family);
2. a kept cut on a day earlier than every such row: the latest one before the earliest row.

As built, a family is placed where every row can be placed in one stretch (#548), and walked from a cut otherwise. That covers rows over two stretches, a row on a stretch's own first or last day, a contract, a coin, a transfer, a record with a leg in another family, and a tainted family, since the cut carries the taint. A placement whose walk leaves the family otherwise than it found it (a buy that stays open, an oversell) also falls back to the cut before it rather than to a whole match.

Then:
1. Everything the match wrote after the cut is taken out of the engine's `Matched` within the family: round trips opened after it, roles and waiting entries of rows after it, units entries after its day, and list rows of rows after it (beyond, dust, arrivals, disagreements, stock dividends, rows not applied). The books, the last prices and the units entries on the cut's day are set to the cut's.
2. The family's rows after the cut, with the added ones, are walked by `match_over`'s own loop from the cut's maps. Where a row falls is decided by its place (`place_in_match`), read from the ledger's per-instrument index, not by scanning the family.
3. The kept state's stretches and cuts after the cut are replaced by the walk's.
4. The account shares gain the added rows' cash only, as for a continued match.

The walk returns the round trips it wrote and the ones it took out, the holdings it walked, and the stock dividends it took out and wrote again. `rederive` works on that reach (deep-history plan, C), so only the round trips after the cut are worked out again. A stock dividend's cashflow row is read from the match, so it is rebuilt for the rows reached as well as for the instruments reached whole.

### C. Held by the oracle

Under `oracles`, every match from a cut is checked against matching the family whole: the engine's `Matched` within the family, `MatchState` with its stretches and cuts, and the cash shares (`Engine::same_as_whole`). The path counter gains `FromCut`.

`engine/tests/placed.rs` is the property test for both ways (placed, and from a cut), over 10 seeds. Each step is labelled by the path it must take:
- **From a cut:** a buy that stays open and joins every later round trip; rows on a stretch's own first or last day; a round trip over two stretches; a contract's; a transfer's; a one-day record with a leg in another family; a sale of more than is held; a tainted family; a coin.
- **Existing rows after the cut (brief 62, 2):** a buy that stays open, before a split, a move between accounts linked on both sides, a return of capital, and a put written and assigned.
- **Whole:** an added expiry row; an added corporate event; a record whose legs are not stated.

After every step the engine equals a fresh build: `differences`, `match_disagrees`, the whole ledger's match, what moved, and the side-filter cashflow. `engine/tests/by_change.rs` and `continued.rs` run with the cuts as well.

After every step the engine equals a fresh build (`differences`, `match_disagrees`, the whole ledger's match, what moved, and the side-filter cashflow).

### D. The gate

`("backdated-trade", Depth, Time)` comes off `OVER_BUDGET` when CI's capacity run shows the request within 100 ms on a Pi. The in-suite growth test asserts the probe took the from-a-cut path.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace --locked` green on CI, clippy clean.
- [ ] **From a cut equals whole, and is used:** the property test above, with a stated number of from-a-cut steps.
- [x] **Each guard bites:** planting the removal of the clean-break condition, the cut's day rule, each part of the take-out in B.1, the stock dividends' rows, and each place a cut is dropped as stale fails the tests (the PR's table).
- [x] **Memory:** the cuts across whole books at both sizes (A's table), and the growth test holds their count and size.
- [ ] **Time:** CI's capacity run shows `backdated-trade` within 100 ms on a Pi and every other operation within its rule (brief 62, 3: judged on CI alone, with no margin claimed). The entry comes off only on that run.
- [ ] `docs/architecture.md` §13: a back-dated row that keeps a position open is matched from the flat point before it.

## Surfaces to check beyond the diff

- `ledger.rs`: `MatchState` (`cuts`), `stretch_starts`, a `match_from_cut` beside `place_in_stretch`, and the take-out of what was written after a cut.
- `engine.rs`: `continuable` and its third way, the `oracles` path, `Path::FromCut`.
- `capacity.rs`: `OVER_BUDGET`, the in-suite path assertion.

## Right to refuse

If the property test finds more state that reaches across a cut, the cut's clean-break conditions grow. A cut is never patched up after the fact. If those conditions then leave the probe's buy with no cut before it, stop and bring the measurement back to the gate.

## Anti-stub self-check

Initialled at build:
- no cut part the walk does not read;
- no condition without a step that trips it (the ones no test could trip were removed, with the reason above);
- the oracle runs on every from-cut step;
- the tests, the plants and the growth test were run, not asserted.

## Verification

The table in "What the measurement says first": a temporary print in `Engine::continuable`, the release build of `be8608fd` with #548, on the depth books built by `capacity build --deep` (a local flag, not committed), on the home server, 2026-10-19.

## Handoff

Built after the gate's verdict, in the PR that carries this plan.

**Nothing left running.**
