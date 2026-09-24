# Plan: <one line: what changes and why>

A done-contract for a heavy lift (the gate in `CLAUDE.md`). Copy this file per plan; do not edit the template in place. Push the plan to `docs/plans/` before building; the reviewer's verdict comes back as the next brief. If the contract turns out to conflict with `SPEC.md`, `docs/decisions.md` or the code, stop and say so (*Right to refuse*).

## For the owner to decide

Plain words, one decision per item: the decision first, then why it matters, then the recommendation and what happens if it isn't taken. No invented terms, and no section numbers without saying what they are. "Nothing open" when there is nothing. A decision the owner makes is written into `docs/decisions.md` and pushed at once.

## Scope

One paragraph: the change, and the figures, screens or `SPEC.md` sections it touches. What is out of it, on purpose.

## The old app here

What the old app does in this area, what is wrong with it (with its entry in `docs/old-app-mistakes.md`, adding one where it is new), and what this plan does instead. For anything carried over (a rule, a structure, a formula, a default, a fallback): why it is right, shown against `SPEC.md` and `docs/architecture.md`, never because it exists or has always worked that way. What the person sees and does on screen stays as it is unless `SPEC.md` changes. At the gate, anything carried over without this answer is a finding.

## How the leading products do it

Bagholder is a trading journal: every definition, figure and behaviour this plan sets is checked first against how the leading journals (TradeZella, Tradervue, TraderSync, Edgewonk), brokers and industry standards do it, from their own documentation, each fact with its link and the date it was read. `SPEC.md`, the engine and what the app did before are what is being checked here, never the evidence. Where this plan departs from the practice found, say why. A plan without this section, or with no source link in it, fails `rust/crates/core/tests/plans.rs`.

## Open questions

Each question states three things: the objective it serves; what is already known that bears on it, or would make settling it unnecessary, and where that comes from (the code, the docs, replies already recorded, what the owner has said); and why the way chosen to settle it is the best course for that objective. "None" when there are none.

## Approach

The existing types, crates, book tables and page components this builds on, named, so a second copy of something is never written. Whether `SPEC.md` must change, and why. What deliberately stays the same.

## Acceptance criteria

Observable and binary; each names how it is verified. If any fails, it is not done; there is no partial credit.

- [ ] `cargo test --workspace` green in `rust/`, with a test for each behaviour change, and no build warnings.
- [ ] If a figure's definition or the engine changed: a case in `rust/crates/engine/tests/cases` whose expected figures were written by an agent that had not read the engine, and `cargo test -p bagholder-engine --test cases` green on it.
- [ ] If the page changed: `npm run check`, `npm test` and `npm run e2e` green in `web/`, with a browser test driving each behaviour `SPEC.md` gives it; the screenshot baselines unchanged, or changed only where `SPEC.md` changed.
- [ ] If the page changed: rendered on the Rust scratch server on a copy of the book per `SPEC.md` §7, every displayed figure traced to its field, no table overflowing at 1200 / 1340 / 1440 / 1680.
- [ ] <the specific, observable outcome this change must produce, in this project's terms>

## Surfaces to check beyond the diff

The named places a reviewer must look that the diff alone does not show: a caller, a generated file (`web/src/lib/generated/wire.ts`), a book migration and its schema snapshot, `TIMED_WAITS`, `docs/old-app-mistakes.md`.

## Right to refuse

If an acceptance criterion cannot be met, or the ask conflicts with `SPEC.md`, `docs/decisions.md` or what the code already guarantees, stop and report before building. A wrong contract built cleanly is still wrong. A required change from the gate that is disagreed with is argued here, for the owner to decide.

## Anti-stub self-check

Initial each: no definition nobody references; no field written and never read; no branch only the switch knows; no real-target run skipped: the suite was actually run, the page actually rendered on the scratch server, not asserted.

## Verification

The exact commands run and what they showed. Paste the numbers, not "passes".

## Handoff

Where this stopped (`file:line`), what is blocking, what not to redo, and the one command to resume.

**Nothing left running.** Every background process and scheduled wake-up this session started, each stopped or with the reason it must stay.
