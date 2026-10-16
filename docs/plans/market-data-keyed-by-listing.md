# Plan: market data is kept per listing and filings per filer, never per ticker

Brief 34, sections 2 and 3, rebuilt to brief 40's design. Section 1 (orders) shipped in #520.

## For the owner to decide

Nothing open. Every rule below uses sources the app already reads: the TSX's interlisted list comes from the same host as its symbol search, and the SEC's ticker-and-exchange file replaces the ticker file it reads now. No new outside service is added.

## Scope

Today the market cache, the market readers, the disclosure and release notices, and the page's listing lookups find a listing's data by its bare ticker. After this change they never do:
- **A listing's market data** (chart, bars, chosen source, TMX form, short interest, breakdowns, news marks, failure lines) is stored and found under a `ListingKey`: a symbol on a listing venue, which cannot be made without its venue.
- **A filing** is stored once, under its own source and id, and belongs to a filer: an SEC CIK or a SEDAR+ profile.
- **Which filers a listing shows** is settled from each regulator's own key. Canada and the US are paired only where the TSX's interlisted list pairs them.

This covers the 23 unsafe server places and 14 unsafe page places in brief 34's inventories ([server](https://github.com/ProfessorBagholder/Bagholder/blob/architecture-briefs/briefs/34/server.json), [page](https://github.com/ProfessorBagholder/Bagholder/blob/architecture-briefs/briefs/34/page.json)), and both unsettled ones.

What changes on screen:
- **A listing's chart, short interest, breakdowns and news marks are its own.**
- **Filings.** A listing shows its own regulator's filings, plus its partner's where the TSX list pairs them. TELUS (`T` on the TSX, paired with `TU`) no longer shows AT&T's filings, and Bombardier no longer shows Bradesco's.
- **A ticker typed or linked without a venue** opens directly only when one listing answers. Otherwise each listing is shown as its own row with its venue.
- **A book instrument whose records name no venue** shows its trades and figures, and says its market data waits for its venue. It no longer gets another listing's data guessed from its currency.

Every `SPEC.md` row that changes is listed under Approach, 7.

