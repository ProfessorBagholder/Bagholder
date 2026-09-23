# Plan: stage 2, the engine on the book

## Scope

The second stage of the order of work in `docs/design-review.md`. It delivers `docs/architecture.md` §8 (the engine): every figure of `SPEC.md` §2 and the aggregates of §4, computed from the book's transactions, the facts the figures use and the market data, by one pure implementation held to cases written from the spec; the engine's half of §13 (recompute only what a change touches, report what moved); the facts of §6 kept in the book (the rates, declared distributions, recorded closes and the values of corporate events and the person's adjustments); and trade identity that survives corrections (§5).

Like stage 1, it is built beside the running app, and the app does not read it yet. The engine is proven on copies of the person's data by a comparison with the old model, every difference attributed to its cause and checked against the record.

**The switch moves to stage 3.** The order of work had the app switch to the book in this stage. The review of this plan found why it cannot: the imported rows are the old app's derived rows, not what Wealthsimple sent. The old store relabels every option row after each pull (`store/src/relabel.rs`): each option buy and sell becomes "to open" (all 60 buys and 59 sells in the person's book), each multi-leg order becomes a buy to close or a sell to open by the sign of its cash, and none of the 13 multi-leg rows states a quantity. The facts the figures need (the Bank's rates for every currency, contract sizes, event values, stated payout frequencies) have no strict reader yet, and a fact is written to the book once and never rewritten, so it must not be written through the old lenient readers. Switching now would put figures built on those guesses in front of the person, or leave them waiting on facts nothing fetches. Stage 3 builds the source contract with strict checking, the fact readers as adapters, and Wealthsimple's adapter, whose own rows replace the imported ones; the app switches there, with no bridge from the old store. `docs/design-review.md` is changed to say so.

`SPEC.md` describes the app the person runs, which this stage does not change, so its edits land with the switch in stage 3; the list is under *Changes to `SPEC.md` at the switch* and is recorded in `docs/design-review.md`'s stage 3 so it cannot be lost.

## Approach

### A new crate, and what it may depend on

`bagholder-engine` (`crates/engine`) holds every figure rule. It depends on `bagholder-core` alone, and `core` re-exports `jiff` so the engine has dates without a second dependency. The boundary test gains the engine's column: no dependency but `bagholder-core`, no clock reads, and `f64` only inside `engine/src/stat/` (win rate, profit factor, percentages, returns, drawdown: the statistics §6 keeps in floating point, each made from exact figures through `Dec::to_f64`). The old model crate is untouched and is deleted with the old app (stage 6).

### The entry point

```text
Engine::build(Inputs) -> Engine          everything, from scratch
engine.apply(Change) -> Moved            one change, recomputing only what it touches
engine.figures() -> &Figures             every per-entity figure
engine.scope(&Filters) -> Scoped         the aggregates over one filter set
engine.identity() -> &Identity           the round trips that need a trade id, and the trades a correction joined
```

**Inputs**, typed, all handed in:

| Part | What it holds |
|---|---|
| `Ledger` | accounts and their broker; instruments with their dated names, option terms and issuer; the live transactions; the problems on each record; the links that pair a transfer out of one account with its transfer into another; trades (anchor → id), groups and their members; the journal |
| `Facts` | the Bank of Canada's rates per currency and day, the currencies it publishes and its holidays; declared distributions per instrument, as of each read; stated payout frequencies; adjustments (corporate event values and the person's own entries, below) |
| `Market` | quotes per instrument with their time and source kind; daily closes per instrument; benchmark levels; what each broker states per account (cash per currency and units per instrument now, its net value per day, buying power) |
| `Clock` | today in the person's home zone, the current instant, the home zone, and the Bank's zone |

**Figures**: the trades, positions, cashflow rows, income holdings, accounts, the equity series (total and per account) with returns and drawdown, the reconciliation against each broker, and the gaps (below). Money is `Money`, quantities `Dec`, dates `jiff` dates; only the statistics are floats.

