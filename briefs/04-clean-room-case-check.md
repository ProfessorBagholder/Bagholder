# Brief 04: a clean-room check of the engine's cases

**Checked:** `svelte-migration` at `61baead0`.

## What was run

Independent agents worked out the expected figures of 15 engine cases: the first three of `shares.json`, `options.json`, `events.json`, `rates.json` and `positions_and_income.json`.
- They used only the written definitions (`SPEC.md`, the stage 2 plan's definitions, the stage 3a engine changes, `docs/architecture.md` §5 to §8).
- They never saw the engine's code or the cases' `expect` and `working`.
- A second agent compared each result with the case and judged every difference against the definitions.

**Result:** 11 cases agree exactly: all of shares, rates and positions and income, and the stock dividend. Four do not.

## Fix these

**1. Options: the design, and the case that breaks it.** *(Revised: an earlier version allowed "the sale closes the long" as a fix. That guesses the label is wrong, so it is withdrawn.)*

**The design.** A broker keeps one net position per contract per account: long, short or flat.
- **Four actions:** buy to open (flat or long, gets longer), sell to close (reduces a long), sell to open (flat or short, gets shorter), buy to close (reduces a short).
- **Three endings:** expiry, assignment, exercise.
- **Strategies are not separate code paths.** Long calls and puts, covered calls, cash-secured puts, naked calls and naked puts all use these same actions and endings. They differ only in what else the account holds, and that matters at assignment and exercise:
  - a covered call delivers shares held;
  - a naked call opens a short share position at the strike;
  - a short put, secured or naked, buys the shares at the strike;
  - an exercised long call buys at the strike, and an exercised long put sells shares held or opens a short.

**The conflict.** A stated effect that contradicts the position before it ("sell to open" while long, "buy to close" with no short) cannot happen with complete, correctly labelled broker records. It means an earlier row is missing or a row is mislabelled, and the record alone cannot say which. So:
- **never** two positions in one contract and account (the case in `options.json`, "a record that says it opens against an opposite position is a conflict, shown", expects a long of 1 *and* a short of 1; that is the bug);
- the contract's current position is the one the broker states for it, marked as the broker's, as the equity series already takes the broker's figure where its own can't be stated; with no broker statement, the position waits;
- the round trips the conflicting transaction touches wait with `effect-conflict` until a missing or corrected row arrives; nothing is applied by guessing which label is wrong.

Write this into the stage 2 definitions ("The ledger", Options), then fix the case and the engine.

**Cases to add**, each through its full life, with effects stated as the broker states them:
- buy to open then sell to close, in parts;
- sell to open then buy to close, in parts;
- a long put exercised, with the shares held and without;
- a naked call assigned with no shares held: a short share position opened at the strike;
- a covered call of 2 contracts assigned with only 150 of 200 shares held;
- a cash-secured put assigned;
- "buy to close" with no short: the conflict rule;
- the conflict case above, rewritten to the rule.

Make the existing "a short call assigned delivers the shares at the strike" case state whether the shares were held.

**2. Trade marks no definition names:** `split` (`events.json`, the split marker case), `continued` (`events.json`, the consolidation case) and `rolled` (`options.json`, the roll case).
- **Why it matters:** the definitions name two marks, `reward` and `basis-unknown`. `SPEC.md` §1 allows nothing beyond what it lists.
- **The fix:** remove the three, or define each one where the definitions live and say where the page shows it. Don't leave them as outputs that only the code and its own cases know about.

## Then run it over every case

Brief 01 §3 asked for one case per file to be re-derived this way. The sample found a wrong expected figure in 1 case of 15, and invented output in 3 more. That's enough to run the check over every case in every file, not a sample:
- one clean-room agent per file, blind to `expect`, `working` and `rust/`;
- one judge per file;
- the verdicts `case-wrong`, `cleanroom-wrong`, `definitions-ambiguous` and `format-only`.

Fix each `case-wrong`. Put each `definitions-ambiguous` into the definitions as a sentence; one that the owner must settle goes to the top of the plan. This is a fan-out job that fits brief 01 §3's rule, so run it as a workflow.
