# Brief 25: the Stage 3 statements build as built (#503, `93a0dca3` and its follow-ups to `d6824aa7`): accepted, one fix

**Reviewed:** the build against each of brief 22's eleven required changes, on master `d6824aa7` (2.5.5). The build was merged with its plan on 2026-10-10 before this review; CTO had applied the eleven changes to the plan (`cb5ae304`) and built to them. Each item was read by one reader against the code and its tests (the named tests run where they build in minutes), then challenged by a second; the verdicts with their citations are in `25/as-built.json`.

## Verdict

Accepted. Eight items are done as written, two are done another way that is as good, one is partly done and needs a small fix. The design landed as reviewed: a period per value with its first and last moment, the newest read by index, the engine told the period's last moment, one account's check recomputed, a pull's rows and statements applied in one write, the oracle test over the writers, the instants the check hinges on read as integers, the day history left alone by a balances read, the tables rebuilt by migration 026 with `schema/v26.sql` committed.

| Item | Verdict | Where |
| --- | --- | --- |
| 1 `as_of` is the period's last moment | done | `statements.rs:613–616`, `engine_inputs.rs:135–137`; tests `a_cash_stated_again_unchanged_extends_its_period_and_is_stated_at_its_last_moment` (book) and `test_a_difference_the_broker_states_again_after_the_read_is_confirmed_through_a_folded_period` (server), both run and passing |
| 2 period store equals row store, property test | done differently | the oracle is an in-memory model of the rows in (instant, write) order, since the row code is gone; random sequences of (instant, cash), non-monotone and tied |
| 3 holds compared NULL-safe after the nulling | done differently | Rust `PartialEq` over loaded rows ordered by record, which is set equality; its own unit test |
| 4 one account's check; `brokers` probe times `broker_changed` | done | `engine.rs` `Change::Broker`; `capacity.rs` probe times the read and the apply |
| 5 a pull's rows and statements in one write | done | `Figures::pulled(now, stated_for)` under one lock, one version; the version test |
| 6 writers held by an oracle | done | `inputs.market.brokers == engine_inputs::brokers(&book)` after `pull`, `pull_activity`, `balances` |
| 7 units statements defined, indexed, measured | done | nullable moments with the cash-only CHECK; index `(account_id, kind, as_of_day)`; not folded, reason given |
| 8 instants as integers, text unread | done | `at_ns` on `broker_reads` and `activity_reads`; `MAX(at_ns)` through indexes |
| 9 no day history on a balances read | done | `stated_latest` for the balances path; `stated` (with days) only on a pull and at a start |
| 10 migration rebuilds the tables | done | 026 rebuilds through `_v26` tables and a rename, `defer_foreign_keys`, 0.64 s on the owner's copy |
| 11 boundary test names the whole readers' callers | partly | below |

## Required fix

**The boundary test scans only the text before a file's first `#[cfg(test)]`.** `tests_boundary.rs:212` `let code = text.split("#[cfg(test)]").next()`. Several production files carry a `#[cfg(test)]` on a field or an import near the top (`feeds.rs:52`, with its test module at :3305; `csv_import.rs:134`; `app.rs:179`; `notify.rs:455`; `update.rs:164`; `http/mod.rs:89`; `orders/mod.rs:13`), so most of their production code is never scanned. Reproduced: a `crate::figures::market(&book, &cache)?` inserted at `feeds.rs:3000` passes the test. A whole read added beside a sweep or an import is exactly the caller the test exists to refuse. Split at the test module (`#[cfg(test)]` followed by `mod`), or strip test items properly; add the planted call to the checker's own test (`the_whole_read_check_finds_a_call_and_not_a_definition_a_method_a_comment_or_a_test`).

## Worth closing in the same change

- **A units statement in the generated migration books.** `migration_26_keeps_what_every_account_s_statements_answer` writes cash, buying power and net value only; the units branch of 026 (:74–75) is held by the owner's copy and a hand run, not by `cargo test`. One `store_units` per generated book, asserting `stated_latest(a).units` after the migration.
- **A test that the balances path reads no day.** Both tests for item 9 hold that the engine's days equal the book's after a balances read, which a `broker_changed` that went back to reading them would also pass. Count the statements SQLite runs against `account_days` during `broker_changed` (the capacity counter already hooks every connection) and assert zero.
- The acceptance bullet at `docs/plans/stage-3-the-brokers-statements.md:122` still says "meet their budgets and the growth rule"; item 4's text has the gate in the right order. Wording.

## For the owner

Nothing to decide. The build is sound; the fix is to a test, not to what the app does. One process point: this build was merged before its as-built review, which is the step that found the gap above. Not a disaster here, but the order matters when it is the app that is wrong.
