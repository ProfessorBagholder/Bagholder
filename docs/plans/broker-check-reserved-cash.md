# Plan: the broker check counts the cash Wealthsimple has reserved for open orders

## For the owner to decide

Nothing open.

## Scope

The broker check (`SPEC.md` §6, the header: "each holding or cash balance where the broker check finds the book's units or cash differing from the broker's statement") compares the cash the book has booked with the cash Wealthsimple states. Wealthsimple's stated cash (`balance(type: TRADING)`) is net of the cash it reserves for open buy orders, which the book rightly does not book until they fill. So every open buy order makes a false disagreement in the header: on 2026-10-01 an option buy order resting at a limit put "USD cash in Trading: Wealthsimple states $14,881.57, the book holds $14,951.57" up, the difference being exactly that order's stated amount ($70.00). This plan keeps, with each statement of cash, the cash the activity feed states as reserved by open orders at the same read, and the check compares the book's cash less that reservation with the stated cash. Out of it: what the screen shows (unchanged but for the false sentence going), the Orders panel, and cash reserved for anything the feed does not state as a pending order (see Open questions).

## The old app here

The old app had no broker check. Nothing is carried over.

## How the leading products do it

- Brokers separate the cash balance from what is available to trade, the difference being what open orders hold. Wealthsimple reserves cash for an open buy order while it is active and releases the unfilled part when it fills, is cancelled or expires (Wealthsimple Help Centre, "Bid on Initial Public Offerings" and "Understanding market orders", https://help.wealthsimple.com/hc/en-ca/articles/50825154775451 and https://help.wealthsimple.com/hc/en-ca/articles/360058451433, search summary read 2026-10-01; the pages themselves answer 403 to a fetch). Interactive Brokers reports cash and "Available Funds" separately for the same reason.
- Portfolio trackers that reconcile against a broker (Sharesight's cash account reconciliation) reconcile booked transactions, not orders: an open order is not a transaction. The book follows that; the reservation is a fact about the broker's balance, kept beside it.

## Open questions

- **Whether `TRADING` cash also nets cash reserved for open short puts (a cash-secured put).** Objective: no false disagreement while one is open. Known: the feed states no pending row for an open short position, so this plan does not cover it; the owner's statements history holds every stated cash since the book began, and the book every short put sold. Settled by checking that history for a period with a short put open: if the stated cash is lower by strike × 100 × contracts while it is open, the reservation is derived from the open short puts the same way; if not, nothing is needed. Checked before building, from the data already on this machine.

## Approach

- `bagholder-wealthsimple`: a function over the rows an activity read returned that gives, per currency, the amount of every row whose `unifiedStatus` is `PENDING` and whose type is a buy (`*_BUY`, and an `OPTIONS_MULTILEG` whose amount is a debit), as the row states it. Read strictly: a pending buy with no amount is a mismatch, said.
- `bagholder-broker` (`pull.rs`): at each full activity read, the reservations go to the book with the cash stated at that read (the same read id the check already pairs, `cash_read`).
- `bagholder-book`: migration 0NN adds `statement_reserved(statement_id, currency, amount)` beside `statement_cash`; `Book::stated()` returns `cash_read` with its reservations.
- `bagholder-engine` (`equity.rs::broker_checks`): the book's cash in a currency less the reservations stated at that read is compared with the stated cash. `Difference::Cash` is unchanged.
- `SPEC.md` §6: the sentence on the broker check says the comparison allows for the cash the broker reserves for open orders, and why.

## Acceptance criteria

- [ ] `cargo test --workspace` green with no warnings; `cargo clippy --workspace --lib --bins` clean.
- [ ] A case in `engine/tests/cases/returns_filters_checks.json`, its expected figures written without reading the engine: an account with an open buy order states cash lower by the order's amount, and no difference is said; when the order's row is final (filled, cancelled, expired), the reservation is gone and the fill, if any, is booked.
- [ ] A mapping test on the recorded reply of a pending option buy and a pending share buy: each gives its amount in its currency; a pending sell gives nothing.
- [ ] On a copy of the owner's data from 2026-10-01: the USD sentence for the Trading account is gone, and no other sentence changed.
- [ ] The open question on short puts answered from the owner's statements history, with the periods checked named in the PR.

## Surfaces to check beyond the diff

The migration and its schema snapshot; `Book::stated()` callers; `engine_inputs.rs` where `cash_read` is set; `docs/old-app-mistakes.md` (nothing to add: the old app had no check).

## Right to refuse

If the short-put history shows reservations the feed does not state and cannot be derived, stop and report before building that part.

## Anti-stub self-check

To initial when built.

## Verification

To fill when built.

## Handoff

Not started.
