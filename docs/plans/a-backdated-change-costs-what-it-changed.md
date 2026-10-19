# Plan: a back-dated change costs what it changed (#527)

## For the owner to decide

Nothing open. Every part below is either a correctness rule the trade definition already settles, or how the capacity job measures it.

## Scope

A change dated in the middle of a holding's history is over the growth rule today (`OVER_BUDGET` in `rust/crates/server/src/capacity.rs`, four operations, issue #527). This plan measures where the work goes, keeps the work a change causes, and removes the work it does not. No figure, screen, wire type or stored table changes. Out of scope: the `feed-step` entry (#426).

## The old app here

The old app re-derived everything on any change. Nothing is carried over.

## What the measurement shows

Each statement's SQLite steps were tallied inside the capacity probe's window on master `af9a726a`. The tally is a temporary local hook in `bagholder_sqlite::work`, not committed. It compares one listing 80 round trips deep (159 rows) with one 320 deep (639 rows), the test sizes `capacity.rs` already uses.

| Operation | Steps | Allocations | Where the growth is |
|---|---|---|---|
| `backdated-trade` | 16,855 → 56,059 | 57,713 → 217,028 | the book: one `UPDATE trades SET anchor_record = NULL … orphaned_reason` per later round trip (39 → 160 runs), with its trade read, journal read and change-log triggers, **each in its own transaction** (`BEGIN IMMEDIATE`/`COMMIT` 40 → 161 runs) |
| `backdated-fill` | 17,002 → 56,084 | 59,073 → 218,682 | the same |
| `backdated-record` (a late distribution) | 4,331 → 4,533 | 36,745 → 128,898 | steps flat; allocations in the engine |
| `backdated-file-row` | 7,043 → 7,044 | 15,895 → 34,089 | steps flat; allocations in the engine |

