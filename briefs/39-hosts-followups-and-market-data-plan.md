# Brief 39: #521 (brief 38's follow-ups): merge after two changes. Plan #522 (market data by listing, at `9642f1e9`): not yet

## #521 at `f03a9ad2`

**Reviewed:** the diff against master `7e0782f1`. One reader ran the suites and undid each change in a throwaway copy, and I checked the findings in the code. CI is green on all eight checks.

What was run, all passing:
- `bagholder-net` (24), `bagholder-ws` (14);
- the server boundary and order tests (75);
- `npm run check`, `npm test` (148);
- the signed-in e2e (6), plus ticket, orderspanel and dryorder (55).

**What holds:**
- **One gate names, counts and refuses every request.** That covers the client on every redirect hop, `Net`'s direct and offline paths, the browser helper's request, both Wealthsimple clients, the updater, the local model, EDGAR and Google News. When the app is not offline, behaviour is unchanged. The cost is one flag read, plus a log line only with `BAGHOLDER_LOG_REQUESTS=1`.
- **The signed-in e2e reads the server's own record of every attempt.** It caught a real GraphQL address hard-coded in the newer client.
- **The e2e homes are removed when a run ends.**
- **The preview test now has a USD rate.** It asserts the converted figure (1,387.10), and removing either the conversion or the listing's-currency override fails it.

**Required changes:**
1. **The older client's real addresses can still be used directly, and nothing would notice.**
   - The scan allows `const GRAPHQL`, `const OAUTH` and `const WEB` at the top of `ws/src/session.rs` (`tests_boundary.rs:285–287`). Any function in that file can read them without going through `BAGHOLDER_WS_BASE`. The comment says "each file's own test holds that"; no test does.
   - Reproduced: the order-placing call (`session.rs:740`) pointed at `GRAPHQL`. The host scan, the `ws` tests, the e2e (where orders are dry) and the full server suite all pass. With dry orders off, a real order would go to Wealthsimple from a test server.
   - **Required:** move each address into the one function that honours the variable, as a local constant, so nothing else can name it. The scan then allows those functions, not the lines.
2. **The e2e's assertion is weaker than its title.** It checks that no Wealthsimple host appears, while the title says requests go "only to the stand-in". Assert that every Wealthsimple call went to the stand-in's address. Also remove the request log the run leaves in `/tmp` (`playwright.config.ts:13`).

**Smaller, for the record (not blocking):**
- **Some requests leave without passing the gate.** The Chrome sign-in opens Wealthsimple's page outside it (`login.rs:1201`, :1208), as does the updater's `git pull` (`update.rs:802`). Redirects inside the browser helper are named only at their first address.
- **Some readers refuse before the gate.** Offline, the browser-session readers (SEDAR+, the Yahoo float and quoteSummary, Google News articles) refuse before the gate with "the browser helper is not installed", which is the wrong reason. The commit message says every refused request is now named; that holds only for `Net`.

## Plan #522 at `9642f1e9`

**Superseded by brief 40**, which gives the design for each problem below. Read brief 40, not this section.

**Reviewed:** the plan against brief 34's inventories, `docs/architecture.md` §5 and §6, `SPEC.md`, and master `7e0782f1`. One reader checked every claim the plan makes about the code, and I checked the findings. The update at `9642f1e9` changed only how two listings are found to be one company. Everything below except item 1 applies to both versions.

### Verdict

**Not yet.** The direction is right: market data per listing, filings per company, and the three rule tests. But:
- the cross-market join it settled on contradicts §5;
- it moves filings out of the store §6 puts them in, without saying what happens to their read text;
- its main mechanism makes duplicate instruments;
- it leaves several of brief 34's places uncovered.

### Required changes

