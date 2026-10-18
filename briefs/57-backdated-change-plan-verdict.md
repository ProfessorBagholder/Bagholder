# Brief 57: Verdict on plan #536 (a back-dated change costs what it changed, #527): Go with changes

## Verdict

**Go with four changes.** The plan rests on a measurement, not a guess: each statement's steps were tallied at 80 and 320 round trips deep. It separates the work a change causes from the waste around it:
- a back-dated buy that keeps a position open joins every later round trip. That follows from the trade definition (flat to flat, as Tradervue groups by position), so one identity write per joined trip is the change's own reach;
- the waste is a transaction and a re-read around each of those writes;
- the late cash rows' growth sits in the engine (`continuable` refusing, `apply` forgetting from `Date::MIN`), not in SQLite.

**Step 2 stands for work, not for time.** Judging a change's work per unit of what it changed is the established measure for incremental computation: cost bounded by the size of the change in the output. It is what §13 already says ("work follows what changed"). A back-dated buy that joins 160 trips has 160 trips of work to do, and the growth rule's question is whether each costs the same at four times the depth. But wall time is what a person waits for, and that is never divided (change 1).

## Required changes

1. **Wall time is judged on the whole change, never per changed trip.** The plan's criterion "under the 100 ms budget per changed trip on a Pi" goes. §16's person-waits rows stay as they are, for the whole operation:
   - an added trade's request answered: 100 ms;
   - its figures on the stream: 1 s.

   If settling the joined trips runs inside the add request's answer, move it to the figures pass. The request answers once the row is written. The joined trips' identities settle with the figures, inside their 1 s, as §16 splits the two rows already.
   If `backdated-trade`'s time still misses at depth after steps 1 and 3, its `("backdated-trade", Depth, Time)` entry stays on `OVER_BUDGET` with #527. The list only shrinks when a measure is met.
   *Test:* the capacity job, unchanged in how it times. The per-trip divisor is applied to steps, allocations and bytes read, and the test that holds §16's table to the constants also asserts that the divisor is never applied to `Measure::Time`.
2. **The divisor is what the change did, checked.** The divisor read from the settling's result must equal the number of round trips whose identity differs between the book before and after the change, computed whole.
   *Test:* for generated back-dated buys and fills at generated depths, settling's count equals the whole recompute's count of moved identities. A divisor taken from anywhere else fails it.
3. **The first stale day is the earliest day the change touches, not the row's own day.** That is the earliest of every date the engine reads from the rows removed or added: trade date, settlement date, and a distribution's ex-date and pay date, where a row carries them.
   *Test:* the plan's every-day test, with rows whose dates differ, a settlement after the trade and an ex-date before the pay date, placed on each side of a generated D.
4. **The cash-only continuation is tested at its boundaries, not only anywhere.** The generated families and late rows in step 3's oracle test must include:
   - a distribution on the day a round trip opens;
   - one on the day a round trip closes;
   - one on a day with no trip open (paid after the sale, with an ex-date before it);
   - a cash row in a currency other than the account's.

   The continued match equals the whole match in every case, or that kind of row stays matched whole and the PR says which.

## Not changed

- Step 1 (one `Book::atomically` per settling) and its bounded-group fallback under *Right to refuse*.
- Step 2's new `backdated-flat` case, judged raw.
- The right to refuse on a cash row that changes a lot match.

Build it next, before #524. One push, with the capacity table in the PR.
