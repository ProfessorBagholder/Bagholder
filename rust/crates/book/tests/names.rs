//! An instrument's names kept as rows (`docs/plans/stage-3-a-change-on-a-deep-history.md`,
//! A): after any sequence of writes, through the book or around it, what the book
//! reads equals the sightings folded.

mod common;

use bagholder_book::clear::Clearing;
use bagholder_core::InstrumentId;
use common::*;
use serde_json::json;

/// A small generator, so a failing sequence is the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len() as u64) as usize]
    }
}

const ISINS: [&str; 3] = ["CA0000000001", "CA0000000002", "CA0000000003"];

fn instrument_of(f: &Fixture, record: bagholder_core::RecordId) -> InstrumentId {
    f.book.transactions_of(record).unwrap()[0].instrument.unwrap()
}

fn instruments(f: &Fixture) -> Vec<InstrumentId> {
    f.book.instruments().unwrap().iter().map(|i| i.id).collect()
}

/// Every instrument's names as kept, once nothing is due, equal to the fold, and
/// what is read equal to the fold whatever is due.
fn held(f: &Fixture, step: &str) {
    for id in instruments(f) {
        let folded = f.book.names_as_folded(id).unwrap();
        assert_eq!(f.book.names(id).unwrap(), folded, "{step}: the names read of {id}");
        assert_eq!(f.book.current_symbol(id).unwrap(), folded.last().map(|n| n.symbol.clone()), "{step}: the symbol of {id}");
    }
    // a write through the book settles what was due, and the kept runs are the fold
    f.book.atomically(|| Ok(())).unwrap();
    let due: i64 = f.book.conn_for_tests().query_row("SELECT COUNT(*) FROM names_due", [], |r| r.get(0)).unwrap();
    assert_eq!(due, 0, "{step}: nothing due after a write through the book");
    for id in instruments(f) {
        let head = f.book.canonical(id).unwrap();
        assert_eq!(f.book.names_as_kept(head).unwrap(), f.book.names_as_folded(head).unwrap(), "{step}: the names kept of {head}");
    }
}

