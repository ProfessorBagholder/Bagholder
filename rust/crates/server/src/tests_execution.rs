//! Execution against a fake Wealthsimple that misbehaves (`docs/plans/stage-4-execution.md`,
//! "The misbehaving fake broker"): it keeps its own order book, never the app's
//! rows, and loses answers, refuses, fills in parts and answers late.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use bagholder_book::orders::OrderRequest;
use bagholder_core::order::{Asker, BrokerStatus, OrderKind, OrderRole, OrderState, Reading, Side, TimeInForce};
use bagholder_core::{Currency, Dec};
use jiff::{SignedDuration, Timestamp};
use serde_json::{json, Value};

use crate::app::App;
use crate::orders::gate::{self, Found, Held, OrderBroker, Sent};

pub fn d(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

/// What the fake does with the next request.
#[derive(Clone, Debug, PartialEq)]
pub enum Behaviour {
    Accept,
    Refuse(&'static str, Option<&'static str>),
    /// It takes the order, and the answer is lost on the way back.
    LoseAfter,
    /// The request never reaches it, and the caller hears nothing.
    LoseBefore,
    /// It takes the order and names no id.
    NoId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FakeOrder {
    pub request: Value,
    pub status: BrokerStatus,
    pub filled: Dec,
    pub average: Option<Dec>,
    pub expires_at: Option<Timestamp>,
}

#[derive(Default)]
pub struct FakeState {
    pub orders: BTreeMap<String, FakeOrder>,
    pub creates: Vec<String>,
    pub cancels: Vec<String>,
    pub next: VecDeque<Behaviour>,
    pub reads_fail: bool,
    serial: u32,
}

/// A Wealthsimple that answers from its own book of orders.
#[derive(Default)]
pub struct FakeBroker(pub Mutex<FakeState>);

impl FakeBroker {
    pub fn install(app: &App) -> Arc<FakeBroker> {
        let fake = Arc::new(FakeBroker::default());
        *app.orders.gate.broker.lock().unwrap() = Some(fake.clone());
        fake
    }

    pub fn then(&self, b: Behaviour) {
        self.0.lock().unwrap().next.push_back(b);
    }

    fn behaviour(&self) -> Behaviour {
        self.0.lock().unwrap().next.pop_front().unwrap_or(Behaviour::Accept)
    }

    /// The broker's own move on an order: a fill, an expiry, a cancel it confirms.
    pub fn set(&self, id: &str, status: BrokerStatus, filled: &str) {
        let mut s = self.0.lock().unwrap();
        let o = s.orders.get_mut(id).unwrap_or_else(|| panic!("the fake has no order {id}"));
        o.status = status;
        o.filled = d(filled);
        if o.filled.is_positive() {
            o.average = Some(d("100"));
        }
    }

    pub fn at_broker(&self) -> usize {
        self.0.lock().unwrap().orders.len()
    }
}

impl OrderBroker for FakeBroker {
    fn create(&self, _app: &Arc<App>, request: &Value) -> Sent {
        let id = request["externalId"].as_str().expect("an external id").to_string();
        let b = self.behaviour();
        let mut s = self.0.lock().unwrap();
        s.creates.push(id.clone());
        let taken = matches!(b, Behaviour::Accept | Behaviour::LoseAfter | Behaviour::NoId);
        if taken {
            assert!(!s.orders.contains_key(&id), "the same order sent twice: {id}");
            s.serial += 1;
            s.orders.insert(id, FakeOrder { request: request.clone(), status: BrokerStatus::Open, filled: Dec::ZERO, average: None, expires_at: None });
        }
        match b {
            Behaviour::Accept => Sent::Accepted { broker_id: Some(format!("ws-{}", s.serial)) },
            Behaviour::NoId => Sent::Accepted { broker_id: None },
            Behaviour::Refuse(why, code) => Sent::Refused { why: why.into(), code: code.map(String::from) },
            Behaviour::LoseAfter | Behaviour::LoseBefore => Sent::Unclear { why: "the connection dropped".into() },
        }
    }

    fn cancel(&self, _app: &Arc<App>, external_id: &str) -> Sent {
        let b = self.behaviour();
        let mut s = self.0.lock().unwrap();
        s.cancels.push(external_id.to_string());
        match b {
            Behaviour::Refuse(why, code) => Sent::Refused { why: why.into(), code: code.map(String::from) },
            Behaviour::LoseBefore => Sent::Unclear { why: "the connection dropped".into() },
            // taken: the fake confirms the cancel itself only when a test says so
            _ => Sent::Accepted { broker_id: None },
        }
    }

    fn modify(&self, _app: &Arc<App>, _external_id: &str, _change: &Value) -> Sent {
        Sent::Accepted { broker_id: None }
    }

    fn read(&self, _app: &Arc<App>, external_id: &str) -> Result<Found, String> {
        let s = self.0.lock().unwrap();
        if s.reads_fail {
            return Err("Wealthsimple could not be reached".into());
        }
        Ok(match s.orders.get(external_id) {
            None => Found::None,
            Some(o) => Found::Order(Reading { expires_at: o.expires_at, ..Reading::of(o.status, o.filled, o.average) }),
        })
    }
}

/// An app of its own, on a home of its own, with orders live against a fake.
pub fn fresh() -> (tempfile::TempDir, Arc<App>, Arc<FakeBroker>) {
    crate::tests_common::home(); // offline, dry orders by default
    let home = tempfile::tempdir().unwrap();
    let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
    bagholder_store::schema::init_schema(&app.open().unwrap()).unwrap();
    app.set_figures(crate::figures::Figures::open(home.path(), Timestamp::now()).unwrap());
    let fake = FakeBroker::install(&app);
    (home, app, fake)
}



pub fn order(id: &str, bracket: Option<(&str, OrderRole)>) -> OrderRequest {
    OrderRequest {
        id: id.into(),
        broker: "wealthsimple".into(),
        broker_account: "acct".into(),
        broker_security: "sec".into(),
        symbol: "SHOP".into(),
        currency: Currency::parse("USD").unwrap(),
        side: Side::Buy,
        kind: OrderKind::Limit,
        quantity: d("10"),
        limit_price: Some(d("100")),
        stop_price: None,
        time_in_force: TimeInForce::Day,
        bracket: bracket.map(|(b, r)| (b.to_string(), r)),
        request: json!({ "externalId": id }),
    }
}

fn t0() -> Timestamp {
    "2026-09-28T14:00:00Z".parse().unwrap()
}

#[test]
fn an_order_whose_answer_is_lost_after_the_broker_took_it_ends_working_by_read_back_never_failed() {
    let _g = crate::tests_common::guard();
    let (_h, app, fake) = fresh();
    app.set_orders_live(true);
    let book = app.figures.get().unwrap().book().unwrap();
    fake.then(Behaviour::LoseAfter);
    let got = gate::place(&app, &book, &order("order-1", None), &Asker::Person, t0()).unwrap().unwrap();
    assert_eq!(got.state, OrderState::Unconfirmed);
    let after = gate::read_back(&app, &book, "order-1", t0()).unwrap();
    assert_eq!(after.state, OrderState::Pending);
    assert_eq!(fake.at_broker(), 1, "exactly one order at the broker");
}

#[test]
fn an_order_whose_request_never_reached_the_broker_is_failed_by_read_back() {
    let _g = crate::tests_common::guard();
    let (_h, app, fake) = fresh();
    app.set_orders_live(true);
    let book = app.figures.get().unwrap().book().unwrap();
    fake.then(Behaviour::LoseBefore);
    gate::place(&app, &book, &order("order-1", None), &Asker::Person, t0()).unwrap().unwrap();
    let after = gate::read_back(&app, &book, "order-1", t0()).unwrap();
    assert_eq!((after.state, after.why.as_deref()), (OrderState::Failed, Some("the broker has no record of it")));
    assert_eq!(fake.at_broker(), 0);
}

#[test]
fn a_read_back_that_cannot_reach_the_broker_changes_nothing() {
    let _g = crate::tests_common::guard();
    let (_h, app, fake) = fresh();
    app.set_orders_live(true);
    let book = app.figures.get().unwrap().book().unwrap();
    fake.then(Behaviour::LoseAfter);
    gate::place(&app, &book, &order("order-1", None), &Asker::Person, t0()).unwrap().unwrap();
    fake.0.lock().unwrap().reads_fail = true;
    assert!(gate::read_back(&app, &book, "order-1", t0()).is_err());
    assert_eq!(book.order("order-1").unwrap().unwrap().fold.state, OrderState::Unconfirmed);
}

#[test]
fn with_orders_off_nothing_reaches_the_broker_and_the_order_is_written_dry() {
    let _g = crate::tests_common::guard();
    let (_h, app, fake) = fresh();
    let book = app.figures.get().unwrap().book().unwrap();
    assert_eq!(gate::place(&app, &book, &order("order-1", None), &Asker::Person, t0()).unwrap(), Err(Held::Dry));
    assert_eq!(book.order("order-1").unwrap().unwrap().fold.state, OrderState::Dry);
    assert_eq!(gate::cancel(&app, &book, "order-1", &Asker::Person, t0()).unwrap(), Err(Held::Dry));
    assert!(fake.0.lock().unwrap().creates.is_empty() && fake.0.lock().unwrap().cancels.is_empty());
}

fn bracket(book: &bagholder_book::Book, id: &str) {
    use bagholder_core::bracket::BracketEvent;
    let place = bagholder_book::orders::BracketPlace { id: id.into(), broker: "wealthsimple".into(), broker_account: "acct".into(), broker_security: "sec".into(), symbol: "SHOP".into(), currency: Currency::parse("USD").unwrap() };
    book.write_bracket(&place, &BracketEvent::Created { quantity: d("10"), stop: None, target: Some(d("110")) }, &Asker::Person, t0()).unwrap();
}

#[test]
fn no_exit_is_placed_while_another_of_its_bracket_is_in_flight_and_nothing_is_written_for_it() {
    let _g = crate::tests_common::guard();
    let (_h, app, fake) = fresh();
    app.set_orders_live(true);
    let book = app.figures.get().unwrap().book().unwrap();
    bracket(&book, "bracket-1");
    fake.then(Behaviour::LoseAfter);
    gate::place(&app, &book, &order("order-1", Some(("bracket-1", OrderRole::Stop))), &Asker::Engine, t0()).unwrap().unwrap();
    assert_eq!(gate::place(&app, &book, &order("order-2", Some(("bracket-1", OrderRole::Target))), &Asker::Engine, t0()).unwrap(), Err(Held::InFlight));
    assert!(book.order("order-2").unwrap().is_none());
    assert_eq!(fake.at_broker(), 1);
}

#[test]
fn a_bracket_that_sends_more_than_it_ever_could_in_a_minute_is_held() {
    let _g = crate::tests_common::guard();
    let (_h, app, fake) = fresh();
    app.set_orders_live(true);
    let book = app.figures.get().unwrap().book().unwrap();
    bracket(&book, "bracket-1");
    for i in 0..gate::BRACKET_CAP_PER_MINUTE {
        let at = t0() + SignedDuration::from_secs(i as i64);
        let id = format!("order-{i}");
        gate::place(&app, &book, &order(&id, Some(("bracket-1", OrderRole::Stop))), &Asker::Engine, at).unwrap().unwrap();
        // each one gone before the next, so only the cap can hold one back
        fake.set(&id, BrokerStatus::Cancelled, "0");
        gate::read_back(&app, &book, &id, at).unwrap();
    }
    let at = t0() + SignedDuration::from_secs(30);
    assert_eq!(gate::place(&app, &book, &order("order-x", Some(("bracket-1", OrderRole::Stop))), &Asker::Engine, at).unwrap(), Err(Held::Capped));
    // a minute on, it may send again
    let later = t0() + SignedDuration::from_secs(75);
    assert!(gate::place(&app, &book, &order("order-y", Some(("bracket-1", OrderRole::Stop))), &Asker::Engine, later).unwrap().is_ok());
}

#[test]
fn a_cancel_is_cancelling_until_the_broker_settles_it_and_a_refused_one_leaves_the_order_working() {
    let _g = crate::tests_common::guard();
    let (_h, app, fake) = fresh();
    app.set_orders_live(true);
    let book = app.figures.get().unwrap().book().unwrap();
    gate::place(&app, &book, &order("order-1", None), &Asker::Person, t0()).unwrap().unwrap();
    fake.then(Behaviour::Refuse("too late", None));
    assert_eq!(gate::cancel(&app, &book, "order-1", &Asker::Person, t0()).unwrap().unwrap().state, OrderState::Pending);
    assert_eq!(gate::cancel(&app, &book, "order-1", &Asker::Person, t0()).unwrap().unwrap().state, OrderState::Cancelling);
    // the fill beats the cancel
    fake.set("order-1", BrokerStatus::Filled, "10");
    assert_eq!(gate::read_back(&app, &book, "order-1", t0()).unwrap().state, OrderState::Filled);
    assert_eq!(gate::cancel(&app, &book, "order-1", &Asker::Person, t0()).unwrap(), Err(Held::NotNow("the order is filled".into())));
}

#[test]
fn who_asked_is_kept_with_every_request() {
    let _g = crate::tests_common::guard();
    let (_h, app, _fake) = fresh();
    app.set_orders_live(true);
    let book = app.figures.get().unwrap().book().unwrap();
    gate::place(&app, &book, &order("order-1", None), &Asker::Person, t0()).unwrap().unwrap();
    gate::cancel(&app, &book, "order-1", &Asker::Agent("helper".into()), t0()).unwrap().unwrap();
    let askers: Vec<(String, &'static str)> = book.order_log("order-1").unwrap().into_iter().map(|l| (l.asker.to_text(), l.event.kind())).collect();
    assert_eq!(askers, vec![("person".into(), "written"), ("person".into(), "accepted"), ("agent:helper".into(), "cancel-asked")]);
}

#[test]
fn wealthsimples_order_answer_is_read_strictly() {
    use crate::orders::gate::read_extended;
    let r = read_extended(&json!({"soOrdersExtendedOrder": {"status": "PARTIALLY_FILLED", "filledQuantity": "3", "averageFilledPrice": 101.25, "orderType": "SELL_QUANTITY", "limitPrice": "110.00", "submittedQuantity": 10, "expiredAtUtc": "2026-12-27T21:00:00Z"}})).unwrap();
    let Found::Order(r) = r else { panic!() };
    assert_eq!((r.status, r.filled, r.average, r.price, r.quantity), (BrokerStatus::Open, d("3"), Some(d("101.25")), Some(d("110.00")), Some(d("10"))));
    assert_eq!(read_extended(&json!({"soOrdersExtendedOrder": null})).unwrap(), Found::None);
    for bad in [
        json!({"soOrdersExtendedOrder": {"status": "SOMETHING_NEW"}}),
        json!({"soOrdersExtendedOrder": {"status": 5}}),
        json!({"soOrdersExtendedOrder": {"status": "FILLED", "filledQuantity": true}}),
        json!({"soOrdersExtendedOrder": {"status": "FILLED", "filledQuantity": "n/a"}}),
        json!({"soOrdersExtendedOrder": ["FILLED"]}),
        json!({}),
    ] {
        assert!(read_extended(&bad).is_err(), "{bad}");
    }
}
