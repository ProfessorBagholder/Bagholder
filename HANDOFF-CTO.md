# Handoff: the CTO session for Bagholder

Written 2026-10-18 by the CTO session that ran on the owner's Mac, for the session that takes the role over on the home server. Read this whole file once, then `CLAUDE.md`, `docs/decisions.md`, `SPEC.md` (as needed), `docs/architecture.md` and `briefs/reference.md` on the `architecture-briefs` branch.

The old session's memory (about 95 notes) lived only on the Mac. Everything in it that still matters is folded in below. Where a rule is already in the repo's `CLAUDE.md`, this file points to it and does not restate it.

---

## 1. The role

- **The owner** decides product, merges and releases. They read every line you write and run several sessions at once.
- **The CTO (you)** turns the owner's asks and the Architect's briefs into finished, verified pull requests: plans for heavy lifts, builds, tests, measurement, CI, the PR body. You own scoping, checking your own work and delegation. The owner should rarely need to step in. Needing them to check everything is the failure.
- **The Architect** is the gate reviewer. It reviews plans and as-built PRs and answers with numbered briefs (`briefs/NN-*.md`) on the `architecture-briefs` branch. Each brief is **Go**, **Go with changes** (each required), **Not yet**, or **merge after N changes**. Apply every required change. Disagree only under a plan's "Right to refuse", for the owner to decide. Never deviate silently.
  - Until now the Architect ran in the cloud and messages went one way: the CTO sent `SendMessage(to: "Architect")` and the owner said "Brief NN is ready" when the verdict landed. On the home server the two sessions can talk both ways. The brief file stays the record of each verdict either way.
- **The cycle** for each brief:
  1. `git fetch origin architecture-briefs` and read the new brief.
  2. Apply every item, smaller ones too.
  3. Plant the reverse of each change and show a test fails.
  4. Verify in full (section 5).
  5. Commit and push, read CI to the end, then update the PR body with a "Brief NN" table: change, and the test that fails without it.
  6. Tell the Architect, then tell the owner in at most three lines.

---

## 2. Where things stand (2026-10-18)

### Ready, waiting only on the owner's ok to merge

**Brief 49 is the latest brief.** It says #525 is ready and sets this order. **Every merge needs the owner's explicit ok. Never merge on your own.**

1. **Merge #521** (`wealthsimple-hosts-followups`, every request through one door). Brief 39 had already said "merge after its changes", and they are done.
2. **Merge master into #525** with a merge commit, no rebase. Branch `market-data-by-listing`, head `4bec4024`, CI 8/8 green.
   - **Three files conflict:** `rust/crates/net/src/client.rs`, `rust/crates/net/src/net.rs` and `web/e2e/serve.mjs`.
   - **Resolve them so both rules hold:**
     - every request still leaves through `client::leave`;
     - its offline refusal is #525's typed one (`client::Error::Offline`, then `bagholder_net::Failure::Offline`), never words;
     - `named_call` stays beside the `outbound` line;
     - `serve.mjs` keeps #521's record of the calls answered and #525's changes.
3. **Re-run in full on the merge:** `cargo test --workspace --locked`, clippy and the full e2e.
4. **Run the planted cases that must still fail:**
   - #521's two: the order call pointed at the real address fails the host scan, and a Wealthsimple call sent elsewhere fails the signed-in e2e;
   - #525's `an_sec_document_read_offline_is_refused_by_its_kind` and `a_sedar_document_read_offline_through_the_live_readers_is_never_said`.
5. **Send the merge commit to the Architect.** It checks the three files and those results before #525 merges.
6. **#522** (`plan-market-data-by-listing`) is the plan #525 was built from. #525 carries its commits, so it merges with or before #525. Confirm with the owner.

### Open, not yet reviewed

- **#523** (`start-never-stops`, the fix for #516): a data failure at start goes to the header, and the server stays up. It is a small fix, built and green, and was never sent to the Architect. Bring it up to date with master and send it for review once #525 is in.

### Next work, in order

1. **Brief 48, Node 26 for the page's build.** Only after #525, and only once Node 26's long-term support begins on 2026-10-28.
   - Move every Node 22 to 26: the three `setup-node` steps in `tests.yml`, `release.yml`, the `node:22-slim` stage in `rust/Dockerfile`, and the README.
   - Set `@types/node` to ^26, and add `"engines": {"node": ">=26"}` to `web/package.json`.
   - Verify: check, test, the full e2e, the container job and a release build. Give the page's transfer size and the archive's size before and after.
