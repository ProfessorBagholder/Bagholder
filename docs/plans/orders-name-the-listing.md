# Plan: an order names its listing at every step, never a ticker

Brief 34, section 1. An order can be placed on the other listing with the same ticker: Buy on a listing's page, or on a search row, opens a ticket for whichever holding or instrument the book holds under that ticker, and the server, given no listing, takes the first instrument with the symbol. This plan makes the listing part of the order's type from the button pressed to the request Wealthsimple is sent, and deletes every path that finds a listing from a ticker. It is a fix to shipped behaviour that places real orders, so it is built alone, before the market readers (brief 34, sections 2 and 3, a plan of their own).

## For the owner to decide

Nothing open.

## Scope

**In scope:**
- **The order ticket on the page.** Opening it, its state and its draft.
- **The ticket's live quote document.**
- **The order requests.** `POST /api/order/preview` and `POST /api/order`.
- **The server's resolution of a listing:** `resolve_security`, `lookup_listing` and the `found` cache.
- **The entry points:** a holding's or trade's page, a listing's page, a ⌘K row (from the book and from outside it), and the Orders panel's draft card.
- **`SPEC.md`:** order-ticket-2, -3 and -4 and orders-6 say "symbol" where they mean the listing.

Nothing on screen changes, except that a ticket now always opens on the listing it was opened from.

**Out of scope, on purpose:**
- The market readers and the page's other ticker keys (charts, history, filings, news, breakdowns, watchlist): brief 34, sections 2 and 3, next. That plan also covers the made-up book's own market data (brief 36, 7). `demo_book.rs` writes one price history per ticker, so a second listing of a ticker (`TD@NYSE`) gets none. `demo_facts.rs` quotes from it by ticker, so such a listing is quoted from its own last fill instead.
- Orders already placed, brackets, and the read-back. They already name the listing by Wealthsimple's id (`OrderRequest::broker_security`, `gate.rs:281`); nothing in them is keyed by ticker.

## The old app here

