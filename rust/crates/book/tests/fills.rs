//! A fill of Bagholder's own order booked from its read-back (`docs/plans/stage-4-execution.md`,
//! "Fills"): exactly what filled, at the broker's average, once however often it is
//! read, and giving way to the broker's own row for the order when the pull brings it.

use bagholder_book::mapping::{Draft, MapContext, Mapped, Mapping};
use bagholder_book::orders::OrderRequest;
use bagholder_book::records::Incoming;
use bagholder_book::Book;
use bagholder_core::account::{AccountKind, AccountRef, AccountStatus, AccountType, Registration};
use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
use bagholder_core::order::{Asker, BrokerStatus, OrderEvent, OrderKind, Reading, Side, TimeInForce};
use bagholder_core::record::RecordState;
use bagholder_core::transaction::Kind;
use bagholder_core::{Broker, Currency, Dec, Leg, Money, SourceName};

fn t(s: &str) -> jiff::Timestamp {
    s.parse().unwrap()
}

fn d(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

/// A book holding one Wealthsimple account.
fn book() -> (tempfile::TempDir, Book) {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-28T12:00:00Z");
    let (book, _) = Book::open_in(dir.path(), "test", at).unwrap();
    let ws = Broker::named("wealthsimple");
    let conn = book.add_connection(&ws, "Wealthsimple", at).unwrap();
    book.add_account(conn, &[AccountRef::new(ws, "acct-1")], &AccountType::Known { kind: AccountKind::Margin, registration: Registration::Unregistered, managed: false, joint: false }, AccountStatus::Open, Some("Trading"), at).unwrap();
    (dir, book)
}

fn order() -> OrderRequest {
    OrderRequest {
        id: "order-1".into(),
        broker: "wealthsimple".into(),
        broker_account: "acct-1".into(),
        broker_security: "sec-1".into(),
        symbol: "SHOP".into(),
        currency: Currency::parse("USD").unwrap(),
        side: Side::Buy,
        kind: OrderKind::Limit,
        quantity: d("10"),
        limit_price: Some(d("102")),
        stop_price: None,
        time_in_force: TimeInForce::Day,
        bracket: None,
        request: serde_json::json!({}),
    }
}

fn read(status: BrokerStatus, filled: &str, avg: &str) -> OrderEvent {
    OrderEvent::Read(Reading::of(status, d(filled), Some(d(avg))))
}

/// The units the book holds and what they cost, from its live transactions.
fn held(b: &Book) -> (Dec, Dec) {
    let mut units = Dec::ZERO;
    let mut cost = Dec::ZERO;
    for tx in b.transactions().unwrap() {
        units = units.checked_add(tx.quantity.unwrap_or(Dec::ZERO)).unwrap();
        if let Some(c) = tx.cash {
            cost = cost.checked_sub(c.amount).unwrap();
        }
    }
    (units, cost)
}

/// The broker's own row for the order, as its pull would store it.
struct BrokerRow;

impl Mapping for BrokerRow {
    fn source(&self) -> SourceName {
        SourceName::named("wealthsimple")
    }
    fn version(&self) -> u32 {
        1
    }
    fn map(&self, _ctx: &MapContext, _payload: &str) -> Mapped {
        let usd = Currency::parse("USD").unwrap();
        let day = jiff::civil::date(2026, 9, 28);
        Mapped {
            legs: vec![Draft {
                leg: Leg::named("trade"),
                account: AccountRef::new(Broker::named("wealthsimple"), "acct-1"),
                occurred_at: None,
                trade_date: day,
                settle_date: None,
                kind: Kind::Buy,
                effect: None,
                instrument: Some(bagholder_book::mapping::InstrumentDraft { refs: vec![Reference::new(RefScheme::BrokerSecurity(Broker::named("wealthsimple")), "sec-1")], kind: InstrumentKind::Security, currency: usd, name: None, option: None }),
                quantity: Some(d("10")),
                price: None,
                cash: Some(Money::new(d("-1012.5"), usd)),
                fee: None,
                fx_rate: None,
                paid_on: None,
                value: None,
            }],
            problems: vec![],
            adjustments: vec![],
        }
    }
}

#[test]
fn what_filled_is_booked_once_at_the_brokers_average_and_gives_way_to_the_brokers_row() {
    let (_d, b) = book();
    let at = t("2026-09-28T14:00:00Z");
    b.write_order(&order(), false, &Asker::Person, at).unwrap();
    b.order_event("order-1", &Asker::Person, at, &OrderEvent::Accepted { broker_id: "ws-1".into() }).unwrap().unwrap();
    for e in [read(BrokerStatus::Open, "3", "100"), read(BrokerStatus::Open, "3", "100"), read(BrokerStatus::Open, "7", "101"), read(BrokerStatus::Filled, "10", "101.25")] {
        let _ = b.order_event("order-1", &Asker::Engine, at, &e).unwrap();
    }
    let fills = b.live_records(&bagholder_book::fills::source()).unwrap();
    assert_eq!(fills.len(), 3, "one for each rise, none for the same reading twice");
    assert_eq!(held(&b), (d("10"), d("1012.5")), "exactly what filled, at the broker's average times what filled");
    assert!(b.problems().unwrap().is_empty(), "{:?}", b.problems().unwrap());

    // the pull brings Wealthsimple's own row for the order
    let conn = b.connections().unwrap()[0].id;
    b.store(&BrokerRow, &Incoming { connection: Some(conn), source_key: "ws-1", payload: "{}", refs: vec![] }, at).unwrap();
    assert_eq!(b.fills_give_way(at).unwrap(), 1);
    for f in &fills {
        assert_eq!(b.record(*f).unwrap().state, RecordState::Superseded);
    }
    assert_eq!(held(&b), (d("10"), d("1012.5")), "counted once: the broker's row alone");
    assert_eq!(b.fills_give_way(at).unwrap(), 0, "once");
    // a later reading of the same order books nothing beside the broker's row
    let _ = b.order_event("order-1", &Asker::Engine, at, &read(BrokerStatus::Filled, "10", "101.25")).unwrap();
    assert_eq!(held(&b), (d("10"), d("1012.5")));
}

#[test]
fn a_filled_quantity_that_falls_books_nothing_negative() {
    let (_d, b) = book();
    let at = t("2026-09-28T14:00:00Z");
    b.write_order(&order(), false, &Asker::Person, at).unwrap();
    b.order_event("order-1", &Asker::Person, at, &OrderEvent::Accepted { broker_id: "ws-1".into() }).unwrap().unwrap();
    b.order_event("order-1", &Asker::Engine, at, &read(BrokerStatus::Open, "6", "100")).unwrap().unwrap();
    assert!(b.order_event("order-1", &Asker::Engine, at, &read(BrokerStatus::Open, "4", "100")).unwrap().is_err());
    assert_eq!(held(&b), (d("6"), d("600")));
}

#[test]
fn a_dry_order_books_no_fill() {
    let (_d, b) = book();
    let at = t("2026-09-28T14:00:00Z");
    b.write_order(&order(), true, &Asker::Person, at).unwrap();
    assert!(b.order_event("order-1", &Asker::Engine, at, &read(BrokerStatus::Filled, "10", "100")).unwrap().is_err(), "a dry order was never sent");
    assert_eq!(held(&b), (Dec::ZERO, Dec::ZERO));
}
