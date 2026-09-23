# Bagholder: the design

This is the design of Bagholder, worked out from what the app is for and where it is going. Every part of the code is judged against it; "the old app did it this way" is never a reason for anything here. `SPEC.md` stays the authority on what each screen shows and how each figure is defined; this document is the authority on how the app is built to deliver that. Where the two disagree, one of them is changed on purpose, and the change says why (§17 lists the changes this design makes to `SPEC.md`).

The previous version of this document was a stage-by-stage plan for porting the Python app to Rust and Svelte. It measured progress by how closely the new build reproduced the old one, and so it carried the old design across with it (that plan is in git at commit 955f7a3). This version replaces it. It was reviewed, before any code was judged against it, by a reviewer who had written none of it; the review's findings are folded in.

## 1. What Bagholder is for

Bagholder is a trader's own record of their money, kept on their own machine, with the market around it and the means to act on it.

- **The record.** Every trade, dividend, interest charge, deposit and corporate event, from every account the person has, kept exactly as the source reported it and never lost.
- **The figures.** What each trade made or lost, what is held and what it is worth, the income it pays, how the whole portfolio has done against the market. Defined exactly in `SPEC.md`. **The figures are the product**: a wrong number is a failed app, however well everything else works.
- **The context.** Prices, charts, news, regulatory filings, short interest, what a fund holds, the market's mood, for what is held and what is watched.
- **The action.** Placing, changing and cancelling real orders, and guarding positions with stop-losses and targets the broker does not offer.
- **The attention.** Telling the person what happened while they were not looking: a fill, a rejection, a filing, a release.

Who uses it: one person, running it themselves, who cares about exactness more than decoration. It runs on whatever machine they have, down to a small single-board computer, and keeps their data to themselves.

What it must never do: show a number it cannot stand behind; lose or silently rewrite what a source reported or what the person wrote; place, change or cancel an order nobody asked for, or believe an order did something the broker did not confirm; hide a failure.

## 2. Where it is going

These are directions the design serves now, not ones bolted on later:

- **More brokerages**, built soon. A person may hold accounts at several at once.
- **Self-hosted, always.** The local, self-hosted app is the product and stays one even if a hosted version appears.
- **Possibly hosted, for many people.** Nothing is built for it ahead of need, but nothing at the centre may assume there is only one person.
- **Other readers**: AI agents through MCP, scripts, the command line.
- **More data and events**: company fundamentals; corporate events booked from the official record (`docs/plans/corporate-events.md`); more regulators and news sources.

## 3. What that demands

1. **Exactness.** Figures are exact to the definition, traceable to the records they came from, never filled in with a guess, and they do not change after the fact without saying so. What is not known is shown as not known, and the person can supply it.
2. **Many unreliable sources.** Brokers' private interfaces, free market-data sites, regulators and news feeds change without notice, rate-limit, go down and disagree, and there will be more of them.
3. **Money moves.** Placing orders and guarding positions is safety-critical; the broker, not the app, is the truth about an order; and an automated action must be bounded.
4. **Many readers, one truth.** The page, agents and scripts see the same figures, computed one way, with access limited to what each is allowed to do.
5. **Live and frugal.** What is on screen is current within seconds of the world changing, and the app does only the work a change requires.
6. **Private, durable, self-contained.** One person's data on their machine, backed up without them having to think about it; one program that keeps running and updates safely.

## 4. The shape

```
     brokers (Wealthsimple, next ones)     files (CSV)     the person (entries, adjustments, journal)
                     │                          │                    │
                     ▼                          ▼                    ▼
        ┌─────────────────────────────────── THE BOOK ───────────────────────────────────┐
        │ source records as received (provenance, revisions) → Bagholder's transactions  │
        │ instruments, issuers, accounts · facts the figures used (rates, event values)   │
        │ journal · watchlist · orders, brackets and their event logs · settings          │
        └──────────────────────────────────────────────────────────────────────────────┘
                     │                                                  ▲
 market sources      ▼                                                  │ confirmed fills
 (quotes, bars,  ┌────────────┐     ┌──────────────┐              ┌────────────┐
  news, filings, │ THE ENGINE │────►│ CHANGE FEED  │──┐           │ EXECUTION  │◄── the person's orders
  shorts, …) ──► │ (figures)  │     │ (what moved) │  │           │            │──► brokers
      │          └────────────┘     └──────────────┘  ▼           └────────────┘
      └──► MARKET CACHE (all of it can be fetched again)   one typed interface, with access scopes
                                                           page · agents · CLI · notifications
```

