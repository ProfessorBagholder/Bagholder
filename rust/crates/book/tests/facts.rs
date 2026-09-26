//! Migration 2 and the facts a figure is computed from
//! (`docs/plans/stage-2-engine.md`, "Facts and adjustments in the book"): each
//! kept as its source stated it, the first rate for a day standing, the span of
//! every read kept, a fund's record read whole, and adjustments derived from
//! records and following the transaction they explain through a supersede.

mod common;

use bagholder_book::facts::{DeclaredRow, RateSeries};
use bagholder_book::schema::{MIGRATIONS, SCHEMA};
use bagholder_book::Book;
use bagholder_core::journal::{Anchor, Opening};
use bagholder_core::{Currency, Leg, Money, SourceName, TransactionId};
use bagholder_sqlite::migrate::{self, Schema};
use common::*;
use serde_json::json;

fn day(s: &str) -> jiff::civil::Date {
    s.parse().unwrap()
}

fn boc() -> SourceName {
    SourceName::named("bank-of-canada")
}

#[test]
fn migration_two_anchors_every_trade_on_its_transaction_and_instrument() {
    // a book at version 1 with a trade anchored on a purchase
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.db");
    let v1: &'static Schema = Box::leak(Box::new(Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..1] }));
    let (conn, _) = migrate::open(v1, &path, "test", t0()).unwrap();
    conn.execute_batch(
        "INSERT INTO broker_connections(id, broker, label, created_at) VALUES ('01900000-0000-7000-8000-000000000001', 'wealthsimple', 'W', 'x');
         INSERT INTO accounts(id, connection_id, kind, registration, managed, joint, status, created_at) VALUES ('01900000-0000-7000-8000-000000000002', '01900000-0000-7000-8000-000000000001', 'cash', 'none', 0, 0, 'open', 'x');
         INSERT INTO instruments(id, kind, currency, created_at) VALUES ('01900000-0000-7000-8000-000000000003', 'security', 'CAD', 'x');
         INSERT INTO source_records(id, connection_id, source, source_key, state, derived_version, first_received_at, state_changed_at) VALUES ('01900000-0000-7000-8000-000000000004', '01900000-0000-7000-8000-000000000001', 's', 'k', 'live', 1, 'x', 'x');
         INSERT INTO transactions(record_id, leg, mapping_version, account_id, trade_date, kind, instrument_id, quantity, cash, cash_currency) VALUES ('01900000-0000-7000-8000-000000000004', 'trade', 1, '01900000-0000-7000-8000-000000000002', '2026-01-02', 'buy', '01900000-0000-7000-8000-000000000003', '10', '-100', 'CAD');
         INSERT INTO trades(id, anchor_record, anchor_leg, created_at) VALUES ('01900000-0000-7000-8000-000000000005', '01900000-0000-7000-8000-000000000004', 'trade', 'x');
         INSERT INTO trades(id, orphaned_reason, legacy_key, created_at) VALUES ('01900000-0000-7000-8000-000000000006', 'gone', 'rt:old', 'x');",
    )
    .unwrap();
    drop(conn);
    let (book, done) = Book::open(&path, "test", t0()).unwrap();
    assert_eq!((done.from, done.to), (1, MIGRATIONS.len() as u32));
    let trades = book.trades().unwrap();
    assert_eq!(
        trades[0].anchor,
        Anchor::Opening(Opening {
            transaction: TransactionId::parse("01900000-0000-7000-8000-000000000004/trade").unwrap(),
            instrument: bagholder_core::InstrumentId::parse("01900000-0000-7000-8000-000000000003").unwrap(),
        })
    );
    assert_eq!(trades[1].anchor, Anchor::Orphaned("gone".into()));
}

