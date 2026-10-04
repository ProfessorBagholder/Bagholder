# Plan: three figures defined as the category defines them (brief 15, decisions 6, 7 and 9)

## For the owner to decide

Nothing open. The owner ruled on 2026-10-04 that these three are not choices: "There is a right and wrong way, it shouldn't be a matter of personal preference" (`docs/decisions.md`, 2026-10-04). Each is settled below from the category's own documentation and goes into `SPEC.md` as a correctness fix.

## Scope

Three definitions in `SPEC.md` and the engine:

1. **A holding's cost after a partial sale.** This is the average cost of the units still held. Today it is what is left of the lots after the oldest are sold first.
2. **Whether a trade won or lost.** This is judged in the trade's own currency, so the Dashboard, the Trades list and the trade's page always agree. Today the Dashboard judges it in CAD.
3. **Portfolio under a past date range.** It shows the positions held on the range's last day, valued at that day's closes. Today it shows anything held at any time in the range, valued at today's price.

Out of scope, on purpose:
- An "include sold positions" switch for a past range. Sharesight has one; it is a new feature and waits (`docs/decisions.md` 2026-10-04: no new features until the bugs are fixed).
- How trades match sells to buys for their own P&L. It stays oldest-first (FIFO).

## The old app here

1. **Cost.** The Python app, and the Rust engine after it (`engine/src/positions.rs`, a holding's `book` built from `matched.books[..].lots`), give a partly sold holding the cost of the lots still unsold, oldest sold first. Wealthsimple, Questrade and the CRA all state the average cost of everything still held. So after a partial sale the holding's Avg, Book and unrealized P&L differ from the broker's own screen. Entry to add to `docs/old-app-mistakes.md`: "A partly sold holding's cost was the FIFO remainder, not the average cost the broker and the CRA state."
2. **Win or loss.** `engine/src/scope.rs` `kpi` classifies a closed trade by the sign of `pnl_cad`. A USD trade that gained in USD while the Canadian dollar moved against it is a loss on the Dashboard and a gain on its own page, which shows the instrument's currency (`docs/decisions.md`: "Per-instrument figures in the instrument's own currency; aggregates in CAD"). Entry to add: "A trade's win or loss was judged after conversion to CAD, so its own page and the Dashboard disagreed."
3. **Date range.** `SPEC.md` §5 Filters: a holding answers the dates "when it was held at some time in them, still valued today, so a range ending today narrows nothing" (`engine/src/scope.rs:267`). No leading product does this. Entry to add: "Portfolio under a past range listed anything held at any time in it, valued today."

Carried over unchanged:
- **FIFO matching for trades' P&L.** Tradervue: "Realized P&L calculations use a First-in, First-out (FIFO) methodology" (help.tradervue.com/article/3437-swing-trades, read 2026-10-04). TradeZella recommends FIFO as its default (help.tradezella.com/en/articles/6826141, read 2026-10-04).
- **CAD totals.** These follow from the owner's rule, aggregates in CAD.

## How the leading products do it

All pages below were read on 2026-10-04. Pages that refused a direct fetch are marked as read through a search summary only; the rule rests on the fetched ones.

**1. A holding's cost after a partial sale: average cost.**
- **CRA.** Identical shares are held at their average cost: "dividing the total cost of identical properties purchased by the total number", and "Dispositions of identical properties do not affect the ACB" (canada.ca, line 12700, special rules).
- **IBKR TWS.** The average price is "your cost … by the quantity of your position", and unrealized P&L is "the difference between the current market value and the average price" (ibkrguides.com/tws/usersguidebook/realtimeactivitymonitoring/profitloss.htm).
- **Sharesight (Canada).** Uses the "'Adjusted Cost Base' sale allocation method" (help.sharesight.com/ca/capital_gains/).
- **Wealthsimple and Questrade.** Their help pages show a sale lowering the total cost while the cost per share is unchanged (search summary only; the pages refused a direct fetch).

So the holding's cost is the average cost per unit. A sale removes cost in proportion and leaves the per-unit cost as it was. Book is the units held × that cost, and unrealized P&L is Market − Book.

The trade keeps FIFO for its own realized P&L, as the journals do. The two methods agree once a position is closed in full. While it is open, the trade's realized part and the holding's unrealized part are each the category's own figure. IBKR shows the same split between TWS and its statements.

**2. Win or loss: in the trade's own currency.**
- None of TradeZella, Tradervue, TraderSync or Edgewonk documents which currency decides a win.
- Tradervue and TradeZella aggregate P&L converted to the base currency (help.tradervue.com/article/3425-pl-reporting-modes; TradeZella's display-currency article).
- The performance-attribution standard separates a holding's local-currency return from its currency return ("currency return = total return − total return (local)", the GIPS draft guide to return attribution, read through a search summary only).
- Sharesight reports "the currency gain purely in relation to the currency movement" as its own column (help.sharesight.com/us/components-return/).

The trade's decision is its local result. The currency's move is a separate effect. The owner's standing rule puts per-instrument figures in the instrument's own currency, and a trade is per-instrument.

So a closed trade is a win, a loss or breakeven by the sign of its P&L in its own currency. Every sum stays in CAD: Realized P&L, Gross W and L, Profit factor, and the average win and loss. A trade won in USD whose CAD P&L is negative therefore counts as a win and adds its negative CAD amount to Gross W, exactly. This case is rare (the currency must move more than the trade's own gain) and is named in `SPEC.md`.

**3. Portfolio under a past date range: held on the range's last day, valued at that day's close.**
- **Sharesight, Portfolio overview.** Values at "The current price or the closing price of the selected end date" (help.sharesight.com/show_portfolio/). Its performance report states "The price, quantity and value figures are as of the end date selected" (help.sharesight.com/us/performance_report/).
- **IBKR PortfolioAnalyst.** Its Open Position Summary "shows all open positions in your portfolio at the end of the period", valued at the period's end (ibkrguides.com/reportingreference/reportguide/openpositionsummary.htm).
- **Wealthsimple.** Its holdings report shows holdings "as of the date you choose" (search summary only).
- No journal (Tradervue, TradeZella, TraderSync) has an open-positions view by date range: open trades count only when realized.

So under a range that ends before today, Portfolio shows the positions held at the close of the range's last day, at that day's close in their own currency, and CAD at the Bank of Canada's rate for that day. A range ending today is today's Portfolio, as now.

## Open questions

None. The one judgement, a won-in-USD trade with a negative CAD P&L in Gross W, follows from the two rules above and is stated in `SPEC.md`, not left open.

## Approach

- **Cost.** `engine/src/positions.rs`: a position's `book` becomes units × average cost. The average is kept per position as lots arrive: a buy adds its cost; a sale removes cost in proportion to the units sold. Corporate events adjust it as they adjust lots today (`SPEC.md` §1, return of capital and spin-off). Lots stay as they are for Hold (quantity-weighted days) and for the trades' FIFO. `SPEC.md` §1 Position: `Avg cost` and `Book` re-defined, with the sources above.
- **Win or loss.** `engine/src/scope.rs` `kpi` and the by-symbol rows classify on the trade's own-currency P&L (`TradeFig`'s native P&L) and sum `pnl_cad`. `SPEC.md` §6 Dashboard: the tile table's Win rate, and a sentence under it.
- **Date range.** `engine/src/scope.rs`: a holding in scope for a past range is a position open at the close of the range's last day, from the ledger as of that day, valued from the daily closes the market cache already holds, at that day's Bank of Canada rate (`engine/src/fx.rs`). A day with no close for an instrument is the gap the engine already has for that, never today's price. `SPEC.md` §5 Filters: the holding sentence.
- **Wire.** No new types: the Portfolio document's fields keep their meaning (Avg, Book, Market, P&L), now as of the range's end.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green, `cargo clippy --workspace --lib --bins` clean.
- [ ] Engine cases written blind (by an agent given `SPEC.md` and the case format, not the engine), for the following, all green:
  - a partial sale at several prices, held at average cost;
  - a partial sale after a return of capital;
  - a USD trade won in USD and lost in CAD, counted as a win with its CAD amount in Gross W;
  - a past range ending on a day a position was held, and one ending after it was sold;
  - a past range whose last day lacks a close.
- [ ] Generated cases: the average cost of a position equals total cost ÷ units, over random buy and sell sequences in several currencies.
- [ ] On a copy of the owner's book (scratch server), a partly sold holding's Avg equals Wealthsimple's stated book value ÷ units, for every such holding the statements give.
- [ ] Browser test: Portfolio under a past range shows the positions of its last day, valued at that day's close.
- [ ] States and screenshots of Portfolio under today and a past range, at 1200, 1340, 1440 and 1680 px.

## Surfaces to check beyond the diff

- `docs/old-app-mistakes.md` (three entries).
- `SPEC.md` §1 Position, §5 Filters, §6 Dashboard.
- The heatmap's holdings (they answer the same filter).
- The broker check, which compares units only and is unaffected.
- The daily-close store's reads under a past range: a cost at the owner's size, measured.

## Right to refuse

If Wealthsimple's stated book value disagrees with average cost on the owner's book for a reason the sources do not cover, stop and report the case before building on.

## Anti-stub self-check

To initial when built.

## Verification

To fill when built.

## Handoff

Plan written 2026-10-04 from brief 15 decisions 6, 7 and 9, after the owner ruled them correctness (`docs/decisions.md`). Research by three read-only web surveys; the fetched sources were cited above, and the summary-only ones are marked. Nothing built.
