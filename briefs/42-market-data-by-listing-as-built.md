# Brief 42: #525 as built (market data by listing, at `777161e7`): not yet

**Reviewed.** Two readers took the server and the page. Each ran the suites, planted faults in a throwaway copy and probed what the tests leave open. I checked every defect below in the code. I also found item 3 myself, against the SEC's live file.

**Suites.**
- **Rust:** 1,659 passed and 1 failed, and clippy is clean.
  - The failure is `test_a_rollback_that_cannot_put_the_previous_version_back_is_said_and_keeps_it`. It fails only because the review ran as root, which ignores a read-only folder; it is not this PR's.
  - `entries::tests::a_trade_by_hand_on_another_venue_is_never_the_holding_of_that_ticker` is flaky: it failed 7 of 30 runs on the PR's own code (item 1).
- **Page:** `npm run check` gives 0 errors, and Vitest passes 149 of 149.
  - **e2e:** 314 of 316 passed. `marketcards:630` cannot load `https://example.com/a` in the sandbox. `marketcards:655` timed out once and passes alone.

## Verdict

**Not yet.** The design holds where it was built:
- `ListingKey` cannot be made without a venue;
- CIRO's rows are kept one per listing;
- SEDAR+ settles only on an exact name, with the tie rule;
- no Canadian listing reaches the SEC by its ticker;
- the TMX retry on other venues' forms is gone;
- the currency fallback is gone;
- no route takes a bare symbol;
- told marks are carried.

Planting each of those fails a test. But seven defects break the design's rules, and several rule tests miss what they claim to hold.

## Your four points

1. **No currency stands in for a venue: accepted.** The holding waits, totals are partial with a left-out count (`scope.rs:299–315`), and nothing is silently zero. Items 1, 5, 6 and 7 below must be fixed first, because each makes a holding lose its venue or keep a wrong price. Then report from the owner's copy how many held instruments have no key, and why each.
2. **Add trade's Exchange: accepted** (brief 40, U14; SPEC what-you-enter-5), with the page defects in items 4 and 10.
3. **`/api/listings` answering `ok` with `unread`: accepted as a shape**, with item 4.
4. **CIRO one row per listing: holds.**

## Required changes

1. **A trade entered by hand on a held share erases its venue.**
   - **The cause:** `Book::name` gives a held instrument no venue (`book/src/person.rs:294`), and `Named::draft` records that as a sighting with no venue (`:156`). Names fold by day (`names.rs:109–119`), so that empty-venue name becomes the current one.
   - **The effect:** the listing key fails and `held_on_listing` no longer finds the holding, so its market data waits. The next hand entry on that listing becomes a second instrument. Reproduced on all 13 held shares in the fixture book; `ONE@XTSX` became "ONE names no venue" after one entry.
   - **Required:**
     - A record naming a held instrument restates its current symbol and venue, or writes no name.
     - Replace the random pick in the test with every held share and venue.
2. **An interlisted filer is never removed.** `settled_filers` (`server/src/filings.rs:139–161`) rewrites the other regulator's filer only while the TSX list still pairs the two listings.
   - **Probe:** the pair was dropped, then another company listed as X in the US. X@US still showed the old Canadian issuer's SEDAR+ documents, and X@XTSE kept the former US filer's.
   - **Required:**
     - At each settle, the `interlisted` rows are replaced by what the current list pairs, or by none.
     - A test where the list changes between settles.
3. **The SEC filter drops real NYSE and Nasdaq listings.** `US_EXCHANGES` keeps NASDAQ, NYSE and CBOE only (`market/src/edgar.rs:30`, :106).
   - **The evidence:** the SEC's live file of 2026-10-16 has 10 rows marked `TXSE` (the Texas exchange) and 168 with no exchange. Among them are Energy Transfer (ET), Dillard's (DDS), Sunoco (SUN) and PHD, a NYSE closed-end fund. Each gets "The SEC lists no filer for … on a US exchange." Master found all four.
   - **Why the filter adds nothing:** the file holds each ticker once (10,444 rows, none twice), so it adds no exactness, only losses.
   - **Required:**
     - Take the SEC's row for the ticker whatever its exchange column says.
     - Read the list again on the filer re-check's daily cadence. Today it is loaded once per process (`edgar.rs:122–135`), so the daily re-check only ever re-reads the list from start-up.
     - Test against a recorded copy of the real file.
