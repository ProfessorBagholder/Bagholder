# Outside reference: how established systems do it

The reviewer's standing reference. Every gate verdict is checked against it. Each line comes from a primary source, fetched on 2026-09-24. **UNVERIFIED** marks what could not be confirmed. This file is not an instruction to the building session.

## Constraints every recommendation must pass first

Each recommendation is checked against every line here before it reaches the owner or a brief. A recommendation that fails one is dropped or reworked.

1. **Who it is for:** "a local-first trading journal for Wealthsimple users" (README, `CLAUDE.md`), self-hosted, one person, public repository.
2. **What it may depend on:** a Wealthsimple account and sources that need no account or key. Anything that needs another account (Questrade, TradingView) is optional: it improves what already works, and nothing goes blank without it.
3. **Wealthsimple's API is unofficial.** It is read only for what only Wealthsimple has (accounts, activity, positions, orders), and only as often as that needs.
4. **A source's terms:** no source is read in a way its own terms forbid (Cboe's delayed quotes forbid automated extraction).
5. **Scope:** the migration only; the Rust+Svelte build that replaces the old app. Nothing for the old app or the period before the switchover. Nothing the owner hasn't asked for, and nothing for futures not on the table (`docs/decisions.md`).
6. **What the owner sees:** the UI and UX stay as they are unless `SPEC.md` changes on purpose. No captions, tooltips or helper text.
7. **Figures:** exact, from stated facts, never a guess presented as a fact; per-instrument figures in the instrument's currency, aggregates in CAD.
8. **Method:** each recommendation rests on the code as it is (read, not assumed) and on established practice (this file), and is the best course for its objective with the information already at hand.

## Facts the broker's data doesn't carry (cost basis, corporate actions)

**Transfer-in cost at brokers**
- **Wealthsimple:** the book cost carries over on an ATON transfer. "If your previous institution is unable to send this information… upload a recent statement… we'll update your records" (help.wealthsimple.com/hc/en-ca/articles/1500003503661).
- **Wealthsimple crypto** sent in from an outside wallet: "book value… based on the fair market value at the time the transfer is completed… We can't change the book value" (…/4405752552859).
- **Questrade:** cost can arrive as zero, and is fixed through support from a statement (questrade.com, brokerage-transfer-in-request; tax-season FAQ).
- **RBC Direct Investing** uses the market price on arrival, and the client files a Book Cost Form, which it "will not verify for tax purposes" (rbcdirectinvesting.com, "what is book value").
- **TD Direct Investing:** a "Book Cost Adjustment Form" (near-verbatim).

**Corporate actions at the broker.** Wealthsimple processes stock splits and reverse splits automatically (…/43045850436635). Return-of-capital book-cost adjustments are made retroactively after tax season (…/4409775037083).

**US regime.**
- Covered securities carry their basis on transfer (IRS 1099-B instructions).
- Issuers file Form 8937 for actions that affect basis.
- Fidelity lets the customer update only lots coded "unknown basis or customer provided basis", never covered ones (fidelity.com, how-to-change-your-cost-basis-info).

**Portfolio trackers**
- **Sharesight entry types:** "Opening Balance – Record the starting quantity and cost of a holding you already owned", and "Adjust Cost Base" (help.sharesight.com).
- **Sharesight corporate actions:** splits are automatic on "All supported markets". Return of capital is automatic on ASX/NZX only, manual elsewhere. Spin-offs are manual, using the issuer's allocation (TC Energy / South Bow 91/9).
- **AdjustedCostBase.ca:** a spin-off is recorded as a return of capital on the parent plus a buy of the child, using the issuer's published allocation.

**Issuers publish spin-off cost allocations.** TC Energy: "91% to a TC Energy common share; and 9% to a South Bow common share", "not binding on shareholders or the Canadian tax authorities" (tcenergy.com tax-information PDF). One issuer checked.

**Trading journals.** Tradervue and TradeZella let users add and edit executions by hand. Their stock-split handling is **UNVERIFIED**.

**Aggregators.** Plaid's `cost_basis` is nullable, and so is SnapTrade's `average_purchase_price`.

**Canadian corporate-action data**
- CDS bulletins need a login, and TMX Datalinx sells the CDSX entitlements, TSX Bulletins and TSX Listings Changes feeds: all paid.
- TSX Venture's daily bulletins are public on newswire.ca.
- No free feed of senior-board TSX bulletins was found (**UNVERIFIED** that none exists).