**What that means.**
- **A back-dated buy or fill** of one unit that is never sold keeps the position open from that day on. A round trip runs from flat to flat (`SPEC.md` §2, Trade), so every later round trip joins the one that day opens. The engine names each joined trade (`Identity::joined`), and the book orphans it with its journal kept for the person to re-attach (`Book::orphan_joined`). That is right, and it is work the change caused: one identity write per round trip the change joined. The waste is around it: each write is its own transaction, and each reads its trade again (`anchored_as`).
- **A late distribution or file row** moves no position. It changes no lot match and no round trip's composition, yet its instrument's family is matched whole (`Engine::continuable` refuses any row placed before the family's last walked place). Every later day's holdings are also forgotten (`Engine::apply`: any ledger change sets the stale day to `Date::MIN`).

## How the leading products do it

- **Tradervue** groups executions into trades by position: a new trade starts when the side changes, and splits are possible only where the position is flat. Its trade view shows the running position after each execution ([Split & Merge Trades](https://help.tradervue.com/article/3480-split-merge-trades); [Tradervue blog on splitting at flat](https://blog.tradervue.com/?p=1240), read 2026-10-18). An execution added in the past therefore changes every later trade it keeps open, as here.
- Neither Tradervue's help nor TraderSync's says how much work a recomputation does; the cost rule is this app's own (`docs/architecture.md` §13 and §16): work follows what changed.

## Open questions

None. The measurement above settles where the work is. Each change below is held by a test over generated inputs.

## Approach

1. **The identity writes of one settling are one transaction.** `Figures` settles the engine's identity changes (`settle_trades`, `server/src/figures.rs`). It applies every `orphan_joined`, `orphan_unclaimed` and `move_trade` the engine named in one `Book::atomically`, which nests. Each call's "anchored as the engine saw it" check stays. It reads from the same transaction, and a trade moved by a write in between is still left for the next settling. Steps per joined trade fall by the transaction's own share and by the names settled at each commit (`Book::atomically` settles them once per transaction). The count of writes stays proportional to the round trips joined, which is the change's own reach.
2. **The capacity job judges a back-dated trade or fill's work per round trip it changed, never its time** (brief 57, 1 and 2). The probe reads the trades the settling changed (`figures::settled`, counted as each write is done) into a new field, `Measured::changed`. The growth rule divides steps, allocations and bytes read by it. Wall time is judged on the whole change, against §16's rows as they stand. `changed` is not the existing `divisor`, which divides a cadence's time budget by accounts or holdings; the two are kept apart. A back-dated buy and its sale on a past day the holding is flat (`backdated-flat`, one file of two rows, a round trip of its own) changes no other trip and is judged raw. If `backdated-trade`'s time still misses at depth, its `Depth/Time` entry stays on `OVER_BUDGET`.
3. **A row the walk passes over continues its family's match, wherever it is dated.** The match walks past a row naming no instrument, and past a distribution no adjustment names (`ledger::Matcher::apply` returns early for both): it moves no lot, so the match after it is the match before it with its cash. `continuable` places a record whose every leg is such a row even before the family's last walked place (`Engine::walk_passes_over`). A distribution with an adjustment (a return of capital) moves the lots and stays matched whole.
4. **Past days are forgotten from the change's earliest date, not all of them** (brief 57, 3). A past day is worked out from the rows dated on or before it (`Engine::past`). A change made of rows alone forgets the days from the earliest trade or settlement date of any row it removes, adds or links (`first_day_touched`). A change to anything else a past day reads (an account, an instrument, a trade, a group, the journal, an adjustment) forgets every day, as now. A row's ex-date and pay date are not fields of the row: a distribution's own row is dated, and its payer's dates are facts, a change of their own.

What stays the same: the trade definition, the joined and orphaned trades and their journal, every figure, and the book's tables.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace --locked` green, clippy clean.
- [ ] **One settling, one transaction:** for generated back-dated buys that join N later trades, the identity writes run in one transaction (counted `BEGIN` 1, not N). A trade moved by a write in between is still left for the next settling (the existing staleness test, run inside the batch).
- [ ] **Per changed trip, checked:** the trades the settling changed equal the trades whose identity differs in the book before and after, read whole, for generated back-dated buys and fills at generated depths. `backdated-trade` and `backdated-fill` meet the growth rule per changed trip, and their steps and allocations entries come off `OVER_BUDGET`. Their time is judged whole: an entry by time stays listed while it misses.
- [ ] **Time is never divided:** a test holds that a slow change is over its time budget whatever it changed.
- [ ] **No-join case:** a back-dated buy and sale on one past day (no later trade joined) meets the growth rule raw: a new probe case, `backdated-flat`.
- [ ] **Cash-only rows continue:** for generated families and late rows at the boundaries (a distribution on a round trip's open day, on its close day, one paid with no trip open, cash in another currency than the account's), placed before what the family matched, the continued match equals a whole match (the `oracles` feature's own assertion in `continue_family`, run over the generated cases), and the probe's paths show the family continued, not matched whole. `backdated-record` and `backdated-file-row` meet the growth rule on allocations, and come off `OVER_BUDGET`.
- [ ] **Past days:** for generated changes dated D, past holdings before D are kept and equal a fresh computation; from D on they are recomputed. A test over every day of a generated book.
- [ ] **Figures unchanged:** the engine's case suite and the book's oracles (`cargo test -p bagholder-book --features oracles`) green, unchanged.
- [ ] **Capacity**, the full job on CI: its table pasted in the PR, the four entries off the list.

## Surfaces to check beyond the diff

- `rust/crates/server/src/capacity.rs`: `OVER_BUDGET`, the new probe case, the divisor read from the settling, and the test holding `docs/architecture.md` §16's table to the constants.
- `docs/architecture.md` §16: the per-changed-trip measure, stated with its reason.
- `rust/crates/engine/src/ledger.rs` (`Continued`), `engine.rs` (`continuable`, `apply`), and `rust/crates/book/src/trades.rs` (the orphaning calls inside one transaction).

## Right to refuse

Stop and report before building if:
- a cash-only row turns out to change a lot match (for instance, a distribution the engine reads as a return of capital without an adjustment). That row would then be a position row, and step 3 would narrow to rows proven not to.
- the settling's identity writes cannot share a transaction without holding the book's write lock longer than a pull's write waits (`busy_timeout`). Step 1 would then batch in bounded groups, with the bound's source stated.

## Anti-stub self-check

*Initialled when built.*

## Verification

*Filled when built.*

## Handoff

*Filled when built.*

**Nothing left running.** *Filled when built.*