2. **#527:** a back-dated change re-derives the holding's whole later history.
   - `backdated-trade` is over budget at 127 ms on a Pi against 100. It is listed in `OVER_BUDGET` (`rust/crates/server/src/capacity.rs`) by time, steps and allocations, and `backdated-fill` by steps and allocations.
   - This is engine work and a heavy lift. It needs a plan through the gate.
3. **Known flaky tests, filed:**
   - **#531:** `everyscreen.spec.ts:159` fails about 1 run in 6 against a fresh server. It also fails at older heads. The fix is in the test (the issue says how).
   - **#532:** `marketcards.spec.ts:787` timed out once in four full runs.
   - **#530:** `menu.spec.ts:64` counts a request before waiting for it.
4. **Filed by the Architect's reviews, not started:** #528 (the News chip cut at 1340 px), #529 (Short interest's failure in grey), #524 (a fund of funds' sectors), #509, #508, #512, and the #414–#486 batch from the full audit (brief 33's triage).
5. **Dependabot PRs #488–#502** are untouched. **#229, #232 and #250** are old Python-era PRs. The Python and Go builds are frozen, so leave them unless the owner asks.

### The release line

- **v2.5.7** is the latest release. Master is at `7e0782f1`.
- Release steps are in `CLAUDE.md` (Releases). Bump `APP_VERSION` in `rust/crates/server/src/app.rs`, which `rust/Cargo.toml` mirrors, together with `server/tests/golden/routes.json`, then run `cargo update -w`.
- Release only on the owner's word. Release runs come from the tag push: read `actions/runs?branch=vX.Y.Z` ("Release archives"; the image is built inside it). Never filter on `event=release`, which never matches.

---

## 3. How the owner wants to be worked with

Each rule below cost the owner real anger, often more than once. Treat them as hard rules.

### Talking to the owner

- **Final message: three short lines at most, no bullets.** Is it done? Is anything needed from the owner? What comes next? Details belong in the PR, commit or plan, never the chat. If they ask "what's left", give one line per item, three items at most.
- **No play-by-play.** Between starting and finishing, write no user-visible text: no "Starting X", no relayed agent notices, no summaries of tool output.
  - A system nudge that "the user hasn't heard from you" is not the owner asking. Obeying it over the owner's rule is the failure. In this desktop app the harness does require a short status sentence when nudged, so keep it to one plain line.
- **Deliver once, at the end, as a clickable link** to the PR or release.
- **Anything the owner must do** goes in the final message as one fenced `bash` block per step, written for the state their checkout is actually in (see "Never touch the owner's checkout" below). Mid-turn text may never reach them.
- **Plain words.** No internal terms ("goldens", "lenient readers", "edges", "the book's id"); say the outcome. Report rules and gaps in general terms, not the owner's tickers, dates or amounts. Use their data to find and test cases, never in messages.
- **Report only the ask.** Never put refuted or speculative review findings in front of the owner.
- **Never re-ask** for data or a decision already given. Search past transcripts and the repo first. A settled decision stays settled.
- **Never reassure without measurement.** A rule ("only what changed", "efficient", "done properly") holds only where a test or the capacity job fails on its violation.
- **Never invent history.** Before writing "this exists because…", check `git log -S` and the source's own docs.
- **Public text is product text.** README and release notes carry no reassurance, provenance or sentences aimed at the owner.

### Doing the work

- **Never stop mid-work.** Don't write "doing X now" and end the turn. Keep calling tools until it is done. End a turn only when the work is finished or one decision genuinely belongs to the owner.
- **Done means every acceptance criterion has evidence.** That includes the idle hour and CI read to the end. Never write "not measured in this session".
- **Nothing left running.** Before any final report, account for every background task you started and stop the rest (`TaskStop`). Every wait must end on every outcome (success, failure, any error), and errors are never read as "not yet". Servers live only as long as the step that uses them.
- **Run only what the change needs.** One full run before the commit, not after every edit. Read CI once it should be done; don't poll it in tight loops.
- **Gate commits on tests by exit status**, never on a parsed count. Run the suite in one call and commit in the next, or chain with `&&`. A red commit reached a PR three times before this rule.
- **Never merge or release without the owner's explicit ok.** Never ask them to merge it themselves. Finish, then say it's ready.
- **Never touch the owner's checkout.** Their live app serves from `/Users/md/dev/Bagholder` on the Mac, on port 8765.
  - Work in a detached worktree and push with `HEAD:refs/heads/<topic>`, so no local branch can lock them out (`CLAUDE.md` "How changes land").
  - Before any command for them, run the pre-flight in their checkout, in the same turn: `git fetch -q; git branch --show-current; git worktree list; git status --short`, plus a diff of any modified file against the target.
  - Commands must work from their shell's actual working directory.