#[test]
fn an_anchor_is_a_transaction_and_the_instrument_it_opened() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let r = f.store(&Spelled::v(1), "buy", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "10", "-100", "2026-01-02T15:00:00Z")]));
    let opening = f.opens(r.record);
    let t = f.book.open_trade(&opening, None, t0()).unwrap();
    assert_eq!(f.book.open_trade(&opening, None, t0()).unwrap(), t, "the same opening is the same trade");
    // the transaction opens nothing of another instrument
    let other = f.store(&Spelled::v(1), "other", &legs(vec![buy("a1", share("CA0000000002", "XYZ"), "1", "-1", "2026-01-02T16:00:00Z")]));
    let wrong = Opening { transaction: opening.transaction.clone(), instrument: f.opens(other.record).instrument };
    assert!(f.book.open_trade(&wrong, None, t0()).is_err());
}

#[test]
fn the_first_rate_for_a_day_stands_and_a_later_different_one_is_kept_beside_it() {
    let f = Fixture::new();
    let usd = Currency::USD;
    let first = f.book.store_rates(usd, &[(day("2026-01-02"), d("1.40")), (day("2026-01-05"), d("1.41"))], (day("2025-12-29"), day("2026-01-05")), &boc(), t0()).unwrap();
    assert!(first.is_empty());
    // the same again writes nothing new; a different value for a stored day is a conflict
    let again = f.book.store_rates(usd, &[(day("2026-01-02"), d("1.40")), (day("2026-01-05"), d("1.42"))], (day("2026-01-02"), day("2026-01-05")), &boc(), at("2026-01-06T12:00:00Z")).unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!((again[0].stands, again[0].later), (d("1.41"), d("1.42")));
    let rates = f.book.rates().unwrap();
    assert_eq!(rates[&usd][&day("2026-01-05")], d("1.41"), "the first stands");
    assert_eq!(f.book.rate_conflicts().unwrap().len(), 1);
    assert_eq!(f.book.rate_reads().unwrap()[&usd], vec![(day("2025-12-29"), day("2026-01-05"), t0()), (day("2026-01-02"), day("2026-01-05"), at("2026-01-06T12:00:00Z"))]);
    // an observation outside the read's span is refused, and nothing of the read is written
    assert!(f.book.store_rates(usd, &[(day("2026-02-02"), d("1.5"))], (day("2026-01-01"), day("2026-01-31")), &boc(), t0()).is_err());
    assert_eq!(f.book.rate_reads().unwrap()[&usd].len(), 2);
}

#[test]
fn the_banks_series_and_holidays_are_kept() {
    let f = Fixture::new();
    let series = |c: Currency, first: &str, last: &str| RateSeries { currency: c, source: boc(), first_day: day(first), last_day: day(last), ended: false };
    f.book.store_rate_series(&[series(Currency::USD, "2017-01-03", "2026-09-22"), series(Currency::parse("EUR").unwrap(), "2017-01-03", "2026-09-22")], t0()).unwrap();
    // a later statement of the same series moves its last day on
    f.book.store_rate_series(&[series(Currency::USD, "2017-01-03", "2026-09-23")], t0()).unwrap();
    // another source's series of the same currency stands beside it
    let noon = RateSeries { currency: Currency::USD, source: SourceName::named("bank-of-canada-noon"), first_day: day("2007-05-01"), last_day: day("2017-04-28"), ended: true };
    f.book.store_rate_series(&[noon.clone()], t0()).unwrap();
    let held = f.book.rate_series().unwrap();
    assert_eq!(held.len(), 3);
    assert!(held.contains(&noon), "an ended series reads back as ended");
    assert_eq!(held.iter().filter(|s| s.currency == Currency::USD).map(|s| (s.first_day, s.last_day)).collect::<Vec<_>>(), vec![(day("2007-05-01"), day("2017-04-28")), (day("2017-01-03"), day("2026-09-23"))]);
    assert!(f.book.store_rate_series(&[series(Currency::USD, "2026-01-02", "2026-01-01")], t0()).is_err(), "a series holding no day");
    f.book.store_bank_holidays(&[(day("2026-02-16"), "Family Day".into())], &boc(), t0()).unwrap();
    assert!(f.book.bank_holidays().unwrap().contains(&day("2026-02-16")));
}

