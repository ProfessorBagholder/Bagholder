# Plan: stage 4, execution — orders and brackets as state machines with event logs in the book, one gate for every request, guards, and a misbehaving fake broker

A done-contract (`PLAN.template.md`). Stage 4 of the order of work in `docs/design-review.md`; the target is `docs/architecture.md` §11 (Execution) and §6 (the book, reconciliation by execution).

## For the owner to decide

Nothing open. (A guard against a single far-off quote and a global switch for automated orders were drafted from one line of the architecture each and dropped on 2026-09-26: neither answers anything observed or established practice for a person's own tool. The architecture's lines are changed with them.)

## Scope

Orders and brackets move from the old store into the book, each an explicit state machine whose state is what its event log says; every request to Wealthsimple leaves through one gate that records it, refuses it when orders are off, and never sends an order twice; a fill is booked at once as a provisional record that gives way to Wealthsimple's own row; nothing that follows live orders works from a list cut at a count; two guards against the app's own automation misbehaving are built (a per-bracket order cap, and no action on a quote whose own time is not current); and the whole is tested against a fake Wealthsimple that loses, repeats, reorders and refuses. The page's Orders panel and ticket stop computing any amount themselves (the last allow-list entries in `web/src/no_money_arithmetic.test.ts`). What the person sees and does stays as `SPEC.md` §6 (Orders, Brackets) describes, except the corrections listed under Approach.

Out of scope, on purpose: agents placing orders (stage 5, through the generated MCP tools, with "who asked" already recorded here so they need no second design); order routes' network access (stage 5, access layer); any second broker (the gate and the state machines are written against the broker interface, with Wealthsimple its only order adapter).

## The old app here

What exists now (survey of `rust/crates/server/src/orders/*` and `rust/crates/store/src/orders/*`, 2026-09-26), and what replaces it:

- **A send error is a final `Failed`.** A timeout, a transport error or an answer the app cannot read sets `Failed` (`orders/ticket.rs:719`), and a failed row is never read back; a bracket exit that "failed" this way is placed again after 60 s while the first may rest unseen, so two exits can rest (`docs/old-app-mistakes.md` "a failed send is never read back"). **Instead:** anything after the request left the app that is not Wealthsimple's own answer is `sent, not confirmed`; the order is read back by its own external id before anything else is sent for it; `Failed` exists only for a failure before anything left the app.
- **The HTTP client re-sends a POST once** on a dropped pooled connection (`net/src/client.rs:557`), so an order can go out twice (`old-app-mistakes.md`). **Instead:** the gate sends order mutations on a request that is never retried by the transport; a lost answer is settled by read-back.
- **Every list the engine acts on is cut at 200 orders** (`orders/tools.rs:161`, `store list_orders(200)`): past 200, a waiting bracket is cancelled "entry not found", the feed's insert hits a duplicate key and panics every refresh, and a filled order is no longer followed (`old-app-mistakes.md` "200-order limit"). **Instead:** live orders and brackets are found by state through an index, with no limit; the page's lists are paged by the reader, never by the engine.
- **A ticket Sell can leave the position with no stop and nothing sold.** The ticket ends the bracket (cancelling its stop) before the sell; if the cancel is not confirmed in 30 s, or the sell is refused, the stop is gone and nothing was sold (`orders/ticket.rs:766-799`). **Instead:** the sell is a bracket state, `closing for a sale`: the stop is cancelled, the sell goes out only on confirmation, and a sell refused or unconfirmed puts the stop back (a new `SPEC.md` line under "Nothing left behind", below).
- **Rows being sent are not counted as in flight** (`brackets.rs:25-31`), so an exit left `sending` by a crash does not block a second exit. **Instead:** `sending` and `sent, not confirmed` are in flight for every check.
- **A bracket stuck in `Firing`** when Wealthsimple rejects the market sell after accepting it (`old-app-mistakes.md`, partly fixed). **Instead:** every state has its exits written down, and "the exit order ended without a fill" is a move from every state that has an exit resting.
- **Firing and re-placing leave the position unwatched.** After a trail move, roll, adjust or expiry clears the stop id, the watched-stop check does not run until the new stop is placed (`brackets.rs:790`). **Instead:** "no stop resting at Wealthsimple" is a state in which the stop level is watched here, whatever brought it about.
- **The HTTP routes and the tick change a bracket at the same time** (`bracket_lock` guards only the tick). **Instead:** one writer: every change to an order or bracket, from a route, the tick or a read-back, is an event appended under that bracket's lock, and a transition that is not allowed from the state the log says is refused and logged.
- **No record of who asked, no history.** Columns are overwritten (`error`, `attempts`, `outcome`); the create's request is the only one kept. **Instead:** the event log below.
- **`may_retry` counts from the bracket's `updated_at`**, so any patch resets the retry wait (`brackets.rs:213`). **Instead:** the wait counts from the last refused request's event.
- **Dry mode is checked in ten places** and read once from the environment (`tools.rs:10`); a dry ticket with legs still creates a bracket that waits forever. **Instead:** one check, in the gate; a dry ticket records the ticket and creates nothing that waits on a fill that will never come.
- **Numbers read leniently** (`store/src/orders/types.rs:118 lenient_num`, `ticket.rs:457 page_num`) against the decision of 2026-09-23 ("strict edges"). **Instead:** Wealthsimple's order answers are read strictly into exact decimals; an answer that does not match is a visible failure and leaves the order `sent, not confirmed`.
- **The page works out order values and leg amounts itself** from float numbers (`web/src/lib/orders/orders.svelte.ts:120-184`, the regex deciding ×100 for options). **Instead:** the orders document carries each amount as exact decimal text from the server, like every other figure since 3c.
- **Fills reach the book only when the next pull lands**, triggered only by `Filled`: a partial fill on an order that is then cancelled or expires is not pulled for early. **Instead:** each newly filled quantity is booked at once as a provisional record (§6), and gives way to Wealthsimple's own activity row when the pull brings it.