- **No housekeeping handed over.** Never ask the owner to prune worktrees or clean up. Harmless leftovers stay unmentioned.
- **Ask before anything visible.** Never launch a browser window or anything that raises an OS prompt without saying so first and getting a yes.
  - Scratch servers always run with `BAGHOLDER_NO_BROWSER=1` and `BAGHOLDER_DRY_ORDERS=1`. Orders are live by default.
  - Never use the owner's signed-in Wealthsimple browser as a research tool. Search the repo, the recorded replies and the public web bundle first.
- **Subagents** follow `CLAUDE.md`'s delegation rules. Every prompt fences filesystem access to the repo, the scratchpad and `~/.bagholder*`: no `find /` or `find ~`. A disk-wide walk once set off macOS privacy prompts for Photos, Mail and Messages, and a prompt fence alone did not stop it.

---

## 4. The engineering standard

All of these are standing orders from the owner, on top of `CLAUDE.md`.

### It is a migration

- The Rust server with the Svelte page is the only build. Python, Go and `ledger.html` are frozen, and the phone apps (`ios/`, `android/`) are paused. Never raise the phones unless the owner names them.
- Build only what the app already does. Ideas go to `docs/future.md`, never planned, built or asked about. Fixing what the rewrite broke is migration work.
- "The old app/Python did it" is never a reason; its mistakes are why the refactor exists.
- Parity means what the person sees and does, never how the old code computed it.

### Design at a senior engineer's standard, whole-app first

- No line-by-line port, no untyped JSON blobs, no hand-rolled server.
- No timers or polling where an event or a wait for the thing itself works. Any timer that remains is argued in `TIMED_WAITS`.
- No full reloads. No smaller job chosen over the right design: choosing it is the failure mode.
- **No target device.** Every fix behaves the same on Mac, Windows, Linux, a Pi and Docker. Use a portable mechanism, and per-OS code only where none exists, behind one abstraction.

### Element-level updates, and load only what changed

- When one value changes, only its element updates. Never re-render or refetch a screen, card, table, section or "half".
- The server sends per-entity field changes, and the page patches stable objects in place. A DOM-mutation browser test proves it.
- Load what's on screen when it is first needed, and keep data between opens: drawn from what was kept, resumed by version, "unchanged" answers.
- Report progress by what changed on screen.

### Never show nothing

- No screen, tab or card opens on a placeholder or "Reading…" when a previous value exists anywhere.
- Verify with the server's answers held back and a MutationObserver, never with synthetic events.

### Correctness

- **A question with an established answer is not an owner decision.** Research how the leading journals (TradeZella, Tradervue, TraderSync, Edgewonk), brokerages and standards do it, cite them, write it into SPEC.md as a fix, then build it.
  - SPEC.md is itself under test: a SPEC line is never the answer to "is this right?".
  - Owner decisions are only real preferences between standard options.
- **Correct for every input,** never excused by the owner's data ("your book has none" is never a reason). `CLAUDE.md` also requires rules tested over generated inputs, never around an example.
- **Find out, never assume.** A missing fact (a payout frequency, a rate) is fetched from a source that states it. Never show "unknown" or make up a number. Design from known facts: the Bank of Canada publishes a rate every business day, so the only failure is ours (fetch, parse), handled as a failure.
- **Strict at the edges.** External answers are read against the shape the source really sends, from recorded replies. A mismatch is a visible error naming the source and field. No silent coercion.
- **Failures must surface.** A failed read is a visible error, said once the next one fails too (`feeds::feed_failed`), never swallowed. An offline refusal (`BAGHOLDER_OFFLINE`) is a type, never words, and is never said.
- **Compute, don't label.** No "N waiting" or "left out" text; find the fact and calculate.
- **Read what the source states.** Dump one real reply and read every field before any watch-it-over-time research.
- **No arbitrary caps.** Handle any size by streaming in bounded memory. A limit is only a size the source states.
- **Progress, not limits.** A long or large operation shows progress and a cancel, and says failures where the action happened.
- **No wait-and-compare.** Defined math is proven by tests over generated inputs, never by a one-off comparison on the owner's data.
- **Goal, not method.** Take a brief's goal and choose the method that proves it. Don't build a heavyweight check just because a document named it.
- **The standard settles it.** Whatever established practice settles is built that way, never left "undecided". Examples: WCAG, Carbon, Material, NN/g, the category's products. Messages never vanish before they can be read; no timed errors.

