# Brief 44: #525 at `fc16347d`, after briefs 42 and 43: not yet, the remaining changes are listed below

**Reviewed:** the range `2616ea3d..fc16347d`. Two readers took the server and the page. Each planted the reverse of every required change, and probed the new code. I checked the defects below in the code.

I checked the venue codes against the ISO 10383 registry, downloaded 2026-10-17:
- `XCHI` is "NYSE TEXAS, INC." and `TXSE` is "TEXAS STOCK EXCHANGE", both active;
- the 16 OTC codes are OTC Markets' tiers (operating MIC `OTCM`), OTC Link's (`OTCN`, `OTCI`, `OTCO`), FINRA's "OTHER OTC" (`OOTC`), and the expired `XOTC`, `PINX`, `PINC` and `PINI`.

**Suites:**
- **Rust:** 1,676 passed. The one failure is the root-sandbox rollback test, which is not this PR's.
- **Clippy:** clean.
- **Page:** `npm run check` 0 errors; Vitest 154 passed.
- **Browser suite:** 317 of 327 passed, and the same 10 failed on two full runs (item 6). One of the 10 is the sandbox's example.com popup.

## Verdict

**Not yet.** Every required change of briefs 42 and 43 is in, and planting its reverse fails a test. That covers:
- interlisted filers following the list;
- the SEC list read whole and daily;
- unread directories named;
- the prices of unkeyed shares forgotten;
- one venue rule;
- OTC as a venue;
- failed reads resting;
- the space rule, now 7,686 generated vectors;
- every earlier uncaught plant;
- the fold;
- fills stating their venue.

A reference check against brief 43's rule, on every day of 60 generated histories, matches the fold.

What remains is below: four defects, one cost regression, and gaps in tests.

## Required changes

1. **An empty SEDAR+ answer hides real filings.** The dormant check (`server/src/filings.rs:1211`) runs before the "answered empty, the stored rows stand" branch (:1220).
   - **The effect:** a one-off empty answer for a profile with recent stored documents records a miss ("…has filed nothing in the last 485 days") and hides those documents until the next day's check.
   - **Probe:** `r525-probes/dormant-empty-answer-probe.diff`.
   - **Required:** judge dormancy from the newest of the stored and the answered documents; an empty answer leaves the stored rows standing. Add a test.
2. **A back-dated change costs more as the instrument's history grows.**
   - **The cause:** `names.rs:454–461` refolds the whole instrument whenever any later sighting states no venue. The `seek` at :402 filters on that with no index covering it.
   - **Measured:** history ×4 took a back-dated broker record from 8,698 to 27,544 SQLite steps, and a back-dated hand entry from 22,477 to 83,203. `2616ea3d` was flat on the same probe. This breaks the growth rule. The capacity depth rows cannot show it, because they add only today's trades.
   - **Required:**
     - Re-form from the change to the next sighting that states a venue. The carried venue stops there, so the work is bounded index reads.
     - Index the venue-less sightings.
     - Add depth rows with a back-dated broker record and a back-dated hand entry.
3. **A record naming a held instrument still restates its symbol.** A hand entry or import on a held instrument writes today's ticker onto its own, possibly earlier, day (`person.rs:289`; this was on master too). A later ticker change then shows false names in the history (probe: ZQV, ZQW on 03-05, ZQV…).
   - **Required:** such a record writes no name, because it names the instrument by its reference. This also removes most of the venue-less sightings behind item 2.
4. **The fold's rule test checks only the newest name.** A planted fold that gives every venue-less record the newest venue passed `book/tests/venue.rs`.
   - **Required:** assert every run of every generated history against a reference fold.
   - **Migration 30's refold is also untested:** removing its `INSERT INTO names_due … FROM instruments` (`030-the-venue-an-order-names.sql:12`) left the book suite green. Add a test that migrates a stored past-schema book with venue-less sightings and asserts its names after.
5. **A look-through holding is resolved alone when a directory did not answer.** `exposure::resolve_name` ignores `unread`, and no test holds it.
   - **Required:** the same rule as the resolver (one listing alone only when every directory answered), with a test.
