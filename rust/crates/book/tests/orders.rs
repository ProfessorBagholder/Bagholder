//! Orders and brackets in the book (`docs/plans/stage-4-execution.md`): written once,
//! every change an event in their log with who asked, their state the log's fold,
//! found by state however many there are.

use bagholder_book::orders::{self, BracketPlace, OrderRequest};
use bagholder_book::{Book, BookError};
use bagholder_core::bracket::{BracketEvent, ExitRole, Phase, StopLeg, Trail};
use bagholder_core::order::{Applied, Asker, BrokerStatus, OrderEvent, OrderKind, OrderRole, OrderState, Reading, Side, TimeInForce};
use bagholder_core::{Currency, Dec};
use serde_json::json;

fn t(s: &str) -> jiff::Timestamp {
    s.parse().unwrap()
}

fn d(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn book() -> (tempfile::TempDir, Book) {
    let dir = tempfile::tempdir().unwrap();
    let (book, _) = Book::open_in(dir.path(), "test", t("2026-09-28T12:00:00Z")).unwrap();
    (dir, book)
}

fn request(id: &str, bracket: Option<(&str, OrderRole)>) -> OrderRequest {
    OrderRequest {
        id: id.into(),
        broker: "wealthsimple".into(),
        broker_account: "acct-1".into(),
        broker_security: "sec-1".into(),
        symbol: "SHOP".into(),
        currency: Currency::parse("USD").unwrap(),
        side: Side::Buy,
        kind: OrderKind::Limit,
        quantity: d("10"),
        limit_price: Some(d("101.25")),
        stop_price: None,
        time_in_force: TimeInForce::Day,
        bracket: bracket.map(|(b, r)| (b.to_string(), r)),
        request: json!({"externalId": id, "limitPrice": 101.25}),
    }
}

#[test]
fn an_order_is_written_once_and_its_state_is_the_fold_of_its_log() {
    let (_d, b) = book();
    let at = t("2026-09-28T14:00:00Z");
    b.write_order(&request("order-1", None), false, &Asker::Person, at).unwrap();
    assert!(b.write_order(&request("order-1", None), false, &Asker::Person, at).is_err(), "an id is written once");
    let o = b.order("order-1").unwrap().unwrap();
    assert_eq!((o.fold.state, o.request, o.created_at), (OrderState::Sending, request("order-1", None), at));
    assert_eq!(b.order_event("order-1", &Asker::Person, at, &OrderEvent::Accepted { broker_id: "o-9".into() }).unwrap(), Ok(Applied::Moved { from: OrderState::Sending, to: OrderState::Pending }));
    let read = Reading { status: BrokerStatus::Open, filled: d("4"), average: Some(d("101.2")), price: Some(d("101.25")), quantity: Some(d("10")), expires_at: Some(t("2026-12-27T14:00:00Z")) };
    b.order_event("order-1", &Asker::Engine, t("2026-09-28T14:00:05Z"), &OrderEvent::Read(read)).unwrap().unwrap();
    let o = b.order("order-1").unwrap().unwrap();
    assert_eq!((o.fold.state, o.fold.filled, o.fold.broker_id.as_deref()), (OrderState::PartlyFilled, d("4"), Some("o-9")));
    assert_eq!((o.stated_price, o.stated_quantity, o.expires_at), (Some(d("101.25")), Some(d("10")), Some(t("2026-12-27T14:00:00Z"))));
    // who asked, kept with each event
    let log = b.order_log("order-1").unwrap();
    assert_eq!(log.iter().map(|l| (l.asker.clone(), l.event.kind())).collect::<Vec<_>>(), vec![(Asker::Person, "written"), (Asker::Person, "accepted"), (Asker::Engine, "read")]);
}

#[test]
fn an_event_that_is_not_a_move_is_kept_with_why_and_changes_nothing() {
    let (_d, b) = book();
    let at = t("2026-09-28T14:00:00Z");
    b.write_order(&request("order-1", None), false, &Asker::Person, at).unwrap();
    let refused = b.order_event("order-1", &Asker::Person, at, &OrderEvent::CancelAsked).unwrap();
    assert!(refused.is_err());
    assert_eq!(b.order("order-1").unwrap().unwrap().fold.state, OrderState::Sending);
    let log = b.order_log("order-1").unwrap();
    assert_eq!(log.len(), 2);
    assert!(log[1].refused.as_deref().unwrap().contains("only a working order can be cancelled"), "{:?}", log[1].refused);
    assert!(matches!(b.order_event("order-2", &Asker::Person, at, &OrderEvent::CancelAsked), Err(BookError::Refused(_))));
}

#[test]
fn every_order_the_broker_may_still_act_on_is_found_however_many_and_however_old() {
    let (_d, b) = book();
    let base = t("2026-01-01T00:00:00Z");
    // 5,000 orders; the oldest and one in the middle still working
    for i in 0..5000 {
        let at = base + jiff::SignedDuration::from_secs(i);
        let id = format!("order-{i:05}");
        b.write_order(&request(&id, None), false, &Asker::Person, at).unwrap();
        b.order_event(&id, &Asker::Person, at, &OrderEvent::Accepted { broker_id: format!("o-{i}") }).unwrap().unwrap();
        if i != 0 && i != 2500 {
            b.order_event(&id, &Asker::Engine, at, &OrderEvent::Read(Reading::of(BrokerStatus::Filled, d("10"), Some(d("101"))))).unwrap().unwrap();
        }
    }
    let live: Vec<String> = b.orders_in_flight().unwrap().into_iter().map(|o| o.request.id).collect();
    assert_eq!(live, vec!["order-00000".to_string(), "order-02500".to_string()]);
    // the page pages the list; the engine never reads a page
    let page = b.orders_before(None, 50).unwrap();
    assert_eq!((page.len(), page[0].request.id.as_str()), (50, "order-04999"));
    let next = b.orders_before(Some(page[49].created_at), 50).unwrap();
    assert_eq!(next[0].request.id, "order-04949");
}

fn every_order_event() -> Vec<OrderEvent> {
    vec![
        OrderEvent::NotSent { why: "no session".into() },
        OrderEvent::Accepted { broker_id: "o-1".into() },
        OrderEvent::Refused { why: "no".into(), code: Some("NOT_ENOUGH_SHARES".into()) },
        OrderEvent::Refused { why: "no".into(), code: None },
        OrderEvent::Unclear { why: "timed out".into() },
        OrderEvent::Read(Reading { status: BrokerStatus::NotFound, filled: Dec::ZERO, average: None, price: None, quantity: None, expires_at: None }),
        OrderEvent::Read(Reading { status: BrokerStatus::Expired, filled: d("3.5"), average: Some(d("12.3456")), price: Some(d("12.5")), quantity: Some(d("7")), expires_at: Some(t("2026-12-01T21:00:00Z")) }),
        OrderEvent::CancelAsked,
        OrderEvent::CancelRefused { why: "too late".into() },
    ]
}

#[test]
fn every_order_event_reads_back_exactly_as_it_was_written() {
    let (_d, b) = book();
    let at = t("2026-09-28T14:00:00Z");
    b.write_order(&request("order-1", None), true, &Asker::Agent("helper".into()), at).unwrap();
    for e in every_order_event() {
        let _ = b.order_event("order-1", &Asker::Engine, at, &e).unwrap();
    }
    let got: Vec<OrderEvent> = b.order_log("order-1").unwrap().into_iter().skip(1).map(|l| l.event).collect();
    assert_eq!(got, every_order_event());
    assert_eq!(b.order_log("order-1").unwrap()[0].asker, Asker::Agent("helper".into()));
}

#[test]
fn an_event_the_book_cannot_read_is_an_error_never_skipped() {
    let (_d, b) = book();
    let at = t("2026-09-28T14:00:00Z");
    b.write_order(&request("order-1", None), false, &Asker::Person, at).unwrap();
    b.conn_for_tests().execute("UPDATE order_events SET body = '{\"dry\": false, \"extra\": 1}' WHERE seq = 0", []).unwrap();
    assert!(matches!(b.order("order-1"), Err(BookError::Corrupt { .. })));
}

fn place() -> BracketPlace {
    BracketPlace { id: "bracket-1".into(), broker: "wealthsimple".into(), broker_account: "acct-1".into(), broker_security: "sec-1".into(), symbol: "SHOP".into(), currency: Currency::parse("USD").unwrap() }
}

fn every_bracket_event() -> Vec<BracketEvent> {
    let trail = Some(StopLeg { level: d("95.5"), trail: Some(Trail::Pct(d("5"))), high: Some(d("100.52")) });
    vec![
        BracketEvent::Armed { quantity: d("10"), high: Some(d("100")), native: true },
        BracketEvent::EntryEnded { why: "entry cancelled".into() },
        BracketEvent::Trailed { level: d("96.01"), high: d("101.06") },
        BracketEvent::Adopted { level: Some(d("94")), target: None, quantity: Some(d("8")) },
        BracketEvent::Adjusted { stop: trail, target: Some(d("120")) },
        BracketEvent::Adjusted { stop: Some(StopLeg { level: d("90"), trail: Some(Trail::Amount(d("2.5"))), high: None }), target: None },
        BracketEvent::Placed { role: ExitRole::Stop, order_id: "order-2".into(), price: Some(d("96.01")), quantity: d("8") },
        BracketEvent::Refused { why: "closed".into(), code: None },
        BracketEvent::CancelAsked { order_id: "order-2".into() },
        BracketEvent::Cleared { filled: d("1") },
        BracketEvent::Moved { to: Phase::Target },
        BracketEvent::Halted { why: "too many orders".into() },
        BracketEvent::SaleAsked { quantity: d("3") },
        BracketEvent::Sold { quantity: d("3") },
        BracketEvent::SaleDropped { why: "refused".into() },
        BracketEvent::Ended { outcome: "stopped".into() },
        BracketEvent::Done,
    ]
}

#[test]
fn a_bracket_is_written_once_its_phase_is_its_logs_fold_and_every_event_reads_back() {
    let (_d, b) = book();
    let at = t("2026-09-28T14:00:00Z");
    let created = BracketEvent::Created { quantity: d("10"), stop: Some(StopLeg { level: d("95"), trail: None, high: None }), target: Some(d("110")) };
    b.write_bracket(&place(), &created, &Asker::Person, at).unwrap();
    assert!(b.write_bracket(&place(), &created, &Asker::Person, at).is_err());
    // the entry and an exit name it
    b.write_order(&request("order-1", Some(("bracket-1", OrderRole::Entry))), false, &Asker::Person, at).unwrap();
    assert!(b.write_order(&request("order-x", Some(("bracket-none", OrderRole::Stop))), false, &Asker::Engine, at).is_err(), "no such bracket");
    assert_eq!(b.orders_of_bracket("bracket-1").unwrap().len(), 1);
    assert_eq!(b.bracket_event("bracket-1", &Asker::Engine, at, &BracketEvent::Armed { quantity: d("10"), high: Some(d("100")), native: true }).unwrap(), Ok(Phase::Guarding));
    let stored = b.bracket("bracket-1").unwrap().unwrap();
    assert_eq!((stored.bracket.phase, stored.bracket.native, stored.place), (Phase::Guarding, true, place()));
    assert_eq!(b.live_brackets().unwrap().len(), 1);
    for e in every_bracket_event() {
        let _ = b.bracket_event("bracket-1", &Asker::Engine, at, &e).unwrap();
    }
    let got: Vec<BracketEvent> = b.bracket_log("bracket-1").unwrap().into_iter().skip(2).map(|l| l.event).collect();
    assert_eq!(got, every_bracket_event());
    let folded = b.bracket("bracket-1").unwrap().unwrap().bracket;
    let phase: String = b.conn_for_tests().query_row("SELECT phase FROM brackets WHERE id = 'bracket-1'", [], |r| r.get(0)).unwrap();
    assert_eq!(phase, folded.phase.as_str(), "the kept phase is the log's fold");
}

#[test]
fn the_words_the_tables_allow_are_exactly_the_states_the_machines_have() {
    let sql = include_str!("../migrations/013-orders-and-brackets.sql");
    let list = |column: &str| -> Vec<String> {
        let at = sql.find(&format!("CHECK ({column} IN (")).unwrap_or_else(|| panic!("no check on {column}"));
        let rest = &sql[at..];
        let inner = &rest[rest.find("IN (").unwrap() + 4..rest.find("))").unwrap()];
        inner.split(',').map(|w| w.trim().trim_matches('\'').to_string()).collect()
    };
    assert_eq!(list("state"), orders::order_state_words());
    assert_eq!(list("phase"), orders::bracket_phase_words());
    assert_eq!(list("side"), Side::ALL.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    assert_eq!(list("order_type"), OrderKind::ALL.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    assert_eq!(list("time_in_force"), TimeInForce::ALL.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    assert_eq!(list("role"), OrderRole::ALL.iter().map(|s| s.as_str()).collect::<Vec<_>>());
}
