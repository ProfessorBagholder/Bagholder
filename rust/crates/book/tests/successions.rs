//! A source's own succession (`docs/architecture.md` §5, "Matching across
//! sources"): an id a broker states retired by a corporate action and the one
//! id it trades the same listing under now are read as one instrument, unless a
//! corporate event row gives up the retired id's units. The join is decided
//! from the records each time and merges nothing, so the order the rows arrive
//! in never changes what the book reads: an event row arriving after the two
//! ids were read as one parts them, and the book reads as if it had come first.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use bagholder_book::identity::Successions;
use bagholder_book::statements::UnitsLine;
use bagholder_book::watched::{ListingDraft, ListingName};
use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
use bagholder_core::journal::{Anchor, JournalEntry, JournalSubject, Opening};
use bagholder_core::{AccountId, Broker, Currency, InstrumentId, RecordId, TradeId};
use common::*;
use serde_json::{json, Value};

const OLD: &str = "sec-s-old";
const NEW: &str = "sec-s-new";

/// `CH` on TSX-V as the broker states it under `id`, standing `standing`.
fn ch_as(id: &str, name: &str, standing: &str) -> Value {
    json!({"refs": [["broker-security:wealthsimple", id]], "kind": "security", "currency": "CAD", "symbol": "CH", "mic": "XTSX", "name": name, "standing": standing})
}

fn old() -> Value {
    ch_as(OLD, "Charbone Hydrogen Corp", "retired-by-event")
}

fn new() -> Value {
    ch_as(NEW, "Charbone Corp.", "live")
}

fn ws(id: &str) -> Reference {
    Reference::new(RefScheme::BrokerSecurity(Broker::named("wealthsimple")), id)
}

/// The rows: 1,000 bought under the old id, a change of code giving up the
/// 1,000 under the old id for 1,000 under the new, 400 sold under the new.
fn row(key: &str) -> Value {
    match key {
        "buy" => legs(vec![buy("a1", old(), "1000", "-150", "2026-01-02T15:00:00Z")]),
        "event" => json!({"legs": [
            {"leg": "trade", "account": "a1", "kind": "corporate-event", "instrument": old(), "quantity": "-1000", "at": "2026-06-01T15:00:00Z", "date": "2026-06-01"},
            {"leg": "receive", "account": "a1", "kind": "corporate-event", "instrument": new(), "quantity": "1000", "at": "2026-06-01T15:00:00Z", "date": "2026-06-01"}
        ]}),
        "sell" => legs(vec![sell("a1", new(), "-400", "60", "2026-07-02T15:00:00Z")]),
        other => panic!("no row {other}"),
    }
}

fn record(f: &Fixture, key: &str) -> RecordId {
    f.book.record_by_key(Some(f.connection), &bagholder_core::SourceName::named("test-broker"), key).unwrap().unwrap()
}

fn instrument_of(f: &Fixture, key: &str) -> InstrumentId {
    f.book.transactions_of(record(f, key)).unwrap()[0].instrument.unwrap()
}

/// A book with account `a1` and `keys` stored in that order.
fn book_of(keys: &[&str]) -> (Fixture, AccountId) {
    let f = Fixture::new();
    let a = f.account(&["a1"]);
    for k in keys {
        f.store(&Spelled::v(1), k, &row(k));
    }
    (f, a)
}

/// What the person and the broker add: a trade on the purchase with a note, a
/// watch on the listing under its new id, and the broker's statement of 600
/// held under it.
fn add_the_rest(f: &Fixture, a: AccountId) -> TradeId {
    let trade = f.book.open_trade(&f.opens(record(f, "buy")), None, t0()).unwrap();
    f.book.set_journal(JournalSubject::Trade(trade), &JournalEntry { thesis: "held through the new id".into(), grade: None, tags: vec![] }, t0()).unwrap();
    let draft = ListingDraft {
        found: None,
        kind: InstrumentKind::Security,
        currency: Currency::CAD,
        refs: vec![ws(NEW)],
        name: ListingName { symbol: "CH".into(), venue_mic: Some("XTSX".into()), venue_name: None, name: None },
    };
    f.book.watch(&draft, t0()).unwrap();
    let read = f.book.broker_read(f.connection, "units", t0()).unwrap();
    let held = f.book.own_instrument(&ws(NEW)).unwrap().unwrap();
    f.book.store_units(a, "2026-07-03".parse().unwrap(), &[UnitsLine { instrument: held, quantity: d("600"), book_value: None, value: None }], &read, t0()).unwrap();
    trade
}