**Scoped**: what one filter set (`SPEC.md` §5) produces: the KPI tiles, monthly P&L, by symbol, grades, the review queue, the portfolio tiles and allocation, the cashflow tiles, months and holdings, the equity block for the accounts in scope, each tile with the count of what it left out. Every per-month figure (projected income, average interest) is the engine's; the page divides nothing.

**Change** has one variant per kind of input: a transaction added, changed or removed; a record's problems; a transfer link; a trade, group or journal entry; an instrument's names or terms (a multiplier stated); an account's type, status or name; a rate, a holiday, the published currencies; a declared distribution read; a stated frequency; an adjustment; a quote; a daily close; a benchmark level; a broker's statement; the day turning; the home zone. What each recomputes:

| Change | Recomputed |
|---|---|
| anything in the ledger but the journal; an adjustment; an instrument's terms | the match and everything from it: milliseconds on a real book, so in full |
| a quote | the positions of that instrument, then the totals that include them |
| a daily close | the positions it prices, and the equity series from that day |
| a rate or a holiday | the CAD figures of the days it governs |
| a declared distribution, a stated frequency | that instrument's income holding, then the income totals |
| the journal, a group | that trade's journal fields, the grade and review figures |
| a broker's statement | that account's reconciliation and broker figures |
| a benchmark level | the yearly returns |
| the day turning, the home zone | hold days, year-to-date and trailing figures, and the rates the new day waits for |

**Moved** names, for each change, the entities whose figures moved, by kind and id, and which fields. A test applies every variant of `Change` (a `match` over the enum, so a new variant cannot be left out) to a built engine and requires the result to equal a fresh build field for field and `Moved` to name exactly what differs. Stage 3 reads `Moved` for the correction notice and stage 5 carries it to the page.

### Arithmetic

- **A fill's value** is its cash less its fee, since the cash is what moved: a purchase pays its value and its fee (value = −cash − fee), a sale receives its value less its fee (value = cash + fee). Only a fill that states no cash (one the app booked itself) is valued at its price × quantity × the contract's multiplier. Where a record states both and they disagree, the cash stands and the record is a problem shown to the person. A value that comes out negative is a sign error on the record and a gap, never taken as its absolute value.
- **Cash in another currency than the instrument's** (a US share bought from a Canadian-dollar balance) is converted at the rate the broker states it applied; with none stated, the fill's value is a gap (`currency-unstated`), never converted at the Bank's rate, which the broker did not use.
- **Parts of a whole**: a lot closed in part, or a fill closing several lots, is shared by quantity to 12 decimal places, the last part taking what is left, so the parts always add up to the whole exactly. So a closed round trip whose fills all state their cash, and whose lots were never moved at cost (by a transfer or a corporate event) or delivered by an assignment without stated cash, has a P&L equal to the sum of its fills' cash, exactly; a test holds that on every case it covers.
- **P&L** of a slice is exit value − entry value for a long (reversed for a short) less both fees, which is `SPEC.md`'s definition; since the values are gross of the fee, each fee is subtracted once.
- **Prices** (entry, exit, average cost) are averages for the screens: total value ÷ (quantity × multiplier), rounded only where they are shown. They are never an input to anything.
- **CAD** amounts are exact products of the amount and the rate, rounded only where they are shown.

### The order of transactions

Applied by the day the broker files them under. Within a day, a transaction that states only its day and brings units in comes before the day's timed ones, and one that states only its day and takes units out comes after them (a position cannot be closed before it is open); the timed ones go by instant. At one instant, or among the undated, records go by what they do: one that states it opens a position, then one that brings units in, then one that takes units out, then one that states it closes. Then by the record's source key, never by when it was stored, so storing a row again never reorders the book. Within one record, closing legs come before opening ones (a roll closes the old contract, then opens the new).

### The ledger

