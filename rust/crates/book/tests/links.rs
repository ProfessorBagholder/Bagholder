//! One record superseding others (`docs/architecture.md` §6, "Reconciliation"):
//! nothing is ever counted twice, trades and their notes move with their
//! openings, and a trade that cannot move is kept, orphaned, with its note.

mod common;

use bagholder_book::records::{positions, Incoming};
use bagholder_core::journal::{Anchor, JournalEntry, JournalSubject};
use bagholder_core::record::RecordState;
use bagholder_core::{Leg, SourceName, TransactionId};
use common::*;
use serde_json::json;

fn opening(r: bagholder_core::RecordId) -> TransactionId {
    TransactionId::new(r, Leg::named("trade"))
}

fn note(thesis: &str) -> JournalEntry {
    JournalEntry { thesis: thesis.into(), grade: None, tags: vec![] }
}

#[test]
fn a_superseding_record_takes_the_trade_and_its_note() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let provisional = Spelled { source: "fills", version: 1 };
    let broker = Spelled::v(1);
    let fill = f.store(&provisional, "order-1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "100", "-150", "2026-01-02T15:00:00Z")]));
    let trade = f.book.open_trade(&opening(fill.record), None, t0()).unwrap();
    f.book.set_journal(JournalSubject::Trade(trade), &note("breakout"), t0()).unwrap();

    // the broker's own row arrives and replaces the provisional fill, in one call
    let text = legs(vec![buy("a1", share("CA0000000001", "QNC"), "100", "-150.01", "2026-01-02T15:00:01Z")]).to_string();
    let row = f.book.store_superseding(&broker, &f.incoming("ws-1", &text), &[fill.record], "the broker's row for order-1", t0()).unwrap();

    assert_eq!(f.book.trade(trade).unwrap().anchor, Anchor::Opening(opening(row.record)));
    assert_eq!(f.book.journal(JournalSubject::Trade(trade)).unwrap().unwrap().thesis, "breakout");
    assert_eq!(f.book.record(fill.record).unwrap().state, RecordState::Superseded);
    assert!(f.book.transactions_of(fill.record).unwrap().is_empty());
    assert_eq!(f.book.revisions(fill.record).unwrap().len(), 1, "what the fill said is kept");
    assert_eq!(f.book.superseded_by(fill.record).unwrap(), vec![row.record]);
    // counted once
    let held = positions(&f.book.transactions().unwrap()).unwrap();
    assert_eq!(held.values().copied().collect::<Vec<_>>(), vec![d("100")]);
}

#[test]
fn with_several_candidates_the_earliest_takes_the_trade() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let provisional = Spelled { source: "fills", version: 1 };
    let broker = Spelled::v(1);
    let fill = f.store(&provisional, "order-1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "100", "-150", "2026-01-02T15:00:00Z")]));
    let trade = f.book.open_trade(&opening(fill.record), None, t0()).unwrap();
    // the order filled in two parts, each its own broker row; the later one arrives first
    let late = f.store(&broker, "ws-2", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "40", "-60", "2026-01-02T15:00:09Z")]));
    let early = f.store(&broker, "ws-1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "60", "-90", "2026-01-02T15:00:02Z")]));
    f.book.supersede(&[fill.record], &[late.record, early.record], "two rows for order-1", t0()).unwrap();
    assert_eq!(f.book.trade(trade).unwrap().anchor, Anchor::Opening(opening(early.record)));
    let held = positions(&f.book.transactions().unwrap()).unwrap();
    assert_eq!(held.values().copied().collect::<Vec<_>>(), vec![d("100")]);
}

