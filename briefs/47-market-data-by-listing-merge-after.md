# Brief 47: #525 at `3904a1ef`, after brief 46: merge after four small changes

**Reviewed:** the range `15e58d0a..3904a1ef`. One reader planted the reverse of each change and re-ran the earlier probes. I checked the findings below.

**Suites:**
- **Rust:** `cargo test --workspace --locked` passes 1,690. The one failure is the root-sandbox rollback test, which is not this PR's.
- **Clippy:** clean.
- **Page:** `npm run check` has 0 errors; Vitest passes 155.
- **Browser:** 336 of 337 pass in one run. The one failure is the example.com popup, which this sandbox cannot load; `everyscreen:159` passed.

## Verdict

**Merge after four small changes.** None of them changes the design, and I will check only those four.

Every change from brief 46 is in:
- **The lock file is current.** With the old lock, `cargo metadata --locked` and the container job's first, now locked, build both exit 101 before an image is built.
- **A row's day decides between instruments.**
  - Picking the first of two, or giving an instrument its first name for earlier days, fails `a_rows_day_decides_between_instruments_that_shared_its_name`.
  - The previous probe's ambiguous row is refused.
  - A row dated on A's listing before B took the ticker goes to A, as trade-37 states.
- **An unplaced file row is said, never dropped:**
  - **What the person sees:** the import report's "1 with a problem", and a header line naming the row, the reason and the day.
  - **Re-imports:** importing the file again changes nothing.
  - **Old records:** those with no `unnamed` field still read.
- **SEDAR+ offline is typed.** Planting the old `.ok().flatten()` fails both new tests. Removing the fallback to SEDAR+'s newest page, when a profile walk fails, is right. That page is not the profile's own, and its filter kept rows that name no profile, so it could show another issuer's documents.
- **A late address answer is dropped.** Removing `still()` fails `listingdata`.
- **The smaller items:** markets-84, the no-listing line clearing, `backdated-file-row` timed, the doc comment moved.

## Required

1. **Two refusals are held by no test.** Plant each, and all 34 entry, import and route tests still pass:
   - **"None then, two now":** `entries.rs:154`, planted to take the first.
   - **`enter` ignoring the refusal:** `entries.rs:237`, `.map_err(bad)?` changed to `.unwrap_or(None)`. The new test calls `held_on_day` directly, never through `enter`.

   **Required:** a test for each, the hand entry driven through `enter`.
2. **The refusal can state something false.**
   - **The case:** for a day before both instruments' histories, it says "ZQRA was the name of 2 instruments on 2020-01-02", though neither existed then (`entries.rs:155`, `engine.rs:597`).
   - **The other message:** the "two now" message names no day, short of trade-37's "saying which name and day".
   - **Required:** word each message by what is true: two instruments have had the name since that day, or two had it on that day. Add a test of the words.
3. **A row's other problems now keep the row instead of refusing the file.** The `.or_else` at `csv_import.rs:229` catches every refusal from `traded`. A row naming exchange `ZZEX` is now kept with "\"ZZEX\" is not an exchange", and the rest of the file imports. At `15e58d0a` the whole file was refused.
   - **The verdict:** this is right by trade-37 ("the row is kept with its problem").
   - **Required:** a test, and a line in trade-37 naming an unknown exchange among a row's problems.
4. **Dead code.** `InstrumentInfo::name_on` (`engine/src/input.rs:42`) has no callers left, and still falls back to the first name, which is the behaviour this change removed. Delete it.

## Smaller, not blocking

- **Unlocked builds:** `rust-macos`, `rust-windows` and `capacity.yml` build without `--locked`. The `rust` job's check fails the PR first, so this is tidiness: add `--locked` to them.
- **`backdated-file-row`:** removing it from `waited_on` fails no test. The only timing test covers start-up.
- **markets-84:** it does not say a typed ticker is resolved only while no chip is on.