/// An instrument as what it stands for: the broker ids read as it.
type Ids = Vec<String>;

/// The book as it reads, every id Bagholder made replaced by what it stands
/// for (an instrument by the broker ids read as it, a record by its key), so
/// two books built in different orders are equal exactly when they read alike.
#[derive(Debug, PartialEq)]
struct Reading {
    instruments: BTreeSet<Ids>,
    transactions: BTreeSet<(String, String, String, Option<Ids>, Option<String>, Option<String>)>,
    held: BTreeMap<Ids, String>,
    names: BTreeMap<Ids, Vec<(String, Option<String>, String, String)>>,
    watched: Vec<Ids>,
    stated: Option<BTreeMap<Ids, String>>,
    trades: BTreeSet<(String, String, Ids, Option<String>)>,
}

fn reading(f: &Fixture, a: AccountId) -> Reading {
    let ids = |i: InstrumentId| -> Ids {
        let mut v: Vec<String> = f.book.instrument_refs(i).unwrap().into_iter().filter(|r| r.identifies()).map(|r| r.value).collect();
        v.sort();
        v
    };
    let key = |r: RecordId| f.book.record(r).unwrap().source_key;
    let transactions = f.book.transactions().unwrap();
    Reading {
        instruments: f.book.instruments().unwrap().into_iter().map(|i| ids(i.id)).collect(),
        transactions: transactions
            .iter()
            .map(|t| (key(t.id.record), t.id.leg.to_string(), t.kind.to_string(), t.instrument.map(ids), t.quantity.map(|q| q.to_text()), t.cash.map(|c| c.amount.to_text())))
            .collect(),
        held: bagholder_book::records::positions(&transactions).unwrap().into_iter().map(|((_, i), q)| (ids(i), q.to_text())).collect(),
        names: f
            .book
            .instruments()
            .unwrap()
            .into_iter()
            .map(|i| (ids(i.id), f.book.names(i.id).unwrap().into_iter().map(|n| (n.symbol, n.name, n.first_seen.to_string(), n.last_seen.to_string())).collect()))
            .collect(),
        watched: f.book.watched().unwrap().into_iter().map(|w| ids(w.instrument)).collect(),
        stated: f.book.stated(a).unwrap().units.map(|(_, m)| m.into_iter().map(|(i, q)| (ids(i), q.to_text())).collect()),
        trades: f
            .book
            .trades()
            .unwrap()
            .into_iter()
            .map(|t| {
                let Anchor::Opening(o) = t.anchor else { panic!("trade {} orphaned: {:?}", t.id, t.anchor) };
                let note = f.book.journal(JournalSubject::Trade(t.id)).unwrap().map(|j| j.thesis);
                (key(o.transaction.record), o.transaction.leg.to_string(), ids(o.instrument), note)
            })
            .collect(),
    }
}

