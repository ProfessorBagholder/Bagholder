# Bagholder architecture: the Rust + Svelte build

This is the design of the application going forward, and the order of work that gets it there. The Rust server with the Svelte page is the only build being developed; the Python and Go servers and `ledger.html` are frozen and are deleted at cutover (stage 9). `docs/frontend-backend-migration.md` records why this stack was chosen (axum on tokio, a Svelte SPA built by Vite, TypeScript types generated from the Rust structs); this document supersedes its phases, which described a port of the page and left the server as it was.

`SPEC.md` remains the authority for every figure, screen and interaction. Where this document and the code disagree, the code is wrong.

## Where the code stands (audit of 2026-09-20)

Six reviews of the branch, each claim carrying a file and line, the consequential ones re-checked by hand. The short of it: the server is a line-by-line translation of a Python standard-library program, and the page is a transliteration of `ledger.html`'s render functions. Both work; neither is designed.

**Server (`rust/`, ~39,000 lines).**
- HTTP is `tiny_http` with an unbounded OS thread per request, two `match path` string routers, hand-written query and body parsing (a malformed or oversized body silently becomes `{}`), hand-written chunked framing for the two streams, panics used as exceptions under `catch_unwind`, `Cache-Control: no-store` on content-hashed assets, no graceful shutdown, no signal handling.
- State is one global `APP` of mutexes, every lock taken with `.lock().unwrap()`; a `syncing` flag that a panic leaves set forever; a `jobs` map that only grows.
- SQLite: a new connection per call (34 sites), **no WAL and no transactions anywhere** (the Python build sets WAL; a database created by the Rust build never is), replace-by-delete-then-insert row by row in autocommit, so a reader can see an empty table and a crash leaves a half-merged book.
- The data model is `serde_json::Value` (~1,800 mentions against ~60 structs): enums compared as strings in hot loops, a missing number read as `0.0`, a missing FX rate read as `1.35`, any non-USD currency treated as CAD, quotes keyed by symbol alone (the coin/share collision is patched by hand), about four deep copies of the trade list per request.
- The model cache keys on one fingerprint that includes quotes, so **every quote tick re-reads every activity and re-runs the FIFO match** (up to twice), which `SPEC.md` §Freshness forbids and the Python and Go builds do not do. The cache has no single flight, so concurrent requests each rebuild. The fingerprint itself is ~36 SQL statements, computed about four times per `/api/model`.
- Seventeen background loops run on fixed periods whether or not a page is open or a market is trading; each sleeps in 250 ms slices (about 70 wake-ups a second, idle). Margin, news and universes are rewritten with `fetched_at = now` when nothing changed, which moves the fingerprint and makes every open page refetch the whole book. No conditional requests anywhere. EDGAR and SEDAR+ pacing sleeps while holding a lock, so a click queues behind a sweep.

**Page (`web/`, ~8,300 lines).**
- One `store.model` replaced wholesale on every load: every derived value re-runs, every row re-renders, and the open trade's chart is destroyed and rebuilt. `only=live` and `/api/trade` are never called, so a quote tick downloads the whole book; the reload lands under a user who is typing or has a trade open.
- Nine polling loops (status at 2.5 s/30 s, sync 1.5 s, connect 0.5 s, orders 10 s, ticket quote 5 s, tiles 3 s and 9 s blind full reloads, shorts 4 s, history 3 s, filings 1.5–15 s), though an SSE channel already exists. About forty `loadModel()` call sites, most of which need one row or one section.
- Five copies of an `api()` helper, four implementations of symbol search, two implementations of the filings store; hand-written types with index signatures and `as unknown as` casts (`Status` lacks the very `coreVersion` field the contract turns on).
- 646 inline `style=` attributes; the sortable header written six times, the modal shell three times, the panel shell three times; no focus management in any dialog, rows that cannot be reached by keyboard, no `aria-sort`; eight dead files; no ESLint, no component tests, no browser tests; **`web/dist` committed to git** while gitignored, and nothing in CI builds or checks `web/`.
- **About 45 features of `SPEC.md` are missing or partial**, among them: the listing page for a ticker the book does not hold (every such click goes nowhere), the update button with its progress and the "Restart Bagholder to finish the update" line, the first-run page, Shift+Enter / Backspace / the Tab ring in the filter popover, ⌘K over the Orders panel, scroll restoration on Back, loading skeletons, the digit roll, the cut-text hover, scrollbars (never shown), browser-channel notifications and the test notification, the heatmap's own address and slideshow. The full checklist is kept in `docs/parity.md` and is closed item by item in stage 7.

## Rules the design holds to

Each of these is enforced by a test that fails when it is broken (stage numbers say where the test arrives). A rule without a test is a wish.

