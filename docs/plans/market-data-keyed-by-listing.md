# Plan: market data is kept per listing and filings per company, never per ticker

Brief 34, sections 2 and 3. Section 1 (orders) shipped in #520.

## For the owner to decide

Nothing open.

How two listings are found to be one company needed no decision. No source the app reads states a shared identifier (checked 2026-10-16):
- Wealthsimple's web app requests 94 fields on a security, and none is an ISIN, a CUSIP or a company.
- The SEC's ticker list covers US listings only.
- SEDAR+ names Canadian issuers only.

Each regulator does name its filer by its legal name. A listing in Canada and one in the US under the same ticker root are one company when the two regulators' own names for their filers agree (Approach, 4). No new source is read.

## Scope

The market cache, the market readers, the disclosure and release notices, and the page's listing lookups are changed so that:
- everything about one listing (its chart, bars, source choice, TMX form, short interest, breakdowns, news marks and failure lines) is stored and found under that listing's Bagholder id;
- everything about one company (its filings, its SEDAR+ profile, its SEC filer number, what was already told about them) is stored and found under that company's id.

These are the 23 unsafe server places and 14 unsafe page places in brief 34's inventories ([server](https://github.com/ProfessorBagholder/Bagholder/blob/architecture-briefs/briefs/34/server.json), [page](https://github.com/ProfessorBagholder/Bagholder/blob/architecture-briefs/briefs/34/page.json)). The two unsettled places are settled below.

`SPEC.md` changes in one place: the Disclosures row (market-data-17) says what makes two listings one company (Approach, 4).

Nothing else on screen changes, except that wrong data stops appearing:
- a listing's chart is its own;
- TELUS's page no longer lists AT&T's filings;
- a notice names the listing it is about.