- **Lots per account and instrument**, first in first out, never per symbol and currency. Two instruments that share a ticker never share a lot.
- **Shares, funds, coins, event contracts.** A buy covers a short the record opened, then goes long; a sale closes longs, and a sale with nothing to close is a sale beyond what is held, a gap on the transaction, never a short invented from it. A short is opened only by a transaction that says so, or by an assignment that obliges delivery of shares not held.
- **Options.** A contract's position is net within an account, as every broker keeps it: a buy closes shorts first and opens long with the rest, a sale closes longs first and opens short with the rest. Where the record states the effect (to open, to close) it is checked against that: a close with nothing to close is beyond what is held, and an open that meets an opposite position is a gap on the transaction. The imported rows state no effect (below).
- **Multi-leg orders.** A roll is one record whose legs close one contract and open another on the same underlying, the same way, in one account: the opening leg continues the round trip the closing leg belonged to, so the chain is one trade named after its last contract. A multi-leg order whose record does not state its legs (the 13 imported rows) moves nothing, and every contract on that underlying in that account, held then or opened later, is waiting on it (`leg-unstated`) until the record states the legs: any of them may be the leg that was not posted.
- **Expiry.** An expiry row closes the contract at zero; one that states no quantity closes the whole holding of that contract, which is what an expiry is. A contract past its expiry with nothing on the record is closed at zero on its expiry date only when the underlying's close that day shows it strictly out of the money by its terms (a call's underlying below the strike, a put's above); at the strike or in the money, or with no close, its figures wait (`no-expiry-record`) for the broker's row.
- **Assignment and exercise.** The contract closes at zero, keeping its premium, and the underlying moves by contracts × the contract's multiplier at the strike: a call's holder and a put's writer receive the shares, the other side delivers them. The movement's value is the cash the record states for it, else strike × shares. It is its own round trip (below). Without a stated multiplier the underlying waits (`multiplier-unstated`).
- **Transfers.** An asset deposited is held at an unknown cost (`basis-unknown`) in a round trip of its own, never merged with bought lots, and its sale is left out of the performance figures, as now. A transfer out takes lots off first in first out at their cost, with no P&L. Where the book links a transfer out of one account to the transfer into another (the person's own accounts), the lots move with their cost and the days they were bought: when the whole holding moves, it is the same round trip, now held in the receiving account; when part of it moves, the part is a round trip of its own there. The link's writer is stage 3's broker adapter, pairing by the broker's own ids for the two sides, never by matching amounts. The person's cost-basis entry for a deposit (an adjustment, below) gives its lots that cost.
- **Coins**: a staking reward opens a lot at zero cost, flagged `reward`; a staking move changes nothing; a residue left after a sale is a lot like any other.
- **Corporate events.** The broker's own quantities are the statement of what moved: units in and out on the event's day are applied as stated. An adjustment gives what the event was, as a list of legs (from, to, units per unit held, share of cost moved, cash per unit), which covers a split or consolidation, a new security id continuing a holding, a stock dividend, a spin-off with any number of children, a merger for cash, shares or both, cash in lieu, a return of capital and a fund merger. A split's ratio applies only to a marker that states no quantity; where the broker states the units, its quantities stand and the ratio is the check. A whole holding continuing under another instrument keeps its round trip, cost and dates; a spin-off's child is a round trip of its own, its lots carrying the parent's acquisition dates and their share of its cost. An event without an adjustment moves the units the broker states at an unknown cost and leaves the holding waiting (`event-unknown`). The engine never decides what kind of event a row was: that is the adjustment's.
- **Waiting spreads forward, not back.** A transaction the engine cannot apply leaves its account's holding of that instrument waiting from that day (and, for a multi-leg order, every contract on the underlying): every slice matched there afterwards, and the position, carries the gap. Nothing before it changes.

### Trade identity

A round trip's key is its opening transaction and instrument (so the shares an assignment delivers never share a key with the contract). A round trip takes the id of the trade anchored on its earliest opening that has one. So a back-dated row that becomes the opening leaves the trade's id and anchor where they are; the book is asked for a new trade only for a round trip none of whose openings has one (`Identity::needs_trade`, which the app passes to `Book::open_trade`, then gives the engine the trades). When a correction joins two round trips, the trade on the earlier opening keeps the id and the other is listed in `Identity::joined`, for the book to orphan with the reason `joined <id>`, its journal kept and shown for the person to re-attach. When a correction splits one, the part holding the anchor keeps the id and the other asks for a new trade. A whole holding continued by a corporate event or moved whole by a linked transfer keeps its round trip and so its id; a part moved, or a spin-off's child, is a new round trip (above), so no two positions ever share an id. A position's id is its round trip's trade id. A round trip that moved between accounts shows the account it is held in now, or was closed in. A group shows its members' round trips as one trade; a member that no longer names a round trip is left out of it and listed with the orphaned trades; two members joined into one round trip are that one round trip.

Migration 2 adds the anchor's instrument to `trades`, filled for every existing trade from its anchor transaction's instrument; a trade's anchor is unique on (record, leg, instrument), and the book's `open_trade` takes the instrument.

### What each figure is

- **Trades and positions**: `SPEC.md` §2's fields, per round trip and per open holding. A position's price is its quote when the quote's source is its kind's (the listing's feed for a share, the coin's spot, the contract's chain); else the last stored daily close, dated; else it has none (`price-unknown`). The person's own fill price is never a market price.
- **Cashflow rows**: each dividend, interest, withholding tax and interest charge, in the currency it was paid, with its CAD value on its day. A dividend of zero cash is the broker's notice of one to come and is not a payment. A stock dividend's income is a Dividend row, in units, at its adjustment's value.
- **Income holdings**: the declared record's latest cash distribution that has gone ex, special and non-cash distributions left out of both the amount and the ex-date gaps; payments per year stated by a source first, then read from the declared record's ex-dates, then from the payments received (`SPEC.md` §2's median rule); with none of these, `frequency-unknown`, a problem the app keeps working on, never 12. A listing held is looked up; being a payer is not the trigger (stage 3's reader).
- **Rates** (`docs/architecture.md` §7): a transaction uses the Bank's rate for its trade date if that is a business day, else the previous business day's. What is not a business day is read from the Bank alone: a weekend, a day in its holiday schedule, or a weekday that a completed read of the series covered and found no rate for. A weekday no completed read covered is a business day whose rate is not stored, so a read that failed can never pass for a holiday: before 16:30 Eastern that day the rate is pending, the one time a rate does not exist yet, and after it a failure of the Bank source. Any currency the Bank publishes converts; one it does not is `rate-unpublished`, never taken to be CAD.
- **Live marks** (a position's market value, unrealized P&L, the day's change, allocation, today's equity) are values at the present instant, not transactions on a day, and use the latest rate the Bank has published; once a business day's 16:30 has passed without its rate stored, that is a failure of the Bank source, shown, not an ageing rate. The owner confirmed this rule for live marks on 2026-09-23; where the rate's date appears on screen is settled in `SPEC.md` at the switch.
- **Days**: everything about a transaction (its rate, its month, its year) uses the day the broker files it under; "today" and the year-to-date and trailing windows are the person's, in the home zone. The home zone is a setting, which stage 3 takes from the page's browser when it is not set, since a server may run in UTC.
- **The equity series** (`docs/architecture.md` §8):
  - Per account and day: its cash in each currency plus each holding's units × its close × multiplier, in CAD at that day's rate. A holding's close on a day is the last session's close on or before it (so a weekend is Friday's), a coin's the UTC day's close. The total is the sum of the accounts.
  - Flows, which returns are net of: money and assets moved in or out (deposits, withdrawals, employer and government deposits, card spending and refunds, and assets transferred in or out at their day's value); a transfer between two of the person's accounts is a flow of each account and nets to nothing in the total. Income, interest, fees and trading are not flows.
  - Every account the person has counts, card and spending accounts included, as the broker's own net value does.
  - A day the own value cannot be stated is empty with its reason, not filled: a holding without a close, a rate not stated, an adjustment missing, and any problem on a record of that account on or before that day (an unclassified row, a quantity or leg not stated, a sale beyond what is held), from that day until the record is corrected. On such a day the series shows the broker's stated net value for the account where it has one, marked as the broker's.
  - An account's daily return is formed only between two days of the same source; across a gap, it runs from the last stated day to the next, net of the flows between, and counts on the day the gap closes. The total's daily return is the value-weighted return over the accounts with a return that day, each weighted by its value at the start of its span. Yearly returns, the annualized figure and the drawdown keep `SPEC.md` §2's definitions on this series.
- **The broker check** is exact, not a threshold: per account, Bagholder's cash in each currency and units of each instrument against what the broker states, every difference a problem named with both figures. A broker's balances include fills its activity has not posted yet, so a difference against a statement newer than the last full read of the account's activity (or with no read known) is pending, not a problem, until a read of the activity made after the statement confirms or clears it. The difference between the own value and the broker's net value is shown as a figure, not judged. From stage 3 the broker's statements are kept daily, so the check covers each day from then on.
- **Filters** name instruments, not symbols: the Symbol filter's values are the instruments in the book, each shown by its current symbol (with its venue or name where two share one), a trade matching when any instrument it held is chosen; the filter's search matches any name an instrument has had.

### Unknown and waiting

A figure the engine cannot state is a `Gap` naming the fact it waits for, never zero and never a guess: `rate-pending`, `rate-missing`, `rate-unpublished`, `multiplier-unstated`, `quantity-unstated`, `leg-unstated`, `event-unknown`, `basis-unknown`, `beyond-held`, `currency-unstated`, `price-unknown`, `close-unknown`, `frequency-unknown`, `no-expiry-record`. The page (from stage 3) shows `—` and the one word in the figure's place, as it does now for a deposited coin. A tile over many trades or positions is the sum of those whose figure is stated, and its subtitle adds how many it left out (`2 trades waiting`); the count is the engine's.

### Facts and adjustments in the book: migration 2

Each is kept because it cannot be fetched again as it was when used (§6). This stage builds the tables and their typed reads and writes, with tests; their writers are stage 3's readers and stage 5's screens.

- `fx_rates(currency, day, rate, source, received_at)`: every observation the Bank published, as received. The first stored for a day is the rate; a later different value for the same day is kept beside it and is a problem naming both, and the first stands.
- `fx_reads(currency, first_day, last_day, received_at)`: the span each completed read of a series covered, so a weekday with no rate inside one is known not to be a business day, and one outside every read is a rate not read.
- `bank_holidays(day, name, source, received_at)`: the Bank's own schedule.
- `declared_reads(instrument_id, read_at, source)` and `declared_distributions(read, ex_date, record_date, pay_date, amount, currency, kind)`: each read of a fund's record as a whole, so a distribution the fund withdrew is absent from the newest read, and `kind` separates regular, special and non-cash.
- `stated_frequencies(instrument_id, per_year, source, stated_at, received_at)`.
- `recorded_closes(instrument_id, day, close, source, received_at)`: the daily closes no source can give again later (an option contract's); the rest stay in the market cache (stage 3).
- `adjustments`: corporate event values and the person's own entries are records, as stage 1 made the person's entries: a source record (the issuer's notice, the exchange's bulletin, the person) whose mapping produces adjustment legs (`adjustment_legs(record_id, leg, applies_to, from_instrument, to_instrument, units_per_unit, cost_share, cash_per_unit, cost, acquired)`) for the transaction they explain, and a sourced record supersedes the person's. When the transaction's own record is superseded (stage 3's Wealthsimple row replacing an imported one), `Book::supersede` moves the adjustment's `applies_to` to the counterpart transaction exactly as it moves a trade's anchor, in the same database transaction.
- `trades.anchor_instrument`.

### The import, version 2

The import mapping (`bagholder-import`) goes to version 2, re-derived through `Book::rederive`: an option buy or sell states no effect (the stored "to open" was the relabel's guess); a multi-leg row states no quantity, no effect and no price and carries a `leg-unstated` problem; an assignment keeps its kind and drops the stored "to close". Each relabelled shape gets a fixture test. Stage 3's Wealthsimple rows supersede these records.

### Cases written from the spec

`crates/engine/tests/cases/*.json`: each a small book (accounts, instruments, transactions, facts, quotes, closes, today) and the figures `SPEC.md` requires, **worked out by hand from the definitions**, the working written beside the figures, never produced by running an engine. A runner builds the engine on each and compares every stated figure exactly. Cases cover: share round trips (whole, partial, same-day, several lots, fees shared, a round trip's P&L equal to its net cash), a sale beyond what is held, the order within a day (timed, undated, mixed), options (net position without effects, a stated effect that conflicts, a roll with both legs, a multi-leg record without legs waiting on every contract of its underlying, an expiry row, an expiry with no row out of and in the money, assignment of a call and a put, exercise, a multiplier of 150, a multiplier not stated), coins (a reward, a deposit with unknown basis standing as its own trade, a transfer out at cost, a residue), a linked transfer between accounts, event contracts, corporate events (a split marker with a ratio, a split stated as units in and out, a new security id continuing a holding, a stock dividend, a spin-off with two children, a cash merger, an event without an adjustment), positions (priced by quote, by last close, with no price, short), cashflow rows (a zero-cash notice, a dividend paid in another currency), income holdings (declared record, a special distribution, payments only, a stated frequency, none), rates (a weekend, a holiday from the schedule, a holiday seen from a later day, today before and after 16:30, a live mark before 16:30, a currency other than USD, one the Bank does not publish), trade identity (a back-dated opening, a join, a split, a group with a joined member), the equity series (own, a weekend, an asset transferred in, a record problem, the broker's figure on an unstated day, returns across a gap), the broker check, and each tile under a filter with a member left out. The old `tests/cases` stay with the old model.

### The comparison on the person's data

`bagholder compare-figures <old database> <book folder>` builds the old model on the old database and the new engine on the book (imported and re-derived with mapping 2, with the old store's USD rates, distributions, quotes and closes standing in for the facts stage 3 will read), and lists every trade, position, cashflow row, income holding and tile whose figure differs. Each difference is attributed to a cause; for each cause, a sample of its differences is checked by hand against the records themselves, not against the new engine's own flags, and the check is written in this plan's Verification. An unattributed difference is a bug in one engine or the other, found and fixed. The result is the list of figures that will change for the person at the switch, handed to them before stage 3's switch.

### Changes to `SPEC.md` at the switch

Made with the switch in stage 3, since until then the app the person runs still does what `SPEC.md` says: trade identity (Bagholder's trade ids, lots by instrument; `docs/architecture.md` §18); options matched as a net position, rolls only as one order's legs (the old same-day folding of separate orders goes; the person groups trades for that); splits and multipliers from the record and the contract; expiry without a record; assignment's delivery; transfers between accounts; the rate rule for transactions and for live marks; the position's price without the person's fills; payout frequency never assumed; the equity series and the broker check; tiles as the sum of what is stated with the count left out (a change to §1's "either exactly what this file defines or not shown", which now reads that a tile states what it covers); per-month figures from the engine.

## Acceptance criteria

The template's Python, Go, shared-case and page lines do not apply: the old builds and cases are frozen, and the page does not change in this stage.

- [ ] Rust, from `rust/`: the whole workspace's tests green and a warning-free build; then the ignored Mac notifier test alone.
- [ ] The boundary test fails when the engine gains a dependency other than `bagholder-core`, reads the clock, or uses `f64` outside `engine/src/stat/`, each checked by feeding the checker a violation.
- [ ] Every case in `crates/engine/tests/cases` passes; the cases cover every item under *Cases written from the spec*, each with its working.
- [ ] The invariants hold on every case they cover: the parts of a lot or fill add up to it exactly (all); a closed round trip's P&L equals the sum of its fills' cash (round trips whose fills all state cash and whose lots were never moved at cost or delivered without stated cash); a position's quantity equals the sum of its account's transactions in that instrument (holdings with nothing waiting and no split marker applied by ratio). A record whose stated price and cash disagree is listed as a problem, a case each way.
- [ ] The order within a day, each a case: an undated purchase before the morning's timed sale, an undated sale after the day's timed purchase, a short opened and covered at one instant, a roll's legs at one instant, and the same book stored in two orders giving the same figures.
- [ ] Rates, each a case: a weekday a completed read covered without a rate takes the previous day's; a weekday no read covered is pending before 16:30 Eastern and a failure after; a live mark uses the latest published rate and is a failure once a later business day's 16:30 has passed without its rate.
- [ ] Every variant of `Change`, applied to a built engine, equals a fresh build field for field, and `Moved` names exactly the entities and fields that differ (one test, a `match` over the enum).
- [ ] No old guess remains in the engine: no quantity inferred from cash, split inferred from prices, ticker-replacement search, fixed multiplier, fee tolerance, residue rule, fallback rate or position marked at a fill; a test reads the engine's sources for each, and a case holds what replaced it.
- [ ] Trade identity, each a test: an id survives re-deriving, a back-dated opening, a supersede, a corporate event continuing the whole holding, a whole holding moved by a linked transfer, and a fresh build; a part moved by a linked transfer and a spin-off's child are new round trips, so no two positions share an id; a join keeps the earlier opening's id and lists the other as joined; a split keeps the id on the anchor's part and asks for one trade for the other; an assignment's delivery never takes the contract's id; a position's id is its trade's.
- [ ] Migration 2: a book at version 1 migrates to 2 with its rows intact and every trade's anchor instrument filled from its anchor; `schema/v2.sql` is committed and compared; each new table's typed write and read is a test; a second different rate for a stored day is kept, reported, and does not replace the first; a read's span is kept; a withdrawn distribution is absent from the newest read; a sourced adjustment supersedes the person's; a supersede moves an adjustment's `applies_to` with the record, as it moves a trade's anchor.
- [ ] Import version 2: each relabelled shape (an option buy and sell under an order label, a multi-leg debit and credit, an assignment) maps as above, a fixture test each; re-deriving a version-1 book changes exactly those transactions.
- [ ] The comparison on copies of both of the person's databases: every difference attributed; for each cause, the sample checked against the records written in Verification; the counts in Verification.
- [ ] `docs/design-review.md` records stage 2 as done, and records as stage 3's: the switch without a bridge, the fact readers, the `SPEC.md` changes listed under *Changes to `SPEC.md` at the switch*, where the live-mark rate's date shows, the wire's number format, what Clear data does to the book and its journal, the home zone taken from the page, and how far back the Bank's series go against the person's oldest day.

## Surfaces to check beyond the diff

`rust/Cargo.toml` (the new member), `Cargo.lock`, `crates/core` (the `jiff` re-export), the boundary test, the book's migrations and `schema/v2.sql`, `Book::open_trade` and its callers (the anchor's instrument), the import mapping and `legacy_import`, the release workflow and the Rust Dockerfile (a new crate must not change what is built or attached), `SPEC.md`, `docs/design-review.md`.

## Right to refuse

Taken, on the switch. The order of work put it in this stage; the imported rows and the missing strict readers make it wrong here, so it moves to stage 3, where it needs no bridge from the old store.

## Anti-stub self-check

- Every new table has a typed writer and reader exercised by a test; its production writer is named (stage 3's readers, stage 5's screens).
- `Moved` and `Identity` are read by the equality and identity tests now, and by the app from stage 3.
- The comparison tool is run on the person's real data, not only on fixtures.

## Verification

(Filled in when the stage is built.)

## Handoff

(Filled in when the stage is built.)
