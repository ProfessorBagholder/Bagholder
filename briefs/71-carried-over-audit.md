# Brief 71: The code carried over from the Python app and never re-examined: four areas audited, the defects to fix, and what to delete

## Why this brief exists

About 16% of the Rust code, 20,327 of 125,384 lines, is still exactly as it was ported on 2026-09-16 (`git blame`, master `5af1bbc5`). The migration's stages rebuilt the engine, the book, the figures and the wire, each through a plan. The rest came across wholesale and was only ever reviewed where a change touched it. Every review, mine included, read the diff in front of it. That is how the Mac notifier, the CPU-only model and the per-company SEDAR+ walk each surfaced one at a time. `CLAUDE.md`'s first rule is that nothing carried over is assumed right.

Five read-only audits were run on 2026-10-20, one per area. Brief 70 took the filings pipeline. This brief takes the other four.

**How to read each finding:**
- **Verified** means I reproduced it against `origin/master` myself.
- **Reported** means an auditor showed it from the code with file and line, but I didn't reproduce it. Write its test first. If the test passes on master, drop the finding and say so in the PR.

Each area is its own PR, in the order given at the end. A finding that changes what a screen shows, or what the app reads by itself, goes to the owner as one line before it is built (`CLAUDE.md`, the gate).

## A. Text and HTML helpers (`model/src`, `sources/src/html`)

1. **`html::unescape` panics on non-ASCII text.** Verified. `sources/src/html/mod.rs:66-74` scans an entity name for up to 32 bytes, then slices `&t[at..stop]`, so a multi-byte character straddling the cut panics. It runs inside `sedar::parse_filings` while SEDAR+'s `state()` mutex is held (`sedar.rs:906-910`). The panic poisons that mutex, and every later `.lock().unwrap()` panics until restart. One filing title with Chinese or accented text after an `&` stops all SEDAR+ reads.
   - Fix: count `char`s, or stop at the first non-ASCII byte, since no entity name contains one.
   - Also: take SEDAR+'s state with `lock().unwrap_or_else(|e| e.into_inner())`, as the rest of the server does.
   - *Test:* generated text with a multi-byte character at every split position after `&` never panics.
2. **Attribute values and URLs are unescaped with the text-content rule, and one URL twice.** Reported.
   - `"…?id=5&copy=1"` becomes `"…?id=5©=1"`. WHATWG keeps an un-terminated reference literal inside an attribute when it is followed by `=` or an alphanumeric.
   - `sedar.rs:434` and `:917` unescape the same URL twice.
   - Controls and noncharacters are dropped, which is Python's behaviour, not WHATWG's.
   - Fix: add an `in_attribute` mode, unescape once, and follow the spec.
   - *Test:* a table of WHATWG's attribute and numeric-reference cases.
3. **`html_tables` loses cells.** Reported.
   - Omitted end tags (`<td>a<td>b<tr>…`, which HTML allows) return no cells.
   - A nested table overwrites the outer one (`tables.rs:148-185`).
   - These feed the payer distribution readers, which produce money facts.
   - Fix: close the open cell or row when a new one starts and at `</table>`, and keep a stack of tables.
   - *Test:* generated tables with every combination of omitted end tags and nesting depths.
4. **`parse_float` copies Python's `float()`.** Reported. It accepts `nan`, `inf`, `1_000` and non-ASCII digits (`textrules.rs:35-73`), and its live callers are quote fields and fund weights, so a NaN reaches every sum it touches.
   - Fix: money through `Dec::parse`; a statistic through `str::parse::<f64>()` that also checks the value is finite.
   - Delete `float_value`, which has no callers.
5. **Delete what Rust's standard library already does:**
   - `unichars.rs`, 479 lines of Python `str` tables frozen at Unicode 15.0 → `char::is_alphabetic`, `is_numeric`, `is_whitespace`;
   - the second, lenient CSV reader in `textrules.rs:114-245`, which is quadratic per field → the importer's strict RFC 4180 reader;
   - `splitlines` → `str::lines`;
   - the hand-rolled `uuid4` → `uuid` (already a dependency);
   - `login.rs:968`'s Python `isdigit` → `is_ascii_digit`.
   - Put `cases.rs` behind `#[cfg(test)]` or a feature.
   - Keep `entities.rs`, which matches WHATWG's table, and add a test checking it against `entities.json`: the count, the sort order, and a sample of entries.

## B. Market data (`market/src`)

6. **Charts convert USD to CAD only, and take a listing with no currency as CAD.** Verified at `history.rs:174-213`, where `!(quote == "USD" && ccy == "CAD")` returns the bars unconverted, and at `:181,195`, where empty becomes "CAD". `feeds.rs:1456` reads USD rates alone. This breaks `docs/decisions.md`: rates for every currency the Bank of Canada publishes, and no currency taken to be CAD.
   - Fix: one conversion over every published currency, through its cross rate against CAD. A listing with no currency is a named failure.
   - *Test:* a property test over every pair of the Bank's currencies, plus a currency nobody holds.
7. **The whole daily history is fetched again every 20 hours.** Verified: `history.rs:529-547` takes `fetch_from = last_start`, the earliest day stored. That breaks §13, where closed days' bars are never asked for again.
   - Fix: top up from the newest stored bar, with a short overlap, as `ensure_intraday` already does. Read everything again only when a split or consolidation is known.
   - *Test:* the request count. The second top-up asks only from the newest stored day.
