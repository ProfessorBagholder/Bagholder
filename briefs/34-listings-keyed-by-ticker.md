# Brief 34: listings keyed by ticker — an order can go to the wrong listing, and 37 places mix listings that share a ticker

**Asked (owner, 2026-10-15):** "obviously mixing up tickers is totally unacceptable and should never have happened let alone left broken after a major refactor".

**Method.** Two readers listed every place, at master `f89634de`, that stores, caches, looks up, matches or requests a listing's data by its ticker instead of the listing itself: one on the server, one in the page. Each place came with its key, its store, the pairs of listings that collide on it and what is shown when they do. I read the order path, the daily history, the TMX form cache and the filings tables myself. The inventories are in [34/server.json](34/server.json) (23 unsafe, 2 unsettled, 16 safe) and [34/page.json](34/page.json) (14 unsafe, 11 safe).

## The rule this breaks

`docs/architecture.md` §5 already says how it must be:
- "Every instrument has a Bagholder id that never changes."
- What it is called, its symbol and venue, is an attribute.
- Each source's identifier (a Yahoo symbol, a TMX form, a SEC CIK, a SEDAR+ profile) is recorded against it as a reference.
- Filings, releases and news belong to the issuer: "a company listed in Canada and the US is one issuer with two listings, each with its own price, position and short interest".

The section's *why* names this very mistake in the old app: "the old app used … bare symbols as identities … So a share could take a coin's price."

The record, the engine and the new sources crate follow the rule. The market readers and parts of the page were carried over from the old app unchanged (stage 6 moved where their tables live, not how they are keyed), and the rule was never applied to them.

## Which listings collide

The code's own comments name the real cases:
- **The same ticker in Canada and the US.** It can be the same company (RY) or another company: `market/src/quotes.rs:243–247` names `PLTR.TO`, "Palantir's Canadian depositary receipt, not the stock".
- **A suffix stripped.** `tmx_symbol` drops `.TO`, `.V`, `.CN` and `.NE`.
- **An option and its underlying.**
- **A coin and a share with the same symbol.**
- **A ticker reused over time.**

TSX and TSX-V share one set of symbols (`sources/src/venue.rs:172–176`), so they never collide with each other.

## 1. Orders: first, on their own

**An order can be placed on the other listing with the same ticker.**
- **The page.** Buy from a listing's page (`TradeDetail.svelte:150`) or a search row (`FilterPopover.svelte:494`) passes no security id. `tkLookup` then takes the first holding or instrument with the same ticker (`ticket/ticket.svelte.ts:39–61`).
- **The server.** `resolve_security` (`orders/ticket.rs:106–145`), given no security id, takes the first instrument in the book with that symbol on any venue. It uses the exchange only when nothing matches. The cache of listings found by Wealthsimple's search (`orders/mod.rs:89`, `found`) is keyed by symbol alone, and `POST /api/order` carries no exchange.
- **The draft.** A saved ticket draft is restored by symbol and side, with no security id (`ticket.svelte.ts:180–198, 215`). So one listing's account, quantity, limit and bracket prices refill the other's ticket.
- **What shows.** Holding RY on the TSX and pressing Buy on the NYSE RY page opens a ticket for, and places an order on, TSX RY. The only sign is the estimated cost in CAD.

**Required:** the listing is decided where the ticket is opened, and never guessed.
- **Every entry point has the listing.** A ticket opens only from a listing that is already known: a holding, a listing's page, a search result, the palette or a notification. Each of these has the listing's security id, and each passes it.
- **The ticket keeps that id.** The ticket and its draft hold it, and a draft is restored only for the same id.
- **The server's guess is removed.** Today, without an id, it picks the first instrument with that symbol. That is the bug. The found-listings cache keyed by symbol alone goes with it.
- **The server rejects a request without an id.** Once every entry point passes it, this cannot happen through the app. It is a guard at the API boundary, so a future bug fails safe instead of guessing; it is never a state the person meets.
- **A test over every entry point.** It holds that each one opens the ticket on the listing it was opened from, for two listings that share a ticker.

This is a fix to shipped behaviour that places real orders, so it goes first, as its own PR.

