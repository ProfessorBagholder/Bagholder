//! Identity (`docs/architecture.md` §5): instruments are found only by what
//! identifies them, never merged on a symbol; accounts carry their broker ids.

mod common;

use bagholder_core::account::{AccountRef, AccountStatus, AccountType};
use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
use bagholder_core::Broker;
use common::*;
use serde_json::json;

fn instrument_of(f: &Fixture, record: bagholder_core::RecordId) -> bagholder_core::InstrumentId {
    f.book.transactions_of(record).unwrap()[0].instrument.unwrap()
}

#[test]
fn a_strong_reference_finds_the_instrument_it_named() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let one = f.store(&m, "r1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "10", "-100", "2026-01-02T15:00:00Z")]));
    let two = f.store(&m, "r2", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "5", "-50", "2026-01-03T15:00:00Z")]));
    assert_eq!(instrument_of(&f, one.record), instrument_of(&f, two.record));
    let id = instrument_of(&f, one.record);
    assert_eq!(f.book.instrument_by_ref(&Reference::new(RefScheme::Isin, "CA0000000001")).unwrap(), Some(id));
    assert_eq!(f.book.instruments().unwrap().len(), 1);
}

#[test]
fn one_ticker_under_two_security_ids_is_two_instruments() {
    // Charbone Hydrogen and Charbone Corp.: `CH` on TSX-V under two broker ids
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let ch = |id: &str, name: &str| json!({"refs": [["broker-security:wealthsimple", id]], "kind": "security", "currency": "CAD", "symbol": "CH", "mic": "XTSX", "name": name});
    let old = f.store(&m, "r1", &legs(vec![buy("a1", ch("sec-s-old", "Charbone Hydrogen Corp"), "100", "-15", "2026-01-02T15:00:00Z")]));
    let new = f.store(&m, "r2", &legs(vec![buy("a1", ch("sec-s-new", "Charbone Corp."), "100", "-15", "2026-07-02T15:00:00Z")]));
    assert_ne!(instrument_of(&f, old.record), instrument_of(&f, new.record));
    assert_eq!(f.book.instruments().unwrap().len(), 2);
}

#[test]
fn a_routing_reference_never_joins_two_instruments_and_never_blocks_one() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    // Charbone Hydrogen and then Charbone Corp. are both asked of Yahoo as `CH.V`
    let first = json!({"refs": [["broker-security:wealthsimple", "sec-a"], ["yahoo", "CH.V"]], "kind": "security", "currency": "CAD", "symbol": "CH"});
    let second = json!({"refs": [["broker-security:wealthsimple", "sec-b"], ["yahoo", "CH.V"]], "kind": "security", "currency": "CAD", "symbol": "CH"});
    let a = f.store(&m, "r1", &legs(vec![buy("a1", first, "1", "-1", "2026-01-02T15:00:00Z")]));
    let b = f.store(&m, "r2", &legs(vec![buy("a1", second, "1", "-1", "2026-07-02T15:00:00Z")]));
    assert!(f.book.problems_of(b.record).unwrap().is_empty(), "{:?}", f.book.problems_of(b.record));
    let (ia, ib) = (instrument_of(&f, a.record), instrument_of(&f, b.record));
    assert_ne!(ia, ib);
    let yahoo = Reference::new(RefScheme::Yahoo, "CH.V");
    assert_eq!(f.book.instrument_by_ref(&yahoo).unwrap(), None, "a routing reference identifies nothing");
    let mut routed = f.book.instruments_routed_by(&yahoo).unwrap();
    routed.sort();
    let mut both = vec![ia, ib];
    both.sort();
    assert_eq!(routed, both);
    // a record naming an instrument by the routing reference alone identifies nothing
    let routing_only = json!({"refs": [["yahoo", "CH.V"]], "kind": "security", "currency": "CAD", "symbol": "CH"});
    let c = f.store(&m, "r3", &legs(vec![buy("a1", routing_only, "1", "-1", "2026-07-03T17:00:00Z")]));
    assert_eq!(f.book.problems_of(c.record).unwrap()[0].code, "instrument-unidentified");
    assert_eq!(f.book.instruments().unwrap().len(), 2);
}