#[test]
fn a_funds_record_is_read_whole_and_a_withdrawn_distribution_is_gone_from_the_newest_read() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let r = f.store(&Spelled::v(1), "buy", &legs(vec![buy("a1", share("CA0000000001", "FUND"), "10", "-100", "2026-01-02T15:00:00Z")]));
    let fund = f.opens(r.record).instrument;
    let row = |ex: &str, amount: &str, reinvested: Option<&str>| DeclaredRow { form: bagholder_core::distribution::Form::Stated, ex_date: day(ex), record_date: None, pay_date: None, amount: Money::new(d(amount), Currency::CAD), reinvested: reinvested.map(d) };
    let tmx = SourceName::named("tmx");
    f.book.store_declared(fund, &[row("2026-02-13", "0.10", None), row("2026-03-13", "0.15", Some("0.12"))], &tmx, at("2026-03-01T00:00:00Z")).unwrap();
    let first = &f.book.declared().unwrap()[&fund];
    assert_eq!(first.items[1].reinvested, Some(d("0.12")), "a reinvested part kept as stated");
    f.book.store_declared(fund, &[row("2026-02-13", "0.10", None)], &tmx, at("2026-03-02T00:00:00Z")).unwrap();
    let read = &f.book.declared().unwrap()[&fund];
    assert_eq!(read.items.len(), 1, "the second was withdrawn");
    assert_eq!(read.read_at, at("2026-03-02T00:00:00Z"));
    f.book.store_frequency(fund, 12, &tmx, None, at("2026-03-01T00:00:00Z")).unwrap();
    f.book.store_frequency(fund, 4, &tmx, Some(day("2026-03-02")), at("2026-03-02T00:00:00Z")).unwrap();
    assert_eq!(f.book.frequencies().unwrap()[&fund].per_year, 4, "the newest statement");
    assert!(f.book.store_frequency(fund, 0, &tmx, None, t0()).is_err());
}

#[test]
fn an_identical_read_of_a_funds_record_records_only_its_time() {
    let f = Fixture::new();
    f.account(&["a1"]);
    let r = f.store(&Spelled::v(1), "buy", &legs(vec![buy("a1", share("CA0000000001", "FUND"), "10", "-100", "2026-01-02T15:00:00Z")]));
    let fund = f.opens(r.record).instrument;
    let rows = vec![
        DeclaredRow { form: bagholder_core::distribution::Form::Stated, ex_date: day("2026-02-13"), record_date: Some(day("2026-02-13")), pay_date: Some(day("2026-02-20")), amount: Money::new(d("0.10"), Currency::CAD), reinvested: None },
        DeclaredRow { form: bagholder_core::distribution::Form::Stated, ex_date: day("2026-03-13"), record_date: Some(day("2026-03-13")), pay_date: Some(day("2026-03-20")), amount: Money::new(d("0.15"), Currency::CAD), reinvested: Some(d("0.12")) },
    ];
    let tmx = SourceName::named("tmx");
    f.book.store_declared(fund, &rows, &tmx, at("2026-03-01T00:00:00Z")).unwrap();
    f.book.store_declared(fund, &rows, &tmx, at("2026-03-02T00:00:00Z")).unwrap();
    let read = &f.book.declared().unwrap()[&fund];
    assert_eq!(read.read_at, at("2026-03-02T00:00:00Z"), "the newer read's time");
    assert_eq!(read.items, rows, "the record as stated");
    let conn = rusqlite::Connection::open(f.dir.path().join(bagholder_book::BOOK_FILE)).unwrap();
    let stored: Vec<(String, i64)> = conn
        .prepare("SELECT r.read_at, (SELECT COUNT(*) FROM declared_distributions x WHERE x.read_id = r.id) FROM declared_reads r WHERE r.instrument_id = ?1")
        .unwrap()
        .query_map([fund.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(stored, vec![("2026-03-02T00:00:00Z".to_string(), 2)], "one stored read, its time the newer one");
}

/// An event row, and a record explaining it.
fn event_and_its_adjustment(f: &Fixture) -> (bagholder_core::RecordId, TransactionId) {
    f.account(&["a1"]);
    f.store(&Spelled::v(1), "buy", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "100", "-1000", "2026-01-02T15:00:00Z")]));
    let event = f.store(
        &Spelled::v(1),
        "split",
        &legs(vec![json!({"leg": "trade", "account": "a1", "kind": "corporate-event", "instrument": share("CA0000000001", "QNC"), "at": "2026-03-02T11:00:00Z", "date": "2026-03-02"})]),
    );
    (event.record, TransactionId::new(event.record, Leg::named("trade")))
}

