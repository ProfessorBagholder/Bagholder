# Plan: a change to the record does work in proportion to the change, never to the book

## For the owner to decide

Nothing open.

## Scope

This is the second plan of brief 15's Stage 3 ("light at the owner's size"). It takes on issues #420 (an added trade does work that grows with the whole book) and #419 (an import links every row any file ever imported against the whole ledger). Both are listed over budget in `capacity.rs` (`OVER_BUDGET`).

Every write to the record reaches the figures the same way today, whichever path made it (an added trade, an import, a pull, a removed row, a link, a trade's journal):

1. `Figures::record_changed` → `Figures::pulled` (`figures.rs:411–461`) reads the whole record from the book: `engine_inputs::ledger` (`engine_inputs.rs:24–58`) reads every account, every instrument with its names and terms, every live record, every problem, every transaction, every transfer link, every trade, group and journal entry.
2. It compares that whole ledger with the engine's (`e.inputs().ledger != ledger`) and hands it over whole (`Change::Ledger`).
3. The engine matches every lot of every holding again (`Engine::rematch`, `engine.rs`; `match_lots`, `ledger.rs:2135`) and rebuilds every trade, position, cash row, payer and broker check (`Engine::derive`), then compares every figure with the one before (`Engine::compare`).
4. `settle_trades` (`figures.rs:74`) reads every trade from the book (`book.trades()`) to give new round trips their ids.

An import then links each of the file's rows to the broker's own row for the same movement (`csv_import::link`, `csv_import.rs:327–398`): every live row any file ever imported, against every transaction in the ledger, a product of the two. Every pull runs the same linking again (`broker_reads.rs:421`), whether or not it stored a row.

All of this grows with the book: on master `d6824aa7` an added trade takes 3.9 million SQLite steps at four times the owner's size against 0.94 million at the owner's size, and about 730 ms on a Pi against a 100 ms budget (the table is under Verification).

The rule both issues break is `docs/architecture.md` §13: work is done because something changed, and only that work. The places it applies, by reading the code, and all fixed here:

| Where | What it does now | What it does after |
| --- | --- | --- |
| `Figures::pulled` / `record_changed` (every write to the record) | reads and compares the whole record | reads what the book's change log names since the engine last read it, by key |
| `Engine::apply(Change::Ledger)` | matches every holding again, rebuilds every figure | matches again only the holdings the change can reach, rebuilds their figures |
| `Engine::apply(Change::Adjustments)` | the same, for a corporate event's values | the same, for the holdings the event's legs name |
| `Engine::apply(Change::Closes)` on an underlying | matches every holding again | matches again only the underlying's family |
| `Engine::apply(Change::Trades / Groups / Journal)` | rebuilds every trade and position | rebuilds the ones the change names |
| `settle_trades` | reads every trade from the book | reads the trades of the round trips it settles, by key |
| `csv_import::link` after an import or a pull | every file row ever imported × every transaction | today's rule run over the movement keys the operation touched, through an index |
| The names cache (`figures.rs:458`, `wire/build.rs:30–46`) | dropped on every change, reloaded whole | kept current from the delta |
| `entries::enter` (`entries.rs:167–171`, `:106`) | finds a transaction by scanning the ledger; finds an instrument by symbol by scanning every instrument | by key; by an index of current symbols |

Nothing on screen changes. `SPEC.md` does not change: every figure keeps its definition, and the engine's figures after any change stay exactly those of a fresh build (held below).

Out of scope, each its own issue: the day turning (`Change::Clock`) still matches everything once a day, since every contract's expiry is decided by it (not an operation that answers a person, and not in the growth rule's list); the stream step's whole views (#426); one signal waking every loop (#421); the book opened per call (#422).

## The old app here

The old app recomputed everything from its rows on every change, and the Rust build's engine carried that over: `docs/architecture.md` §13 says "a change to the record re-derives the book's figures in full, which takes milliseconds on a real book". That sentence was a guess, and it was wrong at the owner's size: the capacity job measures an added trade at far more than milliseconds on a Pi (Verification). It was also never the rule the same section sets ("only that work is done"). This plan changes that sentence in `docs/architecture.md` to the rule built here, and adds the mistake to `docs/old-app-mistakes.md` with the tests that guard it.

What is carried over, and why it is right:

