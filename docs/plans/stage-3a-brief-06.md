# Plan: stage 3a changed by the owner's decisions of 2026-09-24 (brief 06)

## For the owner to decide

Nothing open.

Settled (owner, 2026-09-24, `docs/decisions.md`): the equity series is Wealthsimple's stated daily account value; option prices on screen stay Cboe's delayed chains, read only when shown; each benchmark is a total return in CAD from an ETF tracking the index; 3b has no per-issuer corporate-event readers.

## Scope

Stage 3a is done (`docs/plans/stage-3a-sources.md`); this plan changes what it built to match the owner's decisions:

- **Equity** (`SPEC.md` §2, Equity and returns): each account's value and net deposits per day are the broker's stated ones; the total is their sum; returns chain-linked net of flows. The engine's own valuation (cash plus holdings at their close) goes.
- **What is read**: no past closes of holdings for the equity series, no option close recorded, no after-close option read. Past closes are read for an option's underlying on its expiry day (the expiry rule) and, from stage 5, for a chart someone opens.
- **Option prices**: Cboe's chain for a held contract, stored with its own time, fetched only when due by the rule below.
- **Benchmarks** (`SPEC.md` §2, Index; the market data table): a total return in CAD, from the ETF's closes and dividends, converted at each day's Bank of Canada rate. FRED and TMX's index levels stop.

Out of it, on purpose: the server calling the readers when due (3c), the chart reader (stage 5), 3b's owner-entered facts (3b's plan), and a computed value for an account with no broker statement (not asked for).

## The old app here

The old app draws equity from Wealthsimple's net liquidation value and compares against price-only index levels from FRED and TMX. Its equity source is right and is what `SPEC.md` already says; its benchmark is the wrong kind of return beside a total return (`docs/old-app-mistakes.md` gains the entry). Stage 2 replaced its equity with Bagholder's own valuation; the owner reversed that, so the engine returns to the broker's statement, now per account and typed.

## Open questions

- **Which ETF tracks each index.** Objective: a total return for each benchmark the page offers. Known: the page offers the S&P 500, the S&P/TSX Composite and the S&P/TSX 60 (`SPEC.md` §2); Yahoo's chart states closes, dividends and splits and already has a strict reader (`sources/src/adapters/yahoo.rs`); the oldest, largest trackers are SPY (S&P 500, USD, from 1993), XIU (S&P/TSX 60, from 1999) and XIC (S&P/TSX Capped Composite, from 2001). Chosen: SPY, XIC.TO, XIU.TO. XIC tracks the capped index, the closest listed tracker of the Composite; `SPEC.md` names it. Settled by one Yahoo read of each, the best course because it is the one read that shows the series exists over the span and states its dividends; recorded as fixtures.
- **Whether the total return computed from closes and dividends is Yahoo's adjusted close.** Objective: the brief's "confirm the series with one read each". Known: Yahoo's adjusted close multiplies the days before an ex-date by 1 − dividend ÷ the previous close, so a day's adjusted ratio is close ÷ (previous close − dividend). Settled by the same reads: a test computes the ratio from the stored closes and dividends and compares it with the reply's own `adjclose` ratio on every day. Storing the adjusted close itself is wrong: Yahoo rescales every past value at each new dividend, so a stored value would change.

## Approach

