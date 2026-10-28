# Plan: every amount as Wealthsimple states it

Brief 79 (amended 2026-10-27). Line numbers are from master at dcd86f17 (2.11.5). The capture of §3 is withdrawn: every point it was to record is decided below from the code and the recorded replies, and each names its basis. Where neither settles a point, this plan says so and says what the release does there; nothing is guessed.

## For the owner to decide

**The currency of Book and Avg for a holding listed outside Canada.** The rule is that the figures are the ones Wealthsimple shows the person (brief §1), so it is the currency Wealthsimple shows the owner for their US holdings' book value. The Architect asked the owner on 2026-10-27. The plan shows the currency they name (§2.2). Nothing else waits on it.

## What exists today

- **Cash is already Wealthsimple's on every screen.** The Portfolio shows the newest `TRADING` balance (`server/src/engine_inputs.rs:228-231`), split into cash, margin used and chequing (`engine/src/scope.rs:1224-1279`); buying power is stated (`scope.rs:1280-1286`), and account value is `netLiquidationValue` (`scope.rs:815-817`, `wealthsimple/src/read.rs:76-91`). Bagholder's own cash (`Matched.cash`) has one production reader, the broker check (`engine/src/equity.rs:139`).
- **Units and book value are the journal's.** A holding's quantity is the sum of its open lots (`engine/src/positions.rs:156`), its book the ledger's basis (`:170-171`), its average one over the other (`:173-177`). Every screen and tool that shows a holding reads these (§6).
- **Wealthsimple's units and book value are read, stored and dropped.** The pull reads `FetchHoldingsExportPositionsAsOfDate` as of yesterday (`broker/src/pull.rs:391-422`) and stores each line's units, book value and value (`book/src/statements.rs:549-563`, table `statement_units`). The units reach `PositionFig.broker_qty` for the check alone (`positions.rs:283`); the book value is read by nothing outside a test.
- **Past days are rebuilt.** A past day's cash is today's stated cash less every later transaction (`CashByDay`, `scope.rs:920-984`); its holdings are the journal matched again up to that day (`engine/src/engine.rs:592-622`).

## 1. The rule in the code

**One function builds the holdings, from what Wealthsimple states alone.** `engine/src/stated.rs` (new) holds `holdings(stated: &StatedUnits, quotes: &Quotes, rates: &Rates, day: Date) -> Vec<Holding>`. Its inputs are the stored positions read for each account on the day (§2.1, §2.4), the quotes and the exchange rates. It takes no records, lots, trips or transactions, so nothing the journal works out can reach an amount (§5, test 2).

For each line of the read:
- **units** are the read's `quantity`, a short's negative, as `read::units` already takes them (`read.rs:120-147`);
- **book value** is the read's `bookValue`, in the currency it states (§2.2);
- **average cost** is book value over units;
- **market value** is units times Bagholder's quote for the listing (times the contract's multiplier for an option), the quote being the only figure of Bagholder's own;
- **unrealized** is market value less book value, in the book value's currency (market value converted at today's rate where the two differ).

Allocation, exposure, the heatmap, net worth's holdings, the Cashflow holdings table and projected income are built from these holdings, as they are from the journal's today.

**"Held" follows Wealthsimple's holdings everywhere**, not only on the screens §6 lists:
- **What gets quoted, and which securities' payers are read:** `needs` takes its held listings from the stated holdings, not from `Matched.units` (`engine/src/needs.rs:92-104`), so a holding moved in with no trades is quoted and valued.
- **The held lists** for news, filings, short interest and the exposure refresh: `MarketBase.positions` (`model/src/context.rs:51`, `:63-88`) is built from the positions document, so they follow it (`server/src/feeds.rs:269`, `:481`, `:1056`, `:1240`; `filings.rs:307-309`, `:566`).
- **The notifications' "held" scopes** (`releasesHeld`, `disclosuresHeld`, `server/src/notify.rs:31-32`) read those lists.

What stays on the journal, and why:
- **The currencies a trade is converted in** from its first day held (`needs.rs:96-99`): the journal's own CAD results need them.
- **`payer_rate`'s held-on** (`engine/src/cashflow.rs:251`): whether units were held the day before an ex-date decides a distribution's form, which is the journal reading payments against its trades, not an amount shown.
- **A bracket's sold total** (`server/src/orders/brackets.rs:130-162`): a sum of the fills since it armed, which are Wealthsimple's own records.

