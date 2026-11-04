# Plan: an agent reads every account's money in and out, and a transfer names the account on its other side

## For the owner to decide

Nothing open: the owner's ask is in `docs/decisions.md` (2026-11-04).

## Scope

**In it:**
- The agents' `transactions` read (`GET /api/transactions`, the `transactions` tool) covers every account the book holds, open or closed: an investment account's deposits, withdrawals, contributions, transfers, fees and interest, beside the chequing and credit card rows it covers now.
- Each row says what is on its other side, where Wealthsimple states it: one of the person's own accounts, named as the `lists` tool names it; outside Wealthsimple, with the institution's name where one is stated; or that the source does not say.
- A row whose amount Wealthsimple does not state is listed, with no amount and the reason the book keeps, never left out.
- SPEC `agents-2` and `agents-6`; `docs/decisions.md`.

**Out of it:**
- Rows that name an instrument (buys, sells, dividends, an instrument's interest): they are the `trades` and `income` tools' rows, with their own figures; listing their cash here a second time would give an agent two answers for one trade.
- The page: no screen lists these rows today and none is added (an agent reads the page's own route, as every tool does).
- Matching two rows by amount and time where Wealthsimple names no other account: the read says what the source states, never a guess.

## What happens now

Read from the code, 2026-11-04, at f8806821:
- **Two kinds of account only:** `http/model.rs:220` keeps the accounts whose kind is `Spending` or `CreditCard`; an investment account's rows never reach the read, and `account` naming one is refused (`:226`).
- **Rows with no amount are dropped:** `http/model.rs:232` keeps `t.cash.is_some()` only. A transfer whose amount the feed leaves null (recorded: `rust/crates/wealthsimple/tests/replies/wealthsimple/edited-activity-one-per-kind.json`, `INTERNAL_TRANSFER` rows with `amount: null`) is left out without a word, so a month's count can come up short.
- **No other side:** `bagholder_core::transaction::Transaction` (`core/src/transaction.rs:67`) holds no other account. The read already opens each page row's kept record to name its company (`http/model.rs:250`, `Book::newest_payloads` at `book/src/records.rs:236`, `wealthsimple::spending::company_of_record` at `wealthsimple/src/spending.rs:22`).
- **What the kept record states:** each record's payload is Wealthsimple's row as read (`source_records`/`record_revisions`, `book/migrations/001-the-book.sql:103`). A move between the person's own accounts carries `opposingAccountId` (the recorded reply above: `INTERNAL_TRANSFER` `SOURCE` and `DESTINATION` rows, each naming the other's account); a transfer from another institution carries `institutionName` (`INSTITUTIONAL_TRANSFER_INTENT` `TRANSFER_IN`); a bank transfer (`DEPOSIT`/`WITHDRAWAL` `EFT`) names no account and is outside Wealthsimple by its type; a `LEGACY_INTERNAL_TRANSFER` row names no account. The mapping reads `opposingAccountId` only to work out what a move of holdings moved (`wealthsimple/src/mapping.rs:888`, `:1215`).
- **The book's own account for a broker's id:** `Book::account_by_ref_value` (`book/src/identity.rs:123`).

## How the leading products do it

Read 2026-11-04.
- **Plaid, the aggregator most finance apps read through.** A bank transaction carries `counterparties`, "such as the merchant or the financial institution ... extracted by Plaid from the raw description", typed `financial_institution`, `merchant`, `payment_app` and others; its categories include `TRANSFER_IN_ACCOUNT_TRANSFER` ("General inbound transfers from another account") and `TRANSFER_OUT_INVESTMENT_AND_RETIREMENT_FUNDS`. An investment transaction has only its own `account_id` ("the account against which this transaction posted") and a free-text `name`: nothing identifies a transfer's other account. Sources: https://plaid.com/docs/api/products/transactions/, https://plaid.com/docs/api/products/investments/, https://plaid.com/documents/transactions-personal-finance-category-taxonomy.csv.
- **Wealthsimple's own feed** names the other account of a move between the person's accounts (`opposingAccountId`) and the other institution of a transfer in (`institutionName`), as recorded above.

So the source Bagholder reads states more than an aggregator infers, and the read passes on what it states: the other account by the book's own id and name, an institution by its name, and nothing inferred from a description.

## Open questions

None. Whether to store the other account on each transaction (a new column, filled by a mapping version bump through `Book::rederive`, `book/src/records.rs:480`) was weighed and not chosen: the fact is in the kept record already, the read already opens that record for each row it answers, and a stored copy would be a second home for one fact plus a migration, with nothing else reading it.

## Approach

1. **Every account.** The read lists `cash_of` (`engine/src/input.rs:116`: an account's transactions that name no instrument) for every account the book holds, open or closed; `account` takes any account's id, as `lists` gives it, and an id the book does not hold is refused naming it, as now.
2. **Every row.** A row with no stated amount is listed with `amount` empty and `problem`: the reason the book keeps for that record, in its words. The read's other fields are as now.
3. **The other side**, read from the row's kept record beside its company, one function in `wealthsimple::spending` (`other_side_of_record`), answered as `other`:
   - `opposingAccountId` stated and the book holds that account: `{ "kind": "own", "accountId": <the book's id>, "account": <its name as lists gives it> }`. The broker's id never reaches the answer (SPEC `agents-6`).
   - `opposingAccountId` stated and not an account the book holds: `{ "kind": "own" }`, the account unnamed.
   - a row whose type moves money to or from outside Wealthsimple (a bank transfer, a pre-authorized debit, a bill payment, an e-Transfer, a card funding, a transfer from another institution): `{ "kind": "outside", "institution": <institutionName where stated> }`. An e-Transfer's or a P2P payment's names and messages are never read, as for the company.
   - otherwise: `{ "kind": "unstated" }`.
   The types that are outside are a table beside `COMPANY` with its source (the recorded reply), never inferred from an amount.
4. **SPEC:** `agents-2`'s `transactions` line says every account and the other side; `agents-6` adds that a transaction names the account on its other side by the book's own id, an institution by its name, never a person's name or an account number. `docs/spec-ids.md` only if an id is added.
5. **Cost:** the read pages as now (up to 1,000 rows); for each row on the page it already reads one kept record for the company, and the other side comes from the same record, so no further read and no request to Wealthsimple.

## Acceptance criteria

- [ ] An investment account's deposit, withdrawal and transfer rows reach the read, and `account` naming it is answered. A server test on the made-up book. Plant: the `Spending | CreditCard` filter put back fails it.
- [ ] Both rows of a move between the person's own accounts name each other by the book's id and name. A test over the recorded `edited-activity-one-per-kind.json` `INTERNAL_TRANSFER` pair. Plant: `other` left empty fails it.
- [ ] A transfer from another institution names the institution; a bank transfer says outside; a row that states neither says unstated. Unit tests in `spending.rs` on the recorded rows.
- [ ] No answer carries a person's name, an e-Transfer's or P2P payment's message, a payee's nickname, an account number or the broker's account id. A test over every row of every recorded activity reply in `wealthsimple/tests/replies/wealthsimple`.
- [ ] A transfer whose amount the feed leaves null is listed with no amount and its reason. Plant: `t.cash.is_some()` put back fails it.
- [ ] The read makes no request to Wealthsimple: a test with the stand-in counting its calls.
- [ ] `cargo test` for the touched crates green, clippy clean; SPEC and `docs/decisions.md` as above.
- [ ] In the PR: the read's time for a page of 1,000 rows on the reference-size book, before and after.

## Surfaces to check beyond the diff

- `web/src/lib/generated/` (the route's types: `TransactionRow` gains `other` and `problem`, `amount` becomes optional); `rust/crates/server/tests/golden/routes.json`.
- The agents' tool description (`agents/tools.rs:653`) says every account and the other side.
- `mcp.rs:117`, the route's agent entry.

## Right to refuse

If a recorded reply shows Wealthsimple stating the other account in a field this plan does not read, or a type this plan calls outside that is not, the table changes with the reply cited before the build.

## Anti-stub self-check

To be initialled in the PR.

## Verification

To be pasted in the PR.

## Handoff

To be written at the build.
