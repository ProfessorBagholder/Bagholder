# Brief 09: verdict on the stage 3c plan

**Reviewed:** `docs/plans/stage-3c-switch.md` at `aaea5313`. Each finding was checked against the code at that commit and against outside practice (`briefs/reference.md`).

**Verdict: Go with changes.** Put the changes below into the plan, then build. The plan needs no further review.

## Owner decisions (2026-09-25)

Record each in `docs/decisions.md` with the test that holds it, or "review only".

1. **The owner's list holds only the owner's choices.**
   - It contains a real choice between options the owner cares about, or a departure from one of the owner's standing rules.
   - A correctness fix does not go on it. It goes into `SPEC.md` with its reason and is checked at the gate. This replaces brief 01 §2.2, which asked for every figure change.
   - Of the plan's 13 items, only 9 and 13 were the owner's. Both are decided below. Every other item stands, with the corrections under "Required changes".

2. **Tooltips: the rule as written is wrong, and every session has misread it.**
   - The owner's rule is:
     - no browser `title` tooltips, which are unstyled and were scattered through the app without being asked for;
     - a hover tooltip wherever one is needed, always the app's own styled tooltip, one look everywhere.
   - Beyond "nothing the owner didn't ask for", there is no rule against text on screen.
   - Reword `CLAUDE.md:41`, `:42` and `:85`, `SPEC.md:13` and `docs/decisions.md:59` to say exactly that.
   - Held by a test that fails on a `title` attribute anywhere in `web/src`. There are none today.

3. **Item 9 goes ahead as planned.** `—` appears with its word in the figure's place, and a total's second line counts what it left out.

4. **Clear data clears everything.**
   - A checkbox for each kind of data the app stores, plus Clear all, which ticks every box. After Clear all nothing is left, including the Wealthsimple login.
   - Disconnect stays.
   - `SPEC.md` §4 The menu changes, and so does §Order ticket, Submit ("`Clear data` does not touch them").
   - One safety rule: while a bracket is live, Clear data names it and does not run until the bracket is cancelled in the Orders panel. Deleting its record would leave its stop resting at Wealthsimple with nothing watching it.

5. **Time is correct for whoever is looking, wherever they are.**
   - Times of day are shown in the viewer's local time, as `SPEC.md` already says.
   - Days, months, years and "today" use the zone of the browser in use.
     - Each page states its zone when it opens the stream. The server uses the latest zone stated, and keeps it in the book for when no page is open.
     - This replaces the plan's recommendation 12 and its open question 2.
   - A date a source states as a date is never converted through a zone: ex, record and pay dates, a daily value's day, a close's session. Only moments are converted.
   - The server's own zone is never used.

6. **The Equity curve card gets a switch.**
   - It sits at the right of the card's title, as the Annualized returns card's does, and is remembered on this machine.
   - **`P&L`** (the default) is the equity curve as trading journals define it (TradingView, Tradervue, TradeZella; `reference.md`).
     - It is the running total of Realized P&L in scope, each realized part on its own day.
     - It follows every filter.
   - **`Value`** is the account value series of today (`SPEC.md` §2, Equity series).
     - It follows the account filter.
     - The card names the filters it does not read, as Cashflow does.
   - Returns, the index comparison and Max drawdown stay on the account value series, because a time-weighted return is how brokers set a return against an index (IBKR, Wealthsimple; `reference.md`).
   - Build it in this stage as its own step, after the figures are on the engine.

7. **AI agents reach the app through its MCP tools only.**
   - Remove `docs/architecture.md` §12's paired devices and its tokens with access classes (lines 172-173). The owner never asked for them and argued against paired devices.
   - The Host/Origin check and the page's write token stay.
   - The design doc never adds a capability the owner has not asked for. Anything in it the owner has not asked for (check §14) goes to the owner as one line and is not built.

## Required changes to the plan

1. **Distributions, for any holding (item 7).**
   - **Today only a fixed list is read:** 17 fund companies (`sources/src/payers/mod.rs:117-138`) and 9 named companies (`payers/companies.rs`).
     - A holding outside the list waits forever (`payers/run.rs:78-81`).
     - `exchange.rs` calls itself "a fixed list, not a fallback".
     - The owner requires it to work whatever the symbol.
   - **The design:**
     - Every held payer gets the market's record of its distributions: TMX for a Canadian listing, Yahoo's dividend events for a US listing. These are the readers `exchange.rs` already uses. The schedule is taken wherever that record states it.
     - Where a reader exists for the payer's company, its record is used instead, since the company states a schedule change first (brief 01).
     - A failed read by an existing company reader is that source's failure: the last record is kept, the read is retried, and the header shows it. It is never a switch to another source.
     - The income figure waits only where no source states the schedule.
   - No symbol or fund company is a case of its own in the plan, the docs or the owner's list. Brief 02 named three funds; that framing is withdrawn.
   - **Test:** on recorded replies, a payer that no company reader serves gets its record from the market's.

2. **Avg annualized** (`engine/src/stat/returns.rs:147-163`).
   - **Nothing under a year is annualized.** This is the rule in GIPS and at Sharesight; Hamilton's own page in our fixtures says "Only the returns for periods of one year or greater are annualized returns".
     - The code annualizes from one month up, so two months at +15 % reads +131 %.
     - Under a year, the tile shows the return over the period. `SPEC.md` line 86 changes.
   - **Days are counted from the base day.**
     - A year's return runs from the last value before 1 January, but `days` counts from 1 January.
     - So a full year counts as 364 days, and five years at 10 % read 10.04 %.
   - **Tests:** both cases.

3. **Daily closes** (the plan's table, "daily closes and benchmarks"): drop "for what is held".
   - The owner stopped past closes of every holding (brief 06).
   - Item 6's last close is the last price read, kept in the cache with its day. No date is printed.

4. **Item 4, expiry.**
   - A contract past expiry with no row closes at zero only if it expired out of the money.
   - At or in the money it waits for Wealthsimple's row (`no-expiry-record`). OCC exercises a contract $0.01 or more in the money unless told not to, so only the broker's row says what happened.

5. **Clear data** (the plan's §7 and its criterion): as decision 4.
   - The test lists every table in the book, the cache and the old store.
   - It fails on any table that Clear all leaves filled, and on any table it does not know about.

6. **Time** (the plan's §2, "The home zone", and its criterion): as decision 5.
   - The test holds for every zone, not for one chosen zone. It runs every zone in the time-zone database as the page's zone, with the server process in several zones, UTC among them.
   - Every moment, including those around daylight-saving changes, falls on its day in the page's zone.
   - A stated date never changes.
   - A page that then states another zone moves "today" to that zone's.
   - Plans and docs name no particular zone.

7. **The real run** runs with:
   - `BAGHOLDER_DRY_ORDERS=1` and `BAGHOLDER_NO_BROWSER=1`;
   - the order and bracket loops off;
   - every Wealthsimple request counted and listed in Verification.

8. **Land it in steps, each green.** The steps, in order:
   1. parity baselines;
   2. the server's state and scheduler;
   3. the wire;
   4. the page;
   5. CSV;
   6. Clear data;
   7. the Equity switch;
   8. removing the old path.

   - Each step is a commit with the whole suite green.
   - A baseline changes only where a `SPEC.md` change in that step names it.

## For the owner

One step, when CTO sets up the real run: sign in once on the scratch copy.
