# Plan: stage 3a, sources and the facts the figures use

## For the owner to decide

1. **A daily run on your Mac for option closes.** A contract's closing price for a session can only be read that day: no source gives it later. Until the Rust build is the app you run every day, nothing of it runs daily. The choice: a scheduled job on your Mac runs `bagholder read-sources` once each weekday after the close, from when this part lands. It needs nothing from you once set up, touches only its own data folder, and places no orders. Without it, every session day until then loses the closes of the option contracts held that day for good; on those days the equity series takes Wealthsimple's stated account value where it has one, and waits where it has none.

Settled: a fund's schedule and distributions come from its fund company's own publication; another source only where that publication cannot be read at all (owner, 2026-09-24). Brief 02: Go with changes, applied.

Gated in this plan (brief 01 §1), not built until the gate says Go: book migration 3 (distributions without kinds, the reinvested part, rate series with their days) and the change to how the distribution rate is defined (cash per unit, from the payer's record only). A fund's schedule and distributions come from its fund company's own publication, never worked out from past dates (owner, 2026-09-23; brief 01 §2.1 as revised). Built already, as work inside the plan: the strict reply reader and the network crate with one limiter.

## Scope

Stage 3 of the order of work in `docs/design-review.md` is three parts, each with its own plan, review and verification, landed in order:

- **3a, sources and facts** (this plan): the source adapter contract of `docs/architecture.md` §9, with strict checking, recorded real replies and health; the market cache (§6's second store); and the adapters that supply what the engine reads:
  - the Bank of Canada's rates for every currency, with the spans read and its holidays;
  - each payer's distributions and schedule, from the fund company's own page or the company's own announcement;
  - option contracts' closes (written once into the book);
  - quotes, daily closes and benchmark levels (kept in the market cache).
- **3b, Wealthsimple as the first broker adapter**: its raw rows as source records superseding the imported ones, multi-leg orders' legs, contract sizes, account links, its statements, when each account's activity was last read, and corporate events. An event's values are taken from the official source where there is one (the issuer's filing, the exchange's notice, §6) and from Wealthsimple's rows where its own units state them. Which official source answers for which kind of event is 3b's first research, because the events stage 2 left waiting (a consolidation and two renames under new security ids) are tied to Wealthsimple's rows.
- **3c, the switch**: the server serving the engine's figures from the book and the market cache, exact decimals on the wire, the `SPEC.md` changes stage 2 lists, what Clear data does to the book, the home zone, and the server calling the readers when each is due.

Like stages 1 and 2, 3a is built beside the running app and changes nothing on screen. Its readers run from a command (`bagholder read-sources`) against a book and a market cache. The proof that they are right is the stage 2 comparison run again with every stand-in the old store supplied replaced by what these readers wrote.

Out of this part, on purpose: news, filings, short interest, fund exposures, gauges, universes and chart bars keep their current readers until stage 5 (the data flow) moves each behind the contract. None of them feeds a figure the engine computes.

## Approach

### Research first, each question settled before its reader is built

Each question below is answered from real replies before the reader that depends on it is written. The answer and its evidence are written into this plan's Verification. A question that cannot be answered stops that reader, and the plan changes first (*Right to refuse*). The fixtures that settle them are captured for instruments chosen for the question (a listing with a recent split, a fund with a long record).

1. **Split adjustment of daily closes.** *Answered.*
   - Does each close source (TMX `getTimeSeriesData`, Yahoo's chart, Coinbase candles) give closes as traded, or adjusted for later splits?
   - Checked on a listing with a known split, before and after it.
   - An adjusted source is used only with its own split events to undo the adjustment (Yahoo's chart reports them with `events=split`), so closes are stored as traded, matching the book's units on each day.
   - A source whose adjustment cannot be undone from its own reply is not used for closes.
   - **Found:** Yahoo's closes are split-adjusted and its reply states the splits (NVDA's 10:1 of 2024-06-10), so they are undone from the same reply. TMX's are adjusted too and its reply states no split (SHOP's 10:1 of 2022-06-29), so TMX is not a close source. Listings' daily closes come from Yahoo alone.
2. **Each payer's own statement of its distributions.** *Answered.*
   - A fund's distribution schedule and its distributions (the dates and the amount per unit) are published by the fund company on the fund's own page, and that is the authority (§9). Which page and which part of it states them is found for each fund company the person's funds come from, from its real pages.
   - A company that pays a dividend (a bank, a pipeline) declares each one itself, in the announcement it issues; which of its own publications states the amount, the dates and the schedule is found the same way.
   - Each fund company's page and each announcement form is one adapter under the contract, with its recorded pages as fixtures.
   - **Found:** fourteen of sixteen fund companies publish both over plain HTTP (the table in Verification); Global X's pages answer only a browser but its own monthly releases carry every fund's distribution and schedule; Mackenzie's and WisdomTree's answer only a browser and publish no releases found. The nine companies declare in releases carried by Cision (all nine), InvestorRoom/MediaRoom (three) and Q4's feed (CP); their own history tables are stale or partial. Pages miss rows, repeat rows, lag their own schedules and, once, disagree with the payer's own PDF.
3. **The Bank's rates before 2007-05-01.** *Answered.*
   - Valet holds its daily series from 2017-01-03 and its legacy noon series from 2007-05-01 (both verified on 2026-09-23).
   - **Found:** Statistics Canada's table 10-10-0008 carries the Bank's noon spot rates from 1950-10-02 to 2017-04-28 for twelve currencies still in use (USD, EUR, GBP, JPY, CHF, AUD, NZD, HKD, MXN, DKK, NOK, SEK), equal to Valet's noon series where both exist (checked for USD). It is read as a third series under its own source name. Its web service states a non-business day explicitly (a null value with status 1) and refuses bursts, so it is paced.
   - A day before a currency's oldest series (any day before 2007-05-01 for a currency outside those twelve) is a gap named for what it is, `rate-not-held`: no source holds the Bank's published rate for that day. It is never `rate-missing`, which names a failure of the Bank source.
4. **Which session an option chain's `prev_day_close` belongs to**, and which of the chain's prices is a contract's close for a session.
   - Settled from chains captured before and after a session's close on the same day, and across a weekend.
   - The recorded close must be defined one way (the chain's closing price, or the closing midpoint) before its reader is built.
5. **Benchmark sources.** *Answered.*
   - Replies recorded for each: FRED's S&P 500 series and TMX's `^TSX` and `^TX60` daily series (SPEC §2).
   - **Found:** FRED serves the trailing ten years only, two decimals, a holiday as a row with an empty value. Stooq now answers every request with a browser proof-of-work check instead of data, and S&P's own site refuses a plain request (403), so neither is used. Yahoo's `^GSPC` is the only reachable source for S&P 500 days before FRED's window; it agrees with FRED within half a cent on all but nine of 2,511 shared days. TMX answers `^TSX` and `^TX60` from 2001-12-11 in one reply, each row dated 16:00 with its offset.

### Two new crates, and where the existing code goes

| Crate | Holds | May depend on | May not |
|---|---|---|---|
| `bagholder-net` (`crates/net`) | the HTTP client (`market/src/client.rs`: kept connections, redirects, gzip, the offline switch) and one pacing limiter for every host | what `client.rs` already uses | the book, the market cache, any reply type; reading the clock except through the clock it is handed |
| `bagholder-sources` (`crates/sources`) | the adapter contract, the reply reader, the market cache and its migrations, the adapters of this part, and the domain logic they share with the old readers | `bagholder-core`, `-sqlite`, `-net`, `-book`, `rusqlite`, `jiff` | `bagholder-model`, `-store`, `-market`, `-server`, `-ws`; any clock read |

There is one copy of everything:
- **The moved code.** The client, the limiter and the domain logic the design review kept (TMX's venue forms, Yahoo's venue suffixes, the option midpoint rule, the Bank's reply parsing) move into these crates. `bagholder-market` depends on them and calls them, so the old readers the running app still uses (charts, news, exposures) keep working on the same code until stage 5.
- **One limiter.** Yahoo's own gate (`quotes.rs:55-128`, written three times) and SEDAR+'s (`sedar.rs:117`) become per-host settings of the one limiter: a minimum gap, and a rest after a refusal that honours the source's `Retry-After`.
- **One health record.** The old in-memory health (`market/src/http.rs:81-102`, never read) is removed; `outcomes` in the market cache replaces it.
- **One HTML table reader.** The holiday page is read with `market/src/htmltables.rs`, moved into `-sources`.

The boundary test gains both crates' columns.

### Reading a reply: exact and strict

- **Exact.** A reply is parsed by the book's JSON reader (`book/src/canon.rs`), which keeps every number as the text the source wrote. Its parser moves to `bagholder-core` as a value tree (`core::json`), and `canon` becomes one use of it. No reply number passes through a float. A number with more significant digits than a `Dec` holds (28) is a mismatch, never rounded.
- **Strict.** An adapter reads its fields through a typed reader over that tree:
  - required accessors (`r.text("exDate")`, `r.dec("amount")`, `r.day("d")`, `r.list("observations")`), for which a field that is absent, null or of another type is a **mismatch naming its path** (`dividends.dividends[3].amount: expected a decimal, found null`);
  - optional accessors (`r.opt_day("recordDate")`), used only for a field real replies show null, where null reads as "the source states none" and an absent key is still a mismatch.
  A mismatch makes the whole reply a failure of the source, never data. A field the adapter does not read is ignored.
- **Meaning.** Each adapter checks what its values mean before anything is written. A value that fails is a failure of the source, named with the value. The checks:
  - a currency is the listing's;
  - a date is a real day, inside the span asked for where one was asked;
  - a rate or a price is positive;
  - the series answered is the one asked for;
  - a quote carries a time (below).
- **Shape change.**
  - The set of paths a real reply carries, with array indices folded (`observations[].d`), is taken as the union over the adapter's recorded fixtures.
  - A reply whose set differs (a path gone, a path new) is recorded as a shape change beside its outcome, even when every field the adapter reads is still there.

### The contract

```text
trait Adapter {
    fn source(&self) -> SourceName;
    fn offers(&self) -> &[Offer];            // kind of data × kind of instrument × market, reach, and how late it is by design
    fn form(&self, i: &InstrumentInfo, known: &Routes) -> Vec<Form>;
    fn read(&self, ask: &Ask, net: &Net, now: Timestamp) -> Outcome<Answer>;
}
```

- **Outcome** is one of:
  - `Answered(Answer)`;
  - `NotCarried`, which is not a failure of the source;
  - `Refused { status, retry_after }`;
  - `Unreachable(why)`;
  - `Mismatch { path, why }`;
  - `Meaning(why)`.
  Every outcome is recorded, with `ShapeChange` beside it when it applies.
- **Chains are data.** One ordered chain per kind of data and kind of instrument, as `SPEC.md` §2's Market data table orders them. The winner is remembered per instrument in the market cache and asked first next time. No ticker is special-cased in code.
- **Forms come from references.**
  - A routing reference the book holds (`TmxForm`, `Yahoo`, and the new `CboeCanada` and `CoinbaseProduct`, all `Routing` strength) is used as is.
  - A form learned by asking (TMX's venue forms, as `tmx_resolve` learns them today) is written back as a routing reference only when the source confirms it: the venue its reply names matches the book's venue for the listing.
- **Health.**
  - Every request's outcome is a row in the market cache.
  - A source's state is a pure function of its own rows: `working`, `refusing`, `failing` or `shape-changed`, over its last ten requests or its last day of them, whichever holds more.
  - `bagholder source-health` prints each source's state and its last outcome of each kind.
  - Showing health on the page is stage 5 (§15).
- **Recorded real replies are the tests.**
  - Each adapter's `tests/replies/<source>/` holds real replies captured from the source, one per shape it answers: an answer, an empty answer, not carried, and a refusal where the source gives one.
  - Beside each is a wrong-shaped and a wrong-meaning copy, edited by hand and named for what is wrong.
  - `bagholder record-reply <source> <ask>` captures one. CI never touches the network.

### The market cache

`market.db` in the data folder, opened by `bagholder-sqlite`'s migration runner (numbered migrations, refusing a newer file, a snapshot before a pending migration). Migration 001:

| Table | Holds |
|---|---|
| `quotes(instrument_id, source, price, currency, change, change_pct, quoted_at, allowance_secs, received_at)` | the latest quote per instrument and source |
| `daily_closes(instrument_id, day, close, currency, source, received_at)` | a listing's or a coin's close per session day, as traded; a closed day is written once; a later different value is kept beside it, and the first stands and is what `Market` reads, the disagreement recorded as a meaning outcome |
| `benchmarks(index, day, level, source, received_at)` | S&P 500, S&P/TSX Composite and S&P/TSX 60 levels, written once per closed day |
| `chains(instrument_id, kind, source, form, won_at)` | the winning source and form per instrument and kind of data |
| `outcomes(id, source, host, kind, instrument_id, outcome, detail, shape_change, at)` | every request's outcome; per source, the newest thousand and the last of each outcome kind are kept, so a busy source never erases a quiet one's health |

It is keyed by the book's instrument ids. Nothing in it is a fact a figure was computed from (§6), and deleting it loses only time.

**What the engine reads.** `Market` is built by one typed read of the cache and the book:
- quotes from `quotes`;
- closes from `daily_closes`, merged with the book's `recorded_closes` for option contracts (their `Money` checked against the contract's currency and read as its amount);
- benchmarks from `benchmarks`.

### Session days

A venue's session days are the days its own daily closes exist, as the sources report them. It needs no calendar of its own:
- **A closed day** is one with a close stored, and is never read again.
- **An option's recorded close** is dated to the underlying's session day that the chain's own time falls in or follows (research 4).
- **Venue hours and holidays** as a calendar (§7) are the bracket engine's need in stage 4, not a figure's.

### The fact readers (into the book, written once)

**Bank of Canada** (`bankofcanada.ca/valet`), verified against the live service on 2026-09-23.

- **The currencies.**
  - The group `FX_RATES_DAILY` lists the currencies the Bank publishes daily (26, from `FXAUDCAD` to `FXZARCAD`).
  - The noon archives ended in April 2017 and never change, so which series is a currency's noon spot rate is a fixed table in the adapter, written from the Bank's and Statistics Canada's own metadata: series code, description, first and last day. A label alone does not name one series (`USD_NOON` is both IEXE0101, the spot rate, and IEXE0105, a 90-day forward; `ARS_NOON` and `MMK_NOON` are each two series in turn). Every read checks the reply's description against the table; a different one is a mismatch.
  - A currency in none of them is `rate-unpublished`.
  - Each series is stored with its first and last observation day. This needs **book migration 3**: `fx_series(currency, source, first_day, last_day, received_at)`, keyed by currency and source, with `schema/v3.sql` committed.
- **Which currencies are read.** Every currency the engine converts: every transaction's cash and fee currency, every instrument's currency, and every currency a broker states a balance in.
- **Observations.**
  - `observations/FX<CUR>CAD/json?start_date=…&end_date=…` answers `observations[] { d, FX<CUR>CAD { v } }`, with `v` a decimal string.
  - The reader checks that the reply names the series asked for, every `d` lies inside the span, every `v` is a positive decimal, and no day repeats.
  - A completed read stores its rates and its span with `received_at` (`store_rates`). **The span stored is clamped to the series' own first and last days**, so a read asked from 2010 records its daily span from 2017-01-03, and no weekday before a series begins ever reads as a day the Bank did not publish (stage 2's rule).
- **Three eras per currency, never overlapping.**
  - The daily series (Valet) is the Bank's rate from its first day.
  - Before it, the Bank's noon series (Valet's legacy noon), read and stored only up to the day before the daily series begins.
  - Before that, Statistics Canada's 10-10-0008 noon spot rate, for the twelve currencies it holds, read and stored only up to the day before the noon series begins.
  - So no two series ever state the same day, whichever read runs first. A day before a currency's oldest series is `rate-not-held`.
- **What is read.** For each currency, from the person's oldest day that needs it to today, then forward from the last span's end. A day's rate is written once, and a different later value is kept beside it and reported (stage 2).
- **Holidays.**
  - The Bank's holiday schedule page (`/press/upcoming-events/bank-of-canada-holiday-schedule/`) lists this year's closures as date and name pairs.
  - They are read into `store_bank_holidays`, so today and the coming days are known to be closures before any read could skip them. Past closures need no page: a completed read that skipped them says so.
  - The reader takes every date and name pair on the page. A pair on a weekend is kept as the page states it: an observed holiday is the weekday the page names.
  - A page with no pairs, or with pairs for another year than it names, is a mismatch, not an empty schedule.
  - The fixtures include a year whose holiday falls on a weekend.

**Distributions and schedules, from the payer itself** (research 2).

- **The fund company's own publication** states a fund's schedule (how often it pays) and its distributions: ex-date, record date, pay date, amount per unit, currency. Which publication, for each of the sixteen companies the person's funds come from, is research 2's table: the fund's page, the JSON or GraphQL the page itself reads, or the company's own distribution release (Global X). One adapter per company reads it strictly: every field it needs, typed and checked; a publication that no longer carries them is a mismatch naming what is missing, never an empty schedule. The schedule is read from the company's own words, a closed list per adapter ("Twice-monthly", "Semi-Monthly", "Monthly Variable", the release table a fund sits in); a word not on the list is a mismatch.
- **A company's own announcement** states each dividend it declares: the amount, the record date, the pay date, and the schedule in its own words ("declared a quarterly dividend of …"). One adapter per publication form (Cision's newswire.ca pages, the InvestorRoom/MediaRoom sites, Q4's feed), each company's sentence read strictly. The ex-date is the one the company states (TD), else the exchange's rule from the record date (the same day from 2024-05-27, the business day before until then).
- **What a publication states, kept as stated.** A distribution's cash and reinvested parts where the payer states them (BMO, Vanguard, iShares Canada, Mackenzie, Global X's reinvested table, Fidelity's release); a row repeated identically is one row; two different rows for one ex-date, or a date outside the fund's life, are a meaning failure naming both, and the read writes nothing.
- **Stored in the book, written once**: the distributions as one read of the payer's record (`store_declared`), so a distribution the payer withdrew is absent from the newest read (a read identical to the newest records only its time); the schedule as a stated frequency (`store_frequency`) under the payer's own source name. A later statement stands as the newest; an old one is never left standing alone.
- **What the figures use.** The per-unit amount is the cash per unit of the latest distribution gone ex that pays cash, since the figures are income paid (`SPEC.md` §2, "Annual income"); a reinvested part pays nothing and is shown as the payer stated it. The frequency is the payer's own statement (§18's change: never assumed). The kinds stage 2 gave the engine (regular, special, non-cash) go: book migration 3 drops `declared_distributions.kind` and adds `reinvested`, the stated reinvested part per unit, beside `amount`, the cash part.
- **Where the fund company's own publication cannot be read at all** (the owner's one exception, 2026-09-24): today Mackenzie (QCN, QUU) and WisdomTree (WQTM), whose sites answer only a browser, and getting past a bot check is not something the app does (research 2). For such a company, and only for it, the fund's distributions come from the listing's exchange-side record: TMX's declared distributions for a Canadian listing, Yahoo's dividend events for a US one, marked with that source; they are the fund's declarations as the exchange publishes them. The schedule is used only where a readable source states it (TMX's `dividendFrequency` where not null); it is never worked out from past dates. Where none states it, the fund's annual income is a gap naming why (its company's site cannot be read). Every figure built on the exchange-side record carries its source in the data; how the page shows it is `SPEC.md`'s to say at the switch (3c). Which companies take this path is a fixed list in the adapters, research-backed and tested; it is not a fallback on failure. A fund company's page that fails to answer is a failure of that source, shown as one, and never silently replaced.
- **A payer no adapter reads** is shown as waiting on its payer, named. It is never filled from a default.
- **Which payers.** The instruments whose rate a screen shows: held positions (`SPEC.md` §2, "Distribution rate for a holding", and the income figures built on it). A payer no longer held is not read. A release notification that carries a distribution's figures (`SPEC.md` §4) reads its payer when the release arrives; that is the data flow's (stage 5), not this part's.

