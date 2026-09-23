//! Source records and their transactions (`docs/architecture.md` §6): kept as
//! received, revised only when what the source said changed, derived again by a
//! new mapping, and taken out of the count only when the source says so.

mod common;

use bagholder_book::records::{Changes, Outcome};
use bagholder_core::journal::{Anchor, JournalEntry, JournalSubject};
use bagholder_core::record::RecordState;
use bagholder_core::transaction::Kind;
use bagholder_core::{Leg, TransactionId};
use common::*;
use serde_json::json;

#[test]
fn the_same_payload_again_writes_nothing() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let p = legs(vec![buy("a1", share("CA0000000001", "QNC"), "10", "-100.50", "2026-01-02T15:00:00Z")]);
    assert_eq!(f.store(&m, "r1", &p).outcome, Outcome::New);
    let before = everything(f.dir.path());
    let again = f.store(&m, "r1", &p);
    assert_eq!((again.outcome, again.changes), (Outcome::Unchanged, Changes::default()));
    assert_eq!(everything(f.dir.path()), before);
}

#[test]
fn the_same_content_in_another_order_or_spelling_is_the_same_payload() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let first = r#"{"legs":[{"leg":"trade","account":"a1","kind":"buy","quantity":"10","cash":"-100.50","at":"2026-01-02T15:00:00Z","date":"2026-01-02","instrument":{"refs":[["isin","CA0000000001"]],"kind":"security","currency":"CAD","symbol":"QNC"}}],"n":1.10}"#;
    let reordered = r#"{ "n": 11e-1, "legs": [ { "instrument": { "symbol": "QNC", "currency": "CAD", "kind": "security", "refs": [ [ "isin", "CA0000000001" ] ] }, "date": "2026-01-02", "at": "2026-01-02T15:00:00Z", "cash": "-100.50", "quantity": "10", "kind": "buy", "account": "a1", "leg": "trade" } ] }"#;
    f.book.store(&m, &f.incoming("r1", first), t0()).unwrap();
    let before = everything(f.dir.path());
    let again = f.book.store(&m, &f.incoming("r1", reordered), t0()).unwrap();
    assert_eq!(again.outcome, Outcome::Unchanged);
    assert_eq!(everything(f.dir.path()), before);
}

#[test]
fn a_changed_payload_is_a_revision_and_is_derived_again() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let r = f.store(&m, "r1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "10", "-100", "2026-01-02T15:00:00Z")]));
    let revised = f.store(&m, "r1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "10", "-99.95", "2026-01-02T15:00:00Z")]));
    assert_eq!(revised.record, r.record);
    assert_eq!(revised.outcome, Outcome::Revised(2));
    let id = TransactionId::new(r.record, Leg::named("trade"));
    assert_eq!(revised.changes, Changes { changed: vec![id.clone()], ..Changes::default() });
    assert_eq!(f.book.transaction(&id).unwrap().unwrap().cash, Some(cad("-99.95")));
    let revisions = f.book.revisions(r.record).unwrap();
    assert_eq!(revisions.len(), 2, "the first revision is kept");
    assert!(revisions[0].2.contains("\"-100\""));
}

#[test]
fn two_writers_storing_one_record_make_one_record() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let path = f.dir.path().to_path_buf();
    let conn = f.connection;
    let p = legs(vec![buy("a1", share("CA0000000001", "QNC"), "10", "-100", "2026-01-02T15:00:00Z")]).to_string();
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let (path, p) = (path.clone(), p.clone());
            std::thread::spawn(move || {
                let (book, _) = bagholder_book::Book::open_in(&path, "test", t0()).unwrap();
                let incoming = bagholder_book::records::Incoming { connection: Some(conn), source_key: "r1", payload: &p, refs: vec![] };
                book.store(&Spelled::v(1), &incoming, t0()).unwrap().outcome
            })
        })
        .collect();
    let outcomes: Vec<Outcome> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|o| **o == Outcome::New).count(), 1, "{outcomes:?}");
    assert_eq!(outcomes.iter().filter(|o| **o == Outcome::Unchanged).count(), 3, "{outcomes:?}");
    assert_eq!(f.book.transactions().unwrap().len(), 1);
}

