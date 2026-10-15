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

## Correction, 2026-10-15: seventeen issues were left out

This brief read only the first page of GitHub's issue list. That page holds 100 items, pull requests among them, so the 85 counted were the issues on it. Seventeen more open issues from the same filing, #383 to #399, were left out. They include the only critical issue and five high-priority security issues.

They were re-checked against master `f89634de` the same way: the defect quoted at its current line, or the commit that removed it named, then the verdict argued against once. I checked #383, #386 and #387 in the code myself. All seventeen are still present. Most of the files they cite have no commits since the filing. The evidence is in [24/recheck-383-399.json](24/recheck-383-399.json).

| Issue | Labels | Title | Verdict |
|---|---|---|---|
| #383 | stage-5,critical | The page's write token is the same on every install, and any local process passes the peer check | Present |
| #384 | stage-5,high | The watched folder is accepted unchecked and imported in the same request that sets it | Present |
| #385 | stage-5,high | No Content-Security-Policy or framing headers, and filing documents are served at the app's own origin | Present |
| #386 | stage-2,high | The HTTP client re-sends every header, Authorization included, to whatever host a redirect names | Present |
| #387 | stage-5,high | The container runs as root, the sign-in browser runs without its sandbox, Xvfb runs all the time, and the base images are unpinned | Present |
| #388 | stage-5,high | The sign-in browser's profile is never deleted, and its DevTools endpoint is a TCP port | Present |
| #389 | stage-5,high | The headless sign-in says one sentence for every failure, gives up after a fixed three minutes and is driven on timers | Present |
| #390 | stage-2,high | Order-path failures are only written to the log | Present |
| #391 | stage-2,medium | A price edited from the Orders panel is rounded to the tick without a review | Present |
| #392 | stage-2,medium | A failed quote read is taken as the market being closed by the bracket | Present |
| #393 | stage-2,low | Orders placed in Wealthsimple's own app are forgotten when the app restarts | Present |
| #394 | stage-5,low | A stop or update cuts an order step in progress, and the supervisor neither forwards signals nor restarts a crash | Present |
| #395 | stage-2,high | Brokers are named in the server instead of described by the adapter, and orders still go through the old Wealthsimple client | Present |
| #396 | stage-2,medium | Orders, brackets and statements are keyed by the broker's strings, not the book's ids | Present |
| #397 | stage-2,high | The cutover is unfinished: the earlier market readers are keyed by symbol and the old store is still opened every start | Present |
| #398 | stage-2,medium | The server crate is one 38,000-line crate whose boundaries are held by text scans | Present |
| #399 | stage-2,medium | The journal is deleted and written again on every edit, with no history | Present |

Two corrections to the issue texts:
- #396: the book's statement tables were already keyed by account and instrument id when it was filed, so "statements" in its title does not match the code it cites. The orders and brackets part stands.
- #398 is worse than filed: the server has 49 modules and about 40,300 lines.

With these, the 102 issues filed on 2026-10-09 stand as follows:
- 100 present: 83 as of `d6824aa7` and these 17 as of `f89634de`;
- #412 fixed;
- #442 partly fixed.
