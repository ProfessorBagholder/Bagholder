# Brief 36: the order-listing build as built (#520, `orders-name-the-listing` at `ec84cf47`): not yet

**Reviewed:** the build of plan #519 against brief 35's seven changes and brief 34 section 1, on master `a56126d1`. Two readers took the server side and the page side. Each ran the suites, undid each fix in a throwaway copy, and tried to break what the tests leave open. I read the server's order path myself, and checked the defects below in the code and in Wealthsimple's recorded replies. Copies and build folders are deleted.

Suites on the PR code, all passing:
- server: 83 order tests and the golden wire test;
- page: `npm run check`, `npm test` (144 tests), and the `listingorders`, `ticket` and `orderspanel` e2e specs (59 tests).

## Verdict

**Not yet.** The core is right and holds:
- no order path chooses a listing by ticker;
- the found cache is keyed by id;
- the search counts its matches and refuses none or two;
- a found id is recorded against the book's instrument;
- old drafts are dropped.

Undoing each of those fails a test, with one exception: the page's lookup by ticker, plant (a), is caught by no test. But two defects stand in what was built:
- **A real listing is refused.**
- **A listing page opens a ticket on a listing found by its ticker.**

Several tests also hold less than they appear to. Seven changes are required.

## Required changes

1. **Buy from a search row is refused for every NYSE Arca and NYSE American listing.**
   - **What happens:** the directory now reads Nasdaq's `PSE` and `AMEX` codes as ARCX and XASE (`market/src/search.rs:19–22`; `tmx.rs:44–56`). But Wealthsimple states those listings as `"primaryExchange":"NYSE","primaryMic":"XNYS"`. Its recorded replies do so for VTI and GLDM (`wealthsimple/tests/replies/wealthsimple/positions@anon-personal-24@2025-05-14.json`). Reproduced: "Wealthsimple lists no VTI on ARCX." and "… no CTM on XASE." It covers SPY, VTI, GLDM, MSTY and CTM.
   - **Knock-on effects:**
     - A held VTI is no longer dropped from the search rows as the same listing (SPEC filters-24), so a duplicate row appears whose Buy is refused.
     - A NYSE American or Arca row whose TMX quote states no currency is given CAD (`tmx.rs:277`).
   - **Why the test missed it:** `every_venue_a_directory_names_is_read_as_its_own_code` checks the table against itself, and the collision test feeds Wealthsimple rows the directory's own codes (`tests_orders.rs:231`, :249).
   - **Required:**
     - Match on what both sources state. For US listings that is the US market, since Wealthsimple states the NYSE group as XNYS. For Canada it is the venue, as now. The count rule still refuses two matches.
     - Test with the recorded Wealthsimple replies against real Nasdaq directory rows (`PSE`, `AMEX`).
     - Fix the duplicate-row rule and the currency default the same way.

2. **A listing page with no venue offers Buy on a venue the server found by ticker.**
   - **What happens:** `TradeDetail.svelte:151` checks `trade.exchange`. But `take()` fills that field from `/api/listing`'s answer (`listing.svelte.ts:122–125`), and the server fills an empty venue from a trade or watched row with the same bare ticker, or from the first security with it (`feeds.rs:2699–2704`).
   - **Reproduced:** `#markets/listing:UBER@` showed a Buy and sent `{"kind":"venue","symbol":"UBER","venue":"NYSE"}`. The US heatmap and notifications open such pages. A US `T` tile in a book that traded TELUS `T` opens a ticket on TELUS. This is the bug the PR exists to remove.
   - **Required:**
     - The page's Buy uses only the venue in the page's own id (`listing:SYM@VENUE`).
     - `take()` never writes `exchange` or `securityId` onto a page whose id named no venue.
     - No redirect to a holding page by ticker for such an id (brief 34, U17).
     - An e2e test on a ticker the demo book has traded. ZZQ, which the test uses, is unknown to it.

3. **Selling from a holding with no recorded Wealthsimple id opens with the wrong account and quantity.**
   - **What happens:** `ticket.svelte.ts:50` requires the holding id passed and a match of the security id. The position's security stays empty until the newly recorded id reaches the page.
   - **Reproduced:** with TD's security cleared, the Sell opened at 1 share in `acct-trading` against a holding of 150 in `acct-tfsa`. SPEC: "a Sell goes where the shares are and sells them all". Master matched the holding by its id alone.
   - **Required:** match the holding by the id passed, alone.

