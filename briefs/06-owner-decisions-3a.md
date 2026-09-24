# Brief 06: the owner's decisions that change stage 3a

Owner decisions of 2026-09-24. Record them in `docs/decisions.md`, then adjust 3a.

## 1. Equity curve, returns and drawdown: Wealthsimple's stated daily account value

- **Each account's daily value and net deposits** come from Wealthsimple's historical financials, which the pull already reads (`FetchAccountHistoricalFinancials`). That is what `SPEC.md` defines today ("NAV from sync").
- **The total** is the sum of the accounts. Returns are chain-linked daily returns net of deposits and withdrawals, as `SPEC.md` §2 says.
- **Keep the value per account in the engine**, so a future account with no broker statement can get a computed value later. Don't build that now.
- **Stop:** recorded option closes, the after-close option read, Cboe as a source, and reading past closes of every holding for the equity series. Keep only the past closes a chart shows, and the underlying's close on an option's expiry day, which the expiry rule needs.
- **The spec change** "equity is Bagholder's own" comes off the switchover list.

## 2. Option prices on screen: a free source, fetched only when needed

- **Not Wealthsimple, not Cboe.** Cboe's delayed-quotes terms forbid automated extraction.
- **Candidates:** Yahoo's option chain (the service already used for US quotes and bars) and Nasdaq's. Confirm each with one read, and read its terms, before writing a reader. With no permitted free source, the position's price waits, named.
- **Fetched only when needed.** A price is stored with its own time. It is fetched again only when a screen shows it, its market is open, and the stored one is older than the source can improve on. After the close, the stored final price stands until the next session. A page opening or reloading never causes a fetch by itself.

## 3. Benchmarks: the same kind of return as the owner's

- **The owner's figure is a total return.** It comes from the account value net of deposits and withdrawals, and dividends stay in that value. It is in CAD.
- **So each benchmark is total return in CAD:** a dividend-adjusted close of an ETF tracking each index (Yahoo's adjusted close includes dividends), converted daily at the Bank of Canada rate. Confirm the series with one read each.
- **Price-only FRED and TMX index levels stop being benchmarks.**

## 4. Next, for stage 3b: facts the owner enters

- **Two cases:** the cost of a holding moved in, and the allocation of a spin-off or return of capital, taken from the issuer's published figure. This is how Sharesight and similar trackers do it.
- **Built only through existing patterns:** an opening balance in "Add trade", and an event's allocation on the trade detail beside the journal. `SPEC.md` defines both first.
- **3b drops its per-issuer event readers.** The official Canadian corporate-action feeds are paid.
