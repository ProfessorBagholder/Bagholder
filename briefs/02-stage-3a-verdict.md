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

**1. Mackenzie and WisdomTree: the plan contradicts itself.**
- "Distributions and schedules" follows the owner's exception of 2026-09-24: these three funds take the listing's exchange-side record (TMX, Yahoo).
- The acceptance criteria still say "Mackenzie's and WisdomTree's funds shown as waiting on their payer, named", and `stage-3a-research.md` still says they wait "until the owner decides".

Make the criterion test what the approach says, and update the research note.

**2. Option closes are lost every session until something reads them.**
The plan says rightly that no source gives a contract's close later. But in 3a the readers run only when someone runs the command, the server calls them only from 3c, and the owner runs the Python app until cutover. Every session day from now until then is a close lost for good. Before building the option-close reader:
- check whether the running Python app's database already keeps any per-day option prices that can be imported as recorded closes, and import them if so;
- say how the closes will be captured each session from 3a's landing until the Rust build runs every day on the owner's machine.

If capture needs something running daily on the owner's machine, that is the owner's decision. Put it at the top of the plan with the cost of not doing it (the days the equity series will wait or take the broker's figure).

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
  - `docs/design-review.md` describes the fill bug as "50 filled, then 100, books 150". Neither app books a partial fill: both book only when the order is fully filled (`readback.rs:399`; `python/bagholder.py:4819-4823`). The real problem is two read-backs at once each booking the same fill, because the booking and its "booked" mark aren't one transaction and no lock is shared. The next sync then links neither and adds Wealthsimple's row (`store/src/merge.rs:196-203`). Correct the description there.
