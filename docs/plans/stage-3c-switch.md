# Plan: stage 3c, the switch: the app serves the engine's figures from the book

## For the owner to decide

Nothing open. The owner's two choices in this stage were decided on 2026-09-25 (brief 09, `docs/decisions.md`): a figure the app cannot state shows `—` and its word, and a total counts what it left out; Clear data clears everything, by kind or all at once. The owner's one step: sign in once on the scratch copy (below, "The real run"), since the first live pull cannot run on a sign-in anyone else makes.

## Changes to `SPEC.md`

Each is a correction that follows from a decision already recorded or from the design, written into `SPEC.md` with its reason in the step that makes it true, and checked at the gate.

1. **Trade id.** A trade keeps a Bagholder id from its first fill for good, instead of the Wealthsimple row id; journal notes stay on their trade through corrections. Lots match by instrument, not by symbol and currency (a symbol reused by another security is another instrument).
2. **Splits and contract sizes.** Taken from what the broker's rows and the contract state, never inferred from fill prices or fixed at 100.
3. **Rolls.** Only a multi-leg order's legs are one roll. Separate orders on the same day are separate trades (a saved group joins them, as it does any trades).
4. **Expiry and assignment.** A contract past expiry with no row closes at zero only when it expired out of the money. At or in the money it waits for the broker's row (`no-expiry-record`): OCC exercises a contract $0.01 or more in the money unless told not to, so only the broker's row says what happened. An assignment delivers the shares at the strike into their own trade.
5. **Moves between your accounts.** A holding moved keeps its trade, cost and dates; it is not a sale and a buy.
6. **Position price.** A live quote from the listing's own feed; otherwise the last price read, which the cache keeps with its day, no date printed; otherwise no price (`—`). Your own fill price is never shown as the market price.
7. **Distributions, for any holding.** Every held payer has the market's record of its distributions: the exchange's record for a Canadian listing (TMX), Yahoo's dividend events for a US one, with the schedule wherever that record states it. Where a reader exists for the payer's own company, that company's record is used instead, since a company states a schedule change first. A company reader's failed read is that source's failure (the last record kept, the read retried, the header showing it), never a switch to another source. The rate is the cash per unit of the latest distribution gone ex; the income figure waits only where no source states the schedule. Never worked out from gaps between dates and never 12 assumed. (The text in `SPEC.md` §1 and §2 still says TMX and gaps.)
8. **Currency conversion.** Every currency the Bank of Canada publishes, on the transaction's day (the previous business day's on a weekend or holiday), not USD alone. A value held today uses the Bank's latest published rate. The rate's date is shown nowhere; a Bank not read past 16:30 on a business day shows as a failed source in the header, as every failure does.
9. **A figure the app cannot state** (decided): `—` with one short word saying what it waits for, in the figure's place, as `deposited` does today. Totals add what is stated, and their second line says how many they left out (`2 trades waiting`).
10. **Per-month figures** (income per month, interest per month) come from the server; the page no longer divides by twelve.
11. **Open trades in the Trades list** (decided 2026-09-24, in `SPEC.md` §2): Close reads `Open`, and opening one opens the holding page.
12. **Time** (decided): times of day in the viewer's local time; days, months, years and "today" in the zone of the browser in use. Each page states its zone when it opens the stream; the server uses the latest zone stated and keeps it in the book for when no page is open. A date a source states as a date (ex, record and pay dates, a daily value's day, a close's session) is never converted through a zone; only moments are. The server's own zone is never used.
13. **Clear data** (decided): a checkbox for each kind of data the app stores, and Clear all, which ticks every box; after Clear all nothing is left, the Wealthsimple login included. Disconnect stays. While a bracket is live, Clear data names it and does not run until it is cancelled in the Orders panel. `SPEC.md` §4 The menu changes, and §Order ticket, Submit ("`Clear data` does not touch them").
14. **Avg annualized.** Nothing under a year is annualized, as GIPS and Sharesight rule and as the fund pages in our fixtures state; under a year the tile shows the return over the period (`SPEC.md` line 86). Days are counted from the base day, the last value before the first year, so a full year is 365 or 366 days, not 364.
15. **The Equity curve card's switch** (decided): `P&L` (the default), the running total of Realized P&L in scope, each realized part on its own day, following every filter; `Value`, the account value series, following the account filter and naming the filters it does not read, as Cashflow does. At the right of the card's title, remembered on this machine. Returns, the index comparison and Max drawdown stay on the account value series.

