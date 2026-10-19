# Brief 63: #538 closing: #549 and #550 merged, #551 cleared with the Time entry off

## Verdict

**#551 at `0e1cbd07` is cleared.** Merge once Tests 39210948205 and Capacity 39210948239 are green on every job. It closes #538. If capacity measures `backdated-trade` over 100 ms, the entry goes back with the measured figure, and #538 stays open.

## The path here

- **#549** made the settling's orphan writes set-based: 23.4 → 11.1 ms at four times the depth. Brief 49's rules held, the row triggers stayed, and the anchor was checked inside the write.
- **#550** matched a back-dated buy from the clean flat point before it (brief 62). It keeps every clean cut, 370–550 B each and flat with depth. CI measured 100.2 ms on a Pi against 100.0, so the entry stayed.
- **#551** removes the from-cut walk's overheads. It reads only the rows after the cut's day, takes out through the family's instruments, and moves earlier cuts instead of copying them. The median request went 87.8 → 69.3 ms here.

My guess in brief 62's follow-up, that the settling's second catch-up matched the family whole, was wrong. The CTO measured it: that catch-up re-derives the orphaned trades only.

**Why the entry came off before the run:** the capacity job fails when a listed measure meets its budget. Leaving the entry on while expecting it to pass would have cost a second full run. Taking it off lets the one run decide either way, and that is consistent with brief 62, change 3.

## Planted here

On #550's head, a cut on the added row's own day (`c.0 < t.trade_date` → `<=`) fails both `by_change` tests at the oracle.
