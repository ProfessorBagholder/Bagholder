# Brief 24: the 85 open issues re-checked against master `d6824aa7` (2026-10-11)

**Asked (owner, 2026-10-11):** "there are 85 issues?? are you sure these are still valid."

**Method.** Every open issue (all 85 filed 2026-10-09 by Stage P from brief 15's findings, each "checked against master at 4d102be9") was read by one reader against the current code (`d6824aa7`, 19 commits later: Stage P's fixes, the Stage 3 statements build `93a0dca3` and its follow-ups, 2.5.5), who had to quote the defect at its current line or name the commit that removed it; a second reader then tried to overturn each verdict. No verdict was overturned. The verdicts with their evidence are in `24/recheck.json`.

| Verdict | Issues |
| --- | --- |
| Still present | 83 |
| Fixed | 1: #412 |
| Partly fixed | 1: #442 |

**#412, close.** "Book migrations are tested on the schema text only." The test the issue asks for exists: The issue's claim (only `schema.rs:36-42` loads one connection row; the backfills in migrations 002 and 003 are not exercised) does not hold against the current book tests. `rust/crates/book/tests/facts.rs:27-55` `fn migration_two_anchors_every_trade_on_its_transaction_and_instrument()` opens a book at version 1 (`migrations: &MIGRATIONS[..1]`), inserts a connection, account, instrument, source record, transaction an… It was there when the issue was filed; the filing was wrong, not the code.

**#442, narrow.** "Instants are stored as text that does not sort." The Stage 3 build (`93a0dca3`) moved every instant the broker check turns on to integers (`broker_reads.at_ns`, `activity_reads.at_ns`, the statements' `first_ns`/`last_ns`, read through indexes). What remains: rust/crates/book/src/text.rs:67-70 unchanged: "/// An instant as stored: RFC 3339 in UTC.\npub(crate) fn at(t: jiff::Timestamp) -> String {\n    t.to_string()\n}" — jiff 0.2.37's Display (timestamp.rs:2344-2347) uses DateTimePrinter with precision None, so the fraction is printed only as needed and varies in length. The comparison the issue named is gone: rust/crates/book/src/s… The issue stays open for the text form still written and the tables the build did not touch.

**#485, amend.** Its first claim stands (the rates rule is "held" by a substring scan for `FX_FALLBACK` and `1.35`, `engine/tests/no_guesses.rs:59-60`, which any other constant passes). Its second claim, that `web/e2e/dataflow.spec.ts` has no second-open test, was wrong when filed: the test is at lines 73–95 and asserts the 2026-09-27 decision's words. Strike the second claim.

**Two filing errors in 85** is the error rate of the 9 October filing on this re-read; nothing else moved. The Stage 3 build closed nothing on the list by itself: #417 and #418, its targets, were closed on 2026-10-11 and are not among the 85.
