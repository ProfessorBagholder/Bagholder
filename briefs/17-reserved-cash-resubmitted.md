# Brief 17: the reserved-cash plan resubmitted (#339 at `665ab9e3`): Go with changes

**Reviewed:** `docs/plans/broker-check-reserved-cash.md` at `665ab9e3` against `master` at `6b92c6f9` (the balance probe, #344, merged): open question 4 and the revised Approach; `server/src/orders/readback.rs` (the working-order feed and `FEED_STATUSES`), `orders/gate.rs::read` (`FetchSoOrdersExtendedOrder` by external id: `status`, `timeInForce`, `submittedAtUtc`, `expiredAtUtc`, `cancellationCutoff`); Wealthsimple's articles on order duration (4413542412187, 4413542667675, 360058451433, read 2026-10-03).

## Verdict

Go with changes. The resubmission is right to refuse the activity feed as the source of holds, and right to take them from the broker's list of working orders instead. Two things in it are not yet engineering: the open question is left with two readings when the app can settle it exactly, and the derivation is made to switch itself on from observed behaviour. Both go.

## What the data shows, and what settles it

The $70.00 hold is in the stated cash at 14:47 UTC on 2026-10-01, three and a quarter hours after the order was placed, and gone at every read from 2026-10-02 on, while the activity feed still says `SUBMITTED`. Wealthsimple documents that "limit orders for stocks and ETFs expire at market close if they can't fill within the day" unless placed good-until-cancelled (4413542412187), and says nothing different for options. The likelier reading is therefore the plan's first: a day order that expired at the close of 10-01, the hold released with it, and a feed row that does not move for an order placed outside this app. The app does not have to guess: `orders/gate.rs::read` reads any order back by its external id and gets `status`, `timeInForce` and `expiredAtUtc`; the activity row carries the ids. One read of the 10-01 order, and of the two open sells, answers open question 4 and open question 3 at once.

## Required changes

1. **Settle open question 4 by reading the orders back, before building.** `FetchSoOrdersExtendedOrder` for the 10-01 buy and the two open sells: their `status`, `timeInForce` and `expiredAtUtc` go in the plan under open question 4 and in the PR. The working-order feed on the same day, as the Orders panel reads it, is the second witness. "Either way" is not a finding when the answer is one call away.
2. **No self-switching derivation.** "Turned on only once a working limit buy has been seen held at its stated amount on two reads" makes the check's behaviour depend on what it happened to observe, which no test can hold and no reader of the code can predict. If item 1 shows the order ended, the rule is the documented one and is built as a rule: a working buy order holds quantity × limit price × multiplier (the $70.00 is exactly 1 × 0.70 × 100, so no commission is held), a working market buy or IPO bid holds an unstated amount. If item 1 shows a working order whose hold was released, the derivation is not built: while a buy order is working, the currency's check is pending, and why is recorded as an issue. One or the other; never a switch.
3. **Unverified is unstated.** Any kind of hold the broker documents but does not quantify, or that the owner's history cannot confirm (pending withdrawals and transfers out, IPO bids, market buys), is a hold of unknown size: the check is pending while one is open, as the plan already says for a failed read. The secured-put arm may be built from the article's own figure (strike × multiplier × contracts less the premium), the PR saying it is unverified on the owner's data since no short option was open. The units side has a stated figure the probe surfaced, `FetchTradingBalanceViewPendingOrderQuantity`: if item 1 ever shows stated units net of a working sell, compare with that, derive nothing.
4. **The stale pending row is a finding of its own.** The mapping re-reads a row "until it is final"; a `SUBMITTED` row for an order placed outside this app may never be, on this evidence. That is outside this plan, so it is an issue, not scope creep: what the app does with a pending row whose order the working-order feed no longer lists.

## Accepted as is

The probe and its answer (no gross cash and no held amount stated, so the derivation is needed); the working-order feed as the one source of order holds, read in the same pull as the cash and kept with that statement; a failed read making the statement's check pending; `statement_holds` keyed by the broker's order id; the engine rule (stated holds subtracted, any unstated hold makes the currency pending, nothing new on screen); the open-sell finding on units, subject to item 1 confirming those sells are working; the acceptance criteria as listed, plus one case for item 2's chosen branch.

## For the owner

Nothing to decide. Nothing on screen changes.