4. **A directory that could not be read is dropped, and the one listing left is opened.**
   - **Server:** `symbol_search` reports success when any one directory answered and drops the others' errors (`market/src/search.rs:185–221`). `/api/listings` then sends `unread: ""`.
   - **Page:**
     - The address resolver opens a single listing whatever `unread` says (`listing.svelte.ts:195`). Offline, `#markets/listing:ENB` went straight to the TSX holding.
     - Add trade takes it (`ui.svelte.ts:544`).
     - The News box takes it and drops the failure (`News.svelte:224–226`).
     - Add trade says "Not listed" when the read failed (`Modals.svelte:88`).
     - Short interest drops a failed venue when another answered (`Shorts.svelte:77`).
   - **The rule broken:** a single answer is "the only listing" only when every directory answered.
   - **Required:**
     - `unread` names every directory that did not answer.
     - A listing is opened or taken alone only when `unread` is empty. Otherwise the rows found show beside the failure and none is taken.
     - "Not listed" appears only when every directory answered with none.
     - A test for each.
5. **A holding with no venue keeps the price the currency guess last fetched, as if it were live.**
   - **The cause:** migration 6 leaves `quotes` and `daily_closes` as they were. `prices_of` reads the last stored quote (`read_sources.rs:74`), and `mark_of` uses it as a live quote at any age (`engine/src/positions.rs:96`). Nothing quotes the holding again (`contract.rs:139`).
   - **Probe:** a quote stored a month earlier priced the holding as live. That price may also be another company's: the guess is why it was removed.
   - **Required:**
     - At start, remove the cached quotes and closes of every instrument that has no listing key. The cache can fetch again; that loses only time.
     - A test.
6. **Two functions decide an instrument's venue, and they disagree.**
   - `ListingKey::of` takes the venue code when there is one, and fails on a code it does not know (`core/src/listing.rs:215`).
   - The sources crate falls back to the venue's words (`venue.rs` `known_mic`, its own test at :215: `OTCM` with `NYSE` is XNYS).
   - So such a holding is quoted, while its chart, news, filings and short interest wait.
   - **Required:**
     - One function, used by both.
     - A test over generated pairs of code and words.
7. **OTC holdings lost their prices.**
   - **Before and after:** master quoted an OTC listing through its USD currency, and its tests asserted so (`venue.rs:293`, :302). Now an OTC code gives no key and no source asks. The chart says no record names the exchange (`feeds.rs:1716`), which is false.
   - **Required:**
     - Key the OTC market as its own venue, `OTC`, from the OTC codes ISO 10383 assigns (checked against the registry) and the codes Wealthsimple states.
     - Yahoo quotes it under its symbol. The SEC list's OTC rows give its filer, and FINRA its short interest.
     - The chart's sentence names the real reason.
8. **A failed read is retried at once, every time.**
   - **Interlisted list:** it is stamped only on success (`filings.rs:88–101`), so while it fails it is fetched again on every settle: 6 times in 6 refreshes.
   - **SEDAR+:** a Canadian listing whose filer never settled stays stale (`filings.rs:179`), so an unreachable SEDAR+ is asked again on every five-minute sweep.
   - **Required:**
     - A failed read is stamped with its time and asked again on a stated backoff, never on every pass.
     - Its failure is shown until a read succeeds.
     - A test.
