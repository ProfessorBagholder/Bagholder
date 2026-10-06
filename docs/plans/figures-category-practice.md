# Plan: a holding's cost, a trade's result and a past range (brief 15, decisions 6, 7 and 9)

## For the owner to decide

Nothing open. The owner ruled on 2026-10-04 that these are not matters of preference ("There is a right and wrong way", `docs/decisions.md`, 2026-10-04). Decisions 6 and 9 are settled by the category's own documentation. Decision 7 is not settled by the category: the leading journals score in the base currency, and none documents scoring a trade in its own. It follows from the owner's standing rule instead, as set out below (brief 20).

## Brief 20 applied (Go with changes)

Each of the nine required changes, and where it lands:
1. **Fees are in the average** (cost includes the commission, as at the CRA and IBKR): Approach §6; an engine case with a commission on a buy.
2. **One place changes lots, and the average is derived there**: Approach §6. There is no second running figure. An invariant test runs over generated sequences (transfers, splits, return of capital, stock dividends, option expiry and assignment, a short). Cases cover a short's average (what it brought in) and an option's (premium per contract).
3. **A holding transferred between accounts keeps the journal's cost**: `SPEC.md` §1 says so, and the broker-book acceptance check is scoped to holdings with no transferred-in lot. A mismatch on a transferred holding does not trigger the Right to refuse.
4. **The broker's stated book value is read and stored**: Approach §6. `wealthsimple/src/read.rs` and `broker/src/pull.rs` keep it, strictly. No new header sentence.
5. **Realized (FIFO) and unrealized (average cost) do not sum for a partly sold position**: `SPEC.md` says so. Every screen and export is checked for such a sum. There is a property test: for any sequence that ends flat, Σ realized under FIFO = Σ realized at average cost.
6. **Decision 7's basis is stated as it is**: the owner's rule, not the category. See *How the leading products do it* and `SPEC.md`.
7. **One definition of a win**: one function used by the Result filter (`scope.rs` `outcome`), the KPIs and the by-symbol rows. One test holds the Dashboard's counts to the Trades list under Result = Win / Loss for every filter state. Gross W and Avg win keep a negative CAD amount as it stands, never clamped (one case).
8. **The close of a range's last day is the last close on or before it**, with the Bank of Canada rate found the same way. This is the engine's own convention, stated in `SPEC.md`.
9. **No figure on Portfolio mixes dates**: Approach §9 lists every figure, as of the range's end or `—`. A row's page and its Buy and Sell are covered too. Replaying the ledger to the end day is measured at the owner's size and kept by end date until the book moves.

## Scope

Three definitions, in `SPEC.md` and the engine:

1. **A holding's cost after a partial sale.** It becomes the average cost of the units still held, fees in. Today it is the remainder of the lots once the oldest are sold.
2. **A trade's win or loss.** It is judged by the sign of its P&L in its own currency, the figure its own page shows. Every sum stays in CAD.
3. **Portfolio under a past date range.** It shows the positions held at the close of the range's last day (the last close on or before it), with every figure as of that day or `—`.

Out of scope:
- An "include sold positions" switch (a later feature, `docs/decisions.md` 2026-10-04).
- The trades' FIFO matching, which stays.
- Any header sentence about cost.

## The old app here

1. **Cost.**
   - Today: the Python app, and the Rust engine after it, give a partly sold holding the cost of its unsold lots (`engine/src/positions.rs`: the book as Σ lot value).
   - What the category does: Wealthsimple and the CRA state the average cost.
   - Wealthsimple's stated book value is in its positions reply (`bookValue`, `marketBookValue`) and is dropped today: `wealthsimple/src/read.rs:137` and `broker/src/pull.rs:352` store `book_value: None`.
   - New entry in `docs/old-app-mistakes.md`.