#[test]
fn a_connection_scoped_symbol_names_one_connections_rows_only() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let other = f.book.add_connection(&Broker::named("wealthsimple"), "Second login", t0()).unwrap();
    let t = AccountType::Known { kind: bagholder_core::account::AccountKind::Cash, registration: bagholder_core::account::Registration::Unregistered, managed: false, joint: false };
    f.book.add_account(other, &[AccountRef::new(Broker::named("wealthsimple"), "b1")], &t, AccountStatus::Open, None, t0()).unwrap();
    let m = Spelled::v(1);
    let scoped = |c: bagholder_core::ConnectionId| json!({"refs": [[format!("connection-symbol:{c}"), "QNC|CAD"]], "kind": "security", "currency": "CAD", "symbol": "QNC"});
    let a1 = f.store(&m, "r1", &legs(vec![buy("a1", scoped(f.connection), "1", "-1", "2026-01-02T15:00:00Z")]));
    let a2 = f.store(&m, "r2", &legs(vec![buy("a1", scoped(f.connection), "2", "-2", "2026-01-03T15:00:00Z")]));
    let text = legs(vec![buy("b1", scoped(other), "3", "-3", "2026-01-03T15:00:00Z")]).to_string();
    let b = f.book.store(&m, &bagholder_book::records::Incoming { connection: Some(other), source_key: "r1", payload: &text, refs: vec![] }, t0()).unwrap();
    assert_eq!(instrument_of(&f, a1.record), instrument_of(&f, a2.record));
    assert_ne!(instrument_of(&f, a1.record), instrument_of(&f, b.record));
}

#[test]
fn references_naming_two_instruments_merge_nothing() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    f.store(&m, "r1", &legs(vec![buy("a1", share("CA0000000001", "AAA"), "1", "-1", "2026-01-02T15:00:00Z")]));
    f.store(&m, "r2", &legs(vec![buy("a1", share("CA0000000002", "BBB"), "1", "-1", "2026-01-02T15:00:00Z")]));
    let both = json!({"refs": [["isin", "CA0000000001"], ["isin", "CA0000000002"]], "kind": "security", "currency": "CAD", "symbol": "AAA"});
    let r = f.store(&m, "r3", &legs(vec![buy("a1", both, "1", "-1", "2026-01-03T15:00:00Z")]));
    assert!(f.book.transactions_of(r.record).unwrap().is_empty());
    assert_eq!(f.book.problems_of(r.record).unwrap()[0].code, "instrument-conflict");
    assert_eq!(f.book.instruments().unwrap().len(), 2);
    // nothing of the failed record's resolution stayed behind
    for i in f.book.instruments().unwrap() {
        assert_eq!(f.book.instrument_refs(i.id).unwrap().len(), 1);
    }
}

#[test]
fn a_kind_or_currency_that_differs_from_the_instruments_is_a_problem() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    f.store(&m, "r1", &legs(vec![buy("a1", share("CA0000000001", "AAA"), "1", "-1", "2026-01-02T15:00:00Z")]));
    let as_coin = json!({"refs": [["isin", "CA0000000001"]], "kind": "crypto", "currency": "CAD", "symbol": "AAA"});
    let r = f.store(&m, "r2", &legs(vec![buy("a1", as_coin, "1", "-1", "2026-01-02T15:00:00Z")]));
    assert_eq!(f.book.problems_of(r.record).unwrap()[0].code, "instrument-kind-conflict");
    let in_usd = json!({"refs": [["isin", "CA0000000001"]], "kind": "security", "currency": "USD", "symbol": "AAA"});
    let r = f.store(&m, "r3", &legs(vec![buy("a1", in_usd, "1", "-1", "2026-01-02T15:00:00Z")]));
    assert_eq!(f.book.problems_of(r.record).unwrap()[0].code, "instrument-currency-conflict");
}

