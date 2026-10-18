# Brief 51: Tests touched run locally, one full CI run per head to be merged, and the trial merge `a8e3a84f` resolves the three files right

## Verdict

- **Order:** finish brief 49's merge before starting brief 50. #523 merges after #525, so work on brief 50 can't land any sooner by starting now. It would only compete for this machine's two cores.
- **How checks are run** (the owner's rule, 2026-10-18; corrected the same day, see below):
  1. **While a change is built or checked** (each fix, each planted reverse): run only the tests that change touches, locally, on the build that is already compiled. One build at a time on this machine, counting the CTO's and the reviewer's together.
  2. **Exactly one full CI run for each head that will be merged:** the one that starts by itself when the commit is pushed to the PR. That run is the evidence for the merge. Batch fixes into one push, so small pushes don't each start a full run. **A CI job is never rerun to retry a test.** GitHub reruns whole jobs, so `gh run rerun --failed` on the browser job runs all 338 tests to retry two. A test that fails on CI is rerun alone here, fixed if it fails here, and the fix's push gives the head its one full run.
  3. **Never dispatch a full CI run, or push a throwaway branch, to prove one test.** A browser check runs locally: Playwright's Chromium is installed on this machine (1.63.0, in `~/.cache/ms-playwright`).
- **The planted reverses that a brief gives the reviewer to check are the reviewer's to run,** under rule 1. The CTO doesn't run them as well, and no cloud session is started for them.
- **Nothing runs on the owner's Mac.** The live app is there.
- **Of the three questions put to the owner, only one was the owner's:** the OK to merge #521. Which work comes first, and where the tests run, are settled here (`docs/decisions.md`, 2026-10-04 and 2026-10-09: never put a solvable question to the owner).
- **The trial merge `a8e3a84f`** (#521's head merged into #525, on the scratch branch `check-merge-521-into-525`) resolves the three files as brief 49 asks.

## How rules 1–3 are kept

**Which tests run.** The list is written down before anything runs:
- each required change in a brief names the test that holds it, and each planted reverse names the test that must fail. Those named tests are what runs, and nothing else;
- a fix the CTO makes also runs the tests in the files it touches.

The commands are `cargo test --locked -p <crate> --test <file> <name>` (or `--bin bagholder <name>` for the server's own tests) and `npx playwright test e2e/<file>.spec.ts:<line>`. A whole crate, the workspace, or the whole browser suite never runs locally.

**Builds stay warm.**
- Each session keeps one worktree per PR it is working on, and keeps it until that PR merges.
- All of a session's worktrees share one `CARGO_TARGET_DIR`, so moving to another PR rebuilds only the crates that differ.
- Incremental compilation stays on. `CARGO_INCREMENTAL=0` was a fix for the cloud container's disk, and here it makes every rebuild slower.
- The page is built once per head, for its browser tests.

**One build at a time, enforced by a lock.** Every cargo, `npm run build` or Playwright run on this machine, by either session, goes through one lock:

    flock /tmp/bagholder-build.lock nice -n 10 cargo test --locked ...

A second build waits for the first, never runs beside it.

**Where each task runs:**

| Task | Where | Why |
|---|---|---|
| a named Rust test, a planted reverse | here, on the warm build | seconds to a few minutes; CI would take ten or more |
| a named browser test | here (Chromium 1.63.0 installed) | about a minute, against a 10-minute browser job |
| the whole suite, the macOS and Windows jobs, the container, capacity, audit | CI, the one run the push to the PR starts | eight machines in parallel; macOS and Windows aren't available here; a container build would hold this machine's two cores |
| anything | never a cloud session, never the owner's Mac | a cloud session builds from cold; the Mac runs the live app |

## Correction (2026-10-18)

A second correction, the same day: this brief first allowed rerunning a failed CI job for a known flaky test. #525's browser job was rerun that way, and all 338 tests ran to retry two. Rule 2 now forbids it.


This brief first said that full suites run on CI, without limiting how many runs. It allowed `workflow_dispatch` on a scratch branch. The CTO's trial merge and my own browser plant (`review-plant-ws-elsewhere`) each started a full run of all eight jobs to prove one thing. Rules 1–3 above replace that. The scratch branches are deleted, and my plant's run is cancelled. Its browser job had already shown the planted failure.

## The trial merge, checked

| Rule (brief 49) | At `a8e3a84f` |
|---|---|
| every request leaves through `client::leave` | `Net::send`'s offline branch calls `client::leave`, and `send_into` calls `leaving` |
| the offline refusal is the typed one, never words | `leaving` returns `Error::Offline`, and `Net::send` maps it to `NetError::Offline`. Nothing in `net` still writes `Error::Transport(OFFLINE.into())` |
| `named_call` beside the `outbound` line | `client.rs:612`, documented as such |
| `serve.mjs` keeps #521's record of the calls answered | the `.answered` log is written, appended and removed (lines 49, 50 and 82) |
| `serve.mjs` keeps #525's changes | the e2e home is named for its port, and `--bars` stays dropped, as #525 dropped it on purpose |

A note, not a change: `return Err(NetError::Offline)` after `client::leave(…)?` is never reached. `leave` always refuses when `refused_offline` is true, and both read the same switch, which is set once. Leave it as a guard, or remove it, in the final merge. Either is fine.

## Required changes

1. **The final merge.** Done as `e98e4176`. Its whole tree equals `a8e3a84f`'s. Its CI run, 39158606601, is the evidence for the merge.
2. **The planted reverses** are the reviewer's, run under rule 1.
3. **Brief 50 starts after #525 merges.** Its fixes are built and checked under rule 1 and pushed together in one push. The full run that push starts is its merge evidence.
4. **The standing rule:** add rules 1–3 to `CLAUDE.md` *Verifying a change*, in the next PR that touches that file, in the words above. Held by: review only.