- **The book**: what the person's money did, what they wrote, and every fact a figure was computed from. Precious; backed up automatically.
- **The market cache**: what the world's sources said that can be asked again. Losing it loses only time.
- **Sources**: one adapter per outside source, turning what it sends into records or market data, and saying when it no longer can.
- **The engine**: computes every figure in `SPEC.md` from the book and the cache. One implementation.
- **Execution**: turns the person's intent into broker orders and guards positions, with the broker as the authority and hard limits on what it may do on its own.
- **The interface**: one typed, live interface every reader uses, and the notifications that reach the person when they are not looking.

## 5. Identity: what things are called

Everything rests on naming things in Bagholder's own terms, not in any source's.

- **Instrument.** Every instrument has a Bagholder id that never changes. What it is called is an attribute with dates: a listing's symbol and venue (a ticker change, a move between venues), an option contract's underlying, expiry, strike, right and multiplier as the contract states them, a coin's asset. Each source's identifiers for it (a broker's security id, ISIN, CUSIP, FIGI, the OCC option symbol, a Yahoo symbol, a TMX form, a SEC CIK, a SEDAR+ profile) are recorded against it as references. Routing a request to the right source form is a lookup in those references, learned and remembered, never code that knows tickers.
- **Issuer.** A company or fund, above its listings. Filings, releases, news and corporate events belong to the issuer; a company listed in Canada and the US is one issuer with two listings, each with its own price, position and short interest.
- **Matching across sources.** Two sources' instruments are the same instrument only on a strong identifier (ISIN, CUSIP, FIGI, the OCC symbol, or the same broker's own id). Anything weaker stays a separate instrument until the person links them. Nothing is ever merged on a bare symbol.
- **Account.** Belongs to a broker connection; has a type in Bagholder's vocabulary (cash, margin, registered plan and which), a status, and balances per currency (a Wealthsimple account holds CAD and USD at once). The broker's own account id is a reference.
- **Broker connection.** One login at one broker: its session, its accounts, what it can do (§10). A person may have several.
- **Trade.** A round trip's id is assigned by Bagholder when its opening transaction is first seen and stored; it survives re-deriving transactions, linking a booked fill to the broker's row, a back-dated row arriving, and a corporate event. The journal attaches to that id. A note whose trade no longer exists after a correction is shown as orphaned for the person to re-attach, never dropped.
- **Person.** The self-hosted app has one. Everything the person owns lives in their book; a hosted version is many books opened by id, not a rewrite.

*Why:* the old app used Wealthsimple's ids and bare symbols as identities and derived trade ids from dates, quantities and prices. So a share could take a coin's price, a corporate event had nowhere to live, a second broker would touch every file, and a split inferred differently renamed every earlier trade and cut its notes loose.

## 6. The book

**Two stores, split by whether a thing can be fetched again.**
- **The book** holds everything that cannot: source records, transactions, instruments and their references, issuers, accounts, the journal, watchlist, groups and tiles, orders, brackets and their event logs, notification history, settings, and **every fact a figure was computed from**: the FX rate each conversion used, the declared distribution records a rate was taken from, the values and ratios of corporate events. A figure booked once therefore stays what it was, whatever a source later revises or drops.
- **The market cache** holds what can be asked again: quotes, charts and bar archives, news, filings and their read text, short interest, exposures, universes, gauges. Deleting it loses only time.

Both are SQLite in WAL mode, every multi-row change in one transaction. Broker credentials are in neither (§12).

**Source records are kept as received**, with their source, the source's id, when they arrived and their revisions (a broker replacing its own row keeps the history). A row is marked removed only when its source reports the removal, never because it is missing from an incremental pull.

**Transactions are Bagholder's own**, derived from source records by each source's mapping, deterministically, the mapping's version recorded: buy, sell, dividend, interest, fee, deposit, withdrawal, transfer, corporate event, in one vocabulary for every broker. When a mapping improves, transactions are derived again from the kept records; when that moves a past figure, the app says so ("figures corrected by the Wealthsimple mapping, version 7") rather than changing history silently.

**Reconciliation is by execution and is explicit.** When an order fills, each execution is booked at once as a provisional record. When the broker's own rows arrive they are linked to it by the broker's order id or the app's own id (many to many: a partly filled order, several rows for one fill), and the provisional record gives way. Fuzzy matching is only for rows with no ids (a CSV), and a match with more than one candidate is never linked automatically.

