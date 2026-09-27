# Brief 12: verdict on the filters-everywhere plan

**Reviewed:** `docs/plans/filters-everywhere.md` at `b951e5cc`. Checked against `master` at `8af32571`, `SPEC.md` §4 and §5, and `docs/decisions.md`.

**Verdict: Go with changes.** Changes 1 to 6 are required.

The aim is right: one filter set, and every figure narrowed by every filter that can describe it. TradeZella and IBKR both narrow drawdown and returns by the period. Findings are ranked, most severe first.

## Required changes

**1. A holding is in scope exactly when its open trade is.**
- **The conflict:**
  - The plan filters a holding by its own fields (plan line 18): the sign of its unrealized P&L, its average cost, its unrealized P&L, the units held.
  - The Trades list filters the same open trade by its trade fields (`engine/src/scope.rs:244-257`): the P&L realized so far, entry over all units opened, units opened.
  - One trade can then be in scope on one page and out on the other.
- **Failing scenario:**
  - 100 units bought at $10; 50 sold at $12, realizing +$100; the price is now $8.
  - Result = Win: the Trades list shows it (realized is positive), and the Portfolio hides it (unrealized is negative).
  - Qty ≥ 100: it is in the Trades list (100 opened) and out of the Portfolio (50 held).
- **Fix:**
  - `position_matches` decides a holding by `trade_matches` on the open trade that holds it. The Portfolio then shows exactly the open trades the Trades list shows under the same filters.
  - A holding with no trade (a managed account's, brief 08) is out of scope under any trade filter: grade, tag, side, result, the ranges.
  - The instrument filters (account, symbol, search, kind, exchange) apply to every holding, with or without a trade.

**2. An open trade has no result.**
- **Where:** `outcome` (`scope.rs:208-218`) reads the P&L realized so far.
  - An open trade with no sale reads Breakeven.
  - A partly sold one reads Win or Loss by its sale.
- **The conflict:** this contradicts the owner's decision of 2026-09-24, that a trade's closed-trade figures count once it has closed. TraderSync gives an open trade the status Open, with no win or loss.
- **Failing scenario:** Result = Breakeven lists every open position no one has sold from.
- **Fix:** `outcome` is `None` for an open trade, so a Result filter keeps closed trades only. With finding 1, the Portfolio then shows no holdings under a Result filter.

**3. A payment belongs to the round trip entitled to it, on the ex-date.**
- **Where:** plan lines 19 and 46: "the round trip that held the units on the day paid".
- **Failing scenarios:**
  - Held through the ex-date and sold before the pay date: no trip holds units on the pay date, so under any grade, tag or side filter the payment is dropped, though that trip earned it.
  - Sold and bought back before the pay date: the payment goes to the new trip, which was not entitled to it.
- **Fix:**
  - Entitlement is holding at the record date. The ex-date equals the record date under T+1, the rule `sources/src/payers/companies.rs` already applies.
  - A payment belongs to the trip open at the close of the session before the ex-date. The ex-date comes from the payer's declared record that the payment matches.
  - A payment with no matching declared ex-date is not placed by its pay date. It is left out under a grade, tag or side filter.

**4. A ratio's two sides share one scope.**
- **Where:** `margin_used_pct` (`scope.rs:784`) divides the margin used by the accounts in scope, an account figure, by `market_value`, which is summed from the filtered holdings (`scope.rs:707-712`).
- **Failing scenario:**
  - A margin account holds $100,000 with $20,000 of margin used.
  - A tag filter leaves $5,000 of holdings in scope.
  - Margin used then reads 400 %.
  - This already happens under a symbol filter. The plan extends it to every trade filter.
- **Fix:** an account figure is never divided by a filtered sum. The margin percentage is over the whole market value of the accounts in scope.

**5. The Portfolio's date question isn't the owner's.**
- **The options:**
  - Option (b), positions as of a past date, is a new capability. The owner's rule (2026-09-26: only the migration) rules it out.
  - Option (a) is what finding 1 gives: an open trade is in scope when it was open at any time in the range.
- **Decision:** take (a). Nothing goes to the owner.

**6. The pre-history floor stays in returns and drawdown, over the whole history.**
- **Where:** plan line 28 says the old app's 1 % cutoff had "no stated reason" and is not carried over.
  - For where the curve's line starts, that is fine.
  - But `stat/returns.rs` still uses the floor for the yearly returns and the drawdown, and `SPEC.md` gives its reason.
- **Fix:**
  - The plan says the floor stays for returns and drawdown.
  - The floor is taken from the whole history in scope, never from the date range. Otherwise the same year's return would change with the range chosen.

## In the same commit

- Update `docs/decisions.md:23` ("`Value` … following the account filter"). The plan changes a recorded decision, so write the new rule and why: the date range narrows the value too. Brief 09's wording never weighed the date range.
- Replace `SPEC.md` §5's "Trades are scoped by close date" with the rule the code has: open at any time in the dates chosen, with each sale counting on its own day.
- Rewrite `docs/decisions.md:45`, which still names funds and companies. Replace it with the rule as `payers/exchange.rs` states it: by the listing's market, naming no symbol, brand or company (brief 09, decision 8).

## Not blocking: a plan of its own

The floor is 1 % of the all-time peak, so it erases real years as an account grows.
- **Scenario:**
  - An account held $500 through 2019. It reaches $60,000 in 2026.
  - The floor becomes $600, so 2019 never clears it.
  - 2019 disappears from Annualized returns and from Avg annualized.
- **Why it isn't needed for its stated reason:** that reason is that the first deposit must never read as a return, and the time-weighted return already removes deposits.
- **What remains:** the real risk is a tiny balance, where a cent's move reads as a large return.
- **The ask:** that needs a rule of its own, argued in its own plan, not a floor measured against a peak years later.
