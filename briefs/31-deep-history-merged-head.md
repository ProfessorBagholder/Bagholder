# Brief 31: #514 at its merged head `023e0174`: the merges keep both sides; brief 30 stands, not yet

**Reviewed:** `deep-history` at `023e0174`, against master `c8ed414d` (2.5.7 with #517). Brief 30 already reviewed #514's own diff, at `1a550d63`; it is on this branch as `30-deep-history-as-built.md`. Since then the branch carries only the two merges, `d85ca2bf` (2.5.7) and `023e0174` (#517). CI on `023e0174` at the time of writing: web, audit and rust-windows green; rust, rust-macos, browser, container and capacity still running.

## Verdict

Not yet. The two merges keep both sides' intent. But #514's own code is unchanged, so every finding of brief 30 stands, and none has been addressed. One further defect, below, is on master: it came in with 2.5.7 and is not #514's.

## The merges

- **#514's diff against master is the diff brief 30 reviewed, with one line changed.** I compared, file by file, #514's diff against its old base (`e340e0d7..1a550d63`) with its diff against master now (`c8ed414d..023e0174`). The same thirty files change by the same lines. The only difference is in `figures.rs`: `pulled` calls `broker_account(&book.stated(*a)…)` without the account and without `?`, which is #515's signature, as brief 29 said.
- **Master's side is whole.** No file outside #514's own differs from master, so all of #515 and #517 is in. `pulled` wakes the scheduler with no engine before #514's closure. `state_zone` wakes after installing. Both sets of tests are kept.
- **No rebase:** both merges are merge commits.

## Brief 30's findings, at this head

All stand, at the same lines:
- **A1–A6:** the `book` crate is untouched by either merge.
- **B1–B4:** `engine.rs` and `ledger.rs` are #514's lines unchanged.
- **C1–C5:** unchanged.
- **C6:** unchanged. `pulled` still returns at `if let Err(why) = rest`, before `self.moved(moved)` and the wake.

The four that block are:
- **B1:** a one-day record across two families continues where a fresh build orders it differently.
- **A1:** every book open takes the write lock.
- **A2:** one unreadable join fails every write and every start.
- **A3:** `current_symbol` folds within a large import.

## On master, from 2.5.7: a rate arriving moves nothing on screen

Since #515, net asset value, available margin and the ticket's total put a US-dollar account in CAD at the live rate. But a rates change reports nothing that would carry the new rate to a page. `Change::Rates` (`engine.rs:761–784`) reports what `compare` finds over the snapshot. That snapshot holds the trades, positions, cash rows, payers, equity, checks, benchmarks, the broker's statements and `BookFig` (accounts, instruments, waiting, activity, today). None of these holds the live rate, and none changes for an account that holds no positions. The views re-read only on those entities (`views.rs:229–238`).

Confirmed with a throwaway engine test, on accounts A (9,000 CAD) and U (1,000 USD, cash only), when the day's rate of 1.50 arrives after the previous day's 1.40:
- the net value goes from 10,400 to 10,500 CAD;
- `apply` reports nothing moved.

The NAV tile and the ticket's position size keep 10,400 until some other change reaches the page. Where the rate was missing, they keep waiting after it has arrived. The Cash tile's conversion of a foreign cash balance had the same gap before 2.5.7.

Required, in its own PR on master:
- A rates change reports every account whose stated value, cash or buying power is in a currency whose rate moved, as `Entity::Broker`. The book and holdings views already re-read on that (`views.rs:231`, :235).
- A test: a cash-only US-dollar account, the rate arrives, and `moved` names it. A server test that the positions and book documents carry the new total.
