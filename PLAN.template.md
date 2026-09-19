# Plan: <one line — what changes and why>

A done-contract for non-trivial work. Fill it before writing code, put the acceptance criteria in front of the maintainer, and land it in the PR so the review reads the contract next to the diff. Copy this file per task; do not edit the template in place. If the contract turns out to conflict with `SPEC.md` or the code, stop and say so — see *Right to refuse*.

## Scope

One paragraph: the change, and the figure, screen or `SPEC.md` § it touches. Link the issue or discussion if there is one.

## Approach

The existing model fields, page builders, store queries or helpers this builds on, named, so a second copy of something is never written. Whether `SPEC.md` must change, and why. What deliberately stays the same.

## Acceptance criteria

Observable and binary — each names how it is verified. If any fails, it is not done; there is no partial credit.

- [ ] `python3 -m unittest discover -s tests -t .` green, with a test added for the behaviour change.
- [ ] If the model changed: shared cases regenerated (`python3 tests/make_cases.py`) and reviewed as a diff, and the Rust and Go suites green on them (`cargo test --workspace`; `cd go && go generate ./... && go test ./...`).
- [ ] If the page changed: rendered on a scratch copy of the data per `SPEC.md` §7 — every displayed figure traced to its field, no table overflowing at 1200 / 1340 / 1440 / 1680, `node --check` on the script.
- [ ] If the page and server changed together: `PROTOCOL` bumped in the page and all three servers (a test keeps them equal).
- [ ] <the specific, observable outcome this change must produce, in this project's terms>

## Surfaces to check beyond the diff

The named places a reviewer must look that the diff alone does not show: a caller, a generated file (`go static.go`, the Rust `cases.rs`), the other language ports, a version bump, a release asset.

## Right to refuse

If an acceptance criterion cannot be met, or the ask conflicts with `SPEC.md` or with what the code already guarantees, stop and report before building. A wrong contract built cleanly is still wrong.

## Anti-stub self-check

Initial each: no definition nobody references; no field written and never read; no branch only the switch knows; no real-target run skipped — the suite was actually run, the page actually rendered on the scratch server, not asserted.

## Verification

The exact commands run and what they showed. Paste the numbers, not "passes".

## Handoff

Where this stopped (`file:line`), what is blocking, what not to redo, and the one command to resume.