6. **An offline refusal from the SEC reader is said as a failure.**
   - **The cause:** `edgar.rs:66` wraps the transport error as `<urlopen error …>`, so `is_offline_refusal` (`net/src/client.rs:561`, a match on the text's ending) misses it.
   - **The effect:** in a full browser run here, a demo SEC document's offline refusal reaches the header and stays. It breaks 9 later tests, the same ones on both runs, and each passes alone: `menu.spec.ts:39`, :112; `notifications.spec.ts:51`; `states.spec.ts:57`; `ticket.spec.ts:281`, :399, :412, :433, :449.
   - **Required:**
     - Recognize a refusal by its type, not by its words.
     - Keep the transport's own words in the SEC reader.
     - The browser suite passes in one full run.
7. **On the page.**
   - **Short interest hides a failure when a feed row matches the typed prefix** (`markets/Shorts.svelte:125–128`, where `lookupError` needs `f.rows`). Repro: feed row RZZT, typed RZ, the TSX read fails: the table shows and the failure is never said. Show it, with a test.
   - **With a News chip on, a tagged row that names no venue asks nothing** (`News.svelte:220`, :236–239). Clicking it puts the ticker in the box and sends no `/api/listings`. Route it through the resolver, with a test.
   - **Add trade says the same failure twice**, under Exchange and at the foot (`Modals.svelte:93`, :109; `ui.svelte.ts:315`). Say it once.
   - **A form error stays after the symbol changes** (`ui.svelte.ts:541`; also on master). "No listing of ZQZA is found on an exchange." remains after typing ZQZB. Clear the symbol's errors when it changes.
   - **The unread sentences run together.** They are joined with spaces, and the server's sentences have no full stop: "…is set The TSX's directory…". End each with a full stop, or show each on its own line.
8. **Page tests that hold less than they claim.**
   - **"Not listed":** `modals.spec.ts:96` checks the option before the answer lands, so dropping `&& !f.listingsError` passes. Wait for the answer first.
   - **No test at all** holds:
     - the News failure line when nothing is found;
     - the watchlist's failure line;
     - the filter picker's failure line.
9. **SPEC.** what-you-enter-5 (`SPEC.md:115`) and markets-60 (:686) still say one listing is taken. Add: only when every directory answered; otherwise the rows found are shown with why the others could not be read.

## Smaller, in the same PR

- **Venue codes on orders.** `orders/ticket.rs:907` normalizes the venue code (`XTSV` becomes `XTSX`), unlike the broker's rows, so names split into XTSV / XTSX / XTSV runs. Store the venue as the order's listing states it. Add a test that fails when the ticket writes no venue.
- **OTC in search.** `market/src/search.rs:19–22` and :50 still drop Nasdaq's OTC rows, so an OTC listing nobody holds cannot be found. With OTC a venue, list them with their venue.
- **The SEC list's age.** It is judged by a clock that stops while the machine sleeps (`edgar.rs:132`). Use wall time.
- **A failed SEC list read.** Each listing downloads it again on its own schedule. Rest the failure once for all of them.

## Holds (planted, caught)

| What | Caught by |
|---|---|
| Hand entry and import | `entries` test, `venue.rs` |
| Interlisted filers dropped with the list | `filings.rs:1874` |
| The SEC list's exchange filter | `edgar.rs:118` (the recorded real file) |
| The SEC list read once | `edgar.rs:140` |
| Partial directory failures | `search.rs:286` |
| Forgotten prices | `figures.rs:874` |
| One venue rule | sources `venue.rs:254`, :256 |
| OTC | `listing.rs:374`, `venue.rs:281`, `shorts.rs:76`, `edgar.rs:100`, `feeds.rs:1857` |
| Resting failed reads | `filings.rs:1897` |
| Intraday bars, `bars_source` and `bars_miss` keys | `listing_isolation:129`, :148, :135 |
| Filer re-check | `filings.rs:1971` |
| Migration 6 read text and fold order | `cache.rs:410`, :415, :343 |
| Held lookup | `entries.rs:418` |
| Interlisted age | `filings.rs:1964` |
| Told dedupe | `filings.rs:1947` |
| Space rule | `tests_types.rs:649`, `listingKey.test.ts` |
| Single dormant match | `filings.rs:1991` |
| Fills | `venue.rs` |
| Page: resolver, Add trade and News taking one alone | `listingdata:74`, `modals:83`, `marketcards:617` |
| Page: Short interest | `shorts.test.ts` |
| Page: the exchange prompt | `modals:110` |
| Page: the Exchange box's width | `modals:129` |
| Page: no Re-read | `disclosures.test.ts` |
| Page: "Written by its terms" gone | `modals:102` |
| Page: watchlist and News keys | `markets:56`, `marketcards:649` |
| Page: document by (source, id) | `disclosures.test.ts`, `marketcards:743` |
| Page: waiting line | `disclosures.test.ts` |
| Page: resolver failure line | `listingdata:74` |
| Page: venue-less News tag with no chip on | `marketcards:632` |
