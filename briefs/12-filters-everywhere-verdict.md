# Brief 12: verdict on the filters-everywhere plan

**Reviewed:** `docs/plans/filters-everywhere.md` at `b951e5cc`. Checked against `master` at `8af32571`, `SPEC.md` §4 and §5, and `docs/decisions.md`.

**Verdict: Go with changes.** Changes 1 to 6 are required.

The aim is right: one filter set, and every figure narrowed by every filter that can describe it. TradeZella and IBKR both narrow drawdown and returns by the period. Findings are ranked, most severe first.

## Required changes

Checked page by page against what each page is for:
- the Dashboard and Trades list review how trades went;
- the Portfolio shows what is held now;
- Cashflow shows income;
- the Orders panel manages live orders.

A filter reads, on each page, the value that page shows. So the plan's holding filters stand: side, journal, Avg, Hold, unrealized P&L and units held are the Portfolio's own columns. An earlier version of this brief asked for the Trades list's values instead. It is withdrawn: they would hide a holding under a range that the numbers on its own row satisfy.

**1. An open trade has no result, on the Trades list and the Dashboard.**
- **Where:** `outcome` (`engine/src/scope.rs:208-218`) reads the P&L realized so far.
  - An open trade with no sale reads Breakeven.
  - A partly sold one reads Win or Loss by its sale.
- **The conflict:** this contradicts the owner's decision of 2026-09-24, that a trade's closed-trade figures count once it has closed. TraderSync gives an open trade the status Open, with no win or loss.
- **Failing scenario:** Result = Breakeven lists every open position no one has sold from.
- **Fix:**
  - `outcome` is `None` for an open trade, so on these pages a Result filter keeps closed trades.
  - On the Portfolio, Result reads the sign of the unrealized P&L it shows, as the plan says.

**2. A payment belongs to the round trip entitled to it, on the ex-date.**
- **Where:** plan lines 19 and 46: "the round trip that held the units on the day paid".
- **Failing scenarios:**
  - Held through the ex-date and sold before the pay date: no trip holds units on the pay date, so under any grade, tag or side filter the payment is dropped, though that trip earned it.
  - Sold and bought back before the pay date: the payment goes to the new trip, which was not entitled to it.
- **Fix:**
  - Entitlement is holding at the record date. The ex-date equals the record date under T+1, the rule `sources/src/payers/companies.rs` already applies.
  - A payment belongs to the trip open at the close of the session before the ex-date. The ex-date comes from the payer's declared record that the payment matches.
  - A payment with no matching declared ex-date is not placed by its pay date. It is left out under a grade, tag or side filter.

**3. A ratio's two sides share one scope.**
- **Where:** `margin_used_pct` (`scope.rs:784`) divides the margin used by the accounts in scope, an account figure, by `market_value`, which is summed from the filtered holdings (`scope.rs:707-712`).
- **Failing scenario:**
  - A margin account holds $100,000 with $20,000 of margin used.
  - A tag filter leaves $5,000 of holdings in scope.
  - Margin used then reads 400 %.
  - This already happens under a symbol filter. The plan extends it to every holding filter.
- **Fix:** an account figure is never divided by a filtered sum. The margin percentage is over the whole market value of the accounts in scope.

**4. The Orders panel keeps the account filter alone, as `SPEC.md` says.**
- **Why:** the panel is where live orders are managed. A filter set to review trades must not hide an order resting at Wealthsimple.
- **Where:** the header's badge counts through the same filter (`server/src/orders/doc.rs:547-555`).
- **Failing scenario:**
  - With Symbol = one listing, a bracket's resting stop on another listing disappears from both the panel and the badge.
  - Nothing on screen says so.
  - The account filter is different: the panel's heading names the accounts in scope.
- **Fix:** drop the plan's Orders row.

**5. The Portfolio's date question isn't the owner's. Take (a).**
- **What (a) does:** for every preset ending today it narrows nothing, since every holding is held today. Only a range ending in the past hides the holdings bought after it.
- **Why not (b):** positions as of a past date are a new capability. The owner's rule (2026-09-26: only the migration) rules it out.
- Nothing goes to the owner.

**6. The pre-history floor stays in returns and drawdown, over the whole history.**
- **Where:** plan line 28 says the old app's 1 % cutoff had "no stated reason" and is not carried over.
  - For where the curve's line starts, that is fine.
  - But `stat/returns.rs` still uses the floor for the yearly returns and the drawdown.
- **Fix:**
  - The plan says the floor stays for returns and drawdown, until its own plan replaces it (below).
  - The floor is taken from the whole history in scope, never from the date range. Otherwise the same year's return would change with the range chosen.

## In the same commit

- Update `docs/decisions.md:23` ("`Value` … following the account filter"). The plan changes a recorded decision, so write the new rule and why: the date range narrows the value too. Brief 09's wording never weighed the date range.
- Replace `SPEC.md` §5's "Trades are scoped by close date" with the rule the code has: open at any time in the dates chosen, with each sale counting on its own day.
- Rewrite `docs/decisions.md:45`, which still names funds and companies. Replace it with the rule as `payers/exchange.rs` states it: by the listing's market, naming no symbol, brand or company (brief 09, decision 8).

## Not blocking: a plan of its own

The floor is 1 % of the all-time peak, so it erases real years as an account grows. It came from the old app (the plan cites `model.py` 1821, 1960, 3019). It was carried into `SPEC.md` and the engine without the check `CLAUDE.md` requires.
- **Scenario:**
  - An account held $500 through 2019. It reaches $60,000 in 2026.
  - The floor becomes $600, so 2019 never clears it.
  - 2019 disappears from Annualized returns and from Avg annualized.
- **Why it isn't needed for its stated reason:** that reason is that the first deposit must never read as a return, and the time-weighted return already removes deposits.
- **What remains:** the real risk is a tiny balance, where a cent's move reads as a large return.
- **The ask:** that needs a rule of its own, argued in its own plan, not a floor measured against a peak years later.
