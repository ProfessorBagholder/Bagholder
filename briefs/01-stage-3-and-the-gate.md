# Brief 01: stage 3, and how we work from here

**From:** the architecture reviewer, a separate session that reviews and writes no code.
**For:** the session building `svelte-migration` (reviewed at `bc3cb0d4`).
**Read in full before continuing stage 3.** Where this brief and `docs/architecture.md` disagree, this brief wins and the design doc is updated to match (§4).

## Summary

- Stages 1 and 2 are sound. No rework is asked of them.
- Build for the app as it is. The work now is making the current app work properly. Nothing is designed or built for anything the owner has not asked for.
- The user-facing UI and UX stay exactly as they are. Lock that in with screenshot tests before the switch rewires every figure on the page.
- At the switch, the wire moves to exact decimal text and typed ids, and the page's patches key rows by id only.
- On open, the page draws the last figures it holds at once, and changes them only when newer data arrives.
- Delegate only where it clearly pays. Two uses do here: an independent reviewer at the gate, and spec cases written clean-room.
- The gate is for heavy lifts only: the switch, each later stage plan, new features and large refactors.

## 0. The owner's decisions (2026-09-24)

- **UI and UX:** the user-facing UI and UX match today's app. What is redesigned is the engineering underneath. Nothing the person sees or does changes unless `SPEC.md` changes on purpose.
- **Priority:** make the app work properly as it is. Don't extend it beyond what the owner has asked for.
- **Pace:** no rush. Correctness over speed.
- **The three order bugs** in the running Python app (`docs/design-review.md`, "Money and data risks") wait for stage 4, by the owner's choice. Leave the frozen app as it is.
- **Real figures** from the owner's data in committed docs are acceptable to the owner.
- **On open**, the page shows the last figures it holds at once, and updates them only when newer data arrives (§2.5).
- **What the owner reads:** not plans and docs line by line. A plan puts what the owner must decide at its top, in a few lines. The rest is for this session and the gate.

## 1. The gate

**Gated** (reviewed before it is built):
- each stage plan: the rest of stage 3, then 4, 5 and 6;
- a new feature;
- a refactor across crates, or of the page's data layer;
- any change to how a figure is defined, to stored data (a migration), to the wire, to order execution, or to access.

**Not gated:** fixes, tests, and work inside an approved plan.

**How it works:**
1. Push the plan to `svelte-migration` under `docs/plans/` and carry on with work already approved. The owner points the reviewer at the plan.
2. The verdict comes back as the next numbered brief on `architecture-briefs`: **Go**, **Go with changes** (numbered, each one required), or **Stop**.
3. Apply the required changes. If you disagree with one, say why under the plan's "Right to refuse" and let the owner decide. Don't deviate silently, and don't reopen the owner's decisions.

**Decisions the reviewer can't see:** an owner decision that exists only in a local plan is invisible to the reviewer. Push the plan, or the decision, before a gate relies on it. Where a brief contradicts an owner decision it never saw, the owner's decision stands; say so and carry on.

**Stage 3 now:** it is underway, and the switch itself is gated. Push the stage 3 plan as it stands, including how the switch will be done, before starting the switch.

## 2. Stage 3: what to get right

### 2.1 Scope: the app as it is

- **Justify every abstraction by a second implementation that exists today.**
  - A source contract is justified: the market sources are many (quotes, bars, news, filings, short interest, exposure, gauges, rates, distributions).
  - A seam for execution is justified: stage 4 tests execution against a misbehaving fake broker.
  - Wealthsimple's specifics sit in one adapter module. The rest of the app calls it through the operations it uses today, and nothing more general.
- **Payout frequency:** the owner decided on 2026-09-23 that a fund's schedule comes from its fund company's own statement. Working it out from past distribution dates lags a change: when Ninepoint's funds went from monthly to twice a month, the dates would have shown monthly for about six weeks, and the income figure would have been half what it should be. That decision stands; build it as planned. An earlier version of this brief put the order the other way round because it was written without that decision.
- **Move each market source off the old crates** (`market` and `ws` still import `bagholder_model` and `bagholder_store`) the way stages 1 and 2 worked:
  1. pin the source with goldens;
  2. move it, keeping verbatim the domain logic the design review marked "keep";
  3. delete the old path in the same change.

  No source lives in two places for longer than one change.

