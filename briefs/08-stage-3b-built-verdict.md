# Brief 08: stage 3b as built, and the trade change

**Reviewed:** `svelte-migration` at `3d697f4d`. Every finding was checked against the code, then by an independent reviewer.

**Stage 3b: accepted.** Make the three fixes below before 3c starts. No further review is needed. **The trade change: accepted as built.**

## Owner decision (2026-09-25): managed accounts

Trades Wealthsimple makes in a managed account count in holdings, cash, income, account value and the broker check, but make no round trips. They are not in the Trades list, the journal, or any trade statistic. Record it in `docs/decisions.md`.

## Fixes

**1. One unreadable row must not stop its account.**
- `wealthsimple/src/adapter.rs:278-279` returns early. A row with a status the web app doesn't list, an unreadable `occurredAt`, or a field `assemble::needs` requires fails the whole account: nothing is stored and `activity_read_at` never advances.
- Keep the row as a record with a problem, treat it as not final (read again next pull), and read the rest of the account.
- A row with no `canonicalId` can't be keyed: the account's read is incomplete, named.
- Add a test in `tests/pull.rs`.

**2. Removing rows Wealthsimple no longer lists: keep it, and guard it.**
- It removes only over spans read completely (`broker/src/pull.rs:134-139`). That's the right way to mirror a source readable only by date range.
- **List each removed record in the output, not just a count.** `mark_removed`'s result is discarded today (`pull.rs:186-189`).
- **Guard:** a pull that would remove a completed row older than the rows still being re-read, or more than a few rows in one account, removes nothing for that account and reports the read as suspect.
- **Fix the plan:** line 120 still says rows are never removed for being absent.

**3. A move with no stated amount.**
- A stated amount (the transfer's detail) always wins. Today the inference runs first (`wealthsimple/src/mapping.rs:778` before `:786`).
- The inference searches net deposits with no end date and takes the first opposite pair (`mapping.rs:855-885`, `adapter.rs:152-156`). It can latch onto a later transfer between the same two accounts.
- Bound it to the move's own settlement days, and require exactly one mirrored day. Otherwise the cash is unstated, named.
