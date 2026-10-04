# Plan: the broker check allows for everything the broker holds against an account

Revised for brief 16 (Go with changes); each required change is applied below and named where it lands. Revised again on 2026-10-03 under "Right to refuse": the owner's records contradict the source of the holds brief 16 approved (open question 4); the approach below takes them from the broker's own list of working orders instead, and is resubmitted for review before it is built.

## For the owner to decide

Nothing open.

## Scope

The broker check (`SPEC.md` §6, the header) compares the cash and units the book has booked with what Wealthsimple states. Wealthsimple's stated `TRADING` cash is net of what it holds against the account, which the book rightly does not book: on 2026-10-01 an open option limit buy put "USD cash in Trading: Wealthsimple states $14,881.57, the book holds $14,951.57" in the header, the difference being exactly that order's stated amount ($70.00). This plan makes the check allow for every hold the broker documents, for cash and for units, or say it cannot tell. Nothing new on screen: a false sentence stops appearing, and a real difference is still said in the same words. Out of it: the Orders panel; anything the broker does not document.

## The old app here

The old app had no broker check. Nothing is carried over.

## How the leading products do it

Wealthsimple's own help centre (read through its JSON API on 2026-10-01, brief 16):

- a limit buy holds the order's amount;
- a market buy holds an unquantified reservation "to cover the full balance of your trade in the event of a sudden price movement" (article 360058451433);
- an IPO bid holds "the top of the stated price range, plus a 20% buffer" (50825154775451);
- a short put in a registered or cash account is cash-secured, strike × 100 × contracts less the premium received, for as long as it is open (43198629134235); in a margin account it is secured by buying power, not cash;
- the available balance also excludes recent deposits, unsettled sale proceeds and funds reserved for transfers or fees (360058451453); which of these `TRADING` nets is not stated.

Brokers separate the cash balance from what is available to trade (Interactive Brokers reports cash and Available Funds apart). Trackers that reconcile against a broker (Sharesight) reconcile transactions, never orders: a hold is a fact about the broker's balance, kept beside it, not a transaction.

## Open questions

1. **What the broker states, before anything is derived** (brief 16, required change 1). Objective: compare with a figure the broker states, not a derived one. Known: the app asks `FetchAccountsWithBalance` only with `BalanceType: TRADING`; it holds `FetchTradingBalanceBuyingPower` (`tradingBalanceViewV2 { cash buyingPower }`) and `FetchTradingBalanceViewPendingOrderQuantity` (units committed to open orders) unused; an invalid `BalanceType` value is answered with the enum's valid values. Settled by a probe the app itself makes on its own session, read only, at its next balances read: the enum's values, each balance type and `tradingBalanceViewV2.cash` for every account, written to a file in the data folder. Run from the app itself because a second process using the session could rotate its single-use refresh token and sign the running app out. If the broker states a gross cash figure or the held amounts, the check compares with those and nothing below is derived; the PR records the probe's answer either way.
2. **Short puts and pending withdrawals** (required change 5). Settled from the owner's statement history (cash statements since 2026-09-27) against the short puts and withdrawals open in each period; the periods checked are named in the PR.
3. **Units under an open sell** (required change 4). The owner's records hold a pending `DIY_SELL` row; whether the stated position on its days is net of it is checked from the statement history the same way.

4. **Whether a pending row in the activity feed is a hold.** Settled by the 2.0.14 probe (2026-10-04 00:39 UTC), which read the orders back from Wealthsimple:
   - the $70.00 option limit buy of 2026-10-01 is `EXPIRED`: a day order that lapsed at that day's close, which is why its hold was gone by the 10-02 read;
   - the option sell of 2026-09-28 is also `EXPIRED`;
   - the stock sell of 2026-09-10 is `submitted`, `UNTIL_CANCEL`, lapsing 2026-12-08: still working.

   The activity feed agrees. Both option rows moved to `EXPIRED` (revision 2) at the first full read after each lapsed. The finding first recorded here on 2026-10-03, that the rows stayed `SUBMITTED`, was a query error: the query filtered on the pending status before taking each record's newest revision. So a pending row in the feed is a working order, and the feed is a sound source of holds. Two more facts from the probe: an order reads back by the row's `externalCanonicalId` (its `canonicalId` answers `NOT_FOUND`), and the working stock sell's 500,000 units are in the stated units in full. **Stated units are not net of a working sell, so a sell holds no units.**

## Approach

