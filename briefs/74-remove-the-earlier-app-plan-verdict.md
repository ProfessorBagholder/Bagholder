# Brief 74: Verdict on plan #573 (nothing of the earlier app kept): Go with four changes

## Verdict

**Go with four changes.** This is the right design, and the owner's: nothing reads or writes the old file, and the old-format code goes with it. Before deleting the imported rows, the plan makes sure the one move they still carry is read from Wealthsimple. That's the right order, and it uses the right source.

What the plan doesn't yet protect is the person's own data that came across with the import. A trade is anchored on a record (`trades.anchor_record`, `book/src/trades.rs:47`), and its journal hangs on the trade. The migration's test checks units and cash only, so a trade still anchored on an imported record would be orphaned, and its thesis, grade and tags with it, while every figure still matched.

## Required changes

### 1. The person's own data survives the migration, and a test says so

The migration deletes imported records only when nothing of the person's hangs on them:
- **no trade's `anchor_record`** is an imported record. A trade the engine would re-anchor on the record that superseded it is re-anchored first, through the existing path, and never left orphaned;
- **journal entries, grades and tags:** the same count before and after, each on the same trade id;
- **carried orders and brackets, notification history and the watchlist** are the book's own rows now, and nothing in the migration's delete reaches them. The plan names their tables as untouched.

If any of these would break, the migration refuses, as it does for a `live` row, and says which in words.

*Test,* on a book holding superseded imported rows with trades, journal entries, carried orders and notifications on them:
- units and cash unchanged;
- every trade id still anchored;
- every journal entry on its trade;
- the order, notification and watchlist counts unchanged.

### 2. A conditional step isn't a schema migration

A migration that runs only on some books leaves the schema version saying nothing about whether the rows are gone. Make it the book's own upkeep step, recorded in the book once done (`settings`, as D18 recorded its carries):
- it runs at start, with no timer;
- it's tried again at each start until it's done.

The schema migration is then only what is the same on every book:
- dropping `trades.legacy_key`;
- dropping any table that only imported rows used. This runs after the upkeep step has finished, or in a later release.

### 3. The source scan matches identifiers, not word stems

`carry_` and `legacy_` would also match words that have nothing to do with the old file, and that pushes renames where none is needed. Scan for:
- the exact identifiers: `bagholder.db`, `OLD_FILE`, `bagholder-import`, `OldActivity`, `relabel::`, `legacy_key`, and each deleted function's name;
- any `bagholder_store` path.

`carry_told_by_listing` is renamed for what it does when it moves into `notify.rs`, not only to get past the scan.

### 4. `explain-move`: the owner's step, done once, then deleted

The owner runs it once, and the PR states what it printed: kinds, presence and signs only, as planned. The command is deleted in Part 2, since it exists only for this move. Say so in the plan, so it isn't left behind as a tool nobody uses.

## Not changed

- Part 1's three causes, each with its fix from Wealthsimple's own rows, tested with made-up figures.
- The demo first, as its own PR.
- The store crate removed, with the cache SQL moved into `bagholder-sources`.
- The files-opened test.
- The old file left untouched for the owner to delete.
