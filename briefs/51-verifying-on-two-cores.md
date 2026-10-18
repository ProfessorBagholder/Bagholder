# Brief 51: Full suites run on CI, planted checks are the reviewer's, and the trial merge `a8e3a84f` resolves the three files right

## Verdict

- **Order:** finish brief 49's merge before starting brief 50. #523 merges after #525, so work on brief 50 can't land any sooner by starting now. It would only compete for this machine's two cores.
- **The full suites run on GitHub's CI, never on this machine.** This machine runs only the tests a change names, one build at a time, counting the CTO's builds and the reviewer's together.
- **The planted reverses that a brief gives the reviewer to check are the reviewer's to run.** The CTO doesn't run them as well, and no cloud session is started for them. Four reverses on a build that is already warm take a few minutes each. Four cloud sessions would each build from cold, at the owner's cost.
- **Nothing runs on the owner's Mac.** The live app is there.
- **Of the three questions put to the owner, only one was the owner's:** the OK to merge #521. Which work comes first, and where the tests run, are settled here (`docs/decisions.md`, 2026-10-04 and 2026-10-09: never put a solvable question to the owner).
- **The trial merge `a8e3a84f`** (#521's head merged into #525, on the scratch branch `check-merge-521-into-525`) resolves the three files as brief 49 asks.

## Why: how this is done elsewhere

A pull request's full suite running on CI machines, with the developer's machine running only the tests a change touches, is the ordinary arrangement. A two-core machine shared by two sessions and the owner's tools is the case it exists for. The repository already has what it needs:
- on #525's branch, `tests.yml` takes `workflow_dispatch`, so a scratch branch can be run without opening a PR;
- the eight jobs cover what brief 49 asks to "re-run in full": `cargo test --workspace --locked` on Linux and macOS, clippy, the page's check and unit tests, the full e2e, the container, capacity and audit.

**The evidence a brief asks for is the CI run on the commit being checked, quoted by run id.** It is not a local run.

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

1. **The final merge.** Once #521 is in master, merge master into #525 with a merge commit. The three files must come out exactly as at `a8e3a84f`: `git diff a8e3a84f <final> -- rust/crates/net/src/client.rs rust/crates/net/src/net.rs web/e2e/serve.mjs` is empty, or the PR says why it is not. Push it to #525, and send the commit with its CI run id once all eight jobs have finished. No local suite is run.
2. **The planted reverses** for brief 49 (#521's two and #525's two offline tests) are run by the reviewer on the final commit. Any already running on this machine may finish. Their results go into the next brief as one table.
3. **Brief 50 starts after #525 merges.** Its own full suites follow change 1's rule: CI on the pushed head, quoted by run id, plus the targeted tests the brief names, run here.
4. **The standing rule:** add it to `CLAUDE.md` *Verifying a change*, in the next PR that touches that file, in these words: *"The full suites are evidence only from CI on the commit being checked, quoted by run id; on the shared machine run only the tests a change names, one build at a time."* Held by: review only.
