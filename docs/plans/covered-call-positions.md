# Plan: a covered call is shown with the shares it covered, as one position

## For the owner to decide

Nothing open. The owner raised that covered calls are misread more than once (most recently 2026-11-01, on an agent's analysis); how a journal counts a covered call is a design question with a standard answer, settled at the gate (Architect, #662):

- **The position is the trade the statistics count.** Every statistic (P&L, win rate, worst trade, by symbol, grades, the P&L curve, the Options and Shorts splits) counts a covered call inside its shares' position, never as a trade of its own. That's how the brokers below show the strategy, and it's the only reading under which a covered call's loss can be "the worst trade".
- **No switch.** A switch keeps the wrong reading one click away, and agents, which don't click, would read it.
- **The call keeps its own row and its own P&L** on the Trades page, marked as written against its shares (Side reads Covered call; its row and page link to the shares' trade), so what each call did stays visible; the statistics don't count it twice.

## Scope

**In it:**
- **The link:** each covered call (SPEC.md trade-69 to trade-74) is linked to the shares' trade that covered it.
- **The trade pages:** the shares' trade shows the calls written against it and the position's combined P&L. Each covered call's page names the shares' trade it was written against.
- **The Trades list:** the shares' row gains the combined P&L.
- **Agents:** the same link and figure, so an analysis of options never reads a covered call as a standalone short bet.
- **The statistics:** they count the position, not the call (above).

**Not in it:**
- Puts. A secured put is cash-backed, not a stock position.
- Uncovered calls.
- Premium-adjusted cost basis on the Portfolio tab (the "wheel" view). That's a separate figure; proposed below as a follow-up.

## The old app here

The Python app had no covered calls as such; nothing is carried over. Since 2.17.0 the Rust engine judges each written call's coverage from Wealthsimple's positions read of its opening day (`engine::coverage`). It never links the call to the shares.

## How the leading products do it

Read 2026-11-01.
- **Brokers group the stock and the call written against it as one strategy.** Questrade lists the covered call among its 17 multi-leg strategies, "holding a long position in a stock/ETF … and then selling a call option on that same stock or ETF" (questrade.com/learning/using-questrade/multi-leg-options-trading). Interactive Brokers' Strategy Builder includes covered stock strategies (interactivebrokers.com). Upstox groups a strategy's positions "so traders can track the P&L for individual positions as well as for the entire strategy together" (help.upstox.com, article 254554). Both figures, each leg's and the position's.
- **Journals:**
  - TraderSync groups multi-leg option structures into one trade with consolidated P&L, but its documented structures don't include the covered call (purepowerpicks.com/tradersync-review).
  - TradesViz's wheel tracker reduces the share basis by the option premium (tradesviz.com/wheel-strategy-tracker), as QuantWheel's guides do (quantwheel.com/learn/cost-basis-tracking).
- So the category keeps each leg's own figure and adds the position's. Bagholder does the same, from the coverage it already judges.

## The rule (SPEC.md §2 Trade, new)

- **What covers a call:** a covered call (trade-71) is written against the shares' trade of its underlying that was open in the same account at that day's close: one round trip per account and instrument is open at a time (trade-1), so there is at most one.
- **No link:** where the shares came with no row the journal holds (a transfer the feed doesn't list, trade-67), there's no share trade. The call is covered but linked to nothing, its page says so, and no combined figure is made. Never a guess.
- **The combined P&L:** the shares' trade's P&L plus that of every call written against it, each in the instrument's currency; in CAD, each one's CAD figure added.
- **Assignment:** an assigned call already delivers the shares at the strike into the shares' trade (trade-17), so the combined figure counts each dollar once.
- **Partly covered:** a call only partly covered (`1 of 2`) is linked for the contracts covered. Its combined share is that fraction of its P&L; the rest stays the call's own, uncovered.

## Approach

- **Engine:**
  - `coverage::coverages` already walks each covered opening with its account and underlying. It records the shares' trade key open at that day's close, found through `Matched::trips_of` on the underlying and the account, as `Coverage::Read { …, against: Option<TradeKey> }`.
  - A new `TradeFig.written_against_it: Vec<TradeKey>`, and the combined P&L, are derived after `assign`, in the same pass and under the same incremental rule (accounts touched only).
- **Wire:**
  - `Trade.coverage.against` (the shares' trade id);
  - `Trade.callsWritten` (ids);
  - `Trade.positionPnl`, `Trade.positionPnlCad`.

  Agents get them through the same views.
- **Page:**
  - The Trades list's shares row shows the combined P&L under its own, small, where it has calls.
  - The trade page lists the calls with each one's P&L and the total.
  - A covered call's page links to its shares' trade.
- **The statistics' scope:** every statistic reads the trades through one scope (`engine::scope`). A covered call's covered fraction counts in its shares' trade's figures (P&L, result, grade bucket, symbol, curve) and is not a trade of its own there; an uncovered fraction stays a trade of its own.

## Acceptance criteria

- [ ] Engine cases, their expectations written by an agent that had not read the engine:
  - a call covered and expired, with the shares sold later;
  - a call bought back at a loss while the shares rose;
  - an assigned call;
  - a call partly covered;
  - a call covered by shares transferred in with no row (linked to nothing);
  - two calls in turn against one share trade;
  - a share trade with no calls (no combined figure);
  - a covered call's loss never appears as a trade of its own in worst trade, win rate or by symbol: the position's figure does.
- [ ] The engine suite, including the step-by-step-versus-fresh-build tests, and the capacity growth test pass.
- [ ] A browser test drives the combined figure on the Trades list and on the trade page, and the link from a covered call to its shares.
- [ ] The agents' trade rows carry the link and the combined figure (a server test on the tool's answer).
- [ ] `SPEC.md` states the rule, with these sources.

## Follow-up, proposed, not in this plan

Premium-adjusted cost on a holding with calls written against it, as TradesViz and QuantWheel show it. It's a different figure (cost basis, not P&L) and a separate decision.
