# Brief 07: verdicts on the stage 3b plan and the trade plan

**Reviewed:** `docs/plans/stage-3b-wealthsimple.md` and `docs/plans/trade-open-to-flat.md` at `c778be20`.

**How:** each finding was checked against the plan, the code and outside practice (`briefs/reference.md`). An independent reviewer then tried to refute each one. Findings that didn't survive were dropped; the rest were reworded where the reviewer showed a better fix.

## `trade-open-to-flat.md`: Go with one change

1. **A round trip that goes flat with no sale is not a scored trade.**
   - An unlinked transfer out takes lots and makes no slice, and today such a trip is skipped (`engine/src/trades.rs:268`).
   - Under "one trade per round trip, Closed when no lot is held", a position sent wholly out of the account would become a closed trade at 0, counted as breakeven in Win rate and Expectancy. `SPEC.md` §2 (Crypto) says a transfer out is not a sale and not a fill of the trade.
   - Say such a trip is not a trade, and add a case.

**Not blocking, for the `SPEC.md` edit:** Realized P&L will include open trades' partial sales, while its subtitle count and Expectancy use closed trades. Write each so it says what it covers. Also say what status a group has when one member is open.

## `stage-3b-wealthsimple.md`: Go with changes

Record in `docs/decisions.md`: calls to Wealthsimple's unofficial API are kept to what is necessary (owner, 2026-09-24).

**1. Read only what changed. Never re-read an account's history.** The plan re-reads every account's whole activity feed and whole daily history on every pull. That contradicts a recorded owner decision: "Load and compute only what changed, when it changed: no blind timers, no full reloads" (`docs/decisions.md`, 2026-09-20). A plan that contradicts a recorded decision does not pass the gate. The design:
- **Once:** the first pull reads each account's feed and history in full, to replace the imported records. This happens once, not on every pull.
- **Then only what changed.**
  - Activity dated after the account's last complete read, plus the rows still not final (pending, a placeholder dividend).
  - Daily values after the last stored day.
  - Nothing for a closed account with no new activity.
  - Legs, entitlements and positions read once per row and kept in its record.
- **A missed or revised row is found by the broker check, not by re-reading.** When an account's cash or units differ from Wealthsimple's statement in a way the book cannot explain, that account is re-read over the span the difference points to, and only then.
- **Acceptance criterion:** the requests of a pull with nothing new, and of a pull with one new trade, are counted on the real run and stated.
- **Guarded by a test, not by review.** On recorded replies, a pull with nothing new sends exactly the fixed minimum: the accounts list and one activity page per open account. A pull with one new trade sends only what that trade needs. Any other count fails, so re-reading history turns CI red, as `test_no_wait_on_a_clock_that_is_not_accounted_for` does for timers.

**Every decision that can be tested is guarded by a test.** Each line of `docs/decisions.md` names the test that fails if it is broken, or says "review only" where no test can hold it. Add the missing tests in this stage for the decisions 3b touches.


**2. The mapping reads each row's status.**
- A row not stated as executed (cancelled, expired, rejected, pending) moves nothing and raises no gap. A partly filled, then cancelled, order books its filled part.
- Today the table keys only on `type`/`subType`. A cancelled roll, whose legs' fills are null (question 1), would read as `leg-unstated` and put every contract on its underlying on hold (`engine/src/ledger.rs:926-952`).

**3. One bad row never stops an account's read.**
- A multi-leg row whose amount doesn't match its legs is kept as a record with a problem. The rest of the account is read, and `activity_read_at` still advances.
- Verification lists the shapes the sign rule was checked on: the row's currency against the legs', and any fee.

**4. Events and transfers read from positions: exact, and kept.**
- **Events:** the received security is the unique one whose units changed from the day before to the day after by exactly the stated units, net of the book's own transactions on those days.
- **Transfers:** each security moved is one whose fall in the source account equals its rise in the destination. A move can carry several securities.
- **Otherwise:** the event stays `event-unknown`, or the transfer unlinked, named.
- **Kept with the record:** the position replies are stored in the row's record. That's required, not just a saving: a mapping is pure over one record (`book/src/mapping.rs`), and `rederive` cannot reach the network.

**5. Wealthsimple's book value is not a transfer's cost** (question 7).
- `SPEC.md` §2 keeps a crypto transfer in as `basis-unknown`, and Wealthsimple states a crypto deposit's book value is the market value at deposit.
- An ATON transfer's book value can be a placeholder until the person sends a statement. Return-of-capital adjustments arrive retroactively.
- So the book value is kept as Wealthsimple's statement, for the broker check, and never taken as the cost. The cost is the person's opening balance.
