# Findings: The page as the person sees and uses it

Part of brief 15. Each finding was raised by one of two reviewers and checked by two verifiers (code, standard). `merged` means the same root cause is kept under the id named in the reason; `contested` means one verifier refuted it and the reason is given; a finding both refuted is not listed. Ids: `<area>-A-n` and `-B-n` are the reviewers', `-V-n` and `-S-n` were added by the verifiers.


## examined (A)
Read: SPEC.md §4–§7 (every page, filters, layout, motion, verification), docs/decisions.md, docs/parity.md, docs/design-review.md, docs/old-app-mistakes.md, docs/future.md; web/src/App.svelte, app.css, lib/Dashboard.svelte, Trades.svelte, TradeDetail.svelte, Portfolio.svelte, Cashflow.svelte, Markets.svelte, markets/News.svelte (header), Menu.svelte, Modals.svelte, ConfirmDialog.svelte, Empty.svelte, LoginView.svelte, Skeleton.svelte, FilterPopover.svelte (fields view, summaries), ticket/OrderTicket.svelte, ticket/ticket.svelte.ts (submit, drafts), orders/OrdersPanel.svelte, notes/NotesPanel.svelte, fmt.ts, filters.svelte.ts (activeCount), router.svelte.ts, theme.svelte.ts, ui.svelte.ts (flash), state.svelte.ts (journal save), Donut.svelte (labels), Watchlist.svelte (remove), subs.svelte.ts (FIRST_ROWS); rust/crates/engine/src/scope.rs (monthly bars) and rust/crates/server/src/views.rs (Trades sort keys). Ran: the release server on port 8801 (made-up book, offline, dry orders) and a second instance on an empty home on port 8951 for first run; Playwright screenshots of Dashboard, Trades, Portfolio, Markets, Cashflow, a closed trade page, a holding page at 1200/1340/1440/1680 px in Nocturne, at 1440 in Light and Midnight, and at 1000 and 800 px; drove the menu (Theme, Notifications submenus), Add trade, Import CSV, Load folder and Clear data dialogs, the filter popover (fields, Date, ⌘K search 'aa'), the Orders and Notifications panels, the order ticket from a holding through Review, column sort, Tab order, Escape with a filter set, equity-curve hover, the heatmap alone; read the view API (trades, positions, dashboard, cashflow, book, status) to trace figures. Both servers stopped afterwards. Not covered (offline demo has no quotes, news, filings or regulator data): Short interest tiles and card, Disclosures rows and enrichment, News with items and the chip, Watchlist with rows and the add row, market tiles with quotes and the tile picker/drag, Fear & Greed with a reading, market heatmap universes, the Connect/sign-in stream, Orders panel with live cards, brackets, notification rows, a Load-folder scan or a CSV import result, the update flow; the phone apps are out of scope.

