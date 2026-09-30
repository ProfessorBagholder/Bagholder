# Working on Bagholder

> **The Python app is the original, and its mistakes are why this refactor exists.** Its code, data model and calculations contain many errors. Never copy them blindly, and never assume anything in them is correct or optimal because it exists or because it has always worked that way. Before carrying anything over (a rule, a structure, a formula, a default, a fallback), check it against `SPEC.md` and `docs/architecture.md`, and state in the plan why it is right. Some of it is; that has to be shown, not assumed. What the owner sees and does on screen stays as it is unless `SPEC.md` changes. That is a limit on what a session may change, never a bar on how it is built: structure, data flow, storage, the code that moves money and what the app costs the machine are built as the best engineers build an application of this kind, and a small machine (a Raspberry Pi) must carry it; a request is never implemented so narrowly that this is set aside. Where the screen falls short of how the leading trading journals (Tradervue, TradeZella, TraderSync, Edgewonk) and brokerage screens (Interactive Brokers, Wealthsimple, Questrade, Sharesight) do it, say so, naming the product and what it does, as a proposal to the owner in one line; build it only if the owner takes it.

> **A rule that must hold for any input is built and tested as a rule, never around an example.** Any symbol, fund company, time zone, currency or account: the code has no case of its own for one, and the test covers the whole range (every zone in the time-zone database, a listing no reader knows, a currency other than the ones the owner holds). A plan, doc, brief or message to the owner never names an example as though it were a case of its own: naming one makes the rule look like a special case and hides whether the rule was built at all. The same goes for windows, thresholds and sizes: a window or threshold comes from a stated source (a settlement cycle, the broker's own document) or is a named parameter whose reason is that source, never one person's data; its tests use generated inputs, with the owner's capture as a smoke check only; a cost is measured at the owner's size, never on the demo book alone.

> **Every action must be the best course of action for the objective.** Before you do anything (search, scan, run, measure, poll, build, wait), answer two questions:
> 1. Is this actually the best way to achieve what I am trying to achieve? Not whether it would tell me something, not whether it is allowed, not whether it is cheap.
> 2. Do I already have the information at hand, or information that would otherwise nullify the value of doing this? Check the code in front of you, the docs, the replies already recorded, and what the owner has said.
>
> If what you have settles it, act on it. When something is genuinely missing, the best course is the direct route to that one thing.
>
> A broad or blind move (searching everything, trying things until one works, watching to see what happens) is almost never the best course; it means the problem is not understood yet, so stop and think. If you cannot answer both questions for an action, don't do it.

> **A report names a symptom; the change fixes the class.** Before writing code, name the rule the symptom breaks and list every screen, figure, source or path where the same rule applies, by reading the code. Fix all of them in one PR and hold the rule with one test over the whole list. A fix that would leave the same defect standing elsewhere goes through the gate.

> **Work until it is done, then report once.** Build, verify (tests, the scratch server, the browser, the container where the change touches running, the capacity test where it touches the server) and fix until every acceptance criterion is met before telling the owner anything. The PR is the report: what changed, the numbers from verification, and at most one action only the owner can take, named alone. No progress reports, no play-by-play, no steps for the owner to test.

Bagholder is a trading journal, with the other features a trader keeps beside one. Today it runs on one machine for one person and reads one broker, Wealthsimple; those are facts to design for, not the shape to design from: another broker, another person, another machine or a headless copy are ordinary directions for an application of this kind, never a reason to build a narrower one. The app is one build: the Rust server (`rust/`, a Cargo workspace, toolchain pinned in `rust/rust-toolchain.toml`) with the Svelte page (`web/`, Svelte 5 + TypeScript + Vite, built to `web/dist` and carried in the server's release build). Three documents govern it:

- `SPEC.md`: what the app shows and does, every figure and every screen.
- `docs/architecture.md`: how it is built, and the rules every change is held to (work only because something changed; the server tells the page; one element updates, never a screen; types, not blobs; every behaviour in `SPEC.md` driven by a browser test), with the cost budget the server is held to.
- `docs/decisions.md`: the owner's decisions. Where anything else disagrees with it, it wins and the other is fixed.

Also: `docs/design-review.md` (the review that set the migration's order of work; history, not status: the state of the build is the open pull requests and issues; a known defect that is not fixed in the change that found it is an issue, with the product that shows the standard and the test that will hold the fix, never a note in a file), `docs/old-app-mistakes.md` (the old app's known mistakes and what guards each), `docs/parity.md` (what the old page shows and does, the one place the old app is the reference, for the screen only), `docs/plans/` (stage plans). The phone apps (`ios/`, `android/`, `MOBILE.md`) are on hold: they compute the old model and keep their own version numbers. The repository is public.

## The gate: plans for heavy lifts only

- **Small work needs no plan**: fixes, tests, and work inside an approved plan. A fix still fixes the class (above) and lands as a PR that meets the gate's last rule.
- **Heavy lifts are gated**, reviewed before they are built: each stage plan, a new feature, a refactor across crates or of the page's data layer, and any change to how a figure is defined, to stored data (a migration), to the wire, to order execution or to access.
- **How:** write the plan from `PLAN.template.md`, including how the leading journals and the industry do it, with sources, open it as a PR to `master` under `docs/plans/`, and carry on with work already approved. The owner points the reviewer at it; the verdict comes back as the next numbered brief on the `architecture-briefs` branch: Go, Go with changes (each one required), or Stop. Apply the required changes; disagree only under the plan's "Right to refuse", for the owner to decide. Never deviate silently, and never reopen the owner's decisions.
- **Briefs are messages, not rules.** Once a brief is applied, its standing rules are in this file or in `docs/decisions.md`; old briefs never need reading to know the rules.
- **The owner is asked only for decisions**, at the top of the plan (*For the owner to decide*): a real choice between options the owner cares about, or a departure from one of the owner's standing rules. A correctness fix is not a decision: it goes into `SPEC.md` with its reason and is checked at the gate. A decision the owner makes goes into `docs/decisions.md` and is pushed at once, before any work relies on it.
- **Before merge, the PR quotes each acceptance criterion with its evidence.** A criterion the session cannot run is named at the plan's top with the one capture the owner supplies once. A PR with a criterion not met is not merged. A required change that alters what a screen shows, how it responds, or what the app reads unprompted goes to the owner as one line before it is built, and an accepted departure is written into `docs/decisions.md` in the PR that builds it.

## Writing for the owner

The owner does not review material written for sessions, so anything they must read or decide (a plan's top section, a question, a handoff, a report) stands alone:

- plain words; the decision first, then why it matters, then the recommendation;
- one decision per question;
- no invented terms, and no section numbers without saying what they are.

## What is authoritative

Each kind of rule lives in one place; other files cite it and never restate it.

- `docs/decisions.md`: the owner's rules and decisions, in the owner's own words with where they were said, each with a "Held by" line naming the test that proves it on a violation, or "review only". It wins over every other file.
- `SPEC.md`: every figure and every screen. Check every change against it; when a change needs a definition to differ, change `SPEC.md` in the same commit and say why, naming the product the definition is modelled on.
- `docs/architecture.md`: how it is built, the rules a build is held to, and the cost budget.
- A test that records old behaviour (a golden) is a safety net while code moves, not a statement that the behaviour is right. One that pins a known-wrong behaviour is named `test_known_wrong_…` and points to its entry in `docs/old-app-mistakes.md`.
- A rule with no test is held by reading the change against the file that states it. Do not assume the suite will catch a policy rule.

## Agents: when to delegate

Delegate only when all three hold: the task separates cleanly with a clear contract; it gains from a fresh context or from running in parallel; its result is cheap to check. Otherwise do it here: a handoff loses context, and lost context is where mistakes get in.

- **Worth it:** a gate review (one independent reviewer on the strongest model, given the design's §1–§3, the `SPEC.md` sections touched and the plan; findings that change behaviour or correctness only, ranked, each with `file:line` and a failing scenario, about fifteen at most; split along seams only for a large stage; each finding reproduced before it is fixed; nits batched into the next change). Expected figures for new engine cases, written by an agent given `SPEC.md`, the case format and the inputs and told not to read `rust/crates/engine`. Read-only surveys of the old crates, with `file:line` answers, a sample verified.
- **Not worth it:** designing the switch, the wire or a stage's contracts; changing `core`, `book` or `engine`; anything about money, identity or orders; parallel builders on crates that depend on each other; small fixes, test-fix loops and doc edits; several reviewers on one small diff, or any "review everything" prompt.
- **Mechanics:** every delegated prompt names the goal, the paths in and out of scope (never the whole disk), the `SPEC.md` and design sections, the output format and a length cap. An agent that writes code works in its own worktree and its diff is read before it is taken. Nothing an agent reports goes into a doc until it is verified.

## How changes land

1. Never commit to `master`, and never run `git checkout` in the owner's checkout: their live app serves from it. Work in a detached worktree under the session scratchpad (`git worktree add --detach <dir> origin/master`), commit there, and push the commit straight to a topic branch without creating a local one: `git push origin HEAD:refs/heads/<topic>`. A branch checked out in any worktree locks that name, and the owner's own checkout of it then fails.
2. Verify before committing (below). If something is wrong, stop and report before committing; do not commit and mention it afterwards.
3. Commit with a message that says what changed and why, ending with the co-author line the harness supplies for the model that wrote it. Open the PR from the pushed branch (`gh pr create --head <topic> --fill`); the PR carries what the gate's last rule asks for, and its body ends with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
4. Anything the owner has to do is one fenced `bash` block per step; a step never exists in prose. Before writing a block, check the owner's checkout in the same turn (`git fetch -q; git branch --show-current; git worktree list; git status --short`) and write the block for what it shows. Whatever can be run here is run here, never handed over; the owner is left only what cannot be a command from this machine (a sign-in), named alone.
5. Merging is the owner's call: the owner merges, or says to. Nothing is merged by a session on its own.

## Verifying a change

- **Rust**, from `rust/`: `RUSTFLAGS="-D warnings" cargo test --workspace`, and `cargo clippy --workspace --lib --bins` clean (the workspace denies a discarded failure). A behaviour change comes with a test. A pushed book migration is never edited; a change is a new migration.
- **The page**, from `web/`: `npm run check` (svelte-check), `npm test` (Vitest), `npm run e2e` (builds the page, then Playwright against the real server on a made-up book: offline, dry orders, its own temporary home, port 8791; `E2E_PORT=<n>` gives a second run its own server). A behaviour in `SPEC.md` is not done until a browser test drives it.
- **The wire** is typed in `rust/crates/model/src/wire.rs`; `cargo test -p bagholder-model --test types` regenerates `web/src/lib/generated/wire.ts`, and the page takes its types from there.
- **Timers:** a timer anywhere is replaced by waiting for the thing itself, or argued for in `TIMED_WAITS` (`rust/crates/server/src/tests_misc.rs`), the one list of timers that remain; `test_no_wait_on_a_clock_that_is_not_accounted_for` holds the code to it.
- **Cost**, for any change to the server or to what the page reads: the numbers pasted in the PR against the budget in `docs/architecture.md`: requests and bytes on a first and a second open, time to first figure, server CPU and resident memory over an hour idle with a page open, the time to add a trade on a book of the owner's size, the page's transfer size and the release archive's size. The capacity test on the owner-size book holds the server to the budget.
- **States and screenshots**, for any change to a screen: a states table per screen the change touches (first run, kept, refreshing, source failed, nothing in scope, cleared) and a screenshot of each state in the PR.
- **The container**, for any change to running, sign-in, updates or the entrypoint: the image built and started on the made-up book, and the changed path driven in it.
- **A change to a test named in `docs/decisions.md`** restates in the PR what the test now proves.
- **Rendering**: run a scratch server on a copy of the owner's data folder, never on the live one. From `rust/`:
  ```
  mkdir -p /tmp/bh-scratch-rust && cp ~/.bagholder-rust/*.db /tmp/bh-scratch-rust/
  cargo build --release --bins
  BAGHOLDER_NO_BROWSER=1 BAGHOLDER_DRY_ORDERS=1 BAGHOLDER_HOME=/tmp/bh-scratch-rust BAGHOLDER_PORT=8798 target/release/bagholder
  ```
  `BAGHOLDER_DRY_ORDERS=1` is not optional: orders are live by default, and a scratch copy that ever carries a login must never place one. `BAGHOLDER_NO_BROWSER=1` is not optional: without it every scratch start opens the scratch data in the owner's browser. Open `http://127.0.0.1:8798/` (`localhost` is refused by design). The owner's own instance runs on 8765; never restart or write to it.
- **What to check** is `SPEC.md` §7: every displayed figure traced to its field and meaning; every page at 1200, 1340, 1440 and 1680 px with no table overflowing or clipping at 1340 and above; headers level; lookups by id, exercised with a duplicate symbol in a second account. Take screenshots; measure with JavaScript rather than by eye.

## Releases

A version is a GitHub release tagged `vMAJOR.MINOR.PATCH` (semantic versioning: PATCH for fixes only, MINOR for anything new a user can see or do, MAJOR for a change that breaks existing installs). The product carries one number, bumped in the last change before the release: `APP_VERSION` in `rust/crates/server/src/app.rs` (the workspace `version` in `rust/Cargo.toml` mirrors it); the phone apps are on hold and keep their own numbers. To release, bump, merge, then `gh release create vX.Y.Z --target master --title vX.Y.Z --notes "..."` with the merged PRs in the notes. The tag starts `.github/workflows/release.yml`, which checks the tag against `APP_VERSION` and attaches `bagholder-vX.Y.Z-rust-<target-triple>.tar.gz` (`.zip` on Windows) per platform, each with its `.sha256`, named as the updater looks for them: the server with the page built in, `bagholder-browser`, `disclosures-mcp` and `sedar`. `.github/workflows/docker.yml` builds `rust/Dockerfile` from the repository root, fails when the tag differs from `APP_VERSION`, and publishes `ghcr.io/professorbagholder/bagholder:X.Y.Z` and `:latest`, and `:rust-X.Y.Z` and `:rust`.

## Handoffs from Claude Design

The file served from the design URL carries Design's preview harness on line 4 (`data-omelette-injected`, a script that hooks `fetch`, `postMessage` and cookies). Strip that line, its closing `</script>` and the blank line after it. What the handoff describes is a change to what the page shows: check it against `SPEC.md` and `docs/parity.md`, build it in `web/`, and verify it as above. A handoff can carry a wrong lookup or a wrong formula as easily as a colour.

## Do not

- Commit `.env`, `session.json`, the database, backups, `.claude/`, `rust/target` or `web/node_modules` (all in `.gitignore`); the repository is public.
- Reach for a dependency without weighing it. A crate or an npm package is a real cost (build time, binary size, supply chain), so add one only when the job genuinely needs it and the language and what the workspace already has cannot do it well. When you add or lean on one, list it in the relevant `Cargo.toml` with a comment on why (for an npm package, in the commit that adds it).
- Reformat or "clean up" code you were not asked to change.
- Do anything the owner did not ask for: nothing on screen, no feature and no capability in a design the owner has not asked for. A shortfall against the category's products is proposed (first rule), never built on a session's own decision. Building what is underneath properly is not "something the owner did not ask for": it is the job, and a refusal on the grounds of scope is one line to the owner, never a decision taken inside a plan.