## Scope

Stage 3c of the order of work in `docs/design-review.md`: the server serves every figure `SPEC.md` defines from the book, the market cache and the engine, and nothing it shows is computed from the old store again. What it delivers:

- **Parity locked first** (brief 01 §2.4): the pixel comparison with `ledger.html` at 1200, 1340, 1440 and 1680 px on the demo book, its differences fixed, then screenshot baselines of every tab, the trade and holding pages and each overlay at the four widths, taken on the old path. From then on a baseline changes only where `SPEC.md` changes, named in the commit.
- **The server holds the book, the market cache and the engine**, keeps the engine current by applying each change (`Engine::apply`), and runs the pull and every due reader at the moment each is due.
- **The wire, once** (brief 01 §2.3): money and quantities as exact decimal text; every row with its typed id; patches keyed by declared ids only (the key inference goes on both sides); changes numbered, a missed number answered with the full state; a figure the engine cannot state arrives as its gaps.
- **The page on the new wire**: decimal text formatted without a float, sorted by a decimal comparator, no money arithmetic in the page; open trades in the Trades list; gaps shown; the two entry forms 3b defined (the cost of units that arrived without one, in Add trade; an event's allocation, on the trade page); Add trade and CSV import writing records to the book.
- **Clear data** by kind and all, the time rule, the Equity curve's switch, and the Bank's series checked against the person's oldest day.
- **The `SPEC.md` changes** listed above, and those in `docs/plans/stage-2-engine.md` ("Changes to `SPEC.md` at the switch").
- **The old figure path removed**: the old model's figures, the old store's figure tables, the old Wealthsimple sync and client for reading.
- **The real run**: a live pull on the owner's sign-in into a scratch copy, 3b's last criterion.

Out of it, on purpose:
- **Order execution** (stage 4): orders and brackets keep their tables and logic in the old store until stage 4 rebuilds them as state machines with event logs in the book; moving them now would move them twice. What changes for them here is only what they read (accounts, instruments, rates, buying power), which comes from the book, and whose session they use (one, below).
- **The market around the book** (stage 5): news, filings, short interest, fund exposures, gauges, the heatmap's universes, index tiles and chart bars keep their readers and their tables in the old store until stage 5 moves each behind the source contract (`docs/plans/stage-3a-sources.md`, Scope). None feeds a figure the engine computes. Their values keep their current wire types until then; the wire marks them apart (below).
- **The watchlist, tiles, notification settings and history** stay where they are until stage 5 moves the market context and the notifier; they belong in the book (§6) and move with the code that reads them, once.
- **The browser-side store with resume across restarts**, subscriptions per screen and engine-reported changes (§13): stage 5. Numbering, and ids on every row, are here.
- **How the person signs in** (stage 5, with the keychain; `docs/plans/stage-3b-wealthsimple.md`, Right to refuse).
- The phone apps, the Python and Go builds: frozen.

## The old app here

The old app's code is not consulted (`CLAUDE.md`). What the running Rust server does in this area, and what is wrong with it:

- **Figures from the old model on the old store** (`server/src/model_cache.rs`, `model/src/view.rs`): identity by Wealthsimple ids and symbols, floating-point money, the relabelled rows. `docs/old-app-mistakes.md`, "Identity and the record", "Money and rates". Replaced by the engine on the book.
- **Money on the wire as `f64`** (77 `f64` and 45 `Option<f64>` fields in `model/src/wire.rs`), formatted and summed in the page as `number` (`web/src/lib/fmt.ts:37-132`; sums in `Portfolio.svelte:72-90`, `Cashflow.svelte:15-35`). New entry: "Money on the wire as binary floats". Replaced by decimal text.
- **Patches keyed by guessing** (`model/src/patch.rs:32`, `web/src/lib/live.ts:26`): the first field unique across a list at that moment, so a row can be identified by its symbol or its name. New entry: "Rows identified by whichever field is unique". Replaced by declared ids.
- **Changes not numbered**: nothing detects a lost message (`server/src/events.rs`, `web/src/lib/live.ts:199-248`). Replaced by numbered changes.
- **"Today" from the server's own zone for figures** (`model::clock::today_local`) and from the browser's for the page's own labels (`web/src/lib/fmt.ts:20`): the two disagree whenever the server runs in another zone. Replaced by the zone of the browser in use, the engine's input.
- **Clear data** deletes some old-store tables and keeps the login (`store/src/admin.rs:283-311`); after the switch it clears each kind the person ticks, everything with Clear all.
- **CSV import** reads leniently (`store/src/csvimport.rs`: `parse_float`, `lenient`), writes rows with made-up ids into the activity table and drops "rows already stored" by comparing fields. `docs/old-app-mistakes.md`, "Bad rows dropped or read as zero". Replaced by an adapter writing records.
- **Add trade** writes an activity row with an `f64` quantity and price (`web/src/lib/ui.svelte.ts:220-259`, `server/src/orders/manual.rs`). Replaced by a person record.
- **A booked fill** is written into the old store's activity table (`orders/readback.rs:261-319`), which nothing reads after the switch. Replaced here by a pull of that account when an order fills (below); the provisional booking of §6 is stage 4's.
- **Two sessions**: after the switch the reads use the adapter's session (`wealthsimple/src/session.rs`) while orders use the old client's (`ws/src/session.rs`), each with its own lock on the same file; two refreshes at once post one single-use refresh token twice and the loser signs the person out. Replaced by one session.

Carried over, each with why it is right:
- **The typed differ and the stream** (`model/src/patch.rs` typed path, `diff-derive`, `server/src/events.rs`, the page's `applyOps`): kept by the design review ("Typed routes, page request path, element-level page updates: Keep"), and held by the element tests (`web/src/lib/tick.svelte.test.ts`). Only the key inference goes.
- **The pull's schedule** (`SPEC.md` §2 Market data: weekdays after 2 PM Mountain, and Sync now): what the person sees stays unless `SPEC.md` changes, and the owner's rule on Wealthsimple calls (only what changed) is the pull's own (3b).
- **The quote and portfolio reads only while a page is open** (`feeds.rs:2450`, `session.rs:601`): right under §13, a price is read because something shows it; they move to the new readers, not away.
- **The import of an existing database** (`book/src/import`, stage 1): the person's journal and groups carried into a new book once; the first pull then supersedes its rows (3b).

## How the leading products do it

Read 2026-09-24 from each product's own pages unless marked as a search summary. TraderSync's help centre refused every fetch (HTTP 403).

- **The day a trade falls on is a zone the journal fixes, not the server's.** TradeZella has a display time zone in its global settings, which charts and statistics follow (https://intercom.help/tradezella-4066d388d93c/en/articles/8164985-global-settings-tab-explained). Tradervue works in US Eastern: an intraday trade is one "starting and ending on the same calendar day, in US Eastern time" (https://app.tradervue.com/help/reports_dt). TraderSync keeps it as an account setting (search summary of https://tradersync.com/support/can-i-change-my-timezone/). Edgewonk converts imports to the person's local zone (https://edgewonk.com/time). None documents its default. *Departure:* no setting is added (nothing was asked for); the zone is that of the browser in use, the latest one stated kept for when no page is open, so it is the person's local zone as Edgewonk's is and never the server's.
- **A figure that cannot be stated is kept out of totals, visibly.** Sharesight shows an error and excludes an invalid holding "from performance, total value and all reports until it is corrected" (https://help.sharesight.com/us/negative-balance/). Tradervue's detailed reports cover closed trades only (https://help.tradervue.com/article/3479-open-trades-and-p-l-discrepancy). No product found shows how many rows a total left out. *Departure:* the total says how many it left out, because a total must say what it covers (`docs/architecture.md` §1: never a number the app cannot stand behind).
- **Clearing all data takes the notes with the trades.** TradeZella's "Delete / Clear All Trade Data" removes "all trades and associated data", notes recoverable for 30 days (https://help.tradezella.com/en/articles/5851898-how-to-delete-clear-all-trade-data-in-your-tradezella-account); Tradervue's bulk delete "cannot be un-done" (https://help.tradervue.com/article/3432-delete-trades). What each keeps of the broker connection is not stated. Bagholder's clears by kind, and Clear all leaves nothing, the login included (the owner's decision).
- **Duplicates on import are matched on the execution.** Tradervue flags an execution with the same symbol and exactly the same timestamp as a duplicate, and says it misfires when two accounts trade one symbol at one instant (https://app.tradervue.com/help/faq). TradeZella documents no detection; its remedy is undoing the import (https://help.tradezella.com/en/articles/6153629-i-see-my-trades-as-duplicates-in-tradezella). *Departure:* a CSV row is linked to a broker row only on account, day, instrument, side, quantity and price with exactly one candidate (`docs/architecture.md` §6), so the misfire Tradervue names cannot link across accounts.
- **Exact amounts travel as decimal strings.** Coinbase Advanced Trade sends `price`, `size` and `commission` as strings (https://docs.cdp.coinbase.com/api-reference/advanced-trade-api/rest-api/orders/list-fills); Alpaca sends `qty` and prices as strings (https://docs.alpaca.markets/reference/getallorders-1); Stripe sends integers in the currency's minor unit (https://docs.stripe.com/currencies); Plaid sends doubles (https://plaid.com/docs/api/products/investments/). MDN: `Intl.NumberFormat.prototype.format` given a string "will use the exact value that the string represents" (https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Intl/NumberFormat/format). Taken: strings, as Coinbase and Alpaca do; minor units cannot carry a price's or a coin's digits.
- **Today's value of a foreign holding uses the prevailing rate.** Sharesight's current value is price × quantity "converted into your local currency" (https://help.sharesight.com/show_holding/); its valuation report uses the prevailing intraday rate and names it at the report's end (https://help.sharesight.com/multi-currency-valuation-report/). No product found shows the rate's date beside a value, which change 8 follows.
- **An equity curve in a journal is the running total of realized P&L.** TradingView, Tradervue and TradeZella draw it from closed P&L; brokers (IBKR, Wealthsimple) set a time-weighted return on account value against an index (`briefs/reference.md`). Taken: both, as the card's switch, with returns on value.
- **Annualizing.** GIPS and Sharesight annualize only periods of a year or more; the fund pages in our fixtures say the same. Taken.
- **Resuming a stream.** The HTML standard's server-sent events: an `id` sets the last event id, which the browser sends back as `Last-Event-ID` when it reconnects (https://html.spec.whatwg.org/multipage/server-sent-events.html). Taken for the numbering; resuming across a restart is stage 5.

## Open questions

1. **Where the demo book comes from after the switch.** Objective: the baselines taken on the old path compare with the new path on the same book. Known: the demo is written into the old store by `store/src/bin/demo_book.rs`; the import (`book/src/import`) reads an old database into a book. Settled here: the demo is generated from one definition into both, the old database (before the switch, for the baselines) and a book with its facts and market cache (after), the book through the import so its rows are the same rows; the old half goes with the old store. This is the best course because the comparison then isolates exactly what the switch changed.

## Approach

### 1. Parity locked, on the old path

- The pixel comparison of `docs/parity.md:54`: `ledger.html` and the Svelte page on the same demo book at the four widths, every difference fixed or named as a `SPEC.md` change.
- `web/e2e/baselines.spec.ts`: `toHaveScreenshot` of each tab, the trade page, the holding page, the ticket, the Orders panel, the notifications panel, the menu and the filter at the four widths; the clock, the version and live regions masked; run in the CI Linux image with its fonts (`.github/workflows`), where the baselines are made and checked. A test run elsewhere compares nothing and says so in its output.

### 2. The server's state

`App` (`server/src/app.rs`) gains `Book`, `MarketCache` and the engine behind one lock (`Arc<RwLock<Engine>>`), built at start from `engine_inputs::{ledger, facts, brokers}` and `read_sources::market_from_cache`, which already build it for the commands. Every writer applies its change and nothing else (`Engine::apply`, `engine/src/engine.rs:283`):

| Writer | Change |
|---|---|
| a pull that stored, revised or removed records | `Ledger` (and `Broker` for statements) |
| an entry, Add trade, a CSV import | `Ledger`, `Adjustments` |
| journal, grade, tags, groups | `Journal`, `Groups` |
| a rate, a holiday, a payer's record, a frequency | `Rates`, `Declared`, `Frequency` |
| a quote, a close, a benchmark | `Quote`, `Closes`, `Benchmark` |
| the home day turning | `Clock` |

A book whose version is newer than the build, or a failed migration, stops the server with the page saying so (§15), as `Book::open` refuses today.

**First start of the switched build on an existing data folder**: no `book.db` and an old database present → the import runs once (journal, groups and notes carried; `server/src/legacy_import.rs`), then the first pull reads every account in full and supersedes the imported rows (3b).

**The zone** is a book setting (migration 007, `settings(key, value, source, set_at)`, schema snapshot `v7.sql`). Each page states its browser's zone (`Intl.DateTimeFormat().resolvedOptions().timeZone`) when it opens the stream; the server stores the latest one stated with its time, and a zone different from the stored one is a `Clock` change to the engine, so "today", months and years move to it. With no page open the stored zone stands. The server's own zone is read nowhere: a test fails on `jiff::tz::TimeZone::system`, `Zoned::now` or `today_local` on the figure path. A date a source states as a date (ex, record and pay dates, a daily value's day, a close's session) is a `civil::Date` from the adapter to the page and never passes through a zone; only moments (`Timestamp`) are converted. Before any zone has been stated there is no reader to build a figure for: each page states one before its first figure.

### 3. When each read runs

One scheduler task (`server/src/due.rs`) calls the pure due-functions 3a built over the book, the cache and the clock, sleeps until the earliest instant any is due, and wakes early on a change that can make one due (a new holding, a screen showing a contract). Each wait is a known deadline, listed in `TIMED_WAITS`:

| Read | Due | Function |
|---|---|---|
| the Bank's rates | a business day after 16:30 Eastern without its rate; a currency first seen; the archives when a need reaches before the daily series | `rates::due_daily`, `due_archives` |
| the Bank's holidays | monthly | `rates::due_holidays` |
| a payer's record | its schedule's next distribution due and unread; a payer first held; a release announcing distributions (the news loop's, stage 5's reader) | `payers::run::due` (every held payer, §3a) |
| daily closes and benchmarks | each session after it settles, for the benchmarks and the span a chart shows | `market::due_span`, `due_close` |
| an option contract's chain | while a screen shows the contract, as `SPEC.md` §2 says | `options::chain_due` |
| quotes | every minute while a page shows the price | the quote loop, now `sources::quotes::read_quotes` into the cache |
| the pull | weekdays after 2 PM Mountain; Sync now; an order read back filled, for that account alone | `broker::pull::pull` |
| balances and buying power | every five minutes while a page is open and connected (`SPEC.md` §2) | the adapter's `cash` |

The pull's report reaches the header as the sync status does now: a failed part is a sync error naming it; a suspect read (3b's guard) is one naming the account; rows removed are listed in the log the command prints.

**One session.** The adapter's `SessionFile` (`wealthsimple/src/session.rs`) is the only one. The sign-in (`server/src/login.rs`) writes through it; the order code's client (`ws`) is given its tokens by the adapter and its own refresh (`ws/src/session.rs:501-555`) is removed, so one lock guards every refresh.

**What orders read**: the ticket's accounts, instruments, rate and cash from the book and the engine; buying power from the adapter's balances read. Their own rows stay in the old store (stage 4).

### 3a. Distributions for every held payer

Today only a fixed list is read: the fund companies in `sources/src/payers/mod.rs:117-138` and the companies `payers/companies.rs` names; a holding outside it waits forever (`payers/run.rs:78-81`). The rule replaces the list:

- `adapter_for` picks the payer's company reader where one serves it, and otherwise the market's record for the listing's market: TMX's declared distributions and stated schedule for a Canadian listing, Yahoo's dividend events for a US one (the readers `exchange.rs` uses today, which become the market reader and stop being tied to a brand). No brand, symbol or company is named in that choice.
- A company reader that fails is that source's failure: the last record kept, the read retried after its rest, the header naming it. It never falls to the market's record.
- The schedule is stored only where a source states it; where none does, the income figure waits, named.
- **Test:** on recorded replies, a payer no company reader serves gets its record and schedule from the market's (a Canadian listing and a US one); a company reader's failed read keeps its record, is named, and asks the market nothing.

### 3b. Avg annualized

`engine/src/stat/returns.rs:147-163` annualizes from one month up (`yrs >= 1/12`), and `days` counts from 1 January while the year's return runs from the last value before it. After: a period under a year is the return over the period, not annualized; `days` counts from the base day. **Tests:** two months at +15 % reads +15 %, not +131 %; five full years at 10 % each read 10.00 %.

### 4. The wire

A new module, `server/src/wire/`, replaces `model/src/wire.rs`, built from `Figures` and `Scoped` (`engine/src/engine.rs:217`, `scope.rs:445`) and generated to `web/src/lib/generated/wire.ts` by the existing ts-rs test:

- **Decimals as text.** `Dec` and `Money` serialize as the exact decimal string (`"1247.41"`), typed in the page as `type Dec = string & { readonly __dec: unique symbol }`. No money or quantity field is `f64`; a test fails on an `f64` or `number` money field (a scan of the generated file against a list of the fields that are ratios, counts and days).
- **Figures that may not be stated** are `Fig<T>` on the wire: the value, or `{ gaps: string[] }` with the gap words (`engine/src/gap.rs:81`). A total adds `leftOut`.
- **Ids.** Every row carries its id: trades their `TradeId` (or the key of a round trip without one yet, a group its `GroupId`), positions `(account, instrument, direction)`, cash rows their `TransactionId`, payers their `InstrumentId`, accounts their `AccountId`; a row with no entity of its own gets one from what it is (a month `2026-09`, a year `2026`). The differ keys by `#[diff(key = …)]` only; `KEYS` and `row_key` (`model/src/patch.rs:32-44`) and `rowKey` (`web/src/lib/live.ts:26-39`) are deleted.
- **Numbered.** Each message on a stream carries its number as the SSE `id`; the page checks each is the next, and on a gap asks for the full state (`POST /api/events/resync`), which it reconciles by id. A reconnect sends the full state (resume across restarts is stage 5).
- **The market around the book** (Markets tiles, heatmap, fear and greed, short interest, news, filings) keeps its current types in `wire::context` until stage 5 moves its readers; prices of held and watched listings are the new reader's and are decimal text.
- **Instruments, not symbols**: filters name instruments (`SPEC.md` §5 already names symbols as values; the value becomes the instrument id, shown by its symbol, per `docs/plans/stage-2-engine.md`, Filters).

### 5. The page

- `fmt.ts` takes `Dec` and formats with `Intl.NumberFormat.prototype.format(string)`, which formats the exact value the string states; `web/src/lib/dec.ts` holds the comparator for sorting (`sort.svelte.ts:54-60`) and nothing else, with tests. A chart's plotted coordinate is the one place a `Dec` becomes a number (`plot(d)`), never shown and never summed; hovers format the text.
- No money arithmetic in the page: the Portfolio donut's "Other" slice, the Cashflow tiles' per-month and share figures come from the server; the ticket's notional, risk, CAD value and cash after come from the server's ticket read (`GET /api/order/quote`, extended), computed exactly from what the person typed. A test scans `web/src` for `Number(`, `parseFloat`, `+`, `-`, `*`, `/` on a `Dec` (the type checker refuses arithmetic on the branded type; the scan covers casts).
- A gap shows `—` and its word in the figure's place (`SPEC.md` §3, Missing, amended); the words are listed in `SPEC.md`.
- Trades list: open trades, Close reads `Open`, opening one opens the holding page (`docs/plans/trade-open-to-flat.md`, Wire and page).
- **Entry forms**: in Add trade, an opening balance against an arrival the book holds as `basis-unknown` (the arrival picked, the total cost in the instrument's currency and the day acquired); on the trade page, beside the journal, while an event waits (`event-unknown`), the share of cost for each child of a spin-off or the capital returned per unit; each through `POST /api/entries` to `Book::enter` (`book/src/person.rs:122`), its refusals shown in the form. An entry shows as entered by the person (`SPEC.md` §2, What you enter).
- **Add trade** writes a person record (a new `Entry::Trade` in `PersonMapping`, version 2) with its quantity, price and fees as decimal text.

### 6. CSV import, as an adapter

The CSV importer becomes a broker adapter without execution (§10) in `crates/broker` (`csv.rs`): each layout (`store/src/csvimport.rs:1-13`: Wealthsimple's activity export, its statement export, the Date/Action/Symbol layout) read strictly, a row that fails kept with its problem, each row a source record keyed by its file and line's content, mapping version 1. A row matching a broker row already in the book (same account, day, instrument, side, quantity and price) with exactly one candidate is linked, not added; more than one candidate links nothing and says so (§6). The watch folder scans as now.

### 7. Clear data

The menu's Clear data opens a dialog with a checkbox for each kind of data the app stores, and Clear all, which ticks every box. The kinds, each owning its tables: the broker's records (records pulled, statements, the pull's state), your entries (Add trade, CSV imports, opening balances, allocations), the journal (notes, grades, tags, groups), market data (the market cache), orders and brackets (the old store's until stage 4), the watchlist, tiles and notification settings (the old store's until stage 5), and the Wealthsimple login (`session.json`). Every table in the book, the cache and the old store belongs to exactly one kind, in one list in the server.

`POST /api/data/clear` takes the kinds ticked. It refuses while a pull runs, and while a bracket is live, naming it: deleting its record would leave its stop resting at Wealthsimple with nothing watching it, so it is cancelled in the Orders panel first. Otherwise it empties each ticked kind's tables in one transaction per store, deletes `session.json` when the login is ticked, snapshots nothing (no backups feature), and rebuilds the engine; the stream sends the new state. After Clear all nothing is left. Disconnect stays as it is.

### 8. What goes

- `bagholder-model`: everything but what the market context reads until stage 5 (`markets`, `symbols_of`, `venues`, and the context's wire types, moved to `server/src/wire/context.rs`); `base`, `view`, `fifo`, `fx`, `lenient`, `patch`'s JSON path and the old cases runner go.
- `bagholder-store`: the figure tables and their code (`activities`, `accounts`, `balances`, `margin`, `nav_history`, `securities`, `fx_rates`, `distributions`, `quotes`, `benchmark_prices`, `merge`, `relabel`, `csvimport`, `gens` for figures), each table dropped by an old-store migration; the context's, orders' and notifications' tables stay until their stages.
- `bagholder-ws`: its sync, mapping, reading and session; what orders send stays until stage 4.
- The boundary test gains the rule that nothing on the figure path (`server/src/wire`, `due.rs`, the HTTP routes serving figures) imports `-model`, `-store`, `-ws` or `-market`.

### 9. `SPEC.md`

The changes listed at the top, and from the stage 2 list: trade marks (each mark defined or kept off the page: `reward` is shown today; the rest do not reach the page, since nothing asked for them), §2 Market data's rows (the Bank for every currency; Wealthsimple's pull as 3b reads it; every held payer from its company's record or the market's), the rate rule (§2, after the table), and the passages the design moves out of the spec (Freshness, The store): replaced by what the person sees, since the implementation they described is gone.

### 10. The Equity curve card's switch

`P&L · Value` at the right of the card's title, as the Annualized returns card's, remembered on this machine. `P&L` (the default): the engine's running total of Realized P&L in scope by day, each realized part on its own day (a new `Figures` series, following every filter). `Value`: the account value series of today, following the account filter, the card naming the filters it does not read as Cashflow does. Returns, the index comparison and Max drawdown stay on the value series. Built as its own step after the figures are on the engine.

### 11. The steps

Each step is a commit with the whole suite green (Rust, the page, the browser tests); a baseline changes only where a `SPEC.md` change in that step names it.

1. **Parity baselines** (§1), on the old path.
2. **The server's state and scheduler** (§2, §3, §3a, §3b): the engine held and applied, the zone, the due-readers, one session.
3. **The wire** (§4).
4. **The page** (§5), on the new wire.
5. **CSV** (§6).
6. **Clear data** (§7).
7. **The Equity switch** (§10).
8. **The old path removed** (§8), then the real run.

## Acceptance criteria

- [ ] **Build.** `cargo test -q --workspace` green in `rust/`, `RUSTFLAGS="-D warnings" cargo build -q --workspace --all-targets` clean, the applet test green alone.
- [ ] **Landed in steps.** Each of §11's steps a commit with the whole suite green.
- [ ] **Parity locked before the switch.** The pixel comparison's differences fixed or each named as a `SPEC.md` change (`docs/parity.md:54` checked); baselines committed at the four widths from the CI image; after the switch every baseline unchanged except where a `SPEC.md` change of this plan names it, listed in Verification.
- [ ] **Page.** `npm run check`, `npm test`, `npx vite build` and `npx playwright test` green in `web/`, with a browser test for each behaviour this plan gives the page: an open trade in the list, a gap shown with its word, a total's left-out count, each entry form stored and refused, Add trade, a CSV import with a linked and an ambiguous row, Clear data, a missed change number answered with the full state.
- [ ] **No float, no guessed key.** A test fails on a money or quantity field typed `f64` in `server/src/wire` or `number` in `wire.ts`; `KEYS`/`row_key`/`rowKey` gone, and a test fails on a list type without a declared key; the page's scan finds no arithmetic on `Dec`.
- [ ] **Only what changed.** A test per writer in the table of §2 that the engine after `apply` equals a fresh build, and that the stream sends only the changed fields (the element tests kept green). `TIMED_WAITS` lists every wait the scheduler adds, each with its reason.
- [ ] **One session.** A test that two refreshes at once (reads and an order) post the refresh token once.
- [ ] **Time.** A test runs every zone in the time-zone database as the page's zone, with the server process in several zones, UTC among them: every moment, including those around each daylight-saving change, falls on its day in the page's zone; no stated date changes; a page that then states another zone moves "today" to that zone's. A scan fails on the server's own zone read on the figure path.
- [ ] **Distributions for any holding.** The test of §3a green; no brand, symbol or fund company named in the choice of reader.
- [ ] **Avg annualized.** The two tests of §3b green; `SPEC.md` line 86 changed.
- [ ] **Expiry.** An engine case: a contract past expiry with no row closes at zero out of the money, and waits (`no-expiry-record`) at and in the money.
- [ ] **Equity switch.** A browser test: `P&L` by default follows every filter; `Value` follows the account filter and names the others; the choice survives a reload.
- [ ] **Clear data.** A test lists every table in the book, the market cache and the old store: it fails on any table Clear all leaves filled and on any table it does not know about; `session.json` is gone after Clear all; each kind alone clears its tables and no other's; a live bracket refuses, naming it; the next pull reads in full. A browser test drives the dialog.
- [ ] **The old figure path gone.** No module on the figure path imports `-model`, `-store`, `-ws` or `-market` (the boundary test, checked by feeding it a violation); the old store's figure tables dropped by its migration, tested on a copy of an old database.
- [ ] **The real run.** On a scratch copy of the owner's data, run with `BAGHOLDER_DRY_ORDERS=1`, `BAGHOLDER_NO_BROWSER=1` and the order and bracket loops off, every Wealthsimple request counted and listed in Verification, with the owner signed in once on it: the first start imports the old database; the first live pull's report in Verification (accounts, records new and superseded, removed, suspect, requests sent); every imported record with a Wealthsimple id superseded or listed with why; no journal entry orphaned, or each listed; a second pull the same day sends only 3b's fixed minimum.
- [ ] **The Bank's series reach the person's oldest day**: every currency in the book has a rate for every transaction day, or each day without one is a `rate-not-held` gap listed in Verification.
- [ ] **Rendered.** The Rust scratch server on that copy, at 1200, 1340, 1440 and 1680 px: every displayed figure traced to its engine field, no table overflowing at 1340 and above; screenshots in Verification.
- [ ] **The figures that change, handed over.** `compare-figures` run last on the same copy; its differences grouped by the `SPEC.md` changes above, each group's count in Verification; none unexplained.
- [ ] **Docs.** `SPEC.md` (every change listed at the top and of the stage 2 list), `docs/decisions.md` (the owner's answers), `docs/old-app-mistakes.md` (the three new entries), `docs/design-review.md` (3c done), `docs/parity.md`, and `engine/tests/no_guesses.rs`'s comment on the old crates say what the code does.

## Surfaces to check beyond the diff

- `web/src/lib/generated/*.ts` (wire, routes, book, model_api) regenerated; `web/e2e` baselines.
- Book migration 007 and `schema/v7.sql`; the old store's migration dropping its figure tables.
- `TIMED_WAITS` (`server/src/tests_misc.rs:479`).
- `docs/old-app-mistakes.md`; `rust/crates/engine/tests/cases/README.md`.
- `server/src/orders/*`: every read of the old store's accounts, securities, rates and margin replaced; their own tables untouched.
- `.github/workflows`: the baselines' job.
- `sources/src/payers/*` (the reader choice, `exchange.rs` becoming the market reader) and `sources/tests/payers.rs`.
- `engine/src/stat/returns.rs` and the Annualized returns card.
- The Clear data kinds list against every migration that adds a table, in all three stores.

## Right to refuse

Three departures from what was written before, each argued:
- **The market context stays on its readers until stage 5.** `engine/tests/no_guesses.rs:24` says the old crates go at the switch. `-market`'s news, filings, shorts, exposures, gauges, universes and bars are stage 5's by 3a's scope, and nothing they read is a figure; moving them now would be stage 5 done out of order. What goes at the switch is everything a figure is built from.
- **Orders keep their rows in the old store until stage 4.** Stage 4 redesigns them with event logs in the book; a move now is a move twice.
- **A chart's coordinate is a number.** Brief 01 §2.3 says money never goes through `Number()` in `web/src`; a chart library places points by number. The conversion is one function, used only for a coordinate, never shown and never summed; every figure shown is formatted from the text.

## Anti-stub self-check

- No definition nobody references: `settings`, `Entry::Trade`, the CSV adapter, `/api/entries`, `/api/events/resync` each read by the server, the page or a test.
- No field written and never read: every wire field is shown by the page or used by it (a test compares the generated types with the page's reads).
- No branch only the switch knows: this is the switch.
- No real-target run skipped: the live pull on the owner's sign-in, and the page rendered on the scratch server on a copy of the owner's data.

## Verification

(Filled as it is built.)

## Handoff

(Filled as it is built.)
