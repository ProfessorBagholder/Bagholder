# Plan: times follow the standard convention on every machine

## Scope

How the Rust build and the page turn instants into days and times. What CI's new macOS job found and the audit that followed:

- The server read zone rules only from the operating system. Windows has none, so fill times were blank there and "today" was UTC's; a slim container has none either.
- Tests depended on the machine's rules: GitHub's macOS runner predates Alberta's permanent UTC−6 (1 Nov 2026) and read a Dec 2026 fill as 08:00, not 09:00.
- Two separate readers (`model::clock`, `market::clockzone`) each kept their own cache.
- A TMX bar time sent without an offset was read in the server machine's zone, not the exchange's.
- The page's execution table showed the server's Alberta time, while SPEC ("Intraday bars and execution times are shown in the viewer's local time") and the chart beside it use the viewer's.
- Add trade's default date and the listing window used the UTC date, so after 6 pm in Alberta they gave tomorrow.

## Approach: the convention, as Python's zoneinfo, Go's time and jiff document it

1. Instants are stored and sent in UTC; they become a local day or time only for display (in the viewer's zone, in the browser) or where a business rule names a zone (Wealthsimple's Alberta day, the exchange's session).
2. Zone rules come from the operating system's IANA database, which the OS keeps current.
3. A built-in copy is used only where the system has none. Python's docs recommend exactly this for cross-platform programs (declare `tzdata`); Go has `time/tzdata`. jiff reads the system's first and falls back to its own (`TimeZoneDatabase::from_env`, read at `jiff-0.2.37/src/tz/db/mod.rs:293`).
4. Tests pin their rules, so no machine changes an answer: the Rust tests read jiff's built-in copy (feature `pinned-tzdb`, enabled only from dev-dependencies), the page tests and browser tests run in America/Toronto.

jiff (BurntSushi, modelled on JavaScript's Temporal; 198M downloads, 1,242 dependents on 2026-09-23) replaces tz-rs (20M). Its built-in copy is IANA 2026c.

## Acceptance criteria

- [x] `cargo test --workspace` green sandboxed, 0 build warnings; each crate green on its own (`cargo test -p <crate>`).
- [x] With the machine's database replaced by Tokyo's rules under the Alberta, Toronto and New York names (`TZDIR`), the model's zones, wire and cases tests still pass: the tests read only the pinned rules.
- [x] `zones.rs`: a 2026-12-01T15:00Z fill reads 09:00 and a 2026-01-01 one 08:00 in Alberta; a wall-clock time takes its own day's offset; an unknown zone is none.
- [x] `bars.rs`: a TMX time without an offset is Toronto's wall clock (13:30 UTC in September, 14:30 in January).
- [x] `grep -rn "from_posix_tz\|tz::TimeZone" rust/crates` finds nothing; `clockzone.rs` is gone; every zone is read in `model::clock`.
- [x] Page: `fmt.test.ts` (viewer's day and time, bare dates, today); the e2e test "an execution's day and time are the viewer's, not the server's" passes, and fails against the old line (shows 13:30 where Toronto reads 15:30).
- [x] `npm run check` 0 errors; `npm test` 42; Playwright 198/198.
- [ ] CI green on Linux, macOS (`--include-ignored`) and Windows (the model).

## Surfaces beyond the diff

`rust/Dockerfile` installs `tzdata`, so the container follows the OS like every other install. The outgoing Python and Go builds are unchanged (Python already declares `tzdata`).

## Verification

As in the criteria, run 2026-09-23 in the scratch worktree. A race found on the way: `filterfields.spec.ts` "a custom date range…" read the preset's request as the date's under load; it now awaits each request it triggers.
