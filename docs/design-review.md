# The current code against the design

Five reviewers, none of whom wrote the code, each judged one part of the Rust and Svelte build on `svelte-migration` (as of 955f7a3) against `docs/architecture.md`: keep, change or rebuild. This is their verdict, merged, with the order of work that follows from it. File and line references are to that commit.

## The short of it

The migration built a sound frame and put the old app's foundations inside it.

- **Sound, kept:** the database plumbing (WAL, transactions, the connection pool, change counters); the typed routes with generated page types; the page's single request path and its element-level updates, with the tests that prove them; the HTTP transport and per-host pacing; the real rules inside the figure code (round trips, rolls, assignments, expiries) and inside the source readers (news matching, short-interest periods, fund look-through, filings); the bracket engine's step logic and its tests.
- **Rebuilt:** everything that carries the old foundations. Identity is Wealthsimple's ids and bare symbols. The record is one mutable table of already-mapped rows, rewritten in place. Money is floating point, with a constant 1.35 for a missing rate. Splits are guessed from fill prices, and option terms from symbol text. The sources read anything as data. Wealthsimple is woven through server, store and model instead of sitting behind a broker interface. Orders and brackets have no state machines or event log. Access is a loopback check. Failures are discarded in dozens of places.

## Money and data risks found, regardless of design

These are wrong today, whatever the design.

