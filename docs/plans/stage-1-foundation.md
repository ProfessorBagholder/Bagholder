# Plan: stage 1, the foundation: identity, the book, exact money, numbered migrations

## Scope

The first stage of the order of work in `docs/design-review.md`: what everything else is built on. It delivers `docs/architecture.md` §5 (identity), §6 (the book: source records, transactions, links, the person's own entries, the journal keyed to stable trade ids; decimal money; migrations and the pre-migration snapshot), and the import that turns an existing database into a book.

Nothing the person sees changes in this stage. The running app keeps reading and writing its current database; the new book is built beside it, by an importer, and is read by nothing in the app until stage 2 moves the engine onto it. That is the point of building alongside: the app keeps working while its foundation is replaced, and the switch happens once, in stage 2, with the engine that needs it.

`SPEC.md` does not change in this stage: no figure, screen or behaviour moves. The spec changes of `docs/architecture.md` §18 land with the stages that make them true.

The plan was reviewed before building by a reviewer who wrote none of it; its findings are folded in below.

## Approach

### Three new crates, and what each may depend on

| Crate | Holds | May depend on | May not |
|---|---|---|---|
| `bagholder-core` (`crates/core`) | The vocabulary: ids, currency, exact decimals, money, quantity; instruments, issuers, accounts, broker connections; source records, transactions, links, trades, the journal | `serde`, `rust_decimal`, `jiff`, `uuid` | the database, the network, the clock, any source's reply types |
| `bagholder-sqlite` (`crates/sqlite`) | Opening a database file, transactions, the connection pool, numbered migrations, snapshots | `rusqlite`, `jiff` | everything else |
| `bagholder-book` (`crates/book`) | The book store: its migrations and typed reads and writes; the mapping contract; the import mapping for existing databases | `bagholder-core`, `bagholder-sqlite`, `rusqlite`, `serde`, `serde_json`, `uuid`, `jiff` | the network, the clock, the old crates (`bagholder-store`, `-model`, `-ws`, `-market`, `-server`) |

The stage 2 engine will depend on `bagholder-core` alone, never on `rust_decimal` directly.

**How the boundaries are held.** A test (`crates/core/tests/boundaries.rs`) reads each new crate's `Cargo.toml` and fails on a dependency outside its column. Floats cannot become money by construction: `core`'s decimal type wraps `rust_decimal` privately and has no constructor from a float; a decimal is made from integers or from text, and a float-to-text conversion exists in one module only (the import mapping, which reads the old database's floats), which the same test holds by scanning for `f64` in the new crates' sources outside it. Clock reads are held by a scan of the same sources for `now(`, `SystemTime` and `Instant`; a scan can be evaded by an alias, so it is a tripwire, and review against this plan is the rest. The toolchain is pinned to the minimal profile and has no Clippy, so Clippy's method bans are not available to CI.

**The database plumbing** the current store already has (WAL, `atomically`, the pool, the commit hook) is judged sound by the design review. It moves into `bagholder-sqlite` unchanged, and `bagholder-store` re-exports it, so there is one copy and every current caller is untouched.

### Exact decimals and money

- **`rust_decimal`** underneath: a 96-bit integer with a decimal scale, exact to 28 significant digits, no heap, the common choice for financial Rust. It covers a coin's quantity to 18 places and any price or amount a broker states. `bigdecimal` (unbounded, heap-allocated) was weighed and is not needed at this precision. Recorded in `crates/core/Cargo.toml` with this reason; its serde support is string-only, with no float feature.
- **`Dec`**, core's own decimal: made from text (strictly: plain decimal notation, optional sign, no exponent) or integers; adds, subtracts and multiplies with checked operations that fail rather than round (a product needing more than 28 places is an error, never silently rounded); divides only through `div_rounded(divisor, places, rule)`, which names its rounding rule. A float leaves it only through `to_f64`, for the statistics (ratios, returns) the design keeps in floating point.
- **`Currency`**: an ISO 4217 code (three capital letters), any currency, not CAD and USD alone. A coin is an instrument, not a currency.
- **`Money { amount, currency }`** has no `+` or `-`: adding is `checked_add`, which fails on two currencies, and a sum is `Money::sum(currency, …)`. A `compile_fail` doctest holds that `money + money` does not compile. Converting between currencies is the engine's, at a rate it records as a fact (stage 2).
- **Quantities** are signed `Dec`s: into the account is positive, out is negative.
- **Stored as text**: the decimal's canonical form (`"1050"`, `"0.321720925242232"`), read back strictly; a malformed stored value is an error naming its table and column. Integer minor units were weighed and rejected: quantities and per-unit prices have no fixed number of places. Nothing is summed in SQL.
- **The old database's floats**: read as the shortest decimal that reads back to the same float. That is exact for what Wealthsimple sent and was stored as received; values the old app computed (a per-unit price) are not carried as stated facts (below).

### Identity (§5)

- **Ids** are UUID v7, assigned by the book when a thing is first stored, and never change: `InstrumentId`, `IssuerId`, `AccountId`, `ConnectionId`, `RecordId`, `TradeId`, `GroupId`.
- **Transaction ids are the record's id and a named leg**, `TransactionId { record, leg }`, where the leg is the mapping's name for what that part of the record is (`trade`, `out`, `in`, `fee`, `withholding`; the import's one leg per old row is `row`), never its position. So deriving a record again gives the same ids, and a mapping version that adds a leg does not shift the others.
- **Instruments**: `instruments(id, kind, currency, issuer_id)`. Kinds: listed security (share, ETF, fund, warrant), option contract, crypto asset, event contract (a prediction market's contract), and the market's own instruments the watchlist follows (index, future, rate, currency pair). What an instrument is called is dated, and derived from the records like its transactions: each record's leg keeps a sighting, `instrument_sightings(record_id, leg, instrument_id, symbol, venue_mic, venue_name, name, day)`, on the day the source files it under, and an instrument's names are the unbroken runs of its live records' sightings under one symbol and venue, each from its first day to its last. So a ticker change keeps the id; a ticker that changes and changes back is three names; a move to another venue is another name; and a record revised or removed takes its sighting with it. When exactly a name changed is known only to within the days between sightings; the corporate event that changed it (stage 2) says it exactly. An option's terms are their own row: `option_terms(instrument_id, underlying_id, expiry, strike, right, multiplier, source)`; the strike is an exact decimal; the multiplier is empty until a source states it (the stage 3 Wealthsimple adapter reads `optionDetails.multiplier`), and a figure that needs it waits for it rather than using 100.
- **References**: `instrument_refs(scheme, value, instrument_id)` holds the identifying ones, unique per scheme and value; `instrument_routes(instrument_id, scheme, value)` holds the routing ones, where one value may lead to several instruments (Charbone Hydrogen and Charbone Corp. are both `CH.V` to Yahoo).
  - *Strong* schemes identify an instrument everywhere: a broker's own security id (per broker), ISIN, CUSIP, FIGI, the OCC option symbol.
  - A *connection-scoped* scheme identifies it within one connection only: a symbol and currency as one source names them (a CSV with no security ids), which groups that source's rows and never joins another's.
  - *Routing* schemes only say how to ask a source for it: a Yahoo symbol, a TMX form, a SEC CIK, a SEDAR+ profile. They never identify, and a value shared with another instrument never blocks a record.
  - `resolve` finds an instrument by strong or connection-scoped references only, and nothing is ever matched on a bare symbol. So `CH` on TSX-V under two Wealthsimple security ids (Charbone Hydrogen, then Charbone Corp.) is two instruments, joined later by the corporate event between them (stage 2), not by their shared ticker. When one record's references name two different existing instruments, or a new reference would take a value another instrument already holds, nothing is merged and nothing is picked: the record gets a problem naming both.
- **Issuers**: `issuers(id, name)`, above their listings. The first writers are the stage 3 sources (filings, the exchanges' records); this stage builds the table, the attach call and its test.
- **Broker connections and accounts**: `broker_connections(id, broker, label)`; `accounts(id, connection_id, kind, registration, managed, joint, status, nickname)` with `account_refs(account_id, scheme, value)`. The broker's account id is a reference, so Wealthsimple's separate CAD and USD accounts that it links are one Bagholder account with two references, holding two currencies. The kinds are Bagholder's vocabulary (cash, margin, crypto, event contracts, spending, credit card, line of credit) and the registration (none, TFSA, RRSP, RESP, LIRA, RRIF, FHSA, group RRSP) is separate; a broker type the mapping does not know is kept as `Unrecognised(text)` with a problem, never guessed into another kind.

### The book (§6)

- **Source records**: `source_records(id, connection_id, source, source_key, state)`, unique on connection, source and key (two CSV files from two connections can share a key), and `record_revisions(record_id, revision, received_at, payload)`. `record_refs(record_id, scheme, value)` holds other ids a record is known by, so a later source can find it (an imported row's Wealthsimple id).
- **The payload is canonical JSON**: what the source sent with its keys sorted, no insignificant whitespace, and each number written in its canonical decimal form (exactly, without passing through a float). Nothing a source meant is lost by this, and two replies that say the same thing in a different key order or number spelling are the same payload. Receiving a record again with the same payload writes nothing; a different payload appends a revision and derives the record's transactions again; storing is an upsert inside one immediate transaction, so two writers storing the same record at once make one record.
- **Removal**: a record is marked removed only by `mark_removed`, which an adapter calls when its source reports the removal; a record missing from a pull is left alone. Every write takes the time it happened as an argument: the book never reads the clock.
- **Transactions**: `transactions(record_id, leg, mapping_version, account_id, occurred_at, trade_date, settle_date, kind, effect, instrument_id, quantity, price, price_currency, cash, cash_currency, fee, fee_currency, fx_rate)`, with `CHECK`s that every amount has its currency, a quantity has an instrument and a price has a quantity.
  - One vocabulary for every broker: buy, sell (to open or to close, for options), dividend, interest, interest charge, fee, withholding tax, deposit (from the person), employer deposit, government deposit, withdrawal, transfer in, transfer out, currency conversion, option expiry, assignment, exercise, event contract resolution, staking move, staking reward, card purchase, card refund, cashback, corporate event, and `unclassified` for a record the mapping cannot place, which carries a problem and counts in no figure.
  - The cash is signed as the source states it: a dividend taking cash out is a reversal and is booked as one, not turned into income.
  - `price` is the per-unit price only where the source states one (a CSV's price column, an order's average fill in stage 4); Wealthsimple's activity rows state none, and no price is ever worked out from cash and quantity here. `fx_rate` is a conversion rate only where the source states the one it applied.
  - `occurred_at` is a UTC instant where the source gives one and empty where it gives only a date; `trade_date` is the day the broker files it under, in the broker's zone for an instant (Wealthsimple's is Alberta's, `docs/architecture.md` §7), or the stated date as it is; `settle_date` only where the source states one.
  - A quantity is present where the transaction moves a position; where the record does not state it (a Wealthsimple multi-leg option fill with none) it is empty with a problem on the record, never worked out from the cash.
- **The mapping contract**: a mapping is pure (`fn map(&self, payload) -> Mapped`), versioned, and names instruments and accounts by their references and attributes, never by Bagholder ids. The book resolves references to ids, creating an instrument or account the first time one is seen, and writes the record's revision, its transactions and its problems in one database transaction. A mapping that cannot read a payload leaves the record kept, with no transactions and a problem naming why. `record_problems(record_id, code, detail)`, with the record's `derived_version` saying which version of the mapping they came from, is what makes each of these visible (§15); stage 5 puts them on screen.
- **Re-deriving**: `Book::rederive(mapping)` derives again every live record whose transactions or problems came from an older version of that mapping, in one transaction, and returns the transaction ids added, changed and removed, so stage 2 can say which figures a mapping correction moved. Re-deriving with the same version changes nothing. After re-deriving, each trade's anchor must still be an opening of the same account, instrument and direction; one that is not is orphaned (below).
- **Links**: `links(id, kind, created_at, reason)` and `link_records(link_id, record_id, side)`, many to many; a record's state (live, superseded, removed) is kept on the record and changed only with a link or a reported removal, in the same transaction. `supersede(from, to)` is the one move this stage needs, and a source stores a new record and supersedes the ones it replaces in one call, so the two are never both counted:
  - the `from` records become superseded: their transactions leave the book, their revisions stay;
  - every trade anchored on one of their transactions moves to the `to` records' transaction for the same account, instrument and direction, and for an option the same effect (a buy to open never moves onto a buy to close), the earliest by `occurred_at`, then record id, then leg, when several qualify;
  - a chain (A superseded by B, B by C) ends with every anchor on C;
  - the same replacement delivered again changes nothing, and a record another record already replaced cannot be claimed by a third;
  - a superseding record later reported removed does not bring back the one it replaced: it is a problem for the person;
  - a trade with no counterpart becomes orphaned: its journal is kept and listed for the person to re-attach, never dropped.
  Stage 3 uses it when Wealthsimple's raw row replaces an imported one (found by `record_refs`), stage 4 when the broker's rows replace a provisional fill, and a sourced value uses it to replace one the person entered.
- **What the person enters**: an adjustment (a cost basis for shares transferred in, a split ratio, a spin-off's allocation) is a source record whose source is the person, mapped like any other, and replaced by a sourced record through `supersede`. No separate table. The first writer is the interface (stage 5); this stage holds it with a test.
- **Trades and the journal**: `trades(id, anchor_record, anchor_leg, created_at, orphaned_reason, legacy_key)`: a trade's id is assigned when its opening transaction is first stored, and anchored to it; only a transaction that moves a position (an instrument and a quantity) can open one. The engine creates them from stage 2 on; this stage creates the ones the old journal and groups need. `trade_groups(id, locked, legacy_key, created_at)` with `trade_group_members(group_id, position, trade_id)`; `journal(id, trade_id, group_id, thesis, grade, updated_at)`, each entry on one trade or one group (the earlier app let a note sit on a group the person saved, which the screens show as one trade), with `journal_tags(journal_id, position, tag)`. Their own tables, not text in a settings row.
- **What waits for its stage, written down so nothing is lost**: the facts a figure used (FX rates applied, declared distributions, event values) come with the engine that uses them (stage 2); the watchlist, which names the market's own instruments from the market directory, comes with the sources (stage 3); orders, brackets and their event logs with execution (stage 4); tiles, notification settings and history with the interface (stage 5). The second store, the market cache, opens in stage 3 with the adapters whose data it holds, on the migration and snapshot machinery built here. The old database is never modified or deleted, so each later stage imports its part from it.

### Migrations and snapshots

- `bagholder-sqlite::migrate` runs a list of numbered migrations (1, 2, 3 … with no gaps, checked when a store opens), each in its own transaction with the version bump, recorded in `PRAGMA user_version` and a `schema_migrations(number, name, applied_at, app_version)` table. Each store writes its own id into the file's header, so one store's file is never opened as another's, and a database of tables the runner never made (an old `bagholder.db`) is refused rather than migrated.
- A file written by a newer version is refused with a message naming both versions. A migration that fails rolls back and leaves the file at the previous version, and the error names the migration.
- Before running any pending migration on an existing file, the file is copied whole with `VACUUM INTO` to `snapshots/<file>-v<from>-<time>.db` in the data folder. The newest snapshot of each of the two latest versions is kept: the newest is of the file as the last update found it, the one a failed update restores, and an update that fails and is retried replaces only its own version's snapshot. The file is looked at read-only first: one that is not this store, or is newer, is refused exactly as it was found. Restoring is part of the updater's rollback (stage 5).
- A migration once released is never edited: the schema each version produces is committed as text (`crates/book/schema/v<N>.sql`), and a test builds each version from the migrations and compares.

### The import of an existing database

Two parts, split by what each needs:

- **The book's import mapping** (`bagholder_book::import`, source `bagholder-import`, version 1) reads an existing database (Python's `~/.bagholder/bagholder.db` or the Rust build's `~/.bagholder-rust/bagholder.db`: both are schema 13, the same tables). It first copies the live file with SQLite's backup API, and reads only the copy.
- **The old keys' translation** lives in the server crate, which still has the old matching code: the journal's and the groups' keys are the old engine's own round-trip and slice ids, and only that code can say which trade each names. It hands the book plain anchors (for each journal entry the row that opened its trade or the group it is on, for each group member the row that opened its trade, or why none was found), and `bagholder import-book <old database> <new data folder>` runs both and prints the report. The app starts calling the same code in stage 2.

What the import writes:

- One broker connection, Wealthsimple.
- Accounts from `accounts`, one Bagholder account per group of Wealthsimple ids that the old book pooled (`activities.fifo_id`), each id a reference; a pool that has gained an id since an earlier import adds it to the same account. An unknown broker account type is `Unrecognised` and listed in the report. The old book pooled a linked pair only where both sides had rows; a pair it did not pool stays two accounts until Wealthsimple's own link says otherwise, and stage 3's adapter, which reads `linkedAccount`, makes them one.
- Instruments from `securities` and the rows, one per Wealthsimple security id, priced in the currency the security's row states (a security with no currency stated gets no instrument, with a problem, and the row's cash is still booked); a row without a security id, from an old CSV import, is named by the connection-scoped symbol reference and the currency the file stated. Kind from what the rows do with it (crypto orders, option orders and events, prediction orders, share orders and dividends); a security id rows use as two kinds is a problem, not a choice. Names dated from the symbols its rows carry over time. An option's terms come from the name the old mapping printed from Wealthsimple's contract fields (`QNC 19FEB27 3.00 CALL`, read strictly, the strike exact), recorded as parsed by the import, to be replaced by the contract's own terms in stage 3; the underlying from `securities.underlying_id`; the multiplier empty.
- One source record per activity row, keyed by the row's id, the whole row as the payload (including the row's own `source`: synced, CSV, or a booked fill), its Wealthsimple id (`canonical_id`) as a record reference. The old rows are already mapped, not what Wealthsimple sent, so they are a source of their own; stage 3's first full pull stores Wealthsimple's raw rows and each supersedes the imported record with its Wealthsimple id.
- Transactions by this table of every row shape the old store holds (found in both real databases), cash signed as the row states unless the table says otherwise:

| Old row: type / sub-type / Wealthsimple type | Transaction |
|---|---|
| `Trade` `BUY` / `DIY_BUY` | buy; quantity +; cash − |
| `Trade` `SELL` / `DIY_SELL` | sell; quantity −; cash + |
| `OPTIONS_BUY` `BUYTOOPEN` or `BUYTOCLOSE` | buy to open, buy to close; quantity + (empty with a problem where the row has none: the multi-leg fills); cash − |
| `OPTIONS_SELL` `SELLTOOPEN` or `SELLTOCLOSE` | sell to open, sell to close; quantity − (same rule); cash + |
| `EXPIR` `BUY` | expiry of a short; quantity +; no cash |
| `EXPIR` `SELL` | expiry of a long; quantity −; no cash |
| `ASSIGN`, `EXERCISE` | assignment, exercise; quantity and cash as the row states |
| `STKDIS` (Wealthsimple `CORPORATE_ACTION` or `DIVIDEND`) | corporate event; quantity as signed; its kind and values are stage 2's to book from the record |
| `Dividend` | dividend |
| `Interest` | interest |
| `INTEREST_CHARGE` | interest charge |
| `WITHHOLDING_TAX` | withholding tax |
| `FxExchange` | currency conversion, the one side the row holds (the old rows keep one side per row; stage 3's raw rows join the two) |
| `Deposit` | deposit from the person |
| `GROUP_CONTRIBUTION` | deposit from the employer (employer contribution) or the person (employee contribution) |
| `RESP_GRANT` | deposit from the government |
| `Withdrawal` | withdrawal |
| `Transfer` (`INTERNAL_TRANSFER`), `ASSET_MOVEMENT` | transfer in when the cash is positive, out when negative (no security moves in these rows) |
| `CRYPTO_BUY` | buy of the coin; quantity +; cash − (the old rows record it as a credit: the old mapping took Wealthsimple's `amountSign`, which for crypto orders is not the cash's direction; a buy pays) |
| `CRYPTO_SELL` | sell of the coin; quantity −; cash + |
| `CRYPTO_TRANSFER` `TRANSFER_IN` | transfer in of the coin; quantity +; no cash (the row's amount is the coins' value, not cash moved) |
| `CRYPTO_TRANSFER` `TRANSFER_OUT` | transfer out of the coin; quantity −; no cash |
| `CRYPTO_STAKING_REWARD` | staking reward; quantity +; no cash |
| `CRYPTO_STAKING_ACTION` | staking move; no change to the position |
| `PREDICTIONS_BUY` | buy of the event contract; quantity +; cash − (same sign note as a coin) |
| `PREDICTIONS_RESOLUTION` | resolution; quantity −; cash as the row states |
| `CREDIT_CARD` `PURCHASE` | card purchase |
| `CREDIT_CARD` `REFUND` | card refund |
| `CREDIT_CARD` `PAYMENT` | transfer in (the card receiving a payment) |
| `CREDIT_CARD_PAYMENT` | transfer out (the account paying the card) |
| `REIMBURSEMENT` | cashback |
| `INSTITUTIONAL_TRANSFER_INTENT`, and any shape not in this table | unclassified, with a problem naming the shape: the old row dropped the status that says whether the transfer happened, and stage 3 reads Wealthsimple's own row |

  A row's commission, where it has one, is the transaction's fee. A row from a CSV import carries its price as stated; a synced row's per-unit price was the old app's cash-over-quantity arithmetic (with ×100 for options) and is not carried. A booked fill (a provisional record of the old app's own) carries the broker's average fill price and its quantity, but not its cash, which the old app worked out as price × quantity, × 100 for an option: the engine (stage 2) derives a provisional fill's cash from its price, quantity and the contract's stated multiplier, as it will for stage 4's provisional fills, until the broker's row replaces it. Where the old mapping signed a row itself (share and option orders, expiries), a quantity or cash signed against the row's kind is booked as the kind says and shown as a problem (`sign-against-kind`); a coin's or event contract's rows kept Wealthsimple's unsigned quantity and its `amountSign`, which is not the cash's direction, so the kind alone signs them. A transfer of no cash whose direction the row does not state is unclassified. The trade date is the row's instant on Alberta's calendar (the old rows used the UTC date); a row whose instant is only a date keeps that date. The old rows' settlement date was a copy of the trade date, not a stated fact, and is not carried.
- **Reading the old file strictly**: a row with no id, text that is not UTF-8, or a blob where text or a number belongs stops the import with an error naming the table, column and row.
- **The journal**: `meta.journal_v2`, keyed `rt:<row id>`, the id of the row that opened the round trip. Each becomes a trade anchored on that row's transaction, with the entry written against the trade's id. The older `meta.trade_notes`, keyed by the old page's hash of a group of slices, is translated by the old app's own `migrate_legacy_notes` (which the old builds already run on start when the journal is empty) before it reaches the book; where the journal already has a note on the same trade, the journal's stands and the older one is kept beside it, orphaned, with the reason.
- **The groups**: `meta.trade_groups`, each member the old engine's slice key (the ids of a slice's buy and sell rows and its quantity). The old engine's match says which round trip each slice belongs to, and so which row opened it; that row's trade is the member.
- A key the translation cannot place (no row, or more than one) is kept as an orphaned trade with its old key and the reason, so no note or group is lost.
- `grouped_trades` is left: nothing reads it in either build (Python only creates and clears it).
- Importing the same database again changes nothing. The import returns a report: rows read, records written, transactions by kind, problems by kind, journal entries and group members attached or orphaned with each reason.

### Test data, and the public repository

The repository is public. The import's tests run on a fixture database built by the test from hand-written rows, one per row shape in the table above plus the identity cases (one ticker under two security ids, a linked CAD and USD account pair, a row with no security id, a reversal, a date-only row, a journal key and a group member that name no row), with made-up ids, names and amounts. Nothing is copied from the person's database. The check against the real books runs on copies in the session's scratch folder and is reported as counts, never committed. Recorded Wealthsimple replies arrive in stage 3, with an anonymiser designed there.

### What deliberately stays the same

The running app, its database, the page, and every current crate's behaviour. `bagholder-store` changes only to re-export the moved plumbing; the server crate gains the import binary and the key translation, and nothing the app runs calls them yet.

## Acceptance criteria

The template's Python, Go, shared-case and page checks do not apply: those builds are frozen, and nothing here touches the page or the model cases.

- [ ] Rust, from `rust/`: `cargo test -q --workspace --no-fail-fast` green, and a warning-free build.
- [ ] The boundary test fails when a new crate gains a dependency outside its column, reads the clock, or uses `f64` outside the import mapping (each checked by feeding the checker a violation); no float constructor exists on `Dec` (a `compile_fail` doctest).
- [ ] Money and decimals: a `compile_fail` doctest shows `money + money` does not compile; `checked_add` of two currencies fails; a product needing more than 28 places is an error, not a rounded value; `div_rounded` rounds as its rule names; decimal text round-trips exactly for zero, negative, 28-digit, 18-place and trailing-zero values; text with an exponent or a stray character is refused; a malformed stored decimal is an error naming its table and column.
- [ ] Migrations, each a test: a fresh file reaches the latest version; every earlier version migrates to the latest with its rows intact; a newer file is refused with both versions named; a failing migration leaves the previous version and names itself; a snapshot is taken before a pending migration on an existing file, opens, and holds the old rows; only the newest snapshot of each of the two latest versions remains, and a retried failing update keeps the older version's; a database the runner did not make is refused and left untouched; `user_version` and `schema_migrations` agree; each version's schema equals its committed `schema/v<N>.sql`.
- [ ] Identity, each a test: resolving by a strong reference finds the instrument; the same ticker under two broker security ids is two instruments; a routing reference never joins two, and two instruments sharing one both book; a connection-scoped symbol groups one connection's rows and not another's; one record naming two existing instruments, and a reference already held by another, are each a problem and merge nothing; a ticker change keeps the id with both names dated, a change back is a third name, a venue move is another, and a revised or removed record takes its sighting with it; two listings attach to one issuer; two linked broker account ids are one account.
- [ ] Records, each a test: the same payload again, and the same payload with its keys reordered and a number spelled differently, write nothing; a changed payload adds a revision and re-derives; two writers storing one record at once make one record; a record absent from a pull is untouched; only `mark_removed` removes; a mapping that cannot read a payload keeps the record with a problem; a new mapping version re-derives failed records too and returns exactly the added, changed and removed transaction ids; the same version again changes nothing; transaction ids are the same after re-deriving, and a mapping version that adds a leg before another leaves the other's id and its trade in place; the schema's `CHECK`s refuse an amount without a currency, a quantity without an instrument and a price without a quantity.
- [ ] Links and trades, each a test: `supersede` moves a trade and its journal to the new record's transaction; with several candidates it picks the earliest, then by record id; an option's opening never moves onto a closing; the same replacement delivered twice changes nothing; a chain ends on the last record; a trade with no counterpart is orphaned with its reason and its journal listed, not dropped; a removed superseding record leaves a problem and revives nothing; after a supersede the quantity and the cash are each counted once; the person's own record is superseded by a sourced one.
- [ ] The import on the fixture database: every row of the table maps to the kind, signs, quantity and fee the table states; a reversal keeps its sign; a date-only row keeps its date; an unknown shape is unclassified with a problem; the dual-id ticker, the linked pair and the row without a security id behave as above; the journal and group keys are attached, and the ones that name no row are orphaned with their reasons; the source file's bytes are unchanged; a second import changes nothing.
- [ ] The import on copies of both real databases: every activity row becomes exactly one record; every record has transactions or a named problem; every journal key is attached or orphaned with a reason; the report's counts are listed in this plan's Verification; a second run changes nothing.
- [ ] `docs/design-review.md`'s order of work records stage 1 as done, and moves the facts a figure used into stage 2's list.

## Surfaces to check beyond the diff

`rust/Cargo.toml` (workspace members), `rust/Cargo.lock` (the new crates: `rust_decimal`, `uuid`), `bagholder-store`'s re-exports and every caller of the moved plumbing, the release workflow (new crates and the new binary must not change what is built or attached), the Dockerfile's workspace copy.

## Right to refuse

Taken, twice. The handoff put the market cache's opening in this stage: an empty second store would be a table nobody writes, so it opens in stage 3 with its adapters, on the machinery built here. And it put the whole import in the book crate: the old journal and group keys can only be read by the old matching code, which the book may not depend on, so their translation sits in the server crate.

## Anti-stub self-check

- Tables with no writer in the running app until a later stage (issuers, links, the person's records) each have a test that writes and reads them, and the stage that first writes them is named above.
- The importer is exercised by tests and by a real run on both databases.
- No field written and never read: each column is read by a typed getter that a test calls.

## Verification

Run 2026-09-23 in the scratch worktree, on the committed code.

- Rust, every test binary of the workspace: 745 passed, 0 failed, 1 ignored (the Mac notifier test), 0 build warnings; the ignored test run alone: passed. New crates alone: `bagholder-core` 17 unit + 2 boundary tests, `bagholder-sqlite` 11, `bagholder-book` 8 unit + schema 6 + identity 12 + records 10 + links 11 + import 12; the server's key translation 3.
- "Every earlier version reaches the latest": with one migration there is no earlier version of the book; the test loops over none, and the migration runner's own tests hold the mechanism on a store of three versions.
- The import on copies of both real databases (`bagholder import-book`), each a fresh book, then again:

| | Python's database | Rust build's database |
|---|---|---|
| rows read → records | 6145 → 6145 new | 6123 → 6123 new |
| accounts made (Wealthsimple ids) | 29 (31 ids, two CAD/USD pairs pooled) | 29 |
| transactions | 6145, one per record | 6123, one per record |
| problems | 13 option fills with no quantity, 4 institutional transfer intents | the same |
| journal | 1 attached, 0 orphaned | none kept |
| second run | 0 new, 6145 unchanged, 0 accounts | 0 new, 6123 unchanged, 0 accounts |
| source file | byte for byte the same (`shasum -c`) | the same |

- Reviewed twice by a reviewer who wrote none of it: the plan before building (findings folded into the plan), and the code after (four serious findings fixed: a routing symbol shared by two instruments blocked a record; an option's opening could move onto a closing; the same replacement delivered twice failed; old rows without an id collapsed into one; and eleven smaller ones, each with a test).
- The page (`web/`) is untouched by this stage, so its checks were not run for it.

## Handoff

Stage 1 is done. The book is built beside the running app and nothing in the app reads it yet. Stage 2 starts from `docs/design-review.md`'s order of work: the engine on `bagholder-core`, the app opening the book (`Book::open_in`) and running `legacy_import::import` once on start, the facts a figure used, stable trade ids created by the engine, and the rederive-on-start of every registered mapping.