#[test]
fn an_id_the_broker_retired_by_a_corporate_action_and_its_listing_s_live_id_are_read_as_one() {
    // no event row: bought under the old id, sold under the new, one holding
    let (f, a) = book_of(&["buy", "sell"]);
    let one = instrument_of(&f, "buy");
    assert_eq!(instrument_of(&f, "sell"), one);
    assert_eq!(f.book.instruments().unwrap().iter().map(|i| i.id).collect::<Vec<_>>(), vec![one]);
    assert_eq!(bagholder_book::records::positions(&f.book.transactions().unwrap()).unwrap().get(&(a, one)), Some(&d("600")));
    for id in [OLD, NEW] {
        assert_eq!(f.book.instrument_by_ref(&ws(id)).unwrap(), Some(one));
    }
    // nothing merged: each id keeps its own instrument, the first seen is read
    let (own_old, own_new) = (f.book.own_instrument(&ws(OLD)).unwrap().unwrap(), f.book.own_instrument(&ws(NEW)).unwrap().unwrap());
    assert_eq!(own_old, one);
    assert_ne!(own_new, one);
    assert_eq!(f.book.canonical(own_new).unwrap(), one);
    // the id it trades under comes first among its references
    assert_eq!(f.book.instrument_refs(one).unwrap().iter().map(|r| r.value.as_str()).collect::<Vec<_>>(), vec![NEW, OLD]);
    // its names: one listing, as the broker called it last
    let names = f.book.names(one).unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].name.as_deref(), Some("Charbone Corp."));
    // what was chosen under the new id reads as the one instrument
    add_the_rest(&f, a);
    assert_eq!(f.book.watched().unwrap().iter().map(|w| w.instrument).collect::<Vec<_>>(), vec![one]);
    assert_eq!(f.book.stated(a).unwrap().units.unwrap().1, BTreeMap::from([(one, d("600"))]));
    // settled already: a row read later under either id lands on it, nothing to do
    f.store(&Spelled::v(1), "later", &legs(vec![buy("a1", new(), "5", "-1", "2026-08-02T15:00:00Z")]));
    assert_eq!(instrument_of(&f, "later"), one);
    assert_eq!(f.book.settle_successions().unwrap(), Successions::default());
    // derived again, the same
    f.book.rederive(&Spelled::v(2), t0()).unwrap();
    assert_eq!(instrument_of(&f, "sell"), one);
}

#[test]
fn a_retired_id_joins_its_successor_whichever_is_seen_first() {
    let (f, a) = book_of(&["sell", "buy"]);
    let first = f.book.own_instrument(&ws(NEW)).unwrap().unwrap();
    assert_eq!(instrument_of(&f, "buy"), first, "read as the instrument first seen");
    assert_eq!(bagholder_book::records::positions(&f.book.transactions().unwrap()).unwrap().get(&(a, first)), Some(&d("600")));
}

#[test]
fn a_retired_id_with_none_or_two_live_ids_of_its_listing_stays_its_own() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    f.store(&m, "buy", &legs(vec![buy("a1", old(), "10", "-1", "2026-01-02T15:00:00Z")]));
    // another listing's live id: TSX, not TSX-V
    let tsx = json!({"refs": [["broker-security:wealthsimple", "sec-s-tsx"]], "kind": "security", "currency": "CAD", "symbol": "CH", "mic": "XTSE", "standing": "live"});
    f.store(&m, "r2", &legs(vec![buy("a1", tsx, "1", "-1", "2026-07-02T15:00:00Z")]));
    // delisted, not live
    f.store(&m, "r3", &legs(vec![buy("a1", ch_as("sec-s-gone", "Other", "delisted"), "1", "-1", "2026-07-02T16:00:00Z")]));
    assert_eq!(f.book.settle_successions().unwrap(), Successions::default());
    assert_eq!(f.book.instruments().unwrap().len(), 3);
    // two live ids of the listing: which one it became is not stated
    f.store(&m, "r4", &legs(vec![buy("a1", ch_as("sec-s-new-1", "One", "live"), "1", "-1", "2026-07-02T17:00:00Z")]));
    f.store(&m, "r5", &legs(vec![buy("a1", ch_as("sec-s-new-2", "Two", "live"), "1", "-1", "2026-07-02T18:00:00Z")]));
    assert_eq!(f.book.instruments().unwrap().len(), 5);
    assert_eq!(f.book.instrument_refs(instrument_of(&f, "buy")).unwrap().len(), 1);
}

#[test]
fn an_id_a_corporate_event_row_gives_up_is_that_row_s_succession_not_a_join() {
    let (f, a) = book_of(&["buy", "event", "sell"]);
    assert!(f.book.problems_of(record(&f, "event")).unwrap().is_empty(), "{:?}", f.book.problems_of(record(&f, "event")));
    let (old_i, new_i) = (instrument_of(&f, "buy"), instrument_of(&f, "sell"));
    assert_ne!(old_i, new_i);
    let held = bagholder_book::records::positions(&f.book.transactions().unwrap()).unwrap();
    assert_eq!((held.get(&(a, old_i)), held.get(&(a, new_i))), (Some(&d("0")), Some(&d("600"))));
}

