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

**Benchmarks.** `contract::Benchmark` names its tracker (symbol, currency). The market cache keeps each tracker's closes as traded (the `benchmarks` table, as today) and, in a new migration 003, its dividends and splits (`benchmark_events`); migration 003 also clears the index levels and their reads, which are a different series. `market::read_benchmarks` asks Yahoo for each tracker's due span with its events, the same due rule as every close. The engine (`engine/src/benchmark.rs`) chains each day's total return, close × split ratio ÷ (previous close − dividend), times the day's rate over the previous day's for a USD tracker, into a CAD level per day, computed when a tracker's series or the rates change. The FRED adapter, TMX's series reader and the `^GSPC` path go with their fixtures.

**Option prices** (`sources/src/options.rs`). The reader keeps its chain lookup (OCC symbol, else exact terms) and stores a held contract's quote with its own time; no close, no book write. `recorded_closes` is dropped by book migration 005, and the market no longer merges book closes. A quote is due when the contract is asked for (shown), and either its market is open (09:30 to 16:30 Eastern on a weekday: options trade to 16:15, delayed fifteen minutes) and the stored chain is older than the source's stated allowance (its `cache-control` `s-maxage`), or the stored chain was made before the last session settled, whose final prices it does not hold yet. After that the final price stands until the next session.

**The command** (`server/src/read_sources.rs`) reads the rates, the expiry closes, the payers, the benchmarks, then quotes each held listing and contract once.

**`SPEC.md`** changes in the same commit: Index (total return in CAD from SPY, XIC and XIU), the market data table (Yahoo's trackers replace FRED and TMX's series; Cboe's chains read when shown). `docs/architecture.md` §8 and §18 drop "equity is Bagholder's own".

## Acceptance criteria

- [ ] `cargo test --workspace` green in `rust/`, a warning-free build, and the Mac applet test alone.
- [ ] **Equity from the broker.** Engine cases: an account's series is its stated values; a day's flow is the change in stated net deposits; two accounts sum; a day an account states nothing is not in the total. Expected figures written by an agent that had not read the engine. No holding's close is read for it (a case with holdings and no closes has a full series).
- [ ] **Needs.** A property test over every case: `closes` holds only expiry-day underlyings; `held` is every instrument with units today.
- [ ] **No option close recorded.** Book migration 005 drops `recorded_closes`, schema snapshot re-blessed; nothing in the workspace writes or reads it.
- [ ] **Option quote due rule.** Tests on the clock: in session and older than the allowance → due; within it → not; after 16:30 with a chain made in session → due once, then not until the next session's open; weekend → not; not shown → never.
- [ ] **Benchmarks.** One Yahoo read of each of SPY, XIC.TO, XIU.TO recorded as fixtures; a test shows the engine's daily total return equals the reply's `adjclose` ratio on every day of each (to 1e-9); a USD test shows the CAD conversion by the day's rate; migration 003 clears the old levels (test on a cache holding them).
- [ ] **Removed.** No FRED adapter, no TMX series reader, no `^GSPC` path, no `DataKind::OptionClose`, and their fixtures and shapes gone.
- [ ] **Real run** on copies (`bagholder read-sources`): the report asks no holding's closes, no option close; three trackers stored over the person's span; reported in Verification.
- [ ] `SPEC.md`, `docs/architecture.md`, `docs/design-review.md`, `docs/old-app-mistakes.md` say the same as the code.

## Surfaces to check beyond the diff

`web/src/lib/generated/wire.ts` (no wire change expected); the book schema snapshot and the market cache's; `compare.rs` (the old store's benchmark levels no longer fit the engine's type); the stage 3a plan's Handoff (company ex-dates before 2024-05-27 now read sessions from XIC's stored closes, not `^TSX`).

## Right to refuse

Nothing refused.

## Anti-stub self-check

## Verification

## Handoff

**Nothing left running.** The Cboe and TMX pollers of the earlier session were stopped on 2026-09-24 at 14:28 UTC; nothing has written to their folders since.