#[test]
fn an_adjustment_is_derived_from_its_record_and_a_sourced_one_supersedes_the_persons() {
    let f = Fixture::new();
    let (_, applies) = event_and_its_adjustment(&f);
    let person = Spelled { source: "person", version: 1 };
    let mine = f.store(
        &person,
        "my-split",
        &json!({"legs": [], "adjustments": [{"applies_to": applies.to_string(), "legs": [{"from": [["isin", "CA0000000001"]], "to": [["isin", "CA0000000001"]], "units_per_unit": "2"}]}]}),
    );
    let all = f.book.adjustments().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].applies_to, applies);
    assert_eq!(all[0].legs[0].units_per_unit, Some(d("2")));
    assert_eq!(all[0].source, SourceName::named("person"));
    // the issuer's notice replaces the person's entry
    let issuer = Spelled { source: "issuer", version: 1 };
    let text = json!({"legs": [], "adjustments": [{"applies_to": applies.to_string(), "legs": [{"from": [["isin", "CA0000000001"]], "to": [["isin", "CA0000000001"]], "units_per_unit": "3"}]}]}).to_string();
    f.book.store_superseding(&issuer, &f.incoming("notice", &text), &[mine.record], "the issuer's notice", t0()).unwrap();
    let all = f.book.adjustments().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!((all[0].source.clone(), all[0].legs[0].units_per_unit), (SourceName::named("issuer"), Some(d("3"))));
}

#[test]
fn an_adjustment_naming_an_instrument_no_reference_finds_is_a_problem() {
    let f = Fixture::new();
    let (_, applies) = event_and_its_adjustment(&f);
    let person = Spelled { source: "person", version: 1 };
    let r = f.store(&person, "bad", &json!({"legs": [], "adjustments": [{"applies_to": applies.to_string(), "legs": [{"from": [["isin", "CA9999999999"]], "units_per_unit": "2"}]}]}));
    assert!(f.book.adjustments().unwrap().is_empty());
    assert_eq!(f.book.problems_of(r.record).unwrap()[0].code, "adjustment-instrument-unknown");
}

#[test]
fn a_supersede_moves_an_adjustment_to_the_transaction_that_replaced_the_one_it_explains() {
    let f = Fixture::new();
    let (event_record, applies) = event_and_its_adjustment(&f);
    let person = Spelled { source: "person", version: 1 };
    f.store(&person, "my-split", &json!({"legs": [], "adjustments": [{"applies_to": applies.to_string(), "legs": [{"from": [["isin", "CA0000000001"]], "to": [["isin", "CA0000000001"]], "units_per_unit": "2"}]}]}));
    // the broker's own row replaces the event row
    let broker = Spelled { source: "broker-raw", version: 1 };
    let text = legs(vec![json!({"leg": "trade", "account": "a1", "kind": "corporate-event", "instrument": share("CA0000000001", "QNC"), "at": "2026-03-02T11:00:00Z", "date": "2026-03-02"})]).to_string();
    let raw = f.book.store_superseding(&broker, &f.incoming("raw-event", &text), &[event_record], "the broker's row", t0()).unwrap();
    let all = f.book.adjustments().unwrap();
    assert_eq!(all[0].applies_to, TransactionId::new(raw.record, Leg::named("trade")));
}

