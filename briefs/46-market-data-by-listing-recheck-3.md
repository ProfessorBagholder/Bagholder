# Brief 46: #525 at `15e58d0a`, after brief 45: not yet. One release blocker and four defects remain

**Reviewed:** the range `750723ee..15e58d0a`. Two readers took the server and the page. Each planted the reverse of every change and probed the new code. I checked the lock-file blocker myself.

**Suites:**
- **Rust:** 1,688 passed. The one failure is the root-sandbox rollback test, which is not this PR's.
- **Clippy:** clean.
- **Page:** `npm run check` has 0 errors, and Vitest passed 155.
- **Browser:** 334 of 336 passed in one run. The two that failed:
  - the example.com popup, which this sandbox cannot load;
  - `dataflow.spec.ts:204` ("a minute idle: POST /api/events/watch"), which passed 3 of 3 alone and also passed at `750723ee`.
- **Capacity** (release build, `capacity run`): passes. Every listed operation is over budget only by the measure it is listed for.

## Verdict

**Not yet.** Every change from brief 45 is in, and planting its reverse fails a test or the capacity job:
- the names lookups' index hints;
- the measure-named `OVER_BUDGET`;
- the deep holding's own account in the back-dated rows;
- file rows matched by the day's name;
- offline kept typed on the three named paths;
- the wall clock;
- News dropping a late answer;
- the no-listing line;
- dismissing the offer;
- Add trade's line;
- SPEC markets-84 and markets-60.

Growth is now flat in every back-dated case, including the code-without-words one: 2,309 → 2,309 steps. The reference fold matched over 3,600 batches. Each index exists after every stored past schema. A missing index fails cleanly, per instrument.

## Required changes

1. **The committed `Cargo.lock` is stale, and a release would fail to build.**
   - **Cause:** `crates/market/Cargo.toml:31` adds a dev-dependency of the crate on itself, which the lock does not record. `cargo metadata --locked` fails with "cannot update the lock file … because --locked was passed".
   - **Who it hits:** `release.yml:83` and `rust/Dockerfile:16` build with `--locked`, so the release archives and the image would fail at the tag.
   - **Why CI is green:** CI's container job first runs a plain `cargo build` (`tests.yml:152`), which rewrites the lock in the checkout before the image is built from it.
   - **Required:** commit the regenerated lock. Make CI fail on a stale lock: run `cargo metadata --locked` (or build with `--locked`) before anything can rewrite it.
2. **A symbol two instruments had on the row's day is matched by today's name.** When `named_on` or `listed_on` finds two instruments, `held_on_day` falls through to `held_by_symbol` or `held_on_listing` (`server/src/entries.rs:155–158`).
   - **Probe:** A is HG on the CSE from 11-12. C is HG on the TSX, entered by hand on 11-12. A is renamed on 11-18. The file row `2025-11-12,Buy,HG,,3,…` was booked on C.
   - **Required:** two instruments with the name that day is a refusal, never a pick, with a test.
3. **A row is matched to an instrument that did not exist on its day.**
   - **Cause:** `name_on` gives an instrument its first name for any day before its history (`engine/src/input.rs:42–44`).
   - **Probe:** B, a new HG on the CSE first named on 11-19, took a row dated 11-12, when the CSE's HG was A.
   - **Required:**
     - An instrument is named on a day before its history only where no instrument had that name on that day. This still covers a file older than the broker's history.
     - Two candidates is a refusal.
     - Add a test.
4. **A SEDAR+ document read offline is still said in words.**
   - **Cause:** `scoped_documents` drops its error with `.ok().flatten()` (`market/src/sedar.rs:713`), and the read then fails at :774.
   - **Probe:** with the live readers offline, the header says "ZQRW · TSX's document sedar:1 could not be read: could not open the profile's documents to download from."
   - **Why the test misses it:** the PR's test feeds `Failure::Offline` through a closure, so it never takes this path.
   - **Required:** keep the refusal's kind on this path, and test it through the live readers.
5. **An address naming only a symbol acts on a late answer.**
   - **Cause:** markets-60 now says an answer that lands after the person changed anything changes nothing. But `App.svelte:280` → `listing.svelte.ts:198` acts whenever it lands.
   - **Probe:** hold `/api/listings`, open `#markets/listing:ZQLA`, then go to `#portfolio` and release the answer. The address becomes `#markets/listing:ZQLA@XTSE`, and because `replaceState` is used, the Portfolio history entry is lost.
   - **Required:** drop the answer as News does, with a test.

## Smaller, in the same PR

- **markets-84 against markets-92.** markets-84 says "a token the feed does not hold is resolved". The code resolves any ticker-shaped token, as markets-92 says the Stories tab does. Reword markets-84 to the code.
- **The stale "No listing of ZQNO is found on an exchange." line.** It stays after the person takes another chip or clears it. That action resolves the question the line answered, so clear it then.
- **`backdated-file-row` is never timed.** It has a 1,000 ms budget (`capacity.rs:971`), but `waited_on` (:198–200) leaves it out. An import is waited on, so judge its time.
- **A misplaced doc comment.** `market/src/lib.rs:50–51`: `clock_now`'s comment sits on `wall_now`.
- **`dataflow.spec.ts:204`** failed once in the full run. Check whether this PR's server changes make it intermittent.

## Holds (planted, caught)

| What | Caught by |
|---|---|
| Index hints | `a_back_dated_record_among_coded_sightings_costs_the_same_at_any_depth` (5,204 → 13,004 without them) |
| Allocation exemption put back; fill steps unlisted | the capacity job |
| Backdated-trade allocations unlisted; measure ignored in `listed` | capacity tests |
| Today's-name match for a file row | `a_files_row_is_matched_by_the_name_its_instrument_had_on_the_rows_day` |
| Three offline paths | `a_document_read_refused_offline_is_never_said_and_a_failed_one_is`, `a_canadian_filer_asked_offline_is_refused_by_its_kind` |
| Wall clock | `the_secs_list_read_before_a_days_sleep_is_read_again_after_it` |
| `test-clock` | never in a release build (resolver 2; the release fingerprint lists no features) |
| News: late answer (three plants), no listing, dismissal (three plants) | `marketcards:684`, :726, :737 |
| Add trade's line | `modals:114` |

#527 (the engine re-deriving from a back-dated day) is listed in `OVER_BUDGET` and stays separate. `backdated-trade` runs 364 ms on the Pi at four times the owner's depth.