1. **Work happens because something changed, and only the work that change requires.** No loop runs on a fixed period when an event, a deadline or a request can drive it. A periodic read survives only where the upstream offers no push; it then runs only while someone is looking at what it feeds, only while that market is open, with backoff, jitter and conditional requests, and it is listed in "Timers that remain" below with its reason. (Stages 2–3, 6.)
2. **The server tells the page; the page never asks "anything new?".** One SSE connection carries typed events. (Stages 3, 6.)
3. **A derived value is recomputed only when one of its own inputs moved.** A quote tick re-marks open positions and nothing else; a journal edit recomputes nothing. (Stage 2.)
4. **A write that changes nothing is not a change.** Writers compare before they replace; versions move only on a real difference. (Stage 1.)
5. **Types, not blobs.** Domain data is structs and enums from the SQLite row to the HTTP response; the page's types are generated from them. A missing value is an `Option`, never a default that looks like data. (Stages 4–5.)
6. **The database is never observed half-written.** WAL, one writer, every multi-row change in a transaction. (Stage 1.)
7. **Nothing on screen is rebuilt that did not change**, and nothing changes under a user who is typing. (Stage 6.)
8. **Every feature in `SPEC.md` exists and is driven by a browser test**, not only rendered. (Stages 7–8.)
9. **Money is never silently wrong**: an unknown currency or a missing rate is reported, not converted at a constant. (Stage 5.)

## Target design

### Process and HTTP
One tokio runtime. An axum `Router` under a tower stack: the loopback/Host/same-origin gate as middleware, request timeout, body limit, panic catcher, compression, tracing. Every route has a typed request (`Query<T>` / `Json<T>`) and a typed response; one `ApiError` enum maps to real status codes. State is an `AppState` passed by `State<…>` — no global. Static assets are embedded in the release binary and served `immutable` when hashed, `no-cache` for `index.html`; a debug build reads `web/dist` from disk and development uses the Vite proxy. Graceful shutdown on SIGINT/SIGTERM and on the updater's restart, inside the supervisor's existing contract (exit code 3, `BAGHOLDER_CHILD`, the pending-update marker, the 20-second health window), which does not change. The MJPEG sign-in stream becomes a body stream over a `watch` channel fed by the blocking CDP thread, its part-boundary framing and newest-reader eviction preserved. `bagholder-browser` stays a separate process, called with a timeout. Logging through `tracing`.

### Storage
`rusqlite` stays (the store crate is synchronous and tested). One writer connection on a dedicated thread behind a channel; a small reader pool used through `spawn_blocking`. Pragmas once: `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`. Sync, import, clear and every replace run in a transaction. Schema migrations are versioned and the stored version is consulted. **Each table has a generation counter, bumped by the writer inside the transaction only when rows actually differ**; these replace the COUNT/MAX fingerprint (which costs ~36 statements and misses in-place edits). Anything that copies the database checkpoints first.

### The model: layers, each keyed by what it reads

| Layer | Reads | Rebuilt when |
|---|---|---|
| Book (normalize, FIFO match) | activities, securities, the day | an activity or a security changes, or the day turns |
| Closed trades | Book, FX, saved groups | those move |
| Journal overlay (joined at serialization) | journal | never recomputes anything |
| Cashflow rows | Book, FX, distributions | those move |
| Equity curve and years | NAV history, benchmarks, the day | those move |
| Accounts | accounts, balances, margin | those move |
| Marked positions | Book's open lots, quotes, Accounts | **a quote tick rebuilds this and nothing above it** |
| Markets | marked positions, exposures, news, universes, tiles, watchlist | those move |

Each layer is an `Arc` behind its key with a single-flight build. Views are split in two — a closed view (KPIs, months, by symbol, trades, options) and a live view (positions, portfolio, market) — each cached per canonical filter set in a small LRU; the live endpoint builds only the live view. Filters stay server-side: the aggregates are model outputs and the page must never re-derive money math.

### Domain types
`Activity`, `Fill`, `Lot`, `Leg`, `Trade`, `Position`, `CashflowRow`, `Account`, `Balance`, `MarginRow`, `Security`, `Quote`, `Distribution`, `FxTable`, `EquityPoint`, `Filters`, and the payload structs, with enums for currency, kind, side, direction, category, flag, raw event, grade, price source. Identity is an `InstrumentKey` (security id, or symbol + kind + currency), never a bare symbol. Quantities are exact (`Qty`, 1e-8 units) because trade ids are derived from them; money is a `Native`/`Cad` newtype over `f64` so units cannot be mixed; conversion returns a `Result`. TypeScript types are generated with `ts-rs`, including the event union, and the page has no hand-written API type.

