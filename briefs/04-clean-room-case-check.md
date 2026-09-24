# Brief 04: a clean-room check of the engine's cases

**Checked:** `svelte-migration` at `61baead0`.

## What was run

Independent agents worked out the expected figures of 15 engine cases: the first three of `shares.json`, `options.json`, `events.json`, `rates.json` and `positions_and_income.json`.
- They used only the written definitions (`SPEC.md`, the stage 2 plan's definitions, the stage 3a engine changes, `docs/architecture.md` §5 to §8).
- They never saw the engine's code or the cases' `expect` and `working`.
- A second agent compared each result with the case and judged every difference against the definitions.

**Result:** 11 cases agree exactly: all of shares, rates and positions and income, and the stock dividend. Four do not.

## Fix these

**1. Two opposite positions in one option contract** (`options.json`, "a record that says it opens against an opposite position is a conflict, shown").
- **What the case expects:** after a buy of 1 to open, then a sale of 1 that its record calls "to open", a long of 1 *and* a short of 1 in the same contract and account. The engine passes the case, so it produces this.
- **What the definition says:** stage 2, "The ledger", Options: "A contract's position is net within an account, as every broker keeps it … an open that meets an opposite position is a gap on the transaction."
- **Why it's wrong:** a net position is never long and short at once. The case's own working says only "the contract's holdings wait", not that a short opens.
- **The fix:** apply the conflicting transaction by the net rule (the sale closes the long), or don't apply it at all. Either way the holding carries `effect-conflict`, and no opposite position is invented. Say which in the plan's definitions, then fix the case and the engine.

**2. Trade marks no definition names:** `split` (`events.json`, the split marker case), `continued` (`events.json`, the consolidation case) and `rolled` (`options.json`, the roll case).
- **Why it matters:** the definitions name two marks, `reward` and `basis-unknown`. `SPEC.md` §1 allows nothing beyond what it lists.
- **The fix:** remove the three, or define each one where the definitions live and say where the page shows it. Don't leave them as outputs that only the code and its own cases know about.

## Then run it over every case

Brief 01 §3 asked for one case per file to be re-derived this way. The sample found a wrong expected figure in 1 case of 15, and invented output in 3 more. That's enough to run the check over every case in every file, not a sample:
- one clean-room agent per file, blind to `expect`, `working` and `rust/`;
- one judge per file;
- the verdicts `case-wrong`, `cleanroom-wrong`, `definitions-ambiguous` and `format-only`.

Fix each `case-wrong`. Put each `definitions-ambiguous` into the definitions as a sentence; one that the owner must settle goes to the top of the plan. This is a fan-out job that fits brief 01 §3's rule, so run it as a workflow.