**Equity** (`engine/src/equity.rs`). `AccountEquity` keeps, per account, the broker's stated value per day and the flow of each day (the change in its stated net deposits) and the returns between consecutive stated days. The own valuation, its problem-day rule, `ValueSource` and the broker check's value difference (it compared the own value with the broker's) go. The broker check of cash and units stays. `scope.rs`'s `equity_block` and `stat/returns.rs` are unchanged in what they compute.

**Needs** (`engine/src/needs.rs`). `closes` names only an underlying on its contract's expiry day. A new `held` names each instrument held today, the ones whose price is quoted. A currency's rate is needed from the first day an amount in it is on the record, and today for a holding's live mark; USD from the oldest day for the S&P 500 tracker.

**Benchmarks.** `contract::Benchmark` names its tracker (symbol, currency). The market cache keeps, in a new migration 003, each tracker's closes as traded (`benchmark_closes`, replacing `benchmarks`) and its dividends and splits (`benchmark_events`); migration 003 also clears the index levels and their reads, which are a different series. `market::read_benchmarks` asks Yahoo for each tracker's due span with its events, the same due rule as every close. The engine (`engine/src/stat/benchmark.rs`, beside the other ratios, the one place the engine's floats live) chains each day's total return, close × split ratio ÷ (previous close − dividend), times the day's rate over the previous day's for a USD tracker, into a CAD level per day, computed when a tracker's series or the rates change. The FRED adapter, TMX's series reader and the `^GSPC` path go with their fixtures.

**Option prices** (`sources/src/options.rs`). The reader keeps its chain lookup (OCC symbol, else exact terms) and stores a held contract's quote with its own time; no close, no book write. `recorded_closes` is dropped by book migration 005, and the market no longer merges book closes. A chain is due for a shown contract when none is held; when the chain held was made in session and the market is open (09:30 to 16:30 Eastern on a weekday: options trade to 16:15, delayed fifteen minutes) or its session has settled (its final prices not held yet); or when the chain held is final and a later session is open. A contract shown for the first time is due whatever the chain's age. Every other read sends back the held chain's `Last-Modified`, so Cboe answers only with a newer chain and otherwise a bare 304 (captured 2026-09-24: `chain-BBAI-status-304.json`); the market cache keeps the chain last read per underlying (migration 003, `option_chains`). A failed read waits out the source's rest.

**The command** (`server/src/read_sources.rs`) reads the rates, the expiry closes, the payers, the benchmarks, then quotes each held listing and contract once.

**`SPEC.md`** changes in the same commit: Index (total return in CAD from SPY, XIC and XIU), the market data table (Yahoo's trackers replace FRED and TMX's series; Cboe's chains read when shown). `docs/architecture.md` §8 and §18 drop "equity is Bagholder's own".

## Acceptance criteria

- [x] `cargo test --workspace` green in `rust/`, a warning-free build, and the Mac applet test alone.
- [x] **Equity from the broker.** Engine cases: an account's series is its stated values; a day's flow is the change in stated net deposits; a day an account states nothing is not in its series; an account whose broker states no value has none (combining accounts is `scope.rs`'s, unchanged). Expected figures written by an agent that had not read the engine. No holding's close is read for it (a case with holdings and no closes has a full series).
- [x] **Needs.** A property test over every case: `closes` holds only expiry-day underlyings; `held` is every instrument with units today.
- [x] **No option close recorded.** Book migration 005 drops `recorded_closes`, schema snapshot re-blessed; nothing in the workspace writes or reads it.
- [x] **Option chain due rule.** Tests on the clock: nothing held → due; a chain made in session → due in session and after the settle; a final chain → not due overnight nor before 09:30, due at the next open; Friday's final chain stands through the weekend; a later read sends `If-Modified-Since` and a 304 keeps the prices; a contract shown first is read without it; a failure rests; past expiry → never asked.
- [x] **Benchmarks.** One Yahoo read of each of SPY, XIC.TO, XIU.TO recorded as fixtures; a test shows the engine's daily total return equals the reply's `adjclose` ratio on every day of each (to 1e-6: Yahoo writes single-precision values, which agree to 2e-7; the other dividend convention would differ by about 9e-6); a USD test shows the CAD conversion by the day's rate; migration 003 clears the old levels (test on a cache holding them).
- [x] **Removed.** No FRED adapter, no TMX series reader, no `^GSPC` path, no `DataKind::OptionClose`, and their fixtures and shapes gone.
- [x] **Real run** on copies (`bagholder read-sources`): the report asks no holding's closes, no option close; three trackers stored over the person's span; reported in Verification.
- [x] `SPEC.md`, `docs/architecture.md`, `docs/design-review.md`, `docs/old-app-mistakes.md` say the same as the code.

## Surfaces to check beyond the diff

`web/src/lib/generated/wire.ts` (no wire change expected); the book schema snapshot and the market cache's; `compare.rs` (the old store's benchmark levels no longer fit the engine's type); the stage 3a plan's Handoff (company ex-dates before 2024-05-27 now read sessions from XIC's stored closes, not `^TSX`).

## Right to refuse

Nothing refused.

## Anti-stub self-check

No definition nobody references (`ValueSource`, the own valuation, `Gap::CloseUnknown`, the broker check's value difference, `DataKind::OptionClose`, the FRED adapter and TMX's series reader all removed); no field written and never read (`FactNeeds::held` feeds the quotes; `option_chains` feeds the due rule); the suite was run and the command run on copies of the person's data, below.

## Verification

- `cargo test -q --workspace` in `rust/`: 983 passed before the tracker test, 984 with it, 0 failed; `RUSTFLAGS="-D warnings" cargo build --workspace --all-targets` clean; the Mac applet test alone: 1 passed.
- Engine cases: equity.json (5 cases) and two cases in returns_filters_checks.json, expected figures written by an agent fenced from the engine, working from `SPEC.md`. It found five things `SPEC.md` left open (when a day's flow counts, the first day's flow, a day beside one without net deposits, the day a year is measured from, a drawdown that never falls); each is now stated in `SPEC.md` §2 as the engine already computed it, and the agent re-derived from the clarified text. All pass.
- The tracker test: SPY, XIC.TO, XIU.TO, 252 sessions each, every daily ratio within 1e-6 of Yahoo's adjusted close (worst 2.1e-7).
- Real run (`bagholder read-sources` on copies in `$TMPDIR/bh-3a/b06`, the market cache and book of the 3a run, so migrations 003 and 005 ran on real files): no holding's closes asked; the three trackers stored from the person's first day, 2023-09-07, to 2026-09-23, 764 sessions each, with 13, 12 and 12 dividends; Yahoo states no close for XIC and XIU on 2026-09-22, a session, so the chain is taken over it; three chains read (BBAI and LUNR of 2026-09-24's session in progress, QNC's of 2026-09-23), nine contracts priced; `recorded_closes` gone from the book. Cboe's chains redirect to `cdn-api.cboe.com`, which the host list now names. The failures are the known ones: FTM's CAD spot price has more digits than a decimal holds, TMX does not know PLUG on the CSE, MSTY's page states a record date in 2036.

## Handoff

- The server calling these readers when due (the chain when a screen shows a contract, the trackers after each session settles) is 3c's.
- Company ex-dates before 2024-05-27: the planned fix reads the TSX's sessions from XIC's stored closes.
- Brief 05 §4, the hook in the building session's own settings, is local and not in the repository.

**Nothing left running.** The Cboe and TMX pollers of the earlier session were stopped on 2026-09-24 at 14:28 UTC; nothing has written to their folders since.