## measurements (A)
- Document width and horizontal page scroll at 1200, 1340, 1440, 1680 px on every tab: no page-level horizontal scroll at any width (document width = viewport − 15 px scrollbar); no card/table overflow at 1340 and above  [Playwright, documentElement.scrollWidth vs innerWidth, and scrollWidth > clientWidth over table/.card elements]
- Overflow inside cards below 1340: 1200: News card 46 px, Cashflow positions table 13 px; 1000: KPI card 9 px, Portfolio cards 7 and 38 px, Markets cards 68 and 246 px, Cashflow table 32 px; 800: four KPI cards 3–42 px  [same script, scrollWidth − clientWidth per element]
- News search box vs its card's right edge: 1200: input right 744 px, card right 707 px (37 px clipped); 1340: 822 vs 847; 1440: 922 vs 947; 1680: 1162 vs 1187  [getBoundingClientRect on the input and its closest .card]
- Trades card height on a 1440×1200 window: card 620 px, bottom at 760 px, 440 px empty below; 31 rows loaded  [getBoundingClientRect on the Trades card, tbody tr count]
- Equity curve (P&L mode) time linearity: hover at 25% of width → 2 Dec '24; at 50% → 10 Feb '25; axis Apr '24 … Jul '26 (27 months); 22 points  [mouse.move over the SVG at 25%/50% and reading .tip; GET /api/view?key=dashboard]
- Monthly P&L bars vs calendar span: 14 bars for Apr 2024 – Jul 2026 (28 months); missing 2024-05,07,09,10, 2025-05,07,09,10,11,12, 2026-01,02,04,06  [GET /api/view?key=dashboard monthly keys]
- Cashflow chart months: 31 consecutive months 2024-03 … 2026-09  [GET /api/view?key=cashflow months keys]
- Open trade P&L cell: text '$0.00', colour rgb(111,211,155) (--pos); P&L % '—' in the same green  [getComputedStyle on the first row whose Close reads 'Open']
- Trade page Buy/Sell buttons: 22×22 px, glyph 12 px, colour rgba(233,233,237,.5); on a closed trade the Sell glyph is drawn at rgba(233,233,237,.18)  [getBoundingClientRect and getComputedStyle on .tk-rowbtn]
- Keyboard tab order on the Trades page: Orders, Notifications, Filters, Menu, five tabs, the scroll box, then body; 0 focusable rows (tr[tabindex], tr[role=button], tr a)  [40 Tab presses recording document.activeElement]
- Escape with a 1Y date preset set and focus on the body: chip 'Date 1Y' remains after Escape  [set 1Y via the popover, click the page, press Escape, read .chip texts]
- Order ticket while not connected: quote card dashes, 'Not connected.' text, Review enabled, review step reached with Limit '—' and Submit enabled  [click 'Buy AAPL' on the holding page, read the panel's text, click Review]
- Table header contrast (Nocturne): th rgba(233,233,237,.6) at 10.5 px on rgb(35,37,50) ≈ 5.5:1; dim cells .68 higher  [getComputedStyle, WCAG relative-luminance calculation]
- Text sizes in use on Trades / Markets: body 12.5 px (356 elements), labels 10–11 px, header 10.5 px uppercase, sort arrows 9 px; tiles 21 px  [computed font-size histogram over leaf elements]
- Responsive rules in the stylesheet: 0 width-based @media rules (only two prefers-reduced-motion rules, app.css:138 and :434)  [grep '@media' over web/src]
- Action-notice duration: 4000 ms for every kind including 'err'; order-placed notice 10000 ms  [web/src/lib/ui.svelte.ts:92 and ticket.svelte.ts:298]
- Trades list P&L sort key vs displayed value: sorted by pnl_cad, displayed in the trade's currency (UBER: $784.00 shown, $1,166.65 on the dashboard)  [rust/crates/server/src/views.rs:643, Trades.svelte:77, screenshots]

## examined (B)
Read in full: SPEC.md §2 (Versions, Notifications, Freshness), §3, §4 (Dashboard through Disclosures), §5, §6 (Layout, Motion), §7; docs/decisions.md, docs/design-review.md, docs/parity.md, docs/old-app-mistakes.md, docs/future.md. Page source read end to end: web/src/App.svelte, app.css, index.html (served), lib/router.svelte.ts, ui.svelte.ts, state.svelte.ts, subs.svelte.ts, filters.svelte.ts (activeCount/reset), fmt.ts, theme.svelte.ts, escape.ts, cuttip.ts, scrollbars.ts, listing.svelte.ts, actions/roll.ts, focus.ts, atEnd.ts, tradeChart.ts (label rule), Menu, Modals, ConfirmDialog, Skeleton, Empty, LoginView, Dashboard, Trades, TradeDetail, Portfolio, Cashflow, FilterPopover, Markets, heatmap/Heatmap, markets/Watchlist, MarketTiles, FearGreed, trade/Disclosures, ticket/OrderTicket, orders/OrdersPanel, notes/NotesPanel, notes/notes.svelte.ts, notes/channel.svelte.ts; the heads of ticket/ticket.svelte.ts, orders/orders.svelte.ts, live.svelte.ts, markets/News.svelte, Shorts.svelte, trade/ShortInterest.svelte, Donut.svelte, EventEntry.svelte. Ran: the release server on 8802 via run-demo.sh (demo book, offline, dry orders) and two extra instances on empty homes (18802, 18803) for first run; Playwright scripts (scratchpad/uiux/*.cjs) that screenshotted every tab, a trade page, a holding page, the heatmap alone, the menu with both submenus, the filter popover (search, Date), the notifications and Orders panels, the ticket (form, review, dry submit, draft card), Add trade / Import CSV / Load folder / Clear data, at 1680, 1440, 1340, 1200, 1100, 1024, 900 and 768 px, in Nocturne, Midnight and Light; measured overflow/clipping per width, header scroll-off, focus order in modals and the ticket, focusable elements per page, ARIA roles/landmarks, contrast ratios per theme, the filter badge/Escape behaviour per filter kind, and the Add-trade flow on an empty and a populated book (all servers stopped afterwards). Not covered: the streamed Wealthsimple sign-in view (needs a real session; code read only), Disclosures with real filings and the document reader (offline: only the unavailable/empty states rendered), News/Shorts/Fear & Greed with data (offline), the market heatmap universes, browser notification banners (no permission in headless), tile drag-reorder, the update button and protocol-mismatch line, and the e2e/cargo suites (not run, by instruction).

## measurements (B)
- Add trade on an empty book: page text after the 'Trade added' flash: unchanged ('No activity yet … Connect Wealthsimple') at +1 s, +3 s, +8 s; the trade shows only after a reload  [firstrun3.cjs on an empty BAGHOLDER_HOME (port 18803), reading #page innerText after the modal closed]
- Add trade on a Trades list already showing two rows: rows unchanged (TD, SHOP) at +5 s; Dashboard tile '0 trades'; the new row (ENB) appears after a tab switch and back, or a reload  [firstrun4.cjs, counting table tbody rows and their symbol cells]
- Focus when Add trade / Clear data open; Tab stops before reaching the modal: activeElement = BODY; 12 page controls (five tabs, 1Y chip, ×, P&L, Value, three benchmark pills) before the modal's Close; #confirmDlg and #modalDlg role=null aria-modal=null  [overlays.cjs pressing Tab 14 times and recording activeElement and closest('#modalDlg')]
- Focus with the order ticket open: stays on the page's button; 6 Tabs never enter .tk (reach the TradingView logo link and the thesis textarea behind the scrim)  [ticket.cjs]
- Keyboard-reachable elements on the Trades page; row attributes: 0 focusable (a/button/[tabindex]) inside #page; tr tabindex=null role=null  [kbd.cjs querySelectorAll on #page]
- ARIA/landmarks on the Dashboard: h1 0, nav 0, main 0, tablist 0, aria-sort 0 of 13 th, th[scope] 0, title attributes 0, buttons without a name 0, lang=en  [kbd.cjs]
- Header text read by assistive tech: every column header carries a '▼' glyph (transparent when not sorted): 'OPEN ▼ CLOSE ▼ SYMBOL ▼ …'  [innerText of #page after reload (firstrun2.cjs)]
- Width media queries in app.css: 0 (two @media rules, both prefers-reduced-motion)  [grep @media web/src/app.css]
- KPI tile texts clipped (scrollWidth > clientWidth) per width: 1100: 1 (dashboard); 1024: 2 dashboard / 3 portfolio; 900: 3 dashboard / 6 portfolio; 768: 8 dashboard / 9 portfolio  [widths.cjs, .kpi .v/.s scrollWidth vs clientWidth]
- Elements past the viewport's right edge on Markets: 900 px: 5 (heatmap legend spans and the arrows-out button, right edge up to 1009 px); 768 px: 5  [widths.cjs getBoundingClientRect().right > clientWidth]
- Tables scrolling sideways inside their card at 1200 px: Trades 1150 in 1065; Holdings 777 in 695; Cashflow Positions 1074 in 1061; Distribution history 560 in 506; none at 1340 and above  [widths.cjs table.scrollWidth vs scroll container clientWidth]
- Header position and its bottom edge at the foot of each page (1440×900): #hdr position:static; Dashboard −241 px (docH 1223), Cashflow −245, Markets −112, holding page −107  [hdr2.cjs after window.scrollTo(bottom)]
- Flash notice lifetime and overwrite: 4000 ms, cleared by timer; a later flash replaces the text  [ui.svelte.ts:92-101 read; syncline observed reverting to 'Not connected' by +3 s in firstrun3.cjs]
- Filter badge dot and Escape-to-clear by filter kind: preset 1Y: dot false, Escape leaves the chip; symbol UBER: dot true, Escape clears; year 2025: dot true, Escape clears  [esc2.cjs]
- Duplicate-symbol presentation on Portfolio: two 'NVDA' rows in Holdings; two 'NVDA' legend entries in Allocation (4.4 %, 2.7 %); account only in data-tip  [s1-portfolio.png and Portfolio.svelte:147]
- Order ticket while not connected: Buy button live; quote card '— — Bid — Mid — Ask —' + 'Not connected.'; Review enabled; review rows all '—'; 'A limit price is required.' appears only after Submit  [ticket.cjs innerText of .tk at each step]
- Contrast ratios (text on its card) — Light: gain table cell 4.09:1 @12.5 px; loss KPI value 4.42:1 @21 px; table th 4.38:1 @10.5 px; .lbl 5.31:1 @10 px  [contrast.cjs computing WCAG relative luminance from computed colours with alpha blended on the card background]
- Contrast ratios — Nocturne / Midnight: th 5.47 / 6.03; .lbl 5.73 / 6.03; gain value 8.30 / 9.91; loss value 5.19 / 6.76  [contrast.cjs]
- Smallest type sizes in use: 10 px (.lbl, axes, card footers, year list), 10.5 px (table headers), 11 px (tile subtitles)  [app.css:66,74; Dashboard.svelte:242,258,296]
- Trade-chart marker labels on the SOL holding: 23 executions in view, every one labelled '−0.220000'; labels overprint (rule: labels on while ≤40 in view, no collision test)  [s1-trade.png; tradeChart.ts:107-110]
- First-run page and tab gating: arrival 1015 ms after navigation; Dashboard, Trades, Portfolio, Markets and Cashflow all render 'No activity yet / Connect Wealthsimple'  [firstrun.cjs on an empty home]
- Hash router history behaviour: history.length 5 after load + four hash changes; ArrowRight moved #dashboard → #trades  [kbd.cjs]
- Console/page errors during all drives: 0 page errors, 0 console errors  [pageerror/console listeners in shots1.cjs, overlays.cjs, ticket.cjs]

## findings

### uiux-A-1 [high/robustness] Order ticket reaches Review and Submit with no connection, no quote and no price, and rewrites a bad quantity to 1 without a word
- status: confirmed — code: corrected (Every cited line is accurate (review() at 77, Review button at 192, Submit at 220, t.error at 107, submit() at 273) and the scenario reproduced on the demo book: on AAPL's holding page Buy opened a ticket reading 'Not co) | std: confirmed
- standard: Every brokerage ticket validates before review: IBKR Client Portal and Wealthsimple's own web ticket keep Review/Submit disabled until a Limit or Stop order carries a price and the account is connected, and show the field error inline; with no session there is no ticket at all. WCAG 3.3.1/3.3.3 and NN/g form-validation practice: the error is shown at the field, and a typed value is never silently replaced.
- observed: On the demo book (not connected) the holding page's Buy opens a ticket that reads 'Not connected.' with dashes for last, bid, mid and ask, yet Buy/Sell, every field, Review and Submit are live. Review is reached with Limit price '—'; the review screen reads 'Buy 1 AAPL · Limit — · Day · Trading', 'Stop loss Market at —', every figure a dash, and Submit is enabled; 'A limit price is required.' appears only after Submit, from the server. Typing 'abc' in Shares and leaving the field silently makes it '1'.
- evidence: web/src/lib/ticket/OrderTicket.svelte:77 (review() only forces qty to 1, then t.step = 'review'; nothing checks price, quote or conn); web/src/lib/ticket/OrderTicket.svelte:192 (the Review button carries no disabled state); web/src/lib/ticket/OrderTicket.svelte:220 (Submit is disabled only while t.busy); web/src/lib/ticket/OrderTicket.svelte:107 ('Not connected.' is rendered as text under the quote card; the form below stays interactiv); web/src/lib/ticket/OrderTicket.svelte:65 (onBlur: an invalid quantity becomes 1 and an invalid limit becomes null, silently (measure); web/src/lib/ticket/ticket.svelte.ts:273 (submit() posts whatever the ticket holds (limitPrice may be null) and shows the server's r)
- impact: A person builds a whole order, reads a review made of dashes and presses Submit before learning the app was never in a state to send anything; a Limit order with no price is a refusal after the fact rather than a field error; a mistyped size becomes 1 share without notice, which on a live account is a wrong order. On the one path where a wrong click costs money, the ticket is the least guarded screen in the app.
- proposal: Gate the entry points and the steps: no Buy/Sell affordance (or a disabled one with the reason) while status.connected is false; Review disabled until qty > 0, a price is present for Limit/Stop/Stop-limit and a quote has answered; per-field inline errors on blur instead of rewriting the value; Submit disabled while the preview has not answered. Keep the server refusal as the last line, not the first.
- ux-conflict: True — 
- owner decision: -
- confidence: high

### uiux-A-2 [high/design] The same trade's P&L is two different unlabelled dollar figures on two screens, and the column sorts by a third
- status: confirmed — code: corrected (Accurate in substance and reproduced: UBER shows '$784.00' (FX column 'USD') in the Trades list and '$1,166.65' in both the Review queue and By symbol, no currency mark anywhere; sorting the list by P&L descending puts ') | std: confirmed
- standard: Leading products never show two currencies under one '$' without a mark: Wealthsimple prefixes 'US$' on every USD figure; IBKR shows a currency column per row and a base-currency total; Sharesight shows currency codes and a base-currency view; Tradervue and TradeZella convert every trade to the account's reporting currency so one trade has one P&L everywhere. A sortable column sorts by the value it displays.
- observed: UBER (USD) reads '$784.00' in the Trades list and on its trade page, and '$1,166.65' in the dashboard's Review queue and By symbol; neither figure carries a currency, the only hint is the FX column reading 'USD'. The Trades list's P&L column is sorted on the server by pnl_cad while the cells show the native figure: sorted descending, NVDA $1,170.00 (USD) sits above HOOD $1,320.00 (USD) and COIN −$1,566.00 above ETH −$2,010.00 (CAD).
- evidence: web/src/lib/fmt.ts:69 (money() takes a currency argument that is 'deliberately ignored' (comment at line 11): not); web/src/lib/Trades.svelte:77 (the list shows money(t.pnl, t.currency): the trade's own currency); rust/crates/server/src/views.rs:643 (the 'pnl' sort key is t.pnl_cad, not the pnl the column shows); web/src/lib/Dashboard.svelte:371 (By symbol shows money(r.pnl), the engine's CAD sum (scope.rs:594-597)); web/src/lib/Dashboard.svelte:389 (the Review queue shows money(r.pnl)); rust/crates/server/src/wire/build.rs:532 (the queue row's pnl is fig_money(&t.pnl_cad))
- impact: A trader cross-checking the dashboard against the trade list sees two numbers for one trade and no way to tell which is which; sorting the list by P&L puts rows visibly out of order. Confusion here is a trust problem for every figure on the page.
- proposal: One reporting currency for every P&L column and card (CAD), with a per-instrument toggle (Native · CAD) where the native figure matters, and a currency mark ('US$', 'USD') wherever a figure is not in the reporting currency; sort on what is shown. Record the change against the 'never labelled' rule in SPEC §1.
- ux-conflict: True — 
- owner decision: docs/decisions.md 'Earlier': 'Per-instrument figures in the instrument's own currency; aggregates in CAD, never labelled.'
- confidence: high

### uiux-A-3 [high/ui-ux] The equity curve and Monthly P&L do not use a time axis: points are spaced by index and empty months are dropped
- status: confirmed — code: corrected (Accurate: Dashboard.svelte:45 places points at (i/n)*W, line 76 picks labels from evenly spaced indices, scope.rs:581-590 builds months only from realized parts, line 315 draws one bar per entry; on the demo the axis rea) | std: confirmed
- standard: Every equity curve in the category (Tradervue, TradeZella, TraderSync, Edgewonk) plots cumulative P&L against calendar time, flat where nothing was realized, so the slope reads as rate of return; every monthly P&L chart shows every calendar month including zero months (Tradervue's monthly report, TradeZella's calendar). The app's own Cashflow chart already does this (every month from the first payment).
- observed: In P&L mode the equity curve has one point per realization day placed at equal x intervals, so hovering at 25% of the width reads a date ten months in while 50% reads a date seventeen months in over an Apr '24 to Jul '26 axis; the six axis labels are the dates at evenly spaced indices (measured: Apr '24 · Nov '24 · Jan '25 · Mar '25 · Jun '25 · Jul '26). Monthly P&L draws 14 equal bars for a 28-month span: months with no sale do not exist on the chart, so a seven-month gap reads as one bar next to the next.
- evidence: web/src/lib/Dashboard.svelte:45 (pts = vals.map((v, i) => [(i / n) * W, y(v)]): x is the point's index); web/src/lib/Dashboard.svelte:76 (the six date labels are the dates at evenly spaced indices, not evenly spaced dates); rust/crates/engine/src/scope.rs:581 (months is a BTreeMap keyed by (year, month) filled only from realized parts: no entry for ); web/src/lib/Dashboard.svelte:315 (one bar per entry in ms, equal width, no fill for missing months); rust/crates/engine/src/scope.rs:859 (the cashflow series, by contrast, walks every calendar month from the first payment to the)
- impact: The slope of the curve and the spacing of the bars, which is what a trader reads them for, are wrong: an idle year is as wide as a busy week, drawdown durations cannot be read, and a run of losing-nothing months disappears. The dashboard's two headline charts mislead on real data with any irregular trading rhythm.
- proposal: Plot the P&L curve on a date axis (x = days since the first point, stepping at each realization, held flat between), with axis labels at even calendar intervals; have the engine emit every calendar month from the first to the last (or to the date range's end) with a zero value, as the cashflow series already does, and draw zero months as empty slots. One shared time-axis helper for the three charts.
- ux-conflict: True — 
- owner decision: -
- confidence: high

### uiux-A-4 [medium/ui-ux] Open trades read '$0.00' P&L in gain green, and By symbol shows '0 trades' with a five-figure P&L
- status: confirmed — code: corrected (Reproduced: every open trade row (SOL, NVDA, XEQT, VFV, MSFT, TD, AAPL, the AAPL call) shows P&L '$0.00' and P&L % '—' both coloured rgb(111,211,155), and By symbol lists BTC with $14,840.00, 0 trades, '—', '—'. The evid) | std: corrected (The observation is right (measured: SOL, QQQT and an AAPL call show '$0.00' in rgb(111,211,155); By symbol lists 'BTC | $14,840.00 | 0 | — | —' plus ten ZZ rows at '$0.00 | 0'), but the $0.00 and the 0-trade count are th)
- standard: Tradervue and TraderSync list open positions with their unrealized (mark-to-market) P&L or a blank, never a realized zero; a zero is shown in the neutral colour (Wealthsimple, IBKR); a per-symbol table counts the trade whose partial sales it credits.
- observed: Every open trade in the Trades list shows P&L '$0.00' coloured rgb(111,211,155) (the gain green) and P&L % '—' in the same green, while the same holding's page shows a large unrealized figure (SOL: +$912.86 / +11.7%). The dashboard's By symbol lists BTC with P&L $14,840.00, Trades 0, Win rate —, Avg hold —, because its realized part came from a still-open trade.
- evidence: web/src/lib/Trades.svelte:77 (money(t.pnl) with the server's pnl '0' for an open trade; both cells coloured with color(t); web/src/lib/fmt.ts:175 (color(): 'zero reads as positive, as the page does' (s >= 0 → var(--pos))); web/src/lib/Dashboard.svelte:372 (By symbol row shows r.n (0) beside r.pnl; the API row is {symbol: 'BTC', pnl: '14840', n: ); rust/crates/engine/src/scope.rs:594 (the by-underlying P&L sums every realized part in scope while the count (n) counts closed )
- impact: An open position that is deep in the red reads as a green $0.00; a symbol with a large P&L and zero trades reads as a data error. The list's most-scanned column says nothing true about a third of the rows on the demo book.
- proposal: For an open trade show its unrealized P&L (the positions document already carries unreal and unrealPct for the same holding) marked open, or '—' with the realized part in the subtitle; neutral colour for zero everywhere; in By symbol count open trades that realized in scope, or split the row into 'closed' and 'open' counts.
- ux-conflict: True — 
- owner decision: -
- confidence: high

### uiux-A-5 [medium/ui-ux] The Trades list is a fixed 620 px box inside an otherwise empty page, with no count, totals or Side column
- status: confirmed — code: confirmed | std: confirmed
- standard: Tradervue, TradeZella and TraderSync give the trade list the page (paged or virtualised, headers sticky), print the count of trades in scope and a total P&L for the filtered set, and carry a Side (long/short) column; IBKR's Trades tab does the same. A scroll box shorter than the window is a spreadsheet embedded in a page, not a list.
- observed: At 1440×1200 the Trades card is 620 px tall (card bottom at 760 px of 1200) with 440 px of empty panel beneath while all 31 rows are loaded and 15 are visible; the same cap holds at 900 px windows (roughly 120 px dead). There is no row count, no total for the filtered set, and the columns run Open · Close · Symbol · Exchange · Qty · Entry · Exit · FX · P&L · P&L % · Hold · Grade · Tags with no Side, although the book carries shorts and option sides (SELL, COVER).
- evidence: web/src/lib/Trades.svelte:45 (max-height:620px on the card); web/src/lib/Trades.svelte:17 (the column list: no side, no fees, no count row); web/src/lib/subs.svelte.ts:90 (FIRST_ROWS = 100: rows page inside the 620 px box by scrolling)
- impact: On a desktop monitor most of the screen is unused while the trader scrolls a small window; there is no way to see how many trades a filter matched or what they add up to without going back to the dashboard.
- proposal: Let the card take the viewport (max-height: calc(100vh − header − tabs − padding)) or drop the cap and let the page scroll; a footer line 'N trades · Realized P&L $X' for the set in scope; add Side.
- ux-conflict: True — 
- owner decision: -
- confidence: high

### uiux-A-6 [medium/design] No layout below 1340 px: a control clips at 1200, tiles clip their figures at 1000 and 800
- status: merged — code: refuted (Same root cause as uiux-B-4 (no width breakpoint in app.css; fixed six-column tile grids), which is kept because it carries the broader measurements and the correct conflict note. A-6's one distinct observation, the News) | std: corrected (Measurements hold (News search input 37 px past its card at 1200, exactly as stated; KPI values clip 6 px at 1000 and 39 px at 800), but the finding is the same root cause as B-4 and sets conflicts_with_current_ux=false )
- standard: Responsive reflow with two or three breakpoints (CSS Grid auto-fit/minmax) is baseline for a web product opened in a window the person sizes; TradeZella, IBKR Client Portal and Wealthsimple collapse tile rows and stack cards under roughly 1100 px.
- observed: app.css has no width media query (its only two are prefers-reduced-motion). Measured on the demo: at 1200 px the News card's search input ends 37 px past the card's right edge; at 1000 px the Realized P&L tile value overflows 6 px; at 800 px two tile values overflow (39 px, 6 px) because .kpi .v is white-space:nowrap in a repeat(6,minmax(0,1fr)) row.
- evidence: web/src/app.css:138 (the first of only two @media rules; both prefers-reduced-motion); web/src/lib/Dashboard.svelte:217 (six tiles at every width); web/src/lib/Markets.svelte:22 (minmax(0,1fr) 420px: the News header takes what is left); web/src/app.css:250 (.kpi .v white-space:nowrap in a fixed 87 px tile)
- impact: Beside a broker window or on a small laptop the money figures clip and a control is unreachable.
- proposal: See B-4: three breakpoints, tiles on auto-fit, card pairs stacking under ~1100 px, header rows wrapping; add 1024 to SPEC §7's widths.
- ux-conflict: True — SPEC §4 fixes the six-column grid and §6/§7 verify only 1200–1680 px; reflowing below 1340 changes the layout the owner sees at 1200.
- owner decision: -
- confidence: high

### uiux-A-7 [medium/ui-ux] The two buttons that place orders are 22 px icon targets at half opacity
- status: confirmed — code: corrected (Three findings in one. The keyboard-reach part (rows, headers, cards not focusable) is uiux-B-3's root cause and the preset/Escape part is uiux-B-8's; both are kept there. What is A-7's own, and verified, is the target s) | std: corrected (Every observation checks out (0 focusable elements in the Trades page; Buy/Sell measured 22×22 at rgba(233,233,237,.5), the off state at .18 ink per app.css:323; the 1Y chip left standing after Escape with no dot), but t)
- standard: WCAG 2.2 SC 2.5.8 Target Size (Minimum): 24×24 CSS px; Apple HIG and Material set 44/48 px for primary actions. Brokerage screens label the order actions in words (Wealthsimple 'Buy'/'Sell' buttons, IBKR 'Buy'/'Sell' in the ticket bar) and never dim them below the page's body ink.
- observed: On a holding's page Buy and Sell are 12 px plus/minus glyphs inside 22×22 px buttons drawn at rgba(233,233,237,.5) (measured), with a Sell that does not apply drawn at 18% opacity; they sit beside a 24 px P&L figure and are the smallest, faintest controls on the page.
- evidence: web/src/lib/TradeDetail.svelte:251 (Buy/Sell: a 12 px svg in .tk-rowbtn with only an aria-label); web/src/app.css:322 (.tk-rowbtn: width 22px, height 22px, colour rgba(var(--ink-rgb),.5)); web/src/app.css:323 (.tk-rowbtn.off: colour at .18)
- impact: The one action that costs money is the hardest control on the page to see and to hit, below the WCAG minimum target size.
- proposal: Labelled 28 px-plus buttons ('Buy', 'Sell') in the page's button style, the inapplicable one disabled with its reason rather than faded.
- ux-conflict: True — 
- owner decision: -
- confidence: high

### uiux-A-8 [medium/ui-ux] First run blocks the Markets tab and offers only the Wealthsimple path
- status: merged — code: refuted (Same root cause as uiux-B-10 (showingEmpty gates every tab including Markets; the empty page offers only Connect Wealthsimple), which is kept because it also covers the permanent 'Not connected' status and names the owne) | std: corrected (Code confirms it (App.svelte:252 showingEmpty from activityCount, :495 Empty before every tab branch, Empty.svelte names only Wealthsimple), but the empty page is the owner's recorded decision of 2026-09-28 and SPEC §4 ')
- standard: Onboarding in Tradervue, TraderSync and TradeZella presents broker sync and CSV import side by side; a screen that needs no account data (indices, heatmap, news, watchlist) is usable before any account is connected, as on Wealthsimple's and Questrade's own sites.
- observed: With activityCount 0 every tab, Markets included, renders the 'No activity yet / Connect Wealthsimple' page; Import CSV, Load folder and Add trade exist only in the menu. Nothing on the Markets tab depends on the book's activities.
- evidence: web/src/App.svelte:252 (showingEmpty = !!book.data && !book.data.activityCount, applied to every tab); web/src/App.svelte:495 ({#if showingEmpty} renders <Empty> before any tab branch); web/src/lib/Empty.svelte:16 (copy names only Wealthsimple); web/src/lib/Menu.svelte:45 (Import CSV, Add trade and Load folder are menu items only)
- impact: A new person cannot try the market tools or a CSV before handing over a brokerage login.
- proposal: Markets renders whenever a book document exists; the empty page offers Connect Wealthsimple · Import CSV · Add trade; the other tabs keep their own silhouette (see B-10).
- ux-conflict: True — SPEC §4 'The page with nothing to show' defines the Connect-only page.
- owner decision: docs/decisions.md 2026-09-28: 'No screen ever opens with nothing to show, except on the very first open, before Wealthsimple has been connected or a CSV read'
- confidence: high

### uiux-A-9 [medium/robustness] Failures of the person's own actions are shown for four seconds and never filed
- status: merged — code: refuted (Same root cause as uiux-B-5 (flash() is a 4 s line in a header that scrolls away; failures of the person's own writes get no persistent state), which is kept because it adds the measured non-sticky header and the journal) | std: corrected (The mechanism is as described (ui.svelte.ts:92 one 4000 ms timer for every kind; state.svelte.ts:123 a failed journal save is a flash and a resync; NotesPanel lists server notifications only) and B-5 reports the same roo)
- standard: Nielsen heuristic 9 and Material's snackbar guidance: an error that needs action persists until dismissed or resolved; an app with a notification centre files it there. The app already keeps sync errors in the header until their next success (decision 2026-09-27); action errors get no such treatment.
- observed: flash() defaults to 4000 ms for 'ok' and 'err' alike and every action failure uses it (journal save, trades read, export, disconnect, refresh, connect-cancel); the Notifications panel never receives a page-side failure; a failed journal save leaves the typed text on screen looking saved.
- evidence: web/src/lib/ui.svelte.ts:92 (flash(msg, kind = 'ok', ms = 4000): one duration for every kind, replaced by any later fla); web/src/lib/state.svelte.ts:123 (a failed journal save is a 4-second flash; the draft is not marked unsaved); web/src/lib/notes/NotesPanel.svelte:48 (the panel lists only server notifications)
- impact: A thesis lost while the person looked away for four seconds; the one place errors could be reviewed reads 'Nothing yet.'
- proposal: 'err' persists until dismissed or the same action succeeds, with the header's existing copy icon; a failed write keeps its field unsaved with retry; every err flash is appended to the notifications history (see B-5 for the sticky header).
- ux-conflict: True — SPEC §4: a notice 'takes the sync status's place for four seconds'.
- owner decision: docs/decisions.md 2026-09-27: 'An error never breaks the page's layout: the header's error stays one line…' (compatible: a persisting one-line error with a dismiss)
- confidence: high

### uiux-A-10 [medium/ui-ux] Holdings rows and Allocation slices repeat the same symbol with the account only on hover
- status: merged — code: refuted (Same root cause as uiux-B-6 (per-position Holdings rows and Allocation slices with the account only in a hover tip), which is kept. A-10 also states 'the heatmap likewise shows a per-account tile', which is false: held_t) | std: confirmed
- standard: IBKR's Portfolio shows an Account column or groups by account; Sharesight consolidates a holding across portfolios and names the portfolio on the row; Wealthsimple's all-accounts holdings view marks each row's account. A donut never has two slices with the same label.
- observed: Two NVDA rows sit in Holdings with different Avg and P&L and nothing to tell them apart except a hover tooltip (data-tip) that touch and keyboard never see; the Allocation legend reads NVDA 4.4% and NVDA 2.7% as two slices of the same colour family. The heatmap likewise shows a per-account tile.
- evidence: web/src/lib/Portfolio.svelte:70 (allocItems label = x.label (the symbol); positions are per account so labels repeat); web/src/lib/Portfolio.svelte:147 (the account is data-tip only, by design comment: 'not shown in the row'); web/src/lib/Donut.svelte:83 (the legend and centre show the label as given; no account, no grouping)
- impact: With the common case of the same ETF held in a TFSA and an RRSP, the Portfolio tab is unreadable without hovering each row; the allocation ring overstates the count of holdings and understates concentration in one name.
- proposal: Allocation consolidated per instrument (sum of the accounts' values) with the account split in the hovered centre; Holdings with an Account column or grouped under account headers (sortable), the tooltip kept as a bonus, not the only carrier.
- ux-conflict: True — 
- owner decision: SPEC §4 Portfolio, Holdings: 'the account is not shown in the row, and hovering the row names it'
- confidence: high

### uiux-A-11 [medium/ui-ux] The holding page shows no price or day change
- status: confirmed — code: confirmed | std: corrected (Confirmed at runtime: the NVDA holding's header reads '$2,636.80 / +66.7%' (unrealized P&L) and the facts are Qty, Avg, Book, Market, Hold, Account; SOL's the same; no last price or day change anywhere on the page while )
- standard: Wealthsimple, IBKR and Questrade lead a holding's page with the last price and the day's change, then the position's figures; the app's own listing page does exactly that (TradeDetail.svelte:274) and its Holdings row carries Last and Change.
- observed: A held instrument's page shows the unrealized P&L top right and Qty · Avg · Book · Market · Hold · Account; last price and day change appear nowhere on it (measured on NVDA and SOL), although the positions document that draws it carries last, dayChange and percentChange.
- evidence: web/src/lib/TradeDetail.svelte:274 (trade.last and percentChange rendered only in the {#if trade.listing} branch); web/src/lib/TradeDetail.svelte:312 (the holding branch's facts: Qty, Avg, Book, Market, then Hold and Account); web/src/lib/Portfolio.svelte:150 (the same position's row shows px(r.last) and the day change)
- impact: The screen a holder opens to decide whether to add or sell hides the price they would decide on; Buy/Sell sit beside a P&L with no quote.
- proposal: Price and day change beside the P&L as the listing page draws them; Last in the facts row.
- ux-conflict: True — SPEC §4 'A holding' fixes the header figure as the unrealized P&L and the six facts.
- owner decision: SPEC §4 A holding; docs/decisions.md 'Earlier': 'Only what was asked'
- confidence: high

### uiux-A-12 [medium/design] Journal and analytics are a single thesis box, a grade and tags that are never analysed; no calendar
- status: confirmed — code: corrected (Substance accurate: the dashboard's six cards (Dashboard.svelte:215-395) are KPI tiles, equity, annualized returns, Monthly P&L, Grade vs P&L, By symbol and Review queue, nothing keyed by tag, weekday, side or hold; scop) | std: corrected (The absence is real (Dashboard.svelte:214 six cards; scope.rs:592–610 aggregates by underlying and grade only; TradeDetail.svelte:335 one textarea plus grade and tags) and the finding names both owner decisions ('Only wh)
- standard: Every product in the category makes the calendar of daily P&L the dashboard's centrepiece (Tradervue, TradeZella, TraderSync) and reports by tag/setup, day of week, time of day and hold time (Tradervue Reports, TradeZella Reports, TraderSync Analytics, Edgewonk's tiebreakers); trade notes are timestamped entries with images and a pre-trade plan distinct from the post-trade review (Tradervue, TradeZella templates).
- observed: The dashboard has an equity curve, annualized returns, Monthly P&L, Grade vs P&L, By symbol and a Review queue; there is no calendar and no breakdown by tag, side, kind, weekday or hold time. Tags are collected on every trade (with autocomplete) but no card or table reads them back; the trade page's journal is one textarea ('Why did you take this trade?') plus grade and tags, no dated entries, no attachments.
- evidence: web/src/lib/TradeDetail.svelte:359 (the journal: one textarea bound to thesisDraft, then Grade and Tags); web/src/lib/Dashboard.svelte:215 (the six cards (215-395); nothing keyed by tag, weekday, side or hold); rust/crates/engine/src/scope.rs:592 (the dashboard aggregates by underlying and by grade only)
- impact: The product is called a trading journal, and the two things a trader opens a journal for, seeing the month at a glance and learning which setups make money, are not there; tagging is effort with no return.
- proposal: A monthly calendar card (daily P&L cells, click filters to the day) in the equity row or as a Dashboard switch, a 'By tag' table beside By symbol on the same grammar, and a journal of dated entries under the thesis. Each is a definition to add to SPEC §4 with the category source.
- ux-conflict: True — 
- owner decision: docs/decisions.md 'Earlier': 'Only what was asked: nothing on screen the owner did not ask for'; 2026-09-26 'The rewrite is a migration: nothing new is built in it'
- confidence: medium

### uiux-A-13 [low/ui-ux] The Distribution per-unit column mixes two and four decimals row by row
- status: confirmed — code: confirmed | std: confirmed
- standard: A numeric column keeps one precision so decimals align (IBKR's dividend per share, Sharesight's payout tables); the category's tables show per-unit distributions to four places consistently.
- observed: Cashflow Positions and Distribution history show '$0.9750', '$1.05', '$0.1900', '$0.3800' in one column: four decimals under a dollar, two above, so the decimal points do not line up and $1.05 reads as a rounder figure than $0.9750.
- evidence: web/src/lib/Cashflow.svelte:20 (perUnit: absBelow(v,'1') ? 4 : 2 decimals, per row)
- impact: A per-unit rate is compared across rows; mixed precision makes the column harder to scan and implies precision the source did not state.
- proposal: One precision for the column (four, as payers publish), or the payer's stated precision applied to every row alike.
- ux-conflict: True — 
- owner decision: -
- confidence: high

### uiux-B-1 [high/robustness] A person's own Add trade does not reach the screen they are looking at
- status: confirmed — code: confirmed | std: confirmed
- standard: Visibility of system status (Nielsen heuristic 1) and the owner's own decisions of 2026-09-27/28 ("replaces only what changed", "no screen ever opens with nothing to show"): every leading journal (Tradervue, TradeZella, TraderSync) shows a manually entered trade in its list the moment the save is confirmed, without a reload. The app's docs/architecture.md §13 also promises that "a change made here reaches the page because it was committed".
- observed: On a fresh, empty book, Menu → Add trade → SHOP 10 @ 100 → the header flashes "Trade added" and the modal closes, but the page keeps reading "No activity yet — Connect Wealthsimple" at +1 s, +3 s and +8 s; only a reload shows the trade. On a book already showing two trades, adding ENB flashes "Trade added" and the Trades list still lists only TD and SHOP 5 s later; the Dashboard tile reads "0 trades"; the new row appears only after switching tab and back (which re-subscribes) or a reload.
- evidence: web/src/lib/ui.svelte.ts:283 (saveTrade posts /api/entries, closes the modal and flashes; nothing else is asked for — th); web/src/App.svelte:252 (showingEmpty is derived from book.data.activityCount, which did not change after the entry); web/src/App.svelte:487 (the empty page stays for as long as showingEmpty holds; on the first run every tab is this); web/src/lib/state.svelte.ts:16 ("Nothing polls and nothing is refetched to see the result of a change: a change made here )
- impact: The first thing a CSV/manual user does looks like it silently failed: "Trade added" then the same empty page. On a working book, each manual entry is invisible until the person happens to change tab or reload; they will add it twice.
- proposal: Treat the person's own write as a change the page must show: either have the entries route bump the book and trades documents so the stream sends them (server side, the data-flow reviewer's area), or, as every top product does, apply the confirmed entry to the shown list at once and reconcile when the stream catches up. Add a browser test: on an empty book, Add trade → the Trades list shows the row without a reload.
- ux-conflict: False 
- owner decision: -
- confidence: high

### uiux-B-2 [high/ui-ux] Dialogs, panels and popovers are not dialogs: focus is neither moved in, trapped, nor returned
- status: confirmed — code: confirmed | std: confirmed
- standard: WAI-ARIA Authoring Practices, Dialog (Modal) pattern: on open, focus moves into the dialog; Tab and Shift+Tab cycle inside it; on close, focus returns to the element that opened it; the element has role=dialog and aria-modal=true. WCAG 2.2 SC 2.4.3 Focus Order and 2.1.2 No Keyboard Trap. Radix, Headless UI and every brokerage web app (IBKR Client Portal, Wealthsimple) do this.
- observed: Opening Add trade leaves focus on BODY; pressing Tab walks 12 controls on the page behind (Dashboard, Trades, Portfolio, Markets, Cashflow, the 1Y chip, ×, P&L, Value, S&P 500, S&P/TSX, TSX 60) before reaching the modal's Close. Clear data: focus on BODY, no role, no aria-modal. The order ticket declares role=dialog but not aria-modal; six Tabs from an open ticket land on the page's TradingView logo link and the thesis textarea behind the scrim. Closing the menu or the filter with Escape leaves focus on BODY, not on the button that opened it.
- evidence: web/src/lib/Modals.svelte:32 (#modalDlg is a div with role=presentation; no role=dialog, aria-modal, initial focus or tr); web/src/lib/ConfirmDialog.svelte:46 (#confirmDlg: same, and the destructive button is not focused or announced); web/src/lib/ticket/OrderTicket.svelte:82 (role=dialog without aria-modal; nothing focuses the first field; the scrim only blocks poi); web/src/lib/orders/OrdersPanel.svelte:50 (Orders panel: role=dialog, no focus management); web/src/lib/notes/NotesPanel.svelte:57 (Notifications panel: same); web/src/lib/Menu.svelte:36 (menu is a div of buttons, no role=menu, no arrow-key movement, submenus open on mouseenter)
- impact: A keyboard user cannot get into a dialog without tabbing across the whole page, can act on the page under a modal (change tabs, remove a filter) while it is open, and loses their place on close. A screen reader announces none of the dialogs as such.
- proposal: One overlay primitive used by the modals, the confirm, the three side panels, the filter popover and the menu: role=dialog + aria-modal + aria-labelledby, focus moved to the first field or the primary action on open, Tab cycled inside, inert applied to the page behind (the `inert` attribute is in every current browser), focus restored to the opener on close, Escape handled in the primitive rather than in App.svelte's cascade. docs/future.md already names "one overlay manager, accessibility" as deferred; this is the concrete cost of that deferral.
- ux-conflict: False 
- owner decision: 2026-09-26 "The cutover keeps to the migration" defers "shared components, one overlay manager, accessibility" to docs/future.md
- confidence: high

### uiux-B-3 [high/ui-ux] Nothing in the tables, cards or charts can be reached or operated from the keyboard, and the tables carry no sort semantics
- status: confirmed — code: confirmed | std: corrected (Measured: 0 focusable elements in the Trades page; rows are <tr onclick> (Trades.svelte:63), headers <th onclick> with a transparent ▼ (line 54–57), queue and month cards role=presentation, the account hover-only. All as)
- standard: WCAG 2.2 SC 2.1.1 Keyboard, 4.1.2 Name, Role, Value; WAI-ARIA APG sortable-table pattern (a button in the header cell, aria-sort on the sorted column). Data-heavy products (IBKR Client Portal, Google Finance, Sharesight) make rows links or focusable buttons and headers buttons.
- observed: The Trades page has 0 focusable elements (measured): rows are <tr onclick> with no tabindex or role; By-symbol rows, Review-queue cards (role=presentation), Holdings rows, disclosure rows and the month/grade bars are the same. Headers sort on <th onclick> with a transparent ▼ in every header and no aria-sort. The holding's account is a mouse-only data-tip; the cut-text tip is mouseover-only. No h1, no nav/main landmark; the tab row is plain buttons without tablist or aria-current.
- evidence: web/src/lib/Trades.svelte:63 (<tr class="tab" style="cursor:pointer" onclick=…>: no tabindex, role or key handler); web/src/lib/Trades.svelte:54 (<th onclick={toggleSort}> with a transparent ▼ span; no aria-sort); web/src/lib/Dashboard.svelte:387 (review-queue card: role="presentation" onclick); web/src/lib/Dashboard.svelte:318 (month bar: role="presentation" onclick); web/src/lib/Portfolio.svelte:147 (holding row opens on click only; the account is a hover-only data-tip); web/src/lib/trade/Disclosures.svelte:190 (filing row expands on click only)
- impact: The journal cannot be used without a mouse; a screen-reader user hears every column as sorted; hidden information is unreachable without hover.
- proposal: An <a href="#trades/<id>"> in the symbol cell (every hash route already supports it), tabindex=0 + Enter/Space on cards; a <button> in each sortable <th> with aria-sort and the arrow aria-hidden; nav/main landmarks, a visually hidden h1, role=tablist with aria-current; a focus-reachable form of every hover-only tip.
- ux-conflict: False 
- owner decision: docs/decisions.md 2026-09-26 'The cutover keeps to the migration': 'shared components, one overlay manager, accessibility' deferred to docs/future.md
- confidence: high

### uiux-B-4 [medium/design] One fixed six-column layout at every width: values clip from about 1100 px down, a control overruns its card at 1200 px, and nothing reflows
- status: confirmed — code: corrected (app.css has exactly two @media rules (138 and 434), both prefers-reduced-motion; the tile grids are repeat(6,minmax(0,1fr)) (Dashboard 217, Cashflow 100) and .kpi .v is nowrap in a fixed 87 px tile (app.css:249-251). Re-) | std: confirmed
- standard: Responsive layout with breakpoints is baseline for any web product a person opens in a window they size themselves: Wealthsimple's web app, IBKR Client Portal and Sharesight all collapse their tile rows and stack cards below tablet width; CSS Grid auto-fit/minmax and two or three media queries are the standard means. A 13" laptop with a browser sidebar, a split-screen window or 125% scaling is under 1200 px.
- observed: app.css contains two @media rules, both prefers-reduced-motion; no width breakpoint anywhere. Measured: at 1200 px (a SPEC §7 verification width) the News card's search box ends 46 px past the card's right edge and the Trades table (min-width 1150 px) scrolls sideways inside its card; at 1000 px three KPI texts clip ('$36,316.87', 'W $32,488 · L $12,808', 'Avg W $2,953 · L −$1,423') and a Portfolio card overflows; at 800 px five KPI texts clip and five cards overflow. Every tile value is white-space:nowrap in a 1/6 column with no overflow handling.
- evidence: web/src/app.css:138 (one of the only two @media rules; the other is at line 434; neither is a width query); web/src/app.css:249 (.kpi fixed 87 px high; .v and .s white-space:nowrap — clip rather than wrap or shrink); web/src/lib/Dashboard.svelte:217 (grid-template-columns:repeat(6,minmax(0,1fr)) at every width; same in Cashflow.svelte:100 ); web/src/lib/Markets.svelte:22 (News/Watchlist row fixed at minmax(0,1fr) 420px whatever the width); web/src/lib/markets/News.svelte:265 (the 200 px search box in the header row; at 1200 px its right edge is 752.8 px against a c); web/src/lib/Trades.svelte:48 (table min-width:1150px; sideways scroll inside the card below that)
- impact: Anyone not running a maximised window on a wide monitor sees truncated money figures in the KPI tiles and two-axis scrolling tables, and at 1200 px, a width the spec itself verifies, a control hangs outside its card; the app cannot be used beside another window.
- proposal: Three breakpoints: ≥1340 as now; 1000–1340 tiles at three per row, the 4+2/2+4 card pairs stacked and header rows allowed to wrap; <1000 single column with tables as they are. Tiles get min-width:0 and a font-size that steps down (clamp()) instead of nowrap clipping. Add 1024 to the SPEC §7 widths.
- ux-conflict: True — SPEC §6/§7 fix the six-column geometry and verify only 1200–1680 px; reflowing below 1340 changes the layout the owner sees at those widths.
- owner decision: -
- confidence: high

### uiux-B-5 [medium/ui-ux] Every notice and error is a four-second line in a header that scrolls off the page
- status: confirmed — code: confirmed | std: confirmed
- standard: Feedback anchored to the viewport that stays until read: Material Design snackbars/toasts, Apple HIG alerts, GitHub's flash messages; WCAG 2.2 SC 2.2.1 Timing Adjustable (a timed message the user cannot extend or dismiss fails it) and 3.3.1 Error Identification (the error is shown where the person can see it). Sticky headers in IBKR Client Portal and Wealthsimple keep status visible while the page scrolls.
- observed: flash() writes into #syncline for 4 s (10 s for orders) and is overwritten by the next flash; there is no dismiss and no action. #hdr is position:static, and at 1440×900 the foot of the Dashboard puts the header's bottom edge at −241 px (Cashflow −245, Markets −112, a holding page −107): a failed journal save typed into the thesis box (which sits below the fold on taller pages), a refused watchlist add, a refused order cancel, a failed copy — each is reported only up there. The journal autosaves 600 ms after typing with no saved/unsaved state, so a failed save leaves the text on screen looking saved.
- evidence: web/src/lib/ui.svelte.ts:92 (flash(): ui.notice for 4000 ms, cleared by timer, replaced by any later flash); web/src/App.svelte:403 (#hdr is a static flex row inside the scrolling panel; no position:sticky); web/src/App.svelte:419 (#syncline is the single line every notice, sync step and error shares); web/src/lib/state.svelte.ts:121 (a journal save that fails: flash in the header and resync; the row is put back but nothing); web/src/lib/TradeDetail.svelte:173 (thesis autosave 600 ms after input / on blur; no saving or saved indication)
- impact: Errors that matter (a note lost, a cancel that Wealthsimple refused) go unseen whenever the person is reading the lower half of a page; two failures in quick succession show one.
- proposal: Make the header and tab row sticky (they are 96 px tall) or move notices to a fixed, viewport-anchored region; keep success notices timed but let an error persist with a dismiss and, where one exists, the action ("Retry", "Open Orders"); queue rather than overwrite; show a small saving/saved state beside the thesis box, and report a failed journal save beside it.
- ux-conflict: True — SPEC §4 defines the notice as taking the sync status's place for four seconds; a persistent, dismissable error and a sticky header change what the owner sees.
- owner decision: 2026-09-27 "An error never breaks the page's layout: the header's error stays one line…" (compatible: a sticky header keeps that line and makes it visible)
- confidence: high

### uiux-B-6 [medium/design] Two positions in the same symbol in different accounts are shown as identical rows and legend entries
- status: confirmed — code: confirmed | std: confirmed
- standard: Portfolio screens either aggregate a symbol across accounts with a per-account breakdown or label the account on the row: IBKR Client Portal (positions per account with the account column), Wealthsimple (holdings within an account, the account always named), Sharesight (one holding per portfolio, the portfolio named). A legend never carries the same label twice without a qualifier.
- observed: On the demo book NVDA is held in two accounts: the Holdings table shows two rows both reading "NVDA" and the Allocation legend two entries "NVDA 4.4%" and "NVDA 2.7%" with nothing to tell them apart; the account is only a mouse-hover tip on the row (nothing at all on the legend). Cashflow Positions is also per account and would show the same. SPEC §4 Portfolio states the account is not shown and is named on hover.
- evidence: web/src/lib/Portfolio.svelte:147 (row keyed by position id; data-tip={r.account} is the only place the account appears, hove); web/src/lib/Portfolio.svelte:70 (allocation slices labelled by x.label alone (the symbol), so two positions in one symbol p); web/src/lib/Portfolio.svelte:148 (symbol cell renders symText(r.symbol) with no account or venue qualifier)
- impact: The person cannot tell which NVDA row is which, cannot compare the two, and misreads the allocation legend; with a filter on Account the rows change meaning invisibly.
- proposal: Either aggregate the Holdings table and the Allocation donut by instrument (one NVDA row, expandable into its accounts, as Sharesight does) or show the account as a muted second line under the symbol on every per-account row and legend entry; keep the hover tip as a complement, not the only carrier.
- ux-conflict: True — Adds an account line (or an aggregation) to the Holdings rows and Allocation legend that the spec explicitly leaves out.
- owner decision: "Only what was asked" (SPEC §1, decisions 'Earlier') and SPEC §4 Portfolio: "the account is not shown in the row, and hovering the row names it"
- confidence: high

### uiux-B-7 [medium/ui-ux] An invalid ticket value is silently rewritten (a bad quantity becomes 1) and the Buy affordance is on regardless of connection
- status: merged — code: refuted (Same root cause as uiux-A-1 (the ticket validates nothing before Review and Submit and offers Buy while disconnected); A-1 is kept and corrected to carry B-7's one distinct fact, the silent rewrite of an invalid quantity) | std: corrected (Everything observed reproduces (ticket open while 'Not connected.', Review enabled, dashes through Review, 'A limit price is required.' only after Submit, and 'abc' typed as Shares becoming 1 on blur), but this is the sa)
- standard: WCAG 2.2 SC 3.3.1 Error Identification and 3.3.3 Error Suggestion: an invalid value is identified to the person and described, never replaced; brokerage tickets (IBKR, Wealthsimple) mark the field and keep Review disabled, and offer no ticket without a live session.
- observed: Typing 'abc' into Shares and leaving the field yields '1' with no word (measured); OrderTicket.svelte:65–70 set an unparsable quantity to 1 and drop an invalid limit, stop, trail or target to null on blur. TradeDetail.svelte:156 turns the Buy button on for every share (on: true) whatever status.connected says, so a disconnected app offers a ticket that can only be refused.
- evidence: web/src/lib/ticket/OrderTicket.svelte:65 (onBlur: an invalid quantity becomes 1; invalid prices become null, silently); web/src/lib/TradeDetail.svelte:156 (Buy is on:true for every share regardless of connection state)
- impact: On a real account a mistyped quantity becomes an order for one share without anyone being told; a disconnected person is walked into a ticket that must fail.
- proposal: Mark an invalid field and keep Review disabled with the reason under it; never replace a typed value; dim Buy/Sell with the reason while there is no session (the rest is A-1).
- ux-conflict: True — SPEC §4 Order ticket: 'A field holds what was typed while it has focus and is formatted when it loses it' and 'Defaults: one share'; the Buy/Sell rule at 'A holding' does not mention connection.
- owner decision: -
- confidence: high

### uiux-B-8 [medium/narrow-fix] A date preset is not counted as an active filter: no badge dot, and Escape does not clear it
- status: confirmed — code: confirmed | std: confirmed
- standard: One source of truth for "a filter is active": the chip row, the badge and the clear action must agree (basic consistency; Nielsen heuristic 4). SPEC §5: "With nothing open and nothing being typed, Escape clears every filter, the same as Clear all."
- observed: Choose 1Y in the Date field → the chip "Date 1Y ×" appears, but the funnel shows no dot and Escape leaves the chip in place. Choose the year 2025 or a symbol → dot shown and Escape clears. activeCount() sums lists, ranges, years, from/to and search and omits `preset`, so the dot (App.svelte) and the Escape cascade both ignore the eight presets while chips() shows them.
- evidence: web/src/lib/filters.svelte.ts:47 (activeCount(): no term for filters.preset); web/src/App.svelte:449 (the badge dot is drawn from activeCount() > 0); web/src/App.svelte:153 (Escape clears only when activeCount() > 0)
- impact: The most common filter (a period preset) is invisible to the badge and immune to the keyboard clear; the page and the header disagree about whether anything is filtered.
- proposal: Derive "active" from the same function the chips use (chips().length > 0), or add `filters.preset !== 'all'` to activeCount; add a test over every preset.
- ux-conflict: False 
- owner decision: -
- confidence: high

### uiux-B-9 [medium/ui-ux] Open trades show "$0.00" realized in the gain colour beside "—", while their page shows a large unrealized gain
- status: merged — code: refuted (Same root cause as uiux-A-4 (open trades show a realized $0.00 coloured as a gain), which is kept. B-9's evidence is also wrong: fmt.ts has 203 lines, there is no line 832, and color() (fmt.ts:175-178) returns var(--neg)) | std: confirmed
- standard: Trading journals separate realized from unrealized and never colour a zero as a gain: Tradervue lists open positions with their current unrealized P&L (Tradervue "Open positions" report); TraderSync and TradeZella mark open trades distinctly and show unrealized P&L. Colour carries meaning only for a sign (WCAG 1.4.1 aside, a green zero reads as a win).
- observed: On the Trades list SOL (Open) reads P&L "$0.00" in green and P&L % "—"; NVDA and the AAPL call read "$0.00" green and a green "—". Opening SOL shows +$912.86 / +11.7%. fmt.color() returns the gain colour for zero and for null, and Trades.svelte colours both cells with it; the list gives an open position a break-even look while the same figure is a gain one click away.
- evidence: web/src/lib/fmt.ts:832 (color(): "zero reads as positive" — null/zero → var(--pos)); web/src/lib/Trades.svelte:77 (P&L and P&L % cells coloured with color(t.pnl) for open trades whose realized P&L is 0); web/src/lib/TradeDetail.svelte:281 (the holding page's headline figure is unrealized P&L in the same colour system)
- impact: A reader scanning the list sees green zeros and dashes for positions that are up or down materially, and two different "P&L" figures for one trade with no label saying which is which.
- proposal: On an open row show realized-to-date as a neutral "—" or "$0.00" uncoloured and add the unrealized P&L (the holding's figure) in a clearly named column or beside it, as the category does; never colour zero or null.
- ux-conflict: True — Changes what the Trades list shows in the P&L columns for open trades.
- owner decision: 2026-09-24 "A trade runs from a position's first fill until it is flat; a partly sold position is an open trade in the Trades list; a partial sale's P&L counts on its day"
- confidence: high

### uiux-B-10 [medium/design] First run gates every tab, including Markets, behind "Connect Wealthsimple", and a CSV/manual user reads "Not connected" for ever
- status: confirmed — code: confirmed | std: confirmed
- standard: Leading products let a person use what needs no account link before linking one, and put every way in on the empty state: Yahoo Finance/Google Finance watchlists and market pages work with nothing linked; Sharesight's empty state offers broker import, CSV and manual entry side by side; Wealthsimple's onboarding never shows an error-coloured status for a state the person chose. A status line should describe state, not a missing optional step.
- observed: On an empty book every tab — Dashboard, Trades, Portfolio, Markets, Cashflow — renders the same "No activity yet / Connect Wealthsimple" page. Markets needs no book (tiles, the Canada/US/International heatmaps, Fear & Greed, watchlist, news, short interest are public-source data) yet is blocked. The empty page mentions neither Import CSV nor Add trade, which exist in the menu. The header's status reads "Not connected" permanently for someone who journals from CSV and never intends to connect (it also reads so on the demo book).
- evidence: web/src/App.svelte:495 (showingEmpty → <Empty> for every route, Markets included); web/src/lib/Empty.svelte:16 (copy names only Wealthsimple; the button is Connect only); web/src/App.svelte:278 (syncLine(): 'Not connected' whenever status.connected is false); web/src/lib/Markets.svelte:18 (the Markets tab reads markets/heatmap/news documents that do not depend on the book's acti)
- impact: A new person cannot try the market tools or see the app's shape before handing over a brokerage login; someone who uses CSV files is told they have no activity and, thereafter, that they are not connected.
- proposal: Render Markets on an empty book; make the empty state offer the three ways in (Connect Wealthsimple, Import CSV, Add trade); when no broker login is saved, let the status read nothing or "Manual" rather than "Not connected", and reserve "Not connected" for a saved login whose session lapsed.
- ux-conflict: True — SPEC §4 'The page with nothing to show' defines the Connect-only empty page and the 'Not connected' status.
- owner decision: 2026-09-28 "No screen ever opens with nothing to show, except on the very first open, before Wealthsimple has been connected or a CSV read"
- confidence: high

### uiux-B-11 [low/ui-ux] Trade-chart execution labels collide into an unreadable band
- status: confirmed — code: corrected (The label collision is real: tradeChart.ts:107 labels every marker whenever ≤40 executions are in view with no overlap test, and a screenshot of the SOL holding page (23 executions) shows the '−0.220000' labels overprint) | std: corrected (The label collision is real: a screenshot of the SOL holding at 1440 px shows '−0.220000' labels overprinting in every cluster, and tradeChart.ts:107 labels whenever ≤40 executions are in view with no overlap test. The p)
- standard: Charting products declutter marker labels: TradingView's own trade markers show the label on hover once markers crowd, and Lightweight Charts' series markers are drawn without text when dense; a label that overlaps another is not shown (the same rule the page already applies to its value-axis labels via spaceAxis).
- observed: The SOL holding page marks 23 monthly staking rewards; each carries the label '−0.220000' and, at 1440 px, the labels overprint each other ('−0.220000−0.220000…') across the top of the chart. The rule is labels on whenever ≤40 executions are in view, with no collision test.
- evidence: web/src/lib/actions/tradeChart.ts:107 (relabel: labelled = executions in view ≤ 40; no overlap check); web/src/lib/Dashboard.svelte:306 (the value axis already hides colliding labels (spaceAxis) — the same idea is missing on th)
- impact: The one chart on the page that should tell the story of the trade is illegible for any holding with regular small fills (staking, DRIP, DCA), which are common in this owner's category of accounts.
- proposal: Thin marker labels by a pixel-collision test per visible range (keep the arrow, drop the text, show it on hover), and collapse same-day/same-size fills into one marker with a count.
- ux-conflict: False 
- owner decision: -
- confidence: high

### uiux-B-12 [low/ui-ux] Light theme gain/loss figures and table headers fall under WCAG AA contrast, and 10 px text is used throughout
- status: confirmed — code: confirmed | std: confirmed
- standard: WCAG 2.2 SC 1.4.3: 4.5:1 for text under 18 pt/14 pt bold. Product practice puts the smallest UI text at 11–12 px (Linear 11 px labels, IBKR 12 px tables); 10 px is below every major design system's minimum.
- observed: Measured in Light on the Dashboard: a gain figure in a table cell 4.09:1 at 12.5 px, a loss KPI value 4.42:1, table headers 4.38:1 at 10.5 px; the dark themes pass (5.5–9.9:1). .lbl tile labels, axis labels, card footers and the year list are 10 px; table headers 10.5 px; subtitles 11 px.
- evidence: web/src/app.css:31 (light --pos:#2e8a62 and --neg:#c0505f on --surface #fafafc); web/src/app.css:74 (.lbl{font-size:10px…}); web/src/app.css:66 (.table th{font:500 10.5px/18px…}); web/src/lib/Dashboard.svelte:242 (axis labels font-size:10px)
- impact: In Light the very figures the app exists for (green and red money) are the hardest to read; the labels and axes are small enough to fail many readers on a high-DPI laptop.
- proposal: Darken the light pos/neg tokens to reach 4.5:1 (about #23745a / #a8434f) and the light header ink to .75; raise the smallest text to 11 px (labels, axes) and 11.5 px (headers).
- ux-conflict: True — Changes two theme tokens and the smallest type sizes the owner sees.
- owner decision: -
- confidence: high

### uiux-B-13 [low/ui-ux] Theme is a manual three-way choice with no System option and a dark default before the saved theme is read
- status: confirmed — code: confirmed | std: confirmed
- standard: Every current OS-native and leading web product (GitHub, Linear, Wealthsimple app) offers System/Auto as a theme option following prefers-color-scheme, with the explicit themes as overrides.
- observed: THEMES is Nocturne, Midnight, Light; nothing reads prefers-color-scheme; html defaults to color-scheme:dark and index.html applies only a saved choice, so a Light-mode user gets a dark app until they find Theme in the menu (a hover-opened submenu).
- evidence: web/src/lib/theme.svelte.ts:13 (THEMES: three fixed themes, no 'system'); web/src/app.css:5 (html{color-scheme:dark} as the default before any choice); web/src/lib/Menu.svelte:50 (theme list opens on mouseenter / click; submenu to the left)
- impact: A Light-mode user's first impression is a dark app and a hunt through the menu; the choice never follows the OS's day/night switch.
- proposal: Add System as the default theme (prefers-color-scheme → Nocturne or Light), keep the three as overrides, and apply it in the inline script before first paint.
- ux-conflict: True — SPEC §6 names three themes with Nocturne as the default.
- owner decision: -
- confidence: medium

## missed, added by the code verifier

### uiux-V-1 [medium/narrow-fix] Escape means 'back' on a trade's page but 'clear every filter' on a holding's page, and an open trade's row in Trades lands on the holding's page
- status: added by verifier
- standard: One gesture has one meaning across a product (Nielsen heuristic 4, consistency; the spec's own §5 reasoning against 'two readings of one gesture'). Detail pages in Tradervue, IBKR Client Portal and Wealthsimple close with the same key or control wherever they are opened from, and nothing destroys a person's filter set as a side effect of leaving a page.
- observed: App.svelte's Escape cascade goes back to the list only when route.tab === 'trades'; on a holding's page (route.tab 'portfolio') it falls through to 'clear every filter'. Trades.svelte sends an open trade's row to the holding's page under Portfolio (goSub('portfolio', t.position)), so from one list, Escape on a closed trade goes back while Escape on an open trade stays on the page and wipes the filters. Reproduced: with the chip 'Date 2025 ×' set, opening a holding and pressing Escape left the hash on #portfolio/… and emptied the chips.
- evidence: web/src/App.svelte:152 (history.back() only when route.sub && route.tab === 'trades'; the next line clears the fil); web/src/lib/Trades.svelte:63 (an open trade's row navigates to goSub('portfolio', t.position), a page under the other ta); web/src/lib/state.svelte.ts:25 (refilter() leaves a sub route only under 'trades'; a holding's page stays open with its fi)
- impact: A person reviewing open trades under a filter loses the filter the first time they press Escape to leave a page, with no undo; the same key does opposite things on two pages reached from the same list.
- proposal: Escape on any detail page leaves the page (both tabs), and only a list page with nothing open clears the filters; or drop Escape-as-back and leave clearing to Clear all. Open trades keep their page under Trades (the holding's figures are already in the document that draws it).
- ux-conflict: False 
- owner decision: -
- confidence: high

### uiux-V-2 [low/narrow-fix] In the Light theme the body keeps color-scheme:dark, so native controls render dark on a light page
- status: added by verifier
- standard: MDN color-scheme: the value on the root and body decides how the browser draws form controls, scrollbars, date pickers and select menus; every themed product sets it once with the theme (GitHub, Linear) so the UA parts match the page.
- observed: app.css:28 sets html[data-theme="light"]{color-scheme:light}, but app.css:38 declares html,body{…color-scheme:dark}, and the body's own declaration wins over inheritance. Measured with the Light theme active: html computed color-scheme 'light', body 'dark'; the Add trade modal's date input and account select compute 'dark', so their native pickers and dropdown menus open in the dark UA style over a light page.
- evidence: web/src/app.css:38 (html,body{margin:0;…;color-scheme:dark}: the body declaration applies in every theme); web/src/app.css:28 (the light theme sets color-scheme:light on html only); web/src/lib/Modals.svelte:66 (the date input and select of the Add trade form inherit the body's dark scheme (measured))
- impact: Date pickers, select menus, checkboxes (the Clear data dialog) and any native scrollbar open dark inside the Light theme: the one theme meant for daylight breaks at the first native control.
- proposal: Move color-scheme onto the theme rule alone (html{color-scheme:dark} html[data-theme="light"]{color-scheme:light}) and drop it from body.
- ux-conflict: False 
- owner decision: -
- confidence: high

### uiux-V-3 [medium/performance] Every trade's and holding's chart is fetched on the first open in each browser, one request per row, with intraday bars read from the sources in the background
- status: added by verifier
- standard: Leading products load a chart when its page opens and prefetch at most the next likely page (Tradervue, TradeZella, IBKR Client Portal); a client never issues a request per row of the book on load. Efficiency on a small single-board computer is part of this review's bar.
- observed: On a cold load (fresh browser profile) of the Dashboard on a 33-row demo book, the page issued GET /api/figures/trades (every trade, no filter), GET /api/figures/details (every fill of every trade and holding) and 33 GET /api/history requests within 40 s: ahead.ts walks every trade and holding and reads a chart for each, one at a time while the page is idle, and keeps it in IndexedDB. The server's history handler, for an intraday timeframe whose bars are not stored, schedules ensure_intraday_in_background (feeds.rs:3221-3224), so online each short-held trade also starts a source read. Repeat loads in the same browser skip charts already kept (recall), so the cost is the first open per browser and per new trade; on a book of a few hundred trades that is a few hundred history reads and background fetches before the person has opened anything.
- evidence: web/src/lib/trade/ahead.ts:41 (readAhead: one GET /api/figures/details, then chartAsDrawn(t) for every trade and holding ); web/src/App.svelte:220 (every trade read once per page load, then readAhead over trades plus holdings); rust/crates/server/src/feeds.rs:3221 (an intraday timeframe not yet stored starts ensure_intraday_in_background for that instrum); web/src/lib/reads.svelte.ts:165 (remember(): the drawn chart is saved per browser (IndexedDB), so the walk repeats in every)
- impact: A few hundred trades on a Raspberry Pi means a few hundred history reads and source fetches at the first open of each browser, competing with the page's own reads and the sources' rate limits, for pages most of which are never opened.
- proposal: Prefetch what is likely (the Review queue's trades, the holdings, the first screen of the list) and read the rest on demand; cap the walk, and never start source fetches from a prefetch.
- ux-conflict: False 
- owner decision: docs/decisions.md 2026-09-28: 'No screen ever opens with nothing to show, except on the very first open'
- confidence: medium

## missed, added by the standard verifier

### uiux-S-1 [medium/ui-ux] No status message is ever announced: the header line, every notice and every form error are plain text with no live region
- status: added by verifier
- standard: WCAG 2.2 SC 4.1.3 Status Messages (AA): a status change not moving focus (sync progress, 'Trade added', a refused order, a failed save) is exposed through role=status/alert or aria-live so assistive technology announces it; WAI-ARIA APG alert and status patterns. Every web brokerage and journal that passes an accessibility audit marks its toasts and inline errors this way.
- observed: A search of web/src finds no aria-live, role=status or role=alert anywhere. #syncline is a plain span that carries the sync step, every flash and every error; the ticket's 'Not connected.' and 'A limit price is required.' are plain divs; the Add-trade form's error is a plain div. A screen-reader user pressing Submit hears nothing when the order is refused.
- evidence: web/src/App.svelte:419 (#syncline: a span with no role or aria-live carrying sync state, notices and errors); web/src/lib/ui.svelte.ts:92 (flash() writes ui.notice and clears it after a timer; nothing announces it); web/src/lib/ticket/OrderTicket.svelte:218 (submitError rendered as a plain div on the review step); web/src/lib/Modals.svelte:55 (the form's f.error is a plain div)
- impact: Every outcome the app reports (an order refused, a save failed, a sync finished) is invisible to anyone using a screen reader; with B-2 and B-3 the page cannot be used non-visually at all.
- proposal: role=status (aria-live=polite) on #syncline with aria-live=assertive while noticeKind is 'err'; role=alert on inline form and ticket errors; one helper so every notice goes through it.
- ux-conflict: False 
- owner decision: docs/decisions.md 2026-09-26: accessibility deferred to docs/future.md
- confidence: high

### uiux-S-2 [medium/narrow-fix] The broker check's disagreements are delivered as one run-on sentence in the header's single status line
- status: added by verifier
- standard: Reconciliation differences are a list, one row per difference, each linking to the transaction it concerns: Sharesight's unconfirmed-transaction and holding alerts, IBKR Client Portal's reconciliation views, Wealthsimple's activity feed. A status line describes one state; it is not the carrier for N items. The owner asked that an error not break the layout; a top team would give the list a place, not compress it into a line.
- observed: status.rs:144 joins every standing failure and every disagreement into one string with a space; status.rs:186 emits one sentence per unit disagreement ('A sale of ZZ28 in Manual on 2025-01-01 took 5 units more than the book held.'); broker_reads.rs:393 joins every unreconciled month the same way. On the demo book the header line carries about a hundred such sentences, cut to one line (App.svelte:419), readable only in the hover tip or by pressing the copy icon (:434), with no link from any of them to the trade it names, and it displaces 'Not connected' / 'Synced' on every page for as long as any one stands.
- evidence: rust/crates/server/src/status.rs:144 (failures(): every standing failure said together, said.join(" ")); rust/crates/server/src/status.rs:186 (one sentence per unit disagreement pushed into the same line); rust/crates/server/src/broker_reads.rs:393 (unreconciled months joined into the header text); web/src/App.svelte:419 (one nowrap ellipsis span; the whole is only in the tip and the clipboard)
- impact: A person with several disagreements (a CSV import that overlaps a sync, a corporate action the feed missed) sees a red line they cannot read and a list they cannot act on; every other status is hidden behind it until the last disagreement is resolved.
- proposal: The header line says the count and kind ('Book differs from Wealthsimple in 100 places') and opens a list (a Book check tab in the Notifications panel, or its own panel) with one row per difference linking to the trade or month; single-state failures (a source down, a session lapsed) stay one line.
- ux-conflict: True — Changes what the header line reads and adds a list the spec does not have.
- owner decision: docs/decisions.md 2026-09-27: 'An error never breaks the page's layout: the header's error stays one line, and the whole of it is read on hover and copied with an icon'; SPEC §4: 'every place the book disagrees with the broker' is said in the one line
- confidence: high

### uiux-S-3 [low/ui-ux] The document title never changes: every route, trade and holding reads 'Bagholder' in the tab, history and bookmarks
- status: added by verifier
- standard: WCAG 2.2 SC 2.4.2 Page Titled, applied to single-page apps by updating document.title on each route change (WAI ARIA APG / W3C SPA guidance); the category does it (TradingView 'AAPL 175.20 ▲ +1.2% · TradingView', Tradervue 'Trade: AAPL – Tradervue', Google Finance per quote), so browser history, tab strips and bookmarks name the page.
- observed: web/index.html:7 sets <title>Bagholder</title> and nothing in web/src writes document.title (search: no matches); a trade page (#trades/<id>), a holding, a listing and the heatmap wall display all leave the tab reading 'Bagholder'. The page also has no h1 (B-3), so nothing names the screen at all.
- evidence: web/index.html:7 (static <title>Bagholder</title>); web/src/lib/router.svelte.ts:1 (the hash router changes route.tab and route.sub; no title is set on a change (no document.)
- impact: Ten Bagholder tabs of ten holdings are indistinguishable; the back-button menu and bookmarks say nothing; the wall display's window has no name.
- proposal: Set document.title from the route in the router: 'Trades · Bagholder', 'NVDA · Bagholder', 'Heatmap · Bagholder'; on a holding, optionally the last price as TradingView does.
- ux-conflict: False 
- owner decision: -
- confidence: high

### uiux-S-4 [low/design] Numbers are formatted in en-US for everyone while time is formatted for the viewer
- status: added by verifier
- standard: Intl.NumberFormat with the viewer's locale is the platform's own facility; Wealthsimple's product is bilingual and formats '1 234,56 $' in French, and Sharesight and IBKR format per the viewer's locale. The app's own rule for time ('Time is correct for whoever is looking', decision 2026-09-25) is the same principle, applied to one kind of figure and not the other.
- observed: fmt.ts:40 n2() calls toLocaleString('en-US') for every money and quantity figure and :135 for hold days; the comment at fmt.ts:8 records 'en-US grouping' as a convention ported from the old page. The viewer's locale is never read.
- evidence: web/src/lib/fmt.ts:40 (n2(): v.toLocaleString('en-US', …) for every figure); web/src/lib/fmt.ts:8 (the convention: en-US grouping, ported verbatim from the old page)
- impact: A French-Canadian Wealthsimple user reads every figure in a foreign format while their times are localised; the inconsistency is the app's own.
- proposal: One formatter built from navigator.language (or a menu choice beside Theme), used by money(), num() and the axes; keep U+2212 and the format rules SPEC §3 states.
- ux-conflict: True — SPEC §3 (line 183) fixes thousands separators and the minus sign; grouping and decimal marks would follow the viewer's locale.
- owner decision: docs/decisions.md 2026-09-25 'Time is correct for whoever is looking' (the same principle, not applied to numbers); 'Only what was asked'
- confidence: medium