## 2. The market readers: keyed by listing, filings by issuer

Unsafe on the server, each with the listing's exchange and currency already in hand where the key is built:

| Kind | Places (34/server.json) | What shows when two listings collide |
|---|---|---|
| Daily history, intraday bars, their fetch stamps, misses, pending reads and notes | U1–U5 (`market/src/history.rs`; `store/src/market.rs:217–396`; `INSERT OR IGNORE` keeps the first listing's days) | one listing's chart shows the other's prices, in the other currency; refreshes then mix the two; its own read never runs |
| The source a chart was read from | U6 (`bars_source:<SYM>`) | a chart read from another Canadian venue's listing |
| TMX's form for a ticker, remembered for good | U7 (`market/src/tmx.rs:152–212`, `tmx_form:<bare>`: the remembered form wins over the venue the caller asked for) | every listing with the ticker is read as the first one: TMX bars, news, sector, volume |
| Filings, their sources and read stamps, the SEDAR+ profile, the issuer name searched, told marks | U8–U11, U14 (`store/src/feeds.rs:502–760, 1366–1395`, `filings WHERE symbol = ?`; `sedar_profile:<SYM>`, kept for good) | TELUS's and AT&T's filings in one list (both T); another issuer's SEDAR+ filings indefinitely |
| Release notices | U12, U13, U15 | notices for, or suppressed by, the other listing; another payer's distribution in a notice |
| A listing page opened from a ticker | U17 (`feeds.rs:2626–2725`) | another venue's holding opens; fills of both are merged |
| Sector, country and fund breakdowns | U18–U21 (`share:<SYM>:<form>` with no venue taken as the TSX form; `fund:<SYM>`; an option's underlying falling back to the other market) | wrong sector and country; another fund's breakdown |
| The bar archive, the news "Reading…" mark, the short-interest failure line | U22, U24, U25 | one listing per ticker archived; a wrong wait word or failure line |
| Unsettled | U20 (Canadian fund ticker maps), U23 (CIRO rows) | to be settled in the plan |

Unsafe in the page (34/page.json): the listing page's id with no currency or instrument (U6), disclosures keyed by bare ticker (U2), and matches by ticker in the watchlist, news chip, held marks and shorts lookup (U7–U13). Each needs the id it already has, or the server returns one.

**Required:**
- **Keys.** Every per-listing store in the market cache, and every in-memory map, is keyed by the instrument id: price history, bars, stamps, misses, pending reads, notes, the chosen source, the TMX form, short interest, the archive, wait marks and failure lines.
- **References.** A source's form for a listing is a reference recorded against that listing (§5), never a value shared by ticker.
- **Filings.** Filings, releases and news read-states are keyed by the issuer, with the SEC CIK and the SEDAR+ profile as the issuer's references. So the two listings of one company share them, and two companies with one ticker never do.
- **Page requests.** Every page request names the listing by id.
- **Existing rows.**
  - The market cache's ticker-keyed rows are dropped and read again. §6: "Deleting it loses only time."
  - The told marks in the book are carried to the listing or issuer their ticker named where the book holds exactly one.
  - Where a ticker is ambiguous, its told marks are kept for each candidate, so nothing is told twice.

## 3. The rule, held by tests

These tests hold the rule itself, not single examples:
1. **A property test over every reader with a per-listing store.** For generated pairs of listings that share a ticker (two venues, a stripped suffix, a coin and a share, an option and its underlying), the data stored, cached or shown for one is never the other's, in either order of reads.
2. **A schema test.** No table in the market cache, and no meta key, is keyed by a ticker alone. Every per-listing table has the instrument id, or the issuer id, in its key.
3. **A boundary test.** No route the page calls resolves a listing from a ticker when the book holds more than one listing with it. It refuses instead.

Each test must be shown failing on today's code.

## Order of work

1. **Section 1, the orders, as its own PR now.** A plan from `PLAN.template.md` with its criteria first, as `CLAUDE.md` requires.
2. **Sections 2 and 3, as one plan:** the market readers and the page keyed by listing and issuer, with the three rule tests. This is the work the stage 6 cutover left out.