### The screen

- Keep a clean, aligned, uniform screen. Add no labels, chips, disclaimers, arrows or columns that restate what's shown.
- Research the pattern (NN/g, Material, Carbon, Apple HIG, the category's products) before proposing any screen behaviour, then show it concretely.
- Never ship a state that offers something that doesn't work or hides data. Fix it without asking, keep the look, and never blame the owner's timing.
- No `title=` attributes (a test enforces it); use the app's own tooltip.
- Don't re-shoot README screenshots or the tour unless the owner asks.

### Trading and orders

- Real money. Every execution default is checked against what a trader expects: exits rest good-till-cancelled, and a protective order never depends on the app being up.
- When an agreement implies an unsafe outcome, say the consequence in one sentence at the time.

### Product direction

- More brokerages will come. Design broker-neutral now.
- Self-hosted first and always; a hosted version is possible later, so nothing in the core may assume one user.
- A public launch is not ruled out, so never hard-wire Wealthsimple as the only source.
- AI agents are an app-wide feature. The app imposes no limits of its own on the person's agentic trading.

---

## 5. Verifying a change

`CLAUDE.md` (Verifying a change) is the list. These are the parts that need care in practice.

- **Rust**, from `rust/`: `RUSTFLAGS="-D warnings" cargo test --workspace --locked`, `cargo clippy --locked --workspace --lib --bins`, and the book's oracles with `cargo test -p bagholder-book --features oracles`.
  - The committed `Cargo.lock` must be current: CI's first step is `cargo metadata --locked`.
- **Page**, from `web/`: `npm run check`, `npm test` (Vitest) and the full e2e in one run.
  - The e2e build is `npx vite build`, then `npx playwright test` against the server built from the worktree's `rust/`, on port 8791 with the signed-in project on 8792.
  - Run any new e2e test alone before the full run. The page's `read()` caches GET answers, so a test asking the same URL twice sees one request.
- **Plants.** For every rule you add, undo the change, show its test fails, then restore it. Put the results in the PR's brief table. The Architect replays these.
- **Capacity** (`target/release/bagholder capacity run <fresh dir>`, about 12 minutes on a Mac).
  - Every listed `OVER_BUDGET` entry must still be over by its measure, or the run fails ("take it off").
  - The rule test `every_operation_a_person_waits_on_is_timed` ties `waited_on` to the budget being a RAIL wait.
  - Bytes read are counted only on Linux CI.
- **Idle hour** (CPU and peak memory over an hour, a page open) for any server change.
  - It must be paired: master and the branch side by side, each on a fresh copy of the owner's data (`~/.bagholder-rust/{book,market,bagholder}.db`, never `session.json`).
  - Copy the `models/` folder into each copy, or the local language model downloads and swamps the measurement.
  - Run a warm-up hour first (the branch's one-time migrations), then the hour that counts.
  - The last pair: master 62.9 s and 119 MB peak, the branch 58.0 s and 110 MB.
- **CI** has eight jobs: rust, rust-macos, rust-windows, web, browser, capacity, container, audit. Read each failure's log; never re-run blindly.
  - Two flakes that were fixed: the macOS wall-clock sleep test, and a stale lock that only the container job's unlocked build had hidden.
- **Scratch servers:** use a copy of the data, never the live folder.
  - `BAGHOLDER_NO_BROWSER=1 BAGHOLDER_DRY_ORDERS=1 BAGHOLDER_HOME=<copy> BAGHOLDER_PORT=<n> target/release/bagholder`.
  - Open `http://127.0.0.1:<n>/`; `localhost` is refused by design.

---

## 6. Code map of the newest work (#525, "market data by listing")

- **`ListingKey` (`SYMBOL@VENUE`)** is the key of everything kept per listing. Venues are MIC codes plus `US`, `OTC`, `CRYPTO`, `INDEX` and `OCC`.
  - `listing::venue_of(mic, words)` is the one venue rule.
  - A symbol with no venue is resolved by `GET /api/listings`: one listing is taken only where every directory answered, otherwise the choices are offered with the failure.
  - Filings are kept per filer, `(source, id)`.
- **The book's names** (`rust/crates/book/src/names.rs`) are runs of sightings.
  - A venue-less sighting carries the previous run's venue.
  - A back-dated change re-forms names only up to the next sighting that states a venue, and past it through runs that read their words from the run before.
  - Every lookup names its partial index (`enum Stating`, `INDEXED BY`; migration 31).
  - A hand entry on a held instrument writes no name.
- **Matching a row by its day** (engine `named_on`/`listed_on` → `CalledOn { ids, on_the_day }`; server `entries::held_on_day`).
  - A row is matched by the name its instrument had on the row's day, else by today's name, which writes no name onto that day.
  - Two instruments with the name is a refusal, worded "was the name of 2 instruments on <day>" or "has been the name of 2 instruments since <day>".
  - A file row the book can't place is kept unplaced with its problem (`Payload.unnamed`), and the rest of the file imports.
- **Offline is typed end to end:**
  - `client::Error::Offline`, then `NetError::Offline` and `bagholder_net::Failure::Offline` (`Failure::{Offline, Said}`);
  - `SourceError::Offline`, and `Outcome::Offline`, which is never recorded;
  - `feed_failed` returns without saying anything for an offline refusal.
- **Wall time.** Waits measure on the wall clock (`app::Wall`; `wait` and `park_until_or` chain their clock readings). The market readers use `wall_now()`, which the `test-clock` feature lets a test move on.
- **Capacity** (`server/src/capacity.rs`):
  - `OVER_BUDGET` is `(op, pair, Measure, issue)`.
  - The depth operations are `backdated-trade` (in the deep holding's own account), `backdated-fill`, `backdated-record` (a distribution) and `backdated-file-row` (a listing named by code alone, `CODED`).

Older but still relevant:

- Order execution, the ticket and brackets shipped in v1.10.0.
- Wealthsimple's order operations were recovered from its public web bundle (`astprint.py` re-extracts them).
- A Docker image (`rust/Dockerfile`, `ghcr.io/professorbagholder/bagholder`) is version-checked against `APP_VERSION`.
- SEDAR+ is read headless through the browser helper (`bagholder-browser`), never a visible window. Its access details are in the code and its comments.

---

## 7. Environment notes

The Mac session's tooling quirks, so you can tell which apply on the server:

- **Mac sandbox only:**
  - `gh` had to run as one bare call per command (no pipes, loops, `&&` or `VAR=` prefix), or TLS failed.
  - Playwright had to run as a bare `npx playwright test` after a separate `cd`.
  - git over SSH went through `GIT_SSH_COMMAND="ssh -o ControlMaster=no -o ControlPath=none -o ProxyCommand='python3 ~/.ssh/sandbox-proxy %h %p'"`.
  - Ports other than 8791 were refused for Playwright's server check.
  - On the server, try the plain forms first.
- **CI wait.** A small Python script polled the public runs API for a commit (`actions/runs?head_sha=…`) and ended on every outcome. Each wait must end on every outcome.
- **Release builds:** `cargo build --release --bins` after `npx vite build`, because the server carries `web/dist`.
- **The vault.** The owner's cross-project knowledge base is the private repo `ProfessorBagholder/Claude-Obsidian`. Its rules are in the owner's global `~/.claude/CLAUDE.md`: verified-only, `bin/lint.py`, and nothing secret or financial.
  - Notes that matter here: `claude-code-sandbox-exclusions`, `trading-journal-trade-definition`, `time-zone-convention`, `count-work-per-thread-in-parallel-tests`, `sqlite-table-rebuild-foreign-keys`, `yahoo-*`, `cboe-*`, `bank-of-canada-rates-by-era`.
  - Clone it on the server if that session should read and keep it.
- **The stop hook.** The owner's `~/.claude/settings.json` on the Mac has a Stop hook (`Claude-Obsidian/bin/stop-if-tasks-running`) that refuses to end a turn while a background task runs. The owner may want it on the server too. A wording-check hook was tried and removed: it runs after the reply is shown.
- **Privacy:** the repository is public. Never commit credentials, account ids, emails, balances, `session.json`, databases or `.claude/`. Real tickers as example text are fine; the owner said so.