**The pattern:** every product lets the account holder supply what no feed carries, records it as theirs, and lets an authoritative value replace it. Splits are automated; spin-off allocations and return of capital are mostly entered by the user from the issuer's figure.

## Orders

**FIX order states** (orchimate.org, FIX latest)
- "Pending Cancel" and "Pending Replace" do "NOT INDICATE THAT THE ORDER HAS BEEN" cancelled or replaced.
- "Too late to cancel" is a normal cancel reject.
- An Order Status Request by `ClOrdID` resolves an order whose acknowledgement was lost.
- `ClOrdID`: "Uniqueness must be guaranteed within a single trading day… should ensure uniqueness across days".
- `ExecID`: unique per execution. A cancelled fill references the original through `ExecRefID`, and `PossDupFlag` marks a possible retransmission.
- `LeavesQty = OrderQty - CumQty`; `LastQty` is the quantity of this fill alone.
- An explicit FIX rule to skip an `ExecID` already booked: **UNVERIFIED**.

**HTTP, RFC 9110 §9.2.2:** "A client SHOULD NOT automatically retry a request with a non-idempotent method unless it has some means to know… or some means to detect that the original request was never applied."

**Stripe idempotency keys:** the first result is saved "regardless of whether it succeeds or fails", and a different body under the same key is an error.

**Brackets and OCO**
- IBKR bracket: when the target fills, "the Stop Sell order, is canceled". In a one-cancels-all group, a partial execution reduces the other orders "proportionately".
- Schwab: cancelling the other leg avoids "inadvertently opening a short position".

**Stops the broker watches itself** (IBKR simulated stops, disclosure 9130)
- They trigger only in regular hours, with "a valid bid/ask quote", and when "the last trade price is within, or not more than 0.5% outside of, the consolidated bid/ask".
- A "double bid/ask" method needs two consecutive prices.
- IBKR "may ignore last sale data that is reported outside the prevailing bid-ask".

**Wealthsimple orders** (help centre)
- Stop-market works for stocks, ETFs and options, in regular hours only.
- A Canadian stop-limit's stop and limit must be equal.
- Orders last for the day, or "Good until cancelled (up to 90 days)".
- **After a partial fill**, "the remaining part of your order will expire at the end of the trading period, regardless of the expiry date set".
- "You can't modify the duration". A stop-market's stop price can be edited, its quantity can't.
- Trailing stops, OCO and brackets: no article found (**UNVERIFIED**, most likely not offered).

**Gaps:** "will trigger immediately at market open… much lower than your stop price" (Wealthsimple). A stop kept in client software doesn't work while the software is off (MetaQuotes).

## The page's data and a local server's security

**Stale-while-revalidate**
- RFC 5861: serve stale "while still serving stale responses (i.e., without blocking)" and revalidate.
- SWR returns cached (stale) data, then revalidates. TanStack Query "immediately returns the available cached data" while it refetches.
- Persisted caches: `buster` invalidates across builds, and `maxAge` "silently" discards old caches (24 hours by default).

**Server-sent events.** `id` sets the last event id, and the client sends `Last-Event-ID` on reconnect. `retry` sets the reconnection time. A 204 stops reconnection. A comment line "every 15 seconds or so" keeps the connection alive. EventSource can't set custom request headers. Replaying missed events is left to the server.

**Browser storage (MDN)**
- Best-effort by default, and `persist()` "may or may not" be honoured.
- Eviction removes the least recently used origin, "all of its data".
- Safari deletes script-written data after seven days without interaction.
- Storage is per origin (scheme, host, port).

**Local servers and DNS rebinding**
- Transmission, CVE-2018-5702: a custom header "doesn't work because of an attack called 'DNS rebinding'", so a Host allow-list was added.
- Geth's `--http.vhosts` (server-enforced, default "localhost").
- Jupyter rejects non-local Host headers, has tokens on by default, and requires same-origin pages or a token for its API.
- OWASP: custom headers force a CORS preflight, and `Sec-Fetch-Site` should be checked.
- Chrome's Local Network Access launches in Chrome 142. The spec says services must still defend against CSRF themselves.

**Normalized stores**
- Redux: keep items "with the IDs of the items as keys", and nesting forces unrelated re-renders.
- RTK entity adapter: `{ids, entities}`.
- Apollo: a flat cache keyed by `__typename` + `id`, merged on the same id.

## P&L and returns