Out of scope: orders (#520), and the phone apps.

## The old app here

The old app named every instrument by its ticker, and its caches followed (`docs/old-app-mistakes.md`, "Things identified by Wealthsimple's ids and bare symbols"). Stage 6 moved those tables into the market cache without re-keying them.

The sources crate already follows `docs/architecture.md` §5 for what feeds the figures: quotes, daily closes, the source chosen and the reads are kept per book instrument (`sources/schema/v5.sql`: `quotes`, `daily_closes`, `chains`, `reads`). Those stay as they are.

This plan carries over none of the old readers' keys, and removes three guesses the old app made:
- **The SEC match by bare ticker for a Canadian listing**, guarded by a word-overlap name check (`edgar.rs:296–303`, `has_filer` :319). "Energy Fuels" passes it against "Energy Transfer".
- **SEDAR+'s first ranked match**, kept for the ticker for good (`sedar.rs:566–567`, `sedar_profile:<SYM>`).
- **A venue guessed from currency** (`venue.rs:12–17`, `market_of_listing` :39, `Listing::market`, `contract.rs:135`), and **TMX's retry under another venue's form** (`tmx_resolve`, `tmx.rs:174`). Both read another listing's data as this one's.

What is carried over, each checked: the readers' source chains and parsing, which are not about identity, along with their fixtures.

A new entry in `docs/old-app-mistakes.md` records the market cache, filings and notices kept per ticker, and the three tests that hold the fix.

## How the leading products do it

Each was read 2026-10-16.
- **Interactive Brokers** gives every listing its own contract id, and asks API users to name a contract by id and exchange, never by symbol ([Contract IDs](https://www.interactivebrokers.com/docs/web-api/trading/instrument-discovery/contract-i-ds)).
- **FIGI** identifies a listing per venue, and a composite "within the same country or market" ([OpenFIGI](https://www.openfigi.com/api/documentation)). That is the split `listing_venue` makes: the MIC in Canada, `US` for the US market (`sources/src/venue.rs:81`).
- **TradingView** addresses a symbol as `{EXCHANGE}:{NAME}`, and its search lists each listing as its own row ([widget docs](https://in.tradingview.com/widget-docs/tutorials/build-page/dynamic-symbols/)).
- **Sharesight's trade import** takes a market code per trade ([fields](https://help.sharesight.com/import_bulk_trades)).
- **Canadian exchanges must publish their inter-listed securities** under National Instrument 23-101, s. 6.6.1(3)–(4). The TSX publishes its list at [`tsx.com/files/trading/interlisted-companies.txt`](https://www.tsx.com/files/trading/interlisted-companies.txt):
  - the columns are `Symbol | Name | US Symbol | Sector | International Market`;
  - 266 rows, 195 of them with a US symbol, as of October 1, 2026.
- **The SEC publishes each ticker with its exchange and CIK** ([`company_tickers_exchange.json`](https://www.sec.gov/files/company_tickers_exchange.json), with the fields `cik, name, ticker, exchange`).
- **SEDAR+** identifies a reporting issuer by its profile number. Its reporting-issuer search answers by name.
- **Trading journals.** TradeZella, Tradervue, TraderSync and Edgewonk import executions under the broker's symbol and keep no filings, so there is no journal practice to follow here. The brokers' and data vendors' practice above is followed instead.

## Open questions

1. **Settled 2026-10-16: does SEDAR+ settle each large Canadian issuer to exactly one profile by exact legal name?**
   - *How:* every S&P/TSX 60 member was taken from TMX's constituents list, its company name from the TSX directory (each of the 60 is held by exactly one company record), and that name asked of SEDAR+ through the repo's own `sedar resolve`.
   - *Result: 59 of 60 settle to one profile*, once two facts SEDAR+'s answers show are built in:
     - **The query.** SEDAR+'s search matches names by their leading words, and punctuation in the query breaks it ("Waste Connections, Inc" finds nothing; "Waste Connections" finds the one profile). The query is the name up to its first comma, period or parenthesis, with "The" taken off either end.
     - **The comparison.** SEDAR+ writes the same legal names in other forms: "Bank of Nova Scotia, The", "First Quantum Minerals Ltd" with no period, and notes such as "(formerly Progressive Waste Solutions Ltd.)". Names are compared ignoring case, spacing, periods and commas, the placement of "The" (leading, ", The" or "(The)"), and a "formerly" note.
   - *The one tie is TELUS:* two profiles are both named "TELUS Corporation".
     - 000001114 reports in Nova Scotia only, and last filed in December 1998.
     - 000011998 is the current issuer, and last filed in July 2026.
   - *Rule, accepted by brief 41 on its conditions:* a listed issuer files annual financial statements every year (National Instrument 51-102, s. 4.2: within 90 days of year-end, or 120 for a venture issuer). Among profiles with the same exact name, one with no filing in the last 12 months plus 120 days is not a listed issuer's, and is set aside. With that rule all 60 settle; without it, TELUS shows "SEDAR+ lists no single issuer named TELUS Corporation".
   - *The answers are kept* as fixtures in `rust/crates/market/tests/replies/sedar-tsx60/`, and the reader's tests run against them.
   - *Brief 41's conditions, each held by a test in `market/src/sedar.rs` (`settle_tests`) and `market/tests/sedar_tsx60.rs`:*
     - the one normalization is applied to both names, and a note SEDAR+ adds after a name ("formerly …", "Operating name: …") is not the name;
     - an answer with no exact match, however full the page, is a miss, said as one;
     - the tie rule only narrows the exact matches, settles only where exactly one remains, bounds at 12 months + 120 days before the check, and is decided again at every re-check of the listing's filer;
     - generated tie sets with newest filings on either side of the bound, and the recorded TELUS answer.

## Approach

### 1. One key per listing: `ListingKey` (`sources/src/venue.rs`)
- **What it is:** a symbol on a listing venue. The venue is:
  - the MIC in Canada;
  - `US` for any US venue (`listing_venue`, as #520's orders compare listings);
  - `CRYPTO` for a coin and `INDEX` for a directory instrument, as their references already do (`following.rs:121`, :129);
  - an option's own OCC symbol.
- **It cannot be built without the venue**, as #520's `SecurityId` cannot be empty.
- **One function writes the symbol in one form.** `BRK.B` and `BRK-B` are one key, and `BRK` is another.
- **A book instrument reaches its market-cache rows through its current symbol and venue**, using the function #520's orders use.
  - A ticker change gives a new key. The old key's rows can be fetched again, and they age out of the cache.
  - Filings are not affected, because they are kept by filer.
- **Reading data never makes a book instrument.** `Book::listing` stays limited to watched listings (`watched.rs:58`), so a listing read before it is bought can never become a second instrument.

### 2. Which filings a listing shows
1. **A US listing → its SEC CIK.** Read from `company_tickers_exchange.json`: the row with the listing's symbol (class written with a dash, as the SEC writes it) on a US exchange. This file replaces `company_tickers.json` (`edgar.rs:19`). It is read once per process, as now, and is 275 KB smaller.
   - The bare-ticker match for a non-US listing and its `names_match` guard go.
2. **A Canadian listing → its SEDAR+ profile.**
   - *The issuer name comes from the listing's exchange:* for the TSX and TSX-V, the TSX directory's company record that holds the symbol (`market/src/search.rs`); for the CSE and Cboe Canada, the name TMX's quote gives.
   - *The profile is settled only on an exact match:* SEDAR+'s reporting-issuer search is asked with the name up to its first comma, period or parenthesis, and the profile is settled only when exactly one profile's English or French legal name equals the full name, compared as Open questions, 1 states. A profile with no filing in the last 12 months plus 120 days is set aside first (the proposed rule there).
   - *With none or two,* nothing is picked, and the page says `SEDAR+ lists no single issuer named …`.
3. **Cross-listing comes only from the TSX's interlisted list.** A row pairs `<symbol>@XTSE` or `@XTSX` with `<US symbol>@US`. Each listing in a pair shows both filers' documents.
   - A listing not on the list shows its own regulator's filings only. That includes one inter-listed from the CSE or Cboe Canada, which the list does not cover.
   - The list is read at most once a day, and kept in the market cache with its "As of" date.
   - A failed read keeps the last list, and is shown as that source's failure (`docs/architecture.md` §15).
4. **Where the answers live:** `listing_filers(listing, source, filer, settled_by, checked_at)`, in the market cache. Every input can be fetched again, which is §6's rule for the market cache. The book gains nothing.

### 3. Filings and their read text (market cache)
- **The table.** `filings(source, id, filer, …, subject, summary, enriched_at, enrich_version, enrich_final, enrich_reads, fetched_at)`, with primary key `(source, id)` and an index on `(source, filer)`.
  - `id` is the SEC accession number or SEDAR+'s document key, as today.
  - A listing's filings are the documents of the filers its `listing_filers` rows name.
- **Migration v6 carries every row it can attribute, with its read text unchanged:**
  - A row's filer is read from the row itself: a SEDAR+ row's `profile_no`, or an SEC row's CIK from its URL (`edgar.rs:265–267`).
  - A row with neither falls back to its ticker's `sedar_profile:` value, else it is dropped and counted.
  - A document stored under two tickers becomes one row, keeping the most complete read: `enrich_final`, then `enrich_version`, then `enriched_at`.
- **Stamps.** `filings_fetched:<SYM>`, `filings_sources:<SYM>` and `sedar_profile:<SYM>` go. Fetch stamps are kept per `(source, filer)`, and outcomes per listing.
- **Documents are opened by `(source, id)`.** `/api/filings/doc` and `/enrich` take the document, not a symbol.
- **Notices (in the book):**
  - The told scope `filings:<SYM>:<source>` (`server/feeds.rs:878`) becomes `filings:<source>:<filer>`.
  - Each watermark is carried to the filer or filers its ticker's stored rows came from. Where the cache is gone, the new scope's first sweep sets its mark without telling, as any new scope does now (`notify.rs:564–566`).
  - Release keys (`release:<SYM>:…`, :1312), the wire check (U13), the release scope (U12) and the news scope `news:<SYM>@<EX>` are keyed by `ListingKey`.

### 4. The market cache keyed by `ListingKey` (migration v6)
§6 says deleting the market cache "loses only time", so these tables are remade keyed by `ListingKey`, and their rows are read again:
- `price_history`, `history_fetches`, `price_bars` and `bar_fetches`;
- `shorts` and `news`;
- `exposures` (`share:` and `fund:` records).

The meta keys and in-memory maps that stood for one listing take a `ListingKey`:

| Today | Becomes |
|---|---|
| `bars_source:<SYM>` | `bars_source:<ListingKey>` |
| `bars_miss:<SYM>\|<tf>` | `bars_miss:<ListingKey>\|<tf>` |
| `tmx_form:<SYM>` | `tmx_form:<ListingKey>`, the form of this listing only. A form that answers nothing is this listing's miss. |
| `news_fetched:<SYM>@<EX>` and `news_source_fetched:` | the same, by `ListingKey` |
| pending reads (U4), chart notes (U5), the archive set (U22), "Reading…" (U24), failure lines (U25) | keyed by `ListingKey` |
| single-flight `filings:<SYM>` and `filings-shown:<SYM>` | `filings:<ListingKey>` |

Kept as they are, in rule test 2's allow-list, each with its reason:
- `yahoo_miss:<form>` and `coinbase_product:`, `coinbase_prev:`: each is the source's own name for exactly one listing;
- the TSX list and the SEC file: each is a whole-source copy;
- `carry.rs:97–100`: a one-time carry mark, not a listing.

The sources crate's `cache.won` and `cache.winner` (`DataKind`) and its per-instrument tables are not used for bars: they follow a book instrument across a ticker change, and these rows deliberately do not.

### 5. A ticker with no venue
- **Routes take a `ListingKey` or an instrument id, never a bare symbol.** A request that names a symbol without a venue does not parse, so rule test 3 is held by the types, as #520's are.
- **Each source supplies the venue it states:**
  - *Heatmap:* the US and international tiles come from Nasdaq's screener, which lists only US listings, so they are `@US`. The S&P/TSX 60 tiles take the exchange TMX's constituents give (`universes.rs:20`).
  - *Notifications* store the `ListingKey` they are about.
  - *ETF look-through rows* take the holdings file's own exchange column. Where there is none, the file's own sector and country columns are used, and nothing is looked up by ticker (U18, U19).
- **One resolver for a symbol given with no venue** (⌘K free text, an old link, a news tag). It answers with every listing of that symbol that the exchanges' directories the app reads (Nasdaq's, TSX's) list, each with its venue.
  - Exactly one answer opens directly.
  - Two or more are shown as rows, and the resolver never reduces them to one.
- **A book instrument whose records state no venue** gets no market data. Its venue comes from a record that states one: the broker's row when the pull links it (SPEC trade-43), or an `exchange` column added to the plain import layout. Until then its page shows its trades and figures, and says its market data waits for its venue.
- **A hand-entered trade** names its instrument by id, or by a `ListingKey` from the resolver (page U14).

### 6. Brief 34's remaining places
- **Server:**
  - U10: `known_filing_symbols` and `held_symbols` are keyed by `ListingKey`.
  - U11: the name for SEDAR+ comes from the exchange (Approach, 2), never the first book security with the ticker.
  - U17: `listing_payload` matches rows by `ListingKey`.
  - U21: an option's underlying breakdown takes the underlying's `ListingKey` from the contract.
  - U7's other callers go through the per-listing form: `news.rs:684`, :1852, the search fallback, and `feeds.rs:637` and :2473.
  - U8's `looking_at` and `reading_now` are keyed by `ListingKey`.
- **The page:** U2, U6, U8 and U12 match by `ListingKey`, which its address already carries (`#markets/listing:SYM@VENUE`), and a book row by its instrument id.
- **The demo book** (`write_bars`, `demo_facts.rs:128–170`) and `carry.rs` write through `ListingKey`.

### 7. `SPEC.md` and `docs/architecture.md`, in the same PR
- **market-data-17:**
  - A US listing's CIK comes from the SEC's ticker-and-exchange list.
  - The SEDAR+ profile is the one whose legal name is the issuer name the listing's exchange gives; none or two is said, never picked.
  - "A cross-listed issuer draws from both" becomes "a listing the TSX's interlisted list pairs with a US listing draws from both regulators".
- **market-data-4:** the form that answers nothing is that listing's miss. The other-forms retry, and the sentence that starts from the currency's usual form when the venue is missing, go.
- **trades-13:** the "follows its currency" clause goes.
- **trades-37:** the "stored under the bare ticker" sentence goes.
- **markets-60:** rewritten to Approach, 5 (the resolver).
- **`docs/architecture.md` §5, under Issuer:** two listings in Canada and the US are one issuer where the listing exchange's interlisted list pairs them. That is the exchange's own statement, kept beside the strong identifiers, and it never merges instruments.

### What stays the same
- The sources crate's quotes and closes, and the engine.
- The wire, except that the listing and filings answers carry a `ListingKey` and documents carry `(source, id)` (`model/src/wire.rs`, regenerated into `web/src/lib/generated/wire.ts`).

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green in `rust/`, and `cargo clippy --workspace --lib --bins` clean.
- [ ] `npm run check`, `npm test` and `npm run e2e` green in `web/`.
- [ ] **SEDAR+ fixtures first.** SEDAR+'s answers for every S&P/TSX 60 member are recorded, with which settle to one profile and which do not, and reported in the PR before the rest is built.
- [ ] **Rule test 1 (property).**
  - *Inputs:* generated pairs of listings that share a ticker: two venues; a stripped suffix; a coin and a share; an option and its underlying. Symbols, venues and currencies are drawn from the venue table and the currency list, never named.
  - *Readers:* every reader with a per-listing store.
  - *Assertion:* in either order of reads, what is stored, cached or shown for one is never the other's.

  Shown failing on master.
- [ ] **Rule test 2 (schema).** Every market-cache table with per-listing or per-filer rows has a `ListingKey` or `(source, filer)` in its key, and every meta key that names a listing is built through `ListingKey`, apart from the allow-list with its reasons. Shown failing on master.
- [ ] **Rule test 3 (boundary).** No route the page calls parses a bare symbol into a listing. Held by the types, and a test drives each route in the router's own list. Shown failing on master.
- [ ] **Filings routing.** The inputs are generated listings, regulator answers and interlisted rows, including names that share words ("Energy Fuels", "Energy Transfer").
  - A listing's filings are exactly its own filer's documents, plus its partner's where the TSX list pairs them.
  - A SEDAR+ answer with zero or two exact names settles nothing, and the page says so.
- [ ] **The resolver.** For a symbol with k listings, it answers k rows, each with its venue. Exactly one opens directly, and it never picks one of several.
- [ ] **No venue, no data.** For generated instruments with no venue, no market reader is called and no market-cache key is written.
- [ ] **Migration v6.**
  - Every attributable filings row is carried with its read text, byte for byte.
  - The dropped count equals the rows with no readable filer.
  - Duplicates keep the most complete read.
  - Run on a copy of the owner's cache, the counts are in the PR.
- [ ] **Told marks.** Across the migration, nothing is told twice, and a filing that arrives during the switch is told.
- [ ] **Browser tests.** On the signed-in server's made-up book, which holds TD on the TSX and on the NYSE, each listing draws its own chart and short interest. The demo book gains an interlisted pair and a same-ticker pair of different companies, and each listing lists exactly its own filings, plus its partner's.
- [ ] **Rendered on the scratch server** on a copy of the owner's data (`SPEC.md` §7): the listing page, the News card's tabs, Portfolio sectors and the heatmap, at 1200, 1340, 1440 and 1680 px, with a states table and a screenshot per state.
- [ ] **Cost**, against `docs/architecture.md`'s budget:
  - requests and bytes on a first and a second open;
  - CPU and memory over an hour idle;
  - the capacity test at the owner's size;
  - the TSX list (15 KB a day), the SEC file (275 KB smaller), and one SEDAR+ search per Canadian listing not yet settled;
  - the chart re-read after the migration, measured.

## Surfaces to check beyond the diff

- `rust/crates/sources/schema/` (the v6 snapshot), `web/src/lib/generated/wire.ts` and `docs/old-app-mistakes.md`;
- `server/src/carry.rs` and the demo book (`store/src/bin/demo_book.rs`, `server/src/demo_facts.rs`);
- the told-mark carry for a ticker that named two filers;
- `SPEC.md` (the rows in Approach, 7) and `docs/architecture.md` §5.

## Right to refuse

Nothing found conflicts with `SPEC.md` or `docs/decisions.md`. If the SEDAR+ fixtures show that large issuers do not settle to one profile by exact name, building stops there and the PR reports it.

## Anti-stub self-check

To be initialled at the PR.

## Verification

To be filled at the PR.

## Handoff

The SEDAR+ check is done (Open questions, 1). Building next.

**Nothing left running.**
