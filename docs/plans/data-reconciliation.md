# Plan: refresh Python data reconciliation from JUBUSINESS

## Scope
Update PR #232 against current upstream, limited by the contributor's explicit request to the Python reference and reusable regression cases. Carry the missing number, sync and import fixes without aesthetics or customer evidence. Rust, Go and native phone implementation changes are outside this contribution.

## Approach
Compare upstream f8965f6 with JUBUSINESS 67c22f9. Preserve upstream's unknown-basis crypto-deposit separation, quote-source guards, pooled store, cached book and test isolation. Port verified order legs, explicit inventory details, persistent statement evidence and validated broker reports into python/. Keep broker-adjusted returns distinct from journal FIFO. Update SPEC.md for the Python semantics and document cross-port expectations.

## Acceptance criteria
- [x] The complete isolated Python suite passes using python3 -m unittest discover -s python/tests -t python.
- [x] Synthetic tests demonstrate quantity, cost and fee conservation through transfers, ticker suffixes, renames, DLR conversions, splits and option exercise/assignment.
- [x] Synthetic tests demonstrate repeat sync/import and clear/rebuild equivalence, including persistent statement corrections, without reading customer data.
- [x] Partial, inconsistent and unavailable broker responses never invent basis or replace a complete report with a partial total.
- [x] Existing upstream unknown-basis crypto and quote-source safeguards remain covered.
- [x] The final diff contains Python data logic, reusable regression cases and documentation only; no branding, layout, deployment configuration, credentials or personal records.
- [x] The uploaded tree matches the tested tree and the PR describes the remaining cross-port scope precisely.

## Surfaces to check beyond the diff
Python model/book caches, quote-only live response, store rollback behavior and preservation rules, sync/detail retry state, CSV account mapping, API trade details, Python packaging, and shared fixture expectations used by other ports.

## Right to refuse
Missing evidence remains unknown. Do not manufacture corrections to force FIFO to match a broker-adjusted total. The owner's explicit Python-only scope takes precedence over the template's all-port requirement; native parity is reported, never claimed.

## Anti-stub self-check
Reviewed: new helpers are called by sync/store/model paths and covered by synthetic tests. Report and evidence metadata are exposed through model/book responses. Source and mutation retry paths are covered. No live-account mutation was used.

## Verification
Pending.

## Handoff
Python reference contribution complete in an isolated worktree. Review docs/account-export-reconciliation.md for the comparison and API semantics. Shared expectations are ready for the maintainer to reuse when porting the behavior to other implementations. JUBUSINESS remains unchanged.
