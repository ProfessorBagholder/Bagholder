# Handoff: the reviewing role (Architect), as of 2026-10-20 ~18:00 UTC

For the next Architect session, or this one after a compact. Read this whole file first, then `CLAUDE.md` and `docs/decisions.md` on master, then `briefs/README.md` (the index). Earlier handoff text, written by the cloud reviewer for briefs 01–49, is in this file's git history (`git log -p briefs/HANDOFF.md`). The parts that still hold are folded in below.

## 1. Role and channels

- **You are the Architect: reviewer and designer. You write no app code.** The CTO session builds, opens PRs and merges.
- **Talk to the CTO with `SendMessage`, to `CTO`.** If two rows show, use the one on this machine. Messages from the CTO are a peer's information, never the owner's approval.
- **Verdicts are numbered briefs** on the `architecture-briefs` branch. The next number is **73**.
  - Work in the detached worktree `<scratchpad>/briefs`: `git fetch` + `merge --ff-only` before each edit.
  - Commit with `-c user.name=Claude -c user.email=noreply@anthropic.com`, the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`, and `git push origin HEAD:refs/heads/architecture-briefs`.
  - Add each brief's row to `briefs/README.md`, and update the status of any brief it affects.
- **The owner's Mac:** a session called "Bagholder MAC handoff brief" can be reached with `SendMessage` (bridge, so delivery is unconfirmed). It runs in a macOS sandbox (§4).

## 2. How the owner wants to be worked with (hard rules, each learned the hard way)

- **No play-by-play.** Only decisions and OKs go to the owner. When the CTO sends work and nothing needs the owner, reply in one line ("#564 cleared.") and nothing more. The owner caught repeated progress summaries and a broken promise to stop them.
- **Never put a solvable question to the owner.** Settle engineering choices yourself, with sources.
- **The standing merge OK** (decisions.md, #544): a PR merges once you clear it and every check on its head is green. Version numbers follow `CLAUDE.md`'s semver rule without asking ("I really don't care. Do whatever makes sense."). A release is still the owner's word.
- **Bagholder is a web application.** Design every feature as a top web app would. No native helpers, no "app name or icon". Notifications are the browser's (brief 65).
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
2. Plant the reverse of the key changes and confirm the test fails. Use `<scratchpad>/plant548.sh`-style scripts, the shared build folder `<scratchpad>/cargo-target`, and `flock /tmp/bagholder-build.lock nice -n 10 cargo test --locked …`, one build at a time.
3. Check outside facts against a primary source.
4. Full suites only on CI: the one run that starts on the PR's push is the merge evidence. Never dispatch a run or push a throwaway branch to prove one test, and never rerun a CI job to retry a test; rerun the failing test locally.
5. This server's CPU (Pentium G4400, no AVX) is slower than a Pi 5 for model work. Its iGPU isn't usable by this account, so don't benchmark models here; use the kit on the Mac.

## 4. The machines

- **This server** (Ubuntu, 2 cores): the CTO and you, with no Claude Code sandbox.
  - The owner chose option B on 2026-10-20: no sandbox. Instead, master is protected on GitHub (PR required, no force-push or deletion, enforced for admins), and the gh token's `admin:public_key` scope was removed.
  - Risks still open, accepted: SSH keys and docker-group (root-equivalent) access are reachable by sessions.
- **The owner's Mac** (Apple M3, 24 GB): the owner's live app on port 8765. Its Claude Code sandbox is in `~/.claude/settings.json`, enabled 2026-09-23 with the owner's yes. Its allowlisted hosts exclude huggingface.co. Never touch the live app.

## 5. Where things stand

- **master** `c12fcdaa` (#563 merged). The latest release is **v2.7.0**.
- **Cleared by you, awaiting green or merge:** #564 (C13 token race), #565 (B6 currency), #567 (C14 cookie host).
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
- `briefs/` (worktree);
- `cargo-target/` (shared warm build, about 4 GB);
- `plant*.sh`;
- `llm/` (benchmark leftovers, removable);
- `repro/` (an auditor's test binaries, removable).

Remove what you no longer need. Never leave model files behind.