**Also in the Python app now in use (checked by hand, `python/bagholder.py` on master's code):**
1. **An order that may be live is marked failed and never checked again.** Any error on sending, a timeout included, marks the order failed (`bagholder.py` `submit_order`, lines ~4456–4463; Rust `orders/ticket.rs:648-659`). A failed order is never read back, so an order Wealthsimple did take can rest there unseen. For a bracket's exit, a retry then places a second exit under a new id.
2. **A filled order can be booked twice.** A fill is booked only once the order is fully filled (`readback.rs:399`), but two read-backs running at once can each book it: the booking and its "booked" mark are two writes, not one transaction, and no lock is shared (`readback.rs:317-318`). The next sync then links neither local row and adds Wealthsimple's own (`store/src/merge.rs:196-203`).
3. **Order handling sees only the newest 200 orders** (`store.list_orders(limit=200)`; Rust `tools.rs:172`). Each move of a trailing stop adds an order, so this fills. Past it, a waiting bracket's entry is not found and the bracket is cancelled without a word, and the refresh can fail on every pass.

**In the Rust build** (not in use yet):
- A sale from the ticket does not wait for the stop's cancel to be confirmed (`ticket.rs:687-705`; a test asserts the violation, `tests_brackets.rs:504-521`).
- A watched stop can fire while the target's cancel is unconfirmed (`brackets.rs:633-687`).
- A bracket whose watched market sell is refused is stuck in `Firing` and never retried (`brackets.rs:567-575`).
- With the container port published beyond loopback, anyone on the network can place orders (`http/mod.rs:267-295`).
- An update's rollback can leave no working copy, and never restores the book (`update.rs:448-451, 593-652`).
- Broker rows are rewritten at every start (`store/src/relabel.rs:45-136`). Bad rows are dropped or read as zero (`lenient.rs:20-23`, `base.rs:69-71`).
- Trade ids are built from dates, quantities and prices, so a revised row or a re-inferred split renames earlier trades and cuts their notes loose (`fifo.rs:123-136, 547-600`).

## Part by part

| Part | Verdict | Size |
|---|---|---|
| Instrument, issuer and account identity (§5) | Rebuild: Bagholder ids and reference tables in place of Wealthsimple ids and bare symbols | large |
| Source records, transactions, reconciliation (§6) | Rebuild: append-only records with revisions; derived, versioned transactions; links in place of rewriting | large |
| Money and quantities (§6) | Rebuild: decimals with currency, no constant rate, every currency converted | large |
| Two stores and migrations (§6, §16) | Change: split the file; numbered migrations, a snapshot before each, tests on real old databases | medium |
| Database plumbing | Keep | — |
| Engine purity and entry point (§8) | Change: today's date and zone rules as inputs; ticker tables out of the engine; typed entry point | small–medium |
| Engine's figure rules (FIFO, trades, cashflow, stats, filters) | Keep the rules; change their types and inputs to Bagholder's own transactions | medium |
| Splits, option terms, FX, equity series (§6, §8) | Rebuild: from records and contract terms; equity computed by Bagholder | medium |
| Shared cases | Rebuild the process: expectations written from `SPEC.md`, not generated; new cases for what has none | medium |
| Source adapter contract, checking, health (§9) | Rebuild on the filings code's `Provider` seed; recorded real replies as fixtures; health shown | large |
| Transport, pacing, readers' domain logic | Keep (one limiter for all hosts) | small |
| Broker interface and Wealthsimple adapter (§10) | Rebuild: connection, pull, versioned mapping, capabilities and execution behind one interface | large |
| Order and bracket state machines, event log, safety limits (§11) | Rebuild, keeping the bracket steps and tests; a fake broker that misbehaves | large |
| Ticket validation, order routes | Keep; move Wealthsimple's rules into its capabilities | small |
| Typed routes, page request path, element-level page updates | Keep | — |
| Data flow, from a change to the screen (§13) | Change. Kept: the page writes each change into the element showing it, with tests. To change: the server finds what moved by rebuilding and comparing whole views, rather than the engine reporting it; changes are not numbered, so a missed one goes unnoticed; every open loads the whole model (about 1 MB on a real book: every trade, a thousand news items) whatever tab is shown; nothing is kept between opens but a few display preferences; no read can be answered "unchanged"; quotes are read for everything held rather than what is on screen; the remaining timers are not listed or counted. Needed: engine-reported changes, numbered and typed; subscriptions per screen with paged lists; the browser-side store with versions and resume; conditional reads; the timer list; the second-open, request-budget and element tests | large |
| Access (§12) and AI agents (§14) | Rebuild: Host/Origin and write tokens; paired, scoped, revocable tokens; credentials in the keychain; one MCP server generated from the interface, replacing the filings-only one | medium |
| Failures shown (§15) | Change: per-source health on the header and cards; every discarded error dealt with; the page says when the app cannot start | medium |
| Updater and running as a service (§16) | Change: health by answer not by uptime, book snapshot and restore, signed releases, service install | medium |

The foundation (identity, the record, money) is most of the work, and everything else is built on it.

## Order of work

Each stage starts with its design reviewed by someone who did not build it, and its tests written from `SPEC.md` and real source replies.

1. **The foundation** (done: `docs/plans/stage-1-foundation.md`): identity, the book (records and their revisions, transactions, links, trades and the journal, the person's own entries as records), decimal money, numbered migrations with snapshots, and the import of an existing database. Built beside the running app; nothing reads the book until stage 2.
2. **The engine on the foundation** (done: `docs/plans/stage-2-engine.md`): every figure from the book's transactions, with stable trade ids, contract terms, corporate events and the person's adjustments as records, Bagholder's own equity series and the exact broker check, cases written from the spec, change reporting; the tables for the facts a figure used (rates, declared distributions, recorded closes, adjustments); proven beside the running app by a comparison with the old model on the person's data.
3. **Sources and brokers, and the switch**, in three parts, each with its own plan: **3a**, sources and the facts the figures use (`docs/plans/stage-3a-sources.md`); **3b**, Wealthsimple as the first broker adapter, with corporate events from their official sources; **3c**, the switch. What the three deliver: the adapter contract with checking and health; the market cache, the second store, with the adapters whose data it holds; the readers of the facts a figure uses (the Bank's rates for every currency and its holidays, contract sizes, event values, declared distributions and stated payout frequencies), strict from their first write, since a fact is written once; the broker interface with Wealthsimple as its first adapter, whose raw rows replace the imported ones, state multi-leg orders' legs, and whose account links join accounts the import kept apart, and which records when each account's activity was last read in full, the instant the broker check measures a statement against. Then the app switches to the book and the engine, with no bridge from the old store, and with it: the `SPEC.md` changes stage 2's plan lists under *Changes to `SPEC.md` at the switch*; where the date of the rate a live mark uses shows (a live mark uses the latest rate the Bank has published, as the owner confirmed); money and quantities on the wire as exact decimal text with a branded type in the page, never through a binary float; what Clear data does to the book and the journal now kept there; the home zone taken from the page's browser until the person sets one; and the Bank's series checked to reach the person's oldest day. The switch was stage 2's until its review showed the imported rows carry the old app's guesses (every option fill relabelled "to open", multi-leg orders without legs) and no strict reader yet writes the facts: switching on them would show figures built on guesses.
4. **Execution**: state machines, event logs, one gate for every exit, safety limits, tests against a misbehaving fake broker.
5. **Interface and running**: the data flow to the screen (§13), access layer, AI agents through one generated MCP server, failures shown, safe updater, service install.
6. **The page's own structure** (shared components, one overlay manager, accessibility), then cutover: the Svelte page and this build become the app, and the Python app is retired.