**Option closes** (Cboe's delayed chains, `cdn.cboe.com`).

- **When it is read.** After each session's close, the same day, for every held contract, including those expiring that day, which are gone from the next day's chain.
- **What is written.** The contract's close as research 4 defines it, dated to its session, via `recorded_closes`.
- **Checks.** The chain's `timestamp` is UTC: the reply's `Last-Modified` (03:55:02 GMT) matched it (03:54:59) on 2026-09-23. A chain whose session cannot be told is a meaning failure, and it writes nothing.
- **Exactness.** Cboe writes prices as binary float leftovers (`224.255004882812`). That is its statement, kept exactly as written.
- **What cannot be had, stated now.**
  - A contract's close for a day no reader ran.
  - Any day before the first run. No store holds a past session's close to import (checked 2026-09-24).
  No source gives these later. On those days the equity series takes the broker's stated figure (stage 2's rule) or waits, and says which.
- **Captured every session** by a daily run (decision 1 at the top) until the Rust build runs every day.

**Underlyings on expiry days.** The expiry rule (`engine/src/ledger.rs`) needs an option's underlying's close on the contract's expiry day. That close is read for every contract the book has held, whether or not the underlying was ever held.

### Quotes, daily closes and benchmarks (into the market cache)

The chains of `SPEC.md` §2, as adapters under the contract:

| Instrument | Quotes | Daily closes |
|---|---|---|
| Canadian listings (TSX, TSX-V, CSE) | TMX quote | Yahoo (research 1) |
| Cboe Canada listings | Cboe Canada | Yahoo |
| US listings | Yahoo | Yahoo |
| Coins | Coinbase | Coinbase Exchange candles |
| US options | Cboe chain | the option close (book) |
| Benchmarks | none | FRED for the S&P 500, Yahoo's `^GSPC` for days before FRED's ten years; TMX for `^TSX`, `^TX60` (from 2001-12-11) |

Stooq goes from `SPEC.md` §2: it now answers with a browser check instead of data.

**Every quote has a time.** How each source states it was checked on 2026-09-23:
- **TMX:** the quote's `datetime`, with its offset.
- **Yahoo:** the chart's `regularMarketTime`.
- **Cboe chains:** the chain's `timestamp` (UTC, above).
- **Coinbase, USD pairs:** the Exchange ticker (`api.exchange.coinbase.com/products/<pair>/ticker`) carries `time`. It lists only USD pairs; BTC-CAD, ETH-CAD, SOL-CAD and DOGE-CAD answer 404.
- **Coinbase, a coin's own currency:** the spot price (`api.coinbase.com/v2/prices/<pair>/spot`) states no time, and its origin allows it to be 60 seconds old (`max-age=60`). Such a quote is stamped with the reply's `Date` less that allowance, the allowance stored with it.