4. **The order records the page's symbol and currency for an id neither the book nor the cache holds.**
   - **What happens:** `ticket_order` (`ticket.rs:762–768`, recorded at :801–810) builds the listing from the page's fields. The cache is in memory, so this is the path after a restart, or for a resumed draft. The order goes to the right listing, which is named by id, but its record can be wrong. Reproduced: Wealthsimple states RY in USD, the page says TD in CAD, and the order is recorded as TD in CAD. A fill then creates the book instrument with those words (`book/src/fills.rs:83–93`). `preview_for` likewise uses the page's quote currency (`preview.rs:200`).
   - **Required:** read Wealthsimple's own summary by id, as `ticket_quote` already does. Take the symbol and currency from it, and refuse where Wealthsimple does not know the id. Plan #519 said exactly this.

5. **The listing is not required by the request types.**
   - **What happens:** `Ticket` and `PreviewRequest` carry `#[serde(default)]` (`ticket.rs:688–689`, `preview.rs:21`), and `ListingOf.of` is an `Option`. So a request with no listing parses, and is refused at run time ("No listing named"). That is the "missing" case the owner ruled out.
   - **Required:**
     - A `SecurityId` type that cannot be empty, parsed at the boundary.
     - The listing a required field of `Ticket`, `PreviewRequest` and `ListingOf`, with no container default on them.
     - Then the run-time "no listing" branches go.

6. **A late answer replaces a ticket opened since.** `ticketOn` waits on `/api/order/listing` (`ticket.svelte.ts:227–234`). Reproduced with the answer delayed: a Buy on the NYSE row, then TSX TD opened from ⌘K with 5 typed; when the answer came, the ticket became NYSE with 1, and no draft was kept. **Required:** drop an answer when any ticket opened or closed after the request, as `refreshPreview` does with `asked`.

7. **The e2e test does not test the server's side of the pair, and asserts too little.**
   - **What happens:**
     - The NYSE holding is stood in on the page's event stream (`listingorders.spec.ts:16–24`), so the server never holds it.
     - `/api/order/listing` is stood in.
     - Only the quote key and the listing request are asserted, never the account, the quantity, or the preview and order bodies. Plant (a), `tkLookup` matching by ticker, is caught by no test; the twin copies the TSX quantity, so wrong defaults cannot show.
   - **Why it was stood in:** the PR says the demo book keys its listings by ticker. It does: `demo_book.rs:154–156` makes `sec-`+ticker, every row carries it (:210), and `demo_facts.rs:131–135` reads quotes `WHERE symbol = ?`.
   - **Required:**
     - Key the demo book's listings by symbol and venue, and hold TD on the NYSE in it for real, at a different quantity.
     - Assert the account, the quantity, and the preview and order bodies for every entry point.
     - Stand in only Wealthsimple's search, which offline mode cannot answer.
     - Add the demo book to brief 34's sections 2 and 3, whose inventories missed it.

## Smaller, in the same PR

- **An id another book instrument already holds is refused in raw words.** The search finds an id another instrument already holds, and `add_instrument_ref` then refuses with "broker-security:wealthsimple sec-… already names instrument …" (`ticket.rs:243`; `identity.rs:391`). The id names the listing: open the ticket on it, and record nothing. Also record an id only where the listing found is a share or ETF in the instrument's own currency.
- **A refusal names the venue by its code** ("on XNYS", `ticket.rs:500`). Use the venue's words.
- **SPEC filters-21** still says Nasdaq's directory covers "NYSE, NASDAQ and BATS".

## Accepted as is

- `OrderListing` and `ListingRef` with the venue required.
- `known_listing` by id only, and `found_at` keyed by (bare ticker, MIC) and read only so.
- The count rule. A venue the table does not know is a refusal.
- An id found for a book instrument recorded and the figures told.
- Old drafts dropped; a draft restored only for the same id and side.
- Every ticket opened through `openTicket(OrderListing)`, with no symbol path left.
- A directory row with no venue offering no Buy.
- The server collision test over 72 venue pairs, suffixes, options and coins.
- The SPEC rewording. SPEC trade-57 already names NYSE American and NYSE Arca.
