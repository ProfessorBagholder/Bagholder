# Brief 35: verdict on plan #519, an order names its listing (`plan-orders-listing` at `1a8d603d`): Go with changes

**Reviewed:** `docs/plans/orders-name-the-listing.md` against brief 34 section 1 and master `f89634de`. Read: `orders/ticket.rs` (`resolve_security`, `parse_listing_search`, `lookup_listing`, `ticket_quote`), `orders/mod.rs`, `http/markets.rs` (`/api/symbols/search`), `web/src/lib/ticket/ticket.svelte.ts`, `FilterPopover.svelte`, `TradeDetail.svelte`, `reads.svelte.ts`. The plan's three sources re-read on 2026-10-15:
- **Interactive Brokers:** an order's body requires the instrument's `conid` ("First we must have obtained IB's conid for AAPL stock, trading in the US in USD").
- **Questrade:** an order names `symbolId`, an "internal symbol identifier".
- **Wealthsimple:** an order names `securityId`, as the app already sends it.

## Verdict

Go with changes. The design is the established one, and it removes the bug at its root:
- one listing type, with Wealthsimple's id required;
- the server taking the listing by id only;
- the symbol branch, `tkLookup`'s match by symbol and the symbol-keyed cache all deleted.

Seven changes, none of which alters that design.

## Required changes

1. **Say what the two kinds of listing are, and never call one "lacking an id".**
   - Every place a ticket opens from has an exact listing. It is either the book's instrument, or a listing an exchange's own directory gave: a symbol on a named venue, which is unique on that venue.
   - Turning the second into Wealthsimple's id is looking up a reference by an exact identity, as Interactive Brokers needs its `conid` before an order. It is not a fallback from a ticker.
   - Make that the type. Replace `ticketOn(of, side)`'s loose `of` with `ListingRef = Instrument(InstrumentId) | Venue { symbol, venue }`, the venue required and never empty. No variant holds a symbol alone.

2. **Match the venue through the app's one venue table, not as a raw string.**
   - `parse_listing_search` compares `stock.primary_exchange.to_uppercase()` with the exchange string it was given (`orders/ticket.rs:375`).
   - The rows from outside the book come from the exchanges' own directories (`/api/symbols/search`, `http/markets.rs:61`), whose venue names are not Wealthsimple's. Where the two vocabularies differ, a real listing is refused.
   - Wealthsimple answers `primaryMic`. Compare MICs, with both sides mapped through `sources/src/venue.rs`.
   - The same function takes the first match (`find_map`). The plan's "two found is a refusal" needs the search to count matches.
   - Test it over every venue the directory search returns.

3. **A Wealthsimple id found for a book instrument is recorded against it.** `{ instrument }` searches when the broker has stated no id. Per §5, a source's identifier is "recorded against it as references … learned and remembered". Record it in the book as the instrument's reference, not only in the found cache, so the next order reads it from the book.

4. **Drop every draft of the old shape.** Attaching one "to the one listing the book holds under that symbol" is a symbol lookup in the order path, the thing this plan deletes. A draft is unsent typing; losing one costs nothing.

5. **A listing page with no venue offers no Buy.** The page reviewer found that notifications and the US heatmap tiles open a listing page with an empty exchange (brief 34, page U6). Until brief 34 section 2 gives those pages their listing, their Buy is not offered: there is no exact identity to place an order on. State this in the plan and hold it in the e2e test.

6. **The e2e book contradicts itself.** The criterion holds "two listings with one ticker … each held", then opens "the listing's page for the one not held". Hold one listing and add the other only as a directory row. That is the bug as found: RY held on the TSX, with Buy from the NYSE row or its page. Keep both held for the Sell entry points.

7. **Correct the Questrade citation.** Its search example's second result is `BMO.PRJ.TO`, symbolId 9300: a preferred share of the bank on the TSX, not the common stock on a second venue. The point stands, since every security has its own id, but the sentence describes the example wrongly. The Interactive Brokers page cited says the `conid` must be obtained first. It does not mention `/iserver/secdef/search`, so cite the page that does, or drop it.

## Accepted as is

- `OrderListing` with `security_id` required and non-empty, generated to the page.
- `Ticket.symbol` removed, and `deny_unknown_fields` on.
- `listing(app, security_id)` reads, in order, the book, the cache by id, then Wealthsimple's own summary by id.
- The cache is keyed by id and by the exact (symbol, venue).
- The refusal in the ticket's words when a venue holds no such listing.
- The server property test over every kind of collision pair, in both orders of reading, through quote, preview and a dry order. Add the pair from change 6: one listing in the book and one only outside it.
- The resolver tests and the draft test.
- The `SPEC.md` rewording.
- The entry in `docs/old-app-mistakes.md`.
- Building this alone, before brief 34's sections 2 and 3.