#[test]
fn the_names_kept_equal_the_sightings_folded_after_any_writes() {
    let mut steps_run = 0;
    for seed in 1..=40u64 {
        let f = Fixture::new();
        f.account(&["a1"]);
        f.account(&["a2"]);
        let m = Spelled::v(1);
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut keys: Vec<String> = Vec::new();
        for n in 0..60 {
            let action = rng.below(12);
            let step = format!("seed {seed}, step {n}, action {action}");
            match action {
                // a record stored, or one stored before revised: a sighting under a
                // symbol and venue that change back and forth, with words or without
                0..=4 => {
                    let key = if action == 4 && !keys.is_empty() { rng.pick(&keys).clone() } else { format!("r{n}") };
                    let day = format!("2026-0{}-{:02}", 1 + rng.below(3), 1 + rng.below(28));
                    let mut inst = json!({"refs": [["isin", rng.pick(&ISINS)]], "kind": "security", "currency": "CAD", "symbol": rng.pick(&["AAA", "BBB", "CCC"]), "seen": day});
                    if rng.below(2) == 0 {
                        inst["mic"] = json!(rng.pick(&["XTSE", "XTSX"]));
                    }
                    if rng.below(2) == 0 {
                        inst["name"] = json!(rng.pick(&["Alpha Corp", "Alpha Renamed"]));
                    }
                    if rng.below(2) == 0 {
                        inst["venue"] = json!(rng.pick(&["TSX", "TSX-V"]));
                    }
                    let account = rng.pick(&["a1", "a2"]).to_string();
                    f.store(&m, &key, &legs(vec![buy(&account, inst, "1", "-1", &format!("{day}T15:00:00Z"))]));
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
                // a record the source reports removed
                5 if !keys.is_empty() => {
                    let key = rng.pick(&keys).clone();
                    if let Some(r) = f.book.record_by_key(Some(f.connection), &bagholder_core::SourceName::named("test-broker"), &key).unwrap() {
                        let _ = f.book.mark_removed(r, t0());
                    }
                }
                // a sighting renamed, moved to another day, rewritten whole or
                // deleted, around the book (as `figures.rs` renames one)
                6 => {
                    let c = f.book.conn_for_tests();
                    let sym = rng.pick(&["AAA", "BBB", "ZZZ"]).to_string();
                    c.execute("UPDATE instrument_sightings SET symbol = ?1 WHERE rowid = (SELECT rowid FROM instrument_sightings ORDER BY random() LIMIT 1)", [sym]).unwrap();
                }
                7 => {
                    let c = f.book.conn_for_tests();
                    let day = format!("2026-0{}-{:02}", 1 + rng.below(3), 1 + rng.below(28));
                    c.execute("UPDATE instrument_sightings SET day = ?1, name = NULL WHERE rowid = (SELECT rowid FROM instrument_sightings ORDER BY random() LIMIT 1)", [day]).unwrap();
                }
                8 => {
                    let c = f.book.conn_for_tests();
                    c.execute(
                        "INSERT OR REPLACE INTO instrument_sightings (record_id, leg, instrument_id, symbol, venue_mic, venue_name, name, day)
                         SELECT record_id, leg, instrument_id, 'RPL', venue_mic, 'Replaced', name, day FROM instrument_sightings ORDER BY random() LIMIT 1",
                        [],
                    )
                    .unwrap();
                }
                9 => {
                    let c = f.book.conn_for_tests();
                    c.execute("DELETE FROM instrument_sightings WHERE rowid = (SELECT rowid FROM instrument_sightings ORDER BY random() LIMIT 1)", []).unwrap();
                }
                // a succession joined, or parted again
                10 => {
                    let ids = instruments(&f);
                    let c = f.book.conn_for_tests();
                    let joined: i64 = c.query_row("SELECT COUNT(*) FROM instrument_joins", [], |r| r.get(0)).unwrap();
                    if joined > 0 && rng.below(2) == 0 {
                        c.execute("DELETE FROM instrument_joins", []).unwrap();
                    } else if ids.len() >= 2 && joined == 0 {
                        c.execute("INSERT INTO instrument_joins (instrument_id, into_id) VALUES (?1, ?2)", [ids[0].to_string(), ids[1].to_string()]).unwrap();
                    }
                }
                // the broker's records cleared, or everything
                11 if n > 30 && rng.below(3) == 0 => {
                    let what = if rng.below(2) == 0 { Clearing { broker: true, entries: false, journal: false, market: false, orders: false, following: false } } else { Clearing::all() };
                    f.book.clear(&what).unwrap();
                    held(&f, &step);
                    steps_run += 1;
                    // the accounts and the connection go with the records: the sequence ends here
                    break;
                }
                _ => continue,
            }
            held(&f, &step);
            steps_run += 1;
        }
    }
    assert!(steps_run > 1000, "the sequences ran {steps_run} steps");
}

/// One instrument whose rows cannot be read fails alone: other instruments'
/// names settle, writes and opening the book go on, and its due rows wait.
#[test]
fn an_instrument_whose_names_cannot_be_kept_fails_alone() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let a = f.store(&m, "r1", &legs(vec![buy("a1", share(ISINS[0], "AAA"), "1", "-1", "2026-01-02T15:00:00Z")]));
    let b = f.store(&m, "r2", &legs(vec![buy("a1", share(ISINS[1], "BBB"), "1", "-1", "2026-01-02T15:00:00Z")]));
    let (ia, ib) = (instrument_of(&f, a.record), instrument_of(&f, b.record));
    // a fault planted where that instrument's runs are written
    let c = f.book.conn_for_tests();
    c.execute_batch(&format!("CREATE TRIGGER planted BEFORE INSERT ON instrument_names WHEN NEW.instrument_id = '{ia}' BEGIN SELECT RAISE(ABORT, 'planted'); END;")).unwrap();
    c.execute("UPDATE instrument_sightings SET symbol = 'AAB' WHERE record_id = ?1", [a.record.to_string()]).unwrap();
    c.execute("UPDATE instrument_sightings SET symbol = 'BBC' WHERE record_id = ?1", [b.record.to_string()]).unwrap();
    // the write goes through; the other instrument settled, this one still due
    f.book.atomically(|| Ok(())).unwrap();
    assert_eq!(f.book.current_symbol(ib).unwrap().as_deref(), Some("BBC"));
    let due: Vec<String> = c.prepare("SELECT DISTINCT instrument_id FROM names_due").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(due, vec![ia.to_string()]);
    // and the book opens
    drop(f.book);
    let (book, _) = bagholder_book::Book::open_in(f.dir.path(), "test", t0()).unwrap();
    assert_eq!(book.current_symbol(ib).unwrap().as_deref(), Some("BBC"));
}
