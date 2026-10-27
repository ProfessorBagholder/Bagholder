# Handoff: the CTO session for Bagholder

Written 2026-10-18 by the CTO session that ran on the owner's Mac; section 2 last rewritten 2026-10-26; and the home-server lessons updated the same day by the home-server CTO session for the next one. Read this whole file once, then `CLAUDE.md`, `docs/decisions.md`, `SPEC.md` (as needed), `docs/architecture.md` and `briefs/reference.md` on the `architecture-briefs` branch.

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

## 2. Where things stand (2026-10-26 ~23:10 UTC, written by the home-server CTO session before a compaction)

### Released, newest first (`gh release list` is the truth)
- **v2.10.12** (#629, merge a446c1a2): a booking the statements walk made from a statement, which the feed now states, is withdrawn (`withdraw_stated_by_the_feed`, before `restore_withdrawn`). Every kept row is paired with all the account's transactions in the kept months' span, feed first, and only a booking left unpaired is withdrawn. The gate on a copy of the owner's book: the walk withdrew 5 bookings, and the USD line was gone with nothing new or worse. Published 20:04, Latest, 8 assets.
- **v2.10.11** (#624): a pending card spend (`SPEND`/`PREPAID`) holds its amount. The owner's CAD line cleared.
- **v2.10.10** (#622): takes back 2.10.9.

### Merged today without a release
- #625: the made-up broker states only recorded statement forms; `Shape::Assigned`.
- #626: suites run once (PR on Linux; macOS and Windows on the release tag).
- #627: the PR jobs read master's shared caches (`suite`, `demo-server`) without saving; `suite-cache.yml` builds them on a master push on a miss; capacity is not re-measured on master.
- #628: `tests_figures_golden.rs` and `tests/golden/figures.json` (12 seeds, header lines sorted). `BAGHOLDER_BLESS=1` writes the file. Building and running it takes about 3 minutes here.

### The owner's book
- On 2.10.12 the book should have no header line once it syncs. But a gate copy taken at 22:18 still showed the USD line, so the owner hadn't synced 2.10.12 yet. They were told to update and sync.
- Never say "fixed" until the owner's screen or a gate shows it.
- **Privacy:** never name the owner's securities, coins or account nicknames in anything pushed (handoff, commits, PRs, issues). The owner said "forget about scrubbing history": never rewrite history or scrub old files.

### Brief 77, what is left, in order
1. **#631 (§4, the gated rederive): merged** (730abe16), no release.
2. **#632: merged** (b9017f9c), no release; it ships with (c).
3. **#633 = (c): CLOSED unmerged.** The canary on the owner's book held it; the closing comment states the four causes by rule, with no figures. Branch `statement-fills` kept on origin for reference (`delivery()`, the recorded `contract_of`, the take-back run once); its local worktree and branch are gone.
   - **The causes, by rule** (from a before/after diff of a read-only copy):
     1. The supersede took the statement's cash for coin fills, but a coin fill's statement cash is before Wealthsimple's fee, and the feed's amount is the cash that moved.
     2. The omitted-fill booking took fills the feed does list: where the feed's share security isn't called by the statement's symbol, and where a coin fill's units and cash both differ.
     3. `book.store` under 2.10.9's `fill_key` restored the records 2.10.10 took back.
     4. In the walk, `fills_of` ran before (b)'s pre-pass and changed a currency that agreed.
   - **The Architect's redesign (2026-10-26):**
     1. The statement's cash never supersedes the feed's. `FeeFolded` (invented) goes, replaced by the recorded coin forms (the fill before the fee, with "Fee charged $F" where written). 2.10.8's units supersede stays.
     2. (c) = identity pairing as a MATCHING rule only, inside `match_rows`: a statement fill row pairs with the feed's fill of the same side, security and units on the row's day or within the posting window, whatever the cash. Nothing booked or superseded from it; a month closes by its balances as now. The golden should show header lines leaving with no figure moving.
     3. Guards: the fills step after the pre-pass and only for currencies still differing; a statement row booked only where the month then closes; a book rule, with its own test, that a record a take-back removed is never made live by a store under its key.
     4. Captures first, from recorded forms only: how the feed names a share security the statement calls by its symbol (`names_it` failed on two such), against the positions; and the coin fill forms.
   - **Captures done** (Mac, read-only; real names and figures came to this session only, never pushed):
     - **Share shape:** a Canadian listing interlisted in the US. The feed's fill rows (DIY_BUY/DIY_SELL) and their `security.stock.symbol` say "SYM.TO", in CAD. The statement row says "SYM - NAME: Bought|Sold N shares (executed at D)" (also "… at $P per share"), in CAD, no FX rate. The book also holds the US listing under bare "SYM" in USD, so `names_it` matched the US one and the old rule booked every row there.
     - **Share variant:** two CAD listings of one company, "SYM" (earlier) and "SYM.TO" (later).
     - **Coins:** statement units at ten places equal the feed's `assetQuantity`. Cash in two forms: (a) with "Fee charged $F", equal to the feed's amount; (b) without fee text, the fill before the fee (smaller on a buy, larger on a sell).
   - **The Architect said PROCEED (2026-10-26 ~23:05), with these precisions:**
     - The matching pairs a statement fill row with the account's own FEED FILLS, never the instruments table.
     - Pair on: same side and units, the row's day or the posting window, an instrument called the row's symbol or the symbol with ".TO", and cash in the row's currency. Where two CAD listings carry the names, the fill on the day decides. No row is ever booked under a listing.
     - Record the ".TO" row form with its `security` object as the feed states it, AND the two coin sales' feed sides (form (a) rows that paired with nothing; not yet seen), before any rule claims them. A form not recorded is recorded first, never typed.
     - The three guards go in with the rule, each with its test.
     - The golden diff must be header lines only. If a figure moves, stop and say which.
   - **Next, in order:**
     1. Record the forms: the feed's "SYM.TO" row with its `security` object, made up, into `wealthsimple/tests/replies/wealthsimple/` beside the recorded feed kinds. Then a Mac capture of the two coin sales' feed sides; extend `scratchpad/diag/capture.py`, run it on a made-up book first, and print only the needed fields with id-bearing keys dropped.
     2. Generator shapes from them: the ".TO" listing; `CoinFeeCharged` (a); `CoinBeforeFee` (b), replacing `FeeFolded`. Branch from master (#633's `statement-fills` is only for reference).
     3. The twelve-seed sweep on 2.10.12's walk, to show the stops ("a buy only the statement states").
     4. The matching rule inside `match_rows`. Guards: (i) the fills step after the pre-pass and only for currencies still differing; (ii) a statement row booked only where the month then closes; (iii) a book rule, with its own test, that a record a take-back removed is never made live by a store under its key.
     5. Golden diff, header lines only.
     6. Version bump, the canary on the owner's book (Mac gate; `pull-broker --statements-only` runs the walk AND the fills step), PR, Architect, release.
   - Reusable from #633's branch: the recorded `contract_of` form, `delivery()` for an assignment's shares, the take-back once per book.
   - The diag scripts and every scratch file holding the owner's data were deleted on 2026-10-27. The gate prompt pattern is in the memory file `bagholder-delegation-playbook.md`.
   - **Never** name the owner's securities, coins or account nicknames in this file, a commit, a PR or an issue: the repository is public (a push naming two of them was caught and replaced on 2026-10-26).
4. The agents build (worktree `aa`, branch `agents-anywhere`, local WIP 87d3a815, never pushed). It must include in-app setup, and the owner checks one real connection per agent before release.

### Since the compaction (2026-10-26 evening, UTC into 2026-10-27)
- **(c) dropped** (Architect's go): no recorded fill row states other cash than the feed's in a way identity pairing would close; its only source was the typed `FeeFolded`. Its three guards went with it.
- **#634 merged** (aeccf555), no release: `FILL_FORMS` = `Interlisted`, `Coin2025`, `LimitOrder`, `SplitFill`, each from recorded forms. The typed `FeeFolded`/`FeedOmits`/`FxFunded` are `READ_BY_2_10_9`, only so `tests/take_back.rs` can re-read its two stored books; they go with `take_back_2_10_9`. The split fill's rows were recorded again with their zero. The notation keeps a zero as `0` and writes a fee's coin symbol as `SYM`. `feed-fill-dot-to.json` sits beside the statement forms, where no replay reads it. The golden gained gross units bought/sold per account and symbol; its header lines went from 373 to 2.
- **#635 merged** (341c0608), **v2.10.13 released** 2026-10-27 ~02:10 UTC: every release job green, 8 assets, Latest. The owner was told to update and sync; expected on their screen: the USD trading line gone and the June 2026 round trip back. Not yet confirmed by them.
  - The units step reads no fill from a row stating no units. At every pull it first withdraws (`Book::withdraw`) a statement fill record whose row reads as no fill or as the feed's units; `Book::mapped` gives the feed record's units.
  - `fills` returns (taken, given back), and the offline walk prints both.
  - Canary PASS: the candidate gave back the copy's zero-unit fills (none left), and the header went from one line to none.
  - The owner's live book still had the USD trading line at 01:10 UTC, so it had had no 2.10.12 sync. The owner should update to 2.10.13 and sync.
- **Settled from recorded sides:**
  - (a) Later-form coin purchases: the feed's units are the executed units rounded to 6 places; the units step misses them (it pairs by cash, and the fill row is before the fee). Sales' units are equal.
  - (b) The coin limit sells' feed rows carry only the placement date (one revision, no execution field); their units are rounded too. The walk's 31-day pass pairs them; the step's 7-day window does not.
  - The "cent under" on later-form coin purchases held in that form's first month only.
- **Next (Architect's order):** the coin units.
  1. A generator commit: every coin fill, in both forms, has its executed units to 10 places on the statement and rounded to 6 in the feed.
  2. The sweep shows which fills the step misses: later-form purchases and limit sells.
  3. One change, a subtraction: the units step pairs through the walk's own `match_rows` (combined pass, 31-day late pass) instead of its own pairing. The statement fill record carries its row's day, so a superseded limit sell is dated at execution. A first-month purchase a cent under pairs with nothing and keeps the feed's units; say so in the PR.
  4. Then PR B.

### Brief 78 (the owner's go, 2026-10-27)
- **§1, #636 merged** (856d409d), **v2.11.0** tagged as its own release: `BAGHOLDER_HOSTS` lists exact host names a proxy on the same machine may send. `host_ok` answers a listed name only from loopback (`peer_ok` unchanged); `app::parse_hosts` refuses the start on a bad entry. Docs: README "From another device", architecture §12, decisions.md (brief 15 d3 struck as superseded).
- **§2, the preview tooling: waits for the owner** to name the Cloudflare account and domain. Never use `~/.cloudflared/cert.pem` (2025) or the running `cloudflared.service` (memory `cloudflare-account-confirmed-first.md`). Fresh credentials from the named account go only in the tool's own folder (`~/.claude/tools/preview/`, mode 600), never the repository. Facts checked in Cloudflare's docs: a named tunnel routes by hostname (a leading wildcard is allowed, a catch-all rule is required); its ingress has no hot reload, so a route change is a replica swap (start the new connector, wait until it serves, stop the old); `originRequest.access` (`required`, `teamName`, `audTag`); Access applications take a wildcard; Quick Tunnels buffer SSE, so they are out. Then: a shared `CARGO_TARGET_DIR` under flock/nice, measure command-to-link minutes, a live run from the owner's phone, one link reported, `preview --gc` only if anything were left.
- **The smoke test's leftovers were removed on 2026-10-27:** the portless proxy stopped, `~/.portless` deleted, the package uninstalled, `/tmp/portless-smoke` and its worktree removed. Three things need the owner (sudo or refused by the permission check): portless's CA in the system trust store, Node v24.22.1 under `~/.nvm`, and the test folders in `/tmp`. They were given one line to run.
- **v2.11.0 published** 2026-10-27: every release job green, 8 assets, Latest (`releases/latest` confirmed).
- **Tests' temp folders: #637 merged** (a60aa696), no release (tests only). Single tests take `tempfile::tempdir()`. The two run-long homes live in `tests_common::run_folder`, removed by `atexit`. `bin/check-test-leftovers.sh` (from #589) runs the Linux suite in tests.yml and the macOS/Windows suites in release.yml on a fresh temp folder, and fails on anything left. The browser job is out, with the reason in its comment. The release suites first run under it at the next tag. A run killed by a signal still leaves its one folder.
- **Refused by the permission check, put to the owner:** closing #589 as superseded (an outside write I hadn't been asked to make; the owner can say "close #589"), and the /tmp sweep of the 396 `bh-*`/`bagholder-*` folders older than a day. The owner was given one line covering that sweep, portless's CA (sudo) and `~/.nvm/versions/node/v24.22.1` (nvm's default is `node`, i.e. v26, so nothing depends on it).
- **A sign-in bug, proven, waiting on the owner's go** (they were signed out after updating to 2.10.13; their app runs on their Mac): at start, `boot_session` with a saved, unexpired sign-in and no answer from Wealthsimple leaves `connected` false. `heard(NotAsked)` keeps `connected && live`, and `connected` starts false. The page then offers "Connect Wealthsimple", which opens the sign-in Chrome window. A throwaway test showed connected=false, error "unreachable: the connection dropped", and the saved login still on disk. The rule dates from 8ae43bed (2026-10-04, 2.2.2). Whether this is what hit the owner is unknown; the app logs no reason when it drops a sign-in. Proposed to the owner: at start, an unexpired saved sign-in counts as connected until Wealthsimple refuses it, and an unanswered check is retried; plus a log line saying why a sign-in is dropped. Not built until they say go.
- The sign-in window itself is not new: `login::start_login_browser`, reached only from the page's "Connect Wealthsimple" (menu or empty screen). It uses its own profile in the app's folder and picks Chrome before Brave (`find_chrome`), as it has since the Rust port (2026-09-16).

### Delegated
- **PR A (#630): merged** (cc28ea70), no release. It covers the exponent cap, `take_frame` returning `io::Result`, four clippy denies, and CI's clippy running with `--features bagholder-ws/standin` (the Architect's condition).
- **PR B** (the wider lints): after brief 77.

### Other follow-ups
- The header's lines come in a different order from one opening to the next (#628 sorts them in the golden file only). This needs an owner decision before anything visible changes.
- `take_back_2_10_9`: make its SQL filter one query, and remove it at the next stop release.
- The version-36 carry with a superseded coin fill.
- Older items, not rechecked: the `install-check` branch, the short-interest bug (options sent to FINRA), PR #589, and the open dependabot PRs.

### Delegation, learned today
- Cloud sessions cannot message back. Never tell one to `send_message`; say "ask for no permission"; read its transcript. Start one on a branch pushed beforehand (`source_revision` + `outcome_branch`), because its classifier refuses `git switch -c`.
- Never archive a session before its report reaches me.
- **The Mac gate.** Run it from a bridge named `air:Bagholder:…`; look it up with `list_environments`. Then:
  - Back up a read-only copy with `sqlite3 -readonly .backup`.
  - Build both versions from a clone. For a walk fix, the candidate gets `--features pull-broker`, and `pull-broker <copy> --statements-only` runs before it opens.
  - Run each server inside one polling shell with `BAGHOLDER_EXIT_WITH=$$`; never use `kill`.
  - Offline the app never reports ready, so stop polling once `error` has been unchanged for 60 s.
  - Report the walk's whole output.
- **Merging.** A merge whose commit writes a new workflow file blob is refused (the token has no `workflow` scope). Merge master into the branch first and push; git pushes are allowed.

### Workspace
- Worktrees in this session's scratchpad: `aa` (agents), `g558` (paused, the owner's call), `ho` (this file).
- The shared cargo target is `/tmp/claude-1005/-home-md-dev/2645431b-0517-51b8-a5b6-289b4b14d9d0/scratchpad/cargo-target`. It was removed at 196 GB on 2026-10-26 because the disk filled; on 2026-10-27 its `debug/incremental` (19 GB) was removed, leaving 16 GB. Keep it under ~30 GB (`du -sh`), and remove it when the day's branches merge. Build with `RUSTC_WRAPPER= CARGO_TARGET_DIR=<it> flock -o /tmp/bagholder-build.lock nice -n 10 cargo …`, and only the named tests a change touches. The server crate is a bin: `-p bagholder-server --bin bagholder <filter>`.

### How a release goes (all delegated: Architect clearance + green checks = merge and release)
- The version bump goes inside the fix PR, so CI runs once: `rust/Cargo.toml` `version`, `rust/crates/server/src/app.rs` `APP_VERSION`, `rust/crates/server/tests/golden/routes.json` `"version"`, then `cargo update --workspace --offline`.
- Merge with `gh api -X PUT repos/ProfessorBagholder/Bagholder/pulls/N/merge -f merge_method=merge -f sha=<cleared head>`.
- Then `gh release create vX.Y.Z --target <merge sha> --prerelease --title vX.Y.Z --notes "…"`. release.yml builds 8 assets and marks it Latest; confirm with `gh release view` and `gh api repos/ProfessorBagholder/Bagholder/releases/latest` before telling the owner it is out.

## 3. How the owner wants to be worked with

Each rule below cost the owner real anger, often more than once. Treat them as hard rules.

### Added 2026-10-19 and 2026-10-20 (also in the session's memory files)
- **When the owner flags something:**
  1. Play back your understanding and the intended fix first.
  2. Then the quickest working fix on a test branch, with ONE copy-paste command for them to try it.
  3. Only after their green light: the full process (tests, PR, Architect, CI, merge, release).
- **Say exactly what is and isn't in every proposal or handoff.** Never a silent "later". Never promise something and then not deliver it, and check each promised item against the result (a settled screenshot) before handing over.
- **Answer the question asked.** Don't rush into changes before things are decided: "answer the question instead of rushing to make changes before things are decided".
- **Never commit real figures from the owner's account.** The repo is public. Commands given to the owner print no ids, names or amounts.
- **Nothing of the earlier (Python) app is kept** (2026-10-20). A migration converts once and leaves nothing of the old format behind.
- **Added 2026-10-21, all in the memory files too:**
  - Subtract before adding: find the wrong premise and delete it, never stack mechanisms; size is a criterion (`bagholder-subtract-before-adding`).
  - Look on the machine before answering; never agree to a claim without checking it (`check-the-machine-before-answering`).
  - The delegation playbook: do it myself by default; the Mac bridge's known limits; one full first prompt; checks stay checks (`bagholder-delegation-playbook`).
  - Sessions clean up after themselves and report back via send_message @parent (`sessions-clean-up-themselves`).
  - No unnecessary CI runs: every push is final, run the touched tests locally first, commit named files only, cancel superseded runs (`bagholder-ci-runs`).
  - Explain outcomes in plain words, not plumbing. Never make claims about other users' data.
  - Bagholder runs on any platform. The owner runs it on a Mac and tests on Windows matter. Don't call Windows Server "Windows".

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
- **Done means every acceptance criterion has evidence.** That includes CI read to the end. Never write "not measured in this session".
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

### Lessons from the home server (2026-10-18)
- **Brief 51 is binding.**
  - Locally, run only the named tests and plants; never a whole crate or suite.
  - Wrap every cargo, npm build and Playwright run in `flock /tmp/bagholder-build.lock nice -n 10 …`, since the Architect builds on the same 2 cores.
  - One CI run per merge head, the automatic one; never dispatch CI and never push throwaway branches.
  - No cloud sessions, and nothing on the owner's Mac.
- **Shared cargo target:** `scratchpad/cargo-target`. Each worktree symlinks `rust/target` to it, and Playwright's `serve.mjs` builds into it too. One worktree per PR, removed after merge.
- **Restoring a planted file:** `touch` it after copying it back, or cargo keeps the planted binary (the restored copy carries the old mtime).
- **Plants must be able to fail.** A test that makes its rows unique by the very columns being planted out passes the plant. State the test's own identity.
- **Recorded replies of a new kind go in a folder of their own.** Each `replies/<source>` folder's shape test is the union of everything in it.
- **gh here has no `--json` on `pr checks`:** read the workflow runs by head sha instead (`merge-when-green.sh`).
- **The owner:**
  - no extra or repeated runs beyond what the evidence needs;
  - no pages or artifacts made for them that they won't use;
  - clean up merged worktrees and scratch files;
  - say plainly when a promised step hasn't started, and then do it.

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
- **No idle hours.** Never measure a change by running the app idle for an hour or more; the owner rejected it outright on 2026-10-19 (`docs/decisions.md`). Idle cost is held by `TIMED_WAITS` and the capacity job.
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
