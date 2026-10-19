# Brief 61: #548 (#537) at `58ee5409`: cleared; merge once every run is green

## Verdict

**Cleared.** Brief 60's change 2 is built: same-day trips by name, and the old-comparison plant caught by the multi-day trip whose buy equals the end units. Change 1 was withdrawn (brief 60, addendum), and the payment cases stay in the test.

Three changes from the plan, all recorded under B, are accepted:
- **the scratch timeline is seeded** with the entry on or before the day, and `record_units` itself is unchanged;
- **the last stated price is carried** only where no later fill states one;
- **guards no test could trip are dropped.** They are sent whole by the walk's own check or by guard 1 first, the same reasoning as brief 60's addendum.

`backdated-flat` comes off `OVER_BUDGET`, and the in-suite growth test asserts the placed path on both books. Capacity run 39195541139 is green. Merge once Tests 39195541115 is green on every job.

## Planted here

Run under the build lock on the warm build: `cargo test -p bagholder-engine --features oracles --test placed`.

| Change undone | Result |
|---|---|
| none (baseline) | passes |
| the taint guard (`untainted`) removed from `placeable` | fails: a tainted family's step took the placed path where its label says whole |
| a row on the stretch's end day placed (`e.0 <= day` → `e.0 < day`) | fails, the same way |

Both are caught by the path labels, which encode which cases must go whole. The oracle's equality check sits behind them.

## After the merge

#537 closes, and `OVER_BUDGET` holds only `("backdated-trade", Depth, Time)` → #538. Next: #538, then #542, then plan #535's build. Brief 48 waits for 2026-10-28.
