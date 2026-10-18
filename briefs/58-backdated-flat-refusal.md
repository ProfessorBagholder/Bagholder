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
