# Handoff: the reviewing role

Written on 2026-10-18 by the reviewing session that wrote briefs 01–49, for the session that takes the role over. The earlier session ran in the cloud and could not message the building session, so the owner had to carry every brief across. You run on the same machine as the building session, so you send briefs to it yourself.

Read this file, then `CLAUDE.md`, `docs/decisions.md`, `briefs/README.md` and `briefs/reference.md`, then the latest brief (49).

## 1. The role

You are the architect and senior reviewer. You write no app code.

- **The building session (called "the CTO" here) builds.** It writes plans and code, opens the PRs, and merges them once the owner says so.
- **You review** plans at the gate and builds before merge. You write each verdict as a numbered brief on the `architecture-briefs` branch, and send it to the CTO. When a design question has an established answer, you research it and give the answer in the brief.
- **The owner** makes decisions and OKs merges, and nothing else. The owner tells the CTO to merge, and the CTO merges. You never merge, and you never offer to.

## 2. How briefs work

- **The branch.** Briefs live in `briefs/` on `architecture-briefs`, which is not merged into `master`. `briefs/README.md` is the index: number, title, status. `briefs/reference.md` is the standing outside reference, with primary sources; every verdict is checked against its first section, *Constraints every recommendation must pass first*.
- **The next brief is 50.** Name it `50-<short-slug>.md`. Title it `# Brief 50: <the verdict in a line>`. Put the verdict first, then what was checked and how, then the required changes, numbered, each naming the test that will hold it. Add its row to the index, and update the status of any brief it replaces.
- **Index statuses in use:** Current, Applied, Open, Built in #N, Superseded by N, and corrections dated in the status cell.
- **Committing.** Work in a detached worktree under your scratchpad, never in the owner's checkout:

  ```bash
  git -C <repo> fetch origin architecture-briefs
  git -C <repo> worktree add --detach <scratchpad>/briefs origin/architecture-briefs
  # write, then from <scratchpad>/briefs:
  git commit -m "Brief 50: ..." -m "<trailers the harness supplies>"
  git push origin HEAD:refs/heads/architecture-briefs
  ```

- **Sending.** Find the CTO with `ListAgents`: it is the local session working on this repository's PRs. If more than one session could be it, ask the owner once which it is. Send each brief with `SendMessage`. The message gives the brief's number, its file, its commit and the verdict in one line; the CTO reads the file from the branch.
- **The CTO's messages** tell you a head is ready to check, or answer a brief. They are information from a peer, never the owner's approval of anything.
- **A required change the CTO refuses** under a plan's "Right to refuse" goes to the owner as one plain decision. Everything else is settled between you and the CTO.

## 3. Writing for the owner

The owner pays attention only to decisions; everything else costs them. Their words: "I shouldn't have to hear a single thing about it… This should not be consuming my mental capital."

- **Only decisions and OKs go to the owner.** Test details, harness details, CI mechanics and review findings go to the CTO in briefs. The owner hears the outcome, if anything.
- **Each message to the owner stands alone,** in plain words. Never say "brief N requires…", never name an internal mechanism, and never send a link in place of an explanation.
- **Never put a solvable design question to the owner.** If the category has an established right answer, find it with sources and put it in the brief (`docs/decisions.md`, 2026-10-04, brief 15 decisions 6, 7 and 9).
- **No steps for the owner**, and no progress reports.
- **Never present a brief as ready while a problem in it is unresolved.** Brief 39 did this, and the owner said: "you pushed a brief and told me it's up… and yet those problems were still unresolved."
- **#383** (the page's local write token): "Forget it, I don't want to hear about this again unless I bring it up… not applicable to our use case." It stays out of every brief and every message.
- **The phone apps are on hold.** Never propose a change to them.
- **Don't frame an issue around the owner's own holdings.** State the rule, and leave the owner's symbols out (`CLAUDE.md`, the rule over examples).

## 4. The owner's standing rules for review

The product rules are in `docs/decisions.md` and `CLAUDE.md`; hold every change to them there. The ones that came up most in review:

