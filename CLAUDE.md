# Working on Bagholder

> **The Python app is the original, and its mistakes are why this refactor exists.** Its code, data model and calculations contain many errors. Never copy them blindly, and never assume anything in them is correct or optimal because it exists or because it has always worked that way. Before carrying anything over (a rule, a structure, a formula, a default, a fallback), check it against `SPEC.md` and `docs/architecture.md`, and state in the plan why it is right. Some of it is; that has to be shown, not assumed. What the owner sees and does on screen is the exception: that stays as it is unless `SPEC.md` changes.

> **A rule that must hold for any input is built and tested as a rule, never around an example.** Any symbol, fund company, time zone, currency or account: the code has no case of its own for one, and the test covers the whole range (every zone in the time-zone database, a listing no reader knows, a currency other than the ones the owner holds). A plan, doc, brief or message to the owner never names an example as though it were a case of its own: naming one makes the rule look like a special case and hides whether the rule was built at all.

> **Every action must be the best course of action for the objective.** Before you do anything (search, scan, run, measure, poll, build, wait), answer two questions:
> 1. Is this actually the best way to achieve what I am trying to achieve? Not whether it would tell me something, not whether it is allowed, not whether it is cheap.
> 2. Do I already have the information at hand, or information that would otherwise nullify the value of doing this? Check the code in front of you, the docs, the replies already recorded, and what the owner has said.
>
> If what you have settles it, act on it. When something is genuinely missing, the best course is the direct route to that one thing.
>
> A broad or blind move (searching everything, trying things until one works, watching to see what happens) is almost never the best course; it means the problem is not understood yet, so stop and think. If you cannot answer both questions for an action, don't do it.

Bagholder is a local-first trading journal for Wealthsimple users. The app being built is the Rust server (`rust/`, a Cargo workspace, toolchain pinned in `rust/rust-toolchain.toml`) with the Svelte page (`web/`, Svelte 5 + TypeScript + Vite, served from `web/dist`). Four documents govern it:

- `SPEC.md`: what the app shows and does, every figure and every screen.
- `docs/architecture.md`: how it is built, and the rules every change is held to (work only because something changed; the server tells the page; one element updates, never a screen; types, not blobs; every behaviour in `SPEC.md` driven by a browser test).
- `docs/decisions.md`: the owner's decisions. Where anything else disagrees with it, it wins and the other is fixed.
- `docs/design-review.md`: the order of work, the one place status is kept.

