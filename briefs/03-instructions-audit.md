# Brief 03: the instruction files

**Scope:** every file that tells a session what to do: `CLAUDE.md`, `SPEC.md`'s framing, `PLAN.template.md`, `tests/README.md`, the docs under `docs/`, and the briefs. Reviewed on `svelte-migration` at `ef482c69`, and on `master`.

**Why this brief exists:** the owner's instructions don't reach the sessions reliably. The clearest case: the refactor keeps copying the Python app, because almost every concrete instruction a session reads still names the Python app as the reference, and only one sentence in `docs/architecture.md` says otherwise. Agents follow the most concrete instruction in front of them, so the files have to agree.

Apply this as one change on `svelte-migration`. It is a docs change and needs no gate.

## 1. The rule every session reads first

Put this paragraph at the very top of `CLAUDE.md`, above everything else. It is the owner's wording, kept as written:

> **The Python app is the original, and its mistakes are why this refactor exists.** Its code, data model and calculations contain many errors. Never copy them blindly, and never assume anything in them is correct or optimal because it exists or because it has always worked that way. Before carrying anything over (a rule, a structure, a formula, a default, a fallback), check it against `SPEC.md` and `docs/architecture.md`, and state in the plan why it is right. Some of it is; that has to be shown, not assumed. What the owner sees and does on screen is the exception: that stays as it is unless `SPEC.md` changes.

Then remove every line that contradicts it (§2 to §5).

## 2. `CLAUDE.md`: one build, one set of rules

- **Opening.** Replace the opening description of three desktop implementations with: Rust (`rust/`) and Svelte (`web/`) are the app being built; `SPEC.md` defines what it shows; `docs/architecture.md` defines how it is built; `docs/decisions.md` holds the owner's decisions.
- **Old builds out of the loaded file.** Remove from `CLAUDE.md` everything about `python/`, `go/` and `ledger.html`: descriptions, test commands, scratch-server recipes, the `PROTOCOL` trio, release archives, and the rules for frozen platforms. What stays about the old app is §1's rule, and `docs/parity.md` for what the page looks like and does.
- **Planning and the owner.** Replace "put its acceptance criteria in front of the user before writing code" with:
  - small work needs no plan;
  - heavy lifts (brief 01 §1) get a plan that goes through the gate;
  - the owner is asked only for decisions, at the top of the plan, in plain words (§6).