**A holding links to the journal, never the other way.** Where the journal has an open round trip in the same account and listing, the holding carries that trip's id, so its thesis, grade, tags and executions open from it as now. A holding the journal has no open trip for (moved in, or the journal reads it differently) shows Wealthsimple's figures with no journal link. A trip the journal has open where Wealthsimple states nothing held is not a holding; it stays in Trades, the journal's.

## 2. The reads (§3 of the brief)

### 2.1 Units now: the positions read, asked as of today

**Decided:** units, direction and book value come from `FetchHoldingsExportPositionsAsOfDate` asked as of **today**, for every account in one request, with the balances: every five minutes while a page is open (`server/src/broker_reads.rs:231-239`), and at each pull.

**Basis:**
- It is the one read that states a holding's units, direction and book value together. The average is book over units of the same statement at the same moment. The balances read's entries carry only `quantity` and `securityId` (`graphql/FetchAccountsWithBalance.graphql:28-32`), with no direction, currency or book value.
- Its recordings cover 8 accounts on 17 days (`wealthsimple/tests/replies/wealthsimple/positions@*`, `wealthsimple-pull/positions@*`), and every one writes a short's quantity negative (`read.rs:116-118`). The balances read's security entries are recorded once, in an edited reply whose security ids match no position (`wealthsimple-pull/edited-balances.json`).
- It is the same read, parsed the same way, for today and for every past day (§2.4): one mechanism.
- The document takes a list of accounts (`accounts(ids: $accountIds)`, as the balances read's does), so asking every account costs one request, chunked by 20 as the balances are (`client.rs:316-320`). Today it is sent one account at a time (`client.rs:303-314`); the build sends the list.

**Not settled by any recording:** whether a read asked as of today states a fill made today. No reply was recorded for the day it was asked. The read states each currency's cash beside the units (`read::cash_held`, `read.rs:151-161`), so the build logs, never shows, when that cash differs from the balances read's cash of the same pass. The line carries no amount and no id: the account's kind, the day and "differs", once per account and day. The owner's use of the release is the check (brief §3): a holding that lags a fill is a recorded form to add, never a header line.

**The fallback, named and not built:** if the owner reports a lag, units now come from the balances read's security quantities (`sec-s`, `sec-o`, `sec-z`, skipped today at `read.rs:184`), recorded first.

### 2.2 Book value: in the listing's own currency, as Wealthsimple shows it

Amended 2026-10-28 (2.12.1), replacing the open point this section held. The owner: "an individual stock should never show the wrong currency", and "There can be no ambiguity as to how WS answers".

The read is asked as Wealthsimple's own web app asks it: `currency: "CAD"` with `currencyOverride: "MARKET"` (`wealthsimple/src/client.rs`, `CURRENCY_OVERRIDE`). The web app, release 0.3.720893 read on 2026-10-28 with no sign-in, calls this query in its chunk 652 with `variables:{accountIds:i,asOf:...,currency:"CAD",currencyOverride:G.o2C.MARKET}`, and its main script defines that enum as `tn={MARKET:"MARKET"}`, its one value.

- **A holding's Book** is the node's `marketBookValue`, in the currency the node states it: the listing's own. **Avg** is that over the units. Bagholder converts neither.
- **`bookValue`, in CAD, stays** for what adds up in CAD: a holding's book in CAD, and from it the Portfolio's totals. Unrealized in CAD is the market value in CAD less it.
- **A node stating no `marketBookValue`** leaves the holding's Book a figure not read (`— cost`), never taken from another source. A day read before 2.12.1 asked for it is read again when a page shows that day or a sale's units wait on it; a day read for its cash alone is not.
- **Point 5's stated lots** take `marketBookValue` too.

2.12.0 showed `bookValue` in CAD for every holding. It stayed a pre-release.

### 2.3 The cash shown: unchanged

`TRADING` is the only balance type the code sends and the recordings hold. `SPEC.md:403` says the cash Wealthsimple states is net of holds for rows not yet final; no other type is recorded (`docs/plans/broker-check-reserved-cash.md:31`, `:44`, `:63`). The Portfolio already shows it. The order ticket's `tradingBalanceViewV2` (`server/src/orders/ticket.rs:712`) is the ticket's own read and stays.

### 2.4 Past days: the positions read as of the day, read once and kept

A past day's units, book value and cash are the positions read as of that day: its lines for the holdings, its `sec-c` lines for the cash. Each read is stored as the pull stores yesterday's today (`store_units`, `book/src/statements.rs:549-563`; `statements.kind = 'units'`, keyed by account and `as_of_day`), and is never asked for again. The pull goes on asking for yesterday each day, so a day read while it was today is stated again as it closed.

**When a day is read:**
- **A past Portfolio** (`portfolio_as_of`, `scope.rs:1397-1464`) asks for its day when it opens and the book does not hold it: one request for every account. Until it is answered the screen shows what it shows for a figure not yet read today.
- **The net worth history** needs each past day's cash for two things: what a margin account owes (`owed_of`, `scope.rs:1076-1110`), and what an account with no stated daily value holds. The days it lacks are read on demand: only while a page shows the net worth history, as the market sources read only for what a page shows (`docs/architecture.md` §13). Newest first, one request a second, every account in each. It pauses when no page shows the history, and each day read is kept. A day not yet read draws as the history draws a day it has no figure for now. Account values on past days are already Wealthsimple's daily figures (`scope.rs:1407`) and are unchanged.
- Margin, buying power and outside debts on a past day stay none, as now (`scope.rs:1451-1457`).

**The bound:** one request per past day, every account in it, once for the life of the book; and never while no page shows a past day.

### 2.5 Coins

No recorded positions or balances reply holds a coin, so how the positions read states a coin's units is not recorded. A coin's holding is what that read states, by the same parser, with units as the read writes them (`Dec::parse_to_fit`). The journal takes a coin fill's units as the activity feed states them: a decimal string written to 1, 5, 6 or 8 places (`wealthsimple/tests/replies/wealthsimple/edited-activity-one-per-kind.json`). The units step that replaced them from statements goes (§3.5). The dust write-off stands (`engine/src/ledger.rs:1519-1538`).

## 3. What goes

Each item is all of it; nothing else calls it (callers from `grep -rn` over `rust/` and `web/`).

### 3.1 Bagholder's own cash
- `engine/src/ledger.rs`: `Matched.cash` (355-358), `Continued.cash` (2401-2402), `cash_of` (2618-2630), the cash written at 2650, 2676, 2817 and 2929.
- `engine/src/engine.rs`: the cash shares (`shares` 295-297 and 477, `ByFamily` 388-389, their part of `match_families` 391-409, `total` 411-428, the cash part of `match_all` 526-532 and `rematch_families` 560-588, `add_shares` 1472-1488), the oracle `same_as_whole` (1500-1506) and the cash compare in `match_disagrees` (1815, 1841-1843).
- `rematch_families`' `cash_moved` and the writes of `Reach.accounts` (855-935, 1045-1048) fed only `recheck`, and go with it.

### 3.2 The broker check
- `engine/src/equity.rs:98-271` (`Difference`, `BrokerCheck`, `broker_checks`, `Held`, `held`) and its doc at 9-10. Its tests at 273-329 cover `account_equity` and stay.
- `engine/src/engine.rs`: `Entity::BrokerCheck` (55), `checks` (287, 472), `Figures.checks` (437, 724), the quote and broker rechecks (1031-1037, 1081-1093), `recheck` (1687-1702) and its uses in `rederive` and the snapshot (1668-1671, 1720, 1778-1780, 1919-1936).
- `PositionFig.broker_qty` (`positions.rs:81-82`, `:283`; `engine.rs:197`); `dust::is_dust_worth` (`engine/src/dust.rs:12-22`); the check's inputs `cash_read`, `cash_read_holds`, `activity_read_at` and `held_as_of` (`engine/src/input.rs:780-796`, filled at `server/src/engine_inputs.rs:214-241`).
- `Matched.last_price` **stays**: the matcher prices dust with it (`ledger.rs:1225-1237`).

### 3.3 The header lines that compare
`server/src/status.rs` `broker_failures` (193-236) keeps only the "took N more than the book held" lines (224-234, brief §7). The units lines (208-223) and `broker_label` (238-241) go. After this, no sentence of the app says Bagholder's figures differ from Wealthsimple's.

### 3.4 The gate on a mapping version
`server/src/figures.rs`: `retry_refused` (459-473) and its call (`broker_reads.rs:441-442`), `REFUSED_VERSIONS` and `WORSE` (905-910), `gated_rederive` (912-973), `restate_walk_records` and `disagreeing` (1004-1026). `rederive_all` (894-903) stays and calls `book.rederive` directly. `broker/src/statements.rs`' restating helpers (768-939) go with §3.5. A book's stored `rederive.refused` setting is left unread.

### 3.5 The monthly statements, whole
- **The read:** `FetchMonthlyStatementWithTransactions` (its `.graphql`), the request sent with the web page's own headers (`wealthsimple/src/client.rs:38`, `:45-66`, `:156-163`, `:341-346`) and `Tokens.device`, which only those headers use (`wealthsimple/src/session.rs:38`, `:226`; set at `server/src/session.rs:141`).
- **The walk and the units step:** `broker/src/statements.rs` (1,547 lines) and `wealthsimple/src/statement.rs`'s reading (§3.7 keeps its mapping); `broker/src/lib.rs`' `Step::Statements`, `StatementRow`, `StatementRead`, `StatementKeys`, `cash_on` and the source's statement methods; `broker/src/codes.rs`' `fill_of` and `plain_number`; the pull's call and `Report.statements` (`pull.rs:48-50`, `:375-389`); the adapter's statement half (`wealthsimple/src/adapter.rs:56-59`, `:76-81`, `:105-111`, `:374-406`, `:427-433`, `:669`, `:705-735`).
- **The book's statement-only helpers:** `keep_monthly_statement`, `monthly_statements` (`book/src/statements.rs:157-179`); `mapped`, `note_taken_back`, `live_records_written` (`book/src/records.rs`); `first_superseded_for` (`book/src/links.rs:186-191`).
- **The stand-in's statement shapes:** in `wealthsimple/src/generated.rs`, `STATEMENT`, the statement shapes and `SHAPES`' entry, `FILL_FORMS`, `READ_BY_2_10_9`, `ASSIGNED`, `Said`/`fill_line` and `Source::statement`; `wealthsimple/src/replay.rs`' statement parts.
- **The server's other uses:** `pull-broker --statements-only` (`server/src/pull_broker.rs:5-103`), the step text and the log line (`broker_reads.rs:365`, `:443-445`), `csv_import.rs`' statement tiers (557-578, 656-690), `capacity.rs:1418`.
- **Recorded data:** `wealthsimple/tests/replies/wealthsimple-statements/` (6 files, 82 KB) and `wealthsimple/tests/books/` (3 files, 596 KB, read only by `take_back.rs`).

### 3.6 `CashByDay` and the rebuilt past holdings
`CashByDay` (`scope.rs:920-984`); `engine.rs`' `Entity::Past`, `Past`, `PastCache`, `past()`, `past_built` and their stale handling (67-150, 592-631, 708-721, 766-789); the `as_of` branches of `build_positions` (`positions.rs:178-181`, `:213-216`, `:283`). Their readers (`server/src/views.rs:120`, `:238`, `:434-437`, `:502-552`; `wire/build.rs:589`, `:804-807`; `events.rs:541`; the net worth history) read §2.4's stored days instead.

### 3.7 Records an earlier build booked from statements
The current read wins (no migration). The first pull under this build withdraws every live record of source `wealthsimple-statement` with `Book::withdraw` (`book/src/links.rs:125-174`), which hands each superseding fill's trades back to the feed record it replaced, so that record is live again. A cash booking, gap or side paid booked from a statement is withdrawn with nothing to hand back. `withdraw` and `replaced` stay for this step.

`wealthsimple/src/statement.rs`' mapping (`StatementMapping`) stays, registered with the book (`server/src/figures.rs:387`), because migration 039 maps records with it (`pooled::step`): a book older than 039 runs it before any read. It is removed with the statement tables at the next schema change. Nothing else reads it: no new statement record is ever written. `monthly_statements` and the settings `rederive.refused` and `statements.said` are left unread.

## 4. What stays

- **The journal:** each trade, round trip and result, from the activity feed. The "took N more than the book held" line, which is the journal's reading and not an amount (brief §7).
- **The positions read as of yesterday at each pull**, now one request for every account.
- **`Matched.last_price` and the dust write-off.**
- **The account's stated daily values** (`account_days`), restated on a run's first sync (2.11.5).

## 5. Tests, each against its rule

1. **A book whose history is wrong still shows Wealthsimple's amounts exactly.** Over the stand-in, every shape at twelve seeds and two todays: each amount in the page's data equals the stand-in's stated figure. That covers the positions document, the Portfolio's totals, net worth today and on a past day, the Cashflow holdings table, the ticket's Sell quantity and Max (from the positions document), and the agents' `positions` tool. It holds again after the history is spoiled: a buy removed, a fill doubled, a conversion's side missing, a month of rows missing. **Plant:** a holding's units from the journal's lots fails it.
2. **No path from the history to an amount.** A scan of `engine/src/stated.rs` and the signature of `holdings`: it names no record, lot, trip, transaction or `Matched`. **Plant:** a `&Matched` parameter fails it.
3. **No sentence says the figures differ.** The header's text table, the routes golden, and a scan of the server's strings for every phrase the comparing lines used ("states", "the book holds", "does not reconcile", "no statement lists"). **Plant:** the units line restored fails it.
4. **The journal stands.** The figures golden's diff names every journal figure that moves; only the coin fills a statement superseded may move, each named in the PR. Over every recorded form at twelve seeds, the journal leaves no "took more" line and no row unplaced or kept with a problem.
5. **A past day is Wealthsimple's and is read once.** A past Portfolio's units, book value and cash equal the stand-in's positions as of that day; opened twice, the read is made once. **Plant:** `CashByDay` restored fails the cash.
6. **Bagholder takes units now from the read asked as of today.** The stand-in fills an order today: the next balances pass asks the positions read as of today, for every account in one request, and the holding shows the stand-in's answer. This proves Bagholder's side only. The stand-in answers as of today in the recorded positions form; whether Wealthsimple's read includes a fill made the same day is not recorded (§2.1), and the PR says so. **Plant:** the read asked as of yesterday fails it.
8. **A holding with no journal trip is held.** The stand-in states a holding no activity row names: it is quoted, counted as held by `needs`, and listed in `MarketBase.positions` and the filings' held list. **Plant:** `needs` taking its held listings from `Matched.units` fails it.
7. **Statement records are withdrawn by the first read.** A book holding a statement fill that superseded a feed record, and a statement cash booking: after the first pull the feed record is live with its trades, and nothing of source `wealthsimple-statement` is live. **Plant:** the withdrawal removed fails it.

**Removed with their code:** `wealthsimple/tests/statements.rs` (44 tests), `forms.rs` (8), `take_back.rs` (1); eight statement tests in `wealthsimple/tests/pull.rs`; the `pull_broker` walk test; `engine/tests/past.rs` (2) and `past_range.json` (3 cases); the check's cases (7 of 11 in `returns_filters_checks.json`, `cases.rs:617-653`); `status.rs`' two compare tests (515-598); the gate's tests (`figures.rs:1130-1225`, `:2007-2096`); `capacity.rs:1476`; `due.rs:461`.

**Kept and changed:** three cases in `coins_and_transfers.json` (their `checks` dropped); `status.rs:474-501` (a source failing, without the check); `wealthsimple/tests/pull.rs:72` and the every-shape, read-whole-again and carried-from-version-36 tests; `calls_written_and_assigned`; `stated_currency.rs:44-47`; `net_worth.rs`' four day tests; `csv_import`'s and `clear`'s tests; the figures golden, blessed.

## 6. What the person sees change

- **Every holding's units, book value, average and unrealized are Wealthsimple's** on the Portfolio, Cashflow, the order ticket's Sell quantity and Max, net worth, exposure, the heatmap and the agents' `positions` tool.
- **A listing outside Canada shows Book and Avg in the currency the owner names** (§2.2). Market value stays in the listing's currency, and unrealized is in Canadian dollars.
- **A holding moved in with no trades** shows, with no journal link.
- **The header's units lines are gone.**
- **A past Portfolio** opens on a day not yet read with what it shows for a figure not yet read, then fills in.

## 7. Docs, in the build PR

`docs/architecture.md:93`, `:121` and `:146`; `SPEC.md`'s units sentence at 397, the movements the activity feed leaves out (`movements-the-activity-feed-leaves-out-1`, 187) and the monthly statements' row in the sources table (`market-data-12`, 227); `docs/decisions.md`, with the owner's words of 2026-10-27.

## Not included

As brief §7: the "took N more" line; the journal's deposited lots and the opening-balance form; fees and execution days from statements; dropping the statement tables; brief 75's PR 4 and PR 5; anything else on the page. Also not included: the fallback of §2.1, reading the balances read's security entries.

## Order

1. This plan, reviewed (the Architect's go with five conditions, 2026-10-27, each above).
2. One build PR doing §1 to §3 and §7 with §5's tests, released MINOR.
3. **The release check** on a copy of the owner's book, on their Mac, read-only, as the canary always has: each holding's units and book value on the page against the book's stored positions read, and every comparing line gone. It reports per line identical, new or worse, with no figure; the session reads no figure itself. No session reads the sign-in or what it returns.