#[test]
fn a_ticker_change_keeps_the_id_and_dates_each_name() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let named = |sym: &str, seen: &str| json!({"refs": [["isin", "CA0000000001"]], "kind": "security", "currency": "CAD", "symbol": sym, "seen": seen});
    let spans = |f: &Fixture, id| -> Vec<(String, String, String)> {
        f.book.names(id).unwrap().iter().map(|n| (n.symbol.clone(), n.first_seen.to_string(), n.last_seen.to_string())).collect()
    };
    f.store(&m, "r1", &legs(vec![buy("a1", named("OLD", "2026-01-02"), "1", "-1", "2026-01-02T15:00:00Z")]));
    f.store(&m, "r2", &legs(vec![buy("a1", named("OLD", "2026-02-10"), "1", "-1", "2026-02-10T15:00:00Z")]));
    let r = f.store(&m, "r3", &legs(vec![sell("a1", named("NEW", "2026-03-01"), "-2", "3", "2026-03-01T15:00:00Z")]));
    let id = instrument_of(&f, r.record);
    assert_eq!(spans(&f, id), vec![("OLD".into(), "2026-01-02".into(), "2026-02-10".into()), ("NEW".into(), "2026-03-01".into(), "2026-03-01".into())]);
    // a ticker that changes back is a third name, not one name over the other
    f.store(&m, "r4", &legs(vec![buy("a1", named("OLD", "2026-04-01"), "1", "-1", "2026-04-01T15:00:00Z")]));
    assert_eq!(spans(&f, id).iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), vec!["OLD", "NEW", "OLD"]);
    // a record revised to another day takes its sighting with it
    f.store(&m, "r2", &legs(vec![buy("a1", named("OLD", "2026-01-20"), "1", "-1", "2026-01-20T15:00:00Z")]));
    assert_eq!(spans(&f, id)[0], ("OLD".into(), "2026-01-02".into(), "2026-01-20".into()));
    // a removed record takes its name with it
    let r4 = f.book.record_by_key(Some(f.connection), &bagholder_core::SourceName::named("test-broker"), "r4").unwrap().unwrap();
    f.book.mark_removed(r4, t0()).unwrap();
    assert_eq!(spans(&f, id).len(), 2);
    // a move to another venue under the same symbol is another name
    let moved = json!({"refs": [["isin", "CA0000000001"]], "kind": "security", "currency": "CAD", "symbol": "NEW", "mic": "XTSE", "seen": "2026-05-01"});
    f.store(&m, "r5", &legs(vec![buy("a1", moved, "1", "-1", "2026-05-01T15:00:00Z")]));
    let names = f.book.names(id).unwrap();
    assert_eq!((names.len(), names[2].venue_mic.as_deref()), (3, Some("XTSE")));
}

#[test]
fn two_listings_sit_under_one_issuer() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let tsx = f.store(&m, "r1", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z")]));
    let us = json!({"refs": [["cusip", "000000002"]], "kind": "security", "currency": "USD", "symbol": "QNCCF", "currency_": "USD"});
    let otc = f.store(&m, "r2", &legs(vec![json!({"leg": "trade", "account": "a1", "kind": "buy", "instrument": us, "quantity": "1", "cash": "-1", "currency": "USD", "at": "2026-01-02T15:00:00Z", "date": "2026-01-02"})]));
    let issuer = f.book.add_issuer("Quantum eMotion Corp", t0()).unwrap();
    let (a, b) = (instrument_of(&f, tsx.record), instrument_of(&f, otc.record));
    f.book.attach_issuer(a, issuer).unwrap();
    f.book.attach_issuer(b, issuer).unwrap();
    assert_eq!(f.book.listings(issuer).unwrap(), vec![a, b]);
    assert_eq!(f.book.issuer(issuer).unwrap().name, "Quantum eMotion Corp");
    // each listing keeps its own currency
    assert_eq!(f.book.instrument(b).unwrap().currency.as_str(), "USD");
    let other = f.book.add_issuer("Someone else", t0()).unwrap();
    assert!(f.book.attach_issuer(a, other).is_err(), "a listing is never moved to another issuer silently");
}