A quote with no time is a meaning failure. How late each source is by design (Cboe's chains are delayed fifteen minutes) is part of its `Offer`. A chain never asks a source for a market it is not live for (TMX is not asked to quote a US listing, as SPEC §2 already rules). The quote's time is stored and carried to `Market`, so how old a price is can always be shown.

**Closes as traded.** Daily closes are stored as traded (research 1), for every instrument from the first day it was held and for every underlying on its contracts' expiry days. A closed day is never read again.

### The engine's changes

- **Distribution kinds removed** (`DistributionKind` in the engine and the book): every row of the record counts, as `SPEC.md` §2 defines; the existing cases rewritten to that.
- **The frequency is the payer's statement**, first in `cashflow.rs`; a payer with none waits, named. Cases: a stated schedule used; a payer with no statement shown as waiting.
- **`rate-not-held`** (research 3): a gap for a day no source holds the Bank's rate for, with a case.

### Periodic reads, each listed with its reason

| Read | When | Reason |
|---|---|---|
| Bank of Canada observations | a business day after 16:30 Eastern whose rate is not stored; once when a currency first appears | the Bank publishes at 16:30 (§7) |
| Bank holiday page | the first business day of each month | the page names this year's closures, and a closure must be known before its day passes |
| A payer's page or announcement | when its next distribution is due by its own schedule and not yet read; on an announcement of distributions (3c); when a payer first appears | a payer's record changes only when it declares |
| Option closes | each session day after the close, for held contracts | a session's close is gone the next day |
| Daily closes | each session day after the close, for held instruments and expiring contracts' underlyings | the equity series needs each day's close |
| Benchmarks | each session day after the close | the yearly returns need each day's level |
| Quotes | on demand only, from 3c (a screen showing a price) | §13: a price is read because something shows it |

Each is a pure due-function over the book, the cache and a clock handed in, tested on a fake clock. The server calls them from 3c, where §13's test that counts periodic reads lands. A failure is retried on the source's own rest (`Retry-After`, else the limiter's refusal rest), never in a loop.

### The command, and the comparison run again

- `bagholder read-sources <book folder> [--cache <market.db>] [--now <instant>]` runs every reader that is due, once, writes the facts to the book and the market data to the cache, and prints each source's outcomes.
- `compare-figures --facts-from-book` takes rates, distributions, frequencies, closes and benchmarks from the book and the cache instead of the old store's stand-ins.

## Acceptance criteria

The template's Python, Go, shared-case and page lines do not apply: those builds are frozen, and the page does not change in this part.

- [ ] **Build.** Rust, from `rust/`: the whole workspace's tests green and a warning-free build.
- [ ] **Research.** Each of research 1–5 answered in Verification with the replies that answered it (recorded as fixtures) and the decision taken; no reader built on a question left open.
- [ ] **Boundaries.** The boundary test holds `bagholder-net` and `bagholder-sources` to their columns, each checked by feeding the checker a violation:
  - no `f64` in `bagholder-sources`;
  - no clock read in `bagholder-sources`;
  - `bagholder-net` reads time only through the clock it is handed.
- [ ] **One copy.** `bagholder-market` calls the moved client, limiter, domain logic and HTML table reader, and none of them remains in it: a test fails on their old definitions. The old `note_source` health is gone.
- [ ] **The reply reader**, each a test:
  - a decimal read to its last written digit;
  - a 29-digit number is a mismatch;
  - an absent, null or wrongly typed required field is a mismatch naming its path;
  - an optional field's null reads as stated none, and its absent key is a mismatch;
  - a field not read is ignored;
  - a path gone and a path new are each reported as a shape change, against the union of the fixtures' paths with indices folded.
- [ ] **The limiter.** One per host for every host; a minimum gap, a rest after a refusal, and `Retry-After` honoured, each on a fake clock; Yahoo's and SEDAR+'s old gates removed, their callers on the one limiter.
- [ ] **Health.**
  - Each source state (`working`, `refusing`, `failing`, `shape-changed`) is a test over outcome rows.
  - A thousand quote outcomes do not evict the Bank's last outcome.
  - `source-health` prints each state.
- [ ] **Fixtures.** Every adapter of this part has recorded real replies under `tests/replies/`, with a wrong-shaped and a wrong-meaning copy of each, and a test per reply asserting exactly what is written, or that nothing is and which outcome is recorded.
- [ ] **Bank of Canada**, each a test on recorded replies:
  - the daily currencies stored with each series' first and last day, and the noon table's series checked against the Bank's and Statistics Canada's metadata;
  - a reply whose series description differs from the table (IEXE0105's forward rate in place of IEXE0101) is a mismatch;
  - a span's rates and the span stored, clamped to the series' days (a read asked from 2010 records its daily span from 2017-01-03);
  - a weekday skipped inside a completed span is not a business day to the engine, and a weekday before a series begins is not;
  - each era stops the day before the next begins (Statistics Canada, then noon, then daily), with no conflict whichever read runs first, and a day before a currency's oldest series is `rate-not-held`;
  - a repeated day, another series than asked, a non-decimal value and a day outside the span are each a failure writing nothing;
  - the currencies read are every currency the engine converts;
  - the holiday page's pairs stored, a weekend-dated observed holiday kept as stated, and a page with no pairs or the wrong year a mismatch;
  - the due rule on a fake clock: a business day after 16:30 Eastern with no rate is due, before 16:30 it is not, a holiday is not;
  - book migration 3 applied to a version 2 book with its rows intact, and `schema/v3.sql` committed and compared.
- [ ] **Payers' publications**, for each fund company and announcement form, each a test on its recorded pages:
  - the schedule and every distribution read and stored, with cash and reinvested parts where the payer states them;
  - a schedule word not on the adapter's list, and a page missing a field it must carry, each a mismatch naming it, writing nothing;
  - a repeated identical row stored once; two different rows for one ex-date, or a date outside the fund's life, a meaning failure writing nothing;
  - a company release's amount, record date, pay date and schedule read from its own sentence, and its ex-date by the exchange's rule on each side of 2024-05-27;
  - a read stored as a whole, a later read lacking a row leaving it absent, an identical read recording only its time; a later schedule statement replacing the earlier;
  - Mackenzie's and WisdomTree's funds: distributions from the exchange-side record, marked with its source; the schedule where a source states it, otherwise the fund's annual income a gap naming its unreadable company site; never a schedule worked out from dates.
- [ ] **The engine**, cases passing: distribution kinds removed; the per-unit amount the cash per unit of the latest distribution gone ex that pays cash, a wholly reinvested one passed over and a year-end one with a cash part counting its cash; the payer's stated frequency used, and a payer with none shown as waiting; `rate-not-held` for a day before a currency's oldest series; every existing case still passing.
- [ ] **Option closes**, each a test on recorded replies:
  - a chain read after the close writes each held contract's close once, dated to its session, including a contract expiring that day;
  - a chain whose session cannot be told writes nothing and records a meaning failure.
- [ ] **Quotes, closes and benchmarks**, on recorded replies:
  - each chain in the order of the table, its winner remembered and asked first;
  - `NotCarried` recorded and not counted as a failure;
  - a quote without a time refused;
  - a Coinbase spot quote stamped with its reply's date less its allowance;
  - a closed day written once, and a later different value kept beside it and recorded while the first stands in `Market`;
  - closes stored as traded around a split (research 1);
  - an expiring contract's underlying's close read for its expiry day when the underlying was never held;
  - a form learned from TMX written back as a routing reference only when the reply's venue matches the book's.
- [ ] **The market cache.** Migration 001 applied by the runner, `schema/v1.sql` committed and compared, a newer file refused, every table's typed write and read a test, and `Market` built from the cache and the book's option closes.
- [ ] **Periodic reads.** Each due-function in the table is a test on a fake clock (due, not due, and after a refusal).
- [ ] **A real run.** `bagholder read-sources` run on a copy of the person's book writes rates for every currency the engine converts from its oldest day, a declared record and a frequency statement for each payer, and closes for every held instrument's every day. Every failure it reports is attributed in Verification to a cause checked against the source (not carried, delisted, not published), and **none is a reader bug**.
- [ ] **The comparison.** Run again with `--facts-from-book` on copies of both databases. Every difference from stage 2's run is attributed, a sample of each cause is checked against the records, and the counts are in Verification.
- [ ] **The design review.** `docs/design-review.md` records stage 3 as three parts and 3a as done.

## Surfaces to check beyond the diff

- `rust/Cargo.toml` (two new members) and `Cargo.lock`.
- `bagholder-market` and every caller of the moved client, limiter, domain logic and table reader.
- The boundary test.
- `book/src/canon.rs` and its callers, reading through `core::json`.
- The book's migration 3 and `schema/v3.sql`.
- `book/facts.rs`'s writers, called outside tests for the first time.
- `RefScheme` (two routing schemes) and its text form.
- The book's and the engine's `DistributionKind`, removed with every use, and `cashflow.rs`'s frequency order.
- `engine_inputs.rs` (the series' first and last days).
- The release workflow and the Rust Dockerfile: new crates must not change what is built or attached.

## Right to refuse

- **Taken, on corporate events.** An event's values come from the official source (§6), not from Wealthsimple alone. Which source answers for which event is settled in 3b, beside the rows it explains.
- **Reserved.** If research shows a field or a source this plan counts on is not there, the reader for it is not built on a guess: the plan changes first, and the change is written here.

## Anti-stub self-check

- Every table of the market cache is written by an adapter of this part and read by `Market`'s typed read or by the health function, both exercised by tests.
- Every fact writer in `book/facts.rs` has a production caller in this part: rates, reads, series, holidays, declared, frequencies, option closes.
- The readers are run for real on a copy of the person's data (the command and the comparison), not only on fixtures.

## Verification

(Filled in when the part is built.)

## Handoff

(Filled in when the part is built.)

- For stage 4 (brief 02): the HTTP client moved into `bagholder-net` re-sends a request once on a fresh connection when the reply fails on a reused one (`net/src/client.rs`, the two-attempt loop in `send`). That includes Wealthsimple's order POSTs (`ws/src/session.rs`), so an order can be sent twice. It moved as it was; stage 4 limits the re-send to requests that are safe to repeat.