9. **Rule tests that miss what they claim.** No test failed for any of these plants:
   - **Rule test 1** claims every reader with a per-listing store, but misses three plants:
     - the intraday bars keyed by symbol (`store/src/market.rs:311`, :331);
     - `bars_source` keyed by symbol (`history.rs` `bars_meta_key`);
     - `bars_miss` keyed by symbol (`miss_key`).
   - **Filer re-check:** a filer never re-checked, so the tie rule is kept for good (`filings.rs` `settle_own`).
   - **Migration 6:**
     - `subject` dropped;
     - `enrich_reads` dropped;
     - the fold's completeness order removed, in the SQL and in `carry_filings`.

     The test checks `summary` only (`sources/tests/cache.rs:337`), and its duplicate rows agree on both completeness fields.
   - **Count rule:** a venue-less import or entry matched to one of two same-symbol holdings (`entries.rs:124`).
   - **Interlisted cadence:** the list read on every settle.
   - **Told events:** what was told forgotten (`filings.rs:346`).
   - **Page:**
     - the watchlist keyed by bare ticker (`Watchlist.svelte:39`);
     - News's held mark by bare ticker (`News.svelte:118`);
     - a document opened by symbol (`Disclosures.svelte:123`). No test opens `/api/filings/doc` at all;
     - the waiting line removed (`Disclosures.svelte:61`);
     - the resolver's failure line removed (`App.svelte:549`).

   **Required:** a test that fails on each.
10. **On the page.**
    - **The stale prompt:** "Choose the exchange it trades on." stays after the choice is made, and after the symbol changes to one with a single listing, where the box is disabled (`ui.svelte.ts:527–545`). Clear it when either happens.
    - **The clipped box:** it cuts the chosen listing at every width. "NYSE · Toronto-Dominion Bank" needs 194 px and the box has 180 (`Modals.svelte:86–90`).
    - **The idle button:** the Disclosures waiting state shows an enabled Re-read that sends nothing (`Disclosures.svelte:149`; `discStore.svelte.ts:91`). Offer none there.
    - **The explanatory option:** "Written by its terms" is explanatory text in a disabled box. Leave it empty, as the box's other disabled states are.
11. **The page and the server spell a symbol by different rules.**
    - **The mismatch:** `listingKey.ts:28` and :31 treat U+FEFF as space, and `listing.rs:130` and :132 do not. So the page keys a listing the server answers none for.
    - **The vectors:** they hold the page to the server through 20 hand-picked stems (`tests_types.rs:594`).
    - **Required:** generate the vectors, Unicode spaces included.

## Smaller

- **The single-match case.** A single exact SEDAR+ match whose newest filing is outside the 485-day window is a miss too, for the same reason as the tie rule: a dormant profile is never a listed issuer's. The date can come from the documents read for that filer, with no extra request.
- **A stale comment.** `read_sources.rs:84–88` still says the sources follow the currency.
- **The plan's cost line.** It says one SEDAR+ search "per Canadian listing whose filer is not yet settled". The code re-checks every Canadian listing daily: one TSX directory or TMX request, plus two paced SEDAR+ requests. State that.
- **News tags with no venue.** The server can send a news tag with no venue (`wire/news.rs:351`), and the page then makes a chip with an empty key. Route such a tag through the resolver, and check it on a news row tagged with no venue.

## Holds

| Plant | Caught by |
|---|---|
| Daily bars by ticker | `listing_isolation` |
| Currency fallback | `payers`, `closes`, `tests_routes_golden` |
| TMX other-forms retry | `venue` |
| First of several exact matches, "contains", bound widened to forever, "exactly one" relaxed | `settle_tests`, `sedar_tsx60` |
| Canadian listing to the SEC by ticker | the filings routing test, the `edgar` test |
| Pairing by same ticker | the routing test, the `interlisted` parser test |
| Fold reordered wholesale | the migration test |
| Watermarks or told events not carried | `carry::tests` |
| A route taking a bare symbol | `tests_listing_rules` |
| Page: resolver, News and Add trade taking the first of several | `listingdata`, `signedin.listingorders`, `marketcards`, `menu` |
| Page: Add trade sending with none chosen | `menu` |
| Page: `BRK-B` against `BRK.B` | `listingKey.test.ts` |
| Page: Disclosures and Short interest by bare ticker | `listingdata`, `shorts.test.ts` |

#524, a fund of funds' breakdown, is found on master and stays separate.
