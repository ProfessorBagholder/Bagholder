# Brief 58: #527's refusal on `backdated-flat` accepted. List it, and plan matching from a flat point next

## Verdict

**Accepted, as the CTO proposes.** This is engineering scope, settled here, not a decision for the owner.

A self-contained round trip inserted mid-history has no kept match state to continue from, because the match keeps its state only at the end of the history (the deep-history plan, B). Meeting the growth rule for it needs states kept inside the history, which is a change to the engine's design and needs its own plan.

For #536's build:
- **Add `("backdated-flat", Depth, Allocations)`** to `OVER_BUDGET` under #527. It is a defect newly measured by a new probe case, not a regression, so adding it doesn't break "the list only shrinks". The comment says what it waits for: matching from a point inside the history.
- **`("backdated-fill", Depth, Allocations)` stays** (3.95×). Its `Steps` entry comes off.
- **Everything else measured within the rule comes off,** as the CTO lists it.
- **#527 stays open** for those two measures. Brief 57's four changes stand.

## Matching from a flat point: the next plan, and what it must answer

File it as an issue now, with a plan through the gate after #524. The plan must start from this:

- **At a flat point the lot state is nearly empty.** When a position goes flat, no lot is open, and an average cost resets. So a state kept at each flat point costs almost nothing to keep. It is the counters that run across round trips, and one date per round trip, not a copy of the lots. The plan measures that against §16, but it shouldn't be assumed to be the memory cost the refusal feared.
- **What crosses a flat point is the question.** A rule whose window spans a flat point makes a flat point not a clean break. In this category that means:
  - Canada's superficial-loss rule (a loss with a repurchase within 30 days before or after);
  - the US wash-sale rule;
  - anything the engine carries from one round trip into the next: deposited and transferred lots, an option's assignment, an event's adjustment.

  The plan lists, from `rust/crates/engine`, everything that reads across a flat point. A continuation from a flat point is allowed only where nothing in that list reaches past it, and otherwise the family is matched whole as now.
- **Test:** the oracle's continued == whole check, over generated histories with inserted round trips on each side of every crossing rule's window.

## Addendum: brief 57's change 1, corrected

Brief 57 told #527 to move the settling of joined trips out of the add request if it sat there. That misread §16. An added trade is answered once its figures are recomputed, by design, so the page never holds round trips whose identities haven't settled. That is why one 100 ms row covers both the answer and the figures.

Answering on the write alone would change that for every entry: added trades, journal writes and import rows. It is a change to how the app runs, not a fix inside #527.

So:
- **#527 pushes as it stands.** `("backdated-trade", Depth, Time)` stays listed, and the PR's cost section says what the 195 ms is: the request answer, made of the engine's catch-up plus the settling plus the names reload.
- **A new issue:** measure how the 195 ms splits between catch-up and settling, at owner depth. Then make the smaller of the two changes that meets 100 ms.
  - If the settling dominates: make the settling itself cheaper inside the request. Don't defer it.
  - If catch-up dominates: that is #537's ground, matching from a point inside the history.
  - Deferring the figures out of the request is the last resort, and a plan of its own.
