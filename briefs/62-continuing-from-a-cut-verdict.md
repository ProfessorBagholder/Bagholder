# Brief 62: Verdict on plan #550 (a back-dated buy matched from the flat point before it, #538): Go with changes

## Verdict

**Go with three changes.** This is the forward recompute the category uses: ERPNext reposts from a back-dated entry on, never before it. It starts from the one point where a lot can begin fresh, a flat point. The design has sound guards:
- a cut is kept only at a clean break: no link spans it, nothing is left in lieu, and its day's units are final;
- expiry, assignment, exercise and event rows that are added go whole, because they decide earlier rows;
- the output after the cut is taken out and walked again;
- the oracle checks every match from a cut against a whole match;
- the right to refuse says a cut is never patched up after the fact.

## Required changes

1. **Measure what the cuts hold for the whole book, not one window.** #548's figure (at most 31 entries a point, about +3% allocations) was measured inside `backdated-flat`'s probe window. The cuts are kept permanently, at every clean flat point of every family: 571 stretches in one family at four times the depth. Measure the resident bytes of all cuts across the owner-size book and four times it, and paste them in the PR beside §16's idle-memory figure.
   If they grow with the history in a way §16 won't carry, keep cuts at **geometrically spaced points** instead of at every clean break: the latest, then about every doubling of rows back from the end. That keeps O(log n) cuts, and the walk from the nearest cut at or before a row is at most twice the walk from the ideal one. That is the standard checkpointing trade-off. Name the factor's source in the code.
   *Test:* the in-suite growth test asserts the cuts' count and bytes grow under the growth rule between the two depths.
2. **The generator includes rows already after the cut, not only rows added.** Matching from a cut re-walks existing rows with maps the cut kept. So the property test must also hold families whose rows after the cut include, as existing rows:
   - a corporate event;
   - a transfer with both sides after the cut;
   - a contract's expiry or assignment;
   - a coin with its source's places;
   - a distribution with a return of capital (an adjustment).

   Each step equals a fresh build, as written.
3. **Time is decided by CI, with no margin claimed in advance.** About 89 ms on a Pi is an estimate scaled from the home server. `("backdated-trade", Depth, Time)` comes off only when CI's capacity run shows it within 100 ms. If it doesn't, the entry stays and the PR says by how much. `PI_SLOWDOWN` isn't adjusted to get there.

Build in one push.
