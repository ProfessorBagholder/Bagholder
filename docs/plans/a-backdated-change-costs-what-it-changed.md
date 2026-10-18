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
2. **The capacity job judges a back-dated trade or fill per round trip it changed.** The probe sets the measured `divisor` (already in `Measured`) to the number of round trips whose identity the change moved, as read from the settling's own result. A change that joins 160 trades is judged per trade, against the growth rule and its time budget. A back-dated change that joins nothing (a buy and its sale on the same past day, the probe's second case) has divisor 1, and must meet the growth rule raw.
3. **A row that moves no position continues its family's match.** `continuable` gains one case: a new row with no quantity, no adjustment naming it, and not an event, expiry, assignment or exercise. It is placed into the family's kept state at its own place without walking the family again: its cash leg goes to its account, and its income to the round trip open on its day (`ledger::Continued` gains that slot). Anything else placed before the family's last walked place is still matched whole.
4. **Past days are forgotten from the change's first day, not all of them.** `apply` takes the stale day as the earliest day among the rows the change removes or adds (and among the instruments and accounts it changes, where those carry a day), rather than `Date::MIN`. A change with no dated row still forgets everything.

What stays the same: the trade definition, the joined and orphaned trades and their journal, every figure, and the book's tables.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace --locked` green, clippy clean.
- [ ] **One settling, one transaction:** for generated back-dated buys that join N later trades, the identity writes run in one transaction (counted `BEGIN` 1, not N). A trade moved by a write in between is still left for the next settling (the existing staleness test, run inside the batch).
- [ ] **Per changed trip:** the capacity job's `backdated-trade` and `backdated-fill` meet the growth rule per round trip changed, at owner depth and four times it. Their time is under the 100 ms budget per changed trip on a Pi. Their `OVER_BUDGET` entries come off.
- [ ] **No-join case:** a back-dated buy and sale on one past day (no later trade joined) meets the growth rule raw: a new probe case, `backdated-flat`.
- [ ] **Cash-only rows continue:** for generated families and generated late distributions and file rows placed anywhere in the history, the continued match equals a whole match (the `oracles` feature's own assertion in `continue_family`, run over the generated cases), and the probe's paths show the family continued, not matched whole. `backdated-record` and `backdated-file-row` meet the growth rule on allocations, and come off `OVER_BUDGET`.
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
