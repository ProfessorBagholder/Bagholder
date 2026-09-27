# Plan: every filter narrows every figure it can describe

## For the owner to decide

Nothing. The Portfolio's date range takes option (a) below (brief 12, change 5): positions as of a past date would be a new capability, outside the migration.

## Scope

SPEC.md says one filter set applies to every page (§5, Filters), then exempts figures from it without a reason a trader would accept. Audit of every figure and chart against the filters, 2026-09-27:

| Where | Today | Should |
|---|---|---|
| Dashboard: Equity curve `Value`, Max drawdown, Avg annualized, Annualized returns card | Account filter only (`engine/src/scope.rs` `equity_block`; SPEC §5 Dashboard "following the account filter alone") | Also the date range: the value, its drawdown and its returns over the days chosen |
| Portfolio: tiles, Holdings, Allocation, Sectors and Regions; Markets heatmap `Holdings` | Account, symbol, search, kind, exchange (`position_matches`); date, grade, tag, side, result and the Price/Hold/P&L/Qty ranges ignored | Every filter a holding has, read from the value its own row shows: side (its direction), grade and tag (the journal it shares with its trade), result (unrealized sign), Price (average cost), Hold, P&L (unrealized), Qty; the date range keeps the holdings held at some time in it, still valued today, so every preset ending today narrows nothing and a range ending in the past hides what was bought after it. The margin percentage stays over the whole market value of the accounts in scope: an account figure is never divided by a filtered sum |
| Cashflow | Date, account, symbol; SPEC says the page "says which other filters it ignored", but nothing is shown (owner decision 2026-09-25: nothing written under a figure) | Also kind and exchange (a payment's instrument has both); grade, tag and side by the round trip entitled to it: the trip open at the close of the session before the payment's ex-date, the ex-date taken from the payer's declared record the payment matches (entitlement is holding at the record date, which equals the ex-date under T+1, as `sources/src/payers/companies.rs` applies). A payment with no matching declared ex-date is never placed by its pay date: under a grade, tag or side filter it is left out. Result and the ranges do not describe a payment and are not read. SPEC stops claiming a message |

What stays: the P&L curve, KPI tiles, Monthly P&L, By symbol, Grade vs P&L and the Trades list already follow every filter, except that an open trade has no result (below). The Orders panel keeps the account filter alone, as `SPEC.md` says: it is where live orders are managed, and a filter set to review trades must never hide an order resting at Wealthsimple (its heading names the accounts in scope).

**An open trade has no result.** `outcome` (`engine/src/scope.rs`) reads the P&L realized so far, so an open trade with no sale reads Breakeven and a partly sold one reads Win or Loss. That contradicts the decision of 2026-09-24 that closed-trade figures count once a trade has closed; TraderSync gives an open trade the status Open. `outcome` becomes `None` for an open trade, so on the Trades list and Dashboard a Result filter keeps closed trades only. On the Portfolio, Result reads the sign of the unrealized P&L the row shows. A trade is in scope for a date range when open at any time in it, and a sale counts on its own day. That is already how partial sales are counted (decision 2026-09-24).

Out of scope: new filters, new screens, any text explaining a filter.

## The old app here

The old app applied the account filter alone to the account's value and trimmed the curve's start below 1% of its peak (`model.py` 1821, 1960, 3019), a cutoff with no stated reason. The account-filter-only value is not carried over, nor is the trim of where the curve's line starts: the value curve starts at the first day Wealthsimple states a value in the accounts chosen, narrowed by the date range. The floor stays in the yearly returns and the drawdown (`stat/returns.rs`) until a plan of its own replaces it, and it is always taken from the whole history in scope, never from the date range, so a year's return never changes with the range chosen. New entry in `docs/old-app-mistakes.md`: "Figures exempted from filters without a reason (value, drawdown and returns ignored the date range; holdings ignored tags and grades)".

## How the leading products do it

- **TradeZella:** "Most filters work across all tabs"; the top bar's date range and account filters drive the dashboard's widgets; its drawdown widget shows the selected period, the % view from the account's running balance. [Using Filters](https://help.tradezella.com/en/articles/12417670-using-filters-in-tradezella), [Drawdown widgets](https://help.tradezella.com/en/articles/8427754-understanding-drawdown-widgets-in-tradezella), read 2026-09-27.
- **Interactive Brokers PortfolioAnalyst:** one period selector (7D, MTD, 1M, YTD, 1Y, custom) and an account selector drive the performance view; risk measures including Max Drawdown are computed for the period selected. [Dashboard](https://www.ibkrguides.com/portfolioanalyst/performanceandstatements/pa_viewingaccountperformance.htm), [PortfolioAnalyst](https://www.interactivebrokers.com/en/portfolioanalyst/overview.php), read 2026-09-27.
- **Tradervue:** clicking a report's bar narrows the global trade filter, one filter feeding every report. [Interactive reports](https://app.tradervue.com/help/interactive_reports), read 2026-09-27.

None of them exempts a figure from the period or the account chosen.

## Open questions

None. How a period return is chained (time-weighted, from Wealthsimple's daily value and net deposits) is unchanged; only the days it runs over change.

## Approach

- **`engine/src/scope.rs` `equity_block`:** build the combined series from full history as now (returns need the day before), then take the days in the range for the curve, the drawdown (peak from the range's first day) and the yearly returns. The 1 %-of-peak floor is measured on the whole history in scope before the range is taken. A year cut by the range is returned over its days inside the range, as the current year already is. `unread_by_value` and `unread_by_cashflow` go, with the wire's `skippedFilters`, since nothing reads them and the page must not show them.
- **`outcome`:** `None` while the trade is open.
- **`position_matches`:** takes every filter a holding has, through the `PositionFig`'s own fields (direction, journal, avg, held days, unrealized, qty). Date: held at some time in the range.
- **Margin used %:** margin used over the market value of every holding in the accounts in scope, never the filtered holdings.
- **Cashflow:** payments narrowed by the instrument's kind and exchange, and by the round trip open at the close of the session before the ex-date of the declared record the payment matches (`Matched` trips by account and instrument); a payment with no matching record is left out under a grade, tag or side filter.
- **Orders panel:** unchanged, account filter only.
- **`SPEC.md` §5:** Dashboard, Portfolio and Cashflow say what each filter narrows; "Trades are scoped by close date" becomes the rule the code has: open at any time in the dates chosen, each sale counting on its own day.
- **`docs/decisions.md`:** the 2026-09-25 Equity curve entry says `Value` follows the account filter and the date range, and why (brief 09's wording never weighed the date range); the 2026-09-24 fund-company entry is rewritten as the rule `payers/exchange.rs` states, by the listing's market, naming no symbol, brand or company. One sentence in Filters: a filter narrows every figure it can describe; a figure it cannot describe (an account's value by symbol) shows the accounts chosen.

## Acceptance criteria

- [ ] `cargo test --workspace` green; engine cases written by an agent that has not read the engine, for: value, drawdown and annualized over a date range; a holding narrowed by tag, grade, side and each range; a payment narrowed by kind, exchange and by the tag of the round trip entitled to it (held through the ex-date and sold before the pay date; sold and bought back before the pay date; no matching declared record); an open trade under each Result; margin % under a tag filter.
- [ ] Browser tests: a date range narrows the Value curve and moves Max drawdown; a tag narrows Holdings; a kind narrows Cashflow; a symbol leaves the Orders panel and its badge unchanged.
- [ ] On a copy of the owner's book: the Value curve with a 2024 range starts and ends in 2024; with no filters, every figure is unchanged to the cent (`bagholder compare-figures`).

## Surfaces to check beyond the diff

`web/src/lib/generated/wire.ts` (`skippedFilters` removed); `web/src/no_counts.test.ts`; `docs/old-app-mistakes.md`; the demo book's figures in the browser tests.

## Right to refuse

If a figure turns out to have no meaning under a filter that this plan applies to it, it is named here for the owner rather than given one.
