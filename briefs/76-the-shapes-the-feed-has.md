# Brief 76: The shapes the broker's feed has, in the repository's own made-up Wealthsimple, so a pull change is provable before it ships

For every book and every copy. Line numbers are from master after 2.10.5. Nothing here asks the owner to run anything.

## 0. Why

2.10.0 shipped on generated books and planted faults, which prove that code does what it says, and broke a real book, because the broker's real feed has shapes the repository's made-up Wealthsimple never states: a move listed by its arrival alone, an account under two ids the broker links as one, statements across months whose closing balances settle a conversion's paid side, a row listed in a closed account after it closed. Every one of those shapes was already known to the code (rule 13, `read_beside`, the statements walk, `account_links`) and to the tests that edit recorded replies by hand (`rust/crates/wealthsimple/tests/pull.rs:331-372`, `:690-703`); none was in the stand-in every pull test and the capacity job build books from (`rust/crates/wealthsimple/src/generated.rs`), so a change to the pull could pass every test and be wrong. The owner will not run checks on their machine, rightly: the repository has every line of code, and the proof belongs in it.

## 1. The design

**The made-up Wealthsimple states every shape the pull reads.** `Generated` plans a book's activity (`generated.rs:203-230`, `Plan`) and answers each request with a recorded reply of the real shape, its fields set (`:1-7`). It gains the shapes below, each a planned piece of activity or a stated fact, each off by default and switched on by name, so a test says which shapes its book has and a pull change names the shape it touches:

| Shape | What the real feed does | Where its form is established, without the owner | Today |
|---|---|---|---|
| A move listed by its arrival alone | the arriving row names `opposingAccountId`; the other account's feed never lists a leaving row | `tests/pull.rs:370-384` (`resp_rows`), the RESP replies, rule 13's doc (`mapping.rs`) | hand-edited in two tests only |
| An account under two ids the broker links | the old id `closed`, the new `open`, each naming the other as `linkedAccount` | `tests/pull.rs:690-703` (`link_resp`), `edited-accounts-one.json` | hand-edited in one test |
| A row listed in a closed account after its one full read | a `SOURCE` row dated after the account closed | the `LateMove` fixtures (`tests/pull.rs:419-460`) | hand-edited |
| Statements across months, with closing balances | `FetchMonthlyStatementWithTransactions` per id per month; a conversion's paid side stated only there | `graphql/FetchMonthlyStatementWithTransactions.graphql`, the one recorded month `wealthsimple-statements/lira-2025-06.json`, `broker/src/statements.rs:1-20` | one month, one account |
| A conversion whose paid side the feed states one-sided | `FUNDS_CONVERSION` with `FetchInternalTransfer` detail, the receiving side on the statement | `generated.rs:718-753`, `FetchInternalTransfer-1.json` | the feed side only |
| A closed account with history | `status: closed`, its activity ending, its positions empty | `edited-accounts-one.json`, `resp_replies_with("closed")` | hand-edited |
| An account the broker no longer lists | an id in the book absent from the accounts list | `pull.rs` (`unlisted`) | none |

Each shape is generated from the recorded reply of its kind with its fields set, as every reply already is; where a reply of that kind is not recorded (statements beyond one month), the field names come from the GraphQL document in `graphql/` and the values are made up in the one recorded month's form. Nothing is captured from anyone's account.

**One test per shape over the stand-in,** in `rust/crates/wealthsimple/tests/pull.rs`, each holding the figure the shape must produce: the leaving leg booked once; one account per linked set, open while either id is; the late row booked once from its own row; the conversion's paid side booked from the statement and the month reconciling; the unlisted account's rows kept and said. The hand-edited fixtures those tests use today are replaced by the stand-in's shapes; the recorded replies stay as the templates they are.

**A before-and-after test for every migration that touches records or accounts,** in `rust/crates/book/tests/schema.rs`: a stand-in book with every shape on, built at the version before, opened at the version after, and its figures (each account's units per listing and cash per currency, from the engine) equal before and after, or each difference named in the test as the migration's intended effect. This is the comparison that would have failed 2.10.0; it is a test, never a command.

**The capacity job builds its reference books with every shape on,** so the budget is measured on a book with the feed's real shapes, not a smooth one (`rust/crates/server/src/capacity.rs`).

**The rule, in `docs/architecture.md` §16 beside the cost budget:** a change to how the book is built from the broker names the shapes it touches, and the stand-in states them; a shape the real feed turns out to have that the stand-in lacks is added to the stand-in first, from the recorded reply of its kind, and the change follows.

## 2. What it costs

About 400 lines in `generated.rs` (the shapes and their switches), about 300 lines of tests replacing about 200 of hand-edited fixtures, about 120 lines for the before-and-after test, and no new recorded page: every shape is set on a reply already recorded. No runtime cost: the stand-in exists only in tests and the capacity job.

## 3. Order

1. The shapes in the stand-in, with the per-shape tests, one PR. The capture set is complete when each row of the table has its test.
2. The before-and-after migration test, run from version 36 to the latest, one PR; it names what migrations 37 to 40 change.
3. The capacity job on the shaped books, one PR, with the table re-measured.

Nothing else changes in the app. A pull change after this is accepted only with the shape it touches named and tested.

## 4. Not included

- Any capture from a person's account, any step on a person's machine, any shipped command.
- The closed-account rule of 2026-10-21 (every account read the same way): it stays withdrawn at the owner's word of 2026-10-24; if it is ever reconsidered, the shapes here are what prove it.
- A trade orphaned by a record's removal re-attached on its restore (a schema change; noted at #610).