8. **Unreadable replies are treated as "no data".** Reported (`parse.rs`, `quotes.rs:101`, `history.rs:368,1067`). A changed reply shape quietly moves on to the next source, or records a miss, and nothing is said.
   - Fix: strict typed readers that return a named shape failure (`docs/decisions.md`: a mismatch is a visible error naming the source and field).
   - *Test:* a recorded wrong-shaped reply for each live reader gives a named failure.
9. **Today's chart bar stays frozen for up to 20 hours.** Reported (`history.rs:26,511-525`). Fix: the last bar is a forming bar, updated from the stored live quote and read again once it settles. This changes what the chart shows, so it is **one line to the owner first**.
10. **Opening a 1H or 4H chart can wait on a network read.** Reported (`feeds.rs:1822` → `history.rs:1340`). Fix: answer from the store, fetch in the background and signal, as the daily path does.
11. **Delete the old code nothing calls.** Verified:
    - `quotes::refresh_quotes`, `peek_quote` and `coinbase_prev_close`, none of which has a caller outside `quotes.rs`;
    - `refresh.rs`, which nothing outside its tests calls;
    - the lenient parsers.

    They hold rules the owner has overruled (FRED/Stooq index levels, a USD-only Bank read, New York hours), kept alive by their tests.
    - *Test:* every public function of `market` has a caller outside the tests.
12. **Smaller, reported:**
    - the archive backfills hourly crypto that Coinbase never deletes;
    - news failures from Yahoo, Seeking Alpha and Google are only logged;
    - the session open is hard-coded at 9:30;
    - Yahoo's rest ignores `Retry-After`;
    - a Coinbase market, once found, is never checked again.

    Fix each as reported, each with its own test.

## C. Wealthsimple sign-in (`server/src/login.rs`, `wealthsimple`, `ws`)

13. **Two processes can rotate the same refresh token.** Verified:
    - the refresh lock is a `static Mutex`, so it holds within one process only (`wealthsimple/src/session.rs:72`);
    - `bagholder pull-broker <home>` builds a refreshing client and never takes `hold_home` (`pull_broker.rs`).

    Run beside the server, both can post the same refresh token. Wealthsimple then revokes the sign-in (RFC 9700 §4.14.2, refresh-token reuse), and bracket watching stops with it.
    - Fix: take `hold_home` in `pull-broker`, or make it read-only like the outside-debts capture, and hold an OS file lock on the session file around read, post and write.
    - *Test:* two processes on one folder against a stand-in token endpoint that accepts each token once: one post per token, and no lapse.
14. **The capture takes a token from any site's cookie.** Reported (`login.rs:495-536`): `getAllCookies`, then the first cookie holding an `access_token`. Fix: read the named cookie for Wealthsimple's own host only, and check its domain.
15. **Clear all and Disconnect leave sign-in data behind.** Reported. Only `session.json` goes. `<home>/chrome` (Wealthsimple's cookies and the device id), `client_id` and `user_agent` stay, against the decision that after Clear all nothing is left, the Wealthsimple login included.
16. **The session file isn't crash-safe.** Reported. It is written without fsync and briefly readable by others. Fix: create it with 0600, fsync it, rename, then fsync the folder.
17. **Smaller, reported:**
    - in the container, Chromium runs as root with `--no-sandbox`;
    - DevTools is on a TCP port instead of `--remote-debugging-pipe`;
    - a 403 triggers a refresh, though RFC 6750 says only a 401 should;
    - there are two refresh paths and two Wealthsimple clients;
    - a hung browser helper blocks every caller behind it, because there is no deadline.

## D. The old store and the carry (`store`)

18. **Every start rewrites the old file's broker rows.** Verified. `carry::from_old_store` (`carry.rs:291-292`) opens `bagholder.db` writable and runs `relabel::ensure`, whose `UPDATE activities …` statements flip signs and rewrite quantities and prices. That happens after the book was imported from it, and before its "as it was" snapshot is taken. It is the old app's own documented mistake ("broker rows rewritten at every start", `docs/old-app-mistakes.md`), still running.
    - Fix: open the old file read-only, and drop `relabel::ensure` from the carry.
    - *Test:* a digest of the old file's `activities` rows is unchanged by a start.
19. **The one-time carry runs at every start, forever.** Reported. Fix: once the book records every carry done, move `bagholder.db` into `snapshots/` and never open it again.
    - *Test:* a second start finds no `bagholder.db` in the data folder and opens none.
20. **Delete `store/src/csvimport.rs`.** Verified: `parse_csv` and `import_text` have no callers outside the store's tests. It carries every import risk:
    - f64 money, with an unreadable cell read as 0;
    - day and month order guessed;
    - footer lines dropped by a regex.

    The live importer (`broker/src/csv.rs`) was rebuilt strictly. Move its three key constants into `carry.rs`.
21. **The scan for old patterns skips the live server.** Reported (`engine/tests/no_guesses.rs:24-31`, which skips `server`, `store` and `market` as "go at the switch"). The switch has happened. Add `server` to the scan, with an allow-list naming the carry's files.

## Order

1. A1 (the crash) and C13 (the sign-in race) first, as one small PR each. They can stop SEDAR+ reads, and sign the owner out.
2. Then B6 (currency), D18 (the old file rewritten), C14 and C15, and A3 and A4.
3. Then the deletions (A5, B11, D20) and the rest.

Brief 70's change 1 can go alongside the first step, since they touch different files.
