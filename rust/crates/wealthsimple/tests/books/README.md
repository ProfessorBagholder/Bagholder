# Books an earlier version wrote

Made-up books, written by the made-up Wealthsimple (`src/generated.rs`: `SHAPES` and the typed shapes now named `READ_BY_2_10_9`, `Size { accounts: 8, instruments: 16, trades: 60, months: 12, seed: 13 }`, its day 2026-10-25), each read by a released version's own code and kept gzipped. They hold no one's figures.

- `read-twice-by-2.10.8.db.gz`: read by v2.10.8's code at 18:00 and again at 19:00 (UTC, the broker's day).
- `read-by-2.10.8-then-2.10.9.db.gz`: the same 18:00 read by v2.10.8's code, read again at 19:00 by v2.10.9's.

Written 2026-10-26 from worktrees of the tags `v2.10.8` and `v2.10.9`, each with this version's `src/generated.rs` in place, by a test that opened the book with `Book::open_in(home, "test", now)`, pulled with `Wealthsimple::new(Generated::shaped(size, today, &shapes))` and kept it with `VACUUM INTO`. `tests/take_back.rs` reads both at 20:00 with this version.
