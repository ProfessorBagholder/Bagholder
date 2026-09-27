# Brief 11: verdict on the statement-gaps plan

**Reviewed:** `docs/plans/statement-gaps.md` at `8cc347d1` (PR #301). Checked against `master` at `8af32571`, `SPEC.md` §2, `docs/architecture.md` (the broker adapter; raw rows never rewritten) and `docs/decisions.md` (2026-09-24).

**Verdict: Go with changes.** Changes 1 to 7 are required. The direction is right:
- read the statement only for an account that disagrees;
- match each statement row to the feed;
- book only what the feed lacks;
- prove it by the closing balance.

This is bank reconciliation, as the plan says. The rule as written would raise false alarms, and it can book money twice. Findings are ranked, most severe first.

## Required changes

**1. The month-end check compares balances dated two different ways.**
- **Where:** plan lines 35 and 44. The code dates a feed row at execution (`wealthsimple/src/mapping.rs:259`, `:297`).
- **The cause:** the statement dates a trade at settlement: "the statement dates the sale 2025-06-26 and states `(executed at 2025-06-25)`". The book dates it at execution, in Toronto's zone.
- **Failing scenario:**
  - A sale executed on a month's last trading day settles in the next month.
  - The book's cash at the statement's last day includes the sale; the statement's closing balance does not.
  - The month "does not reconcile", and the header says so, though nothing is wrong.
  - The walk never stops at that month and reads on towards the account's first month.
- **Fix:** reconcile as bank reconciliation does.
  - A month reconciles when every statement row in it is matched or booked.
  - A book row the month's statement lacks counts as reconciled only if the next month's statement holds it: an outstanding item, not an error.
  - The balance comparison counts each book row on its matched statement row's date.

**2. Unmatched statement rows are booked before the month is proven.**
- **Where:** plan line 44, "Its unmatched rows still count".
- **Failing scenario:**
  - The feed has a movement but states it differently from the statement: net against gross with the tax on its own row, a day apart, or another kind (finding 3).
  - The statement row matches nothing, so it is booked, and the movement now counts twice.
  - The header then shows a difference, but the book is already wrong.
- **Fix:**
  - Walk back to find the base month, the newest month that reconciles with nothing unmatched. Then go forward from it.
  - A month's unmatched rows are booked only if the month reconciles with them.
  - If it doesn't, none of that month's rows are booked, nor any later month's, and the header names the month.
  - This keeps the plan's own Right to refuse ("booking on a weaker rule would put double-counting back"), which today applies only to the owner's first read, true for every user and every pull.

**3. "Same kind" makes a match depend on two code systems agreeing.**
- **Where:** plan line 42.
- **The cause:**
  - The feed maps `INTERNAL_TRANSFER` to `TransferIn` or `TransferOut` (`mapping.rs:271`, `:737`).
  - The statement code table maps `WD` to `Withdrawal` (`broker/src/csv.rs:435`).
  - The owner's own capture shows Wealthsimple coding a move between the person's own accounts as `WD`.
- **Failing scenario:** a move the feed does carry, stated `WD` on the statement, matches nothing, and with finding 2 unfixed it is booked twice.
- **Fix:** match on account, currency, exact signed amount and the day rule. Use the kind only to break a tie. With finding 2 in place, a mismatch that gets through still books nothing.

**4. A statement month is read once, never again.**
- **Where:** plan line 40 ("a re-read stores nothing twice") and line 45 ("A later disagreement reads again from where it starts"). Both assume re-reads.
- **Failing scenario:**
  - A disagreement the statements cannot explain persists: a movement in the current month, not yet in any statement, or a difference in units.
  - Every pull then reads at least the newest month again.
  - Where no month reconciles (finding 1), every pull reads back to the account's first month: dozens of requests per pull to an unofficial API.
  - That breaks the decision of 2026-09-24 and brief 07 (no full reloads).
- **Fix:**
  - A completed month's statement is read once and kept.
  - A walk resumes from the oldest month not yet read.
  - A difference the kept statements cannot explain is reported, and nothing more is read until a new month's statement is issued.
  - A statement not issued yet, as in the first days after a month ends, is its own named state: not a failure, and not an empty month. Its reply shape comes from a recorded reply.
- **Test:** two pulls in a row with a persistent difference. The second sends no statement request.

**5. One movement can come from three sources.**
- **The three:** the feed, the statement the sync reads, and a statement file the person imports.
- **Where:** `server/src/csv_import.rs:191-193`. `link` treats every record that isn't the person's or the file's as the broker's, so a sync statement row becomes a candidate too.
- **Failing scenario:**
  - The June 2025 LIRA statement file, which the owner has already downloaded, goes into the Watch folder.
  - If the file pass runs while a statement row for a fed movement is still unsuperseded, the file's row has two candidates. It is ambiguous, nothing is linked, and the movement counts twice.
  - The missing withdrawal is then held by both the sync's statement row and the file's row. It is booked twice unless the file row links to the statement row.
- **Fix:**
  - One precedence, applied in order: the feed, then the sync's statement, then the file.
  - One-to-one across all three.
- **Tests:**
  - a movement in all three sources is booked once;
  - a movement in the statement and the file only is booked once.

**6. The plan's own figures don't close.**
- **Where:** plan line 9. The LIRA holds $72,950 it no longer has; the chequing account is $72,948 short.
- **The gap:** open question 2 captured January's LIRA row, not the chequing account's January arrival.
- **Failing scenario:**
  - The two sides differ by $2 or land a day apart, so the transfer join (line 43, "same day and amount") doesn't pair them.
  - The acceptance criterion that both header sentences go (line 57) cannot pass.
- **Fix:**
  - Read the chequing account's January statement first and explain the $2 before the rule is fixed.
  - Allow no tolerance on the amount.
  - A pair that doesn't join stays two named, unlinked movements.

**7. No header-by-header experiments on the owner's session.**
- **Where:** plan line 33: "settled by the build's first read on the owner's session, one header at a time".
- **The cause:** the plan already holds a request that works: the page's own request, replayed. Probing sends deliberately refused requests to an unofficial API on the owner's account, to learn what is already known (brief 05).
- **Fix:** send the page's request as captured, with its operation name, hash and headers. A later refusal is a sync failure naming the request, as the plan says.

## Not blocking

- **Dates across sources.** Matching and the reconciliation use the feed's day in Toronto's zone (`mapping.rs:54`, `:259`) and the statement's own dates, never the viewer's zone. That is right today.
  - Say it in `SPEC.md`.
  - Add one test with the home zone set far from Toronto.
