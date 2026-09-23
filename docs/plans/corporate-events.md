# Plan: corporate events and option contracts booked from the record, not guessed

## Scope

Every event that changes a holding without a trade is booked today by one of two shortcuts, and both are wrong for some inputs:

- **Shares that arrive without cash** (`mapping.rs` `is_corp_share_move` → `STKDIS`, folded by `normalize.rs` `fold_stkdis`) open a lot at **$0**. That is right for a name or ticker change (netted to nothing) and wrong for:
  - a **stock dividend** (a dividend paid in shares of the same issuer): for a Canadian holder, whether the issuer is Canadian or foreign, the shares' value on the day is dividend income and becomes their cost. Today: no income, $0 cost, and the whole value later shows as trading gain.
  - a **spin-off** (shares of a new issuer handed to the parent's holders): not income. The parent's cost is split between parent and child by their market values on the distribution date, and the child's lots keep the parent's acquisition dates. Today: child at $0, parent keeps its full cost, so the P&L of both is wrong.
- **Splits and consolidations** (`fifo.rs` `split_markers`): Wealthsimple posts a quantity-zero corporate action with no ratio, and the ratio is **inferred from the median of the user's own fill prices** either side of it. With no fill on one side the split is ignored (quantities wrong from then on); a ratio outside the rounding band is dropped.
- **Option contracts** are priced and sized at a fixed **×100** (`symbols.rs:80`, `fifo.rs:294/428/456/756`, `synth.rs:55`, `mapping.rs` unit price). Adjusted contracts (after a split, spin-off or special dividend) and mini contracts have another multiplier; Wealthsimple publishes it (`optionDetails.multiplier`), and it is not read.
- **Option names** print the strike to two decimals (`mapping.rs` `option_symbol`), so a strike of 2.125 is named 2.13, and two contracts can share a name.
- **Reinvested ("phantom") distributions** of Canadian ETFs (income added to cost, no new units): not known whether Wealthsimple reports them as a row at all.

`SPEC.md` defines only splits (§ Splits, line 24). It changes in this work: every event above gets its definition there.

## Approach

- **Classify from the record.** Wealthsimple's markers and the book decide the kind: shares of a security already held arriving without cash is a stock dividend; shares of a security not held, arriving while the parent is held, on the same day as a corporate-action row for the parent, is a spin-off; a quantity-zero corporate action is a split or consolidation; a code/name change stays as it is (already netted correctly).
- **Values from a source, never invented.** The day's value of a stock dividend and the parent/child market values of a spin-off are the listings' closing prices on the distribution date from the stored price history (`store::bars`, the chain the charts already use). A split's ratio comes from the market-data source's split record (Yahoo's chart API reports split events with their ratio; TMX where it has it), stored with the security, not from the user's fills.
- **When the source has nothing,** the lot is booked with an unknown basis, exactly as a deposited coin is since v1.46.0 (`normalize.rs:96`, `trades.rs:211`, `view.rs:389`): excluded from performance figures, flagged in the trades list, and named on the sync's error line. Never $0, never a guess.
- **Multiplier from the contract.** `FetchSecurity`/`FetchSecurities` also ask for `optionDetails { multiplier }`; the securities table stores it; every place that multiplies by 100 takes the contract's multiplier (100 only when the record says 100, or while it has not been read, which the sync then reads).
- **Strike at its own precision.** Two decimals, three when the strike has a third (`150.00`, `2.125`). A stored row keeps its name (raw rows are never rewritten); the model joins an option's rows by their `securityId` where they have one, so a name printed differently before cannot split a book.
- **Phantom distributions:** read the activity types in Wealthsimple's public web bundle (as `wealthsimple-order-api` did for the order operations) and settle whether they are reported; if they are, book them as income with the cost raised by the amount.
- **Where it is built.** This is a model change, and the shared cases every model runs are generated from the Python model today. Python and Go are frozen and deleted at cutover (stage 9 of `docs/architecture.md`), after which the Rust model is the reference and generates the cases. This work is built directly after cutover, on the Rust model and the Svelte page. Building it before would mean the same change in two frozen models and two phone models.

Deliberately unchanged: name and ticker changes, deposits of coins, the cash rows.

## Acceptance criteria

- [ ] A shared case per event, each asserting the lots, the trades' P&L and the Cashflow income: stock dividend (Canadian issuer, US issuer), spin-off (parent held in two accounts, child sold later; holding period carried), split and consolidation (with no fills on one side), a name change (unchanged), an adjusted option (multiplier 150) opened, closed, expired and assigned, a strike with three decimals, and each of these with no price in the history, which books an unknown basis. `cargo test -p bagholder-model --test cases` green on them.
- [ ] No lot in any case opens at $0 unless the record says it cost nothing; no split ratio comes from fill prices (`split_markers` deleted, test asserts a split with no surrounding fills is applied).
- [ ] No literal `100.0` contract multiplier left in the model (`grep` in the verification).
- [ ] The sync reads `optionDetails.multiplier` and the split record; golden tests (`ws/tests/golden_fetch.rs`, `market/tests/*`) extended to hold what is read and stored.
- [ ] A missing value is named on the header's sync error (the path added in b589193) and the lot is flagged in the trades list: e2e test.
- [ ] Phantom distributions: either handled with a case, or `SPEC.md` states, with the bundle evidence cited, that Wealthsimple does not report them.
- [ ] `SPEC.md` defines each event, the valuation source and the unknown-basis rule, in the same commit as the model change.
- [ ] Rendered on a scratch copy of the book at 1200/1340/1440/1680: the trades list, Cashflow and the holdings show the figures the cases assert.

## Surfaces to check beyond the diff

`tests/cases` (regenerated by the Rust generator after cutover), `web/src/lib/generated/wire.ts` if a flag is added to a trade, the securities table migration (new `multiplier`, split records), the phone models (on hold; they will diverge from the cases until their own parity pass), the release notes.

## Right to refuse

If Wealthsimple's markers cannot tell a stock dividend from a spin-off for some row, that row is booked with an unknown basis and named, not classified by a guess.