#[test]
fn a_spin_offs_second_child_and_an_events_marker_open_trades_the_adjustment_names() {
    let f = Fixture::new();
    f.account(&["a1"]);
    f.store(&Spelled::v(1), "buy", &legs(vec![buy("a1", share("CA0000000001", "PARENT"), "100", "-1000", "2026-01-02T15:00:00Z")]));
    // the event row names the first child; the second child is named only by the adjustment
    let child = |isin: &str, sym: &str, qty: &str| json!({"leg": "trade", "account": "a1", "kind": "corporate-event", "instrument": share(isin, sym), "quantity": qty, "at": "2026-03-02T11:00:00Z", "date": "2026-03-02"});
    let event = f.store(&Spelled::v(1), "spin-c", &legs(vec![child("CA0000000002", "C", "50")]));
    f.store(&Spelled::v(1), "spin-d", &legs(vec![child("CA0000000003", "D", "20")]));
    let applies = TransactionId::new(event.record, Leg::named("trade"));
    f.store(
        &Spelled { source: "issuer", version: 1 },
        "notice",
        &json!({"legs": [], "adjustments": [{"applies_to": applies.to_string(), "legs": [
            {"from": [["isin", "CA0000000001"]], "to": [["isin", "CA0000000002"]], "units_per_unit": "0.5", "cost_share": "0.2"},
            {"from": [["isin", "CA0000000001"]], "to": [["isin", "CA0000000003"]], "units_per_unit": "0.2", "cost_share": "0.1"}
        ]}]}),
    );
    let d_instrument = f.book.instrument_by_ref(&bagholder_core::instrument::Reference::new(bagholder_core::instrument::RefScheme::Isin, "CA0000000003")).unwrap().unwrap();
    f.book.open_trade(&Opening { transaction: applies.clone(), instrument: d_instrument }, None, t0()).unwrap();
    // an instrument no adjustment on the event names is still refused
    let parent = f.book.instrument_by_ref(&bagholder_core::instrument::Reference::new(bagholder_core::instrument::RefScheme::Isin, "CA0000000001")).unwrap().unwrap();
    assert!(f.book.open_trade(&Opening { transaction: applies, instrument: parent }, None, t0()).is_err());
}

fn event_leg(leg: &str) -> serde_json::Value {
    json!({"leg": leg, "account": "a1", "kind": "corporate-event", "instrument": share("CA0000000001", "QNC"), "at": "2026-03-02T11:00:00Z", "date": "2026-03-02"})
}

fn person_split(f: &Fixture, key: &str, applies: &TransactionId) -> bagholder_core::RecordId {
    let person = Spelled { source: "person", version: 1 };
    f.store(&person, key, &json!({"legs": [], "adjustments": [{"applies_to": applies.to_string(), "legs": [{"from": [["isin", "CA0000000001"]], "to": [["isin", "CA0000000001"]], "units_per_unit": "2"}]}]})).record
}

#[test]
fn a_record_read_again_without_the_transaction_an_adjustment_explains_reports_it_on_the_adjustment() {
    let f = Fixture::new();
    let (_, applies) = event_and_its_adjustment(&f);
    let mine = person_split(&f, "my-split", &applies);
    assert!(f.book.problems_of(mine).unwrap().is_empty());
    // the source's record now states its event under another leg
    f.store(&Spelled::v(1), "split", &legs(vec![event_leg("other")]));
    let problems = f.book.problems_of(mine).unwrap();
    assert_eq!(problems.iter().map(|p| p.code.as_str()).collect::<Vec<_>>(), ["adjustment-target-gone"]);
}

