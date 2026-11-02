# Plan: the market heatmaps are read at start, kept fresh, and the Canada read is one request

## For the owner to decide

Nothing open: the owner's decision is in `docs/decisions.md` (2026-11-02).

## Scope

**In it:**
- The three market universes the heatmap shows (Canada's S&P/TSX 60, the US and International from Nasdaq's screener) are read when the app starts and refreshed every `UNIVERSE_STALE_SEC` (30 minutes) while it runs, whether a page shows them or not.
- The Canada read becomes one request for every constituent's price and change.
- The heatmap's empty state says what it is doing.

**Out of it:** any other source's schedule. The holdings and watchlist heatmaps are already built from the book and quotes.

## What happens now

Read from the code, 2026-11-02:
- **On demand only:** `feeds::universe_shown` reads a universe only while a page watches `universe:<key>`.
- **Already kept:** the rows are stored (`sf::replace_universe`) and loaded at start. A scratch copy read Canada, restarted, and showed all 60 at once.
- **So the gap:** a market no page has opened yet shows "Not read yet." for its first read.
- **The first read is slow:** `fetch_canada` asks TMX `getQuoteBySymbol` once per constituent, paced at 0.6 s on `app-money.tmx.com` and queued behind the start's other TMX reads. That's about 50 s on a scratch copy, and several minutes on the owner's after a start.

## How others do it

Read 2026-11-02.
- TMX's own GraphQL service (`app-money.tmx.com/graphql`, the one `fetch_canada` already uses) answers `getQuoteForSymbols(symbols: [String])` with `symbol`, `longname`, `price` and `percentChange` for many symbols in one request. Checked here with three constituents.
- Its `Quote` type has no sector field (the query is refused with "Cannot query field sector"). `getQuoteBySymbol` carries `sector`.

## Approach

- **At start:** a loop on the app's own waits (`App::wait`, listed in `TIMED_WAITS` with its reason). Each source is read when it's due (`source_due_in`, unchanged), at most every 30 minutes, and as the app starts, book open or not: the universes read nothing from the book, so an install whose sign-in waits never waits on it. `universe_shown` stays as the page's way to wake it early. The loop stops with the app.
- **Canada:**
  - The constituents come from `getIndexConstituents` as now.
  - Prices and changes come from one `getQuoteForSymbols` request with every constituent's symbol.
  - Each row's sector is the one stored for that symbol from an earlier read. A constituent with none stored, one new to the index, or one whose stored sector is older than `UNIVERSE_SECTOR_STALE_SEC` (30 days, a named constant: TMX reclassifies a company's sector rarely, and a month bounds how long a reclassification stays wrong) is asked `getQuoteBySymbol` for its sector, paced as now. That's a few single calls a day at most, spread out, never the 60 at once after the first read.
- **The empty state:** while a first read is under way, the heatmap says `Reading the S&P/TSX 60…` (or the screener's name); a failed read says why, as now.
- **Cost:** two Canada requests and one Nasdaq request every 30 minutes while the app runs. The idle-cost test (`TIMED_WAITS`) gains this one loop, with its reason.

## Acceptance criteria

- [ ] A started app with no page open reads each source, book open or not. Held by a test with the stand-in source (`fetch_universes`' test seam), counting the reads with no page watching.
- [ ] Refreshed at most every 30 minutes, and not again within that while rows are fresh. A test on the clock seam.
- [ ] The Canada read makes one quote request for all constituents, and asks a sector only for symbols with none stored. Tests with recorded TMX answers:
  - the batch's answer is the one recorded here, made up and cut to its parsed part;
  - the second read asks no sector;
  - a stored sector older than `UNIVERSE_SECTOR_STALE_SEC` is asked again, and one younger is not (a test on the clock seam).
- [ ] The empty state reads `Reading …` while a first read is under way. A page unit test.
- [ ] `TIMED_WAITS` lists the loop; `test_no_wait_on_a_clock_that_is_not_accounted_for` passes.
- [ ] Capacity: the idle cost is unchanged apart from the listed loop.
