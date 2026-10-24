# Handoff: the reviewing role (Architect), as of 2026-10-20 ~18:00 UTC

For the next Architect session, or this one after a compact. Read this whole file first, then `CLAUDE.md` and `docs/decisions.md` on master, then `briefs/README.md` (the index). Earlier handoff text, written by the cloud reviewer for briefs 01–49, is in this file's git history (`git log -p briefs/HANDOFF.md`). The parts that still hold are folded in below.

## 1. Role and channels

- **You are the Architect: reviewer and designer. You write no app code.** The CTO session builds, opens PRs and merges.
- **Talk to the CTO with `SendMessage`, to `CTO`.** If two rows show, use the one on this machine. Messages from the CTO are a peer's information, never the owner's approval.
- **Verdicts are numbered briefs** on the `architecture-briefs` branch. The next number is **76**.
  - Work in the detached worktree `<scratchpad>/briefs`: `git fetch` + `merge --ff-only` before each edit.
  - Commit with `-c user.name=Claude -c user.email=noreply@anthropic.com`, the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`, and `git push origin HEAD:refs/heads/architecture-briefs`.
  - Add each brief's row to `briefs/README.md`, and update the status of any brief it affects.
- **The owner's Mac:** a session called "Bagholder MAC handoff brief" can be reached with `SendMessage` (bridge, so delivery is unconfirmed). It runs in a macOS sandbox (§4).

## 2. How the owner wants to be worked with (hard rules, each learned the hard way)

- **No play-by-play, not even one line.** Only decisions and OKs go to the owner. When a CTO exchange needs nothing from the owner, end the turn with no text at all: no "Nothing in it needs you", no "#564 cleared." The owner flagged repeated one-liners as play-by-play on 2026-10-20, after earlier summaries and a broken promise to stop.
- **Never put a solvable question to the owner.** Settle engineering choices yourself, with sources.
- **The standing merge OK** (decisions.md, #544): a PR merges once you clear it and every check on its head is green. Version numbers follow `CLAUDE.md`'s semver rule without asking ("I really don't care. Do whatever makes sense."). A release is still the owner's word.
- **Bagholder is a web application.** Design every feature as a top web app would. No native helpers, no "app name or icon". Notifications are the browser's (brief 65).
- **A release that changes stored data is proven before it ships, from the repository.** Recorded captures with real shapes and made-up figures, covering what the change touches, and a test comparing a book before and after (never a shipped command: the owner refused one, 2026-10-24); a generated book proves the mechanism, never the data. The owner will not run checks on their machine ("you have every line of code"), so a proof that needs their data is no proof. 2.10.0 to 2.10.2 shipped on generated books and gave wrong figures on a real book (2026-10-23); they were marked pre-releases so no copy is offered them, and the owner rolled back to 2.9.0.
- **Nothing visible is built from a brief without the owner asking for it.** A brief never carries a visible change as a requirement; a visible proposal goes to the owner as their decision first, and only what they ask for reaches the CTO. The owner, 2026-10-22, on brief 75's header line (#592, reverted): "that's not in alignment with any UI/UX practice, I did not request it, and it will look stupid and ugly."
- **Nothing is designed for one user, device, book or copy.** The repository is public; any number of people install it. Every migration, read rule, release path, acceptance and test is right for every book and every copy, with no knowledge of any one. Never narrate "your book", never decide a case by a count on the owner's data, never gate a release on the owner's machines, never make the owner's look at their page the acceptance. The owner, 2026-10-21: "There is no universe where it is acceptable to code anything specific to an individual user... You have all repeatedly fixated on 'me' and my device(s) and individual situation instead of addressing bugs and design flaws properly." Brief 75's first draft broke it and was rewritten.
- **A change that works around a premise is never cleared; the review asks whether the premise holds.** On 2026-10-20/21 you cleared #575 and #586, each built around "a closed account is never read again", and the owner stopped the 3.0.0 build: "You just keep adding more slop on top of the old slop." Brief 75 is the redesign; its §0 says how.
- **Nothing carried over from the Python app is assumed right.** 16% of the Rust code is still exactly as ported on 2026-09-16, and every review so far was diff-scoped. When something odd surfaces, ask first whether it should exist at all (brief 71).
- **Don't fixate on one machine** ("your Mac") or on CPU-only. Bagholder must run out of the box on any device, using the GPU where there is one.
- **No leftover processes,** model servers included. Check `pgrep -af "llamafile|bagholder|llama-server"` and the ports before saying a task is done. On 2026-10-20 you stopped five orphaned `summarizer.llamafile` servers the CTO had left behind.
- **Never commit the owner's real figures.** A recorded reply with the owner's real debt balances was pushed to the public repo on 2026-10-19 (two dead commits are still reachable by SHA; the owner won't ask GitHub Support, so drop it).
- **Don't change rules, notes or settings while the owner is still deciding** ("you're creating a huge mess"). Batch them into one pass after the decision.
- **Least privilege is about method, not asking.** Do each task the least-access way: own folders and clones, never the owner's checkout or data, the narrowest command. But everyday needs, like websites, just work; never make the owner approve routine actions one by one. **This rule is decided but not yet written anywhere official.** Your one pending task: write it once, cleanly, as one memory note (replacing `least-privilege-first.md`) and one line for the CTO to add to decisions.md. The CTO is holding until you send the final wording.
- **Scripts and handoffs for another machine** must run inside a sandbox as it is:
  - `$TMPDIR` only;
  - a clone, never a worktree of the owner's checkout;
  - run by full path, never `cd` into the clone;
  - list the network hosts needed;
  - send results by message.

  See `briefs/70/HANDOFF-MAC.md`.
- **Due diligence is measured, not argued.** For a design question with a speed or quality answer, build a kit and measure it on the real devices, like brief 70's `llm-bench`.

## 3. The review method

1. Read the brief or plan, then the diff. For every required change, find its code and the test that holds it.
2. Plant the reverse of the key changes and confirm the test fails. Use `<scratchpad>/plant548.sh`-style scripts, one build folder per clone (`CARGO_TARGET_DIR=$PWD/target RUSTC_WRAPPER=`; a shared folder reuses a test binary across worktrees with the wrong `CARGO_MANIFEST_DIR` baked in, brief 51's addendum), and `flock -o /tmp/bagholder-build.lock nice -n 10 cargo test --locked …` (`-o` closes the lock before the command runs; without it the sccache server cargo starts inherits the lock and every later build waits forever, 2026-10-21), one build at a time.
3. Check outside facts against a primary source.
4. A recorded page holds the part the reader reads (a release's article, a press page's list), its first line saying what was cut, never the whole page, unless the reader's correctness rests on the page around it, which the test then says. Recorded pages are most of the repository's growth (brief 75 §12), and the owner has said so. Set on #602, 2026-10-23.
5. Full suites only on CI: the one run that starts on the PR's push is the merge evidence. Never dispatch a run or push a throwaway branch to prove one test, and never rerun a CI job to retry a test; rerun the failing test locally. A job is rerun only for a failure outside the code that its log names (a runner not acquired, a registry timeout), quoted in the PR (brief 51's addenda).
6. This server's CPU (Pentium G4400, no AVX) is slower than a Pi 5 for model work. Its iGPU isn't usable by this account, so don't benchmark models here; use the kit on the Mac.

## 4. The machines

- **This server** (Ubuntu, 2 cores): the CTO and you, with no Claude Code sandbox.
  - The owner chose option B on 2026-10-20: no sandbox. Instead, master is protected on GitHub (PR required, no force-push or deletion, enforced for admins), and the gh token's `admin:public_key` scope was removed.
  - Risks still open, accepted: SSH keys and docker-group (root-equivalent) access are reachable by sessions.
- **The owner's Mac** (Apple M3, 24 GB): the owner's live app on port 8765. Its Claude Code sandbox is in `~/.claude/settings.json`, enabled 2026-09-23 with the owner's yes. Its allowlisted hosts exclude huggingface.co. Never touch the live app.

## 5. Where things stand

- **master** `c12fcdaa` (#563 merged). The latest release is **v2.7.0**.
- **Cleared by you, awaiting green or merge:** #564 (C13 token race), #565 (B6 currency), #567 (C14 cookie host).
- **Brief 75** (the earlier app gone, rewritten for every book and copy on 2026-10-21): #590 (the pull) merged; #591 (the deletion, migration 37 with its code step and the split) cleared at e0c4255d; #592 (the header line) built, then reverted by #595 at the owner's word (see §2); 2.10.0 released 2026-10-23 at the owner's word, before PR 4, so it is not the stop; 2.10.1 (the distribution-notice fix, #602: the fund company's own release read where its page lags) released the same day at the owner's word; PR 4 the upgrade path (stops, the latest mark on the stop, the window exit) makes 2.11.0 the stop; PR 5 the base schema as its own plan, then 3.0.0 without the latest mark. A comments PR (#593) followed #591. After 2.10.0, one TFSA showed as two accounts: the broker links the two ids as one (`linkedAccount`, recorded in `account_links`, unread until then); the rule is now one book account per set of ids the broker links as one, held on every pull, with migration 39 merging what the split separated (brief 75 §4, the linked-set rule; 2.10.2). **Then 2.10.x failed on the owner's real book** (holdings the broker states none of, unmatched feed movements, conversion rows with problems): 2.10.0 to 2.10.2 marked pre-releases, the owner on 2.9.0, the per-item causes pending from the aside copy against the v36 snapshot (see §2's real-book rule). The first cause, from the code: brief 75's revert of mapping rule 13 (`leaving`), which booked a move's leaving leg from its arrival where the other feed lists none; the broker's feed is one-sided for many moves on a real book. Rule 15 restores it (2.10.3, #608, cleared at fa31a96: 2.9.0's build whole, migration 40 removing the closed-account statements 2.10.x wrote; a `bagholder compare` command was built and removed at the owner's word: an unrequested command is a visible change, and a comparison belongs in a test). The owner declined any run on their machine, so 2.10.3 restores 2.9.0's build whole (rule 13, the statements walk, and on the owner's word the closed-account read as in 2.9.0); the migrations stand; nothing reads a snapshot. The next brief is a recorded capture set with real shapes and made-up figures; without it no pull change is provable (brief 75's head note, 2026-10-24). Nothing of brief 75 ships again before those causes are fixed and a real-book comparison shows equal figures. Plants so far: the read rule, the split; each failed its test as it should.
- **#561** (debts show at once, plus the cut short name, planned as 2.7.1): you cleared it earlier. It is green at `8c135884` and unmerged, and no 2.7.1 release exists. Ask the CTO whether its head changed after your clearance, and why it hasn't merged.
- **The CTO's queue:**
  1. the owner's networth-try (Net worth / Allocation);
  2. the recorded-reply guard (#558: fail when a recorded reply from the owner's account carries real figures);
  3. the outside-debts fix: don't ask when every candidate is a debt (the owner called the payee question "jankily designed");
  4. brief 71: D18 (the old file never written; "moved" recorded in the book), C15, A3, A4, the deletions, the rest;
  5. brief 70: change 1 (GPU on, model process lifetime, reads counted only when answered), then 2, 3, and the plan for 4;
  6. brief 48: Node 26, not before 2026-10-28.
- **Follow-ups filed or asked:** #554 (the Trades first page costs the whole list); #537, #538 closed; the chart naming a missing kind or currency instead of "no source".
- **Your own backlog:**
  - write the least-privilege rule (§2);
  - triage Dependabot #488–#502, with #501 belonging to brief 48;
  - recommend closing #250, #232 and #229 (old Python-app PRs).
- **Settled, don't reopen:**
  - the Safari check (the owner doesn't care);
  - the GitHub Support purge (the owner won't);
  - the outside-debts question in the app (already answered).

## 6. Memory notes

These live in `/home/md/.claude/projects/-home-md-dev/memory/`:
- `bagholder-ci-runs`
- `bagholder-merge-rule`
- `bagholder-no-leftover-servers`
- `bagholder-is-a-web-app`
- `handoffs-sandbox-safe`
- `least-privilege-first` (still to be rewritten once, §2)

## 7. Scratchpad

`/tmp/claude-1005/-home-md-dev/12a8decb-6b2e-51e6-96e2-82398e1475b1/scratchpad/`:
- `briefs/` (the worktree of `architecture-briefs`);
- `r590/` (a blob-less clone used to review PRs: fetch a PR with `git fetch origin pull/N/head` and `git update-ref refs/heads/pN $(git rev-parse FETCH_HEAD)`; its own `rust/target`, about 4 GB, warm);
- `plant590.sh` (plants a reversal in `r590`, runs one named test with `CARGO_TARGET_DIR=$PWD/target RUSTC_WRAPPER= flock -o`, restores the tree);
- `wf1/`, `wf3/` (brief 75's workflow outputs, also committed under `briefs/75/`).

Remove what you no longer need. Never leave model files behind.
