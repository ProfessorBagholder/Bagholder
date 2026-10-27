# Brief 79: Every amount as Wealthsimple states it

Line numbers are from master at 47e7649 (2.11.2). Read by five readers and two checkers over the code on 2026-10-27; what they could not settle from the code is in §5, and nothing there is guessed. This brief supersedes brief 77 §7's remaining open items (the coin units change) and the removal of brief 77 §4 sent the same day: both repaired the rebuild this brief removes.

## 0. Why

The owner, 2026-10-27: "it should be completely impossible for Bagholder to somehow have the wrong amounts vs WS when all it does is pull from WS." And: such header errors "simply don't exist, period, in any serious app."

The premise that failed is in `docs/architecture.md:93` and `:121`: transactions are Bagholder's own, derived from the records, and "the broker check compares Bagholder's cash and units with the broker's, exactly". So Bagholder rebuilt each account's cash and units by adding up the history, then compared the rebuild with what Wealthsimple states. Every disagreement line of 2.10.x came from the rebuild, and the statements walk, the units step, the gate and the coin units plan were each a repair to make the rebuild agree. The rebuild is removed; nothing is left to disagree.

What the code shows today:
- **Cash is already Wealthsimple's on every screen.** The Portfolio's cash, margin used, buying power and NAV read the stated figures (`engine/src/scope.rs:815-817`, `:1224-1290`). Bagholder's own cash (`Matched.cash`, `ledger.rs:2619-2650`) has exactly one reader, the broker check (`equity.rs:139`), plus one capacity test.
- **Units, book value and everything built on them are the rebuild's.** A holding's qty is the sum of the journal's open lots (`positions.rs:148-156`), its book the ledger's basis (`:170-177`). Market value, unrealized, allocation, the heatmap, exposure, the Cashflow holdings table, the order ticket's Sell quantity and Max (`web/src/lib/ticket/ticket.svelte.ts:119-152`), the "held" lists for news and filings, and the agents' positions tool all follow from them.
- **Wealthsimple's own figures are read and dropped.** Its book value per holding is stored (`book/src/statements.rs:63`, `:721`) and read by nothing outside a test. Its units reach `PositionFig.broker_qty`, which the page never receives (`wire/build.rs:228-268`). The balances read every 5 minutes (`FetchAccountsWithBalance`, `broker_reads.rs:35-36`) also lists current quantities of shares and contracts (`sec-s-…`, `sec-o-…`), skipped at `wealthsimple/src/read.rs:184`.
- **Past days are rebuilt too.** A past Portfolio's cash and net worth's past debts come from `CashByDay`, today's stated cash less every later transaction (`scope.rs:920-985`), and its holdings from matching the history to that day (`engine.rs:593-621`).

## 1. The rule

**Every amount for an account is Wealthsimple's, read by the pull, and none is computed from the history:** cash per currency, units per holding, book value per holding, account value, buying power, margin, and the same on a past day. Average cost is Wealthsimple's book value over Wealthsimple's units. Market value is Wealthsimple's units times Bagholder's quote, the quote being the only figure of Bagholder's own in it; unrealized is that less Wealthsimple's book value; allocation, exposure, the heatmap, net worth's holdings and projected income follow from those.

**The history is read for the journal only:** each trade, its round trip and result, the Cashflow rows, the notes. Nothing the journal works out is ever shown or used as an account's amount.

**The figures are the ones Wealthsimple shows the person.** Where Wealthsimple has more than one read of an amount (the balance types, `SPEC.md:403`; the ticket's `FetchTradingBalanceBuyingPower`), the one Bagholder shows is the one Wealthsimple's own web app shows for that account, chosen in the plan (§3).

## 2. What goes

All of it is code whose only purpose is the rebuild or its agreement:
1. **Bagholder's own cash:** `Matched.cash`, `cash_of`, `Continued.cash`, the engine's cash shares (`shares`, `ByFamily`, `total`, `add_shares` and the cash part of `match_all`/`rematch_families`).
2. **The broker check:** `equity.rs:98-271` (`Difference`, `BrokerCheck`, `broker_checks`, `Held`, `held`), `checks`, `recheck`, `Entity::BrokerCheck`, `Figures.checks`, `PositionFig.broker_qty`, and `Matched.last_price` where only the check reads it.
3. **The header lines that compare or reconcile:** cash disagrees, units disagree (`status.rs:201-223`), a statement month does not reconcile, feed movements no statement lists (`:415-454`), and `settled_conversions` (`:248-330`). No sentence of the app may say Bagholder's figures differ from Wealthsimple's. **Amended 2026-10-27:** the lines for rows the mapping cannot place or kept with a problem (`status.rs:235-404`) go too. The owner, of an ATM reimbursement row in a chequing account: "obviously you cannot account for every possible transaction that could happen in a checking account." Under §1 such a row cannot touch an amount, so it is never a header error; the plan says where, if anywhere already on the page, such a row is seen, adding nothing.
4. **The gate on a mapping version:** `gated_rederive`, `REFUSED_VERSIONS`, the retry after a pull (`figures.rs:459-473`, `:894-1027`, `broker_reads.rs:426`).
5. **The monthly statements, whole:** the read (`FetchMonthlyStatementWithTransactions`, the one request sent with the web page's own headers, `client.rs:44-65`), the walk and the units step (`broker/src/statements.rs`, 1,547 lines), `wealthsimple/src/statement.rs`, the statement-only helpers in the book, the pull's call (`pull.rs:366-380`), the stand-in's statement shapes and `READ_BY_2_10_9`, and their tests and data: about 2,770 lines of production code, 3,000 of tests and 0.68 MB of recordings (the D reader's list, in the PR). Their bookings served cash only, except the units step, which fed the journal a coin fill's units.
6. **`CashByDay`** and the rebuilt past holdings. A past day's units, book value and cash are the positions read as of that day (its `sec-c` lines are that day's cash, `read.rs:149-161`), read when first needed and kept.