The old app opened its ticket from a symbol and looked the listing up by that symbol (`ledger.html`'s `tkLookup`). The Svelte port carried `tkLookup` over unchanged (`web/src/lib/ticket/ticket.svelte.ts:39–61`). The Rust server carried over the symbol fallback (`orders/ticket.rs:106–145`) and the symbol-keyed search cache (`orders/mod.rs:89`). That is the mistake `docs/architecture.md` §5 names ("bare symbols as identities"). Nothing of it is carried over. A new entry in `docs/old-app-mistakes.md` records it, with the test that guards it.

## How the leading products do it

- **Interactive Brokers** (Web API, "New Order Example", read 2026-10-15, https://www.interactivebrokers.com/docs/web-api/trading/orders/new-order-example). An order names its instrument by `conid`, IBKR's contract id, which must be obtained before the order is placed. The order body has no symbol.
- **Questrade** (API, "accounts/:id/orders" and "symbols/search", read 2026-10-15, https://www.questrade.com/api/documentation/rest-operations/order-calls/accounts-id-orders, https://invest.questrade.com/api/documentation/rest-operations/market-calls/symbols-search). An order names `symbolId`, Questrade's "internal symbol identifier": every security has its own, and each search result states its `listingExchange`.
- **Wealthsimple** (its own GraphQL, as the app already calls it). An order names `securityId`, one id per listing: `sec-s-…` for a share, `sec-o-…` for a contract. Search (`FetchSecuritySearchResult`) returns each listing with its `primaryExchange`.

All three key an order by the listing's id and take the symbol as display. This plan does the same. The listing's id is Wealthsimple's `securityId`, the broker's reference recorded against the instrument (§5).

## Open questions

None. Every place a ticket opens from has an exact listing, of one of two kinds (brief 35, 1):
- **The book's own instrument,** by its `InstrumentId`. Wealthsimple's `securityId` is recorded against it as a reference where the broker has stated one (`Names::security`).
- **A listing an exchange's own directory gave:** a symbol on a named venue, which is unique on that venue (`/api/symbols/search`, `http/markets.rs:61`).

Turning either into Wealthsimple's id is looking up a reference by an exact identity, as Interactive Brokers needs a `conid` before an order. It is never a fallback from a ticker.

## Approach

**One type for the listing an order is on.**
- `OrderListing { security_id, symbol, exchange, currency }` in `rust/crates/server/src/orders/ticket.rs`, generated to `web/src/lib/generated/orders.ts`.
- `security_id` is Wealthsimple's id and is never empty; the others are what the ticket shows.
- Every request that names a listing carries `security_id` as a required field: `Ticket`, the quote document's parameters and `PreviewRequest`. Each refuses a request without it (`deny_unknown_fields` is already on). `Ticket.symbol` is removed.

**One way to get one.** `POST /api/order/listing` takes a `ListingRef`, one of two variants, neither holding a symbol alone (brief 35, 1):
- `Instrument { id }`: the book's own. Its answer is Wealthsimple's id from `Names::security`. Where the broker has stated none (an instrument only entered by hand or imported), Wealthsimple's search is asked with the instrument's current symbol and venue (`InstrumentInfo::current_name`). The id it finds is recorded in the book as the instrument's reference (`broker-security:wealthsimple`), so the next order reads it from the book (§5; brief 35, 3).
- `Venue { symbol, venue }`: a listing a directory gave. The venue is required and never empty.

The venue is matched as a market identifier code, never as a raw string (brief 35, 2). The directory's venue words and Wealthsimple's `primaryMic` are both mapped through the app's one venue table (`sources/src/venue.rs`, `mic_of`). The bare symbols must be equal and the MICs equal. A venue the table does not know is a refusal. The search counts the matches: none, or more than one, is a refusal in the ticket's words ("Wealthsimple lists no RY on the NYSE."), never a choice.

It answers the `OrderListing`. The cache of what the search found is keyed by `security_id` and by the exact (symbol, MIC) asked, never by symbol alone.

**The server takes the listing as given.** `resolve_security` becomes `listing(app, security_id)`. It reads the book's instrument for that id where the book holds it, else the cache by id, else Wealthsimple's own summary of the security (`fetch_quotes` already reads `SummarySecurity` by id: symbol, exchange, currency). The symbol branch and the "first instrument with that symbol" choice are deleted. `ticket_quote`, `preview_for` and `ticket_order` call `listing` with the request's `security_id`.

**The page holds the listing.**
- `openTicket(listing: OrderListing, side, holding?)`. A caller that holds an `OrderListing` (a holding or trade with Wealthsimple's id on its row) passes it. Every other caller passes its `ListingRef` to one helper, `ticketOn(ref: ListingRef, side)`, which asks `POST /api/order/listing`, then opens the ticket, or says the refusal in the header and opens nothing.
- **A listing page with no venue offers no Buy** (brief 35, 5). Notifications and the US heatmap's tiles open a listing page with an empty exchange (brief 34, page U6), so such a page has no exact listing to place an order on. Its Buy is not offered until brief 34's section 2 gives those pages their listing.
- The `Ticket` state holds `listing: OrderListing`; `symbol`, `securityId` and `exchange` as separate fields go.
- `tkLookup`'s match by symbol goes. The holding and position the ticket defaults from are found by the listing's `security_id`, or by the holding id the page passed.
- The draft (`TicketDraft`) holds the listing. A draft is restored only onto a ticket for the same `security_id` and side; a draft for one listing never fills another's ticket. A draft saved before this change holds a symbol only, and is dropped: attaching it to a listing would be a lookup by ticker in the order path (brief 35, 4). A draft is unsent typing.

**What stays:** the ticket's figures, its accounts, the bracket legs, the gate and the read-back, `OrderRequest`, the book's order tables, and every screen.

**`SPEC.md` changes,** in the same commit, so that "symbol" is not read as the key:
- order-ticket-2: a row opens the ticket "on that row's listing";
- order-ticket-3: "on that instrument's listing";
- order-ticket-4: "opening a ticket on the same listing and side brings every field back";
- orders-6: the card shows "the exchange and symbol of the draft's listing".

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green in `rust/`, and `cargo clippy --workspace --lib --bins` clean.
- [ ] `npm run check`, `npm test` and `npm run e2e` green in `web/`.
- [ ] **No order path resolves a listing from a ticker.** Verified by reading, and named in the PR:
  - `tkLookup`'s symbol match, `resolve_security`'s symbol branch and the symbol-keyed `found` are gone;
  - `Ticket`, `PreviewRequest` and the quote document's parameters require `security_id`;
  - a request without it is refused, by a Rust test per request type.
- [ ] **The rule over every kind of collision, server side.** A Rust test generates pairs of listings that share a ticker, of each kind brief 34 names: two venues, one company or two; a suffix stripped; an option and its underlying; a coin and a share. Each pair is tried with both in the book, and with one in the book and the other only outside it, in both orders of reading. For each listing, a quote, a preview and a placed order (dry) name that listing's `security_id` in the `OrderRequest` written, never the other's. It fails on today's code, shown in the PR.
- [ ] **The resolver.** Rust tests:
  - `Venue` matches only the exact (symbol, MIC), against a recorded search answer holding one symbol on two venues;
  - the venue words of every venue the directory search returns map to the MIC Wealthsimple answers for it;
  - it refuses when none or two match, and on a venue the table does not know;
  - `Instrument` answers that instrument's own id, and an id found by search is in the book afterwards;
  - the cache holds both listings of one symbol apart.
- [ ] **Every entry point, in the browser** (brief 35, 6). The bug as found: one ticker held on the TSX, with Buy from the US listing's directory row or its page.
  - The e2e book holds the TSX listing. The US listing with the same ticker is only a directory row, whose answer is from the offline stand-in.
  - A second ticker is held on both venues, for the Sell entry points.

  A Playwright test opens the ticket from:
  - each holding's page (Buy and Sell);
  - the trade's page;
  - the US listing's page;
  - the ⌘K book row and the ⌘K directory row;
  - the draft card's Resume.

  Each time, the ticket's listing and the request it sends name that entry point's own listing. A listing page with an empty exchange shows no Buy. It fails on today's page, shown in the PR.
- [ ] **The draft.** A Vitest test: a draft closed on one listing does not fill a ticket opened on the other listing with the same ticker, and a draft of the old shape is dropped.
- [ ] **Rendered.** The ticket opened from each entry point on the scratch server on a copy of the owner's book, at 1340 px, screenshots in the PR.
- [ ] `docs/old-app-mistakes.md` has the entry, naming the tests above.

## Surfaces to check beyond the diff

- `web/src/lib/generated/orders.ts` and `wire.ts` (regenerated by `cargo test -p bagholder-server`'s type export).
- `docs.rs`'s quote document key (`quote:` followed by the parameters), and where pages subscribe to it.
- `rust/crates/server/tests/golden/routes.json`, for the new route.
- The e2e book's generator (`web/e2e`), which must hold the pair.
- `OrdersPanel.svelte`'s draft card, which reads the draft's fields.
- `docs/old-app-mistakes.md`.

## Right to refuse

If a holding's row in the book has no Wealthsimple id and Wealthsimple's search cannot find its symbol on its venue, its Buy and Sell cannot open a ticket. Today they would open one on a guess. The ticket then says why in the header, which is the rule; it is not a departure.

## Anti-stub self-check

- No definition nobody references: `OrderListing`, `ListingRef`, `ListingOf` and `OrderListingAnswer` are each read by the page or the server. — Claude
- No field written and never read: `found_at` is read before every search; `TicketDraft.listing` is read on resume and on restore. — Claude
- No branch only the switch knows: none. — Claude
- No real-target run skipped: the suites ran, and the ticket was opened on the scratch server on a copy of the owner's book. — Claude

## Verification

- `RUSTFLAGS="-D warnings" cargo test --workspace --no-fail-fast`: green; `cargo clippy --workspace --lib --bins`: clean. After master (#514) was merged in: `cargo test -p bagholder-server -p bagholder-market` green, and the release build succeeds.
- `npm run check`: 0 errors; `npm test`: green, the draft test among them.
- `npx playwright test`: the whole suite green, 312 tests, after the one test of the order's request shape was brought to the new shape.
- `e2e/listingorders.spec.ts` run on master `f89634de` fails all five tests. The directory's NYSE row opened `quote:symbol=TD&security=sec-td&…&exchange=NYSE`: a ticket on the TSX holding, which is the bug.
- The scratch server on a copy of the owner's book, at 1340 px: the ticket opens from the ⌘K row and from the holding's page on that holding's own listing, and says "Not connected." where the copy has no sign-in. The console is clean.

## Handoff

Built in full; nothing open.

**Nothing left running.** The scratch server was stopped.