**GIPS**
- A time-weighted return "negates the effects of external cash flows".
- Portfolios are valued "on the date of all large cash flows", and sub-period returns are "geometrically link[ed]".
- Modified Dietz and IRR are acceptable estimates.
- "Returns for periods of less than one year must not be annualized."
- Returns are after transaction costs.
- "a price-only index will not satisfy". A benchmark must be "of the same return type…, in the same currency, and for the same periods".

**CRA**
- Identical properties use average cost (ACB), and "Dispositions of identical properties do not affect the ACB".
- A superficial loss involves a repurchase within 30 days before or after the sale, and the loss is added to the substituted property's ACB.
- T5008 box 20 "may or may not reflect your adjusted cost base".

**Trading journals**
- Tradervue starts a new trade on a change of side, or (by setting) whenever the position goes flat, and switches between gross and net P&L.
- TradeZella offers FIFO, LIFO or weighted average ("recommended to leave it as FIFO"), with gross and net P&L.

**Sharesight:** a "dollar-weighted" return, "a variation of the Modified Dietz method", aimed at an investor's portfolio. Periods under a year are not annualised.

**The equity curve** is the running P&L of closed trades, which prices don't move.
- TradingView's strategy report: it "visualizes the dynamic changes in your account balance based on closed trades"; Cumulative PnL "shows the accumulated profit or loss after each closed trade".
- Tradervue: a cumulative P&L chart "for trades matching the current filter".
- TradeZella: cumulative P&L, the equity curve.

**A return set against an index**
- IBKR PortfolioAnalyst (MWR/TWR white paper): sub-period return = ending market value ÷ (beginning market value + cash flow) − 1, geometrically linked. "TWR is the preferred method of calculating returns by industry standards." A TWR/MWR toggle sits beside up to three benchmarks.
- TradingView Portfolios: TWR over "free cash, current value of open positions, and accumulated realized profit". Its benchmark is bought and sold virtually on the portfolio's own trade dates.
- Wealthsimple (magazine, "wealthsimple-returns"): "If what you're interested in is comparing the performance of different investments or money managers, time-weighted return is the relevant number."
- CRM2 (NI 31-103): Canadian dealers must report a money-weighted return on the annual performance report. That is the personal return, not a comparison with an index.
- Money-weighted against an index: the index is run through the same cash flows, the Long-Nickels public market equivalent (PME). Neither TWR nor MWR counts a deposit as return.

**Option expiry:** OCC exercises an equity option $0.01 or more in the money unless the clearing member instructs otherwise (exercise by exception, Cboe circular RG08-073).

**Time zones in journals:** TradeZella has a display time zone setting that charts and statistics follow, and TraderSync an account setting. Tradervue works in US Eastern.

**Max drawdown** (CFTC, 17 CFR 4.10(l)): the "greatest cumulative percentage decline in month-end net asset value due to losses", so withdrawals are not drawdown.

## Prices

**Adjustment**
- CRSP's price factor adjusts for splits and stock dividends; cash dividends aren't in it.
- Alpha Vantage returns raw closes, adjusted closes and the split and dividend events together.
- Yahoo: undocumented. A live read showed `close` split-adjusted, `adjclose` dividend-adjusted as well, and split events in the reply.

**Delayed data**
- Cboe and Nasdaq: delayed means at least 15 minutes, and a delay message must "prominently appear on all displays".
- Cboe's delayed-quotes page: "IT IS STRICTLY PROHIBITED TO DOWNLOAD DELAYED QUOTE TABLE DATA FROM THIS WEB SITE BY USING AUTO-EXTRACTION PROGRAMS."
- Observed on 2026-09-24 during market hours: `cdn.cboe.com/api/global/delayed_quotes/options/*.json` returned data timestamped about 35 hours earlier, still listing expired series.

**Yahoo delays:** TSX and TSXV real-time, OPRA 15 minutes, crypto real-time from Coinbase. The data "is not intended for trading or investing purposes".

**Bad prints can be cancelled after they are shown**
- Nasdaq's clearly-erroneous thresholds: 10%, 5% and 3% by price band.
- CIRO cancelled WSP trades on 2026-09-22 (tsx.com trading notice).

**Timestamps**
- CTA quotes carry the exchange's own timestamp.
- Coinbase: the Exchange ticker has `time`; the v2 spot price has none.
- FRED SP500 is the daily close, with ten years of history.

**Option contract adjustments**
- OCC doesn't adjust for ordinary cash dividends, nor for specials under $0.125.
- An adjusted option gets "a numeral following the letters of the option symbol", and the deliverable changes (for example 5 shares after a 1-for-20), with the multiplier "to remain 100".
