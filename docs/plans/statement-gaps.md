# Plan: the movements Wealthsimple's activity feed leaves out, read from its statement transactions

## For the owner to decide

Nothing open. (Whether to do this at all was the owner's call, 2026-09-27: "let's see what you propose".)

## Scope

Wealthsimple's activity feed, the one source the sync reads a record from, leaves some cash movements out. Found on the owner's book: two withdrawals from a locked-in account (a LIRA) to a chequing account, $35,650.00 on 2025-06-26 and $37,300.00 around 2026-01-12. The feed has the sale before each and the withholding tax, but no row for the money leaving, and no row for it arriving. Wealthsimple's own activity screen doesn't show them either. Its monthly statement does: `WD Withdrawal −35,650.00` in the LIRA, `Transfer in $35,650.00` in the chequing account. So the book holds $72,950 the LIRA no longer has, and the chequing account is $72,948 short. The header says so, correctly, in two sentences.

This plan reads each account's statement transactions, which Wealthsimple serves as data (not a document), and books only what the feed left out. Proof is by reconciliation: each statement row is matched to the feed's row for it, and each month's closing balance must agree with the book.

It also closes a hole the work exposed. Importing a statement file (Scan now, Watch folder) links only trade fills to the broker's rows. That file's cash rows (deposits, withdrawals, dividends, tax) are booked a second time beside the synced ones.

Out of scope: statement rows that are trades. Their units and price are the feed's to state. A trade row only the statement has is reported, not booked. Credit-card statements are also out, because the card is not in the book.

## The old app here

It read the same activity feed and had the same gap: neither withdrawal is in its database (checked, 2026-09-27). It never read statements. Nothing is carried over.

`docs/old-app-mistakes.md` gains nothing: this is a hole in the source, and the old app missed it too.

## How the leading products do it

- **Bank reconciliation** (the accounting practice every ledger product follows) matches each line of the institution's statement to one entry in the ledger. Unmatched statement lines are the entries missing from the ledger, and the reconciled closing balance must equal the statement's. [Sage Intacct, "Troubleshoot reconciliation"](https://www.intacct.com/ia/docs/en_US/help_action/Cash_Management/Reconcile/Troubleshooting/top-solutions-bank-rec.htm), read 2026-09-27. This plan is that procedure, run by the sync.
- **Sharesight**, which keeps a synced broker feed beside imported files, names duplicates as the common failure of importing next to a sync: a trade imported that the sync already brought. It leaves removing them to the person. [Sharesight, "My portfolio doesn't match my broker"](https://www.sharesight.com/partners/my-portfolio-doesnt-match-my-broker-what-to-do/) and ["Common errors when bulk importing trades"](https://help.sharesight.com/common-errors-when-bulk-importing-trades/), read 2026-09-27. This plan links the duplicate instead of booking it, and reports any row it cannot link to exactly one.
- **Wealthsimple** builds its own statement CSV from `FetchMonthlyStatementWithTransactions` (its web app, release read 2026-09-27: the Documents page's Download CSV calls it with the account's id, the statement's period, and `statementType` `cash_monthly_statement` or `brokerage_monthly_statement`). The rows carry a type code (`SELL`, `WHTFED`, `WD`, …), a description, a signed cash movement and the running balance. Captured by the owner on the LIRA, June 2025.

## Open questions

1. **The request's values.** Objective: ask exactly what Wealthsimple's page asks. What is known: the query text (captured, identical to ours), `statementType` (the page's code), and that the account id is the account's own. Two direct requests with `period` `2025-06-01` were refused (`UNPROCESSABLE_ENTITY`). What settles it: the Variables of the owner's captured request, one paste. Guessing formats again is not the best course.
2. **The chequing account's row for an arriving withdrawal.** Objective: book the arriving side from Wealthsimple's own row, not infer it. What is known: its statement reads `Transfer in` for $35,650.00 on 2025-06-26. What settles it: the first read of that account's June 2025 statement, printed before anything is booked (a step of the build, below).
3. **How statement dates relate to feed dates.** Objective: match every row one-to-one. What is known: the statement dates the sale 2025-06-26 and states `(executed at 2025-06-25)`, the feed's day for it. What settles it: the build's first step reconciles every statement row of every month of the owner's accounts and lists any row that matched none or several. The rule below is fixed only once that list holds nothing but the known gaps.

## Approach

- **The adapter.** `BrokerAdapter` (`rust/crates/broker/src/lib.rs`) gains `statement(account, month) -> Answer<Vec<StatementRow>>`. The Wealthsimple adapter answers it with `FetchMonthlyStatementWithTransactions` (new `rust/crates/wealthsimple/graphql/FetchMonthlyStatementWithTransactions.graphql`, Wealthsimple's own document, as the others are). A row is read strictly: a type code the mapping does not know is a problem naming it, never guessed.
- **Stored as the broker's raw rows.** Each statement row is a record of its own under a new source, `wealthsimple-statement`, kept as it came (raw rows never rewritten). Its key is the account, month, and the row's position among identical rows, so a re-read stores nothing twice. The mapping turns `WD`, `DEP`, transfers in and out, `WHTFED`/`WHTPROV`, interest, fees and dividends into cash transactions. Codes beyond `SELL`, `WHTFED` and `WD` are taken from the reads, not assumed. Trades become no transaction, only the reconciliation's match.
- **Linking, one mechanism for both statements and imported files.** The linking pass that supersedes an imported file's fill with the broker's row (`server/src/csv_import.rs`, `link`, and `Book::supersede`) is widened to cash movements: same account, same kind, exact signed cash amount, and the same day, or the day the row states it was executed. One broker row takes at most one other row's place. A statement row with exactly one match gives way to the feed's row. With several, it is linked to none, and the header names it. With none, it counts: it is the missing movement. The same widening fixes the statement-file import.
- **The two sides of a withdrawal between the person's own accounts** are joined as a transfer by the existing `transfer_links`: the LIRA's `WD` and the chequing account's transfer in, same day and amount. It is one move of money, not income or a withdrawal from the person's wealth.
- **Reconciliation per month.** Each statement row states the running balance. The book's cash for that account and currency at the end of the statement's last day must equal the last row's balance, to the cent, after linking. A month that does not reconcile is a broker disagreement in the header, with the account, month and both balances (`status.rs` `broker_failures`, a new `Difference`). Its unmatched rows still count: a failure is shown, never hidden by leaving money out.
- **What is read, and when** (the owner's decision of 2026-09-24: calls kept to what is necessary; an account is re-read only over the span a broker-check difference points to). Statements are read for an account only while its cash disagrees with the broker's. Reading starts at the newest completed month and goes back one month at a time until a month reconciles with no unmatched row, or the account's first month. A later disagreement reads again from where it starts. An account that agrees costs nothing. On the owner's book today that is the LIRA and the chequing account, not all fourteen.
- **`SPEC.md`**: §2's paragraph on where the record comes from gains the statement as the source of movements the feed leaves out, with the linking and reconciliation rules; the header paragraph gains the month that does not reconcile. `docs/architecture.md`'s broker-adapter section gains the method.
- **Stays the same**: the feed stays the source of every row it has. No figure's definition changes. Nothing on screen changes except which header sentences stand.

## Acceptance criteria

- [ ] `cargo test --workspace` green, no warnings; clippy clean.
- [ ] Adapter tests on recorded replies (the owner's June 2025 LIRA capture, anonymised by `ws-anonymise`): rows read exactly; an unknown type code is a problem naming it; a refused request is a failure naming the account and month.
- [ ] Linking tests: a statement row with exactly one feed row gives way to it; with two, neither is linked and the header names it; with none, it counts. The same three for an imported statement file's cash rows, so a statement file never double-counts a deposit.
- [ ] Reconciliation tests: a month whose closing balance agrees says nothing; one that differs is a header sentence with account, month and both balances.
- [ ] Reading tests (request counted, as the pull's are): an account that agrees reads no statement; one that disagrees reads back month by month and stops at the first month that reconciles.
- [ ] **On a copy of the owner's book**, one pull with the statements read: every statement row read matches exactly one feed row, except the two LIRA withdrawals and their two arrivals. That list is printed and checked by hand before anything else. Then the LIRA and chequing sentences are gone from the header, and every other account's cash, units, trades and figures are unchanged to the cent (compared with `bagholder compare-figures`).
- [ ] A browser test: a statement-only withdrawal between two accounts on the made-up book leaves no header sentence, and both accounts' cash agree.

## Surfaces to check beyond the diff

`rust/crates/broker/src/pull.rs` (when the method is called, request count); `rust/crates/wealthsimple/src/replay.rs` (captures for tests); a book migration if the new source needs a table (records and links should not); `SPEC.md` §2 and the header paragraph; `docs/architecture.md` (the adapter contract); `web/src/lib/generated/wire.ts` if the header's disagreement type is on the wire.

## Right to refuse

If the owner's first full read shows statement rows that cannot be matched one-to-one by any rule stated here (dates that drift without an executed-at date, amounts split differently), building stops and the list goes to the owner. Booking on a weaker rule would put double-counting back.

## Anti-stub self-check

To be initialled at the end: no definition nobody references; no field written and never read; the owner-copy pull actually run and its list read.

## Verification

To be filled when built.

## Handoff

Blocked on open question 1: the Variables of the owner's captured `FetchMonthlyStatementWithTransactions` request.
