# Brief 51: Tests touched run locally, one full CI run per head to be merged, and the trial merge `a8e3a84f` resolves the three files right

## Verdict

- **Order:** finish brief 49's merge before starting brief 50. #523 merges after #525, so work on brief 50 can't land any sooner by starting now. It would only compete for this machine's two cores.
- **How checks are run** (the owner's rule, 2026-10-18; corrected the same day, see below):
  1. **While a change is built or checked** (each fix, each planted reverse): run only the tests that change touches, locally, on the build that is already compiled. One build at a time on this machine, counting the CTO's and the reviewer's together.
  2. **Exactly one full CI run for each head that will be merged:** the one that starts by itself when the commit is pushed to the PR. That run is the evidence for the merge. Batch fixes into one push, so small pushes don't each start a full run. A failed job that is a known flaky test is rerun alone (`gh run rerun <id> --failed`), never by starting a new full run.
  3. **Never dispatch a full CI run, or push a throwaway branch, to prove one test.** A browser check runs locally: Playwright's Chromium is installed on this machine (1.63.0, in `~/.cache/ms-playwright`).
- **The planted reverses that a brief gives the reviewer to check are the reviewer's to run,** under rule 1. The CTO doesn't run them as well, and no cloud session is started for them.
- **Nothing runs on the owner's Mac.** The live app is there.
- **Of the three questions put to the owner, only one was the owner's:** the OK to merge #521. Which work comes first, and where the tests run, are settled here (`docs/decisions.md`, 2026-10-04 and 2026-10-09: never put a solvable question to the owner).
- **The trial merge `a8e3a84f`** (#521's head merged into #525, on the scratch branch `check-merge-521-into-525`) resolves the three files as brief 49 asks.

## Correction (2026-10-18)

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