### 2.2 The switch, and after it

The switch moves the Rust build onto the book. The owner keeps running the Python app until cutover (stage 6), so the switch is not the moment of risk for the owner; cutover is. Therefore:

- **Keep `compare-figures` running** on scratch copies of the owner's data regularly from the switch until cutover, not once. Decide cutover on that record: no difference left unattributed over several weeks of real syncs.
- **The owner's one stage 3 decision** is the table of figures that change at the switch: one row per cause, giving figure, before, after and why. Keep it short. It is the only stage 3 document the owner reads.
- **Keep rollback in view.** Stage 6 must say what happens to anything written in the new build after cutover (journal, groups, watchlist, orders, live brackets) if the owner goes back, rehearse it on scratch copies, and put it to the owner as a decision. Don't make a stage 3 choice about the book's format that would rule out an answer there.

### 2.3 The wire at the switch

The switch replaces every figure the page receives. Do it once, properly.

- **Money and quantities as exact decimal text.**
  - In the page, type them as a branded string. Money never goes through `Number()` or `parseFloat` in `web/src`.
  - Display needs no decimal library: `Intl.NumberFormat.prototype.format` accepts a string and formats the exact value it represents (MDN, `Intl.NumberFormat.prototype.format`).
  - Sorting needs a small comparator for decimal strings, with tests.
- **A figure the engine cannot state** arrives as its `Gap` and is shown as the design says. That is one of the `SPEC.md` changes at the switch.
- **Every row carries its typed id, and patches key by id only.**
  - Delete the key inference in `rust/crates/model/src/patch.rs:32` and `web/src/lib/live.ts:26` (`id, key, d, year, symbol, grade, label, date, name`).
  - The inference takes whichever field happens to be unique in a list at that moment, so a row can be identified by its symbol or its name. That is the identity-by-symbol §5 removes, and it is the same class of bug (reconciling without stable keys) that started this migration.
  - A row with no entity of its own (a month, a year) gets a stable id from what it is, assigned by the server.
- **Number the changes** on each subscription, and resync when one is missed (§13). It is cheap, and it is a matter of correctness.

### 2.4 UI parity, held by a test before the switch

The owner's parity decision needs a mechanical guard, because the switch touches every component. `web/e2e` has no screenshot assertions today.

1. **Close the open pixel item in `docs/parity.md`.** Compare the Svelte page with `ledger.html` on the same data, at 1200, 1340, 1440 and 1680 px, once, and fix what differs.
2. **Lock it.** Take Playwright `toHaveScreenshot` baselines of every tab, the trade detail and each overlay, at the four widths, on the demo book:
   - run them in one pinned environment (the CI Linux image, fixed fonts);
   - mask the clock and other live regions.
3. **Treat every diff as one of two things from then on:** an intended `SPEC.md` change, with the baseline updated in the same commit and named in its message, or a bug.

### 2.5 §13: the page shows what it has at once, then only what changed

**The owner's decision:** on open, the page draws the last figures it holds at once, and changes them only when newer data arrives. Last-known figures stay valid until something replaces them, so the page never makes the person wait to see them.

**Build it as §13 describes:**
- a browser-side store of each subscription's last state, drawn from at once on open;
- resume: the page sends the version it holds, and the server answers with only what changed since, or with "unchanged";
- changes keyed by id and numbered, where a missed number, or a version the server cannot resume from, gets the full state again;
- work done only on change, and subscriptions for what is on screen.

**Get these right:**
- **A version that survives a restart.** Base it on something the server keeps across restarts, such as the store's change counters (kept by the design review), not a count held in memory. A page holding a version the server can no longer vouch for gets the full state, never a wrong resume.
- **One store per book and wire version.** Key the store by the book's id and the wire's version, and drop it when either differs, so a release that changes the wire never has the page read old shapes.
- **No loading state over kept figures.** Nothing kept (the first open on a device) shows the per-tab skeletons as now. Kept figures show no skeleton, spinner or cross-fade.
- **Live figures arrive the same way.** A quote is a change like any other.

**Defer until measured:**
- paging long lists, only if the owner's data shows a list is too slow whole;
- per-interaction request budgets, and widening the element-touch tests into a blanket rule. Keep the tests that exist.

