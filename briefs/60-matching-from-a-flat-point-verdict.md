# Brief 60: Verdict on plan #548 (a round trip added on a past flat day, #537): Go with changes

## Verdict

**Go with two changes.** The plan answers brief 58's two points with evidence:
- **the cost, measured:** a full state at every flat point is at most 31 entries, the same at the owner's depth and four times it, for about +3% allocations;
- **what crosses a flat point, read from the engine:** taints for the whole life, coin places, an expiry decided ahead, links, events, and the units timeline. Each is held by a guard that sends the family whole.

The tax-window note is right. The engine applies no superficial-loss or wash-sale rule, and the plan says how one would join the guards. The design keeps two order keys per stretch, places only what falls strictly inside one, checks it against a whole match under the oracle, and has a third path in the counter. That is the snapshot-as-cache pattern, with the record always the source.

## Required changes

1. **Rows the walk passes over that fall inside the added round trip's span.** A stretch can hold rows the walk passes over (`walk_passes_over`): a distribution no adjustment names, a row with no instrument. Guard 2 forbids only rows that move a lot, so such a row may fall between the added buy and its sale. A whole match attributes a distribution's income to the round trip open on its day (#527, step 3). Once the added trip is open across that day, the whole match gives the distribution to the new trip, and the placed walk, which starts from the added rows alone, does not.
   - Either **guard 2 forbids any family row** dated within the added rows' span, on its days included, for every date the engine reads from it (#527, change 3: ex-date and pay date as well as the row's day);
   - or **the placed walk also walks the passed-over rows** within that span, in their places.

   Choose by which leaves `backdated-flat`'s ordinary case on the placed path. Say which in the PR.
   *Test:* the generator inserts round trips around distributions that sit inside stretches:
   - the distribution's day before the buy, on the buy's day, between the buy and the sale, on the sale's day, and after it;
   - a distribution whose ex-date and pay date fall on either side of the added trip.

   The placed result equals whole on every step, or the step's label says whole.
2. **The same-day round trip is in the generator by name.** `backdated-flat`'s own case is a buy and its sale on one past day. That is where `record_units`' new comparison, with the entry on or before the row's day, meets a day whose units go up and back to the same value. The whole walk's entries on that day must be equalled exactly.
   *Test:* same-day trips, generated:
   - alone;
   - next to another holding's row that same day;
   - on the day a stretch starts or ends, which goes whole.

   Plant the old comparison (with the timeline's last entry), and the test fails.

Everything else stands as written, including the right to refuse if the guards send the ordinary case whole. Build it next, in one push.

## Addendum: change 1 withdrawn

Change 1 rested on #536's plan text: a distribution's income goes "to the round trip open on its day". The code doesn't do that.
- The match gives a distribution to no trip. `apply` returns for a `Dividend` into `apply_distribution`, which returns at once with no adjustment (`ledger.rs`). `Trip` has no income field.
- A payment answers to a trip at read time, from the trades list and the ex-date (SPEC filters-8, in the scope).

So a guard, or a walk of the passed-over rows, would change nothing that any test could catch, which is the kind of guard the deep-history plan rejects. **No guard and no extra walk.**

The generated distributions stay in the test, around and inside the added trip, with the ex-date and pay date on either side. Each step equals whole (`Matched`, `MatchState`), a fresh build's trades and cash rows, and the scope's entitled filter. The PR states the reason, citing those lines. Change 2 stands.
