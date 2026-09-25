# Plan: stage 3b, Wealthsimple as the first broker adapter, and the facts the owner enters

## For the owner to decide

1. Settled (owner, 2026-09-24): the adapter's tests use the replies the owner already captured, anonymised before commit; the owner is asked for more only if a rule cannot be tested properly without it.
Nothing else open. Sequencing, not a decision for the owner: the two entry forms (the cost of shares moved in, a spin-off's or return of capital's split of cost) are defined, stored and computed here, and appear on the page in 3c, when the page starts showing the new build's figures; before that an entry could not be seen changing anything.

Settled (owner, 2026-09-24, `docs/decisions.md`): the facts no feed carries are entered by the owner, as Sharesight does, through existing patterns (an opening balance in "Add trade"; an event's allocation on the trade detail beside the journal), each defined in `SPEC.md` first; no per-issuer corporate-event readers; the equity series is Wealthsimple's stated daily account value.

## Scope

Stage 3b of the order of work in `docs/design-review.md`: Wealthsimple as the first adapter behind the broker interface of `docs/architecture.md` §10, reading what only Wealthsimple has (accounts, activity, positions and balances, each account's daily value), and the facts no feed carries, entered by the owner. What it delivers:

- **The broker interface** (connection and pull) and **the Wealthsimple adapter**: its session, strict readers checked against recorded real replies, every request through the one limiter, its health recorded like every source's.
- **Wealthsimple's rows as source records** in the book, each superseding the imported record with the same Wealthsimple id, derived by a versioned mapping that reads what each row states: option fills' opening or closing, multi-leg orders' legs, contract sizes, assignments and expiries, transfers between the person's own accounts (linked by the broker's ids), and corporate events whose units Wealthsimple's rows state (a consolidation, a holding continuing under a new security id).
- **Account links** (an account and the account it is linked to), **the broker's statements** (each account's value and net deposits per day, its cash and units now and when it stated them), and **when each account's activity was last read in full**: everything `BrokerAccount` holds, read from the book.
- **The facts the owner enters**: the cost of a holding that arrived without one, and a spin-off's or return of capital's allocation from the issuer's published figure; defined in `SPEC.md`, stored as records whose source is the person, replaced by a sourced value where one arrives, computed by the engine.

Like 3a, 3b is built beside the running app and changes nothing on screen. The adapter runs from a command (`bagholder pull-broker`) against a book; the proof is the stage 2 comparison run again on the book the pull wrote.

Out of it, on purpose:
- **Execution and capabilities** (placing, changing, cancelling orders; order types, time in force, stop handling): stage 4. The interface gains them there; a capability nothing reads is not declared now.
- **How the person signs in.** The session (tokens, refresh, a lapse noticed and shown) is the adapter's here; the sign-in itself, and keeping credentials in the operating system's keychain, are the access layer's (§12, stage 5). The command signs in through the app's existing sign-in.
- **When the pull runs**, and the page: 3c (the switch), with the forms of decision 2 if taken.
- **Per-issuer event readers**: dropped (owner, 2026-09-24).

## The old app here

The old app's code is not consulted (`CLAUDE.md`). The current Rust client (`rust/crates/ws`) is the code being replaced, and what is wrong with it is recorded:

- It reads replies leniently: an absent or unreadable quantity reads as 0, text as `""` (`ws/src/wire.rs:5-7`, `model/src/lenient.rs`). `docs/old-app-mistakes.md`, "Bad rows dropped or read as zero". Replaced by the strict reader of 3a (`sources/src/reply.rs`).
- It decides a multi-leg row's direction from its cash sign and fixes every contract at 100 units (`ws/src/mapping.rs:576, 612-626`). `docs/old-app-mistakes.md`, "Broker rows rewritten…", "A contract count worked out from cash". Replaced by what the row and the contract state.
- It rewrites the old store's rows in place by `canonical_id` (`store/src/merge.rs:164`). Replaced by records with revisions (§6).
- Its incremental pull starts fourteen days before the newest stored row, an overlap nobody measured, and ignores whether a pull reached the start (`ws/src/sync.rs:202-235`, `server/src/session.rs:375`). New entry in `docs/old-app-mistakes.md`: "An incremental pull from a guessed overlap, and no record of a full read". Replaced by question 5 below.
- Its fixtures are invented (`ws/tests/golden/*.json`, ids like `ws-cid-0001`). Replaced by recorded real replies.
- It is woven through the server (`server/src/session.rs`, `orders/*`) and depends on the old store and model. The adapter depends on neither.

Carried over, each with why it is right:
- **The refresh discipline** (`ws/src/session.rs:501-555`): one refresh at a time; the saved session re-read before posting, and a token another caller already rotated adopted instead of posting the old one; a refresh token Wealthsimple refused never posted again. Right because a rotated refresh token is single-use (OAuth 2.0 Security Best Current Practice, RFC 9700 §4.14, refresh token rotation, read 2026-09-24: https://www.rfc-editor.org/rfc/rfc9700.html), so posting a spent one fails and a second refresh racing the first signs the session out.
- **The session file** (`session.json` in the data folder, written atomically, mode 0600): right until stage 5 moves credentials to the keychain (§12); a new location now would be moved again.
- **The GraphQL operations' texts** (`ws/graphql/*.graphql`, recovered from Wealthsimple's public web bundle): kept as the queries asked, since they are Wealthsimple's own documents, each re-checked against the bundle (done 2026-09-24: one no longer exists, question 8). The new ones this plan asks (`FetchSoOrdersMultilegOrder`, `FetchCorporateActionChildActivities`, `FetchIdentityPositions`, `FetchHoldingsExportPositionsAsOfDate`) are added the same way, each file naming the bundle release it was taken from.

## How the leading products do it

All read 2026-09-24 from each product's own pages. TraderSync's and Edgewonk's help centres refused the fetch (HTTP 403); a fact taken from a search engine's summary of a page, not the page, says so.

- **A holding that arrived without a buy is the person's opening balance: quantity and total cost.** Sharesight's Opening Balance records "the starting quantity and cost of a holding you already owned" (https://help.sharesight.com/ca/adding-buy-and-sell-trades-or-adjustments-manually/); its import takes a quantity and a cost base, "the total cost of the shares owned on the opening balance date" (https://help.sharesight.com/upload-import-opening-balances/); a transfer between portfolios is an opening balance dated the transfer (https://help.sharesight.com/how-to-record-share-transfer-between-portfolios/). A holding with more sold than bought is left out of performance "until it is corrected" (https://help.sharesight.com/negative-balance/), as `basis-unknown` leaves it out here. The trading journals document no opening balance: TradeZella's manual trade is entered execution by execution (https://help.tradezella.com/en/articles/5829532-how-to-add-a-trade-manually-in-tradezella); Tradervue's answer is to import "from a point when you were flat" (https://help.tradervue.com/article/3479-open-trades-and-p-l-discrepancy).
  *Departure:* Sharesight has one date per opening balance; here each lot carries the day it was acquired, because a trade's open date, its hold and first-in-first-out order (`SPEC.md` §2) all read it. Whether a broker-supplied cost later replaces an entered one is not documented by Sharesight; here it does, as every sourced fact replaces the person's (`docs/architecture.md` §6).
- **Return of capital reduces the cost, per unit held, entered by the person outside the markets a tracker covers.** Sharesight's Return of Capital "reduces the cost base of your holding", automatic on ASX and NZX, "All other markets: You need to record this manually" (https://help.sharesight.com/ca/how-sharesight-automatically-handles-corporate-actions/); its worked example is the per-share amount times the units held (https://help.sharesight.com/how-to-handle-suncorp-distribution/). The CRA: a return of capital "will reduce the adjusted cost base" (https://www.canada.ca/en/revenue-agency/services/tax/individuals/topics/about-your-tax-return/tax-return/completing-a-tax-return/personal-income/line-12700-capital-gains/completing-schedule-3/tax-treatment-mutual-funds.html); a cost below zero "is deemed to be a capital gain in the year" and the cost "is deemed to be zero" (https://www.canada.ca/en/revenue-agency/services/tax/individuals/topics/about-your-tax-return/tax-return/completing-a-tax-return/personal-income/line-12700-capital-gains/special-rules-other-transactions.html; Income Tax Act s.40(3), https://laws-lois.justice.gc.ca/eng/acts/I-3.3/section-40.html). Taken: the excess over the remaining cost is realized P&L on the ex-date and the cost is zero.
- **A spin-off's cost is split by the issuer's published share, entered by the person.** Sharesight records TC Energy / South Bow by hand with the issuer's "91% to TC Energy … and 9% to South Bow" (https://help.sharesight.com/how-to-handle-the-tc-energy-trp-tse-spinoff-of-south-bow-sobo-tse/); AdjustedCostBase.ca the same split, as a return of capital on the parent and a buy of the child (https://www.adjustedcostbase.ca/blog/tax-treatment-of-the-tc-energy-south-bow-spin-off/). The law allocates by relative fair market value (Income Tax Act s.86(1)(b), https://laws-lois.justice.gc.ca/eng/acts/I-3.3/section-86.html; for a foreign spin-off under s.86.1, the CRA's A × B ÷ C, https://www.canada.ca/en/revenue-agency/services/tax/businesses/topics/information-canadian-shareholders.html), which is what the issuer publishes. Taken: the person enters the share of cost each child takes; the units are the broker's.
- **Consolidations are automatic; a rename carries the cost base across.** Sharesight applies splits and consolidations on all supported markets, and on other markets a name or code change is a merge of "Full quantity on hand" whose "adjusted cost base … will be transferred across to the new holding" (same corporate-actions page; https://help.sharesight.com/mergers/). TraderSync, by a search summary of https://tradersync.com/support/handling-stock-splits-and-reverse-splits/, says brokers "do not always report these events accurately" and has the user edit quantities by hand. Taken: both come from the broker's own rows here, which state the units; the cost and the acquisition days carry across.
- **The legs of a spread are one trade.** TraderSync groups an iron condor's four legs as one trade and keeps them grouped when one leg closes later (search summary of https://tradersync.com/support/how-does-tradersync-group-trades/). `SPEC.md` §2 already makes a multi-leg fill one trade; this plan only reads the legs as stated.
- **No journal checks its figures against the broker's.** Sharesight's comparison with the broker's statement is by hand (search summary; its page answered 404); TradeZella, Tradervue and TraderSync document none. Bagholder's broker check (`docs/plans/stage-2-engine.md`) goes beyond the practice found, on purpose: a figure it cannot stand behind is shown as such (`docs/architecture.md` §1).

## Open questions

None open. Each was settled on 2026-09-24, first from Wealthsimple's own web app (its public code, read without signing in), then by read-only lookups the owner ran in their own browser on their own history. Why this way: the web app shows what can be asked without touching the account, and the owner's history holds each kind of row the figures wait on, so each rule was checked on a real row of its kind. The rules, for any instrument and any account:

1. **Multi-leg orders.** Objective: each leg as stated, never inferred from cash. Settled: the activity row states the order but not its legs; the order it names (`externalCanonicalId` as `orderBatchId`, `FetchSoOrdersMultilegOrder`) states each leg's contract, buy or sell, open or close, filled quantity, price and cash. An order that expired or was cancelled also appears as a row, with its legs' fills null. **The row's sign is not the cash:** on every filled order checked, the row's amount equals the legs' net cash with the opposite sign. So a multi-leg order's cash is the legs' (a buy pays, a sell receives), the row's amount is checked to equal it, and a mismatch is a failure of the reply, never booked.
2. **Contract terms.** Objective: contract size and the OCC symbol as stated. Settled: an option's security record states its multiplier and OCC symbol with its underlying, expiry, strike and right.
3. **Corporate events.** Objective: an event's units from the broker's statement of what moved. Settled: the event row names the old security only; the event's entitlements (`FetchCorporateActionChildActivities`) state the units given up and the units received (and cash, where there is any), with no ratio and no security id. The received units' security is the one the account's positions name on the day after the event holding exactly those units; an event whose received units no position names that way stays `event-unknown`. Never matched by symbol.
4. **Positions.** Objective: the broker check against the broker's own statement. Settled: positions now carry no time (their `as_of` is when they were asked); positions as of a given day (`FetchHoldingsExportPositionsAsOfDate`) state quantity, direction, book value and close per security. Units are checked on the last day whose activity is read in full; cash against the balances now.
5. **A full read of an account's activity.** Objective: `activity_read_at` means every row posted by then is on the record. Settled: the feed filters by account, dates, types and status, never by when a row changed. The first pull reads each account's feed and daily history in full, once, to replace the imported records. After that a pull reads only what changed: activity dated after the account's last complete read, and the rows not yet final (pending, a placeholder dividend); a row the read no longer lists over the span it covered (a card purchase's pending row, posted again under another id once final; a row withdrawn) leaves the book, and so does an imported record of a row the first full read does not list; daily values after the last stored day; nothing for a closed account with no new activity. A row's legs, entitlements and positions are read once, with the row. A row missed or revised is found by the broker check, not by re-reading: an account whose cash or units differ from Wealthsimple's statement in a way the book cannot explain is re-read over the span the difference points to, and only then (brief 07 §1). No overlap in days is guessed.
6. **Moves between the person's own accounts.** Objective: a holding moved keeps its round trip, joined by the broker's ids, never by amounts. Settled: a move is two rows (source and destination) naming each other's account, stating a CAD amount only, not what moved. What moved is the change in each account's positions across the move (as of the day before and the day after), joined through the two rows' stated accounts and instant. A "full in kind" move of an account sold to cash first states no amount on its row or in its detail (`FetchInternalTransfer`'s `amount` is empty), and its cash moves days after the row: its amount and day are those of the first day on which one account's net deposits (`FetchAccountHistoricalFinancials`) fall by exactly what the other's rise. A withdrawal from a registered account is three rows: the gross amount out, the tax withheld (its own row, sharing the gross row's id) and the net amount received; the gross row books what reached the other account, the tax row the tax.
7. **A transfer in from another institution.** Objective: what arrived, and its cost where Wealthsimple states it, otherwise the person's entry. Settled: the row states the value asked for, not what arrived: the cash arrives days later, less the other institution's fee or more by interest, and the transfer's detail (`FetchInstitutionalTransfer`) leaves the value that arrived (`actualValueLegCad`, `actualValueLegUsd`) empty on every completed transfer of the owner's history. What arrived is what the account's positions show rising from the day before the row to the day the detail's `completed` event falls on, net of the book's own moves over those days: its cash, and for a transfer in kind or mixed (`transferType`), its holdings; booked on that day. Where the detail states the value that arrived, that is taken. Another move of the account read against positions over the same days leaves which moved what unstated, named. A position states Wealthsimple's book value, which is never taken as a cost: a crypto deposit's is the market value at deposit, an ATON transfer's can be a placeholder until the person sends a statement, and return-of-capital adjustments arrive retroactively (brief 07 §5). It is kept as Wealthsimple's statement for the broker check; the cost of a holding moved in is the person's opening balance.
8. **The daily history.** Settled: `FetchAccountHistoricalFinancials` states each account's value and net deposits per day; the identity-wide query the current code asks no longer exists and is not used.

## Approach

### Research first

The questions above, in order, each answered in Verification with the replies that answered it (kept as fixtures). A question that cannot be answered stops the reader that depends on it, and this plan changes first (*Right to refuse*).

### Two new crates, and where the existing code goes

| Crate | Holds | May depend on | May not |
|---|---|---|---|
| `bagholder-broker` (`crates/broker`) | the broker interface (connection state, the pull's parts, statements), the pull that writes records and statements into the book, and building `BrokerAccount` from the book | `bagholder-core`, `-book`, `-sqlite`, `-sources` (outcomes and health), `jiff` | any broker's reply types; `-model`, `-store`, `-market`, `-ws`, `-server`; any clock read |
| `bagholder-wealthsimple` (`crates/wealthsimple`) | the Wealthsimple adapter: session and refresh, the GraphQL documents, strict readers, the mapping (`book::Mapping`) and its version | `bagholder-core`, `-book`, `-broker`, `-net`, `-sources` (the reply reader and the recorded-replies harness), `jiff` | `-model`, `-store`, `-market`, `-ws`, `-server`; `f64`; any clock read |

`bagholder-ws` stays, unchanged, for the running server until 3c removes it with the old store, as `bagholder-market` stayed through 3a. Nothing is copied from it but the query texts and the refresh discipline above; its lenient wire types and its mapping are not.

The boundary test gains both crates' columns.

### The interface

```text
trait BrokerAdapter {
    fn broker(&self) -> Broker;                         // core's name (core/src/names.rs:70), "wealthsimple"
    fn connection(&self) -> Connection;                 // signed out | connected | lapsed(why) | refused(why)
    fn accounts(&self, net: &Net, now: Timestamp) -> Outcome<Vec<AccountStated>>;
    fn activity(&self, net: &Net, account: &AccountRef, since: Since, now: Timestamp) -> Outcome<ActivityRead>;
    fn statement(&self, net: &Net, account: &AccountRef, now: Timestamp) -> Outcome<Statement>;
    fn history(&self, net: &Net, account: &AccountRef, from: Date, now: Timestamp) -> Outcome<Vec<DayValue>>;
    fn instruments(&self, net: &Net, ids: &[String], now: Timestamp) -> Outcome<Vec<InstrumentStated>>;
    fn mapping(&self) -> &dyn book::Mapping;
}
```

`Outcome` is 3a's (`sources/src/outcome.rs`); every request's outcome is recorded in the market cache's `outcomes` under the broker's source name and the part it served (`accounts`, `activity:<account>`, `statement:<account>`, `history:<account>`, `security`), so the broker's health is a source's health (`sources/src/health.rs`). An answer that says the session lapsed is its own outcome, `Lapsed`, which sets the connection state and is never retried with the same token.

### Reading Wealthsimple's replies

- Every reply through 3a's strict reader (`sources/src/reply.rs`): a required field absent, null or of another type is a mismatch naming its path; an optional accessor only for a field real replies show null. Numbers are read exactly as written.
- **Meaning checks** before anything is written: an account in the reply is one asked for; a currency is a currency code; a quantity on a fill is positive and its sign is the row's side; an option row's contract terms agree with the security it names; a page's rows fall inside the span asked; a day's value has a day.
- A GraphQL `errors` answer is a refusal naming Wealthsimple's code and message; an HTTP 401 or 403 is `Lapsed`.
- **Recorded replies** under `crates/wealthsimple/tests/replies/`, one per operation and shape, with a wrong-shaped and a wrong-meaning copy of each, named as 3a names them. Their shape files are the union of the recorded replies' paths.
- **Anonymised, not scrubbed of figures.** Real figures and tickers stay (owner, 2026-09-23). A committed tool (`bagholder record-reply wealthsimple …`) replaces, consistently within and across replies, every value of the fields that identify the person: identity and account ids, names, e-mail addresses, handles, external account numbers, e-transfer and counterparty names, merchant names, institution account numbers. A test fails when a recorded reply holds a value of those fields that is not a stand-in.

### The session

The session file, the refresh discipline and the refusal memory as carried over above, inside the adapter. Requests go through `bagholder-net` with Wealthsimple's host on the one limiter (3a left it unpaced, `docs/plans/stage-3a-sources.md` Handoff). A lapse is noticed from the reply (401 or 403) or from the token's stated expiry, and the connection's state says which; the command prints it. No request is re-sent automatically after a failure it cannot tell was not applied (RFC 9110 §9.2.2); every read here is a query, so a read is retried once after a refresh, never more.

### The pull, into the book

For each connection, in one run of `bagholder pull-broker <home>`:

1. **Accounts.** Each account stated is added or updated with its type in Bagholder's vocabulary, its status and its broker id as a reference (`book/src/identity.rs`). An account Wealthsimple states as linked to another (`linkedAccount`) is recorded as that link.
2. **Instruments.** Each security a row names and the book does not hold is read (`FetchSecurities`, in batches) and added with Wealthsimple's security id as a strong reference: a listing's symbol, venue and currency; an option's underlying, expiry, strike, right, OCC symbol (a reference) and **multiplier as stated**, which fills `option_terms.multiplier` (left empty "until a source states it", migration 001). A security whose multiplier is not stated keeps the gap `multiplier-unstated`.
3. **Activity.** As question 5 sets (in full once, then only what changed), each row stored as a source record (source `wealthsimple`, key its `canonicalId`, the payload canonical JSON), a changed row as a revision (`book/src/records.rs:173`). An imported record carrying the same id (`broker-record:wealthsimple`, `book/src/import/mapping.rs:35`) is superseded by it (`records.rs:118`), and its trade anchors move with it (`book/src/links.rs`). A row is marked removed only when Wealthsimple states it removed, never because a pull did not return it.
   A multi-leg row's legs (`FetchSoOrdersMultilegOrder`) and a corporate event's entitlements (`FetchCorporateActionChildActivities`) are read with the row and kept in its record beside it, each reply verbatim, so the mapping derives from one record everything Wealthsimple stated about that row, and a changed leg or entitlement is a revision of the row's record.
4. **Statements.** Each account's cash per currency now (the instant it was asked is its `as_of`), its positions as of the last day its activity is read in full (question 4); each account's value and net deposits per day from `FetchAccountHistoricalFinancials`, after the last stored day.
5. **The full read.** When every page of an account's activity was read without a failure, its `activity_read_at` is the instant the first page was asked for; a pull that failed part-way leaves the previous one standing.

Each part's failure is its outcome, named in the command's output and in health; the other parts carry on, and nothing already stored is removed or overwritten by a failed part.

### The mapping

`WealthsimpleMapping`, version 1, implementing `book::Mapping` (`book/src/mapping.rs:27`): one table from Wealthsimple's `type` and `subType` to Bagholder's transaction kinds, each entry naming the fields it reads.
- **A type or sub-type not in the table** is kept as a record with the problem `unclassified`, never guessed.
- **Status first.** A row not stated as executed (cancelled, expired, rejected, pending) moves nothing and raises no gap; a partly filled then cancelled order books its filled part (brief 07 §2).
- **One bad row never stops an account's read.** A row that fails a check (a multi-leg row whose amount differs from its legs' net) is kept as a record with its problem; the rest of the account is read and `activity_read_at` still advances (brief 07 §3).
- **Option fills** state opening or closing as their sub-type says; nothing is inferred from cash.
- **Multi-leg orders**: each leg as the order states it (question 1): its contract, side, opening or closing, filled quantity, price and cash; the order's cash is the legs', the row's amount checked against it. A leg whose quantity or side is not stated keeps `leg-unstated`.
- **Assignments and expiries**: the contract closed and, for an assignment, the shares delivered at the strike, in the quantities the rows state.
- **Transfers between the person's accounts**: the two rows joined by the accounts and instant they state; each security moved is one whose fall in the source account equals its rise in the destination, a move carrying one or several; otherwise the transfer stays unlinked, named (brief 07 §4). The position replies are kept in the row's record, since a mapping is pure over one record and `rederive` cannot reach the network; written to the transfer links the engine reads (`server/src/engine_inputs.rs:43` notes their writer is this adapter).
- **Corporate events** as `docs/plans/stage-2-engine.md` ("Corporate events") sets: the broker's own units are the statement of what moved. Where the event's entitlements state the units given up (`SUBMIT`) and received (`RECEIVE`) (a consolidation, a holding continuing under a new security id), the mapping writes the adjustment from those units, the received security being the unique one whose units changed from the day before to the day after by exactly the stated units, net of the book's own transactions on those days (brief 07 §4): the old holding's lots continue into the units received with their cost and acquisition days, cash in lieu from its own row. Where a row does not state what the event did to cost (a spin-off's split of cost, a return of capital), the event stays `event-unknown` until the person's entry or a sourced value.
- **The day a row is filed under** is Alberta's (`docs/architecture.md` §7), as the import mapping already applies (`import/mapping.rs:38`).

When a later version of the mapping changes past transactions, `rederive` (`records.rs:373`) derives them again from the kept records, and the command reports what moved.

### The book's new tables

Book migration 006 (gated here), with its schema snapshot `schema/v6.sql`:

| Table | Holds |
|---|---|
| `account_links(account_id, linked_to, kind, stated_at)` | an account and the account Wealthsimple states it is linked to |
| `account_days(account_id, day, net_value, net_deposits, currency, read_id)` | the broker's value and net deposits per account per day; a later read stating a different value for a day is kept beside the first, the newest used, and the change reported, as a broker revising its own row is (§6) |
| `statements(id, account_id, kind, stated_at, as_of_day, read_id)`, `statement_cash(statement_id, currency, amount)`, `statement_units(statement_id, instrument_id, quantity, book_value, book_value_currency)` | each statement of cash now and of units as of a day, kept (the broker check is computed from them). The book value is not read: no figure uses it, and Wealthsimple writes it to more digits than a decimal holds |
| `activity_reads(account_id, read_at, complete)` | each read of an account's activity and whether it was read in full |
| `broker_reads(id, connection_id, part, at)` | each read the rows above came from |

`BrokerAccount` (`engine/src/input.rs:230`) is built by one typed read of these tables (`server/src/engine_inputs.rs`, `brokers`, beside the ledger's own read; the broker crate stays free of the engine): `net_value` and `net_deposits` from `account_days`, `cash` and `as_of` from the newest balances, `held` from the newest positions as of a day (with that day, a field `BrokerAccount` gains), `activity_read_at` from the newest complete read. Buying power is not stored: it is not a fact a figure in the book is computed from, and the Portfolio tile reads it live in 3c.

### The facts the owner enters

Defined in `SPEC.md` first, in the same commit as the code (the text below is the proposal):

- **Opening balance** (in "Add trade"): for units that arrived in an account without a cost (a transfer in, whose cost Wealthsimple does not state, question 7, shown today as `basis-unknown`), the person states their total cost in the instrument's currency and the day acquired, as Sharesight's opening balance takes a holding's cost and date; the units are the arrival's, as the broker states them. They become the holding's lot at that cost and day.
- **An event's allocation** (on the trade detail, beside the journal, for a holding whose event is `event-unknown`): for a spin-off, the share of the parent's cost that moves to each child, as the issuer published it; for a return of capital, the capital returned per unit, as the issuer published it for that distribution, lowering the cost of the units held on the day it is paid (the day its record carries; Sharesight records it on the day paid). Returned beyond a lot's remaining cost is realized that day and the lot's cost is zero.
- **Each is shown as entered by the person**, and a value from a source replaces it where one arrives (`input.rs:171`, `Adjustments::choose`: a sourced adjustment wins).

In the book, each entry is a record whose source is the person (`SourceName::person()`), its mapping (`book/src/person.rs`, `PersonMapping`) producing adjustment legs (`core/src/adjustment.rs:15-24`), so no new table. `Book::enter` keeps one, refusing what the book cannot stand behind (a cost in another currency than the instrument's, a share of cost outside (0, 1] or adding past the whole, capital returned that is not an amount). The HTTP route that calls it, and the forms, are 3c's: the server holds the book from the switch.

Where a return of capital exceeds the holding's remaining cost, the excess is realized P&L on the ex-date and the cost is zero (the CRA's rule, above).

### The comparison, run again

The stage 2 comparison (`server/src/compare.rs`) run on a copy of the person's data, with the book the pull wrote in place of the imported one: trades, positions, income and the portfolio's value, each against the old model, and each difference explained. The gaps 3a's run left waiting on 3b (`leg-unstated`, `multiplier-unstated`, `event-unknown`, `price-unknown` on an in-the-money expiry, `value-unstated`) are each resolved or named with what Wealthsimple states.

## Acceptance criteria

The template's page lines do not apply: the page does not change in this part.

- [x] **Build.** From `rust/`: `cargo test -q --workspace` green; `RUSTFLAGS="-D warnings" cargo build -q --workspace --all-targets` clean; the applet test green alone.
- [x] **Research.** The eight rules above each hold on a recorded reply of their kind, a test per rule.
- [x] **Boundaries.** The boundary test holds `bagholder-broker` and `bagholder-wealthsimple` to their columns, each checked by feeding the checker a violation (a reply type in `-broker`; `f64`, a clock read, or `-ws` in `-wealthsimple`). The scan for the old guesses (`engine/tests/no_guesses.rs`, `lenient`, `option_multiplier`, `from_int(100)`, `relabel`) and the no-float scan cover both crates.
- [ ] **Recorded replies.** Every operation the adapter asks has recorded real replies (an answer, an empty answer, a refusal, a lapsed session where Wealthsimple gives one), each with a wrong-shaped and a wrong-meaning copy, and a test per reply asserting exactly what is written, or that nothing is and which outcome is recorded. The anonymisation test passes on every one.
- [x] **Session**, each a test on recorded replies and a fake clock: a refresh adopted from a session another caller rotated, posting nothing; a refused refresh token never posted again; a 401 sets `Lapsed` and the read is not retried with the same token; a read retried once after a refresh, never twice.
- [x] **One limiter.** Every request the adapter sends goes through `bagholder-net`, on Wealthsimple's host settings: a test on the fake network counts them.
- [x] **Records.** On recorded replies: each row stored once as a source record, a changed row as a revision, an unchanged row as nothing new; an imported record with the same Wealthsimple id superseded and its trade and journal kept (the trade id unchanged); a row absent from a later pull not marked removed.
- [x] **Mapping**, each a test on a recorded row: a multi-leg order's cash from its legs, and a row whose amount differs from its legs' net a failure writing nothing; every `type`/`subType` pair in the person's rows in the table or kept as `unclassified`; option opening and closing as stated; a multi-leg order's legs as stated; an assignment's delivery; an expiry; a transfer's two sides linked by the broker's id (and a test that two rows with equal amounts and no shared id are not linked); a consolidation and a continuation under a new security id from their units, with cash in lieu.
- [x] **Statements and reads.** Migration 006 applied to a version 5 book with its rows intact, `schema/v6.sql` committed and compared; a day's value restated kept beside the first and the newest used; `activity_read_at` set only by a complete read, a pull failing part-way leaving the previous one; `BrokerAccount` built from the tables, a test per field.
- [x] **Owner entries.** `SPEC.md` defines the opening balance and the event allocation; engine cases whose expected figures were written by an agent that had not read the engine: a transfer in with its entered cost, and without; a spin-off allocation; a return of capital, including one exceeding the remaining cost and units sold before the day paid; a sourced value replacing an entry. `cargo test -p bagholder-engine --test cases` green on them. `Book::enter` writes a person record and a test reads its adjustment back; its refusals each a test.
- [x] **Only what changed, held by a test.** On recorded replies, a pull with nothing new sends exactly the accounts list and one activity page per open account; a pull with one new trade sends only what that trade needs; any other count fails. The same two counts on the real run, stated in Verification.
- [x] **Status and bad rows**, each a test: a cancelled and an expired multi-leg order move nothing and raise no gap; a partly filled then cancelled order books its filled part; a row failing its check is kept with its problem and the account's read completes. Verification lists the shapes the multi-leg sign rule was checked on (the row's currency against the legs', any fee).
- [x] **Decisions held by tests.** Each line of `docs/decisions.md` names the test that fails if it is broken, or says "review only"; the missing tests for the decisions 3b touches are added.
- [ ] **The real run.** `bagholder pull-broker` on a scratch copy of the person's data with the scratch sign-in of decision 1: its output in Verification (accounts, records stored and superseded, requests sent, outcomes); every imported record with a Wealthsimple id superseded, or listed with why no Wealthsimple row exists; no journal entry orphaned, or each listed.
- [x] **The broker check on the real run.** For every account, the check's differences between the book and the newest statement: none, or each named with its cause (a row not yet posted, a gap the engine names, a mapping error then fixed).
- [ ] **The comparison run again**, its numbers in Verification against 3a's run, and each of 3a's waiting gaps resolved or named with what Wealthsimple states.
- [ ] **Docs.** `SPEC.md`, `docs/architecture.md` (if the interface departs from §10), `docs/old-app-mistakes.md` (the guessed overlap), `docs/design-review.md` (the order of work) and the cases README say what the code does.

## Surfaces to check beyond the diff

- `rust/crates/ws` and `server/src/session.rs`: untouched; the running server keeps using them until 3c.
- `rust/crates/book/migrations/006-*.sql` and `schema/v6.sql`; `book/src/import/mapping.rs` (the ref the supersede finds).
- `TIMED_WAITS` (`server/src/tests_misc.rs`): unchanged; 3b adds no periodic read.
- `docs/old-app-mistakes.md`; `rust/crates/engine/tests/cases/README.md`.
- `docs/plans/trade-open-to-flat.md`: independent of this plan, but both touch `engine/src/identity.rs` and the engine cases; whichever lands second rebases its cases on the other.

## Right to refuse

Nothing refused. Two points where the plan departs from what was written before, each argued:
- `docs/plans/stage-3a-sources.md` (Scope) gave 3b "which official source answers for which kind of event" as its first research. The owner's decision of 2026-09-24 drops per-issuer event readers; events come from Wealthsimple's units where they state them and from the person's entry otherwise.
- `docs/architecture.md` §10 lists signing in under the broker's connection. The sign-in method is left to stage 5 with the keychain (§12), because changing it changes the Connecting screen in `SPEC.md`, and nothing 3b delivers depends on it. The bundle shows why it is its own piece of work: Wealthsimple's password sign-in carries a Cloudflare Turnstile token (`x-wealthsimple-turnstile`) and a second factor (`x-wealthsimple-otp`), and passkeys (`grant_type=webauthn`) are bound to Wealthsimple's own origin, so a sign-in that never shows Wealthsimple's page needs its own research.

## Anti-stub self-check

- No definition nobody references: `Source`, `Replay`, `Client`, `BrokerAdapter::prepare`/`holds`, the statements tables and `Book::enter` are each read by the pull, the command or a test; `statement_units.book_value` is written empty and read by nothing (migration 006 was pushed with it; a pushed migration is not edited).
- No field written and never read: `BrokerAccount.held_as_of` is read by the broker check.
- No branch only the switch knows: the entries' HTTP route and forms are 3c's whole, not stubbed.
- No real-target run skipped: the pull ran on a copy of the owner's book from the owner's own capture (below). The network client has not met Wealthsimple itself: its tests are on a fake network; its first real run is the owner's first pull after the switch, or a scratch sign-in.

## Verification

- Suite, from `rust/`: `cargo test -q --workspace` 1,035 passed, 0 failed; `RUSTFLAGS="-D warnings" cargo build -q --workspace --all-targets` clean.
- Research: questions 1–8 settled from the web app's bundle (release 0.3.668812) and the owner's own lookups; each rule held by a test in `wealthsimple/tests/mapping.rs` on a real row of its kind.
- The mapping on the owner's whole history (`BAGHOLDER_WS_CAPTURE=… cargo test -p bagholder-wealthsimple --test capture -- --ignored`): 7,859 rows, every kind read and placed; problems left, each named: 36 conversions whose paid side Wealthsimple does not state (`conversion-side-unstated`) and 2 rows naming a security the capture did not read. No move is left unstated.
- The real run (`bagholder pull-broker <copy of the book> --replay <the owner's capture, as sent>`): accounts added 0 (every account matched), 7,859 records new, 6,088 imported records replaced, 35 removed (below); 23,995 account days stored. A second pull the same day read 21 rows, all unchanged, and asked only the accounts, their activity and their cash.
- Only what changed, held by `wealthsimple/tests/pull.rs`: a pull with nothing new asks exactly `accounts`, one activity read per open account and `balances`; one new trade adds one `securities` batch.
- The broker check on the real run: 21 of 29 accounts agree exactly. Fixed on the way, each held by a test on the owner's own replies:
  - Card purchases counted twice: Wealthsimple lists a purchase under one id while pending and another once posted; the import held the pending ones. A pull now removes what the broker no longer lists over the span it read (`tests/pull.rs`).
  - Transfers from other institutions booked at the value asked for: now what arrived, on the day it completed (above; `tests/mapping.rs`). All four of the owner's arrived differently (80,650.30 for 80,807.80 asked; 1,160.10 for 1,159.24; 1,910.89 for 1,910.26; 4,788.55 for 4,238.00).
  - Moves that state no amount (±9,307.07; 6,861.21): now from both accounts' net deposits (`tests/mapping.rs`).
  - Tax withheld counted twice on each withdrawal from a registered account (four, 3,341.54 in all): now part of its gross amount (`tests/mapping.rs`).
  - The card's balance, not in the balances reply, now read from `creditCardAccount.balance.current` (what is owed; `tests/pull.rs`): it agrees to the cent.
  The differences left, each with its cause:
  - Two withdrawals from a locked-in account are in no activity row of Wealthsimple's, on either side: the locked-in account's rows hold only the withholding tax, its net deposits drop by the gross amounts on those days, and the chequing account they were paid into holds no deposit. The locked-in account's cash differs by 72,950.00, exactly the two net payouts; the chequing account's by the same less 1.73 left from May 2024 through the last full day, and 20.00 of the day's rows posted after the capture read its activity.
  - One inflow of 50.01 into the crypto account in no row: its stated cash, read day by day over a year, follows the book to the cent until a purchase of 99.44 went through with 49.43 on hand; from then it stands 50.01 above the book.
  - Conversions whose paid side a funding intent does not state (`conversion-side-unstated`): the trading account's CAD and USD, the margin account's USD (its three conversions received 8,260.26 CAD for the 6,159.07 USD the book still holds) and the event-contracts account (100.00 CAD paid for 70.75 USD).
  - A holding under a new security id with no event row (positions only): its units show on both ids, in two accounts.
  - Coins whose positions Wealthsimple states to fewer digits than its rows: the crypto account's differences below 0.000001 of a unit; and a coin whose rows state its units rounded to whole units (PEPE: 2,590,417.0 on a fill) while its positions state them exactly: 1,567.76 PEPE in the book that Wealthsimple does not hold.
  - Two coins the crypto account holds as dust after token migrations (RNDR to RENDER, FTM to S) that no row records: the units statement names them as a failure and leaves them out, so the check cannot show them beside the old ids.
  - A spending account's 30.00: its cash matches Wealthsimple's stated value to the cent on every day through the last full day; the difference is the day's withdrawals posted after the capture read its activity.
- Session replies: a read refused for its session (401, `UNAUTHENTICATED`) and a read of what does not exist (200 with `NOT_FOUND`) are Wealthsimple's own, recorded in the owner's browser (`tests/session.rs`).
- The comparison run again (`compare-figures … --facts-from-book`): portfolio market value 627,165.03 CAD with 3 left out (3a's run: 632,070.43 with 15), the contract sizes and the MSTY consolidation now stated; realized P&L by instrument 106 agree to the cent, 62 differ; the per-trade and per-payment matching reads 0 the same, because the tool matches by the imported records' keys, which Wealthsimple's records replaced (the tool's matching by the broker's own id is the follow-up below).
- Owner entries: `engine/tests/cases/entries.json` (9 cases written by an agent that had not read the engine; one re-derived after `SPEC.md` set the day a return of capital applies) green; `book/tests/person.rs` green.

## Handoff

Stopped at: the broker check traced. What is left, in order:
1. A statement's units in an instrument no row names: make the instrument from the broker's security record (read in the securities batch) so the check shows it as a difference beside the instrument it replaced, rather than a failure that leaves it out.
2. `compare-figures` now names a broker record by the imported row it replaced (`record_refs`); per-trade pairing still reads 0 the same, which points at the round trips' boundaries rather than the keys (the old model split a position's trips where the new one keeps one open to flat): pair by instrument and close instead, then read the differences.

Not to redo: the mapping's rules, the capture, the anonymiser, the pull and its request counts.

To resume: `cd rust && cargo run -q -p bagholder-server --bin bagholder -- pull-broker $TMPDIR/bh-3b/run7 --replay <capture as sent>` and `compare-figures $TMPDIR/old-ro.db $TMPDIR/bh-3b/run7 2026-09-24 --facts-from-book --cache $TMPDIR/bh-3b/run7/market.db`.

**Nothing left running.**
