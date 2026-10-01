# Brief 16: verdict on the reserved-cash plan (#339): Go with changes

**Reviewed:** `docs/plans/broker-check-reserved-cash.md` at `17668b0a` on `plan-reserved-cash`, against `master` at `1da70ca9`: `engine/src/equity.rs::broker_checks`, `broker/src/pull.rs::balances`, `book/src/statements.rs::stated`, migration 006, `wealthsimple/src/mapping.rs::map_record`, the GraphQL documents the app holds in both Wealthsimple crates, the recorded replies, and Wealthsimple's own help-centre articles on orders, IPO bids, secured puts and withdrawals (read through the help centre's JSON API on 2026-10-01).

## Verdict

Go with changes. The symptom is real and the layering is right: the adapter states the hold, the book keeps it with the cash statement, the engine subtracts it. But the plan builds the rule around the one row the owner saw (an option limit buy with a stated amount) and leaves the rest of the class as a question or out of scope. The rule the broker applies is wider, and the plan as written would turn one false sentence into others.

## What the broker actually does

From Wealthsimple's own articles, the `TRADING` balance is net of everything it holds against the account, not only open buy orders:

- A **limit buy** holds the order's amount (the owner's data: the feed's `PENDING` row states it, and the stated cash fell by exactly that).
- A **market buy** holds an amount the article does not quantify: "WSII has implemented a cash reservation on market buy orders to ensure you have sufficient funds in the account to cover the full balance of your trade in the event of a sudden price movement" (360058451433). Nothing states the figure, and a `PENDING` market-buy row may state no amount.
- An **IPO bid** holds "the top of the stated price range, plus a 20% buffer" until allocated, cancelled or released (50825154775451).
- A **short put** in a registered or cash account is cash-secured: "the buying power or cash amount is reserved and unusable for as long as the short put position is open", the strike × 100 × contracts less the premium received (43198629134235: "$10,000 … $9,650"); in a margin account it is secured by buying power, not cash.
- The available balance is also lower for "recent deposits (less than 5 days)", "unsettled proceeds from recently sold assets" and "funds that are reserved for transfers or fees" (360058451453). Which of these `TRADING` nets, as opposed to the withdrawable balance, is not stated.

The plan covers the first case, asks about the fourth, and does not mention the others. Under the first rule of `CLAUDE.md` as merged yesterday (a rule for any input, never around an example; a report names a symptom, the change fixes the class), that is not enough to build.

## Required changes

1. **Find out what the broker states before deriving anything.** The app already holds two documents that may answer this without derivation: `FetchAccountsWithBalance` takes a `BalanceType` and the app has only ever asked `TRADING`; `FetchTradingBalanceBuyingPower` (`ws/graphql`) returns `tradingBalanceViewV2 { cash buyingPower }`. Before building, establish the values of `BalanceType` (a GraphQL enum answers an invalid value with the list of valid ones) and what each balance and `cash` figure is for an account with an open buy order, from the owner's connection, dry. If the broker states a gross cash figure, or the held amount, the check compares with that and nothing is derived; this plan reduces to asking for it. The PR records the probe's answer either way.
2. **If holds must be derived, derive the class, not the case.** One rule in the adapter, "what this broker holds against an account and how much", with one arm per kind the broker documents: limit buy (the row's stated amount), market buy (unstated), IPO bid (unstated unless the row states the reserved figure), secured put in a non-margin account (strike × multiplier × contracts less premium), pending withdrawal or transfer out (verified from the owner's statement history, as the plan does for puts). An open item whose held amount the broker does not state is a hold of unknown size: the check for that currency is then pending, never "no difference" and never a difference. "A pending buy with no amount is a mismatch" goes: it is the broker's documented behaviour for market orders, not a malformed row.
3. **Key the held amounts by what holds them.** `statement_reserved(statement_id, currency, amount)` cannot say which order or position held the cash, and cannot hold an unstated amount. Store one row per hold: the statement, the record (the pending row or the open position) that holds it, its kind, currency, and the amount or "unstated". The engine subtracts the stated holds and marks the currency pending while any hold is unstated. Nothing new on screen: the existing sentence stays as it is, and a difference that remains is still said in the same words.
4. **Check the units side of the same rule.** A pending sell does not hold cash, but the broker's stated positions may be net of units under an open sell order. One check on the owner's data (the recorded `DIY_SELL … PENDING` row exists) and, if they are, the same hold applies to units in `broker_checks`.
5. **Settle the open questions from the data before building, as the plan says, and write the answers into `SPEC.md` §6** with the article each rule rests on, so the next session does not re-derive them.

## Accepted as is

The adapter-book-engine split; pairing holds with `cash_read`; reading pending rows strictly; the migration going through the gate; the old-app section ("no broker check, nothing carried over"); the e2e criterion on the owner's copy ("the USD sentence is gone, and no other sentence changed").

## Acceptance criteria to add

- The `BalanceType` and `tradingBalanceViewV2.cash` probe, with its answer, in the PR; the design taken follows from it.
- Engine cases, expected figures written without reading the engine: a limit buy open (no difference); a market buy open with no stated amount (pending, no difference said); a secured put open in a registered account (no difference, hold = strike × multiplier × contracts − premium); the same put in a margin account (no cash hold); an open sell (cash unaffected; units as item 4 finds).
- The statement-history checks for short puts and pending withdrawals, with the periods named, in the PR.
- `SPEC.md` §6 states the rule and cites the articles.

## For the owner

Nothing to decide. No screen changes: one false sentence stops appearing.