A test that fails whenever something grows pushes the work toward satisfying the test instead of the person. Pin known bugs (the tab underline, the heatmap leak) with targeted tests instead.

## 3. Agents: when to delegate

**The rule:** delegate only when all three hold:
- the task separates cleanly and has a clear contract;
- it gains something from a fresh context or from running in parallel;
- its result is cheap to check.

Otherwise do it in the main thread. A handoff loses context, and lost context is where mistakes get in.

**Worth it here:**

1. **Gate reviews:** a plan before it is built, and a stage's diff after.
   - Use one independent reviewer on the strongest model. Brief it with §1 to §3 of the design, the `SPEC.md` sections the stage touches, and the plan.
   - Ask only for findings that change behaviour or correctness, ranked, each with its `file:line` and a concrete failing scenario, and capped at about fifteen.
   - Split the review along seams only when a stage is large; for stage 3: sources / the Wealthsimple adapter and the switch / the wire and the page. Remove duplicate findings before acting.
   - Treat each finding as a hypothesis: reproduce it before fixing it.
   - Batch nits into the next change. They never get a cycle of their own.
2. **Clean-room spec cases.** §8 says cases are "reviewed by a person", but the owner doesn't review this session's work line by line, so the cases' independence has to come from the process.
   - Expected figures and working for new cases are written by a subagent that is given `SPEC.md`, the case format and the inputs, and is told not to read `rust/crates/engine`. The main session implements.
   - Once, for the existing cases: a clean-room agent re-derives one case from each case file. A disagreement is a finding.
   - Settle a disagreement against `SPEC.md`. A genuine ambiguity in `SPEC.md` goes to the owner as one question.
3. **Read-only surveys of the old crates**, for example every Wealthsimple-specific branch in `server`, `market` and `ws`, or every `f64` on a money path. Use Explore, ask for `file:line` answers, and verify a sample.

**Not worth it:**
- designing the switch, the wire or a stage's contracts;
- changing `core`, `book` or `engine`;
- anything about money, identity or orders. All of these need one thread that holds the whole design;
- parallel builders on crates that depend on each other;
- small fixes, test-fix loops and doc edits, where the handoff costs more than the work;
- several reviewers on one small diff, or any "review everything" prompt.

**Mechanics:**
- Every delegated prompt names the goal, the paths in scope and out of scope, the `SPEC.md` and design sections by number, the output format and a length cap.
- An agent that writes code works in its own worktree, and the main session reads the diff before taking it.
- Nothing an agent reports goes into a doc until it has been verified.

## 4. Docs: the plan of record must be true

The owner doesn't read the docs line by line, so they have to stay consistent without the owner catching drift. Fix these now:

- **The timers list.** `CLAUDE.md` on the branch and the assertion in `rust/crates/server/src/tests_misc.rs` (`test_no_wait_on_a_clock_that_is_not_accounted_for`) both point to a "Timers that remain" section of `docs/architecture.md` that the rewrite removed. The list now lives only in `TIMED_WAITS` in that test, and its notes still use the old stage numbers ("stage 6"). Point both at the list in the test, or restore the section.
- **Stale stage numbers and a superseded plan.**
  - `docs/parity.md:52` sends the pixel comparison to "stage 8", and `docs/plans/corporate-events.md` refers to "stage 9". Neither stage exists.
  - `docs/plans/corporate-events.md` is also written against the old model (`fifo.rs`, cases generated from Python) and is superseded by stage 2's engine. Mark it superseded or delete it.
- **The old migration plan.** `docs/frontend-backend-migration.md` is superseded by `docs/architecture.md`. Say so at its top.
- **Who reviews cases.** In `docs/architecture.md` §8, "reviewed by a person" should describe the process that actually runs (§3, item 2 above).
- **Status.** Status lives in one place: the order of work in `docs/design-review.md`. Plans don't restate it.

## 5. Decisions from this brief

Settled by the owner: on open, the page draws its kept figures at once and updates them only when newer data arrives (§2.5). No decision is left open.

The switch's table of figures and the rollback reach the owner with their plans.

## Verdict on what exists

**Stages 1 and 2: Go.** The calls to keep making:
- goldens pinned before each change;
- the crate boundary test;
- `compare-figures` on real data;
- `Gap` in place of a guessed number;
- the right to refuse used when the evidence called for it, as when the switch moved to stage 3.