#[test]
fn a_record_missing_from_a_pull_is_left_alone_and_only_a_reported_removal_removes() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let r1 = f.store(&m, "r1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "10", "-100", "2026-01-02T15:00:00Z")]));
    // a later pull that carries only another record
    f.store(&m, "r2", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "5", "-50", "2026-01-03T15:00:00Z")]));
    assert_eq!(f.book.record(r1.record).unwrap().state, RecordState::Live);
    assert_eq!(f.book.transactions_of(r1.record).unwrap().len(), 1);
    // the source says it is gone
    let trade = f.book.open_trade(&TransactionId::new(r1.record, Leg::named("trade")), None, t0()).unwrap();
    let changes = f.book.mark_removed(r1.record, at("2026-09-24T00:00:00Z")).unwrap();
    assert_eq!(changes.removed, vec![TransactionId::new(r1.record, Leg::named("trade"))]);
    assert_eq!(f.book.record(r1.record).unwrap().state, RecordState::Removed);
    assert!(f.book.transactions_of(r1.record).unwrap().is_empty());
    assert!(matches!(f.book.trade(trade).unwrap().anchor, Anchor::Orphaned(_)));
    assert_eq!(f.book.revisions(r1.record).unwrap().len(), 1, "what the source said is kept");
}

#[test]
fn a_payload_the_mapping_cannot_read_is_kept_with_a_problem() {
    let f = Fixture::new();
    let m = Spelled::v(1);
    let r = f.store(&m, "r1", &json!({"something": "the mapping does not know"}));
    assert_eq!(f.book.record(r.record).unwrap().state, RecordState::Live);
    assert!(f.book.transactions_of(r.record).unwrap().is_empty());
    assert_eq!(f.book.problems_of(r.record).unwrap()[0].code, "unreadable");
    assert_eq!(f.book.revisions(r.record).unwrap().len(), 1);
    // text that is not JSON cannot be kept at all, and says so
    assert!(f.book.store(&m, &f.incoming("r2", "{not json"), t0()).is_err());
}

#[test]
fn a_new_mapping_version_derives_again_and_says_exactly_what_moved() {
    let f = Fixture::new();
    f.account(&["a1"]);
    // version 1 books a fee as part of the cash and cannot read the third record
    let v1 = Spelled::v(1);
    let kept = legs(vec![buy("a1", share("CA0000000001", "QNC"), "10", "-100", "2026-01-02T15:00:00Z")]);
    let moved = json!({"legs": [buy("a1", share("CA0000000001", "QNC"), "5", "-51", "2026-01-03T15:00:00Z")], "v2": {"legs": [
        {"leg": "trade", "account": "a1", "kind": "buy", "instrument": share("CA0000000001", "QNC"), "quantity": "5", "cash": "-50", "at": "2026-01-03T15:00:00Z", "date": "2026-01-03"},
        {"leg": "fee", "account": "a1", "kind": "fee", "cash": "-1", "at": "2026-01-03T15:00:00Z", "date": "2026-01-03"}
    ]}});
    let failed = json!({"v2": {"legs": [{"leg": "trade", "account": "a1", "kind": "dividend", "cash": "3", "date": "2026-01-04"}]}});
    let gone = json!({"legs": [{"leg": "trade", "account": "a1", "kind": "interest", "cash": "1", "date": "2026-01-05"}], "v2": {"legs": []}});
    let a = f.store(&v1, "kept", &json!({"legs": kept["legs"], "v2": kept}));
    let b = f.store(&v1, "moved", &moved);
    let c = f.store(&v1, "failed", &failed);
    let e = f.store(&v1, "gone", &gone);
    assert_eq!(f.book.problems_of(c.record).unwrap()[0].code, "unreadable");

    // version 2 reads the `v2` part of each payload
    struct V2;
    impl bagholder_book::mapping::Mapping for V2 {
        fn source(&self) -> bagholder_core::SourceName {
            bagholder_core::SourceName::named("test-broker")
        }
        fn version(&self) -> u32 {
            2
        }
        fn map(&self, ctx: &bagholder_book::mapping::MapContext, payload: &str) -> bagholder_book::mapping::Mapped {
            let v: serde_json::Value = serde_json::from_str(payload).unwrap();
            Spelled::v(2).map(ctx, &v["v2"].to_string())
        }
    }
    let trade = f.book.open_trade(&TransactionId::new(a.record, Leg::named("trade")), None, t0()).unwrap();
    let mut changes = f.book.rederive(&V2, t0()).unwrap();
    changes.added.sort();
    let leg = |r: bagholder_core::RecordId, l: &'static str| TransactionId::new(r, Leg::named(l));
    let mut expected = vec![leg(b.record, "fee"), leg(c.record, "trade")];
    expected.sort();
    assert_eq!(changes.added, expected);
    assert_eq!(changes.changed, vec![leg(b.record, "trade")]);
    assert_eq!(changes.removed, vec![leg(e.record, "trade")]);
    // the record whose content did not move keeps its id and its trade
    assert!(!changes.added.contains(&leg(a.record, "trade")) && !changes.changed.contains(&leg(a.record, "trade")));
    assert_eq!(f.book.trade(trade).unwrap().anchor, Anchor::Opening(leg(a.record, "trade")));
    assert_eq!(f.book.transaction(&leg(a.record, "trade")).unwrap().unwrap().mapping.version, 2);
    assert!(f.book.problems_of(c.record).unwrap().is_empty(), "the record that failed is read by the new version");
    // the same version again changes nothing
    let before = everything(f.dir.path());
    assert_eq!(f.book.rederive(&V2, t0()).unwrap(), Changes::default());
    assert_eq!(everything(f.dir.path()), before);
}