Also: `docs/old-app-mistakes.md` (the old app's known mistakes and what guards each), `docs/parity.md` (what the old page shows and does, the one place the old app is the reference, for the screen only), `docs/plans/` (stage plans). The old builds (`python/`, `go/`, `ledger.html`) are frozen and removed at cutover; nothing is done for them. The phone apps (`ios/`, `android/`, `MOBILE.md`) are on hold. The repository is public.

## The gate: plans for heavy lifts only

- **Small work needs no plan**: fixes, tests, and work inside an approved plan.
- **Heavy lifts are gated**, reviewed before they are built: each stage plan, a new feature, a refactor across crates or of the page's data layer, and any change to how a figure is defined, to stored data (a migration), to the wire, to order execution or to access.
- **How:** write the plan from `PLAN.template.md`, including how the leading journals and the industry do it, with sources, push it to `svelte-migration` under `docs/plans/`, and carry on with work already approved. The owner points the reviewer at it; the verdict comes back as the next numbered brief on the `architecture-briefs` branch: Go, Go with changes (each one required), or Stop. Apply the required changes; disagree only under the plan's "Right to refuse", for the owner to decide. Never deviate silently, and never reopen the owner's decisions.
- **Briefs are messages, not rules.** Once a brief is applied, its standing rules are in this file or in `docs/decisions.md`; old briefs never need reading to know the rules.
- **The owner is asked only for decisions**, at the top of the plan (*For the owner to decide*): a real choice between options the owner cares about, or a departure from one of the owner's standing rules. A correctness fix is not a decision: it goes into `SPEC.md` with its reason and is checked at the gate. A decision the owner makes goes into `docs/decisions.md` and is pushed at once, before any work relies on it.

## Writing for the owner

The owner does not review material written for sessions, so anything they must read or decide (a plan's top section, a question, a handoff, a report) stands alone:

- plain words; the decision first, then why it matters, then the recommendation;
- one decision per question;
- no invented terms, and no section numbers without saying what they are.

## What is authoritative

- `SPEC.md` defines every figure and every screen. Check every change against it; when a change needs a definition to differ, change `SPEC.md` in the same commit and say why.
- The owner's standing rules, all recorded in the spec and `docs/decisions.md`: only what was asked, and nothing on screen the owner did not ask for (there is no rule against text beyond that); no browser `title` tooltips, and wherever a hover tooltip is needed it is the app's own styled one, one look everywhere; per-instrument figures in the instrument's own currency; aggregates in CAD, never labelled; payout frequency from the fund's own record, never assumed; raw broker rows never rewritten; nothing synthetic on a chart.
- Which is checked how: tests enforce the mechanical ones (the engine's cases, the crate boundaries, `no_guesses.rs`, the timers list, the browser tests, `web/src/no_title.test.ts` for browser tooltips). The rest are policy, held by reading the change against `SPEC.md`: nothing unasked on screen, per-instrument currency, nothing synthetic on a chart. Do not assume the suite will catch a policy rule.
- A test that records old behaviour (a golden) is a safety net while code moves, not a statement that the behaviour is right. One that pins a known-wrong behaviour is named `test_known_wrong_…` and points to its entry in `docs/old-app-mistakes.md`.

## Agents: when to delegate

Delegate only when all three hold: the task separates cleanly with a clear contract; it gains from a fresh context or from running in parallel; its result is cheap to check. Otherwise do it here: a handoff loses context, and lost context is where mistakes get in.

- **Worth it:** a gate review (one independent reviewer on the strongest model, given the design's §1–§3, the `SPEC.md` sections touched and the plan; findings that change behaviour or correctness only, ranked, each with `file:line` and a failing scenario, about fifteen at most; split along seams only for a large stage; each finding reproduced before it is fixed; nits batched into the next change). Expected figures for new engine cases, written by an agent given `SPEC.md`, the case format and the inputs and told not to read `rust/crates/engine`. Read-only surveys of the old crates, with `file:line` answers, a sample verified.
- **Not worth it:** designing the switch, the wire or a stage's contracts; changing `core`, `book` or `engine`; anything about money, identity or orders; parallel builders on crates that depend on each other; small fixes, test-fix loops and doc edits; several reviewers on one small diff, or any "review everything" prompt.
- **Mechanics:** every delegated prompt names the goal, the paths in and out of scope (never the whole disk), the `SPEC.md` and design sections, the output format and a length cap. An agent that writes code works in its own worktree and its diff is read before it is taken. Nothing an agent reports goes into a doc until it is verified.

## How changes land

1. Never run `git checkout` in the owner's checkout: their live app serves from it. Work in a detached worktree under the session scratchpad (`git worktree add --detach <dir> origin/svelte-migration`), commit there, and push the commit straight to the branch without creating a local one: `git push origin HEAD:refs/heads/svelte-migration`. A branch checked out in any worktree locks that name, and the owner's own checkout of it then fails.
2. Verify before committing (below). If something is wrong, stop and report before committing; do not commit and mention it afterwards.
3. Commit with a message that says what changed and why, ending with the co-author line the harness supplies for the model that wrote it.
4. Anything the owner has to do is one fenced `bash` block per step; a step never exists in prose. Before writing a block, check the owner's checkout in the same turn (`git fetch -q; git branch --show-current; git worktree list; git status --short`) and write the block for what it shows. Whatever can be run here is run here, never handed over; the owner is left only what cannot be a command from this machine (a sign-in), named alone.
5. Merging into `master` is the owner's call, at cutover.

## Verifying a change

- **Rust**, from `rust/`: `cargo test --workspace`, and `cargo build --workspace --all-targets` with no warnings. A behaviour change comes with a test. A pushed book migration is never edited; a change is a new migration.
- **The page**, from `web/`: `npm run check` (svelte-check), `npm test` (Vitest), `npm run e2e` (builds the page, then Playwright against the real server on a made-up book: offline, dry orders, its own temporary home, port 8791; `E2E_PORT=<n>` gives a second run its own server). A behaviour in `SPEC.md` is not done until a browser test drives it.
- **The wire** is typed in `rust/crates/model/src/wire.rs`; `cargo test -p bagholder-model --test types` regenerates `web/src/lib/generated/wire.ts`, and the page takes its types from there.
- **Timers:** a timer anywhere is replaced by waiting for the thing itself, or argued for in `TIMED_WAITS` (`rust/crates/server/src/tests_misc.rs`), the one list of timers that remain; `test_no_wait_on_a_clock_that_is_not_accounted_for` holds the code to it.
- **Rendering**: run a scratch server on a copy of the owner's book, never on the live one. From `rust/`:
  ```
  mkdir -p /tmp/bh-scratch-rust && cp ~/.bagholder/bagholder.db /tmp/bh-scratch-rust/
  cargo build --release --bins
  BAGHOLDER_NO_BROWSER=1 BAGHOLDER_DRY_ORDERS=1 BAGHOLDER_HOME=/tmp/bh-scratch-rust BAGHOLDER_PORT=8798 target/release/bagholder
  ```
  `BAGHOLDER_DRY_ORDERS=1` is not optional: orders are live by default, and a scratch copy that ever carries a login must never place one. `BAGHOLDER_NO_BROWSER=1` is not optional: without it every scratch start opens the scratch data in the owner's browser. Open `http://127.0.0.1:8798/` (`localhost` is refused by design). The owner's own instance runs on 8765; never restart or write to it.
- **What to check** is `SPEC.md` §7: every displayed figure traced to its field and meaning; every page at 1200, 1340, 1440 and 1680 px with no table overflowing or clipping at 1340 and above; headers level; lookups by id, exercised with a duplicate symbol in a second account. Take screenshots; measure with JavaScript rather than by eye.

## Handoffs from Claude Design

The file served from the design URL carries Design's preview harness on line 4 (`data-omelette-injected`, a script that hooks `fetch`, `postMessage` and cookies). Strip that line, its closing `</script>` and the blank line after it. What the handoff describes is a change to what the page shows: check it against `SPEC.md` and `docs/parity.md`, build it in `web/`, and verify it as above. A handoff can carry a wrong lookup or a wrong formula as easily as a colour.

## Do not

- Commit `.env`, `session.json`, the database, backups, `.claude/`, `rust/target` or `web/node_modules` (all in `.gitignore`); the repository is public.
- Reach for a dependency without weighing it. A crate or an npm package is a real cost (build time, binary size, supply chain), so add one only when the job genuinely needs it and the language and what the workspace already has cannot do it well. When you add or lean on one, list it in the relevant `Cargo.toml` with a comment on why (for an npm package, in the commit that adds it).
- Reformat or "clean up" code you were not asked to change.
- Do anything the owner did not ask for: nothing on screen, no feature and no capability in a design the owner has not asked for, and no work for the app that runs today.