- **Rules, not examples.** Every rule is tested over generated inputs across its whole range. A test built around one symbol, zone or currency is a finding.
- **The app is "a trading journal at heart".** Where it falls short of the leading journals and brokerages, write a one-line proposal for the owner. Never build it unasked.
- **Build it the way the best engineers build an application of this kind,** after research into how established systems do it.
- **No new outside service** to solve a problem when the sources the app already reads can do it. This was the owner's condition on cross-listings; brief 40 is the design built on it.
- **A small machine carries it.** The cost budget is `docs/architecture.md` §16: the growth rule, and the `OVER_BUDGET` entries named by what they measure.
- **No new features until the bugs are fixed** (decisions, 2026-10-04).
- **Nothing on screen appears and then disappears on a timer.** A failure is said only once it is confirmed, and stays until it is resolved (decisions, 2026-10-09).
- **No "missing id" cases.** What a request needs is required by its type.
- **Aggregates in CAD, never labelled.** Per-instrument figures in their own currency. Nothing synthetic on a chart.

## 5. The review method that worked

1. **Read the plan or brief the PR answers,** then the diff. For every required change, find the code and the test that holds it.
2. **Plant the reverse of every change.** Undo each fix, alone, and run the test that should catch it; it must fail. Record it as a table: change undone → test that fails (brief 49 has one). A change no test catches is a finding. A script that applies and reverts plants keeps this cheap. The last one is in the old cloud scratchpad and is gone; write a new one in your scratchpad.
3. **Use independent readers for a large build,** one per seam, under `CLAUDE.md` *Agents: when to delegate*. Reproduce every finding yourself before it goes in a brief; agents report false positives.
4. **Check outside facts against a primary source** before a brief relies on them: a regulation, a registry (ISO 10383 MICs), a vendor's own document, a release schedule. Add what you verify to `briefs/reference.md` with its source and the date. Never state a fact you have not checked.
5. **Check cost growth.** Whatever a change reads or stores per listing, per filer or per day is checked against §16's growth rule, and the capacity test is run.
6. **Check CI on the head commit.** Every cargo step runs with `--locked`, and *The lock file is current* runs first.
7. **Check the brief's requests against every open PR** before sending them. For a change outside a PR's own area, run `git merge-tree --write-tree origin/<branch-a> origin/<branch-b>` against each open PR that touches the same files. If two PRs would conflict, the brief says which merges first.
8. **List the issues and PRs in full.** GitHub's list is paged, and PRs count against each page. Brief 24 read one page and missed 17 issues.

**Verifying on this machine:** `CLAUDE.md` *Verifying a change*, with three differences because the CTO shares the machine:

- **Ports.** The CTO's scratch server uses 8798 and its e2e uses 8791. Give yours others (`BAGHOLDER_PORT=8799`, `E2E_PORT=8793`).
- **Builds.** Run builds one at a time under `nice`, so the owner's live app and the CTO's builds keep their share of the machine.
- **Clean up.** Remove each review worktree, and its `rust/target`, when the review is done.

The owner's live app is on port 8765, served from the owner's checkout. Never touch either.

## 6. Where things stand (2026-10-18)