- **Verification.** Keep only the Rust and page commands (`cargo test --workspace`; `npm run check`, `npm test`, `npm run e2e`), the Rust scratch server with `BAGHOLDER_DRY_ORDERS=1` and `BAGHOLDER_NO_BROWSER=1`, and `SPEC.md` §7's page checks.
- **Commit attribution.** Remove the hard-coded "Claude Fable 5.1" line. Commits name the model that actually wrote them, as the harness supplies.
- **Keep, unchanged:** the worktree rule (never `git checkout` in the owner's checkout), the one fenced `bash` block per step for anything the owner runs, "only what was asked", the dependency rule, and "Do not".
- **Briefs are messages, not rules.** Once a brief is applied, fold its standing rules (the gate in brief 01 §1, the delegation rules in brief 01 §3) into `CLAUDE.md` or `docs/decisions.md`. A session should never need to read old briefs to know the rules.

## 3. `docs/decisions.md`: the one home for the owner's decisions

A dated list, one line per decision with its reason, newest first, each naming where it was made. Seed it with what is already decided:

- 2026-09-24: the user-facing UI and UX stay at parity; the engineering underneath is redesigned.
- 2026-09-24: make the app work properly as it is; nothing is built for expansion the owner hasn't asked for.
- 2026-09-24: correctness over speed; no deadline.
- 2026-09-24: the work is the migration. Nothing is done, proposed or asked about for the app that runs today or for the period before the Rust+Svelte build replaces it, unless it bears directly on building the new version properly.
- 2026-09-24: the known order defects are fixed in the new build, in stage 4.
- 2026-09-24: real figures in committed docs are acceptable.
- 2026-09-24: on open, the page shows its last figures at once and updates only when newer data arrives.
- 2026-09-23: a fund's schedule and distributions come from its fund company's own publication, never worked out from past dates.
- 2026-09-24: where that publication cannot be read at all, distributions come from the exchange-side record; the schedule only where a source states it, otherwise the income is a named gap.
- 2026-09-23: a live mark uses the latest rate the Bank of Canada has published.
- Every earlier decision the session holds (the watched stop stays a market sell; Clear data as the spec says; autonomous agent trading is the person's choice; and the rest in the design commits).

Rules for it:
- A decision is written here and pushed the moment it is made, before any work relies on it. A decision that lives only in a local plan is invisible to the reviewer; that is how brief 01 contradicted the 2026-09-23 decision.
- Where a brief, a plan or a doc disagrees with this file, this file wins, and the other is fixed.

## 4. `SPEC.md`: what the app shows, not how any build does it

- **Implementation claims.** Rewrite the opening paragraph: `SPEC.md` defines every figure and screen by itself. Remove "the model is defined by `python/model.py`, the reference implementation", the module names, and every "as `model.py` does".
- **Principles.** Take "One data folder per build" out of §1; it describes builds, not the app.
- **Timing.** This framing change lands now. The figure changes already listed for the switch (stage 2, stage 3a) land at the switch, as planned.

## 5. The tests: held to the spec, not to the old app

- **`tests/README.md`** says "The Python model is the reference." Mark `tests/cases` as the old model's cases, frozen with it.
- **The engine's cases** (`rust/crates/engine/tests/cases`) get their own short README saying:
  - they are written from `SPEC.md`, never generated from any implementation;
  - new expectations come from an agent that has not read the engine (brief 01 §3, item 2).
- **Pinned tests.** A golden that records old behaviour is a safety net while code moves, not a statement that the behaviour is right. One that pins a known-wrong behaviour says so in its name and points to its entry in §7's list.

## 6. `PLAN.template.md`: rewritten for this build

- **At the top, "For the owner to decide".** Plain words, one decision per item, each with the recommendation and what happens if it isn't taken; "Nothing open" when there is nothing. The 3a plan already does this; make it the template.
- **A required section, "The old app here".** What the old app does in this area, what is wrong with it, and what this plan does instead; or, for anything carried over, why it is right, shown against `SPEC.md` and the design. At the gate, anything carried over without this answer is a finding.
- **Acceptance criteria defaults.** Replace the Python, Go and shared-case lines with the Rust and page commands and the screenshot baselines (brief 01 §2.4).
- **Keep:** Right to refuse, Anti-stub self-check, Verification, Handoff.

## 7. `docs/old-app-mistakes.md`: the named list

One line per known mistake of the old app, each with the test that fails if it comes back, or "not guarded yet". Gather them from:
- `docs/design-review.md`: "Money and data risks" and each part's verdict;
- the "Why" lines of `docs/architecture.md` §5 and §6;
- stage 2's `tests/no_guesses.rs`.

For example: identity by bare symbol; floating-point money; the constant 1.35 rate; every non-USD currency treated as CAD; splits inferred from fill prices; the ×100 multiplier; frequency assumed as 12; a failed send never read back; the order list cut at 200; a fill booked twice by two simultaneous read-backs; the network layer re-sending an order POST; rows rewritten at start.

Extend `no_guesses.rs` so every entry that a source scan can guard is guarded.

## 8. The rest of `docs/`

- **Superseded plans.** Delete `docs/frontend-backend-migration.md` and `docs/plans/corporate-events.md`; git keeps them. A superseded plan that is only marked still gets read and followed.
- **`docs/parity.md`** is the one place where the old page is the reference, for what the page looks like and does. Add at its top: "What the page shows and how it behaves comes from here; how anything is built never does."
- **Status** lives only in `docs/design-review.md`'s order of work, as brief 01 §4 said.

## 9. Writing for the owner

Anything the owner reads (a plan's top section, a question, a handoff):
- plain words;
- the decision first, then why it matters, then the recommendation;
- one decision per question;
- no invented terms, and no section numbers without saying what they are.

The owner has said they won't review material written for sessions, so what they must decide has to be readable without it.
