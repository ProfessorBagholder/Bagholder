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

- [x] **Source failures never reach the header** (the header's error is now built from each source's newest outcome in the cache, beside Wealthsimple's and the figures' own; a cache commit tells the page) (§1, §2 Distribution rate). A failed read of the Bank of Canada, TMX, Yahoo, a payer reader or a quote source is recorded in the cache and nothing more; the header shows only Wealthsimple's failures. `sources/src/rates.rs:185`, `server/src/status.rs:84`.
- [x] **The page never reloads after an update** (§2 Versions). A new server answering only drops the chart cache; the old page keeps running, and a protocol bump reads "Restart Bagholder to finish the update". `web/src/lib/live.ts:258`. — fixed (428e9589): a restarted server of another version or protocol reloads the page, once.
- [x] **Back loses the list's scroll** (§4 Trades). Only the window's scroll is kept; the Trades and Holdings tables scroll inside their cards. `router.svelte.ts:73`. — fixed (f62f5307): each list's inner scroll comes back.
- [x] **A sync error is cut to 57 characters in the header** (§4 header). The Chrome-closed message reads "…Choo…". Old page same. `App.svelte:185,302`. — fixed (65d14d29): shown whole, the header never overflowing.
- [x] **The market universes are never read at start or every 30 minutes** (§4 Markets). Only a click on an unread one reads it; after that their day changes go stale. `feeds.rs:2305`. — fixed (7dd46288): read while a page shows one, when it has no rows or they are 30 minutes old; SPEC corrected.
- [x] **A Sell from the ticket on shares a bracket holds does not wait for the stop's cancel** (fc293360: it waits for the confirmation, and without it nothing is sold) (§4 Order ticket, Nothing left behind). After 8 s it sends the sell whatever the cancel's state, so a stop and a sell can both rest on the same shares. `orders/ticket.rs:735`.
- [x] **A fill of a Bagholder order is not written at once** (§4 Orders). It asked one pull and, if Wealthsimple had not listed the fill yet, nothing more until the next day's sync. Now it is pulled for until Wealthsimple's own row for that order is in the book, soon at first and hourly after three hours; SPEC describes the pull (the broker's row, never a synthetic one), not the local trade it used to name.

## Medium

- [x] Sync status: `folder scanned` notice missing after Scan now or Watch folder. `ui.svelte.ts:343`. — fixed (22be8ddf, 80272e5b; the count is the server's).
- [x] A failed figures pass's header error is never cleared by a later good pass (it has its own place now, cleared by the next good pass and by nothing else). `due.rs:45`.
- [x] A failed update's rollback is not said in the header; in a git checkout the supervisor exits instead of restarting. `update.rs:640`. — fixed (d162919e): the executables are kept before a checkout's build, so the supervisor puts them and the checkout's commit back and starts that version; the failure is written for the restarted server, which says it in the header.
- [x] A disclosure or release banner opens the page only for a held symbol. `notes/channel.svelte.ts:88`. — fixed (ba7870a3): it opens the listing's page, the holding's where the book holds it.
- [x] Short interest is asked for an option's underlying and for a coin. `trade/shorts.svelte.ts:19`. — fixed (690f1757): asked and drawn for shares only.
- [ ] The book's own rows name an index, a rate, a currency pair and an event contract `Shares` (`server/src/wire/build.rs` `kind_word`), so the page cannot tell them from shares (the short-interest gate reads that word). Each kind needs its own word, and the Kind filter must read them; a wire change.
- [x] A failed bars request shows "No price history for this span." instead of the failure. `trade/chart.ts:91`. — fixed (175cd3d0).
- [ ] With no history source, the priced executions are not plotted on a time axis. Old page same.
- [x] A remembered or addressed universe with no rows is never read until clicked. `heat.svelte.ts:35`. — fixed (7dd46288).
- [x] The tile picker opens inside the tile row, not under the header. `MarketTiles.svelte:142`. — fixed (9ffdfb6a).
- [x] News search: any 1–6 letter word becomes a chip. Old page same. `News.svelte:205`. — fixed (0a803fbd): the book's rows, then the directories, by the ticker itself. Not yet: SPEC's last step, a ticker no directory carries settled by the security records and TMX's resolver, needs a server lookup the page can ask without reading news; such a ticker stays a text search until it is built.
- [ ] Brackets: the stop leg never reads `Watching`; an ended bracket reads `Cancelled` not `Off`; a card being cancelled has no `Cancelling`. (Fixed in 1d95dcda: a quantity changed at Wealthsimple is adopted; a refusal for shares not there ends the bracket; the checks run through a sync; trailing moves at half a percent.)
- [ ] Ticket: switching Buy/Sell keeps the quantity and account; a draft card shows no amount or legs; the badge counts every account while the panel follows the filter. Old page same for the first two.
- [x] Disclosures: the Summary column is drawn with no summary; half-filled rows are read again without limit; a titled row without a sentence waits until shown again; the re-read button shows no reading state. — fixed (1684d6de: two reads that could have answered settle a row, a read with no model up does not count and the row waits, the background reader takes titled rows without a sentence; e5af0731: the Summary column only with a summary, Reading until a forced read answers).
- [x] Filters: single-list filters other than Symbol and Tag have no keys; a found instrument row has two icons and does nothing on click. Old page same for the second. — fixed (8ef8d0a6).

## Low

- [x] Left and Right change tabs with the Notifications panel open. Old page same. — fixed (80eabab4), the order ticket too.
- [x] The Update button stays while updating. — fixed (97dc817f).
- [x] The page's 3-minute connect timer races the server's own message. — fixed (7574691f).
- [ ] Win rate hides a zero breakeven count; Grade vs P&L and Review queue carry subtitles and empty states SPEC does not list. Old page same.
- [x] A holding row has no hover text naming its account. Old page same. — fixed (7aa8b8b5): the app's own tooltip; SPEC's "title text" corrected.
- [x] Opening a page does not read cash and buying power when they are under five minutes old. — fixed (6c391436): every page opening, a second beside an open one included, wakes the broker's loop and reads them.
- [x] Available margin's subtitle reads `Buying power` while its value waits. — fixed (9b215d20).
- [x] Event entry makes its own required-field messages and flashes `Entered`. — fixed (891c39b6).
- [x] Trade and holding pages pick Buy/Sell by symbol in any account, not by the holding's id. — fixed (7307d68a).
- [x] Fear & Greed: only the meter on show is kept fresh; "did not answer" shows while the first read runs; the word is not in the band's colour. — fixed (8467b31f: both meters watched while the card shows; `Reading…` while a read is in the air; the word in the band's colour; SPEC says both are refreshed while a page shows the card).
- [x] News is read only while a page is open. — decided and fixed (2dd5f539): read while a page shows the News card or a Releases notification set is on, not at start for nobody and not for a page on another tab; SPEC corrected.
- [x] Short interest: a searched ticker outside the scope, or among prefix matches, is not added. — fixed (825920d8).
- [x] Listing addresses are percent-encoded. — fixed (55d05da4).
- [x] Watchlist match quotes never expire. — fixed (024b887b).
- [ ] A `sending` order left by a stop is on no tab; dry orders do not cover bracket edits; no row is written without a session; trade page Buy/Sell open the ticket; Max on a Sell uses the position picked at open; trailing moves only strictly above half a percent; position size converts only USD; `—/mo margin interest` with no month charged.
- [ ] The skeleton's removal uses an animation callback. (Fixed: Disclosures `Opening…`, 47dbc752; the data modal's entrance motion, ab423bc7.)
- [x] The store runs `synchronous = NORMAL`. — fixed (b319efa3): FULL, every commit flushed.
- [x] A replaced database is not migrated until restart. — fixed (f54d5c93): each borrow checks the file is the one kept connections hold and reads its stamped version; a replaced or rolled-back file is prepared on that borrow.

## SPEC text to correct (the build is right)

- [x] The notification history arrives on the shared stream, not its own endpoints; the page is sent changes, it does not poll every 30 seconds (§2).
- [x] Quotes are read while a page shows them (§2 table contradicts §2 Position).
- [x] Payer records follow §1's schedule, not a 20-hour cycle; the Bank's rates are read when needed and at each publication (§2).
- [x] `SEDAR+ is unavailable.` (PR #275); the filter's search order (PR #276); the Notifications bell in the header; the watchlist's scroll height (§4; the build's 436 px, the News card's cap). SPEC corrected for all four lines (b78d6dd3).