#[test]
fn an_event_row_arriving_after_the_ids_were_joined_parts_them_as_if_it_had_come_first() {
    // the ids read as one, the person writes on the trade and watches the
    // listing, the broker states what is held; then the event row arrives
    let (late, a) = book_of(&["buy", "sell"]);
    let joined = instrument_of(&late, "buy");
    assert_eq!(instrument_of(&late, "sell"), joined);
    let trade = add_the_rest(&late, a);
    let stored = late.store(&Spelled::v(1), "event", &row("event"));
    assert!(late.book.problems_of(stored.record).unwrap().is_empty(), "{:?}", late.book.problems_of(stored.record));
    // two instruments again, each row on the one its id names
    assert_ne!(instrument_of(&late, "buy"), instrument_of(&late, "sell"));
    assert_eq!(late.book.joined().unwrap(), BTreeMap::new());
    // the trade keeps its id, its opening and its note
    assert_eq!(late.book.trade(trade).unwrap().anchor, Anchor::Opening(late.opens(record(&late, "buy"))));
    assert_eq!(late.book.journal(JournalSubject::Trade(trade)).unwrap().unwrap().thesis, "held through the new id");

    let (first, b) = book_of(&["buy", "event", "sell"]);
    add_the_rest(&first, b);
    let r = reading(&late, a);
    assert_eq!(r, reading(&first, b));
    // and what it reads is the event's succession
    assert_eq!(r.held, BTreeMap::from([(vec![OLD.to_string()], "0".to_string()), (vec![NEW.to_string()], "600".to_string())]));
    assert_eq!(r.watched, vec![vec![NEW.to_string()]]);
    assert_eq!(r.stated, Some(BTreeMap::from([(vec![NEW.to_string()], "600".to_string())])));
}

#[test]
fn the_rows_read_the_same_whatever_order_they_arrive_in() {
    let orders: [[&str; 3]; 6] =
        [["buy", "event", "sell"], ["buy", "sell", "event"], ["event", "buy", "sell"], ["event", "sell", "buy"], ["sell", "buy", "event"], ["sell", "event", "buy"]];
    let mut readings = Vec::new();
    for keys in orders {
        let (f, a) = book_of(&keys);
        add_the_rest(&f, a);
        readings.push((keys, reading(&f, a)));
    }
    for (keys, r) in &readings[1..] {
        assert_eq!(r, &readings[0].1, "{keys:?} reads otherwise than {:?}", readings[0].0);
    }
}

#[test]
fn the_event_row_removed_joins_the_ids_again() {
    let (f, a) = book_of(&["buy", "sell", "event"]);
    let trade = add_the_rest(&f, a);
    f.book.mark_removed(record(&f, "event"), t0()).unwrap();
    assert_eq!(instrument_of(&f, "buy"), instrument_of(&f, "sell"));
    let (never, b) = book_of(&["buy", "sell"]);
    add_the_rest(&never, b);
    assert_eq!(reading(&f, a), reading(&never, b));
    assert!(matches!(f.book.trade(trade).unwrap().anchor, Anchor::Opening(_)));
}

/// Put `f` in the state an earlier build left a joined book in: it merged the
/// new id's instrument into the old one's, moving everything onto it and
/// deleting it.
fn merge_as_an_earlier_build_did(f: &Fixture) {
    let keep = f.book.own_instrument(&ws(OLD)).unwrap().unwrap().to_string();
    let gone = f.book.own_instrument(&ws(NEW)).unwrap().unwrap().to_string();
    let c = rusqlite::Connection::open(f.dir.path().join("book.db")).unwrap();
    for sql in [
        "UPDATE instrument_refs SET instrument_id = ?1 WHERE instrument_id = ?2",
        "UPDATE instrument_sightings SET instrument_id = ?1 WHERE instrument_id = ?2",
        "UPDATE transactions SET instrument_id = ?1 WHERE instrument_id = ?2",
        "UPDATE trades SET anchor_instrument = ?1 WHERE anchor_instrument = ?2",
        "UPDATE OR IGNORE watched SET instrument_id = ?1 WHERE instrument_id = ?2",
        "UPDATE statement_units SET instrument_id = ?1 WHERE instrument_id = ?2",
        "UPDATE adjustment_legs SET to_instrument = ?1 WHERE to_instrument = ?2",
    ] {
        c.execute(sql, [&keep, &gone]).unwrap();
    }
    c.execute_batch(&format!(
        "UPDATE OR IGNORE listings_named SET instrument_id = '{keep}' WHERE instrument_id = '{gone}';
         DELETE FROM listings_named WHERE instrument_id = '{gone}'; DELETE FROM watched WHERE instrument_id = '{gone}';
         DELETE FROM instrument_joins; DELETE FROM instruments WHERE id = '{gone}';"
    ))
    .unwrap();
}