Out of scope: orders (#520), and the phone apps.

## The old app here

The old app named every instrument by its ticker, and its caches followed (`docs/old-app-mistakes.md`, "Things identified by Wealthsimple's ids and bare symbols", the mistake `docs/architecture.md` §5's *why* records). Stage 6 moved these tables into the market cache without re-keying them. The record, the engine and the sources crate already follow §5: quotes, daily closes, the source chosen and the reads are kept per instrument id (`sources/schema/v5.sql`: `quotes`, `daily_closes`, `chains`, `reads`), and the TMX form a quote answered under is written to the book as a reference (`sources/src/quotes.rs:226–230`).

This plan carries nothing over from the old readers' keys. It does carry over two things, each checked:
- **The readers' source chains and parsing.** They are not about identity, and their fixtures stay.
- **The SEC reader's name match** (`market/src/edgar.rs:296–302`). It compares Wealthsimple's name for the listing with the SEC's. It is replaced by a comparison of the two regulators' own names for their filers (Approach, 4), and it is never used to pick a listing.

A new entry in `docs/old-app-mistakes.md` records what was wrong: the market cache, filings and notices kept per ticker, and what holds the fix (the three rule tests).

## How the leading products do it

- **Interactive Brokers** gives every listing its own contract id. The same product in another market or currency is another contract, and IBKR tells API users to name a contract by its id and exchange, never by its symbol ([Contract IDs](https://www.interactivebrokers.com/docs/web-api/trading/instrument-discovery/contract-i-ds), [best practices](https://www.interactivebrokers.com/docs/general/contracts/contract-management/best-practices.md), read 2026-10-16). Their ids never change.
- **FIGI** (the open identifier standard) has three levels ([openfigi.com/api/documentation](https://www.openfigi.com/api/documentation), read 2026-10-16):
  - a listing on one venue;
  - a composite for one country;
  - a share class across countries.

  That is the split this plan makes: price data per listing, and company facts above the listings.
- **The SEC** identifies a filer by its CIK, and its published ticker list ties tickers to CIKs ([company_tickers.json](https://www.sec.gov/files/company_tickers.json), as `market/src/edgar.rs:19` reads it). One CIK can have several tickers.
- **SEDAR+** identifies a reporting issuer by its nine-digit profile number (`SPEC.md` market-data-17).
- **Trading journals.** TradeZella, Tradervue, TraderSync and Edgewonk publish nothing on keying per listing. They import a broker's executions with the broker's symbol and do not keep company filings. Bagholder follows the brokers' and data vendors' practice above.

## Open questions

1. **A listing nobody holds or watches (a directory row, a news chip, a heatmap tile): which id is it under?**
   - *Objective:* one id per listing for everything the cache keeps.
   - *Known:*
     - §5 says every instrument has a Bagholder id, and §6 puts instruments and their references in the book.
     - The book already makes an instrument for a watched listing nobody holds: `Book::listing` in `book/src/watched.rs:58`, found by its identifying references, else made new, with a `Listing` reference `<symbol>@<MIC>`.
     - The sources crate already writes learned references to the book.
   - *Settled:* the same function, widened from "watched" to "read". The first time the app reads market data for a listing, that listing becomes a book instrument with no holding and no watch row, found next time by the same references. Nothing is added to what any screen lists.
   - *Cost:* one row per listing ever opened.
   - The `Listing` reference's comment ("identifies only among watched listings") becomes "among listings no record names". It still never merges a record's instrument, because a record's instrument is found by the broker's id first.
2. **U20, Vanguard's and iShares' Canadian fund lists.** They are lists of Canadian listings. They are asked only for a listing on a Canadian venue, and the venue is checked against the row. A US fund with the same ticker is not looked up there.
3. **U23, CIRO's short-position and short-volume files.** Their rows are kept by (symbol, venue), not by symbol, so two Canadian venues' rows for one ticker both stand.

## Approach

### 1. One id per listing, from the page to the cache
- **The id comes from one function.** `Book::listing` (renamed `Book::listing_of`, made public) is the only place a listing gets its id: by its broker id, then its directory reference, then `<symbol>@<MIC>`, else new. It refuses when its references name two instruments, as now.
- **A route given a symbol and venue resolves it once.** The id goes back in the answer, and the page uses that id afterwards. Where the venue is missing, or the book holds two listings for it, the route refuses with the listing's words. It never picks one (rule test 3).
- **The page keys by id.** Its listing state, watch matches, news chip, held marks and shorts lookup match the listing by that id (`web/src/lib/listing.svelte.ts`, `markets/Watchlist.svelte`, `markets/News.svelte`, `markets/newsChip.ts`, `markets/Shorts.svelte`, `trade/discStore.svelte.ts`).
  - The address stays `#markets/listing:SYMBOL@VENUE`, which names one listing and stays shareable.
  - An address with no venue keeps today's rule from #520: no Buy, and no holding opened.

### 2. The market cache keyed by listing (market cache migration v6)
§6 says deleting the market cache "loses only time". So the ticker-keyed tables are dropped and remade keyed by `instrument_id`:
- `price_history`, `history_fetches`, `price_bars`, `bar_fetches`;
- `shorts`, `news` (listing rows);
- `exposures` (`share:` and `fund:` records become one row per instrument);
- `universes` is not per listing and stays.

The meta keys that stood for per-listing facts are removed, each replaced by the per-instrument store the sources crate already has, so no second copy is written:

| Today (meta key) | Becomes |
|---|---|
| `bars_source:<SYM>` | `chains(instrument_id, kind)`, through `cache.won` / `cache.winner` |
| `bars_miss:<SYM>\|<tf>` | `reads(subject = instrument id, kind, …)` with outcome not-carried |
| `tmx_form:<SYM>` | the book's `TmxForm` route reference on the instrument, as `sources/src/quotes.rs` already writes it, read through `Listing::route` |

The in-memory maps (pending reads U4, chart notes U5, the archive set U22, the news "Reading…" set U24, the short-interest failure lines U25) are keyed by `InstrumentId`.

Every reader in `market/src/history.rs`, `tmx.rs`, `exposure.rs` and `shorts.rs` takes the sources crate's `Listing`, which carries its id, in place of a bare symbol.

### 3. Filings and their notices keyed by company (book migration v30)
- **`issuer_refs(scheme, value, issuer_id)`.** A new book table, mirroring `instrument_refs`. It holds the SEC CIK and the SEDAR+ profile number. `book/src/identity.rs` already has `issuers`, `add_issuer`, `attach_issuer` and `listings(issuer)`. Nothing in production calls them yet; this plan is their first user.
- **Filings move into the book's company scope.** `filings` and its stamps (`store/src/feeds.rs`) are keyed by `issuer_id`.
  - The SEDAR+ profile is an issuer reference, so `sedar_profile:<SYM>` goes.
  - A filing's document is opened by (issuer, id).
- **Notices.** The told marks and watermarks (`filings:<SYM>:<source>`, `release:<SYM>:<digest>`) are re-keyed to the issuer, and to the listing for a release. The migration carries each old mark:
  - where the book holds exactly one listing with that ticker, to that listing's issuer;
  - where it holds several, to every candidate, so nothing is told twice.
- **The rest of the release and payer paths match by id**, never by ticker: release scope (U12), the "a wire already carried it" check (U13), and the payer read (U15).

### 4. Which listings are one company
- **Within one market, the regulator's own number decides.** Two listings are one company when they map to the same SEC CIK (the SEC's ticker list, for US listings) or the same SEDAR+ profile.
- **Across the two markets, the regulators' own names decide.** A Canadian listing and a US listing with the same ticker root are one company when the SEDAR+ issuer name and the SEC filer name match under `names_match`, the rule the SEC reader uses today (`market/src/edgar.rs`). Wealthsimple's name for the listing is never one of the two. So TELUS Corporation and AT&T Inc., both T, stay two companies, and Royal Bank of Canada on both markets is one.
- **A join is read, never merged**, as §5 already does for successions. It is decided again whenever either name is read.

`SPEC.md` market-data-17 gains one sentence saying so.

### What stays the same
- The sources crate's quote and closes paths, and every source chain's order.
- The engine.
- The wire types, except that the listing and filings answers carry the listing id and the issuer id (`model/src/wire.rs`, regenerated into `web/src/lib/generated/wire.ts`).

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green in `rust/`, and `cargo clippy --workspace --lib --bins` clean.
- [ ] `npm run check`, `npm test` and `npm run e2e` green in `web/`. The screenshot baselines are unchanged.
- [ ] **Rule test 1** (a property test, `server/src/tests_listings.rs`):
  - *Inputs:* generated pairs of listings that share a ticker, of four kinds: two venues; a stripped suffix; a coin and a share; an option and its underlying. Symbols, venues and currencies are drawn from the venue table and the currency list, not named.
  - *Readers:* every reader with a per-listing store (daily and intraday bars, fetch stamps, misses, pending reads, notes, chosen source, TMX form, short interest, breakdowns, archive, wait marks, failure lines).
  - *Assertion:* in either order of reads, the data stored, cached or shown for one listing is never the other's.

  It is shown failing on master in the PR.
- [ ] **Rule test 2** (a schema test): every table in the market cache with a per-listing or per-company row has `instrument_id` or `issuer_id` in its key, and no meta key is written whose name holds a ticker. It reads the schema and every `meta` write in the workspace's source, and is shown failing on master.
- [ ] **Rule test 3** (a boundary test): every route the page calls that takes a symbol refuses when the book holds two listings with it and the request names no venue, and resolves to the one id when it names one. It drives the routes through the router's own list, and is shown failing on master.
- [ ] **A company test.** Two listings sharing a CIK or a SEDAR+ profile, or a Canadian and a US listing whose regulators name their filers alike, show one set of filings. Two companies sharing a ticker never do, generated across names and tickers. One notice is told once across the old marks the migration carried.
- [ ] **The browser test** on the signed-in server's made-up book, which holds TD on the TSX and the NYSE: each listing's page draws its own chart and its own short interest. The demo book gains a second company with a shared ticker (made up, `demo_book.rs`): each listing's page lists only its own company's filings.
- [ ] **Migrations:**
  - market cache v6 on a copy of the owner's cache: it opens, and charts read again;
  - book v30 against every stored past schema (the existing migration test), with the told marks carried as stated.
- [ ] Rendered on the scratch server on a copy of the owner's data (`SPEC.md` §7): the listing page, the News card's three tabs, Portfolio sectors and the heatmap at 1200, 1340, 1440 and 1680 px, a states table, and a screenshot per state.
- [ ] **Cost** against `docs/architecture.md`'s budget: requests and bytes on a first and a second open, CPU and memory over an hour idle, and the capacity test on the owner-size book. The cache re-read after migration is measured: the first open of each chart reads it again once.

## Surfaces to check beyond the diff

- `rust/crates/sources/schema/` (v6 snapshot) and `rust/crates/book/schema/` (v30 snapshot);
- `web/src/lib/generated/wire.ts`;
- `docs/old-app-mistakes.md` (the new entry);
- the told-mark migration's handling of an ambiguous ticker;
- `TIMED_WAITS`, if any read changes how it waits (none is planned);
- `SPEC.md` market-data-17;
- the `Listing` reference's doc comment in `core/src/instrument.rs`.

## Right to refuse

Nothing found conflicts with `SPEC.md` or `docs/decisions.md`.

## Anti-stub self-check

To be initialled at the PR.

## Verification

To be filled at the PR.

## Handoff

Not started. Built in one PR after the gate's verdict.

**Nothing left running.**
