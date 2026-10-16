# Brief 40: Plan #522 (market data by listing, at `9642f1e9`): the design to build. Replaces brief 39's #522 section

Brief 39 listed four problems with plan #522 and put the cross-listing question to the owner. It should have given the solutions. Each one has an established answer, and every answer below uses sources the app already reads. No new outside service is added. Brief 39's #521 section stands as written.

**Verdict:** rewrite the plan to this design, then build it. The plan's direction holds: market data per listing, filings per company, and the three rule tests. What changes is how a listing is keyed, which filings a listing shows, and where filings live.

## What established systems do (each checked 2026-10-16)

- **A listing is a symbol on a venue, and the US is one market.** FIGI identifies a listing per venue, and then a composite that "enables users to link multiple FIGIs at the trading venue level within the same country or market in order to obtain an aggregated view for an instrument within that country or market" ([OpenFIGI](https://www.openfigi.com/api/documentation)). #520 already compares listings this way (`sources/src/venue.rs:81`, `listing_venue`): the venue's code in Canada, `US` for any US venue. US symbols are unique across US exchanges under one national plan (SEC Release 34-56037, brief 37).
- **Quote sites address a listing by venue and symbol.** TradingView's symbol format is `{EXCHANGE}:{NAME}` ([widget docs](https://in.tradingview.com/widget-docs/tutorials/build-page/dynamic-symbols/)). A search for a ticker lists each listing as its own row with its venue.
- **Canadian exchanges publish which of their securities are also listed in the US.** This is required by National Instrument 23-101, s. 6.6.1(3)–(4): "A recognized exchange must maintain a list of inter-listed securities that are listed on the exchange as of the last day of each calendar quarter", and must disclose it on its website. An inter-listed security is "an exchange-traded security that is also listed on an exchange that is registered as a 'national securities exchange' in the United States" (s. 6.6.1(1)).
  - The TSX publishes its list as a text file: [`tsx.com/files/trading/interlisted-companies.txt`](https://www.tsx.com/files/trading/interlisted-companies.txt). That is the host the app's symbol search already reads (`market/src/search.rs:13`).
  - The file is headed "As of October 1, 2026". It has 266 rows (14.7 KB), with the columns `Symbol | Name | US Symbol | Sector | International Market`.
  - 195 rows give a US symbol. Two examples: `T:TSX TELUS Corporation TU NYSE` and `ABX:TSX Barrick Mining Corporation B NYSE`.
  - TSX says it compiles the list "on a best efforts basis from information received from issuers" ([NI 23-101 page](https://www.tsx.com/en/trading/toronto-stock-exchange/fee-schedule/ni-23-101)).
- **The SEC publishes each ticker with its exchange.** [`company_tickers_exchange.json`](https://www.sec.gov/files/company_tickers_exchange.json) has the fields `["cik","name","ticker","exchange"]`.
  - It is 524 KB. The file read today, `company_tickers.json`, is 799 KB.
  - 189 of the TSX list's 195 US symbols are in it, each on the exchange the TSX names. NYSE American and NYSE Arca appear as `NYSE`, so the two are compared at the US-market level.
- **The TSX's company directory names the issuer of each listing.** The app already reads it. A search returns company records, each with its instruments, and exactly one record holds a given symbol: `BBD.B` is in `Bombardier Inc.`, `GIB.A` in `CGI Inc.`, and `TD` in `Toronto-Dominion Bank (The)`. TMX's quote gives the issue's name instead ("Bombardier Inc. Class B Subordinate Voting Shares").
- **Imports state the market.** Sharesight's trade importer takes a Market Code column ([fields](https://help.sharesight.com/import_bulk_trades)). Its BMO InvestorLine guide says: "Against each trade add the market code under which the stock is held, e.g. TSE, CNSX, NASDAQ, NYSE" ([guide](https://www.sharesight.com/eu/partners/bmo-investorline/)).
- **Ruled out:**
  - **TMX's quote API has no identifier.** It answers "Cannot query field" for `cusip`, `isin`, `figi`, `lei` and `sedarId`.
  - **SEDAR+ gives an issuer's CUSIP-6**, the unparsed cell beside the profile number (`sedar.rs:489–505`). But no source the app reads gives a listing's CUSIP to compare it with.
  - **TMX's `getCompanyFilings` lists a TSX symbol's filings**, but its documents are served from `app.quotemedia.com`, a new host.

## 1. Which filings a listing shows (replaces plan §4, brief 39 item 1)

**Each listing reaches its own regulator's filer by that source's own key. A Canadian and a US listing are one company only where the TSX's interlisted list pairs them. No name comparison joins anything.**

1. **A US listing → its SEC CIK.**
   - Look the listing up in `company_tickers_exchange.json`: the row whose ticker is the listing's symbol (class written with a dash, as SEC does) and whose exchange is a US exchange.
   - Remove the bare-ticker match for non-US listings and its `names_match` guard (`edgar.rs:296–303`, `has_filer` :319). A Canadian listing reaches SEC only through rule 3.
2. **A Canadian listing → its SEDAR+ profile.** The issuer name comes from the listing's exchange:
   - for TSX and TSX-V, the TSX directory's company record that holds this symbol;
   - for the CSE and Cboe Canada, the name TMX's quote gives.

   SEDAR+'s reporting-issuer search is asked with that name. The profile is settled only when exactly one profile's English or French legal name equals it. Letter case and spacing are ignored, and the directory's trailing "(The)" is read as the leading "The" it stands for. With none or two, nothing is picked and the page says so ("SEDAR+ lists no single issuer named …").
   - **Today** the first of the search's ranked matches is taken (`sedar.rs:566–567`) and kept for the ticker for good (`sedar_profile:<SYM>`, brief 34 U9).
   - **I could not check SEDAR+'s real answers from here**: its bot gate refuses plain clients. The PR records its answers for every S&P/TSX 60 member, from the TMX constituents list the heatmap already reads, as fixtures. It shows that each member settles to one profile, and lists any that does not, with SEDAR+'s answer, before anything else is built.
3. **Cross-listing comes from the TSX's interlisted list.** A row pairs `<symbol>@XTSE` or `@XTSX` with `<US symbol>@US` when its market is a US exchange. Each listing of the pair shows both filers' documents: its own, and its partner's through rules 1 and 2.
   - **Not on the list:** each listing shows its own regulator's filings only. That includes a security interlisted from the CSE or Cboe Canada, which the TSX's list does not cover.
   - **What breaks today's wrong joins:** Canada's `T` is TELUS, paired with `TU`. It never meets AT&T's `T`, and `BBD` on the NYSE (Bradesco) never meets Bombardier.
   - **Reading the list:** at most once a day, kept in the market cache with its "As of" date. A failed read keeps the last list and is shown as that source's failure (§15).
4. **Where each answer lives.** In the market cache, as `listing_filers(listing, source, filer, settled_by, checked_at)`. Every input can be fetched again, which is §6's rule for the market cache. The book gains nothing, so the plan's book v30 and `issuer_refs` go.

## 2. Where filings and their read text live (replaces plan §3, brief 39 item 2)

**Filings stay in the market cache (§6), keyed by the document. A document never changes and its read text belongs to it.**

1. **The table.** `filings(source, id, filer, …, subject, summary, enriched_at, enrich_version, enrich_final, enrich_reads, fetched_at)` with primary key `(source, id)`, and an index on `(source, filer)`.
   - The `id` is SEC's accession number (`sec:<accession>`) or SEDAR+'s document key, as today.
   - The `filer` is the CIK or the profile number.
   - A listing's filings are the documents of the filers its `listing_filers` rows name.
2. **Migration v6 carries every row it can attribute, with its read text unchanged.**
   - **Each row's filer is read from the row itself:** a SEDAR+ row's `profile_no`; an SEC row's CIK from its URL (`/Archives/edgar/data/<cik>/` or `CIK=<cik>`, `edgar.rs:265–267`).
   - **Gaps:** a row with neither falls back to its ticker's `sedar_profile:` value, or else is dropped and counted.
   - **Duplicates:** the same document stored under two tickers becomes one row, keeping the most complete read (`enrich_final`, then `enrich_version`, then `enriched_at`).
   - **Why this is safe:** a row that sat under the wrong ticker is still its own filer's document. It shows only where rule 1 routes to that filer.
3. **Fetch stamps and source outcomes.** `filings_fetched:<SYM>`, `filings_sources:<SYM>` and `sedar_profile:<SYM>` go. They are replaced by stamps per `(source, filer)` and outcomes per listing.
4. **Documents are opened by `(source, id)`.** `/api/filings/doc` and `/enrich` take the document, not a symbol (U8).
5. **Notices (book).**
   - **The told scope** `filings:<SYM>:<source>` (`server/feeds.rs:878`) becomes `filings:<source>:<filer>`. The events stay as they are, since they already name no ticker (`filing_mark`, :736: source, date, form, title, size).
   - **Watermarks** are carried to the filer or filers that ticker's stored rows came from, so a filing that arrives during the switch is still told. Where the cache is gone, the new scope's first sweep sets its mark without telling, as any new scope does now (`notify.rs:564–566`).
   - **Release keys** (`release:<SYM>:…`, :1312) are keyed by the listing (section 3), and so are the wire check (U13) and the release scope (U12).

## 3. Listings nobody holds (replaces the plan's open question 1 and the instrument-id keys of §1–2, brief 39 item 3)

**Reading data about a listing never creates a book instrument. The market cache keys what it fetched by the listing itself. So a listing read before it is bought cannot become a second instrument, and Clear data has nothing to reconcile.**

1. **One key type, `ListingKey`.** It is a symbol on a listing venue, and it cannot be built without the venue, as #520's `SecurityId` cannot be empty.
   - The venue is the MIC in Canada and `US` for the US market (`listing_venue`).
   - Coins use `<asset>@CRYPTO` and directory instruments `SPX@INDEX`, as their references already do (`following.rs:121`, :129). An option uses its OCC symbol.
   - The symbol is put in one spelling by one function, so `BRK.B` and `BRK-B` are one key and `BRK` is another.
2. **Every market-reader table and per-listing meta key takes a `ListingKey`.**
   - The tables are `price_history`, `history_fetches`, `price_bars`, `bar_fetches`, `shorts`, `news` and `exposures`. The meta keys are `bars_source:`, `bars_miss:`, `tmx_form:`, `news_fetched:` and the rest of brief 34's list.
   - So do the in-memory maps (U4, U5, U22, U24, U25) and the single-flight keys (`filings:<SYM>`, `filings-shown:<SYM>`).
3. **A book instrument reaches its rows through its current symbol and venue**, by the same function #520's orders use.
   - **A ticker change** gives a new key. The old key's rows are refetchable and age out with the cache, which §6 accepts ("Deleting it loses only time"). Filings are untouched, because they are keyed by filer.
   - **Unchanged:** the sources crate's per-instrument tables (`quotes`, `daily_closes`, `chains`, `reads`, S5), which feed the book's figures and must follow an instrument across a ticker change.
4. **`Book::listing` stays limited to watched listings** (`watched.rs:58`).
5. **The page keys a listing by its `ListingKey`**, which its address already carries (`#markets/listing:SYM@VENUE`), and keys a book row by its instrument id.

## 4. A ticker with no venue (brief 39 item 4)

**No path chooses a listing by ticker. A venue comes from whatever source states it. Where nothing states it, every listing that answers is shown as its own row, and one is opened only when it is the only one.**

1. **Routes take a `ListingKey` or an instrument id, never a bare symbol.** A request that names a symbol without a venue does not parse. Rule test 3 is held by the types, as #520's are.
2. **Sources carry the venue they state:**
   - **Heatmap:** the US and international tiles come from Nasdaq's screener, which lists only US listings, so they are `@US`. The S&P/TSX 60 tiles take the exchange TMX's constituents state (`universes.rs:20`).
   - **Notifications** store the `ListingKey` they are about.
   - **ETF look-through rows** take the holdings file's own exchange column where it has one. Where it has none, the holding's sector and country come from the file's own columns. They are never looked up by ticker (U18, U19).
3. **One resolver for a symbol that comes with no venue.** That covers ⌘K free text, an old link and a news tag with no venue.
   - It answers with every listing of that symbol that the exchanges' directories the app reads (Nasdaq's, TSX's) list, each with its venue, as a quote site's search does. The page shows them as rows.
   - Exactly one answer opens directly. Two or more are never reduced to one.
   - This replaces SPEC markets-60's "the securities the sync brought and then TMX's own resolver … the regulator's own report".
4. **A book instrument whose records state no venue gets no market data from a guessed listing.**
   - Its venue comes from a record that states one: the broker's row when the pull links it (SPEC trade-43), or an `exchange` column added to the plain import layout, as Sharesight takes a market code per trade. Until then its page shows its trades and figures and says its market data waits for its venue.
   - **Goes:** the currency fallback (`venue.rs:12–17`, `market_of_listing` :39, `Listing::market` in `contract.rs:135`). So does TMX's "other forms" retry: a form that answers nothing is that listing's miss, shown as such, because another venue's answer is another listing (U7, `tmx_resolve` :174).

## 5. Still required from brief 39, against this design

1. **Coverage.**
   - Server U10 (`known_filing_symbols`, `held_symbols`) and U21 (an option's underlying breakdown, which takes the underlying's `ListingKey` from the contract).
   - Page U14 (a hand-entered trade names its instrument by id, or by a `ListingKey` from the resolver).
   - The partly covered U7, U8, U11, U17, U18, U19 and page U2, U6, U8, U12.
   - `carry.rs`, the demo book (`write_bars`, `demo_facts.rs:128–170`), and the told marks' `news:<SYM>@<EX>` scope.
2. **Rule test 2's allow-list**, each entry with its reason:
   - `yahoo_miss:<form>` and `coinbase_product:`, because each is the source's own name for exactly one listing;
   - the TSX list and SEC ticker file, because each is a whole-source copy.
   
   Everything else is built through `ListingKey`, or `Filer` for `(source, filer)`.
3. **The plan's text.**
   - List every SPEC row that changes (below).
   - Drop "Nothing else on screen changes".
   - Drop the claim that `identity.rs` has `issuers`.
   - Add `server/src/carry.rs` to the surfaces.

## 6. Tests and acceptance (added to the plan's)

Rule-based, over generated inputs:
- **Filings routing.** The inputs are generated listings, regulator answers and interlisted rows, including names that share words (the `names_match` failure, "Energy Fuels" and "Energy Transfer").
  - A listing's filings are exactly its own filer's documents, plus its partner's where the TSX list pairs them. They are never another filer's.
  - A SEDAR+ answer with zero or two exact names settles nothing, and the page says so.
- **The resolver.** For a symbol with k listings across venues, it answers k rows, each with its venue. One row opens directly; it never picks one of two or more.
- **A book instrument with no venue.** For generated instruments, no market reader is called and no market-cache key is written.
- **Migration v6.**
  - Every attributable filings row is carried with its read text byte for byte.
  - The dropped count equals the rows with no readable filer.
  - Duplicates keep the most complete read.
  - Run on a copy of the owner's cache, the counts are reported in the PR.
- **Told marks.** Across the migration, nothing is told twice and no filing that arrives during the switch is left untold.

Recorded and reported in the PR:
- **SEDAR+:** its answers for the S&P/TSX 60 (section 1, rule 2).
- **The TSX list:** the US symbols the SEC file resolves (189 of 195 today); the other six show their SEDAR+ filings only.
- **Cost:**
  - the TSX list, 15 KB once a day;
  - the SEC file, read once per process as now, 275 KB less than today;
  - one SEDAR+ search per Canadian listing whose profile is not yet settled, as today.

## SPEC and architecture changes in the same PR

- **market-data-17.**
  - The SEDAR+ profile is the one whose legal name is the issuer name the listing's exchange gives; none or two is said, never picked.
  - A US listing's CIK comes from the SEC's ticker and exchange list.
  - "A cross-listed issuer draws from both" becomes: a listing the TSX's interlisted list pairs with a US listing draws from both regulators.
- **market-data-4.** The form answering nothing is that listing's miss. The other-forms retry and the "venue is missing … starts from its currency's usual form" sentence go.
- **trades-13.** The "follows its currency" clause goes.
- **trades-37.** It describes today's bars "stored under the bare ticker"; that sentence goes, because they no longer are.
- **markets-60.** Rewritten to section 4, rule 3.
- **`docs/architecture.md` §5.** Under Issuer: two listings in Canada and the US are one issuer where the listing exchange's interlisted list pairs them. That is the exchange's own statement, kept beside the strong identifiers, and it never merges instruments.
