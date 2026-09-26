# Stage 3c: the new build against SPEC, section by section

On 2026-09-25 the owner found three things the rebuild had changed unasked: the header's sync step read one fixed label, a month's margin interest fell into the month before, and the tab showed a template icon. Each had passed every check the plan ran, because those checks covered figures and data, not everything the app does and says. So every checkable statement of `SPEC.md` §1–§6 was checked against the code of the new build: about 470 statements, read, not guessed, each with the code that does it or the reason it does not.

This file is the done-contract for what was found. An item is ticked when it is fixed and held by a test, or when `SPEC.md` is corrected in the same commit with the reason (where the build is right and the text is stale). "Old page same" marks a gap `ledger.html` already had: still wrong against SPEC, carried over rather than caused.

## Found by the owner

- [x] The header shows each step of a sync, account by account (7504f0fa).
- [x] A row's day is Toronto's, where Wealthsimple states its days; each month's margin interest is in its own month again (fc0e54d9).
- [x] Stored rows follow a new mapping version at startup (fc0e54d9).
- [x] The tab icon is the app's own (fc0e54d9).
- [x] No count of what a figure left out, and no note under a chart, anywhere (17ffe15e); a page test fails on either.
- [x] A coin moved in costs what Wealthsimple states it arrived at; the person's cost comes first (17ffe15e).
- [x] A sale past what the book holds keeps the P&L of what was held; a stated zero value is a value (498ff87d). On the owner's book nothing is left out: Realized P&L, the Equity curve's end and the sum of every trade's P&L agree.
- [x] Annualized returns: hovering a year says how far it was over or under the index (624b960f).

## High

- [ ] **Source failures never reach the header** (§1, §2 Distribution rate). A failed read of the Bank of Canada, TMX, Yahoo, a payer reader or a quote source is recorded in the cache and nothing more; the header shows only Wealthsimple's failures. `sources/src/rates.rs:185`, `server/src/status.rs:84`.
- [ ] **The page never reloads after an update** (§2 Versions). A new server answering only drops the chart cache; the old page keeps running, and a protocol bump reads "Restart Bagholder to finish the update". `web/src/lib/live.ts:258`.
- [ ] **Back loses the list's scroll** (§4 Trades). Only the window's scroll is kept; the Trades and Holdings tables scroll inside their cards. `router.svelte.ts:73`.
- [ ] **A sync error is cut to 57 characters in the header** (§4 header). The Chrome-closed message reads "…Choo…". Old page same. `App.svelte:185,302`.
- [ ] **The market universes are never read at start or every 30 minutes** (§4 Markets). Only a click on an unread one reads it; after that their day changes go stale. `feeds.rs:2305`.
- [x] **A Sell from the ticket on shares a bracket holds does not wait for the stop's cancel** (fc293360: it waits for the confirmation, and without it nothing is sold) (§4 Order ticket, Nothing left behind). After 8 s it sends the sell whatever the cancel's state, so a stop and a sell can both rest on the same shares. `orders/ticket.rs:735`.
- [ ] **A fill of a Bagholder order is not written at once** (§4 Orders). It waits for Wealthsimple's activity feed. `orders/readback.rs:262`.

## Medium

- [ ] Sync status: `folder scanned` notice missing after Scan now or Watch folder. `ui.svelte.ts:343`.
- [ ] A failed figures pass's header error is never cleared by a later good pass. `due.rs:45`.
- [ ] A failed update's rollback is not said in the header; in a git checkout the supervisor exits instead of restarting. `update.rs:640`.
- [ ] A disclosure or release banner opens the page only for a held symbol. `notes/channel.svelte.ts:88`.
- [ ] Short interest is asked for an option's underlying and for a coin. `trade/shorts.svelte.ts:19`.
- [ ] A failed bars request shows "No price history for this span." instead of the failure. `trade/chart.ts:91`.
- [ ] With no history source, the priced executions are not plotted on a time axis. Old page same.
- [ ] A remembered or addressed universe with no rows is never read until clicked. `heat.svelte.ts:35`.
- [ ] The tile picker opens inside the tile row, not under the header. `MarketTiles.svelte:142`.
- [ ] News search: any 1–6 letter word becomes a chip. Old page same. `News.svelte:205`.
- [ ] Brackets: a quantity changed at Wealthsimple is not adopted; a refusal for shares not there does not end the bracket; the 5 s check pauses during a sync; the stop leg never reads `Watching`; an ended bracket reads `Cancelled` not `Off`; a card being cancelled has no `Cancelling`. `brackets.rs`, `orders.svelte.ts`.
- [ ] Ticket: switching Buy/Sell keeps the quantity and account; a draft card shows no amount or legs; the badge counts every account while the panel follows the filter. Old page same for the first two.
- [ ] Disclosures: the Summary column is drawn with no summary; half-filled rows are read again without limit; a titled row without a sentence waits until shown again; the re-read button shows no reading state.
- [ ] Filters: single-list filters other than Symbol and Tag have no keys; a found instrument row has two icons and does nothing on click. Old page same for the second.

## Low

- [ ] Left and Right change tabs with the Notifications panel open. Old page same.
- [ ] The Update button stays while updating.
- [ ] The page's 3-minute connect timer races the server's own message.
- [ ] Win rate hides a zero breakeven count; Grade vs P&L and Review queue carry subtitles and empty states SPEC does not list. Old page same.
- [ ] A holding row has no hover text naming its account. Old page same.
- [ ] Opening a page does not read cash and buying power when they are under five minutes old.
- [ ] Available margin's subtitle reads `Buying power` while its value waits.
- [ ] Event entry makes its own required-field messages and flashes `Entered`.
- [ ] Trade and holding pages pick Buy/Sell by symbol in any account, not by the holding's id.
- [ ] Fear & Greed: only the meter on show is kept fresh; "did not answer" shows while the first read runs; the word is not in the band's colour.
- [ ] News is read only while a page is open.
- [ ] Short interest: a searched ticker outside the scope, or among prefix matches, is not added.
- [ ] Listing addresses are percent-encoded.
- [ ] Watchlist match quotes never expire.
- [ ] A `sending` order left by a stop is on no tab; dry orders do not cover bracket edits; no row is written without a session; trade page Buy/Sell open the ticket; Max on a Sell uses the position picked at open; trailing moves only strictly above half a percent; position size converts only USD; `—/mo margin interest` with no month charged.
- [ ] Disclosures `Opening…`; the data modal has no entrance motion; the skeleton's removal uses an animation callback; the store runs `synchronous = NORMAL`; a replaced database is not migrated until restart.

## SPEC text to correct (the build is right)

- [ ] The notification history arrives on the shared stream, not its own endpoints; the page is sent changes, it does not poll every 30 seconds (§2).
- [ ] Quotes are read while a page shows them (§2 table contradicts §2 Position).
- [ ] Payer records follow §1's schedule, not a 20-hour cycle; the Bank's rates are read when needed and at each publication (§2).
- [ ] `SEDAR+ is unavailable.` (PR #275); the filter's search order (PR #276); the Notifications bell in the header; the watchlist's scroll height (§4).