#[test]
fn a_removed_record_reports_the_adjustments_that_explained_it() {
    let f = Fixture::new();
    let (event_record, applies) = event_and_its_adjustment(&f);
    let mine = person_split(&f, "my-split", &applies);
    f.book.mark_removed(event_record, t0()).unwrap();
    assert_eq!(f.book.problems_of(mine).unwrap()[0].code, "adjustment-target-gone");
}

#[test]
fn a_supersede_never_moves_two_adjustments_of_different_transactions_onto_one() {
    let f = Fixture::new();
    f.account(&["a1"]);
    f.store(&Spelled::v(1), "buy", &legs(vec![buy("a1", share("CA0000000001", "QNC"), "100", "-1000", "2026-01-02T15:00:00Z")]));
    let event = f.store(&Spelled::v(1), "split", &legs(vec![event_leg("a"), event_leg("b")]));
    let first = person_split(&f, "first", &TransactionId::new(event.record, Leg::named("a")));
    let second = person_split(&f, "second", &TransactionId::new(event.record, Leg::named("b")));
    // the broker's one row replaces both
    let broker = Spelled { source: "broker-raw", version: 1 };
    let text = legs(vec![event_leg("trade")]).to_string();
    let raw = f.book.store_superseding(&broker, &f.incoming("raw-event", &text), &[event.record], "the broker's row", t0()).unwrap();
    let onto = TransactionId::new(raw.record, Leg::named("trade"));
    let moved: Vec<_> = f.book.adjustments().unwrap().into_iter().filter(|a| a.applies_to == onto).collect();
    assert_eq!(moved.len(), 1, "one adjustment explains the new row");
    assert!(f.book.problems_of(first).unwrap().is_empty());
    assert_eq!(f.book.problems_of(second).unwrap()[0].code, "adjustment-target-gone");
}

#[test]
fn migration_three_keeps_each_distribution_as_stated_without_a_kind() {
    // a book at version 2 with a regular distribution and a non-cash one
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.db");
    let v2: &'static Schema = Box::leak(Box::new(Schema { name: SCHEMA.name, application_id: SCHEMA.application_id, migrations: &MIGRATIONS[..2] }));
    let (conn, _) = migrate::open(v2, &path, "test", t0()).unwrap();
    conn.execute_batch(
        "INSERT INTO instruments(id, kind, currency, created_at) VALUES ('01900000-0000-7000-8000-000000000003', 'security', 'CAD', 'x');
         INSERT INTO declared_reads(id, instrument_id, source, read_at) VALUES (1, '01900000-0000-7000-8000-000000000003', 'tmx', '2026-03-01T00:00:00Z');
         INSERT INTO declared_distributions(read_id, ex_date, record_date, pay_date, amount, currency, kind) VALUES (1, '2026-02-13', NULL, '2026-02-20', '0.1', 'CAD', 'regular');
         INSERT INTO declared_distributions(read_id, ex_date, record_date, pay_date, amount, currency, kind) VALUES (1, '2025-12-29', '2025-12-29', '2025-12-31', '0.84031', 'CAD', 'non-cash');
         INSERT INTO fx_series(currency, source, received_at) VALUES ('USD', 'bank-of-canada', 'x');",
    )
    .unwrap();
    drop(conn);
    let (book, done) = Book::open(&path, "test", t0()).unwrap();
    assert_eq!((done.from, done.to), (2, MIGRATIONS.len() as u32));
    let fund = bagholder_core::InstrumentId::parse("01900000-0000-7000-8000-000000000003").unwrap();
    let items = &book.declared().unwrap()[&fund].items;
    assert_eq!((items[0].amount.amount, items[0].reinvested), (d("0"), Some(d("0.84031"))), "a non-cash row paid no cash: its amount is what was reinvested");
    assert_eq!((items[1].amount.amount, items[1].reinvested), (d("0.1"), None));
    assert!(book.rate_series().unwrap().is_empty(), "a series row without its days is read again");
}