2. **Win or loss.**
   - Today: the Result filter (`scope.rs:174`), the KPIs (`scope.rs:511-540`) and the by-symbol rows (`scope.rs:602`) each decide a win on `pnl_cad`. So a trade can read "Loss" while its own page shows +$50 in USD.
   - New entry.
3. **Date range.**
   - Today: `SPEC.md` §5 keeps any holding "held at some time in them, still valued today" (`scope.rs:267`).
   - New entry.

Carried over, and why it is right:
- **FIFO for trades' P&L.** Tradervue: "Realized P&L calculations use a First-in, First-out (FIFO) methodology" (help.tradervue.com/article/3437, read 2026-10-04); TradeZella recommends FIFO (help.tradezella.com/en/articles/6826141).
- **Lots moving between accounts with their cost and dates** (`ledger.rs` `apply_transfer`). It is the same round trip, as a journal keeps it; the broker resets the cost, and the journal deliberately does not.
- **CAD totals**, by the owner's rule.

## How the leading products do it

Every source was read on 2026-10-04.

**6. Average cost, fees in.**
- Wealthsimple (help article 4409775037083): "Your cost base/share hasn't changed, it's still $10/share. This is always the case for any sales you make."
- Wealthsimple (37274145854875): "Unrealized Return ($) = Current market value - Book cost".
- CRA, identical properties: the average is the total cost, acquisition expenses included, over the number owned.
- IBKR TWS: average price = "your cost (execution price + commission)" over the position (ibkrguides.com/tws/usersguidebook/realtimeactivitymonitoring/profitloss.htm).
- Sharesight (Canada): "'Adjusted Cost Base' sale allocation method".
- On a transfer between accounts, Wealthsimple resets the cost: "the asset's book cost in the new account is updated to reflect its current market value" (24667492921883). The journal keeps the lots' own cost (above).

**7. Win or loss: the category does not settle it.**
- Tradervue: "the reports will use the converted P&L in the base currency to aggregate and compare performance across all trades" (help.tradervue.com/article/3425). No journal documents scoring in the trade's own currency.
- This plan scores in the trade's own currency, on the owner's rule ("Per-instrument figures in the instrument's own currency"). A trade's page shows its P&L in that currency, so its Win or Loss must read from the same figure: the alternative would put a green +$50 under "Loss".
- The performance-attribution literature and Sharesight ("the currency gain purely in relation to the currency movement", help.sharesight.com/us/components-return/) separate local return from currency return.
- It matters only for a trade the currency moved against by more than its own gain.

**9. Past range: as of its end.**
- Sharesight values at "the current price or the closing price of the selected end date" (help.sharesight.com/show_portfolio/).
- IBKR's Open Position Summary lists "all open positions in your portfolio at the end of the period" (ibkrguides.com/reportingreference/reportguide/openpositionsummary.htm).
- Neither says what happens when the end date is not a trading day. Here the last close on or before it is taken: the engine's convention for today (`positions.rs:99`) and for rates (`fx::rate`).

## Open questions

None.

## Approach

**§6 Cost.**
- The lot primitives in `engine/src/ledger.rs` (`open_lot`, `take`, `close`, the corporate-event adjustments, `apply_transfer`, option expiry and assignment) are the only code that changes lots. The average-cost basis is kept by those same primitives on the position they act on: total cost and units, a sale removing cost in proportion. No other code updates it.
- Fees go into a buy's cost.
- A short's average is what it brought in. An option's is the premium per contract.
- `positions.rs`: Avg = total cost ÷ (units × multiplier); Book = total cost; P&L = Market − Book.
- `SPEC.md` §1 Position re-defines Avg cost and Book, and says:
  - a transferred holding keeps the journal's cost, not the broker's reset;
  - a partly sold position's realized (FIFO, the trade) and unrealized (average cost, the holding) figures are each the category's own and are not added together.
- The broker's stated book value is read strictly. Which of `bookValue` and `marketBookValue` is the cost, and in which currency, is settled from the recorded replies in `wealthsimple/tests`. It is stored in the statements table's existing column.