Carried over, and why: the bracket rules of `SPEC.md` §6 (Brackets) are the owner's and are kept as they are; they become invariants of the states below, each with a test. The five-second check, the retry spacing (1, 5, 15 minutes, then hourly), GTC exits renewed within 7 days of 90, the half-percent trail step and the 1% give-back of the target are `SPEC.md`'s. The read-back by external id for a row left sending is right (it is FIX's order status request, below) and is extended to every unconfirmed state.

`docs/old-app-mistakes.md` gains the new entries above (the sell with no stop, rows sending not in flight, unwatched gaps, two writers, retry clock, dry brackets, lenient numbers) and marks the stale ones (the fill booked twice is gone since 3c; the sell-before-cancel test was renamed).

## How the leading products do it

Execution is set by brokers and exchanges, not by journals (TradeZella, Tradervue, TraderSync and Edgewonk import fills; none places orders), so the references are the FIX protocol, broker APIs and the market-access rules. All read 2026-09-26.

- **States move only on the other side's reports.** FIX's order state matrices give the next `OrdStatus` "as reported back to the buy-side via an execution report or order cancel reject message"; a pending cancel "DOES NOT INDICATE THAT THE ORDER HAS BEEN CANCELED" (FIX Trading Community, *Order State Changes*, 2023, https://www.fixtrading.org/wp-content/uploads/download-manager-files/FIX-Latest-as-of-EP284-Order-State-Changes.pdf). Interactive Brokers: `PendingSubmit` "sent but not yet confirmed", `PendingCancel` "cancel sent, not confirmed" (https://interactivebrokers.github.io/tws-api/order_submission.html). Taken: `sent, not confirmed` and `cancelling` are states that only Wealthsimple's answer or a read-back leaves.
- **A lost answer is settled by asking, never by sending again.** FIX: an order resent with the same client id returns the order's current state, not a new order; an order status request answers with the current state, or "unknown order" (same document, scenarios F.1.b, G.1). Alpaca keys orders on `client_order_id` (https://docs.alpaca.markets/reference/postorder). Taken: the external id the app already makes is written before the send; after any unclear answer the order is read back by it; nothing is resent until the read-back says Wealthsimple has no such order. Whether Wealthsimple refuses a repeated external id is not documented and is not relied on.
- **A fill that fills and a cancel that races it.** FIX scenario B.1.c: a cancel for an order that fills first is rejected; the fill wins. Taken: a cancel is a request; the order's state after it is what the read-back says, and a fill read after a cancel request is booked.
- **Fill identity.** FIX keys executions on `ExecID` and recomputes cumulative quantity on corrections (`CumQty`, `LeavesQty`). Wealthsimple's order read-back carries no execution id, only `filledQuantity` and `averageFilledPrice` (`rust/crates/ws/graphql/FetchSoOrdersExtendedOrder.graphql`). Taken, as §6 of the architecture already says for a broker with no execution id: a fill is keyed by the order and the cumulative quantity newly filled since the last booking, so reading the same state twice books nothing new.
- **Partial fills and time in force at Wealthsimple.** "Good until cancelled (up to 90 days)"; and after a partial fill "the remaining part of your order will expire at the end of the trading period, regardless of the expiry date set" (help centre, updated 2026-09-18, https://help.wealthsimple.com/hc/en-ca/articles/4413542412187). Stop orders trigger and fill in regular hours only (https://help.wealthsimple.com/hc/en-ca/articles/39734190700187). Taken: a partly filled exit that expires at the close is an expected move, and the remaining quantity is placed again GTC by the reconcile step (a test).
- **Brackets.** Alpaca's bracket exits activate only after the entry fills completely, cancel each other, and "may" both fill in fast markets; a partial take-profit fill resizes the stop (https://docs.alpaca.markets/docs/orders-at-alpaca). Interactive Brokers' OCA groups with "block" route one order at a time to rule out overfill (https://interactivebrokers.github.io/tws-api/oca.html). Wealthsimple documents no bracket or OCO. Taken: `SPEC.md`'s one-order-per-share rule is the block: the stop and the target never rest together, and the swap is a state with the stop watched here while the target rests.
- **Automated-order controls.** SEC Rule 15c3-5 asks for controls rejecting orders that "exceed appropriate price or size parameters … or that indicate duplicative orders" (https://www.law.cornell.edu/cfr/text/17/240.15c3-5); FINRA Notice 15-09 asks for a kill switch "with a minimal number of steps" and throttles on outbound message volume (https://www.finra.org/rules-guidance/notices/15-09); MiFID II RTS 6 Art. 12 and 15 ask for kill functionality that identifies who is responsible for each order, and maximum message limits (https://www.legislation.gov.uk/eur/2017/589/article/15). These bind firms, not a person's own tool; taken where they guard against Bagholder's own automation misbehaving: a per-bracket cap on orders a minute (FINRA's throttle, RTS 6's maximum message limit), no duplicative order (the in-flight rule), and "who asked" on every request (RTS 6 Art. 12). A global kill switch is for a firm running many strategies; here cancelling a bracket stops its orders, and the cap stops a runaway one. Size and count limits on what the person chooses are theirs to set and are not built (the architecture: "any limits on size or count are theirs to set, or not").

## Open questions

- **Does Wealthsimple accept a second create with the same external id?** Objective: whether a resend could double an order. Known: nothing public; the design never resends before a read-back says Wealthsimple has no record, so the answer is not needed for correctness. Settled: not asked of Wealthsimple (it would take a real order); the design does not depend on it.
- **Does Wealthsimple's order read-back ever lower `filledQuantity`** (a trade bust)? Objective: whether a provisional booking must be able to shrink. Known: FIX allows busts (`ExecType` H); Wealthsimple's schema states only cumulative figures. Settled by the design: a lower cumulative quantity is recorded as an event and shown as a failure on the order, never booked as a negative fill; the provisional record gives way to Wealthsimple's activity rows either way. A test feeds it.

## Approach

**Book tables** (book migration 013, schema snapshot `v13.sql`): `orders` (the app's external id as key; Wealthsimple's order id; account, instrument, side, type, quantities and prices as exact decimal text; role in a bracket), `order_events` and `bracket_events` (append-only: sequence, time, kind, who asked — `person`, `engine`, `agent:<name>` — the request or the answer as received, the state it moved to), `brackets` (legs and levels). An order's or a bracket's state is the fold of its events; the stored state column is a cache rebuilt from the log and checked against it by a test. The first start after the migration imports the old store's orders and brackets once, each as an `imported` event carrying the old row, with the old store's tables dropped after (the 3c pattern: snapshot first, `legacy_import`).

**Order states** (`rust/crates/broker`, a new `order` module, Wealthsimple-neutral): `draft`, `sending`, `sent, not confirmed`, `pending`, `partly filled`, `filled`, `cancelling`, `cancelled`, `expired`, `rejected`, `failed` (before anything left the app only), `dry`. Allowed moves are a table in code; every move names the event that makes it (Wealthsimple's answer, a read-back, the person's request); a test walks every row of the table and every move not in it is refused.

**Bracket states:** `waiting for the entry`, `armed` (stop resting at Wealthsimple), `stop watched` (no stop resting: during a trail move, a renewal, a re-place after expiry or refusal, or a watched-mode bracket; the level is watched here), `swapping to the target` (stop cancel sent, not confirmed), `target resting` (stop watched here), `swapping back` (target cancel sent), `firing` (market sell sent), `closing for a sale` (the ticket's Sell), `closing` (ended, cancels not all confirmed), `ended`. `SPEC.md`'s bracket rules are written as invariants over these: at most one exit resting per share; a stop level is always either resting at Wealthsimple or watched here while the bracket is live; nothing is sent from a state whose last request is unconfirmed.

**One gate** (`rust/crates/server/src/orders/gate.rs`, replacing `gql_as` for order mutations): takes an order request and who asked; refuses when orders are off (dry), when an order for the same bracket leg is in flight, or when the bracket has sent its cap this minute; appends the request event before sending; sends once, never retried by the transport; appends the answer as received, or `sent, not confirmed` for anything else. The ticket, the Orders panel's routes and the bracket engine all go through it; `bagholder_ws::session::Client::graphql` is no longer reachable for a mutation from anywhere else (a boundary test, as 3c's).

**Guards:** per bracket, at most 10 orders a minute; one more trips it: the bracket sends nothing more (its stop level watched here), and the header says so until the person acts on the bracket, because that many can only be a fault (a trail step is cancel + new, two orders). Triggers read Wealthsimple's own quote, as now, while the market is open; a quote whose own time (`quotedAsOf`) is more than 15 seconds old in the session, or a quote read that fails, is a failed read of Wealthsimple's quote: nothing is acted on from it, the header says so until the next good read (the rule every source follows since 3c), and the next five-second check reads again.

**Fills:** each read-back or feed row with a higher cumulative filled quantity books the difference as a provisional record in the book (`RecordState::Provisional`, core), at the price that makes the booked total equal Wealthsimple's stated average times its cumulative quantity (exact decimal), in the same transaction as the order's event. When the pull brings Wealthsimple's activity row for the order, it supersedes the provisional records for that order (the book's existing supersede link, `book/src/links.rs`). A provisional record is shown like any fill; the engine takes it as a transaction until it gives way.

**Page:** `web/src/lib/orders/orders.svelte.ts` stops computing values, leg amounts and leg words; the orders document carries them (decimal text and the word). The no-money-arithmetic allow-list loses its orders and ticket entries. The generated types come from the new wire structs.

**`SPEC.md` changes, each with its reason in the same commit:**
- §6 Arming, "with the ticket's time in force" → GTC (the decision of 2026-09-10, "exits rest GTC", and §6 Stop loss already say so; the line contradicts them).
- §6 Nothing left behind: the ticket's Sell is refused nothing and loses nothing: if the stop's cancel is not confirmed, or the sell is refused, the stop is placed again (the position is never left with neither).
- §6 Status: an answer the app cannot read, or none, reads `Sent · not confirmed` until the read-back settles it; `Failed` only when nothing left the app.
- §6 Failures: "There is no other throttle" → the per-bracket cap; a stale or failed quote read acts on nothing and is said in the header.
- §6 A fill reaches the book: booked at once as a provisional record that gives way to Wealthsimple's row.

**Stays the same:** every screen, card, word and flow `SPEC.md` §6 gives the ticket and the Orders panel; the Wealthsimple GraphQL operations and their recorded answers (`tests_orders_wire_golden.rs`); the five-second bracket check and the 30-second orders refresh; the pull-until-in-the-book schedule for a fill (it now replaces a provisional record instead of being the first sight of the fill).

## Acceptance criteria

- [x] `cargo test --workspace` green in `rust/`, no build warnings, with a test for each behaviour change below.
- [x] `npm run check`, `npm test` and `npm run e2e` green (e2e: CI's browser job; Chromium cannot launch in this machine's sandbox) in `web/`; every `SPEC.md` §6 behaviour of the ticket and Orders panel has a browser test, and the Orders panel and ticket have no entries left in `web/src/no_money_arithmetic.test.ts`'s allow-list.
- [x] **State tables:** a test walks every allowed move of the order and bracket tables from each state, and every move not in the tables is refused and logged without changing the state.
- [x] **The state is the log:** for every order and bracket after the whole fake-broker suite, the stored state equals the fold of its events (a test over every row).
- [x] **Who asked:** every order request in the book carries its asker (`person` from the page's routes, `engine` from the bracket engine); a test posts from each and reads the events.
- [x] **The misbehaving fake broker** (`rust/crates/server/src/tests_execution.rs`, a fake Wealthsimple that answers from its own order book, not from the app's rows), each scenario a test:
  - the create's answer lost, unreadable, or timed out after Wealthsimple accepted it → the order ends `pending` by read-back, is never `failed`, and exactly one order exists at the fake;
  - the create's answer lost when Wealthsimple did *not* accept it → read-back says no record, the order ends `failed`, and a bracket exit is placed again exactly once;
  - an order filling in three parts, the same state read twice, and two read-backs at once → the book holds exactly the filled quantity at Wealthsimple's average, in provisional records, and they give way to the activity row when the pull brings it;
  - a cancel confirmed after a fill, and a fill read after a cancel request → the fill is booked, the order ends `filled`;
  - answers out of order (a later state read before an earlier one) → the state never moves backwards; the later one stands;
  - a cumulative quantity that goes down → recorded, shown as a failure on the order, nothing booked negative;
  - 5,000 orders with a live bracket's entry and a live exit among the oldest → both followed, nothing cancelled "not found", no panic;
  - the session lapsing between the stop's cancel and the target's placement → the stop level is watched (`stop watched`), the target is placed when the session is back, and no moment passes with neither resting nor watched;
  - the target's market sell rejected after acceptance → the bracket leaves `firing` and the stop is back;
  - a partly filled GTC exit expired at the close → the rest is placed again GTC on the next check;
  - the ticket's Sell with the stop's cancel unconfirmed, and with the sell refused → the stop is resting again; the position is never left with neither.
- [x] **One gate:** no order mutation reaches `bagholder_ws` except through `orders/gate.rs` (a boundary test fed a violation to prove it fails); the gate's request is sent once (a test drops the connection after the send and sees one order at the fake).
- [x] **Dry:** with orders off, nothing leaves the gate (the fake records no request), the ticket is recorded, and no bracket is created that waits on a fill.
- [x] **Guards:** the 11th order a bracket sends within a minute is refused, the bracket sends nothing more with its stop level watched, and the header says so; a quote more than 15 seconds old by its own time, or a failed quote read, fires nothing and is said in the header until the next good read.
- [x] **No list cut at a count:** no query in `orders`, `brackets` or the gate carries a `LIMIT` except the page's own paging (a test greps the crate, fed a violation).
- [x] **Strict reading:** Wealthsimple's order answers are read into exact decimals with no lenient reader (`lenient_num`, `page_num` gone from the order path); a malformed answer is a visible failure and leaves the order `sent, not confirmed`.
- [x] **Migration:** book migration 013 with its schema snapshot; the old store's orders and brackets imported once on a copy of the owner's database, every live bracket and open order still followed after the start, the old tables dropped after a snapshot.
- [x] **Rendered:** the ticket and Orders panel on the Rust scratch server (`SPEC.md` §7) on a copy of the owner's book with orders off: every amount traced to its server field, no overflow at 1200 / 1340 / 1440 / 1680; nothing on screen that `SPEC.md` §6 does not give.
- [x] `SPEC.md`, `docs/old-app-mistakes.md` and `docs/decisions.md` changed as listed, in the same commits as the code.

## Surfaces to check beyond the diff

`rust/crates/book/migrations/013-*.sql` and `book/schema/v13.sql`; `core/src/record.rs` `RecordState` and the `source_records.state` check constraint; the engine reading provisional records (`engine` input); `net/src/client.rs` retry on POST; `server/src/main.rs` loops; `server/src/clear.rs` (Clear data refuses while a bracket is live; orders now in the book); `web/src/lib/generated/orders.ts`; `web/src/no_money_arithmetic.test.ts`; `docs/old-app-mistakes.md`; `tests_orders_wire_golden.rs` (unchanged answers).

## Right to refuse

If a `SPEC.md` bracket rule cannot be held as a state invariant as written, it is reported, not reinterpreted. If Wealthsimple's read-back cannot settle an unconfirmed create (no external-id lookup for an order in some state), the plan stops there and says so before any order path is switched.

## Anti-stub self-check

At the gate, initialled: no state in the tables that no move reaches; no event kind written and never folded; no guard only a test can trip; the fake-broker suite actually run, the page actually rendered on the scratch server with orders off.

## Verification

Departures from the approach above, each for a reason found while building:

- **A provisional fill is a record of its own source, not a record state.** `book/src/fills.rs`: each newly filled quantity is a record of the source `bagholder-fill` (keyed by the order and the cumulative quantity it brings the order to), written in the same transaction as the reading that said so; when the pull brings Wealthsimple's row for the order, that row supersedes every such record of the order through the book's existing supersede link (`Book::fills_give_way`, run after every pull). A new `RecordState` would have duplicated what the supersede link already does for CSV rows. Its value is the cash that makes the booked total equal the stated average × the cumulative quantity × the contract's size, exactly (`book/tests/fills.rs`).
- **Orders placed in Wealthsimple's own app are held in memory, not in the book** (`OrdersState::elsewhere`): they are not the app's requests and have no asker; one that leaves the feed is read back once for how it ended (a fill is told). They still act from their card: Cancel and Edit go through `gate::cancel_elsewhere` / `gate::modify_elsewhere`, sent once, never by another path.
- **The engine reads back a bracket's exit on every five-second check** while the broker may act on it, so a fill, a cancel or a change made by hand is seen at the check, as `SPEC.md` §4 has it.
- **`orderType` on Wealthsimple's read-back is the side** (`buy_quantity`), not how the order is priced: the stated price is the stop where one is stated, else the limit (`gate::read_extended`). The read-back also carries the rejection's reason and code, which end a bracket whose exit is refused for shares that are not there.
- **The old store's orders and brackets are carried at start** (`server/src/legacy_orders.rs`), each with an `imported` first event keeping the old row; a row that cannot be read stops the start, naming it, with nothing half carried and the old tables kept; orders Wealthsimple reported from its own app are left to the feed.

Commands and their numbers (2026-09-26, commit on `stage-4-execution`):

- `cd rust && cargo test -q --workspace`: 1,272 passed, 0 failed. `RUSTFLAGS="-D warnings" cargo build -q --workspace --all-targets`: clean. The applet test alone: passed.
- The state tables: `core/src/order.rs` `every_move_in_the_table_is_made_and_every_other_event_moves_nothing`; `core/src/bracket.rs` `every_move_in_the_table_is_made_and_every_other_event_is_refused_and_changes_nothing`.
- The fake broker: `server/src/tests_execution.rs` (31 tests, each scenario of the criterion by name; each ends with `Book::states_disagreeing` empty); the ports of every earlier order test: `server/src/tests_orders.rs` (52 tests); the wire golden re-blessed and read (`tests/golden/orders_wire.json`).
- One gate, no count, nothing lenient: `server/src/tests_boundary.rs` (each check fed a violation).
- Fills: `book/tests/fills.rs` (3 readings book exactly 10 at the stated average; the broker's row takes their place; nothing counted twice).
- Migration: `server/src/legacy_orders.rs` tests; on a copy of the owner's earlier database, 10 orders and 1 bracket carried, 3 Wealthsimple-app orders left to the feed, the old tables dropped after a snapshot.
- `cd web && npm run check` 0 errors; `npm test` 129 passed; `npx vite build` built.
- Rendered on the Rust scratch server (a copy of the owner's data, `BAGHOLDER_DRY_ORDERS=1`, offline): the three tabs read as §4 gives them, every amount from the server's field (a carried bracket's stop leg `Filled 5 at 1.64 · $8.19`, the target `Cancelled · $9.65`, dated when it ended); nothing clipped at 1200, 1340, 1440 or 1680 px (measured).

## Handoff

Built and verified as above; nothing left running. The browser suite runs in CI.
