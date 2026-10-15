# Brief 32: brief 30's fixes on #514 (`deep-history` at `5057ddc3`), and #518: #518 merges; #514 after five small changes

**Reviewed:** commits `2e12b22c` (brief 30's sixteen fixes) and `5057ddc3` (the brokers probe's account) against brief 30, and #518 (`live-rate-moves` at `d20ffd3f`) against brief 31. Three readers took parts A, B and C. Each ran the suites on a copy of the PR and undid each fix in turn, to see whether the test the PR names fails. They also tried to break what each fix left untested. I read #518 and ran its test with the fix undone. I checked every finding below against the code myself. The copies and build folders are deleted.

Suites on the PR code, all passing:
- `bagholder-book`;
- engine `continued`, `by_change` and `changes`;
- server `figures::` (16 of 16) and `capacity::tests` (8 of 8, the growth test among them).

## Verdict

**#518: merge.** The fix is right and its test holds it.

**#514: merge after five small changes.** All sixteen fixes are in. With each undone, its named test fails, except where noted below. No figure differs from a fresh build in anything tried. What remains:
- two tests that do not hold the code they are meant to;
- a probe that compares two different kinds of account;
- one trigger.

None of the five changes the design. The order #518, #511, #514 stands. After #518 merges, #514 merges master again; I ran that merge and it is clean.

## #518

`Change::Rates` and `Change::Clock` compare each account's live rate per stated currency before and after (`engine.rs:944–971`). Each account whose rate moved is reported as `Entity::Broker`.

The test covers three cases:
- the clock passing 16:30 with the day's rate unread: the total waits;
- the rate arriving: the total is 10,500;
- the same rates again: nothing moves.

With the two `live_rates_moved` calls removed, `a_live_rate_that_moves_reports_the_accounts_stated_in_that_currency` fails; I ran it. The view test's `same_as_fresh` compares every held document with a fresh build after the rate is read. The cost is one live-rate lookup per foreign currency per account, on a rates or clock change only.

Merged into `5057ddc3`, `engine.rs` merges without conflict. The rates handling then reads in order: the live rates before, #514's re-match of coin families, `derive`, `compare`, and the live rates moved.

## #514: the sixteen fixes

