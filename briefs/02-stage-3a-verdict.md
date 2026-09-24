# Brief 02: verdict on the stage 3a plan

**Reviewed:** `docs/plans/stage-3a-sources.md` on `svelte-migration` at `1bde088a`.

## Verdict: Go with changes

Make the four required changes to the plan, push it, and build. Another review isn't needed unless a change alters the plan's scope.

**The two gated items: Go.**
- **Book migration 3** (distributions without kinds, the reinvested part, `fx_series`).
- **The distribution rate** as the cash per unit from the payer's own record, with the schedule the payer states.

Both follow the owner's decisions of 2026-09-23 and 2026-09-24.

**What's good:**
- research before each reader, answered from real replies and recorded as fixtures;
- one copy of the moved code;
- the three Bank of Canada eras that never overlap;
- `rate-not-held` kept apart from a failure of the source;
- the refusal to read a site by getting past its bot check.

## Required changes

**1. Mackenzie and WisdomTree: stated facts only, never a worked-out schedule.**
For the three funds whose company site the app cannot read (the owner's exception of 2026-09-24):
- **Distributions:** take them from the listing's exchange-side record (TMX, Yahoo), marked with that source. They are the fund's declarations as the exchange publishes them.
- **The schedule:** use it only where a source states it. Drop "else worked out from its ex-dates" (plan, "Distributions and schedules"). Working a schedule out from past dates is the inference the owner ruled out: it showed Ninepoint as monthly for six weeks after the change. Where no readable source states the schedule, the fund's annual income is a gap naming why (its company's site cannot be read), never a worked-out number.
- **The acceptance criterion** that says these funds are "shown as waiting on their payer" becomes: distributions from the exchange-side record; the schedule where stated, otherwise the income figure shown as a named gap. Update `stage-3a-research.md`'s "until the owner decides" to match.

**2. Option closes: no interim capture.** *(Revised: the earlier text asked for a way to capture closes before the Rust build runs every day. That was outside the owner's focus and is withdrawn.)*
- Drop the "daily run" question from the top of the plan. Nothing is built or run for the period before the Rust build is the app.
- A session day with no recorded close is covered by the plan's own rule: the equity series takes the broker's stated account value where there is one, and waits where there is none.

**3. Drop the 50% quote hold.**
- **Numbers the session picked, withholding real prices.** "More than 50% from the last close is held until the next read agrees within 1%, or a second source states it" uses figures that neither `SPEC.md` nor the owner chose. It withholds real prices on exactly the days that matter: a small-cap's big day, a halt that reopens, a gap on news.
- **It can't clear the way the plan expects.**
  - US listings have one quote source, so no second source can clear a hold.
  - Quotes are read on demand, so the "next read" may be hours away.
  - In a fast move, two reads a few seconds apart can differ by more than 1%, so the hold can repeat while the person watches.
- **It adds nothing.** The remaining checks already catch a reply that is wrong, not a price that is surprising: a time on every quote, the listing's currency, a positive price, and the series asked for.

Remove the rule from 3a. If a bad price from a source is ever seen for real, bring it back with that case as its fixture. Stage 4's rule that a trigger never fires on a single tick far from its neighbours is separate, and stays.

**4. Read only the payers a figure needs.**
"Every instrument the book holds or has been paid a distribution by" means an adapter per fund company, and a schedule to keep current, for funds long since sold. Limit the payers read to those whose rate a screen shows, by `SPEC.md`: held positions, and any past payer only if a screen shows its rate. Name that screen, or narrow the list to held positions.

## Also, not blocking

- **The switch's `SPEC.md` list** gains §2's "Distribution rate for a holding". Today it says TMX first, frequency from ex-date gaps, and 12 assumed; all three change with this part.
- **Showing the exchange-side source on screen.** "Every figure built on it names that source" is 3c's to show, and `SPEC.md` must say how, since the page shows no helper text beyond what `SPEC.md` lists. Until it does, the figure is marked in the data, not on screen.
- **Two findings for stage 4, written here so they aren't lost:**
  - The HTTP client this part moves into `bagholder-net` re-sends a request once on a fresh connection when the reply fails on a reused one (`market/src/client.rs:365-395`). That includes Wealthsimple's order POSTs (`ws/src/session.rs:433`), so an order can be sent twice. Move it as it is, and say in the plan's handoff that stage 4 limits the re-send to requests that are safe to repeat.
  - `docs/design-review.md` describes the fill bug as "50 filled, then 100, books 150". The Rust build doesn't book a partial fill: it books only when the order is fully filled (`readback.rs:399`). The real problem is two read-backs at once each booking the same fill, because the booking and its "booked" mark aren't one transaction and no lock is shared. The next sync then links neither and adds Wealthsimple's row (`store/src/merge.rs:196-203`). Correct the description there.
