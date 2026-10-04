# Plan: the code that guards money holds in every state it can reach

Brief 15, stage 1 ("the money"): every hole its section 2 lists, built in six named parts, one PR each, in this order. Every finding was re-checked against `master` at 4fc87ccf on 2026-10-04 by two read-only surveys, with a sample re-read here. All were confirmed: 22 as stated, 6 partly, and where they differ the line below says what the code actually does. Paths are relative to `rust/crates`.

## For the owner to decide

Nothing open. The owner took all three changes on 2026-10-04 (`docs/decisions.md`, 2026-10-04), in the form brief 19 put them:
1. A stop that can never succeed is told once and watched here; one that could succeed later is placed again when what blocks it changes; a send with no answer is repeated when the connection or sign-in is back. Never a timer.
2. A stop goes on for the shares already bought from the first partial fill and grows with each fill, and the rest of the entry is cancelled if the stop fires first.
3. Every problem with a Wealthsimple row is said in the header's existing sentence.

## Brief 19 applied (Go with changes)

Each required change, and where it lands:
1. **Refusals are classed by whether the same request can succeed later** (part A): *invalid* (wrong price step or decimal count, an order type the security does not take): told once, never resent, watched here; *depends on state* (shares committed to another order, outside the session, a price outside the band): asked again on the event that changes the answer (the working-order list or a holding changes, the session opens, the quote crosses), never on a timer; *no answer* (transport, timeout, 401/403/429): resent when the cause clears. An answer of a shape not on record is *depends on state*, with a bounded number of event-driven retries (a named constant), then watched here and said. The first release after part A shows, in its PR, every refusal answer in the order log and its class.
2. **Arming while the entry still works** (part A), three states with a test each in `core/tests/brackets.rs`: (a) the stop fires while the entry works: the entry is cancelled at once; (b) the stop's quantity follows the fills by a modify, coalesced (at most one per interval and per material change, both named constants), never a cancel and place; (c) the entry ends: the stop's quantity is checked once against the filled quantity.
3. **Recorded as the owner's decision with accurate sources:** done (`docs/decisions.md` 2026-10-04). The "departs from Interactive Brokers and Alpaca" argument is dropped: their pages are silent on a partly filled parent. `SPEC.md` *Arming*'s "or ended with a partial fill" is the sentence that changes.
4. **The signal comes after the commit** (part E): SQLite's `commit_hook` runs before the commit completes (its documented veto turns a non-zero return into a rollback); `wal_hook` runs after the commit and after the write lock is released. The book and the cache both signal from `wal_hook` (or from `atomically` after `commit()` returns). The route test runs with each write delayed, so a signal before the commit fails it.
5. **No `arbitrary_precision`** (part B): Cargo unifies it across every crate and it breaks internally tagged enums and `flatten` on numbers (serde-rs/json issue 505). Prices and quantities go out as `serde_json::value::RawValue` (feature `raw_value`) in those fields only.
6. **The import slot is taken before the body is read** (part D): a second import is refused (409, in the page's own words) before its body is accepted. The test sends twelve imports at the same instant and measures memory flat.
7. **A failed market-data read** (part B): the failure is said in the ticket either way. Superseded in part by the owner's correction of 2026-10-04 (`docs/decisions.md`): Wealthsimple takes all four order types on every stock, ETF and option, by product and not by security, so the order types never depend on a read; the per-security lookup was a session's invention and is removed. What the read still gives (the margin rate) is said when it fails.
8. **A repeat submit while the first is in flight** (part B) answers "in flight" with the same id, never a second send; the page disables Submit from the click to the answer.
9. **A refresh consumed without a reply** (part C): a refusal that came on the first try is told apart from `invalid_grant` after a lost reply; the second is said as "sign in again", never "refused". A test for each.
10. **The sign-in browser's port** (part C, moved from stage 5): a free port, or a pipe; only the endpoint the launched child names in its own `DevToolsActivePort` file is accepted; the sign-in fails if that port is taken. A test with a foreign listener on the port. The headless sign-in stays in stage 5.
11. **Snapshots are transient** (part D): a pre-migration snapshot exists only until the new version has started and answered, then is removed; a Clear while one exists clears the same kinds inside it; `prune` keeps `snapshots/` bounded.

Open question 4 is answered by the brief: **every account's return is measured between the same pair of dates.** The combined days are those on which every started account states a value. The combined return chains over consecutive such days, each account's return over exactly that interval from its own two stated values and its net deposits on those days. An account whose statements stop leaves the weighting at its last stated day, and the gap is said. GIPS' composite provisions are read before part F is built.

## Scope

The six parts below. On screen: the three lines above, if taken, and wording where a failure is told. No layout changes.

Out of scope on purpose:
- The headless sign-in (stage 5, remote access). The sign-in browser's fixed port is in part C (brief 19, change 10).
- Execution behind a broker trait, and the `ws` crate's removal (stage 2).
- Costs at your size (stage 3).
- The list of unplaced rows (decided 2026-09-30: designed first).

## The old app here

The old app had brackets with the same five-second loop and no recovery for a sale left half-done. Nothing is carried over. Where today's Rust behaviour is kept, it is kept for the reason given against `SPEC.md` and the brokers' documents below, never because it exists.

## How the leading products do it

- **Attached stops.**
  - Interactive Brokers: an attached stop "will be created, but will not be submitted until the parent order fills" ([IBKR guide, Attached Orders](https://www.ibkrguides.com/ipad/attached.htm), read 2026-10-04).
  - Alpaca: "The second and third orders won't be active until the first order is completely filled", and "if the take-profit order is partially filled, the stop-loss order will be adjusted to the remaining quantity" ([Alpaca, Orders](https://docs.alpaca.markets/docs/orders-at-alpaca), read 2026-10-04).
  - Neither page says what happens to a partly filled parent, so neither is a rule this plan departs from (brief 19). NinjaTrader's ATM strategies: when scaling into a position "all of the Stop Loss and Profit Target orders will be automatically updated to reflect the new position size" (NinjaTrader help, ATM strategies, as read by the reviewer 2026-10-04).
- **A rejected order.** Alpaca rejects a price with too many decimals with a stated code (same page). The rejection is the answer; it is shown, and the order is not resent. The one rejection on your account (2026-09-10) was exactly that case: "Limit price has too many decimal places. Max allowed: 2".
- **Repeat submissions.**
  - FIX: the client's order id "must be guaranteed [unique] within a single trading day" ([FIX 4.4, ClOrdID tag 11](https://www.onixs.biz/fix-dictionary/4.4/tagnum_11.html), read 2026-10-04).
  - Stripe: the first result is saved under the client's key, and "subsequent requests with the same key return the same result", keys kept at least 24 hours ([Stripe, Idempotent requests](https://docs.stripe.com/api/idempotent_requests), read 2026-10-04).
  - Wealthsimple's order request already carries an id the app makes (`externalId`, `server/src/orders/ticket.rs:700`). The fix is for the page to make it at Review, and for the server to answer a repeat with the first result.
- **Order windows.** Wealthsimple: limit orders "expire at market close if they can't fill within the day" unless placed good till cancelled, which lasts up to 90 days (help centre 4413542412187, read 2026-10-03). Seen on your account on 2026-10-04: an option limit buy expired at the close.

## Open questions

1. **Does Wealthsimple refuse a second create with an `externalId` it has already taken?**
   - *Objective:* a repeat must never become a second order.
   - *Known:* nothing in the replies on record; FIX and Stripe both assume the server refuses.
   - *How it is settled:* the server answers a repeat from the book before anything is sent (part B), so the answer does not change the design. It is asked once by a dry-run probe, written to a file, as the 2.0.13 and 2.0.14 probes were. A live probe could place an order, so it is not run without the owner.
2. **The source for each venue's sessions**, so brackets with nothing in flight rest until the next session.
   - *Known:* Wealthsimple states only `marketStatus` on a quote. There is no session calendar in the code; `market/src/shorts.rs:193` is weekdays only.
   - *How it is settled:* each exchange's own published hours and holiday calendar (NYSE, Nasdaq, TSX/TSXV, Cboe for options), held as data with its source named, never a constant. The quote's `marketStatus` overrides it when they differ, so an early close is followed.
3. **Can a stop resting at Wealthsimple fill outside the regular session?**
   - *Objective:* how often a resting stop is read back outside the session.
   - *Known:* not stated in the replies on record.
   - *How it is settled:* Wealthsimple's help centre article on extended hours, read through its JSON API as brief 16 did. If it says nothing, a resting exit is read back once when each session opens and closes, and on any change the order feed shows.

4. **How a combined return treats a day one account did not state.**
   - *Objective:* a gap in one account's statements never erases the other accounts' returns.
   - *Known:* the combined chain keeps only days on which every started account has a value (`engine/src/scope.rs:996-997`). A search of Sharesight's help on 2026-10-04 found nothing stating its rule for a missing day.
   - *How it is settled:* from the GIPS standards' valuation provisions and the performance documentation of Interactive Brokers' PortfolioAnalyst and Sharesight, read before part F is built. The two candidates are: carry the account's last stated value with no flow, or chain that day from the accounts that stated. The method is set in `SPEC.md` with its source. If the sources disagree, it comes to the owner as one line.

## Approach

Each part names the code it changes and the test that holds it. Every new state of a bracket is a case in `core/tests/brackets.rs` and a fake-broker test in `server/src/tests_execution.rs`.

### Part A: the bracket never leaves a position unguarded without saying so
- **The market sell never waits on a stop's retry rest.** `place()` stops gating `ExitRole::Market` on `may_retry` (`core/src/bracket.rs:601-606`, 465-473). A refused market sell is sent again as soon as the cause clears, as in decision 1.
- **Decision 1.**
  - Each refusal is classed by what Wealthsimple's answer says: `Answered` (a price, the shares, the order type: not resent, watched here); `NoAnswer` (transport, timeout, 401/403, 429: resent when the cause clears); `OutsideSession` (resent when the session opens).
  - The class comes from the answer's own code and words, read strictly against recorded replies. An answer of a shape not on record is `NoAnswer` and is said.
  - `RETRY_SECONDS` goes, and `SPEC.md` *Failures* is rewritten.
- **A halted bracket keeps guarding.** The cap stops the engine sending more, but a halted bracket still reads quotes and fires the watched market sell (`brackets.rs:345`, `core/src/bracket.rs:637`). The cap counts that sell apart, since it is one order and the last.
- **`closing-for-sale` recovers.** On start, and on every check, a bracket in `ClosingForSale` whose sale is not in flight returns to guarding with its stop placed again. A `?` error inside `sell()` drops the bracket to guarding before returning (`orders/ticket.rs:876`, 894).
- **A ticket sale ends the bracket only when it fills.**
  - Accepted is not sold (`ticket.rs:933-939`): the bracket is `Selling` while the sale rests.
  - An expired or cancelled sale puts the stop back and says so. A partial fill shrinks the bracket.
- **Arming (decision 2).** `arm()` arms on the first fill and grows with each further fill (`core/src/bracket.rs:786-790`). If the owner refuses, it stays as today.
- **A failed token refresh does not stop the watch while the access token still has life.**
  - `orders_can_run` checks a token the broker issued and its expiry, not `state.connected` (`orders/tools.rs:92-94`).
  - A bracket that cannot run says so on its card and in the header from the first missed check, with the cause.
  - A refresh token Wealthsimple refused (`server/src/session.rs:57`) is a sign-in the person must make, told at once.
- **A stop Wealthsimple cancelled is a problem told, not an outcome.** An exit cancelled without the app asking (`core/src/bracket.rs:820`) is told as `Stop cancelled at Wealthsimple · SYM`, and the position is watched here until the person acts. Today's ending is kept only when the person cancels from the panel.
- **`cancelling` is asked again.** An unclear cancel answer is read back, and resent if the broker still says open (`gate.rs:137, 150`; `bracket.rs:679, 697, 776-781`).
- **"No such order" after an unclear send waits.** For a window from the broker's own behaviour, read from the recorded replies, it is read again before the order is called failed (`core/src/order.rs:382-387`). The bracket places nothing new until then. The window is a named constant with its source.
- **One slow call stalls nothing else.** Each bracket's check runs on its own task, with an order timeout of 10-15 s and the read timeout of the adapter's own client (`brackets.rs:388-392`; today `ws/src/session.rs:647, 717` allow 90 s).
- **Brackets rest outside the session.** Between sessions, with nothing in flight, a bracket is read on the session's change and on the order feed's changes, not every five seconds (open questions 2 and 3; `brackets.rs:33, 410`).

### Part B: the ticket sends exactly what was confirmed, once
- **The id is made at Review.** The page makes the order id when it opens Review and sends it with Submit. The server writes the order under that id. A repeat with the same id returns the first answer, and the same id with different contents is refused (`ticket.rs:602-621, 700`; `gate.rs:100-129`).
- **Review shows the price that will be sent.** The tick is applied when Review is built (`orders/preview.rs:141-143, 198-205`), and a price off the grid is refused there, never rounded after confirming (`ticket.rs:655, 660, 675, 695`). This follows the owner's "the order ticket refuses what it cannot send" (2026-09-30).
- **Exact decimals on the wire.** Prices and quantities go out as decimal text in the JSON number position, as `serde_json::value::RawValue` in those fields only (brief 19, change 5), never `to_f64` (`ticket.rs:706-714`, `brackets.rs:147-155`, `gate.rs:204-207, 448-451`). The recorded web-app request remains the golden.
- **The stop is always a stop order at Wealthsimple, and the order types are Wealthsimple's by product** (owner, 2026-10-04): the per-security `stop_allowed` lookup and its watched-here branch go; a bracket's stop is watched here only when refused, cancelled at Wealthsimple, or behind a ticket sale. A failed market-data or buying-power read is said in the ticket (brief 19, change 7, as corrected). Review checks Wealthsimple's stop-limit table.
- **401/403/429 are "not sent", not "rejected"** (`gate.rs:261`, `ws/src/session.rs:729-731`).
- **Every failure on the order path is an event on the order, shown.** The log-only paths become events on their bracket's card: `Held::InFlight/Dry/NotNow`, failed read-backs, a failed book read, not armed yet, and a disallowed bracket move (`brackets.rs:177, 231-233, 257, 264, 288, 299, 390, 417`; `ticket.rs:510, 930`).

### Part C: the sign-in is real before it is used
- **Kept in memory until accepted.** A captured sign-in stays in memory and is written to `session.json` only after Wealthsimple answers the refresh with new tokens (`server/src/session.rs:68-70, 431-468, 512`). A refused capture leaves the earlier good login in place.
- **Transport failures are retried.** A failure in transport during capture is retried while the window is up; only `invalid_grant` or a 401 is a refusal (`login.rs:566, 575`).
- **One writer for "connected".** The session code alone writes it: connected means it holds a token Wealthsimple issued that has not lapsed. The pull, the token loop, `refresh_now`, `capture_tokens`, `boot_session`, `note_session_expired`, `delete_session` and `orders/tools.rs:31` report to it, and none writes the flag directly.

### Part D: the app's own switches and limits
- **One parser for every boolean switch.** It takes `1/true/yes/on` and `0/false/no/off` and refuses anything else at start (`orders/mod.rs:109`, `app.rs:401-403`, `main.rs:290`, `net/src/client.rs:501, 522`, `market/src/pdftext.rs:17`). A start line says `orders: live` or `orders: dry`. This is behaviour on the terminal, not the screen.
- **One app per data folder.** An advisory lock on the data folder is taken before anything opens (`main.rs:165-169`); a second start on the same folder is refused, naming the first.
- **The port.** A port that was asked for is taken, or the start fails (`main.rs:183-190`).
- **Body limits per route, and one import at a time.** Import rows are streamed and written in batches with a cancellation check (`http/mod.rs:80, 232`; `model.rs:257-264`; `csv_import.rs:68`).
- **A ceiling on every upstream body.** It is checked while reading, never after, including the gzip-inflated size. The model and the release archive stream to disk (`net/src/client.rs:379, 396, 433, 462, 644-650`; `update.rs:22, 339`).
- **The rollback restores the stores or refuses to roll back.** It restores the stores' snapshots taken by the update it undoes (`update.rs:471, 575, 593`; `sqlite/src/migrate.rs:164`).
  - Snapshots are transient (brief 19, change 11): one exists only until the new version has started and answered. A Clear while one exists clears the same kinds inside it (`clear.rs:164`).

### Part E: every write to the book reaches the screens
- The book and the cache signal from SQLite's `wal_hook`, after the commit (brief 19, change 4; today the cache uses `commit_hook`, `figures.rs:214`, `app.rs:161`). A journal note, a grade, a manual trade or an import then signals the stream by construction (`http/model.rs:192, 223, 257`; `entries.rs:243`).
- One server test walks every writing route and asserts a stream message follows each.

### Part F: the book's own correctness holes on the money path
- **Decision 3.** A record problem of any code reaches the header sentence (`status.rs:199`). That covers a record with no legs (`wealthsimple/src/mapping.rs:104`, `book/src/mapping.rs:54`) and a placed leg with a problem (`mapping.rs:414, 526, 539, 568`; `broker/src/csv.rs:447-448`). `Gap::RecordProblem` (`engine/src/gap.rs:80`) is built for the figures it stops.
- **A CSV re-import links by movement identity for every kind.** Account, day, kind, instrument and signed amounts; feed first, then statement. Today only exact fills and bare cash link, so dividends and other kinds double (`csv_import.rs:227-262`).
- **A combined return keeps every account's day.**
  - A day one account did not state no longer drops every account's return from the chain (`engine/src/scope.rs:987-997`), and an account whose statements stop ends its own series, not the combined one.
  - Every account's return is measured between the same pair of combined dates (open question 4, answered by brief 19). `SPEC.md` §2 changes with the method and its source, GIPS' composite provisions. The expected figures come from an agent that has not read the engine.

`SPEC.md` changes: the order ticket's *Arming* (decision 2), *Failures* (decision 1) and *Nothing left behind* (a ticket sale ends the bracket on its fill), the header's problem sentence (decision 3), and §2's combined return. Each change names its source.

What stays: the order and bracket state machines and their event log (brief 15: "what a top team would build"), the gate, the one-resting-order rule, the ninety-day re-placement, and the five-second check inside a session.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace` green and `cargo clippy --workspace --lib --bins` clean, with a test for each behaviour change.
- [ ] One test per state, with a failing case before the fix, as a `core/tests/brackets.rs` case or a fake-broker test in `server/src/tests_execution.rs`:
  - a halted bracket fires its market sell;
  - a crash between `SaleAsked` and the sale recovers on start;
  - an expired ticket sale puts the stop back;
  - a partial entry arms (or not, per decision 2);
  - a failed refresh with a live access token keeps the watch;
  - each refusal class behaves as decision 1 says, and a fired market sell is never delayed by a stop's refusal;
  - an exit cancelled at Wealthsimple is told;
  - an unclear cancel is resent;
  - a NotFound inside the window is read again;
  - a bracket whose broker call hangs does not delay another;
  - outside the session a bracket with nothing in flight makes no request.
- [ ] `POST /api/order` sent twice with one id makes one order and two identical answers (test). The same id with other contents is refused.
- [ ] The price on Review is the price on the wire (test over generated prices on every tick band). The wire carries decimal text (golden).
- [ ] A failed read beside the quote is said in the ticket, and the order types stay Wealthsimple's four (server test; browser test).
- [ ] The switch parser refuses `maybe`, and the start line names the order mode; a second app on one data folder is refused; an asked-for port in use fails the start (tests).
- [ ] An upstream body over its ceiling is refused while being read, at the ceiling plus one chunk at most (test with a generated stream). Twelve concurrent imports run one at a time with flat memory, measured and pasted.
- [ ] An update that migrated and then dies rolls back to a starting app with its stores (test). A Clear leaves snapshots that hold none of what was cleared (test).
- [ ] Every writing route produces a stream message (one test over the route list).
- [ ] Every record problem code reaches the header sentence (a test over every code the mappings can raise, generated from their code tables). A dividend re-imported from CSV into a connected account counts once (test). The combined-return engine case passes, its expected figures written blind.
- [ ] The three owner decisions recorded in `docs/decisions.md` in the PR that builds each, each with its "Held by".
- [ ] Each part's PR quotes its criteria with their evidence, plus the cost numbers for server changes (`CLAUDE.md`, Verifying a change).

## Surfaces to check beyond the diff

- `web/src/lib/generated/wire.ts` (the ticket's id at Review, refusal classes on the card).
- `TIMED_WAITS` in `server/src/tests_misc.rs`: the retry rest leaves it, the order timeout and the NotFound window enter it, each with its source.
- `docs/old-app-mistakes.md`; `SPEC.md` sections named above; `docs/decisions.md`.

## Right to refuse

If Wealthsimple's answers do not let the refusal classes be told apart strictly (decision 1), this plan stops before part A's classing, and every refusal stays told and watched, with the classing reported back. If the exchange calendars cannot be held as data with a named source (open question 2), the five-second check stays outside sessions and that line is reported.

## Anti-stub self-check

To initial when built.

## Verification

To fill when built.

## Handoff

Plan written 2026-10-04 from brief 15 §2 and the two surveys; brief 19 (Go with changes) applied, owner decisions recorded. Build order: A, B, C, D, E, F.
