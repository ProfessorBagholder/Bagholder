# Brief 50: #523 at `d14b71a1`: not yet. A market cache that cannot be opened still leaves the page with no header and no update button

## Verdict

**Not yet.** The server no longer stops. But in the case closest to the 2.5.6 crash loop, a store this build cannot open, the page is still left with no header and no update button. A failure is also still said after it is resolved, and said in sentences that are untrue. Seven changes, below. Merge #523 **after #525** (see *Merge order*).

#523 makes **no change to the wire.** `start_error` sits on `app::State` and is folded into the existing `error` string by `status::failures`. `wire.rs` and `wire.ts` are untouched. Change 1 below does change the wire. This brief is its gate, and passes it.

## What was checked, and how

I built `d14b71a1` in a worktree of its own and ran `tests/start_failures.rs` and the figures tests. Then I added one case: start the server on a market cache that cannot be opened, mend the file while the server runs, and read `/api/status`. I also printed the header's error line in each of the PR's five cases. The results:

| Case | What `/api/status` answers |
|---|---|
| a book this build cannot open | `the book could not be opened: file is not a database. The book could not be opened for the notifications: the book is not open yet.` |
| a market cache this build cannot open | **refused** (`ok: false`): `The market cache could not be opened: file is not a database`. The header, the version and the update fields are not sent |
| the cache's file a folder, then mended while running | after mending: `the market cache could not be opened: the market cache: disk I/O error. the market cache could not be opened: unable to open database file: … The book could not be opened: the book is not open yet. The book could not be opened for the notifications: …` |
| an earlier database that cannot be carried into a new book | `the earlier database could not be carried into the book: the book: not an error. The book could not be opened for the notifications: the book is not open yet.` |
| the Python app's database that cannot be carried | the same as the row above |
| an earlier store that cannot be carried beside a book | `the earlier store could not be opened: file is not a database.` (right) |

The page reads the header from `status` (`web/src/App.svelte`, line 202). When the status answer is refused, `status` is undefined: there is no version, no *Update available* and no error line. That is the state #516 exists to end.

**Planted reverses:**

| Change undone | Test that fails |
|---|---|
| the data-carry failure stops the server again (`return 1`) | `an_earlier_store_that_cannot_be_carried_beside_a_book_is_said_and_the_server_stays_up` |
| `ensure_built` never derives the records again | `records_that_cannot_be_derived_again_at_the_open_are_tried_again_by_the_pass` |
| `Figures::open` builds from records not derived again (the early `return Ok(f)` removed) | **none.** The planted record also breaks the build, so "nothing is built" holds either way |

## Required changes

1. **The header answers whatever the market cache does.** `status::answer` refuses the whole answer when the cache cannot be borrowed (`status.rs:513`), and the cache is used there only for `dataVersion` and `coreVersion`. Keep the header's part, `status(app)`, free of the cache. Type the two versions as absent when the cache cannot be read: `Option<String>` in `wire.rs`, `string | null` in `wire.ts`. The answer's `error` then carries the reason, said once (change 3). The page does not reload data on a null version, because a reload would only meet the same failure. This is a wire change, and this brief is its gate.
   `test_a_market_cache_that_will_not_open_is_said_by_the_pass_that_met_it_until_it_opens` changes: the status answer is no longer refused, and its error says why. Restate in the PR what the test now proves. Its rule stays: no empty answer and no default answer. A version that cannot be read is null, never made up.
   *Test:* in `start_failures.rs`, a cache that cannot be opened → `/api/status` is 200 with `ok: true`, the version, the update fields, and the error naming the cache. Add an e2e on the made-up book with its cache unreadable: the header shows the version and the error line.

2. **A failure stops being said once a read shows it resolved** (`docs/decisions.md`, 2026-10-09). `start_error` is written once and stands until the next start, even for steps the server tries again. Hold each failure with the step that met it:
   - **The market cache.** It is borrowed on every status read, so the next borrow that succeeds takes its failure away. This is what `tests_failures` already requires of the passes.
   - **`Figures::open`.** If it failed, the next pass tries it again, and `set_figures` takes the result. A book or cache mended while the server runs then opens without a restart. A failure the app recovers from by itself is logged, not shown (2026-10-09).
   - **Steps that run once per start** (the Python app's database, the earlier store's carry): their failures stand until the next start, as now.
   *Test:* the case I added (mend the cache while running; the next status read no longer names it and the figures open), and the same for a book file mended while running.

3. **One failure, one true sentence.**
   - A market cache that cannot be opened is said today as a book that could not be opened. `Figures::open` opens the cache too, so its failure lands under the book's name. Say it as the cache.
   - The readers that need the open book add "the book is not open yet" under their own names: the notifications (`feeds.rs:200`) and the book reader. While a start failure stands, they add nothing of their own.
   - One cache failure is said twice in two wordings.
   - Sentences start in lower case, because `status::sentence` ends a sentence but does not start one. Each sentence starts with a capital letter.
   *Test:* each `start_failures` case checks the whole error line, not a substring: exactly one sentence, naming the store that failed.

4. **"the book: not an error"** is what the person reads when the earlier database cannot be carried. It is a reason with nothing true in it: "not an error" is SQLite's text for success. Somewhere on the path through `legacy_import::import` (`copy_database`, then `Book::open_in`), the real error is lost. I have not found the line. The sentence names what is wrong with the file.
   *Test:* both carry cases in `start_failures.rs` check the reason (here, that the file is not a database).

5. **The guard that nothing is built from records not derived again is held by no test** (the third planted row). Plant a record that `rederive_all` refuses but the build would accept, so removing the early return fails the test.

6. **Hold the rule as a rule, not five cases.** The five tests name today's steps. A step added later is not covered. #525 adds one: `forget_unkeyed_prices`, inside `Figures::open`. Add a source test beside the other source scans in `tests_lints`: `serve()` has no exit between `App::new` and the bind, and every failure there goes through the one function that says it. With change 2, a failure of any step inside `Figures::open` is said and tried again by the pass, whichever step it is. `forget_unkeyed_prices` keeps its own sentence.
   *Test:* the source scan fails on a planted `return 1` after `App::new`.

7. **The verification `CLAUDE.md` asks of a change to running:**
   - `RUSTFLAGS="-D warnings" cargo test --workspace --locked`. The PR ran only `-p bagholder-server`.
   - The container image built and started on the made-up book with a store planted unreadable. The header and the update path are driven in it.
   - A states table for the header, with a screenshot of each start failure at 1340 px.
   - The cost line: no extra read on a good start, and no read on a timer while a failure stands.

## Merge order

#523 conflicts with #525 in `rust/crates/server/src/main.rs` and `figures.rs` (`git merge-tree` against `origin/market-data-by-listing`). #525 is ready and #523 is not, so the order stays: **#521, then #525, then #523.** When master is merged into #523:
- `main.rs`: #525's `return 1` on the cache goes. Its carry of the told marks to each listing's scope (`carry_told_by_listing`) stays under the branch where the cache opened.
- `figures.rs`: #525's `forget_unkeyed_prices` stays inside `Figures::open`, under change 2's rule. A failure there is said in its own words and tried again by the next pass, and nothing is built before it succeeds. Like `rederive_all`, it runs before anything is built, so a quote read under a guessed venue never stands as a price.
- `app.rs` and `status.rs` merge cleanly.

#523 does not overlap #521.
