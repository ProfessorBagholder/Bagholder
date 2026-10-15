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
- The market readers and the page's other ticker keys (charts, history, filings, news, breakdowns, watchlist): brief 34, sections 2 and 3, next.
- Orders already placed, brackets, and the read-back. They already name the listing by Wealthsimple's id (`OrderRequest::broker_security`, `gate.rs:281`); nothing in them is keyed by ticker.

## The old app here

The old app opened its ticket from a symbol and looked the listing up by that symbol (`ledger.html`'s `tkLookup`). The Svelte port carried `tkLookup` over unchanged (`web/src/lib/ticket/ticket.svelte.ts:39–61`). The Rust server carried over the symbol fallback (`orders/ticket.rs:106–145`) and the symbol-keyed search cache (`orders/mod.rs:89`). That is the mistake `docs/architecture.md` §5 names ("bare symbols as identities"). Nothing of it is carried over. A new entry in `docs/old-app-mistakes.md` records it, with the test that guards it.

## How the leading products do it

- **Interactive Brokers** (Web API, "New Order Example", read 2026-10-15, https://www.interactivebrokers.com/docs/web-api/trading/orders/new-order-example). An order names its instrument by `conid`, IBKR's contract id. One listing has one conid, and it is found first through `/iserver/secdef/search`. The order body has no symbol.
- **Questrade** (API, "accounts/:id/orders" and "symbols/search", read 2026-10-15, https://www.questrade.com/api/documentation/rest-operations/order-calls/accounts-id-orders, https://invest.questrade.com/api/documentation/rest-operations/market-calls/symbols-search). An order names `symbolId`, an internal id per listing. The search's own example for BMO returns two results, NYSE and TSX, with different `symbolId`s and a `listingExchange` to tell them apart.
- **Wealthsimple** (its own GraphQL, as the app already calls it). An order names `securityId`, one id per listing: `sec-s-…` for a share, `sec-o-…` for a contract. Search (`FetchSecuritySearchResult`) returns each listing with its `primaryExchange`.

All three key an order by the listing's id and take the symbol as display. This plan does the same. The listing's id is Wealthsimple's `securityId`, the broker's reference recorded against the instrument (§5).

## Open questions

None. Each listing a button can be pressed on is already known to the page, as one of two things:
- **The book's own instrument.** It has an `InstrumentId`, and Wealthsimple's `securityId` where the broker has stated one (`Names::security`).
- **A listing outside the book.** It has a symbol and an exchange, as the search row or the listing's page shows it. Wealthsimple's search is already matched on both exactly (`parse_listing_search`, `orders/ticket.rs:368–387`). Only the symbol-keyed cache and the fallback before it lose the exchange.

## Approach

**One type for the listing an order is on.**
- `OrderListing { security_id, symbol, exchange, currency }` in `rust/crates/server/src/orders/ticket.rs`, generated to `web/src/lib/generated/orders.ts`.
- `security_id` is Wealthsimple's id and is never empty; the others are what the ticket shows.
- Every request that names a listing carries `security_id` as a required field: `Ticket`, the quote document's parameters and `PreviewRequest`. Each refuses a request without it (`deny_unknown_fields` is already on). `Ticket.symbol` is removed.

**One way to get one.** `POST /api/order/listing` takes either:
- `{ instrument }`, the book's own: Wealthsimple's id from `Names::security`. Where the broker has stated none (an instrument only entered by hand or imported), Wealthsimple's search is asked with the instrument's current symbol and venue (`InstrumentInfo::current_name`), matched on both; or
- `{ symbol, exchange }`, a listing outside the book: Wealthsimple's search, matched on the bare symbol and the exchange exactly, as now.

It answers the `OrderListing`, or a refusal in the ticket's words ("Wealthsimple lists no RY on NYSE."). A search that finds two listings on one exchange with one symbol is a refusal too, never a choice. The cache of what the search found is keyed by `security_id` and by the exact (symbol, exchange) asked, never by symbol alone.

**The server takes the listing as given.** `resolve_security` becomes `listing(app, security_id)`. It reads the book's instrument for that id where the book holds it, else the cache by id, else Wealthsimple's own summary of the security (`fetch_quotes` already reads `SummarySecurity` by id: symbol, exchange, currency). The symbol branch and the "first instrument with that symbol" choice are deleted. `ticket_quote`, `preview_for` and `ticket_order` call `listing` with the request's `security_id`.

**The page holds the listing.**
- `openTicket(listing: OrderListing, side, holding?)`. Every caller either has an `OrderListing` (a holding or trade: `security`, `symbol`, `exchange` and `currency` are on the row already) or asks `POST /api/order/listing` first, through one helper `ticketOn(of, side)`. The helper says a refusal in the header and opens nothing.
- The `Ticket` state holds `listing: OrderListing`; `symbol`, `securityId` and `exchange` as separate fields go.
- `tkLookup`'s match by symbol goes. The holding and position the ticket defaults from are found by the listing's `security_id`, or by the holding id the page passed.
- The draft (`TicketDraft`) holds the listing. A draft is restored only onto a ticket for the same `security_id` and side. One saved before this change holds a symbol only: it is attached to the one listing the book holds under that symbol, and dropped where the book holds none or more than one. A draft for one listing never fills another's ticket.

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
- [ ] **The rule over every kind of collision, server side.** A Rust test generates pairs of listings that share a ticker, of each kind brief 34 names: two venues, one company or two; a suffix stripped; an option and its underlying; a coin and a share. Both of each pair are in the book, and the test runs both orders of reading. For each listing, a quote, a preview and a placed order (dry) name that listing's `security_id` in the `OrderRequest` written, never the other's. It fails on today's code, shown in the PR.
- [ ] **The resolver.** Rust tests:
  - `{ symbol, exchange }` matches only the exact pair, against a recorded search answer holding the same symbol on two exchanges;
  - it refuses when none or two match;
  - `{ instrument }` answers that instrument's own id;
  - the cache holds both listings of one symbol apart.
- [ ] **Every entry point, in the browser.** The e2e book holds two listings with one ticker, one on the TSX and one on a US exchange, each held. A Playwright test opens the ticket from each entry point:
  - the holding's page (Buy and Sell);
  - the trade's page;
  - the listing's page for the one not held;
  - the ⌘K book row and the ⌘K row from outside the book;
  - the draft card's Resume.

  Each time, the ticket's listing and the request it sends name that entry point's own listing. It fails on today's page, shown in the PR.
- [ ] **The draft.** A Vitest test: a draft closed on one listing does not fill a ticket opened on the other listing with the same ticker. A draft of the old shape is attached where the book holds exactly one listing with its symbol, and dropped otherwise.
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

To be initialled at the build.

## Verification

To be filled at the build.

## Handoff

To be filled at the build.

**Nothing left running.**