- **First in, first out per account and instrument, in the record's order** (`ledger.rs`, its header): `SPEC.md` defines the lots that way. The plan does not change the match; it changes which holdings are matched.
- **A full build at a start and when the day turns**: nothing is held across a start but the book, and the day turning decides every contract's expiry, so a full match is the work those changes need.
- **The figures compared field by field to say what moved** (`Engine::compare`): the page's element-level updates read it. Kept, but run over the figures recomputed only.

## How the leading products do it

- **A trade added in a portfolio tracker updates that holding.** Sharesight files every trade under its holding ("Trades & Income" tab of the holding) and computes performance from each trade's traded date: "This is the date the buy or sell transaction occurred (and is the date used for performance and tax calculations)" ([Sharesight help, Setting up your holdings](https://help.sharesight.com/setting-up-your-holdings/), read 2026-10-12). A trade belongs to one holding; the holding is what its figures are worked out over.
- **An import matches only the rows it brings.** Tradervue flags an incoming execution as a duplicate when "another trade execution exists (from prior to the import) for the same symbol at exactly the same timestamp" ([Tradervue FAQ](https://app.tradervue.com/help/faq), read 2026-10-12): each new row is looked up against what is there, not every past row against every other.
- **Incremental view maintenance.** The database field's standard for keeping derived results current is to process the change, not the whole input: DBSP gives "efficient incremental view maintenance techniques" for rich query languages ([Budiu, McSherry, Ryzhyk, Tannen, DBSP, arXiv:2203.16684](https://arxiv.org/abs/2203.16684), read 2026-10-12). Its building block applies here directly: a computation partitioned by a key needs to be redone only for the keys a change touches.
- **Change capture in the store.** SQLite's own change capture, the session extension, works per connection and must be attached before the writes it records: "A single session object monitors changes made to a single database ... via a single sqlite3* database handle" ([SQLite, The Session Extension](https://www.sqlite.org/sessionintro.html), read 2026-10-12). The book is written by several connections (the server's, a pull's, an import's), so a session per connection would miss writes it was not attached to. A trigger fires as part of the statement that makes the write, on whichever connection ([SQLite, CREATE TRIGGER](https://www.sqlite.org/lang_createtrigger.html), read 2026-10-12), so a change log kept by triggers records every write, in the same transaction, from every writer the book has or will have. This is the change-table (outbox) form of change data capture, chosen for that reason.

## Open questions

None. The objective is that an added trade and an import do work in proportion to what they change. What settles the approach:
- the code paths in the Scope table, read in full;
- the owner's book: 7,151 live transactions on 191 instruments, 3,296 of them with no instrument (cash moved); the largest single instrument has 349 (a copy of `~/.bagholder-rust/book.db`, counted with `sqlite3` on 2026-10-12). Matching that instrument's family again is a small part of the book, but how it grows as one instrument's history grows is measured by the gate's depth pair (G), not assumed;
- the growth rule's list in `docs/architecture.md` §16, which names "an added trade's write" and "an import's linking".

## Approach

### A. The book keeps a log of what changed in the record (book migration 028)

A table `ledger_changes (seq INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, key TEXT NOT NULL)`. `AUTOINCREMENT` is required, not a style: without it SQLite gives a new row `max(rowid)+1`, which after the log is pruned to empty is 1 again, below the seq the engine has applied, and every change after it would be skipped (brief 26, 2; `notifications` already uses it, `v27.sql:315`).

On each table the engine's ledger and its adjustments are read from, `AFTER INSERT / UPDATE / DELETE` triggers append **every key whose read changes**, not only the row's own. The ledger is derived through joins (`transactions` and `trades` read their instrument as `COALESCE(j.into_id, …)`, `records.rs:609–617`, `trades.rs:285–288`; `option_terms` reads the canonical underlying, `identity.rs:423–430`; an instrument's names fall back to the person's pick and a source's description, `identity.rs:404–417`), so a row on one table can change what another key reads:

| Kind | Key | Tables whose triggers write it |
| --- | --- | --- |
| `record` | the record's id | `source_records`, `transactions`, `record_problems`, `transfer_links` (both sides' records) |
| `instrument` | the instrument's id | `instruments`, `instrument_sightings` (its names), `listings_named`, `instruments_described`, `option_terms` |
| `account` | the account's id | `accounts`, `broker_connections` (its accounts) |
| `trade` | the trade's id | `trades` |
| `group` | the group's id | `trade_groups`, `trade_group_members` |
| `journal` | the subject | `journal`, `journal_tags` |
| `adjustment` | the transaction it applies to | `adjustments`, `adjustment_legs` |

**A succession is expanded where it is written.** A row of `instrument_joins` (old and new, on insert and delete) logs an `instrument` key for both sides and for every contract whose underlying is either (through a new index `option_terms(underlying_id)`), a `record` key for every record with a transaction on either (through `transactions_instrument`), and a `trade` key for every trade anchored on either (through a new index `trades(anchor_instrument)`). `settle_successions` rewrites the joins whole (`identity.rs:707–726`), so a join or a part logs every row it re-points, in the transaction that re-points them; it does so only when the joins change, which is rare.

**The tables are found by the test, not listed by hand.** A test opens a freshly migrated book seeded with every kind of row (including an instrument no live record names, so the names' fallbacks are read), installs rusqlite's authorizer (`Connection::authorizer`, `AuthAction::Read { table_name, .. }`; rusqlite 0.32 with `hooks`, already enabled) on a fresh connection so every statement is prepared again, runs `engine_inputs::ledger` and `facts().adjustments`, and asserts each table read has INSERT, UPDATE and DELETE triggers. Run on a freshly migrated book, a later migration that rebuilds a table and drops its triggers fails it.

**Read in one snapshot, applied under the lock, the watermark only rising.** The engine's holder (`Figures`) keeps the highest `seq` it has applied. A change takes the engine's lock first, then opens one read transaction (WAL gives it one snapshot), reads `SELECT DISTINCT kind, key FROM ledger_changes WHERE seq > ?applied` and its `MAX(seq)`, reads every named thing by key in that same transaction, applies the delta, sets the watermark to that `MAX(seq)`, and only then releases the lock. Two writers racing on one record therefore apply in seq order, and a read can never be applied over a newer one: the read and the watermark come from one snapshot taken under the lock (brief 26, 3). The rows `settle_trades` writes under the lock append to the log like any other write; the next change re-reads them, applies nothing new (the engine already holds them), and moves the watermark past them.

At a start the engine is built whole, as now, inside one read transaction, with the watermark set to its `MAX(seq)`; rows at or below it are then deleted. The server is the log's only consumer; a tool that opens the book (the comparison, the capacity build) reads it whole and leaves the log alone. Clear data empties the log with the record and the engine is rebuilt.

Why a log and not the writer naming what it wrote: the record has many writers (`records.rs`, `identity.rs`, `statements.rs`, `trades.rs`, `links.rs`, `watched.rs`, `clear.rs`, the import, the pull), and more will come with another broker; a raw connection that bypasses `Book` (the demo's facts, the successions test) is caught too. A writer that forgets to name a change leaves the figures wrong with nothing to show it. A trigger cannot be forgotten, and it is written in the transaction that makes the change, so a write that rolls back leaves no entry (held by a test).

### B. The engine is told the change, keyed, and matches again only what it reaches

`Change::Ledger(Ledger)` becomes `Change::Ledger(LedgerDelta)`: for each keyed collection of the ledger, the entries put (new or revised) and the keys removed. `Change::Trades`, `Groups` and `Journal` become parts of the same delta (they are read in the same write already, `figures.rs:431–446`), so every part of `Inputs` is still changed by exactly one variant. The ledger's transactions are held by record (`BTreeMap<RecordId, Vec<Transaction>>`) with an index by transaction id, so a delta is applied by key; the 33 readers of `ledger.transactions` (engine and server) read it through an iterator in the book's order, which is what they read now.

**What a change can reach: the couplings of the match.** Every field of `Matcher` (`ledger.rs:702`) was traced to one of these (brief 26 confirms none is missed beyond those added here):
1. every account's holdings of one instrument share its coin places and its last stated price (`Matcher.places`, `Matched.last_price`);
2. a contract and its underlying (assignment, exercise, multi-leg orders tainting by underlying: `underlying_taint`);
3. a corporate event: the anchor row's instrument and every leg's `from` and `to` (`event_groups`; the anchor's `apply_event` writes the legs' books, `ledger.rs:1786–1799`, and a spin-off's parent is the person's choice, not necessarily the event row's instrument);
4. a transfer between accounts: the two sides' **instruments** as the engine reads them, as well as their accounts. The moved lots land on the receiving transaction's own instrument, and nothing keeps a link's two sides on one instrument (a revision clears a record's transactions but keeps `transfer_links`; the statements linker pairs by cash and day);
5. every holding of an account, where a multi-leg order names no instrument (`account_taint`) — and also where a row with no instrument carries `leg-unstated` among its record's problems (`ledger.rs:1309–1317`, `:840–849`): that row makes its account one family and is matched with it. The problem row decides the coupling, so a change to `record_problems` recomputes families.

The engine keeps these as a disjoint-set: a **family** is a set of instruments joined by 1–4, made one with every holding of an account where 5 applies. Transactions with no instrument and no such problem form one family per account; they move the cash totals and cash rows only. A change reaches the families of every transaction, instrument and adjustment it names, before and after the change (a revision can move a transaction from one family to another; a removed link can split one). The families are recomputed for what changed and merged or split as links come and go.

**A record's legs order as one record.** `record_rank` (`ledger.rs:690–700`), the record's legs (`by_record`) and the one-leg look-ahead (`:2214`) are over every leg of a record, and a Wealthsimple corporate action before the person enters its adjustment has legs on two instruments no coupling joins (`wealthsimple/src/mapping.rs:692–715`). So the family match computes them over the whole record, from the ledger's by-record map, while applying only the family's legs: a leg ranks and orders exactly as in a full match. A record's instruments are not made a coupling; its other legs' holdings are not changed by this leg, only its place in the order, which the whole-record rank gives (brief 26, 8, the first of its two options).

**Matching again.** `match_lots` takes the set of families to match and runs exactly as today over their transactions only, in the same order; its result replaces those families' entries in `Matched`. To make that a replacement by key, every collection in `Matched` that is a list today becomes a map keyed so no row is lost: `beyond`, `unit_disagreements`, `stock_dividends` and `unapplied` hold several rows per transaction (one per event leg, `ledger.rs:1792–1799`, `:1949–1967`), so they are keyed by (transaction, instrument), with `unapplied`'s gaps merged per transaction as today; `dust`, `arrived` and `disagreements` likewise by what they are about. The engine keeps, per family, the keys it produced. Where a list is shown in an order (the header's sentences, `status.rs`), it is read in the match's order (by day, then the match's own key), not the map's: record ids are v7 uuids, so a map's order is when a row was stored, and a back-dated import's sentence would move. Each account's cash totals (`Matched.cash`) are kept per (account, family) and summed per account, so a family's change moves only its own share.

**Deriving again.** `build_trades`, `build_positions` (which already takes an instrument), `build_cashflow`, `payer_rates`, `identify` and `broker_checks` (which already takes an account) take the families (or their trips, holdings, transactions and accounts) to build. **A saved group is rebuilt when any member's trip is**: `build_trades` pools every member trip's slices into one `TradeKey::Group` and leaves those trips out (`trades.rs:305–338`), and a group's members can be on any instrument and account (`book/src/trades.rs:408–429`, `SPEC.md` trade-15), so the engine keeps an index from trip to group, and a recomputed trip rebuilds every group holding it, with the `used` rule run over all groups in the book's order. `trades`, `positions` and `cash` stay sorted vectors, because they are the order the page is shown (trades by last day then key, `trades.rs:339`; cash rows newest first; `Portfolio.positions` and `Cashflow.rows` are indexes into them), with a map from key to place beside each for replacement. Only the recomputed entries are compared to say what moved. `Entity::Book`'s figure (accounts, instruments, what waits on the person, the count of transactions) is updated by key.

**The same for the other changes that matched everything:** `Change::Adjustments` reaches the families of the transactions the changed adjustments apply to; `Change::Closes` of an underlying reaches that underlying's family.

### C. Trade ids are settled by key

`settle_trades` reads the trades of the round trips the change produced or moved (`Book::trades_by_id`, `Book::trades_on`), not every trade, and `Identity` is updated for the recomputed families only.

### D. An import, and a pull, link by the movements they touched

The linking rule does not change (it is `SPEC.md`'s): which rows are compared does. Today `link` re-examines every live file row on every import and pull (`csv_import.rs:325`, `broker_reads.rs:422`), and that is what lets an ambiguity resolve when a competing broker row is taken, revised or removed. So the unit of work is the **movement key**, never "the rows stored":

- **Every operation yields the movement keys it touched**: of every record it stored, revised, removed or superseded, before and after (from the same change log and delta as B). A pull that only removed a duplicate candidate, or a second file that took a competitor, touches the key the ambiguous row waits on.
- **For each touched key, today's rule runs whole over that key**: the live file rows under it, in the ledger's order (account, trade day, occurred_at, record, leg), against the free broker rows under it, the feed's tier before the statement's, a broker row taking one file row only, more than one free candidate reported as ambiguous. Looking up from a stored broker row's side is not done: it would link a file row to the first of two identical fills where today it is ambiguous.
- **The index is today's three matches, not one key** (`csv_import.rs:370–371`, `:396`, `:412`): fills by (account, trade day, instrument, kind, quantity), the price compared per pair as today; other movements on an instrument by (account, kind, instrument, quantity, cash) under each of the row's days (its trade day and its settle day), so the three day pairings today accepts are all found; cash moved by (account, cash), with no kind, under each of its days. A broker row is indexed under each of its days; a file row is looked up under each of its own.
- The exclusion sets (`live_records` of the person's and the files' sources, `superseding`) are read for the keys' records, not whole.

The engine maintains the index from the same delta as B.

### E. An added trade finds what it names by key

`entries::enter` reads a transaction by its id through the ledger's index, and an instrument by symbol through an index of current symbols the engine keeps with the instruments (`held_by_symbol`, `contract_by_terms`), maintained by the same delta.

### F. The names the page reads follow the delta

`pulled` drops the cache of the broker's names on every record change (`figures.rs:458`), and the next reader reloads every instrument's and account's refs (`Names::load`, `wire/build.rs:30–46`): a read of the whole book after every change, outside every probe's window. It is kept current from the delta instead: an `instrument` key reloads that instrument's refs, an `account` key that account's.

### G. The gate measures depth as well as breadth

The ×4 capacity book grows breadth only (`sized`, `capacity.rs:243`, multiplies accounts, instruments and trades together, about four round trips per instrument at both sizes), so it cannot tell a family match linear in its rows from one that is worse. A depth pair is added beside the breadth pair: the owner's accounts and instruments, with the probed listing carrying the owner's deepest row count (349), and its ×4 four times deeper; the add-trade and import probes target the deepest family. It is judged by the growth rule like the breadth pair. Whether a family needs finer incrementality is then a number from the gate, not a claim; if the depth pair misses the rule, the plan stops there and says so (Right to refuse).

### What stays the same

What the figures are, the order the match applies transactions in, the linking rule, the brief-22 rule that a pull's rows and statements are applied in one write under one version (`Figures::pulled` keeps its shape: it reads the log instead of the whole record), and the full build at a start and when the day turns.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green; `cargo clippy --workspace --lib --bins` clean.
- [ ] **Equal to a fresh build, for every kind of change, including what is re-keyed.** A property test over generated books applies random sequences of writes through the book's own writers, reads the change log after each, and holds the engine equal to `Engine::build` from the book after every step: `Engine::differences` empty, **and `Matched` and `Identity` equal whole** (both derive `PartialEq`), since those are what B replaces by family and `differences` does not compare them. The generator writes accounts of several kinds; shares, coins, contracts on underlyings; transfers between accounts, including one whose sides are two instruments; corporate events with their adjustments, including a spin-off whose parent is not the event row's instrument; a multi-leg order naming no instrument; a no-instrument row whose record carries `leg-unstated`; a record with legs on two uncoupled instruments at one instant beside a sale of one of them; saved groups across instruments and accounts; broker statements; a succession joined and then parted by a late event row (the scenario of `book/tests/successions.rs:223–234`). The writes: insert, revise and remove a record, back-dated and today; add and remove a transfer link; set and clear an adjustment and a problem; open, move and orphan a trade; group and ungroup; a journal entry; a join and a part. Also held by the existing `each_writer_leaves_the_engine_as_a_fresh_build_would`.
- [ ] **What moved is what changed.** For the same sequences, the `Moved` an incremental apply reports equals the difference between the fresh builds before and after.
- [ ] **The order shown is kept.** For the same sequences, the presented order of trades, positions, cash rows and the header's sentences equals a fresh build's, including after a back-dated import.
- [ ] **The log misses nothing.** The authorizer test of A: every table the whole read touches has its three triggers, on a freshly migrated book. A write rolled back leaves no entry; a write through a second connection is in the log; a join and a part log every record, trade and contract they re-point.
- [ ] **The log's sequence never goes back.** A test: start, prune, one write, the figure moves; the same after Clear data.
- [ ] **Racing writers.** A test with two writers on one record (an order's read-back storing a fill while a pull supersedes it by the broker's row; a pull moving a trade from one record to another while another change reads): after both, the engine equals a fresh build and no trade is orphaned.
- [ ] **Linking equal to today's.** Today's whole-ledger `link` is kept as the reference (as `compare.rs` keeps whole builds). An oracle test over random imports and pulls (duplicates across files, prices stated at several places, cash moved with and without a kind, dividends dated on their pay day, rows stored, revised, removed and superseded, ambiguities resolved by a competitor taken or removed) holds the supersede links and the ambiguous set equal to the reference's after every step.
- [ ] **Linking reads only the rows under the keys the operation touched.** A test counts the rows `link` compares: an import of N rows, and a pull that touched K keys, compare only the rows under those keys, at any size of book.
- [ ] **No change handler reads the record whole.** Held by work counted, since the boundary scanner matches free functions only and the whole readers are `Book` methods: a one-row `record_changed` whose SQLite steps and allocations do not grow between the small book and four times it, and the same for one import of N rows.
- [ ] **The names follow the delta.** The names cache reload sits inside the add-trade and import probes' windows, so the growth rule sees it.
- [ ] **The `Capacity` workflow, breadth and depth:** `add-trade` and `import` meet the growth rule on both pairs, then their budgets (100 ms answered; 1 s for a small import, on a Pi), and are removed from `OVER_BUDGET`; #419 and #420 closed by the PR. The job's table pasted in the PR.
- [ ] **The owner's book:** a copy migrates (028); `Engine::build` before and after the migration gives the same figures (the determinism digest); an added trade and a 20-row import on the copy, timed and counted, pasted in the PR.
- [ ] `docs/architecture.md` §13's sentence on a change to the record replaced by the rule built here; `docs/old-app-mistakes.md` gains the entry.

## Surfaces to check beyond the diff

- Book migration 028 and its committed `schema/v28.sql` (the log, its triggers, the indexes `option_terms(underlying_id)` and `trades(anchor_instrument)`); `Book::clear` (`clear.rs`) empties the log with the record.
- `settle_successions` (`book/src/identity.rs:707–726`): its rewrite of the joins logs every key it re-points.
- `status.rs`: the header's sentences read in the match's order.
- Every reader of `ledger.transactions` (33 sites across `engine/src` and `server/src`: `ledger.rs`, `needs.rs`, `scope.rs`, `cashflow.rs`, `engine.rs`, `clear.rs`, `status.rs`, `entries.rs`, `csv_import.rs`, `wire/build.rs`).
- `Engine::past` builds a past day's holdings from a filtered ledger; it reads the new shape.
- `compare.rs` and `read_sources.rs` build engines whole; they keep doing so.
- The capacity probes `add-trade` and `import` (`capacity.rs:584–606`) measure the whole operation, write included; unchanged.
- `docs/decisions.md` 2026-10-09 (a pull's rows and statements in one write): its test still holds.

## Right to refuse

If matching a family alone cannot equal a full match for some coupling not listed above (the property test would show it), that coupling is added to the families, never worked around; if one makes every family one (a coupling across all instruments), the plan stops and says so before building further. If the depth pair (G) misses the growth rule, the plan stops there and brings the measurement back to the gate before designing finer incrementality.

## Anti-stub self-check

To initial at build: no delta field nobody applies; no trigger whose kind nobody reads; no family the engine keeps that the match does not use; the suite run, the capacity job run, the owner's copy migrated and measured, not asserted.

## Verification

Before the build: master `d6824aa7`, `target/release/bagholder capacity run` on this Mac (arm64), at the owner's size and four times it, 2026-10-12.

| Operation | This Mac (ms) | Pi (ms) | Budget (ms) | VM steps ×1 → ×4 | Allocations ×1 → ×4 |
| --- | --- | --- | --- | --- | --- |
| add-trade | 347.1 | 729.0 | 100 | 938,375 → 3,896,785 (4.15×) | 814,754 → 3,220,728 (3.95×) |
| import (20 rows) | 638.8 | 1,341.5 | 1,000 | 1,343,091 → 5,503,460 (4.10×) | 841,420 → 3,288,393 (3.91×) |

Both grow as the book does (the growth rule allows under 2×), and both miss their wall-time budgets on a Pi. The step and allocation counts do not depend on the machine (`docs/architecture.md` §16); the milliseconds are this Mac's, scaled by `PI_SLOWDOWN`. The same run on this Mac also reported `startup` and `brokers` over budget, while the Linux runner passes both on the same commit; neither is this plan's, and the build's PR reads the runner's table for them.

## Handoff

Not started; brief 26 (Go with changes) applied. To resume: build A first (the log, its triggers and the authorizer test), then G (so every later step is measured on depth too), then B behind the equality property test, then C–F.

**Nothing left running.**