| Fix | Verdict | Fails with the fix undone |
|---|---|---|
| A1 an open takes no lock | Done | `opening_the_book_takes_no_lock` waits out the 30 s busy timeout |
| A2 one join fails its instrument alone | Done | the test's write step fails; its "the book opens" step no longer tests A2, since A1 stopped the open from settling |
| A3 `current_symbol` without the fold | Done | depth import grows 2.76× in steps (the author's figure, reproduced) |
| A4 `recursive_triggers` | Done | the property test fails at seed 2 without the new trigger |
| A5 oracle out of the window | Done | none possible: the query costs a constant amount |
| A6 property test | Done | across 40 seeds: 47 entries-only clears, 124 raw inserts, 31 joins on other pairs, 138 replaces on a plain connection |
| B1 the earliest leg | Done | `continued` and both `by_change` tests |
| B2 guard 1 revised or removed | Done | `continued` at the revision step and at the removal step |
| B3 `t3` | Done | the state oracle at `t3` |
| B4 assignment; adjustment naming another family | Partly | the assignment step fails; the adjustment step holds nothing (change 1) |
| C1 stale-anchor race | Done | the move path; the orphan guards are held in `book/tests/links.rs` |
| C2 a group's journal alone | Done | `changes.rs` |
| C3 the roll's other holding | Done | `continued` at `TradeOn("r1")` |
| C4 a window per probe | Done | a planted whole match on the small book passes the old check and fails the new |
| C5 payers by key | Done in the code; untested (change 2) | — |
| C6 a failed pull keeps what moved | Done | `a_pull_that_fails_after_its_catch_up_keeps_the_names_right` |

**The CTO's choices:**
- **A1, settling only after a migration: right.** A due row written outside the book survives the open, and `names()` folds while it is due. `current_symbol` stays right, and the next write settles it. Only `Book::open` migrates, so no path skips the settle after a migration.
- **A4, editing 029 in place: right.** No tag contains the commit that added 029 (`ed043d0a`); v2.5.7 ends at 028. `schema/v29.sql` matches the migration.
- **A3, 150 rows by depth and 20 by breadth: right.** At 150 rows about fifty groups commit two rows, so the depth probe now meets what is due. Nothing on the breadth path grows with the book.
- **C1:** the scenario is a real race, refused on the stale anchor.
- **C5:**
  - A payer's rate reads only that instrument's declared amounts, frequency, ex-dividend dates, matched holdings and payments, and today. Working out one payer leaves no other stale.
  - `paid` is kept in step on every path that changes cash rows.
- **`settle_successions_again`, unreadable joins:** as on master. The successions settle and `joined()` are byte-identical to master; an unrelated write beside an unreadable join fails the same way on both. It belongs with #516.

## The five changes

1. **Two lines on the path that replaced guard 4's dropped clause are held by no test.**
   - The lines: the seeds from an adjustment's legs (`engine.rs:675`) and guard 1's clause on an adjustment's legs (:1076).
   - The step meant for them adds no rows (`continued.rs:213`), so no family can continue there whatever the guards do. With line 675 removed, every engine test passes. With line 1076 removed, only `c1` fails, and on its label alone.
   - Both lines matter. Reproduced with S and T as two families: one change adds a dividend on S after S's last row, carrying an adjustment whose legs name only T.
     - Without line 675, S continues, S and T are never joined, and T's round trip differs from a fresh build.
     - Without line 1076, with a T buy in the same change, both continue and the families differ.
   - Required: labelled steps for both cases, each shown failing with its line removed. X and Y stay two families throughout and can carry them.
   - The step at :213 also joins S and T before `c1`, so `c1`'s label ("names an instrument of another family") is no longer true. Move that step after `c1`, or relabel `c1`.

2. **The `paid` upkeep is held by no test.** Skipping the removal loop in `engine.rs:1263–1276`, or the insertion loop, still passes `changes`, `continued` and both `by_change` tests. The generator writes no cash dividends, and `changes.rs`'s ledger change is a sale. Reproduced on `tests/fixtures/book.json`:
   - remove U's only dividend: with the removal skipped, U keeps a payer a fresh build does not have;
   - add a dividend on X: with the insertion skipped, X has none.

   Required: cash dividends added, moved to another day or instrument, and removed, in the `by_change` generator or in a `changes.rs` case. Correct the PR body's statement that both `by_change` tests hold the rates.

3. **The brokers probe compares a manual account with a broker's.** The probe takes the middle-sized account among those holding anything (`capacity.rs:660–677`). On the small book, after the add-trade probe has created the manual account (`entries.rs:140`), that is "manual". It then stores a Wealthsimple cash statement for an account the broker never states, which cannot happen in use: 152 allocations, against about 280 for a broker account. The large book picks a broker account, at 279. The brokers check therefore compares unlike accounts at 1.84× against the 2.0 limit. A rise of about 9% in a broker account's own cost would fail CI with no growth with the book. `5057ddc3` made the choice the same on every run, which is right; the measurements show the old failures came from which account was drawn, not from growth with the book. Required: choose among the accounts the connection's broker states, leaving out `manual`, so both books measure the same kind of account.

4. **`UPDATE OR REPLACE` outside the book leaves the kept runs stale.** On a connection without `recursive_triggers`, the row the collision deletes is never marked due. Reproduced: six sightings, one `UPDATE OR REPLACE` of a sighting's `record_id` onto another's key on a plain connection, then a write. The kept runs differ from the fold, and `names()` reads them, since nothing is due. No production code issues it, and it is not a regression, but brief 28 item 1 is "every change to a sighting leaves a due row". Required, in 029 in place (unreleased): a `BEFORE UPDATE OF record_id, leg, instrument_id` trigger marking the row at NEW's key whose rowid differs from OLD's. It was checked with the pragma on and off.

5. **A2's test:** its "the book opens" step passes with or without the fix, since an open at the current schema no longer settles. Drop the step, or open after a migration.

## Optional

- Records whose legs fall on different days are also held to the earliest leg (`engine.rs:985`), but the walk applies them leg by leg. Reproduced: such a record sends a family whole although continuing it would match a fresh build. Gating the check on the walk's same-day test fixes it.
- `current_symbol` could read through the index whether or not anything is due; it gives the same answer.
- The plan's line 71 still cites `identity.rs:939`; `record_sighting` is now at 863–869.
