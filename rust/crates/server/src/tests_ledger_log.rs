//! The record's change log covers every table the engine's ledger is read from
//! (`docs/plans/stage-3-the-record-by-change.md`, A; brief 26, 11): the tables are
//! found by SQLite's authorizer as the whole read prepares its statements, on a
//! book holding a row of every kind the read reaches, never listed by hand; each
//! must carry the log's three triggers. A migration that rebuilds a table and
//! drops its triggers fails here.

use std::collections::BTreeSet;

/// A freshly migrated book holding a row of every kind the ledger's read reaches:
/// the broker's rows, a trade with a group and a journal entry with a tag, an
/// adjustment with a leg, and two instruments no live record names (one the person
/// named, one a source described), so the names' fallbacks are read too.
fn seeded(home: &std::path::Path) {
    crate::tests_common::pulled_book(home);
    let (book, _) = bagholder_book::Book::open_in(home, "t", "2025-11-20T00:00:00Z".parse().unwrap()).unwrap();
    let c = book.conn_for_tests();
    let (record, leg, instrument): (String, String, String) =
        c.query_row("SELECT record_id, leg, instrument_id FROM transactions WHERE instrument_id IS NOT NULL ORDER BY record_id LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    c.execute_batch(&format!(
        "INSERT INTO trades (id, anchor_record, anchor_leg, anchor_instrument, created_at) VALUES ('00000000-0000-7000-8000-0000000000e1', '{record}', '{leg}', '{instrument}', 'x');
         INSERT INTO trade_groups (id, locked, created_at) VALUES ('00000000-0000-7000-8000-0000000000f1', 0, 'x');
         INSERT INTO trade_group_members (group_id, position, trade_id) VALUES ('00000000-0000-7000-8000-0000000000f1', 0, '00000000-0000-7000-8000-0000000000e1');
         INSERT INTO journal (trade_id, thesis, updated_at) VALUES ('00000000-0000-7000-8000-0000000000e1', 'why', 'x');
         INSERT INTO journal_tags (journal_id, position, tag) VALUES (last_insert_rowid(), 0, 'tag');
         INSERT INTO adjustments (record_id, leg, applies_record, applies_leg) VALUES ('{record}', 'adj', '{record}', '{leg}');
         INSERT INTO adjustment_legs (record_id, leg, position, from_instrument) VALUES ('{record}', 'adj', 0, '{instrument}');
         INSERT INTO instruments (id, kind, currency, created_at) VALUES ('00000000-0000-7000-8000-0000000000c1', 'security', 'CAD', 'x'), ('00000000-0000-7000-8000-0000000000c2', 'security', 'CAD', 'x');
         INSERT INTO listings_named (instrument_id, symbol, named_at) VALUES ('00000000-0000-7000-8000-0000000000c1', 'NMD', 'x');
         INSERT INTO instruments_described (instrument_id, source, symbol, day) VALUES ('00000000-0000-7000-8000-0000000000c2', 'tmx', 'DSC', '2025-11-19');"
    ))
    .unwrap();
}

#[test]
fn every_table_the_ledger_is_read_from_logs_its_changes() {
    bagholder_sqlite::work::start_counting();
    let home = tempfile::tempdir().unwrap();
    seeded(home.path());
    // a book opened inside the count: a fresh connection prepares every statement
    let ((), read) = bagholder_sqlite::work::tables_read(|| {
        let (book, _) = bagholder_book::Book::open_in(home.path(), "t", "2025-11-20T00:00:00Z".parse().unwrap()).unwrap();
        let ledger = crate::engine_inputs::ledger(&book).unwrap();
        assert!(!ledger.groups.is_empty() && !ledger.journal.is_empty(), "the seeded group and journal are read");
        assert!(!book.adjustments().unwrap().is_empty(), "the seeded adjustment is read");
    });
    let (book, _) = bagholder_book::Book::open_in(home.path(), "t", "2025-11-20T00:00:00Z".parse().unwrap()).unwrap();
    let triggers: BTreeSet<(String, String)> = {
        let mut st = book.conn_for_tests().prepare("SELECT tbl_name, name FROM sqlite_master WHERE type = 'trigger'").unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
    };
    // `names_due` is the queue an instrument's kept names are settled from
    // (migration 029): a read checks it to fall back to the fold, and nothing the
    // engine reads changes with it, so it has no triggers of the log's own
    let tables: Vec<&String> = read.iter().filter(|t| !t.starts_with("sqlite_") && *t != "names_due").collect();
    for want in ["trade_group_members", "journal_tags", "adjustment_legs", "listings_named", "instruments_described", "instrument_joins", "instrument_names"] {
        assert!(read.contains(want), "the seeded book did not reach {want}: the seed misses a kind of row ({read:?})");
    }
    let mut missing = Vec::new();
    for t in &tables {
        for event in ["insert", "update", "delete"] {
            if !triggers.contains(&(t.to_string(), format!("{t}_changed_{event}"))) {
                missing.push(format!("{t} on {event}"));
            }
        }
    }
    assert!(missing.is_empty(), "read by the ledger and not logged: {missing:?}");
}