#[test]
fn an_options_terms_and_underlying_are_recorded_once() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let m = Spelled::v(1);
    let call = |mult: Option<&str>| {
        let mut o = json!({"underlying": share("US0000000001", "LUNR"), "expiry": "2027-01-15", "strike": "12.5", "right": "call"});
        if let Some(x) = mult {
            o["multiplier"] = json!(x);
        }
        json!({"refs": [["occ", "LUNR  270115C00012500"]], "kind": "option", "currency": "USD", "symbol": "LUNR 15JAN27 12.50 CALL", "option": o})
    };
    let r = f.store(&m, "r1", &legs(vec![json!({"leg": "trade", "account": "a1", "kind": "buy", "effect": "open", "instrument": call(None), "quantity": "2", "cash": "-300", "currency": "USD", "at": "2026-01-02T15:00:00Z", "date": "2026-01-02"})]));
    let id = instrument_of(&f, r.record);
    let terms = f.book.option_terms(id).unwrap().unwrap();
    assert_eq!((terms.strike, terms.multiplier), (d("12.5"), None));
    assert_eq!(f.book.instrument(terms.underlying).unwrap().kind, InstrumentKind::Security);
    // a source that states the multiplier fills it in
    f.store(&m, "r2", &legs(vec![json!({"leg": "trade", "account": "a1", "kind": "sell", "effect": "close", "instrument": call(Some("100")), "quantity": "-2", "cash": "400", "currency": "USD", "at": "2026-01-03T15:00:00Z", "date": "2026-01-03"})]));
    assert_eq!(f.book.option_terms(id).unwrap().unwrap().multiplier, Some(d("100")));
    // one that states another is a conflict, not an overwrite
    let r3 = f.store(&m, "r3", &legs(vec![json!({"leg": "trade", "account": "a1", "kind": "buy", "effect": "open", "instrument": call(Some("150")), "quantity": "1", "cash": "-1", "currency": "USD", "at": "2026-01-04T15:00:00Z", "date": "2026-01-04"})]));
    assert_eq!(f.book.problems_of(r3.record).unwrap()[0].code, "option-terms-conflict");
    assert_eq!(f.book.option_terms(id).unwrap().unwrap().multiplier, Some(d("100")));
}

#[test]
fn a_brokers_linked_ids_are_one_account() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-cad", "tfsa-usd"]);
    let ws = Broker::named("wealthsimple");
    assert_eq!(f.book.account_by_ref(&AccountRef::new(ws.clone(), "tfsa-cad")).unwrap(), Some(a));
    assert_eq!(f.book.account_by_ref(&AccountRef::new(ws.clone(), "tfsa-usd")).unwrap(), Some(a));
    assert_eq!(f.book.account_refs(a).unwrap().len(), 2);
    // a CAD and a USD row in it land in the one account
    let m = Spelled::v(1);
    let usd = json!({"refs": [["isin", "US0000000001"]], "kind": "security", "currency": "USD", "symbol": "LUNR"});
    let r1 = f.store(&m, "r1", &legs(vec![buy("tfsa-cad", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z")]));
    let r2 = f.store(&m, "r2", &legs(vec![json!({"leg": "trade", "account": "tfsa-usd", "kind": "buy", "instrument": usd, "quantity": "1", "cash": "-1", "currency": "USD", "at": "2026-01-02T15:00:00Z", "date": "2026-01-02"})]));
    assert_eq!(f.book.transactions_of(r1.record).unwrap()[0].account, a);
    assert_eq!(f.book.transactions_of(r2.record).unwrap()[0].account, a);
    // an id another account holds is refused, never shared
    let t = AccountType::Unrecognised("SOMETHING_NEW".into());
    assert!(f.book.add_account(f.connection, &[AccountRef::new(ws, "tfsa-usd")], &t, AccountStatus::Open, None, t0()).is_err());
}

#[test]
fn an_unrecognised_account_type_is_kept_in_the_brokers_words() {
    let f = Fixture::new();
    let t = AccountType::Unrecognised("SELF_DIRECTED_SOMETHING_NEW".into());
    let a = f.book.add_account(f.connection, &[AccountRef::new(Broker::named("wealthsimple"), "x")], &t, AccountStatus::Closed, None, t0()).unwrap();
    let got = f.book.account(a).unwrap();
    assert_eq!(got.account_type, t);
    assert_eq!(got.status, AccountStatus::Closed);
}

#[test]
fn a_row_naming_an_account_the_book_does_not_have_books_nothing() {
    let f = Fixture::new();
    let m = Spelled::v(1);
    let r = f.store(&m, "r1", &legs(vec![buy("nobody", share("CA0000000001", "QNC"), "1", "-1", "2026-01-02T15:00:00Z")]));
    assert!(f.book.transactions_of(r.record).unwrap().is_empty());
    assert_eq!(f.book.problems_of(r.record).unwrap()[0].code, "account-unknown");
    // and made no instrument on the way
    assert!(f.book.instruments().unwrap().is_empty());
}