1. **No join by name.**
   - **What §5 says:** "Two sources' instruments are the same instrument only on a strong identifier (ISIN, CUSIP, FIGI, the OCC symbol, or the same broker's own id) … Anything weaker stays a separate instrument until the person links them."
   - **What the update does:** it joins a Canadian and a US listing with the same ticker root when the regulators' names pass `names_match`. That rule (`market/disclosures.rs:67–74`) is a word overlap: half of the shorter name's words, after "Inc", "Corp", "Ltd", "Holdings" and the like are dropped. For two-word names one shared word is enough; "Energy Fuels" and "Energy Transfer" match. A wrong join shows another company's filings on a listing, which is the bug this plan exists to remove. Decided again "whenever either name is read", it can also change over time, moving filings and their told marks between companies.
   - **Required:** two listings are one company only on a strong identifier. Without one, each listing shows its own regulator's filings.
   - **This is the owner's decision.** SPEC market-data-17 says "A cross-listed issuer draws from both", and no source the app reads ties a Canadian listing to a US one. Keeping that line needs a source that does; OpenFIGI's share-class FIGI is "assigned to an instrument that is traded in more than one country" ([OpenFIGI](https://www.openfigi.com/api/documentation): free, no key, 25 requests a minute). Dropping the line means each listing shows its own regulator's filings, always correct and less complete.
   - **Required, either way:** put this back at the top of the plan with both options stated, and change SPEC market-data-17 to match the answer.
   - Separately, the plan's account of today's rule is wrong. `edgar.rs:296–303` matches a non-US listing to an SEC filer by its bare ticker, with that word overlap as a guard, and skips the guard when the name is empty. It is not an exact legal-name match.

2. **Filings stay in the market cache, and their read text is carried.**
   - **The conflict:** §6 puts "news, filings and their read text" in the market cache, and that is where they are (`sources/schema/v5.sql:65–86`). §3 of the plan says "Filings move into the book's company scope … (book migration v30)". Rule test 2 expects per-company rows in the market cache, and the v6 drop list leaves `filings` out.
   - **What dropping them costs:** each filing's read text means a document fetched again plus up to two local-model calls on a Pi (`enrich.rs:300`, :374). The reader has no bound (#448). SPEC says a filed document "is kept" (market-data-17), and the reading "never discards what was read" (disclosures-29).
   - **Required:**
     - Filings stay in the market cache, keyed by issuer id. The issuer and its references live in the book.
     - Carry existing rows and their read text to the issuer wherever the stored rows name it unambiguously. Drop only the rows of a ticker that named two companies.
     - §5 lists the SEC CIK and the SEDAR+ profile as instrument references today (`core/src/instrument.rs:106–107`). Change §5 in the same PR.

3. **A listing read before it is bought must not become two instruments.**
   - **The mechanism:** open question 1 widens `Book::listing` to every listing whose data is read. That function is private, and it finds by references, else by current name, else makes one new (`book/src/watched.rs:58–96`). The `<symbol>@<MIC>` reference is made by its caller, `following.rs:143`, and no record path reads it.
   - **The duplicate:** a listing read first, from a news chip or a directory row, without Wealthsimple's id, becomes an instrument. When it is later bought, the pull makes a second one. A draft carrying both references is then refused (`watched.rs:60–65`).
   - **Required:**
     - Name which reads make an instrument: listing pages, charts, short interest, filings, news chips. Glance quotes and heatmap tiles do not.
     - A record's instrument adopts one a read made for the same `<symbol>@<MIC>`, with a test.
     - Clear data, which deletes unnamed instruments on every run (`book/clear.rs:235–261`), also clears or re-keys their market-cache rows.

4. **A ticker with no venue.** SPEC markets-60 settles a venue-less ticker from "the securities the sync brought and then TMX's own resolver", which picks by ticker. The plan's rule test 3 refuses instead. **Required:**
   - Settle it only when exactly one listing answers. Where two do, show each as its own row with its venue.
   - Rewrite markets-60 to say so.
   - US heatmap and international tiles (the Nasdaq screener gives no exchange, `universes.rs:61–76`) take the US market as their venue, as #520 does.

5. **Cover every place in brief 34's inventories.**
   - **Not covered:**
     - server U10 (`known_filing_symbols`, `feeds.rs:812`; `held_symbols`, `model/src/symbols_of.rs:21–24`);
     - server U21 (an option's underlying breakdown, `model/src/exposure.rs:110–116`);
     - page U14 (a hand-entered trade sends `instrument: null`, `entries.rs:106–111`).
   - **Partly covered:**
     - U7: `news.rs:684`, :1852, the search fallback, and the direct `tmx_resolve` calls at `feeds.rs:637` and :2473;
     - U8: `looking_at` and `reading_now`, `feeds.rs:37–39`, and the single-flight keys `filings:<SYM>` and `filings-shown:<SYM>`;
     - U11 inside `refresh_filings_in`;
     - U17;
     - U18 and U19: look-through holdings with no venue, and `watch_exposure_key`;
     - page U2, U6, U8 and U12.
   - **Also:**
     - the demo book: `write_bars`, `demo_facts.rs:128–170` and `carry.rs`;
     - the told marks' `news:<SYM>@<EX>` scope.

6. **Say how the meta keys map.**
   - `cache.won` and `cache.winner` are keyed by a `DataKind` with no bars or timeframe (`sources/src/contract.rs:14–21`); state the new kinds.
   - The sources crate writes a `TmxForm` reference only for the venue's own form (`quotes.rs:229–230`). The resolved other form, which `tmx_form:` remembers, is not written; state the change.

7. **Rule test 2, as worded, fails on keys the plan keeps:** `yahoo_miss:`, `coinbase_product:`, `coinbase_prev:`, `news_fetched:`, `news_source_fetched:`, and `carry.rs:97–100`. **Required:** for each, either re-key it per listing, or name it in an allow-list with its reason, such as "the source's own name for one market" (`coinbase_product:`).

8. **Smaller.**
   - "SPEC changes in one place" and "Nothing else on screen changes" do not hold; list every SPEC row this changes.
   - `identity.rs` has no `issuers` function, and `attach_issuer` refuses a move (:207–218). State what the join is built on.
   - Add `server/src/carry.rs` to the surfaces.
