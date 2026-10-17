# Brief 45: #525 at `750723ee`, after brief 44: not yet; three items are short, and seven new defects are found

**Reviewed:** the range `fc16347d..750723ee`. Two readers took the server and the page. Each planted the reverse of every required change and probed the new code. I checked the defects below in the code.

**Suites:**
- **Rust:** 1,683 passed. The one failure is the root-sandbox rollback test, which is not this PR's.
- **Clippy:** clean.
- **Page:** `npm run check` gives 0 errors, and Vitest passes 155.
- **Browser:** 331 of 333 passed in one full run. The two that did not:
  - the example.com popup, which this sandbox cannot load;
  - `menu.spec.ts:51`, a race in the test that is also on master. Line 64 checks the request count right after the click, without waiting for the request.

  No offline SEC failure reached the header.

## Verdict

**Not yet.** Most of brief 44 is in, and planting its reverse fails a test:
- an empty SEDAR+ answer keeps the stored documents;
- a hand entry on a held instrument writes no name;
- the fold's reference test checks every run;
- migration 30's refold is tested;
- the look-through rule;
- the SEC requests are typed;
- the order's venue is stated in Wealthsimple's words;
- OTC rows appear in search;
- the SEC list rests once after a failure;
- the page's single-listing rule, Short interest's failure, Add trade's single failure line and the "Not listed" timing;
- every page failure line and the SPEC rows.

A harder reference check found no history where the bounded re-form differs from a whole fold. It ran 120 seeds × 30 batches, with two-leg records, replacements, removals, CSV rows and a joined second instrument.

Three of brief 44's items are short (items 1–3 below), and the review found seven new defects (items 4–10).

## Required changes

1. **A back-dated record still grows with history** (brief 44, item 2).
   - **The cause:** migration 31's index misleads SQLite's planner. The book has no `sqlite_stat1`, and the app never runs ANALYZE. So both words lookups (`names.rs:676`, :648) are planned on `instrument_sightings_stating` instead of `instrument_sightings_venue_named`, and every sighting that states a code without words sits in that index. Hand entries and file rows that name an exchange are such sightings (`person.rs:169`).
   - **Measured** (698 → 2,792 sightings):
     - a back-dated record grew from 11,663 to 38,885 steps (3.33×);
     - a back-dated record with a venue-less record at the end grew from 11,662 to 38,884;
     - with that index dropped, the back-dated record is flat.
   - **Why capacity cannot see it:** its book is built from Wealthsimple rows, which state both code and words.
   - **Required:**
     - Make each lookup use its index whatever the planner guesses, by query shape or `INDEXED BY`.
     - Add a depth row whose deep listing has sightings that state a code without words.
2. **The capacity job hides growth it should report.**
   - **The hidden exemption:** `judge` drops allocations for both back-dated operations (`capacity.rs:940`), and neither §16 nor `OVER_BUDGET` says so. Without it, `test_every_operation_is_measured_and_the_growth_rule_judged` fails: "BackdatedTrade (Depth): allocations grew 2.29 times".
   - **The engine's growth:** a back-dated buy in the holding's own account re-derives every later trade: 32,394 → 117,282 steps, and allocations ×4.02. That growth is the engine's (`record_changed`), and it is identical on master `7e0782f1`.
   - **The weak row:** the `backdated-trade` row enters on the Manual account (`capacity.rs:775`), so it never touches the deep holding's trades.
   - **Required:**
     - Remove the exemption.
     - Measure a back-dated execution in the holding's own account.
     - List the engine's growth in `OVER_BUDGET` with its own issue, as §16 provides. The job then fails when the engine's growth is fixed and the entry is still listed.
3. **A file's row on a held instrument still writes today's ticker** (brief 44, item 3, the import half).
   - **The cause:** a row is matched to a held instrument by its current symbol or current listing only (`csv_import.rs:226`, `entries.rs:122`, `engine.rs:307`/:312). So "the file's own symbol" is always today's.
   - **Probe:** after a ticker change from PNG to ZQNEW on 11-19:
     - a row dated 11-03 under ZQNEW writes `ZQNEW 11-03` into the history;
     - a row under that day's own symbol, PNG, becomes a separate instrument.
   - **Required:** match a file's row by the name the instrument had on the row's day, and test it across a ticker change.
4. **The page path still says an offline refusal.**
   - **Where:** `filings_shown` (`filings.rs:1400`) turns the typed failure back into words.
   - **Probe:** offline, a page watching a US listing with unread SEC documents shows "…could not be read: offline: BAGHOLDER_OFFLINE is set" in the header within 1.2 s.
   - **Also untested:** the feed's `Failure::Offline` arm. A plant saying it is caught by no test, because the test at `feeds.rs:1964` passes words, not the type.
   - **Also on master, and part of "typed end to end":**
     - Offline, a Canadian filer's settle records a failed read ("SEDAR+ could not be asked who files …: could not be reached"), because `issuer_name` maps every fetch error into words (`disclosures.rs:185`).
     - Offline, SEDAR+ says "the browser helper is not installed" (`sedar.rs:124`).
   - **Required:** the refusal is kept as its type on all three paths, with a test on each.
5. **A late answer in the News box overrides what the person did since.** `newsSym` calls `resolveTicker(key, () => true)` (`News.svelte:255`).
   - Click a no-venue row (answer delayed), then click another chip: the chip changes back by itself.
   - Type a filter word meanwhile: when the answer lands, the word is wiped.
   - **Required:** drop an answer when anything changed since it was asked, as the ticket's request counter does (brief 37, item 4). Add a test.
6. **A no-venue row that no directory lists does nothing.**
   - **What happens:** no chip, an empty box, no message, and the list is asked with an empty query (`News.svelte:224`, :250–256). At `fc16347d` the click typed the ticker in the box.
   - **Required:** say there is no listing, as markets-60 now says where every directory answered.
7. **The listing choice a no-venue row offers cannot be dismissed.**
   - **What happens:** Escape, a scope button or a tab leaves it in place, because choices clear only when the query changes (`News.svelte:233–239`).
   - **Required:** clear it on those, with a test.
8. **Add trade's "No listing of ZQZA is found on an exchange." stays while the box reads ZQZB, until the box is left.**
   - **The cause:** it clears on `onchange` (`Modals.svelte:85`, `ui.svelte.ts:545`), while the line under Exchange follows the text as typed (`Modals.svelte:93`). The test presses Tab, so it misses this.
   - **Required:** clear it as the text changes.
9. **SPEC markets-84 is stale.** It still says "the first listing it names takes the chip" for the Disclosures tab's box. The code now offers each listing, which is right. Change the row.
10. **The directories' full stop is held by no test.** Removing `sentence()` at `search.rs:224` passes the whole market crate; only TMX's sentence is asserted (:311). Add the assertion.

## Smaller

- **The SEC list's wall-time age** is held by no test. The tests call `ticker_map_at` directly, so feeding `ticker_map` a clock that stops while the machine sleeps passes. Test it through `ticker_map`.

## Found here, also on master (separate issues)

- **The News chip is clipped at 1340 px with Holdings and Stories on:** scrollWidth 133 against clientWidth 67, so only "Symbol" shows. Same markup and CSS as master.
- **Short interest's failure with no rows is shown in the grey `.muted` line,** not in `.status-err` (`Shorts.svelte:120`, :167).
- **The race at `menu.spec.ts:64`,** above.