**The units step is not kept.** The journal takes a fill's units as the activity feed states them, which is what Wealthsimple's own activity list shows for it. Whether the feed's coin units leave a round trip open where Wealthsimple states none held is not shown by any recorded form (the stand-in states a coin's units equal on both sides, `generated.rs:266-274`). The existing dust write-off (`ledger.rs:1523-1538`) stands; a case the owner meets is recorded first and its rule follows, never a guess.

**Records an earlier build booked from statements** (the current read wins): the first read under this change withdraws every live record booked from a statement, and every feed record a statement fill superseded is live again. No migration: the statement tables are left unread, and their removal waits for the next schema change.

## 3. The capture: withdrawn (2026-10-27)

Three bridge runs on the owner's Mac could not take it: a downloaded script was refused until the owner trusted their own repository, a session reading its own permissions was refused as a bypass, and reading the output of a script that used the owner's sign-in was refused as personal data, rightly. No session reads the owner's sign-in or what it returns. So the plan decides each point the capture was to record from the code and the recorded replies already in the repository, and names its basis:
- **units now:** the balances read's `sec-s`/`sec-o` entries (`read.rs:184`, read every 5 minutes; one edited recorded reply), or the positions read asked as of today; the plan chooses and says why;
- **book value:** its currency as the positions read is asked (`client.rs:305`);
- **the cash shown:** which balance type, from the recorded replies and `SPEC.md:403`;
- **past days:** the positions read as of a day (recorded for several days);
- **coins:** as the recorded feed and positions state them.

The check that the choice is right is the owner's own use of the release: every amount equal to what Wealthsimple shows them. A difference they report is a recorded form to add, never a header line.

## 4. Proof

- **A book whose history is wrong still shows Wealthsimple's amounts exactly.** Over the stand-in with every shape at twelve seeds and two todays, every amount on the page's data (the positions document, portfolio totals, net worth today and on a past day, the Cashflow holdings, the ticket's Sell quantity and Max, the agents' positions tool) equals the stand-in's stated figure. The same holds after the history is spoiled: a buy removed, a fill doubled, a conversion's side missing, a month of rows missing. Plant: a holding's qty from the journal's lots fails it.
- **No path from the history to an amount.** The holdings are built from what Wealthsimple states alone, so that a test can hold it: the function that builds them takes no records, lots or transactions. Its form is the plan's.
- **No disagreement sentence exists.** The header's text table, the golden routes and a scan of the server's strings hold that no sentence says Bagholder's figures differ from Wealthsimple's.
- **The journal stands.** The golden figures file's diff names every journal figure that moves (only the coin fills a statement superseded may, each named). Over every recorded form at twelve seeds, the journal leaves no "took more than the book held" line and no row unplaced or kept with a problem.
- **The release check** on a copy of the owner's book: every amount equal to Wealthsimple's, every comparing line gone, reported per line as identical, new or worse.

## 5. Open, settled by the plan from what is recorded, never by a guess

- Which read states current units (§3).
- Whether Wealthsimple's book value differs from the journal's cost for a deposit, a transfer in or a return of capital (`stage-3b-wealthsimple.md:65`). It does not matter to the rule: the holding shows Wealthsimple's, and the journal keeps its own for its trades.
- Stock dividends and coin fees charged in units: whether the feed states them (the checkers found the statement forms, no feed row). With units from Wealthsimple they no longer touch a holding; for the journal, the plan says what the recorded feed shows.

## 6. The order and the PRs

The agents build (#639) first, as the owner ordered; this takes the coin units change's place. Then the plan (§3 says what it decides in the capture's place) (CLAUDE.md: a refactor across crates is written first), naming every file removed and every test kept; one build PR doing §1 and §2 with §4's tests, released MINOR (what a user sees changes: the amounts are Wealthsimple's and the comparing lines are gone). `docs/architecture.md:93`, `:121` and `:146` and `SPEC.md`'s statements section say the rule in the same PR; `docs/decisions.md` records the owner's words.

## 7. Not included

The "took N more than the book held" line (`status.rs:224-234`), from the journal's reading, not an amount; §4 holds that no recorded form produces it; the journal's deposited lots and the opening-balance form; fees and execution days from statements (the journal reads neither today); dropping the statement tables; brief 75's PR 4 and PR 5; anything else on the page.
