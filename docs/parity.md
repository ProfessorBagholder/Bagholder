# Feature parity: `SPEC.md` against the Svelte page

Every behaviour `SPEC.md` gives the web app, and whether `web/src` has it. Built 2026-09-20 by reading the code, not by running it, so an item marked done here is still owed its browser test (stage 8 of `docs/architecture.md`), and anything marked **unsure** is settled by driving it. An item is closed when it works on real data and a Playwright test holds it. `L:` is a line in `ledger.html`, the reference until cutover.

Counted then: about 60 done, 17 partial, 25 missing, 3 unsure.

## Open items

### Header and menu
- [x] **Update button** "Update to vX.Y.Z", and the "Update available" / "image available" link (`L:943-946`, action `update` `L:4972`); nothing calls `/api/update`.
- [x] `Downloading…`, `Installing…`, `Restarting…` and the update-failed line (`L:929-930`).
- [x] **"Restart Bagholder to finish the update"** on a protocol mismatch (`L:931`); the page has no `PROTOCOL`.
- [x] Sync status: the spinner alone before the server's first answer (`L:937`); `Refreshing session…` (`L:933`, `L:4861`).
- [x] Theme row: opens on a tap as well as hover (`L:4964`); choosing a theme closes the list (`L:5019`).
- [x] Notifications row: opens on a tap; the state word reads `Blocked` and `Unavailable` as well as `On`/`Off`; the permission prompt on first press; dimmed switches when refused (`L:1086-1122`).
- [x] `Send a test notification` (`L:1101`, `L:1123`).

### Filters
- [x] A symbol row for a held symbol opens its holding page, not the listing route (`L:751-752`).
- [x] Shift+Enter narrows by the highlighted symbol (`L:5174`, `L:5203`).
- [x] Backspace/Delete in an empty box deselects the highlighted value (`L:4825-4829`).
- [x] Tab ring: box → Done → Clear all → box, Shift+Tab backwards (`L:4820-4824`, `L:5149-5155`).
- [x] The highlight scrolls into view (`L:5163`); focus returns to the box after a click toggle (`L:4956`); ⌘K with the popover open on a field returns to the fields view.
- [x] Filtering while a trade is open returns to the list (`L:843-847`).
- [x] Cashflow names the filters it ignored (SPEC §5; in neither page yet). *(Found done in both pages; now driven by a test.)*

### Dashboard
- [x] Value-axis labels that collide are hidden (`spaceAxisLabels`, `L:2399-2412`).

### Trade, holding and listing detail
- [x] **The listing page** for a ticker the book does not hold: route `markets/listing:SYMBOL@VENUE`, `/api/listing`, `listingAsTrade` (`L:710-749`). Every click on an unheld ticker (watchlist, shorts, heatmap, ⌘K, a notification) depends on it.
- [x] Back returns to the list at its scroll position; a newly opened page starts at the top (`L:4726`, `L:4739-4763`).
- [x] A journal save that fails says so in the header (`L:909`).
- [x] Disclosure rows waiting on the local model wake when it is ready (`summaryReady`, `L:866`). *(Design: the server's document reader wakes on `localmodel::on_change` and sends the rows; the page waits for nothing.)*
- [x] No redraw under an open trade or a note being typed (`L:871-876`). *(Design: stage 6.)*
- [x] Chart cache reset when the server restarts (`startedAt`, `L:855-859`).
- [x] The short-interest reading expiring after 30 minutes: asked again when the card is shown or the reader returns to the tab with an older one. The original re-asked from its 30-second redraw; no clock runs here, since the exchanges report twice a month.

### Markets
- [x] The heatmap on its own has an address, `#heatmap/<universes>/<size>/<secs>`: bookmark, wall display, scope list and dwell, `replaceState` (`L:4699-4708`, `L:4911-4913`); arrow keys must not move the tabs beneath it.
- [x] **Unsure:** two sectors each folding to `Other (2)` give heat tiles the same key (`HeatBox.svelte:62`); drive it. *(Driven: it was real -- the duplicate key stopped the whole heatmap drawing. Tiles are now keyed apart by sector, venue, place.)*
- [x] Enter adds the banded watchlist suggestion (`L:5146`).
- [x] Esc in the News box clears the words, then the chip; Esc in the Shorts box clears its words (`L:5134-5137`).
- [x] News reads `Reading…` while a pass is reading the scope, not only for a chip lookup (`newsPassReads`, `L:3618`).
- [x] The Shorts feed refreshes on an open page. *(Design: by event, stage 6.)*
- [x] Esc closes the tile picker and the watchlist add row wherever focus is (`L:5133`).

### Order ticket and Orders panel
- [x] The page behind an open panel does not scroll (`panel-open`, `L:4648`).
- [x] ⌘K works over the Orders panel (`L:5104`).
- [x] Enter saves an order editor; the editor takes focus when it opens (`L:4992-4995`, `L:5113`).
- [ ] The cards' pixel grammar against the original, line by line: belongs to stage 8's pixel comparison, which runs both pages on the same orders.

### Notifications
- [x] The browser channel: `new Notification`, `/api/notifications/seen`, the banner's click (`L:1137-1149`).
- [x] A fill or order-problem card opens the Orders panel at its tab (`L:1059`).
- [x] A row arriving while the panel is open reads at once; the stream opens with `?after=`; the server's returned settings are adopted (`L:1071`, `L:1133`, `L:1114`).

### Connect, sync, data
- [x] **The first-run page**: "No activity yet" / "Pulling your history", its button, the full sync error (`L:4329`).
- [x] Enter submits Add trade and the folder path (`L:5180-5181`).

### Keyboard and navigation
- [x] `#positions` is an alias of `#portfolio` in the router (`L:4699`).
- [x] Esc in the tag box does not navigate back (the guard looks for an id no element has).
- [x] Every write carries the `X-Bagholder` header (several rely on the browser's `Sec-Fetch-Site` today).

### States
- [x] **Found while testing:** a view arriving again (reconnect, filters) took the open trade's fills off its row and blanked its chart; the detail is now held beside the model and refreshed when the whole view comes again.
- [x] Per-tab loading skeletons with the cross-fade (`L:4367-4431`).
- [x] The digit roll on tile values (`L:4442-4500`); an action on each tile's figure (`actions/roll.ts`), ended by the last wheel's `transitionend`.
- [x] The cut-text hover (`#cutTip`, `L:3379-3395`, `L:5259`).
- [x] **Scrollbars appear while scrolling** (`scrolling` class, `L:5270-5277`); `scrollbars.ts`.
- [x] A model reply with `ok:false` is handled; the error state offers a retry (`L:787`, action `reload`).
- [x] A quote tick fetches only live figures (`L:803-818`). *(Design: carried by the event, stage 6.)*

## Found done (each still owed its browser test)
Header brand, version, sync line and notices; Orders button and bell with badges; menu items and the confirm dialog; filter fields, presets, ranges, chips, ranked search with external lookup; ⌘K, Esc cascade, arrow keys between tabs; Dashboard tiles, equity curve, annualized card and benchmark switch, monthly bars, grades, by symbol, review queue; Trades list columns, sorts and reset; trade and holding detail, chart timeframes and markers, executions, journal, short interest, disclosures; Portfolio tiles, donuts and holdings; Markets tiles, fear and greed, heatmap card, watchlist, shorts, news; Cashflow tiles, chart, tables and donut; order ticket and brackets; Orders panel tabs, drafts and actions; notification history and stream; connect with the streamed sign-in, sync, refresh, disconnect; CSV import, watch folder, add trade, export, clear data; reload on a new server version.