**§7 Win or loss.**
- One function, `engine/src/scope.rs` `result(trade)`, judges on the trade's own-currency P&L. `outcome()`, `kpi` and the by-symbol rows all call it.
- The sums stay `pnl_cad`, unclamped.
- `SPEC.md` §6 Dashboard states the basis as above.

**§9 Past range.**
- A position is in scope if it is open at the close of the range's last day, from the ledger replayed to that day.
- Closes come from the stored daily closes, the last on or before that day; rates come from `fx::rate` the same way.
- Every Portfolio figure under a past range:
  - **As of the range's end:**
    - Qty, Avg, Book;
    - Last (that close) and Market (Qty × Last);
    - P&L;
    - the day's Change (that session's close against the one before);
    - Net asset value (the accounts' stated value that day);
    - Cash (the book as of that day);
    - Allocation, Sectors and Regions (from those values).
  - **`—`:** Margin used and Available margin (stated only now).
- A row's page opens as of that day and offers no Buy or Sell.
- The replay is cached by end date until the book moves. Its cost is measured at the owner's size.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green; `cargo clippy --workspace --lib --bins` clean.
- [ ] Blind engine cases, all green, covering:
  - partial sales at several prices;
  - a commission on a buy;
  - a partial sale after a return of capital;
  - a short;
  - an option;
  - a won-in-USD, down-in-CAD trade (counted a win, its negative CAD amount in Gross W);
  - a past range ending on a weekend;
  - a past range ending after a sale;
  - a past range whose instrument has no close on or before its end.
- [ ] Generated sequences (transfers, splits, return of capital, stock dividends, option expiry and assignment, shorts):
  - units = the open lots' units;
  - total cost = Σ lot value − cost removed;
  - for every sequence that ends flat, Σ realized FIFO = Σ realized at average cost.
- [ ] One test over every filter state: the Dashboard's win and loss counts equal the Trades list's lengths under Result = Win and Result = Loss.
- [ ] Wealthsimple's book value is read and stored (a reply test).
- [ ] On a copy of the owner's book, Avg = the stated book value ÷ units, for every partly sold holding with no transferred-in lot.
- [ ] No screen or export adds a trade's realized P&L to a holding's unrealized P&L (checked by reading every place both are shown).
- [ ] Browser test: Portfolio under a past range shows the end day's positions and figures, and `—` for margin.
- [ ] States and screenshots at 1200, 1340, 1440 and 1680 px.
- [ ] The cost of the end-day replay at the owner's size, pasted in the PR.

## Surfaces to check beyond the diff

- `docs/old-app-mistakes.md` (three entries).
- `SPEC.md` §1, §5 and §6.
- The heatmap's holdings.
- The holding page (open trade and unrealized P&L shown together).
- Exports.
- The broker check (units only).

## Right to refuse

If Wealthsimple's stated book value disagrees with average cost on a holding with no transferred-in lot, stop and report that holding.

## Anti-stub self-check

To initial when built.

## Verification

To fill when built.

## Handoff

- Plan written 2026-10-04 from brief 15 decisions 6, 7 and 9.
- Brief 20 (Go with changes) applied above.
- 2026-10-06: decisions 6 and 7 built (average cost kept by the lot primitives, `Basis`; one `result`; Wealthsimple's book value read and stored), with `engine/tests/basis.rs`, `results_agree` and the blind `average_cost.json`. Released as 2.4.0 (#373). Decision 9 built next: `Engine::past` (the ledger matched to the day, kept while a screen holds it), `scope::portfolio_as_of`, the past day's closes among the engine's needs, the holding page as of that day with no ticket; `past_range.json` (blind), `engine/tests/past.rs`, `web/e2e/pastrange.spec.ts`. The check against Wealthsimple's book value runs on the owner's book once a pull on 2.4.0 has stored it.
