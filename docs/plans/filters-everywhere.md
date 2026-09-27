# Plan: every filter narrows every figure it can describe

## For the owner to decide

1. **What a date range does on the Portfolio tab.** Holdings are a snapshot of now. Options:
   - (a) the date range narrows holdings to those held at some time in the range, still valued today, as every other page's range picks trades open in it;
   - (b) the Portfolio shows holdings as they stood at the range's end, valued at that day's closes, as a broker's "positions as of" statement does.

   (a) is consistent with the rest of the app and small. (b) is a new capability, outside the migration. **Recommended: (a).**

## Scope

SPEC.md says one filter set applies to every page (§5, Filters), then exempts figures from it without a reason a trader would accept. Audit of every figure and chart against the filters, 2026-09-27:

| Where | Today | Should |
|---|---|---|
| Dashboard: Equity curve `Value`, Max drawdown, Avg annualized, Annualized returns card | Account filter only (`engine/src/scope.rs` `equity_block`; SPEC §5 Dashboard "following the account filter alone") | Also the date range: the value, its drawdown and its returns over the days chosen |
| Portfolio: tiles, Holdings, Allocation, Sectors and Regions; Markets heatmap `Holdings` | Account, symbol, search, kind, exchange (`position_matches`); date, grade, tag, side, result and the Price/Hold/P&L/Qty ranges ignored | Every filter a holding has: side (its direction), grade and tag (the journal it shares with its trade), result (unrealized sign), Price (average cost), Hold, P&L (unrealized), Qty; date per the owner's choice above |
| Cashflow | Date, account, symbol; SPEC says the page "says which other filters it ignored", but nothing is shown (owner decision 2026-09-25: nothing written under a figure) | Also kind and exchange (a payment's instrument has both); grade, tag and side by the round trip that held the units on the day paid; result and the ranges do not describe a payment and are not read. SPEC stops claiming a message |
| Orders panel | Account filter only | Also symbol, search, kind and exchange (an order has an instrument) |

What stays: the P&L curve, KPI tiles, Monthly P&L, By symbol, Grade vs P&L and the Trades list already follow every filter. A trade is in scope for a date range when open at any time in it, and a sale counts on its own day. That is already how partial sales are counted (decision 2026-09-24).

Out of scope: new filters, new screens, any text explaining a filter.

## The old app here

The old app applied the account filter alone to the account's value and trimmed the curve's start below 1% of its peak (`model.py` 1821, 1960, 3019), a cutoff with no stated reason. Neither is carried over. The value curve starts at the first day Wealthsimple states a value in the accounts chosen, narrowed by the date range. New entry in `docs/old-app-mistakes.md`: "Figures exempted from filters without a reason (value, drawdown and returns ignored the date range; holdings ignored tags and grades)".

## How the leading products do it

- **TradeZella:** "Most filters work across all tabs"; the top bar's date range and account filters drive the dashboard's widgets; its drawdown widget shows the selected period, the % view from the account's running balance. [Using Filters](https://help.tradezella.com/en/articles/12417670-using-filters-in-tradezella), [Drawdown widgets](https://help.tradezella.com/en/articles/8427754-understanding-drawdown-widgets-in-tradezella), read 2026-09-27.
- **Interactive Brokers PortfolioAnalyst:** one period selector (7D, MTD, 1M, YTD, 1Y, custom) and an account selector drive the performance view; risk measures including Max Drawdown are computed for the period selected. [Dashboard](https://www.ibkrguides.com/portfolioanalyst/performanceandstatements/pa_viewingaccountperformance.htm), [PortfolioAnalyst](https://www.interactivebrokers.com/en/portfolioanalyst/overview.php), read 2026-09-27.
- **Tradervue:** clicking a report's bar narrows the global trade filter, one filter feeding every report. [Interactive reports](https://app.tradervue.com/help/interactive_reports), read 2026-09-27.

None of them exempts a figure from the period or the account chosen.

## Open questions

None beyond the owner's choice. How a period return is chained (time-weighted, from Wealthsimple's daily value and net deposits) is unchanged; only the days it runs over change.

## Approach

- **`engine/src/scope.rs` `equity_block`:** build the combined series from full history as now (returns need the day before), then take the days in the range for the curve, the drawdown (peak from the range's first day) and the yearly returns. A year cut by the range is returned over its days inside the range, as the current year already is. `unread_by_value` and `unread_by_cashflow` go, with the wire's `skippedFilters`, since nothing reads them and the page must not show them.
- **`position_matches`:** takes every filter a holding has, through the `PositionFig`'s own fields (direction, journal, avg, held days, unrealized, qty). Date per the owner's choice.
- **Cashflow:** payments narrowed by the instrument's kind and exchange, and by the round trip holding the units on the day paid (`Matched` trips by account and instrument).
- **Orders panel** (`server/src/orders/doc.rs`): its filter reads the instrument filters as it reads the account.
- **`SPEC.md` §5:** Dashboard, Portfolio, Cashflow and Orders say what each filter narrows. One sentence in Filters: a filter narrows every figure it can describe; a figure it cannot describe (an account's value by symbol) shows the accounts chosen.

## Acceptance criteria

- [ ] `cargo test --workspace` green; engine cases written by an agent that has not read the engine, for: value, drawdown and annualized over a date range; a holding narrowed by tag, grade, side and each range; a payment narrowed by kind, exchange and by the tag of the round trip that held it.
- [ ] Browser tests: a date range narrows the Value curve and moves Max drawdown; a tag narrows Holdings; a kind narrows Cashflow; a symbol narrows the Orders panel.
- [ ] On a copy of the owner's book: the Value curve with a 2024 range starts and ends in 2024; with no filters, every figure is unchanged to the cent (`bagholder compare-figures`).

## Surfaces to check beyond the diff

`web/src/lib/generated/wire.ts` (`skippedFilters` removed); `web/src/no_counts.test.ts`; `docs/old-app-mistakes.md`; the demo book's figures in the browser tests.

## Right to refuse

If a figure turns out to have no meaning under a filter that this plan applies to it, it is named here for the owner rather than given one.