Only if the probe shows no stated figure (it shows none). The holds come from the activity rows read in the same pull as the cash, as brief 16 approved, now that open question 4 shows a pending row is a working order. Brief 17 accepted the working-order feed in place of the rows only on the 2026-10-03 finding, which was wrong. With the rows, the holds and the cash come from the same full read, and each hold is keyed by its record as brief 16 required. The order feed would need one more call per pull and would answer the same thing.

Brief 17 (Go with changes) applied:

- **Open question 4 is settled by reading the orders back before building** (required change 1): version 2.0.14 (#345) reads back each order a not-final activity row names by each id the row states (`status`, `timeInForce`, `expiredAtUtc`) and the working-order feed, once, at the app's next orders read on its own session; the answers go here and in the build's PR.
- **One branch, never a switch** (required change 2). The read-back shows the 10-01 buy ended (`EXPIRED`), so this branch is built: the documented rule is built as a rule (a working buy order holds quantity × limit price × multiplier, no commission, the $70.00 being exactly 1 × 0.70 × 100; a working market buy or IPO bid holds an unstated amount). One engine case holds it.
- **Unverified is unstated** (required change 3): pending withdrawals and transfers out, IPO bids and market buys are holds of unknown size (the check pending while one is open). The secured-put arm is built from the article's figure (strike × multiplier × contracts less the premium) and named unverified in the PR, no short option having been open. If the read-back shows stated units net of a working sell, units compare with `FetchTradingBalanceViewPendingOrderQuantity`, nothing derived.
- **The stale pending row** (required change 4): there is none. Issue #346 was filed on the wrong finding and is closed.

Brief 16's approach, kept for the record:

- `bagholder-wealthsimple`: one rule, what the broker holds against an account, with one arm per documented kind (required change 2): a limit buy, the pending row's stated amount; a market buy, unstated; an IPO bid, unstated unless the row states the reserved figure; a secured put in a non-margin account, strike × multiplier × contracts less the premium received; a pending withdrawal or transfer out, as the history in open question 2 shows; an open sell's units, as open question 3 shows. A pending row with no amount is a hold of unknown size, never a mismatch.
- `bagholder-broker` (`pull.rs`): at each full activity read the holds go to the book with the cash statement of that read (`cash_read`).
- `bagholder-book`: a migration adds `statement_holds(statement_id, record_id, kind, currency, instrument_id, amount NULL, quantity NULL)`, one row per hold, keyed by the record that holds it (required change 3); `NULL` amount is "unstated". `Book::stated()` returns them with `cash_read`.
- `bagholder-engine` (`equity.rs::broker_checks`): the book's cash in a currency less its stated holds is compared with the stated cash; while any hold in that currency is unstated, the currency's check is pending (no sentence either way). Units the same with unit holds.
- `SPEC.md` §6: the rule, with the articles it rests on (required change 5).

## Acceptance criteria

- [ ] The probe's answer (the `BalanceType` values, each balance and `tradingBalanceViewV2.cash` for an account with an open buy) in the PR, and the design taken follows from it.
- [ ] `cargo test --workspace` green with no warnings; `cargo clippy --workspace --lib --bins` clean.
- [ ] Engine cases in `engine/tests/cases/returns_filters_checks.json`, expected figures written without reading the engine: a limit buy open (no difference); a market buy open with no stated amount (pending, nothing said); a secured put open in a registered account (no difference; the hold is strike × multiplier × contracts − premium); the same put in a margin account (no cash hold); an open sell (cash unaffected; units as open question 3 finds).
- [ ] A mapping test on recorded replies of a pending limit buy, a pending market buy and a pending sell.
- [ ] The statement-history checks of open questions 2 and 3, with the periods named, in the PR.
- [ ] On a copy of the owner's data from 2026-10-01: the USD sentence for that account is gone, and no other sentence changed.
- [ ] `SPEC.md` §6 states the rule and cites the articles.

## Surfaces to check beyond the diff

The migration and its schema snapshot; `Book::stated()` callers; `engine_inputs.rs` where `cash_read` is set; the probe's file is written once and read by nothing at run time.

## Right to refuse

If the probe shows a stated figure, the derivation is not built. If the history shows holds the feed does not state and cannot be derived, stop and report before building that part.

## Anti-stub self-check

To initial when built.

## Verification

Built in #347 (version 2.0.15); its body quotes each acceptance criterion with its evidence.

## Handoff

Revised for brief 16; the probe ran (2.0.13): no gross cash and no held amounts stated. Building stopped at open question 4; brief 17: Go with changes, applied; the 2.0.14 probe settled open questions 3 and 4; built in #347.
