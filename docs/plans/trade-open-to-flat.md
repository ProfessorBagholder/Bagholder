# Plan: a trade runs from open to flat, a partly sold position is an open trade, and each partial sale's P&L counts on its day

## For the owner to decide

Nothing open.

Settled (owner, 2026-09-24, `docs/decisions.md`): one trade per position from its first fill until it is back to zero; a partly sold position is an open trade in the Trades list; a partial sale's P&L counts on the day of that sale in realized P&L and the monthly figures, while win rate and the other closed-trade statistics count a trade once it has closed.

## Scope

What a trade is (`SPEC.md` §2 Trade), the Trades list and its trade page (§Trades), and every figure built from trades: the Dashboard tiles (Realized P&L, Win rate, Profit factor, Expectancy), Monthly P&L, Grade vs P&L, By symbol and the Review queue (§Dashboard). Positions, the Portfolio page and the holding page are unchanged in what they show; an open trade is the position they already show, seen from the Trades list. Out of it: the trade marks (`split`, `rolled`, …), which `SPEC.md` defines at the switch; how the date filter treats an open trade's list row beyond what is stated below.

## How the leading products do it

All read 2026-09-24 from each product's own help pages.

- **A trade is one position from its first fill until it is flat.** Tradervue groups executions into trades and splits a trade only where the position is flat, with a setting to start a new trade "whenever you get flat" (https://help.tradervue.com/article/3480-split-merge-trades).
- **A partial exit stays inside its trade, which is open until flat.** TraderSync's trade status is Winner, Loser, Breakeven or Open, "Open" being a trade whose position is still held, and it reports the P&L of "each partial exit within a larger trade" as Partial Return (https://tradersync.com/support/open-open-trade-percentage/, https://tradersync.com/support/partial-return-net/). Tradervue labels such a trade "open" and shows its realized P&L so far (https://help.tradervue.com/article/3437-swing-trades).
- **A partial exit's P&L is realized on the day of the exit.** TradeZella: daily P&L is "based on when a trade is closed or a partial exit is taken", and a trade still open contributes nothing beyond that (https://help.tradezella.com/en/articles/10528734-how-the-dashboard-calendar-calculates-and-shows-daily-profit-loss-p-l). Tradervue: a position still open after closing some contracts is not in the closed-trade reports but is in the overview's realized P&L (https://help.tradervue.com/article/3479-open-trades-and-p-l-discrepancy).
- **Closed-trade statistics use closed trades.** Tradervue's detailed reports "only include closed trades" (same page); TraderSync gives an open trade no win or loss (its status is Open).
- **Lots are matched first in, first out** for realized P&L (Tradervue, same page; TraderSync lets the trader choose FIFO, LIFO or weighted average). Bagholder keeps FIFO (`SPEC.md` §2); the choice of method is not changed here.

What `SPEC.md` and the engine did, checked against that: `SPEC.md` said "a trade always has a close date; there is no open trade" (no open trades at all), and the engine made the sold part of a still-held position a finished trade of its own (a trade before the position was flat). Both depart from the practice above; this plan replaces both.

## Open questions

- **The date filter and an open trade.** Objective: a date range shows what happened in it. Known: TradeZella dates realized P&L by the exit (above); Tradervue's Trades view shows a swing trade's realized data against its entry date (swing trades page above), its calendar by the day realized. Chosen, as the realized figures already are dated by exit: a trade's realized parts count in a range by the day each was realized; a trade's row is in a range when it was open at any time in it. Settled from the sources above; nothing further to find.

## The old app here

Not consulted (`CLAUDE.md`, the owner's rule on the old app). What the person sees changes on purpose: open trades appear in the Trades list, and a partial sale's P&L moves into the month it was made.

## Approach

**Engine** (`engine/src/ledger.rs` round trips and slices, `engine/src/trades.rs`, `engine/src/scope.rs`, `engine/src/identity.rs`). A round trip (`TripKey`) already runs from the fill that opens it to the one that leaves it flat, and its closed parts are `Slice`s. `TradeFig` becomes one per round trip, open or closed: a status (`Open` while any lot is held, else `Closed`), `closed_on` only when closed, its realized P&L the sum of its slices, its slices kept with their days. Per-trade fields for an open trade: Open; Close none (the row reads `Open`); Qty the units opened; Entry over all units opened; Exit over the units closed so far (none before the first exit); P&L the realized part so far, P&L % over the entry basis of the units closed; Hold the days so far. A trade id and its journal belong to the round trip from its first fill, as the holding and its trade already share one journal (`SPEC.md` §Trades, "A holding").

**Figures** (`scope.rs`):
- Realized P&L: every realized part in scope, of closed and open trades, each on its own day. Subtitle: the closed trades in scope.
- Win rate, Profit factor, Expectancy (closed trades' P&L ÷ their count), average win and loss, Grade vs P&L, By symbol's win rate and average hold, the Review queue: closed trades only.
- Monthly P&L: each realized part in the month it was realized; the hover counts the trades realizing in that month.
- By symbol P&L: realized parts, as Realized P&L; its Trades column counts closed trades.
- `basis-unknown` round trips stay out of the performance figures as today.
- **A round trip that goes flat with no sale is not a trade** (brief 07): a position sent wholly out of the account by a transfer out takes lots and makes no slice, and a transfer out is not a sale (`SPEC.md` §2, Crypto), so it is neither a closed trade nor counted in Win rate or Expectancy. A case holds it.
- **Each figure says what it covers** in `SPEC.md`: Realized P&L includes open trades' partial sales, its subtitle count and Expectancy count closed trades; a group is open while any member is open.

**Wire and page** (`server/src/http/model.rs`, `web/src/lib/generated/wire.ts`, `web/src/lib/Trades.svelte` and the trade page): the trade row carries its status and optional close; the Trades list shows open trades (Close reads `Open`), newest activity first by default; opening an open trade opens the holding page, as a Holdings row does. No caption or helper text is added. **Built with the switch (3c):** the page is served from the old model until then (`server/src/http/model.rs` reads `bagholder_model`, not the engine), so a page change here could not show the engine's trades; the engine's `TradeFig` carries what the page needs (`status`, `closed_on`, `last_on`, `realized`), and 3c's wire and page read it.

**`SPEC.md`** changes in the same commit: §2 Trade (the definition, the per-trade fields for an open trade), §Dashboard (each figure above), §Trades (the list and default order). `docs/old-app-mistakes.md` gains the entry.

## Acceptance criteria

- [x] `cargo test --workspace` green in `rust/`, warning-free, applet test alone. The page's criteria (a browser test driving an open trade, rendering on the scratch server) move to 3c with the wire.
- [x] Engine cases, expected figures written by an agent that has not read the engine: a partly sold position is one open trade with its realized part (the ETH case of `coins_and_transfers.json`); the same position sold to flat is one closed trade whose P&L is all its parts; Realized P&L counts a partial sale on its day while Win rate, Profit factor and Expectancy do not count the open trade; Monthly P&L puts each part in its own month; a roll chain still open is one open trade; a position transferred wholly out with no sale is no trade.
- [x] The blind check of brief 04 §8, run again on every case file this changes, agrees with the committed figures, or each disagreement is settled against `SPEC.md`.
- [x] `SPEC.md`, `docs/architecture.md` and `docs/old-app-mistakes.md` say what the code does.

## Surfaces to check beyond the diff

`identity.rs` (trade ids and orphans when a round trip is open), the journal keyed by trade id, `web/src/lib/generated/wire.ts`, the Review queue, the phone's §8 (on hold; noted, not built).

## Right to refuse

Nothing refused.

## Anti-stub self-check

- No definition nobody references: `TradeStatus`, `Realized`, `last_on` are read by `scope.rs`, the list order and the case runner; `Kpi.realized_left_out` by the runner. ✔
- No field written and never read: `closed_on` (now optional) by the date filter, the runner and the Review queue. ✔
- No branch only the switch knows: the page part moved to 3c whole, not stubbed. ✔
- No real-target run skipped: the suite was run (below); the page is not rendered here because it is not changed here.

## Verification

- `cargo test -q --workspace` in `rust/`: 996 passed, 0 failed. `RUSTFLAGS="-D warnings" cargo build -q --workspace --all-targets`: clean. The applet test alone: 1 passed.
- Engine cases: an agent that had not read the engine re-derived the 11 cases the change moved (`events.json` 1, `options.json` 7, `shares.json` 2, `coins_and_transfers.json` 1) and wrote the 8 cases of `open_trades.json` from `SPEC.md` (partly sold ETH open; sold to flat closed; realized counts an open trade's sale while win rate, profit factor and expectancy do not; one trade across two months; a date range; an open roll chain; a transfer out with no sale is no trade; a saved group open while a member is). The engine agreed with every figure but two readings `SPEC.md` left open, settled against it (`cases/README.md`): an open trade with no sale has realized 0.00; a spin-off's children are opened by the event the adjustment applies to. `cargo test -p bagholder-engine --test cases`: green.
- Brief 04 §8's blind check on the files this changes: the eleven moved cases are that check, re-derived blind; the other cases of those files are unchanged.

## Handoff

Done for the engine, `SPEC.md` and the cases. The Trades list's wire and page (open trades in the list, Close reading `Open`, opening one opens the holding page) are built in 3c with the switch.

**Nothing left running.**