#[test]
fn a_book_an_earlier_build_merged_reads_the_same_once_repaired() {
    let (f, a) = book_of(&["buy", "sell"]);
    let trade = add_the_rest(&f, a);
    let before = reading(&f, a);
    merge_as_an_earlier_build_did(&f);
    f.book.repair_merged_successions(&[&Spelled::v(1)], t0()).unwrap();
    assert_eq!(reading(&f, a), before);
    assert_ne!(f.book.own_instrument(&ws(OLD)).unwrap(), f.book.own_instrument(&ws(NEW)).unwrap(), "each id on its own instrument again");
    assert_eq!(f.book.trade(trade).unwrap().anchor, Anchor::Opening(f.opens(record(&f, "buy"))));
    // done once
    f.book.repair_merged_successions(&[&Spelled::v(1)], t0()).unwrap();
    assert_eq!(reading(&f, a), before);
}

#[test]
fn a_merged_book_whose_event_row_came_later_is_parted_by_the_repair() {
    // the earlier build merged the ids; the event row then landed on the one
    // instrument, giving up and receiving units on it
    let (f, a) = book_of(&["sell", "buy"]);
    let trade = add_the_rest(&f, a);
    // it kept the instrument first seen: the new id's, here
    let (keep, gone) = (f.book.own_instrument(&ws(NEW)).unwrap().unwrap().to_string(), f.book.own_instrument(&ws(OLD)).unwrap().unwrap().to_string());
    let c = rusqlite::Connection::open(f.dir.path().join("book.db")).unwrap();
    for sql in [
        "UPDATE instrument_refs SET instrument_id = ?1 WHERE instrument_id = ?2",
        "UPDATE instrument_sightings SET instrument_id = ?1 WHERE instrument_id = ?2",
        "UPDATE transactions SET instrument_id = ?1 WHERE instrument_id = ?2",
        "UPDATE trades SET anchor_instrument = ?1 WHERE anchor_instrument = ?2",
    ] {
        c.execute(sql, [&keep, &gone]).unwrap();
    }
    c.execute_batch(&format!("DELETE FROM instrument_joins; DELETE FROM instruments WHERE id = '{gone}';")).unwrap();
    drop(c);
    f.store(&Spelled::v(1), "event", &row("event"));
    assert_eq!(f.book.instruments().unwrap().len(), 1, "merged, the event row lands on the one instrument");
    f.book.repair_merged_successions(&[&Spelled::v(1)], t0()).unwrap();

    let (first, b) = book_of(&["sell", "event", "buy"]);
    add_the_rest(&first, b);
    let (repaired, reference) = (reading(&f, a), reading(&first, b));
    assert_eq!(repaired.instruments, reference.instruments);
    assert_eq!(repaired.transactions, reference.transactions);
    assert_eq!(repaired.held, reference.held);
    assert_eq!(repaired.names, reference.names);
    assert_eq!(repaired.trades, reference.trades);
    // the trade on the purchase under the old id followed it, its note kept
    let Anchor::Opening(Opening { instrument, .. }) = f.book.trade(trade).unwrap().anchor else { panic!("orphaned") };
    assert_eq!(Some(instrument), f.book.instrument_by_ref(&ws(OLD)).unwrap());
    assert_eq!(f.book.journal(JournalSubject::Trade(trade)).unwrap().unwrap().thesis, "held through the new id");
    // a statement the merge added up under both ids is not read against one
    assert_eq!(repaired.stated, None);
}