#[test]
fn a_chain_ends_on_its_last_record() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let a = f.store(&m, "a", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z")]));
    let b = f.store(&m, "b", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z")]));
    let c = f.store(&m, "c", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z")]));
    let trade = f.book.open_trade(&opening(a.record), None, t0()).unwrap();
    f.book.supersede(&[a.record], &[b.record], "b replaces a", t0()).unwrap();
    f.book.supersede(&[b.record], &[c.record], "c replaces b", t0()).unwrap();
    assert_eq!(f.book.trade(trade).unwrap().anchor, Anchor::Opening(opening(c.record)));
    assert_eq!(positions(&f.book.transactions().unwrap()).unwrap().values().copied().collect::<Vec<_>>(), vec![d("1")]);
    // a superseded record cannot be linked again
    assert!(f.book.supersede(&[a.record], &[c.record], "again", t0()).is_err());
}

#[test]
fn a_trade_with_no_counterpart_is_orphaned_and_its_note_kept() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let a = f.store(&m, "a", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z")]));
    let trade = f.book.open_trade(&opening(a.record), None, t0()).unwrap();
    f.book.set_journal(JournalSubject::Trade(trade), &note("keep me"), t0()).unwrap();
    // replaced by a record for another instrument
    let b = f.store(&m, "b", &legs(vec![buy("a1", share("CA0000000002", "XYZ"), "1", "-1", "2026-01-02T15:00:00Z")]));
    f.book.supersede(&[a.record], &[b.record], "corrected", t0()).unwrap();
    let t = f.book.trade(trade).unwrap();
    assert!(matches!(&t.anchor, Anchor::Orphaned(why) if why.contains("replaced")), "{t:?}");
    let orphaned = f.book.orphaned_journal().unwrap();
    assert_eq!((orphaned.len(), orphaned[0].1.thesis.as_str()), (1, "keep me"));
}

#[test]
fn a_removed_superseding_record_brings_nothing_back() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let a = f.store(&m, "a", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z")]));
    let b = f.store(&m, "b", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z")]));
    f.book.supersede(&[a.record], &[b.record], "b replaces a", t0()).unwrap();
    f.book.mark_removed(b.record, t0()).unwrap();
    assert_eq!(f.book.record(a.record).unwrap().state, RecordState::Superseded);
    assert!(f.book.transactions().unwrap().is_empty());
    let problems = f.book.problems().unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!((problems[0].0, problems[0].1.code.as_str()), (b.record, "superseding-record-removed"));
}

#[test]
fn what_the_person_entered_gives_way_to_a_sourced_record() {
    let f = Fixture::new();
    f.account(&["a1"]);
    // the person enters the cost of shares transferred in, which no source had
    let person = Spelled { source: "person", version: 1 };
    let entered = json!({"legs": [{"leg": "trade", "account": "a1", "kind": "transfer-in", "instrument": share("CA0000000001", "QNC"), "quantity": "50", "price": "2.10", "date": "2025-03-01"}]});
    let text = entered.to_string();
    let mine = f.book.store(&person, &Incoming { connection: None, source_key: "cost-basis-1", payload: &text, refs: vec![] }, t0()).unwrap();
    assert_eq!(f.book.record(mine.record).unwrap().source, SourceName::person());
    assert_eq!(f.book.record(mine.record).unwrap().connection, None);
    let trade = f.book.open_trade(&opening(mine.record), None, t0()).unwrap();
    // later, the broker's statement states it
    let stated = json!({"legs": [{"leg": "trade", "account": "a1", "kind": "transfer-in", "instrument": share("CA0000000001", "QNC"), "quantity": "50", "price": "2.12", "date": "2025-03-01"}]}).to_string();
    let sourced = f.book.store_superseding(&Spelled::v(1), &f.incoming("statement-1", &stated), &[mine.record], "the statement states the cost", t0()).unwrap();
    assert_eq!(f.book.trade(trade).unwrap().anchor, Anchor::Opening(opening(sourced.record)));
    let held = f.book.transactions().unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].price, Some(cad("2.12")));
}

#[test]
fn a_link_is_refused_on_records_that_are_not_live_or_on_both_sides() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let a = f.store(&m, "a", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z")]));
    assert!(f.book.supersede(&[a.record], &[a.record], "itself", t0()).is_err());
    assert!(f.book.supersede(&[], &[a.record], "nothing", t0()).is_err());
    let b = f.store(&m, "b", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z")]));
    f.book.mark_removed(b.record, t0()).unwrap();
    assert!(f.book.supersede(&[a.record], &[b.record], "removed", t0()).is_err());
    // a refused link changed nothing
    assert_eq!(f.book.record(a.record).unwrap().state, RecordState::Live);
}

fn option_leg(account: &str, kind: &str, effect: &str, qty: &str, cash: &str, when: &str) -> serde_json::Value {
    let call = json!({"refs": [["occ", "LUNR  270115C00012500"]], "kind": "option", "currency": "USD", "symbol": "LUNR 15JAN27 12.50 CALL"});
    json!({"leg": "trade", "account": account, "kind": kind, "effect": effect, "instrument": call, "quantity": qty, "cash": cash, "currency": "USD", "at": when, "date": &when[..10]})
}

#[test]
fn an_opening_moves_only_to_an_opening_of_the_same_kind() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    // a buy to open, replaced by records holding a buy to close first and a buy to open after
    let opened = f.store(&m, "open", &legs(vec![option_leg("a1", "buy", "open", "1", "-100", "2026-01-02T15:00:00Z")]));
    let trade = f.book.open_trade(&opening(opened.record), None, t0()).unwrap();
    let closing = f.store(&m, "close", &legs(vec![option_leg("a1", "buy", "close", "1", "-90", "2026-01-02T14:00:00Z")]));
    let reopening = f.store(&m, "reopen", &legs(vec![option_leg("a1", "buy", "open", "1", "-100", "2026-01-02T15:00:01Z")]));
    f.book.supersede(&[opened.record], &[closing.record, reopening.record], "split", t0()).unwrap();
    assert_eq!(f.book.trade(trade).unwrap().anchor, Anchor::Opening(opening(reopening.record)));
    // and a revision that turns the opening into a closing orphans the trade
    f.store(&m, "reopen", &legs(vec![option_leg("a1", "buy", "close", "1", "-100", "2026-01-02T15:00:01Z")]));
    assert!(matches!(f.book.trade(trade).unwrap().anchor, Anchor::Orphaned(_)));
}

#[test]
fn the_same_replacement_delivered_again_changes_nothing() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let provisional = Spelled { source: "fills", version: 1 };
    let fill = f.store(&provisional, "order-1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "100", "-150", "2026-01-02T15:00:00Z")]));
    let text = legs(vec![buy("a1", share("CA0000000001", "QNC"), "100", "-150.01", "2026-01-02T15:00:01Z")]).to_string();
    let first = f.book.store_superseding(&Spelled::v(1), &f.incoming("ws-1", &text), &[fill.record], "the broker's row", t0()).unwrap();
    assert_eq!(first.changes.removed, vec![opening(fill.record)], "the fill's transaction left the book");
    let before = everything(f.dir.path());
    let again = f.book.store_superseding(&Spelled::v(1), &f.incoming("ws-1", &text), &[fill.record], "the broker's row", t0()).unwrap();
    assert_eq!((again.outcome, again.changes), (bagholder_book::records::Outcome::Unchanged, bagholder_book::records::Changes::default()));
    assert_eq!(everything(f.dir.path()), before);
    // a record another record replaced cannot be claimed by a third
    let other = legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-03T15:00:00Z")]).to_string();
    assert!(f.book.store_superseding(&Spelled::v(1), &f.incoming("ws-2", &other), &[fill.record], "wrong", t0()).is_err());
}

#[test]
fn candidates_at_the_same_instant_are_taken_in_record_order() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let fill = f.store(&Spelled { source: "fills", version: 1 }, "order-1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "2", "-2", "2026-01-02T15:00:00Z")]));
    let trade = f.book.open_trade(&opening(fill.record), None, t0()).unwrap();
    let x = f.store(&m, "x", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:05Z")]));
    let y = f.store(&m, "y", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:05Z")]));
    f.book.supersede(&[fill.record], &[x.record, y.record], "two rows", t0()).unwrap();
    let first = if x.record < y.record { x.record } else { y.record };
    assert_eq!(f.book.trade(trade).unwrap().anchor, Anchor::Opening(opening(first)));
}

#[test]
fn cash_is_counted_once_after_a_replacement() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let fill = f.store(&Spelled { source: "fills", version: 1 }, "order-1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "100", "-150", "2026-01-02T15:00:00Z")]));
    let text = legs(vec![buy("a1", share("CA0000000001", "QNC"), "100", "-150.01", "2026-01-02T15:00:01Z")]).to_string();
    f.book.store_superseding(&Spelled::v(1), &f.incoming("ws-1", &text), &[fill.record], "the broker's row", t0()).unwrap();
    let cash: Vec<bagholder_core::Money> = f.book.transactions().unwrap().iter().filter_map(|t| t.cash).collect();
    assert_eq!(bagholder_core::Money::sum(bagholder_core::Currency::CAD, cash).unwrap(), cad("-150.01"));
}