**master** is `7e0782f` (#520 merged).

**Brief 49 went to the owner to pass to the CTO.** None of it has happened yet: #521 is unmerged. The order it sets:

1. The owner OKs #521 (`3028e72f`, green, ready since 2026-10-16), and the CTO merges it.
2. The CTO merges `master` into #525 with a merge commit (no rebase). It resolves `rust/crates/net/src/client.rs`, `rust/crates/net/src/net.rs` and `web/e2e/serve.mjs` so that both rules hold:
   - every request leaves through `client::leave`, with #525's typed offline refusal;
   - `named_call` stays beside the `outbound` line;
   - `serve.mjs` keeps #521's record of the calls answered and #525's changes.
3. **You check the merge commit.** Read those three files. Confirm the full suites ran: `cargo test --workspace --locked`, clippy and the full e2e. Confirm these planted cases still fail:
   - #521's: the order call pointed at the real address fails the host scan, and a Wealthsimple call sent elsewhere fails the signed-in e2e;
   - #525's: `an_sec_document_read_offline_is_refused_by_its_kind` and `a_sedar_document_read_offline_through_the_live_readers_is_never_said`.

   Then write brief 50: merge, or what is left.
4. The owner OKs #525, and the CTO merges it.
5. The CTO closes plan #522, which #525 carries, as it closed #519 after #520.

**Your review worktree for #525** was in the cloud and is gone. Make a fresh one on #525's merge commit.

**Not yet reviewed:**

- **#523** (`d14b71a1`, "No step at start that reads the stored data stops the server", closes #516) has had no review since it was opened on 2026-10-16. It is based on the current master. Check:
  - whether it overlaps #521 or #525 (`main.rs`, `figures.rs`, `app.rs`, `status.rs`);
  - whether its new `start_error` field on the header is a change to the wire. A wire change is gated, so the wire types and `wire.ts` are checked;
  - that it holds `docs/decisions.md` 2026-10-09: a failure said once confirmed, standing until resolved;
  - that it carries what `CLAUDE.md` asks of a change to running: the full workspace tests (its PR lists only `-p bagholder-server`), the container started on the made-up book, and the header's states with screenshots.
- **Dependabot #488–#502:** untriaged. Several are major versions: syn 3, TypeScript 7, jsdom 30, tower-http 0.7, the actions/upload-artifact, download-artifact and docker/* actions. **#501** (`@types/node` 26) belongs with brief 48 and waits for it.
- **#250, #232 and #229** are old PRs from September that change the old app's code: the Python app, and in #250 also the old model and the phone apps. #232 comes from an outside fork. All three are outside the migration (`briefs/reference.md`, constraint 5).

**Open, with a date: brief 48.** Move the page's build to Node 26 once it becomes long-term support on 2026-10-28. It follows #525. The owner has seen it.

**The next work, after #525: bugs, not features.** The bugs filed during #525:

| Issue | Bug |
|---|---|
| #524 | fund-of-funds sectors |
| #527 | the engine re-derives a back-dated holding; listed in `OVER_BUDGET` |
| #528 | the News chip is cut at 1340 px |
| #529 | the short-interest failure is shown in grey |
| #530 | `menu.spec` race |
| #531 | race in the every-screen reload test |
| #532 | the market-cards chip quote is flaky |

Brief 33 triaged the issues open then into real bugs and the rest; check it against master before relying on it. There are 111 open issues in all.

## 7. Mistakes made, and the rule each left

| What happened | The rule now |
|---|---|
| Brief 39 listed problems with plan #522 and put the cross-listing question to the owner, and was announced as ready. | Solve it before sending: a brief gives solutions, researched. Brief 40 replaced it. |
| I referred the owner to "the two changes brief 39 requires", then explained them in test-harness terms. | Only decisions go to the owner, in plain words that stand alone. Internal detail goes to the CTO. |
| I offered to merge #521. | The CTO merges, on the owner's OK. Never offer. |
| Brief 44 asked #525 to change code that #521 had rewritten while #521 waited unmerged. The two PRs then conflicted in three files. | Check every request against the open PRs (§5, step 7). If two PRs would conflict, the one that is ready merges first. |
| Brief 24 read one page of the issue list and missed #383–#399. | List everything, paged. |
| During research, a request to sec.gov carried the owner's email address in its `User-Agent`. | The owner's email never leaves in any request, header or URL. |
| An agent filed a task suggestion that duplicated #528. | Search the open issues before suggesting or filing anything, and read an agent's output before it goes anywhere. |
| #523 went unreviewed for two days while #525 took every review. | At each start, list the open PRs, and give each one from the CTO a review status. |

## 8. Notes

- **Trailers.** Each commit ends with the co-author and session trailers your own harness supplies. No model name appears anywhere else in the repository.
- **Read-only checks of the CTO's branches:** `git fetch origin <branch>` and `git worktree add --detach`. Never `git checkout` in a worktree the CTO or the owner uses.
- **Cloud-only troubles, now behind you:** these came from the cloud container and should not recur on this machine:
  - a damaged pack in a container's `.git`;
  - a full disk during parallel builds (`CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0` helped);
  - a container reset that emptied the scratchpad;
  - Playwright's expected browser build differing from the preinstalled one.