### Events
An in-process bus with a sequence number and a short ring buffer. Writers publish where they commit: `QuotesChanged{keys}`, `BookChanged`, `AccountsChanged`, `NewsChanged{key}`, `FilingsChanged{symbol}`, `FilingEnriched{symbol,id}`, `ShortsChanged{key}`, `UniverseChanged{key}`, `TilesChanged`, `OrderChanged{id}`, `BracketChanged{id}`, `HistoryReady{key,tf}`, `SyncProgress{step}`, `SessionChanged`, `UpdateState`, `NotificationCreated{id}`. `GET /api/events` is the one SSE stream; `Last-Event-ID` replays from the ring and a gap makes the page reconcile once. A live event carries its payload, so a tick needs no fetch. The page tells the server what it is showing (`POST /api/view`: route and instrument keys); subscriber and view state is what lets the periodic reads below run only when watched.

### Background work
Tasks on the runtime with cancellation, no sleep-polling of a stop flag. Deadlines instead of periods where the next moment is known (token expiry, next pull due). On demand where a page asks (universes, fear and greed, shorts, exposure on a security change). Interactive requests get priority over sweeps at every paced upstream, and no pacing sleep holds a lock.

**Timers that remain, and why** — each upstream below has no push interface:
- Quotes (TMX, Yahoo, Coinbase, Cboe): only for keys some open page is showing, only during that venue's session (Coinbase always), jittered.
- Wealthsimple order status and the bracket engine: only while a live order or an armed bracket exists; the bracket cadence is what a stop needs and is not relaxed.
- Wealthsimple activity pull and portfolio: on the computed deadline, on page open, after a fill.
- News, and the filings sweep when its notification switch is on: slow cadence, conditional requests where the source supports them, compare before replace.
- FX, benchmarks, distributions, the release check: daily-scale, conditional requests.
- CSV folder watch: operating-system file notification if the dependency earns its place, else on page open.

### The page
One typed API client (generated types, one `request<T>`, abortable, always carrying the header). One `EventSource`. State is separate `$state.raw` slices — core (per filter set), live, markets sections, accounts/options, status, orders, notifications — each replaced only by the event or response that owns it, rows held by id so an edit patches one entry. A full core apply waits while an input has focus or a trade is open; live applies at once. Per-instrument data (history, filings, shorts, listing, search) comes through one resource cache: keyed, in-flight de-duplicated, aborted when nothing is subscribed, errors expiring. Mutations are optimistic: patch, send, reconcile from the response, roll back on failure; a full reload is never the success path. Charts update their series in place.

Structure: shared components for the sortable header, grid header, dialog and side-panel shells, clickable row, pills, donut, hover crosshair; one overlay manager owning stacking, Escape order, outside click, focus trap and focus return; one shortcut table. Styles move from inline strings to tokens and component-scoped CSS, declaration for declaration, each screen gated by a computed-style and screenshot comparison at 1200, 1340, 1440 and 1680 px — nothing about the look changes. ESLint, Prettier, strict TypeScript, Vitest for logic and components, Playwright driving every interaction in `SPEC.md`.

## Order of work

The app stays runnable after every stage; each stage lands with the tests that hold its rule.

0. **Guardrails.** CI builds, checks and tests `web/` on every PR; `web/dist` leaves git and is built by CI, release and Docker, embedded for release; dead files deleted. Freeze the wire contract: ordered, unrounded JSON snapshots of every endpoint for the shared cases, plus new cases for quotes, a split, a stock distribution and saved groups — the net under stages 2, 4 and 5. `docs/parity.md` written from the checklist.
1. **Storage.** WAL, writer and reader pool, transactions, versioned migrations, generation counters, compare-before-replace.
2. **Layered model cache**, single flight, live view built alone. Test: a quote write leaves the Book and closed-trade layers pointer-equal.
3. **Event bus and `/api/events`; demand-driven background work.** Test: with no page open and markets closed, the server makes no outbound request over a simulated hour except those listed above as unconditional.
4. **axum server**: typed routes, `ApiError`, tower stack, graceful shutdown, embedded assets, `ts-rs`. Orders and brackets move last, behind their existing suites and `BAGHOLDER_DRY_ORDERS`.
5. **Typed domain model** replacing `Value` through the pipeline, verified by the shared cases and the stage-0 snapshots.
6. **Page data layer**: client, event stream, slices, resource cache, optimistic mutations, in-place charts. Tests: a live event triggers no request and re-renders no closed-trade row; no timer-driven fetch exists outside the ticket's venue-session quote.
7. **Feature completeness**: every open item in `docs/parity.md`, each closed with a Playwright test.
8. **Page structure**: shared components, overlay manager and accessibility, style migration under pixel comparison.
9. **Cutover**: the Svelte page at `/`; `ledger.html`, `python/` and `go/` removed; release, Docker and the updater ship the one build; `SPEC.md`, `CLAUDE.md` and `README.md` rewritten for it.

Stages 0–3 remove the waste a user can feel; 4–5 make the server what it should have been; 6–8 do the same for the page; 9 retires everything else.
