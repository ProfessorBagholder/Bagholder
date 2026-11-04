# Plan: Distribution history shows the distributions owed and not paid yet, a holding sold since its ex-date included

## For the owner to decide

Nothing open: the owner's ask is in `docs/decisions.md` (2026-11-04).

## Scope

**In it:**
- Distribution history (SPEC `cashflow-37`) lists, above the payments received, each distribution owed and not yet paid: one row per account and declared distribution where
  - the payer's declared record states the distribution (its ex-date, pay date and cash a unit);
  - its ex-date is on or before today;
  - the account held the units going into the ex-date: Wealthsimple's positions as of the day before the ex-date (`days_held`, brief 79 §2.4) state a positive long quantity of the instrument. The calendar day before is enough: positions as of a day the market is closed are the last close's, so no venue calendar is needed;
  - no payment of that instrument has reached that account on or after the ex-date and before the next ex-date (the rule `cashflow-26` and `found_form` already use for "paid");
  - it is still within its time: an owed row stands from its ex-date until the payment posts, or until the posting allowance `found_form` already uses has passed (two business days after the pay date, `posted_by`); after that it is gone and the record's own judgement (paid in units, or not known) stands as today. A distribution with no pay date stands until the next ex-date. Without the bound, every past distribution in a record that spans years, whose payment the book does not hold (an account whose history starts later), would read "not paid yet" for ever and ask for the day before each of their ex-dates.
- The row: the pay date, the symbol, the account, the units held going into the ex-date, the cash a unit, and the amount (units × cash a unit, in the distribution's currency), marked as not paid yet. It goes when the payment arrives, and the received row takes its place.
- It holds whether the account still holds the instrument or not: the units were held into the ex-date, so the distribution is owed.
- SPEC `cashflow-37` to `-39`, a new id for the owed row in `docs/spec-ids.md`; the agents' Cashflow read carries the rows as the page shows them.

**Out of it:**
- Units bought on or after the ex-date, and units sold before it: not owed, never shown (the owner, 2026-11-04: "only what's actually going to arrive").
- A distribution whose day's positions have not been read: the row waits on that read (asked for as a past day's holdings are), never shown from the journal's lots.
- A short position held into an ex-date owes a payment in lieu: a debit of its own, not a distribution received; not in this change.
- The tiles (YTD, All time, Last 12 months): they stay sums received.

## What happens now

Read from the code, 2026-11-04, at 016fd46d:
- **Cashflow Positions** (SPEC `cashflow-15`) is one row per open long position. A holding sold after its ex-date has no row, so its Ex-Div and Pay Day go with it.
- **Distribution history** (`cashflow-37`) is the payments received, newest first; nothing is listed before the payment posts.
- **The engine already holds** each payer's declared record (`inputs.facts.declared`: ex-date, record date, pay date, cash a unit, form) for every instrument with one, held or not (`cashflow::payer_rates` walks paid and declared instruments alike), and the "paid since the ex-date" rule (`found_form`).
- `found_form`'s `held_on` reads the journal's units on the day before the ex-date (`matched.units_on`). Units are Wealthsimple's as stated for the day (brief 79), so the owed rows read `days_held`, not the journal.

## How others do it

Read 2026-11-04.
- **Interactive Brokers** shows "Open Dividend Accruals": dividends pending for positions held into the ex-date, with "the quantity held prior to ex date", reversed "due to payout in cash" when paid (IBKR, Reporting Reference "Change in Dividend Accruals", and its Knowledge Base "Dividend Accruals"). A credit if long on the record date, a debit if short.
- **Entitlement under T+1** (TMX trading notices, March and May 2024): since May 27, 2024 the ex-date is the record date, so units bought on or after the ex-date carry no right to the distribution, and units held at the close of the business day before it do.

## Approach

1. **One definition of "held into the ex-date":** `cashflow::held_into(inputs, account, instrument, ex_date) -> Result<Dec, Gap>`, the units Wealthsimple's positions state for the account as of the day before the ex-date, a gap naming that day where it is not read. Its two callers: `found_form` (the Units judgement, which then waits on an unread day exactly as an owed row does, in place of the journal's `units_on`) and `owed`. The needs name the day once.
2. **Engine:** `cashflow::owed(inputs, today) -> Vec<Owed>`: for each instrument with a declared record and each account, each cash distribution within its time (above) whose `held_into` is positive and whose payment has not reached the account: owed.
3. **Wire:** the Cashflow document's history gains the owed rows, ahead of the received ones, each with `owed: true`, by its stable id (instrument, account, ex-date), so the page patches it in place and drops it when the payment row arrives.
4. **Page:** the owed rows at the top of Distribution history, the pay date in the muted style the Positions table gives a date still to come, and the row marked "not paid yet" in the app's own words; widths as now.
5. **Agents:** the Cashflow read answers the same rows (the page's own route).

## Acceptance criteria

- [ ] **Owed only where held into the ex-date**, on generated books: units held the business day before the ex-date and sold on or after it: owed; sold the day before: not owed; bought on the ex-date: not owed; held in two accounts: one row each. Engine cases written from the SPEC text by an agent that has not read the engine.
- [ ] **Paid:** a payment of the instrument in the account on or after the ex-date (before the next) removes the row. Plant: the paid rule removed keeps it.
- [ ] **Bounded:** a pay date two weeks past with nothing posted: no row; within two business days of the pay date: the row; no pay date: the row until the next ex-date. Plant: the bound removed lists the old distribution.
- [ ] **One definition:** `found_form` and `owed` both call `held_into`; `found_form`'s Units judgement waits on an unread day. A case where the journal's units differ from Wealthsimple's on the day before the ex-date: both follow Wealthsimple's.
- [ ] **The day's positions:** with the day before the ex-date not read, no row and that day in the needs, named once. Plant: the journal's units used fails the case above.
- [ ] **The page:** a browser test: a holding sold after its ex-date shows its owed row at the top of Distribution history; the payment's arrival replaces it with the received row, one element updated.
- [ ] SPEC lines and spec-ids; the agents' Cashflow read holds the rows.