#[test]
fn a_leg_added_before_another_leaves_the_others_id_and_its_trade() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let v1 = Spelled::v(1);
    let payload = json!({
        "legs": [buy("a1", share("CA0000000001", "QNC"), "10", "-100", "2026-01-02T15:00:00Z")],
        "v2": {"legs": [
            {"leg": "withholding", "account": "a1", "kind": "withholding-tax", "cash": "-1", "date": "2026-01-02"},
            buy("a1", share("CA0000000001", "QNC"), "10", "-100", "2026-01-02T15:00:00Z")
        ]}
    });
    let r = f.store(&v1, "r1", &payload);
    let opening = TransactionId::new(r.record, Leg::named("trade"));
    let trade = f.book.open_trade(&opening, None, t0()).unwrap();
    f.book.set_journal(JournalSubject::Trade(trade), &JournalEntry { thesis: "kept".into(), grade: None, tags: vec![] }, t0()).unwrap();
    struct V2;
    impl bagholder_book::mapping::Mapping for V2 {
        fn source(&self) -> bagholder_core::SourceName {
            bagholder_core::SourceName::named("test-broker")
        }
        fn version(&self) -> u32 {
            2
        }
        fn map(&self, ctx: &bagholder_book::mapping::MapContext, payload: &str) -> bagholder_book::mapping::Mapped {
            let v: serde_json::Value = serde_json::from_str(payload).unwrap();
            Spelled::v(2).map(ctx, &v["v2"].to_string())
        }
    }
    let changes = f.book.rederive(&V2, t0()).unwrap();
    assert_eq!(changes.added, vec![TransactionId::new(r.record, Leg::named("withholding"))]);
    assert!(changes.changed.is_empty());
    assert_eq!(f.book.trade(trade).unwrap().anchor, Anchor::Opening(opening));
    assert_eq!(f.book.journal(JournalSubject::Trade(trade)).unwrap().unwrap().thesis, "kept");
}

#[test]
fn a_trade_whose_opening_now_says_something_else_is_orphaned_with_its_note() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let r = f.store(&m, "r1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "10", "-100", "2026-01-02T15:00:00Z")]));
    let opening = TransactionId::new(r.record, Leg::named("trade"));
    let trade = f.book.open_trade(&opening, None, t0()).unwrap();
    f.book.set_journal(JournalSubject::Trade(trade), &JournalEntry { thesis: "why I bought".into(), grade: Some(bagholder_core::journal::Grade::B), tags: vec!["core".into()] }, t0()).unwrap();
    // the source revises the row: it was a sale
    f.store(&m, "r1", &legs(vec![sell("a1", share("CA0000000001", "QNC"), "-10", "100", "2026-01-02T15:00:00Z")]));
    assert!(matches!(f.book.trade(trade).unwrap().anchor, Anchor::Orphaned(_)));
    let orphaned = f.book.orphaned_journal().unwrap();
    assert_eq!(orphaned.len(), 1);
    assert_eq!(orphaned[0].1.thesis, "why I bought");
}

#[test]
fn a_record_does_not_state_what_it_does_not_state() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let r = f.store(&m, "r1", &json!({"legs": [{"leg": "trade", "account": "a1", "kind": "buy", "instrument": share("CA0000000001", "QNC"), "cash": "-100", "date": "2026-01-02"}], "problems": ["quantity-not-stated"]}));
    let t = &f.book.transactions_of(r.record).unwrap()[0];
    assert_eq!((t.kind, t.quantity, t.price, t.occurred_at), (Kind::Buy, None, None, None));
    assert_eq!(f.book.problems_of(r.record).unwrap()[0].code, "quantity-not-stated");
    // a quantity with no instrument, or a price with no quantity, is refused whole
    let bad = f.store(&m, "r2", &json!({"legs": [{"leg": "trade", "account": "a1", "kind": "buy", "quantity": "1", "cash": "-1", "date": "2026-01-02"}]}));
    assert_eq!(f.book.problems_of(bad.record).unwrap()[0].code, "quantity-without-instrument");
    assert!(f.book.transactions_of(bad.record).unwrap().is_empty());
    let dup = f.store(&m, "r3", &legs(vec![
        buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z"),
        buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z"),
    ]));
    assert_eq!(f.book.problems_of(dup.record).unwrap()[0].code, "duplicate-leg");
    assert!(f.book.transactions_of(dup.record).unwrap().is_empty(), "a record is never half counted");
}