**Corporate events are records** from the official source where there is one (the issuer's filing, the exchange's notice, the contract's terms), with the values they need and where each came from. An event whose value is not known is booked as unknown, and the figures it touches say so. Nothing is inferred from the person's own fill prices.

**The person can supply what no source has**: an adjustment record (a cost basis for shares transferred in, a spin-off's allocation, a missing split ratio), recorded as "entered by you", and replaceable by a sourced value later.

**Money and quantities are exact.** Amounts and quantities are decimals, never binary floating point; every amount carries its currency and the types do not allow adding two currencies; conversion takes the day's rate for that currency pair and gives "unknown" when there is none. Ratios and returns, which are statistics, are floating point.

**The data is protected without the person thinking about it.** The book is backed up automatically on a schedule, with SQLite's own backup mechanism (never a copy of a live file), kept for a set period, and restoring one is tested. Before every schema migration the book is snapshotted, migrations are tested against stored copies of every past schema, and an update that is rolled back restores that snapshot.

*Why:* the old book was a table of Wealthsimple rows with derivations mixed in; floating-point money was summed with error compensation so five implementations would agree on the last digit; a missing rate was the constant 1.35 and every non-USD currency was treated as CAD; bad rows were dropped or read as zero.

## 7. Time

- Instants are stored and sent in UTC. They become days and times only where a rule names a zone.
- **The person's home zone** is a setting, an input to the engine. "Today", a year's boundary and a month's are the person's, never the server's (a container runs in UTC).
- **The day a broker files a row under** is part of that broker's mapping (Wealthsimple's is Alberta's).
- **Each venue has its session hours and holidays**, used for which quotes can have moved, the bracket engine's session rules and a resting order's renewal.
- **FX is by the Bank of Canada's business day**; the engine is told the rate, it never guesses one.
- Display is in the viewer's zone. Zone rules come from the system's database with a built-in copy where there is none, and tests pin the rules (`docs/plans/time-zone-rules.md`).

## 8. The engine

The engine computes every figure in `SPEC.md`: round trips, positions, cash flows, distributions and their rates, **the equity series and returns**, drawdown, aggregates in CAD, the markets views, and per-month figures (the page does no money arithmetic at all).

- **A pure function of the book and the cache.** No network, clock or disk inside it: today's date, the home zone and the zone rules are inputs. The same inputs always give the same figures.
- **Equity is Bagholder's own**, computed from the record, prices and FX, so it covers every broker and every account however they report. Each broker's own net value is a check against it, and a disagreement is shown.
- **How it stays fast.** A change to the record re-derives the book's figures in full (milliseconds for a real book). One layer marks positions to prices, and a price updates only the positions of that instrument and the totals that include them. What changed is found by comparing the new figures with the previous ones by entity id and field. A test holds the incremental result equal to a full rebuild for every kind of change.
- **Held to `SPEC.md` by cases written from the spec.** Each case gives inputs and the figures the spec requires, worked out from the definitions and reviewed by a person, never generated from an implementation's output. There are cases for every definition, including corporate events, unknown rates and values, several currencies, several brokers and adjustments.
- **One implementation**, in Rust, with a typed entry point (records in, figures and changes out) and no dependency on the server.

## 9. Sources

Every outside source is an adapter behind one contract:

- **What it offers**: which kinds of data for which instruments or markets, with what reach and freshness.
- **How it is asked**: its identifiers, resolved through §5's references; its pace, enforced by one limiter per host; its credentials if any.
- **What it answers, checked.** An adapter checks what it reads: the fields it needs are present and of the right type and range; what it does not read is ignored. On top of shape, it checks meaning: the currency matches the listing's, a timestamp is as fresh as the source claims (a quote fifteen minutes old is not a live price), a price is within bounds of the last one. A reply that fails is a failure of that source, never data. A change in the reply's overall shape (a field that disappeared or appeared) is reported separately so it is noticed before it matters. Real recorded replies are kept as the adapter's test fixtures, with a wrong-shaped and a wrong-meaning reply beside them.
- **Its health.** Every request's outcome is recorded per source, and a source that fails, refuses or changes shape is visible where it matters (§14) while the rest carries on.
- **Choice between sources is data**: an ordered chain per kind of instrument, the winner remembered. No ticker is special-cased in code.

**Finding out what no single source says.** Where a figure needs a fact and the usual source does not carry it, the app goes looking in every source that can state it, rather than assuming or giving up. Payout frequency is the model case. A frequency the issuer *states* comes first: the fund's own page, its distribution announcements ("announces monthly distribution"), and its filed documents. A frequency *inferred* from the declared record's ex-dates, or from the payments received, comes after that. A new holding is looked up the moment it appears, so the person never sees a guess. A fund no source answers for yet is a problem the app keeps working on and shows as one; it is never a steady state and never replaced by a default. Which issuer sources state it is established from their real pages before it is built.

A new source is a new adapter and its fixtures; nothing else changes.

## 10. Brokers

A broker is a source with more to it. Each broker adapter provides:

- **Connection.** Signing in from the person's own device (the page, wherever it is open), keeping the session fresh, knowing when it has lapsed. The server never needs a screen of its own.
- **The pull.** Accounts, balances, positions as the broker states them, its net value history, activity, turned into source records and reconciled.
- **Capabilities**, declared: order types per instrument, whether it holds stop orders, whether it modifies in place and what, time-in-force rules, how long it keeps a resting order, **whether it rejects a duplicate client order id**, session rules.
- **Execution**: place, modify, cancel, read one order back, list open orders; every request and reply recorded.

Wealthsimple is the first broker adapter; the CSV importer is a broker adapter without execution. A second brokerage is a new adapter; the book, the engine, execution and the page do not change for it. Differences between brokers are capabilities, not conditions scattered through the app.

## 11. Execution

The part that moves money is built the way order systems are built.

- **Orders are state machines** with their states written down (draft, sending, sent-unconfirmed, pending, partly filled, filled, cancelling, cancelled, expired, rejected, failed) and the only allowed moves. A move happens only on the broker's confirmation, or on a definite failure before anything left the app.
- **Brackets are state machines too**: waiting for the entry, armed, stop resting, target reached and swapping, target resting with the stop watched here, renewing, closing, ended, with their allowed moves. The dangerous moments `SPEC.md` describes (the stop-to-target swap, the seconds with no stop at the broker, renewal at ninety days, closing) are states, not code paths.
- **Every order and bracket has an event log**: each request, each reply, each state change, and **who asked**: the person on the page, an agent, or the engine. The state is what the log says; the log is what any question about an order is answered from.
- **The broker is the authority.** An answer the app cannot read leaves the order "sent, not confirmed" with the problem shown, and the next read-back settles it. Before anything is sent again, the order is read back by its own id; a broker that rejects duplicate ids makes that a second guarantee, not the only one.
- **Brackets run on the broker's capabilities**, broker-neutral: a stop the broker holds is placed with the broker; one it does not is watched here. The rules in `SPEC.md` §Brackets are invariants of the bracket's states, each with a test.
- **Hard limits on the engine.** One switch stops all automated order activity at once. The engine's orders per bracket per minute are capped, and tripping the cap stops that bracket and tells the person. An order larger than a set share of net value is refused. A trigger fires only on the broker's own quote, never on one older than a set age, and never on a single tick far from the ones around it.
- **Tested against a fake broker that misbehaves**: a timeout after a send, a duplicate fill, a cancel confirmed after a fill, replies out of order, a session lapsing mid-swap. Every state and every move has a test.
- **Dry mode** stops every write at the one place requests leave the app.
- **The person can see the engine is watching**: each bracket shows whether the app is running and guarding it, and what protects the position when it is not.

## 12. The interface and access

**One typed interface** for every reader: the page, agents through MCP, the command line. Requests and answers are declared types, and the page's types are generated from the server's.

**Live changes.** The server tells, the reader does not ask. Each reader subscribes to what it shows (a view and a filter set); the server keeps that subscription and sends the changes to it: which entities moved and which fields, keyed by id, each change numbered. A reader that reconnects resumes from its last number, and after a gap is sent a fresh snapshot. The page holds each entity as one object and writes changed fields into it, so one price moving changes that holding's figures and the totals that include them and nothing else on screen; a test watches the page across each kind of change.

**Access.**
- The server answers only its own machine unless the person turns on remote access. Every request is checked for its Host and Origin, and every write carries a token that proves it came from the app's own page, so another website open in the same browser cannot reach it.
- Remote access (from another device) goes through a secure tunnel or reverse proxy the person already trusts; the app does not manage certificates itself. A device is paired, receives its own token, and can be revoked.
- Tokens have scopes. Reading is one scope; placing, changing and cancelling orders is another, never given by default. An agent reads the book; it places an order only if the person granted that scope to it, and the order's log says it did.
- **Broker credentials** live in the operating system's keychain where there is one, otherwise in a file only the person's account can read, outside the book, so a backup or a moved book never carries them.

**Notifications** are events from the book, execution and sources, each identified by what it is (a fill, a filing's content, a release's words), so one event is told once, through the operating system's channel or the browser's.

## 13. Work follows demand

Sources are read because something needs them: a screen showing what they feed, a notification someone turned on, an agent's request (which reads fresh data when what is stored is stale, within a bounded wait), or a known moment (a token's expiry, the Bank of Canada's daily rate, a pull time). **Execution is always demand**: while an order is open or a bracket is live, the broker is read on the cadence the order needs, whether or not anyone is looking. Every periodic read that remains is listed with its reason, and a test counts them.

## 14. Failures are seen

Every failure has an owner and a place it shows:

- a source failing or changing shape: on the card or chart it feeds, and in the header's status while it persists;
- a broker session lapsing, a pull or part of a pull failing: the header, naming what failed;
- an order or bracket problem: the order, the Orders panel, a notification;
- a figure touched by an unknown (a missing rate, an unknown basis or event value): the figure says so;
- the app itself (a book it cannot open, a migration that failed): the page says so instead of loading.

Nothing is logged only to a terminal, and nothing is caught and discarded.

## 15. Running it

- **One program, run as a service.** A single binary serves the page and runs the engine and background work. It installs itself as the operating system's service (launchd, systemd, a Windows service), so it starts at boot and after a crash; while it runs, brackets are guarded.
- **Updates are safe.** Releases are signed and the binary checks the signature with a key built into it. An update waits until no order or bracket action is in progress, snapshots the book, swaps the binary, and returns to the previous version and snapshot if the new one does not come up. The container never updates itself; it says a new image is available.
- **One data folder** holds the book, the market cache, backups and settings.
- **Resource use is bounded**: work follows demand, each host is paced, every cache has a limit.

## 16. How the design is held

- **Tests come from `SPEC.md` and from real source replies**, never from what an older build did.
- **The boundaries are enforced by the build**: the engine is a crate that cannot depend on the network, the store or the clock; only adapters depend on a source's reply types; only execution depends on a broker's order interface; money types have no floating-point conversion outside statistics; a periodic wait not listed in §13 fails a test.
- **Every change is placed in this design before it is built**; a change this design does not cover changes this design first.
- **The design and each stage of work are reviewed by someone who did not build the code**, against §1 to §3.

## 17. Changes to `SPEC.md`

These passages of `SPEC.md` describe an old implementation or contradict this design, and change with it. The ones marked **for the owner** change what the person sees or what the app does with their money or notes, so they are decided by the owner; the rest follow from this design.

- Payout frequency (§1, §2 Distribution rate): never assumed. The spec's "12 is assumed" goes; the app finds the frequency out (§9, "Finding out what no single source says").
- **For the owner — Clear data** (§4, the menu): it deletes the journal, which nothing can fetch again. This design has Clear data remove only what can be fetched or derived again, makes deleting the journal a separate action, and takes a backup first.
- **For the owner — a watched stop's order** (§Brackets): it fires as a market sell, which can fill far away on a thin listing; a limit order a set distance through the bid fills almost always and never at any price. Which one a trader wants is the owner's call.
- **For the owner — an unavailable source** (§4 Disclosures, "Source off"): the spec shows nothing; §14 names it on the card.
- Trade identity (§2 Trade): Bagholder-assigned, surviving corrections (§5); lots matched by instrument, not symbol and currency.
- Equity series (§2 Equity): Bagholder's own, the broker's net value a check (§8).
- Splits (§2 Trade) and option multipliers (§4 Orders, the fill booking): from the record and the contract, never inferred or fixed at 100 (§6, `docs/plans/corporate-events.md`).
- Freshness (§2), the store (§6), one data folder per build (§1), the update check's mechanics (§2 Versions), the notifier's Mac applet (§2 Notifications), connecting through a Chrome window on the server (§4): implementation, replaced by §6, §10, §12, §13 and §15, and moved out of the spec.
- The page dividing by twelve (the spec's introduction): the engine gives per-month figures.
