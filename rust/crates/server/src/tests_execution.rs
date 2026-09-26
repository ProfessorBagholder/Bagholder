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
    /// Nothing is sent: the session lapsed.
    NotSent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FakeOrder {
    pub request: Value,
    pub status: BrokerStatus,
    pub filled: Dec,
    pub average: Option<Dec>,
    pub expires_at: Option<Timestamp>,
    /// The price and quantity it states now: another than was sent is a change made
    /// by hand at the broker.
    pub price: Option<Dec>,
    pub quantity: Option<Dec>,
    /// Why it rejected the order, and its code for the reason.
    pub why: Option<String>,
    pub code: Option<String>,
}

#[derive(Default)]
pub struct FakeState {
    pub orders: BTreeMap<String, FakeOrder>,
    pub creates: Vec<String>,
    pub cancels: Vec<String>,
    pub next: VecDeque<Behaviour>,
    pub reads_fail: bool,
    /// The orders read back, by id, in order.
    pub reads: Vec<String>,
    /// Orders whose read-back fails, by id.
    pub read_fails: Vec<String>,
    /// The changes asked, by id.
    pub modifies: Vec<(String, Value)>,
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
            s.orders.insert(id, FakeOrder { request: request.clone(), status: BrokerStatus::Open, filled: Dec::ZERO, average: None, expires_at: None, price: None, quantity: None, why: None, code: None });
        }
        match b {
            Behaviour::Accept => Sent::Accepted { broker_id: Some(format!("ws-{}", s.serial)) },
            Behaviour::NoId => Sent::Accepted { broker_id: None },
            Behaviour::Refuse(why, code) => Sent::Refused { why: why.into(), code: code.map(String::from) },
            Behaviour::LoseAfter | Behaviour::LoseBefore => Sent::Unclear { why: "the connection dropped".into() },
            Behaviour::NotSent => Sent::NotSent { why: "Not connected.".into() },
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

    fn modify(&self, _app: &Arc<App>, external_id: &str, change: &Value) -> Sent {
        let b = self.behaviour();
        self.0.lock().unwrap().modifies.push((external_id.to_string(), change.clone()));
        match b {
            Behaviour::Refuse(why, code) => Sent::Refused { why: why.into(), code: code.map(String::from) },
            Behaviour::LoseBefore => Sent::Unclear { why: "the connection dropped".into() },
            _ => Sent::Accepted { broker_id: None },
        }
    }

    fn read(&self, _app: &Arc<App>, external_id: &str) -> Result<Found, String> {
        let mut s = self.0.lock().unwrap();
        s.reads.push(external_id.to_string());
        if s.reads_fail || s.read_fails.iter().any(|f| f == external_id) {
            return Err("Wealthsimple could not be reached".into());
        }
        Ok(match s.orders.get(external_id) {
            None => Found::None,
            Some(o) => Found::Order(Reading { expires_at: o.expires_at, price: o.price, quantity: o.quantity, why: o.why.clone(), code: o.code.clone(), ..Reading::of(o.status, o.filled, o.average) }),
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

// ---------------------------------------------------------------------------
// the bracket engine and the ticket against the fake
// ---------------------------------------------------------------------------

use std::collections::HashMap;

use bagholder_book::orders::StoredOrder;
use bagholder_book::Book;
use bagholder_core::bracket::{BracketEvent, Phase, StopLeg, Tape, Trail};
use crate::orders::Quoted;

/// A bracket's world: an app with orders live against the fake, a session, a clock.
pub struct World {
    _home: tempfile::TempDir,
    pub app: Arc<App>,
    pub fake: Arc<FakeBroker>,
    pub book: Book,
    pub now: Timestamp,
    pub bid: Option<Dec>,
    pub open: bool,
}

impl World {
    pub fn new() -> World {
        let (home, app, fake) = fresh();
        app.set_orders_live(true);
        *app.orders.seam.session.lock().unwrap() = Some(bagholder_ws::session::Session { access_token: "tok".into(), ..Default::default() });
        *app.orders.seam.stop_allowed.lock().unwrap() = Some(true);
        let book = app.figures.get().unwrap().book().unwrap();
        World { _home: home, app, fake, book, now: t0(), bid: None, open: true }
    }

    /// A bracket on an entry of `qty` at 100, sent and taken.
    pub fn bracket(&mut self, id: &str, qty: &str, stop: Option<StopLeg>, target: Option<&str>) -> String {
        self.bracket_in("acct", id, qty, stop, target)
    }

    /// A bracket as `bracket`, in the broker's account `account`.
    pub fn bracket_in(&mut self, account: &str, id: &str, qty: &str, stop: Option<StopLeg>, target: Option<&str>) -> String {
        let place = bagholder_book::orders::BracketPlace { id: id.into(), broker: "wealthsimple".into(), broker_account: account.into(), broker_security: "sec".into(), symbol: "SHOP".into(), currency: Currency::parse("USD").unwrap() };
        self.book.write_bracket(&place, &BracketEvent::Created { quantity: d(qty), stop, target: target.map(d) }, &Asker::Person, self.now).unwrap();
        let entry = format!("{id}-entry");
        let mut o = order(&entry, Some((id, OrderRole::Entry)));
        o.quantity = d(qty);
        o.broker_account = account.into();
        gate::place(&self.app, &self.book, &o, &Asker::Person, self.now).unwrap().unwrap();
        entry
    }

    pub fn stop(level: &str) -> Option<StopLeg> {
        Some(StopLeg { level: d(level), trail: None, high: None })
    }

    pub fn quotes(&self) -> HashMap<String, Quoted> {
        let tape = self.bid.map(|b| Tape { last: b, bid: Some(b) });
        HashMap::from([("sec".to_string(), Quoted { open: self.open, tape: if self.open { tape } else { None }, problem: None })])
    }

    pub fn tick(&mut self) {
        crate::orders::bracket_tick(&self.app, Some(self.quotes()), self.now).unwrap();
    }

    /// Ticks until nothing more happens in one (at most a few).
    pub fn settle(&mut self) {
        for _ in 0..6 {
            let before = (self.sent(), self.log_len());
            self.tick();
            let after = (self.sent(), self.log_len());
            if before == after {
                return;
            }
        }
    }

    /// Creates and cancels the fake has been sent.
    fn sent(&self) -> (usize, usize) {
        let s = self.fake.0.lock().unwrap();
        (s.creates.len(), s.cancels.len())
    }

    fn log_len(&self) -> usize {
        self.book.live_brackets().unwrap().iter().map(|b| self.book.bracket_log(&b.place.id).unwrap().len()).sum::<usize>() + self.book.orders_in_flight().unwrap().len()
    }

    pub fn later(&mut self, secs: i64) {
        self.now = self.now + SignedDuration::from_secs(secs);
    }

    pub fn quote(&mut self, bid: &str) {
        self.bid = Some(d(bid));
    }

    pub fn phase(&self, id: &str) -> Phase {
        self.book.bracket(id).unwrap().unwrap().bracket.phase
    }

    pub fn bracket_of(&self, id: &str) -> bagholder_core::bracket::Bracket {
        self.book.bracket(id).unwrap().unwrap().bracket
    }

    /// The bracket's current exit, as the book holds it.
    pub fn exit(&self, id: &str) -> Option<StoredOrder> {
        let (_, oid) = self.bracket_of(id).exit?;
        self.book.order(&oid).unwrap()
    }

    /// The exits the fake holds working for the bracket's shares.
    pub fn working_exits(&self) -> Vec<(String, FakeOrder)> {
        self.fake.0.lock().unwrap().orders.iter().filter(|(k, o)| !k.ends_with("-entry") && !k.starts_with("sale-") && o.status == BrokerStatus::Open).map(|(k, o)| (k.clone(), o.clone())).collect()
    }

    /// Every stored state is what its log folds to.
    pub fn state_is_the_log(&self) {
        assert_eq!(self.book.states_disagreeing().unwrap(), Vec::<String>::new());
    }

    pub fn armed(stop: Option<StopLeg>, target: Option<&str>) -> (World, String) {
        let mut w = World::new();
        let entry = w.bracket("b1", "10", stop, target);
        w.quote("100");
        w.fake.set(&entry, BrokerStatus::Filled, "10");
        w.settle();
        assert_ne!(w.phase("b1"), Phase::Waiting, "armed");
        (w, "b1".into())
    }
}

#[test]
fn a_fill_arms_the_bracket_and_the_stop_goes_to_the_broker_at_once() {
    let _g = crate::tests_common::guard();
    let (w, b) = World::armed(World::stop("95"), Some("110"));
    assert_eq!(w.phase(&b), Phase::Guarding);
    let exits = w.working_exits();
    assert_eq!(exits.len(), 1, "one exit rests: {exits:?}");
    assert_eq!((exits[0].1.request["executionType"].as_str(), exits[0].1.request["stopPrice"].as_f64(), exits[0].1.request["timeInForce"].as_str()), (Some("STOP"), Some(95.0), Some("UNTIL_CANCEL")));
    let askers: Vec<String> = w.book.order_log(&exits[0].0).unwrap().into_iter().map(|l| l.asker.to_text()).collect();
    assert!(askers.iter().all(|a| a == "engine"), "the engine asked for the stop: {askers:?}");
    w.state_is_the_log();
}

#[test]
fn a_lost_answer_on_an_exit_the_broker_never_got_places_it_again_exactly_once() {
    let _g = crate::tests_common::guard();
    let mut w = World::new();
    let entry = w.bracket("b1", "10", World::stop("95"), None);
    w.quote("100");
    w.fake.set(&entry, BrokerStatus::Filled, "10");
    w.fake.then(Behaviour::Accept); // (the entry's read is not a create)
    w.fake.0.lock().unwrap().next.clear();
    w.fake.then(Behaviour::LoseBefore);
    w.tick(); // arms, and the stop goes out in the same check: its answer is lost
    assert_eq!(w.exit("b1").unwrap().fold.state, OrderState::Unconfirmed);
    w.tick(); // read back: no such order; failed; placed again
    w.settle();
    let creates: Vec<String> = w.fake.0.lock().unwrap().creates.iter().filter(|c| !c.ends_with("-entry")).cloned().collect();
    assert_eq!(creates.len(), 2, "the lost one and exactly one more: {creates:?}");
    assert_eq!(w.working_exits().len(), 1);
    w.state_is_the_log();
}

#[test]
fn a_lost_answer_on_an_exit_the_broker_took_ends_working_and_nothing_more_is_placed() {
    let _g = crate::tests_common::guard();
    let mut w = World::new();
    let entry = w.bracket("b1", "10", World::stop("95"), None);
    w.quote("100");
    w.fake.set(&entry, BrokerStatus::Filled, "10");
    w.tick();
    w.fake.then(Behaviour::LoseAfter);
    w.tick();
    w.settle();
    assert_eq!(w.exit("b1").unwrap().fold.state, OrderState::Pending, "never failed: read back working");
    assert_eq!(w.working_exits().len(), 1, "exactly one at the broker");
    w.state_is_the_log();
}

#[test]
fn an_order_filling_in_three_parts_read_twice_and_at_once_holds_exactly_what_filled() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    gate::place(&w.app, &w.book, &order("o1", None), &Asker::Person, t0()).unwrap().unwrap();
    for (filled, avg) in [("3", "100"), ("3", "100"), ("7", "101"), ("10", "101.5")] {
        {
            let mut s = w.fake.0.lock().unwrap();
            let o = s.orders.get_mut("o1").unwrap();
            o.filled = d(filled);
            o.average = Some(d(avg));
            o.status = if filled == "10" { BrokerStatus::Filled } else { BrokerStatus::Open };
        }
        // two read-backs at once
        let (a1, a2) = (w.app.clone(), w.app.clone());
        let h1 = std::thread::spawn(move || {
            let b = a1.figures.get().unwrap().book().unwrap();
            gate::read_back(&a1, &b, "o1", t0()).unwrap()
        });
        let h2 = std::thread::spawn(move || {
            let b = a2.figures.get().unwrap().book().unwrap();
            gate::read_back(&a2, &b, "o1", t0()).unwrap()
        });
        h1.join().unwrap();
        h2.join().unwrap();
    }
    let o = w.book.order("o1").unwrap().unwrap();
    assert_eq!((o.fold.state, o.fold.filled, o.fold.average), (OrderState::Filled, d("10"), Some(d("101.5"))));
    assert_eq!(w.book.orders_filled().unwrap().len(), 1);
    w.state_is_the_log();
}

#[test]
fn a_filled_quantity_that_goes_down_is_kept_shown_and_never_taken() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    gate::place(&w.app, &w.book, &order("o1", None), &Asker::Person, t0()).unwrap().unwrap();
    w.fake.set("o1", BrokerStatus::Open, "6");
    gate::read_back(&w.app, &w.book, "o1", t0()).unwrap();
    w.fake.set("o1", BrokerStatus::Open, "4");
    let after = gate::read_back(&w.app, &w.book, "o1", t0()).unwrap();
    assert_eq!(after.filled, d("6"), "nothing taken back");
    let stored = w.book.order("o1").unwrap().unwrap();
    assert!(stored.refused.as_deref().is_some_and(|r| r.contains("went down")), "{:?}", stored.refused);
    let card = crate::orders::orders_doc(&w.app).orders.into_iter().find(|c| c.id == "o1").unwrap();
    assert!(card.why.as_deref().is_some_and(|r| r.contains("went down")), "shown on the order: {:?}", card.why);
    w.state_is_the_log();
}

#[test]
fn five_thousand_orders_leave_the_oldest_live_bracket_followed() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), None);
    for i in 0..5000 {
        let mut o = order(&format!("filler-{i}"), None);
        o.request = json!({"externalId": o.id});
        w.book.write_order(&o, true, &Asker::Person, t0() + SignedDuration::from_secs(60 + i)).unwrap();
    }
    w.later(120);
    let stop = w.exit(&b).unwrap().request.id;
    w.settle();
    assert!(w.fake.0.lock().unwrap().cancels.is_empty(), "nothing cancelled as not found");
    assert_eq!(w.exit(&b).unwrap().request.id, stop, "the stop is still the bracket's");
    // its fill is still seen
    w.fake.set(&stop, BrokerStatus::Filled, "10");
    w.later(5);
    w.settle();
    assert_eq!((w.phase(&b), w.bracket_of(&b).outcome.as_deref()), (Phase::Ended, Some("stopped")));
}

#[test]
fn the_target_reached_cancels_the_stop_and_a_lapsed_session_leaves_the_stop_watched_until_the_target_goes_out() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), Some("110"));
    let stop = w.exit(&b).unwrap().request.id;
    w.quote("110");
    w.tick();
    assert_eq!(w.phase(&b), Phase::ToTarget);
    assert_eq!(w.fake.0.lock().unwrap().cancels, vec![stop.clone()]);
    // the broker confirms the cancel; the session lapses before the target goes out
    w.fake.set(&stop, BrokerStatus::Cancelled, "0");
    *w.app.orders.seam.session.lock().unwrap() = None;
    w.fake.then(Behaviour::NotSent);
    w.later(5);
    w.tick(); // the cancel is read back: cleared
    w.tick(); // the target is tried: not sent
    assert!(w.working_exits().is_empty(), "nothing rests at the broker");
    assert_eq!(w.phase(&b), Phase::ToTarget, "the stop level is watched meanwhile");
    // the price falls to the stop while nothing rests: a market sell goes out (after the retry rest)
    *w.app.orders.seam.session.lock().unwrap() = Some(bagholder_ws::session::Session { access_token: "tok".into(), ..Default::default() });
    w.quote("94");
    w.later(61);
    w.tick();
    let working = w.working_exits();
    assert_eq!(working.len(), 1, "{working:?}");
    assert_eq!(working[0].1.request["executionType"].as_str(), Some("MARKET"));
    w.state_is_the_log();
}

#[test]
fn a_market_sell_rejected_after_it_was_taken_puts_the_stop_back() {
    let _g = crate::tests_common::guard();
    let mut w = World::new();
    *w.app.orders.seam.stop_allowed.lock().unwrap() = Some(false); // watched here
    let entry = w.bracket("b1", "10", World::stop("95"), None);
    w.quote("100");
    w.fake.set(&entry, BrokerStatus::Filled, "10");
    w.settle();
    w.quote("94");
    w.tick();
    assert_eq!(w.phase("b1"), Phase::Firing);
    let market = w.exit("b1").unwrap().request.id;
    w.fake.set(&market, BrokerStatus::Rejected, "0");
    w.later(5);
    w.tick();
    assert_eq!(w.phase("b1"), Phase::Guarding, "out of firing, the stop watched again");
    // after the rest a refusal earns, the watched stop fires again
    w.later(61);
    w.tick();
    assert_eq!(w.phase("b1"), Phase::Firing);
    w.state_is_the_log();
}

#[test]
fn a_partly_filled_exit_that_expires_is_placed_again_for_the_rest_good_till_cancelled() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), None);
    let stop = w.exit(&b).unwrap().request.id;
    w.fake.set(&stop, BrokerStatus::Expired, "4");
    w.later(5);
    w.settle();
    let again = w.exit(&b).unwrap();
    assert_ne!(again.request.id, stop);
    assert_eq!((again.request.quantity, again.request.stop_price, again.request.time_in_force), (d("6"), Some(d("95")), TimeInForce::UntilCancel));
    assert_eq!(w.bracket_of(&b).quantity, d("6"));
    w.state_is_the_log();
}

#[test]
fn a_cancel_confirmed_after_a_fill_and_a_fill_read_after_a_cancel_are_both_the_fill() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    gate::place(&w.app, &w.book, &order("o1", None), &Asker::Person, t0()).unwrap().unwrap();
    gate::cancel(&w.app, &w.book, "o1", &Asker::Person, t0()).unwrap().unwrap();
    w.fake.set("o1", BrokerStatus::Filled, "10");
    assert_eq!(gate::read_back(&w.app, &w.book, "o1", t0()).unwrap().state, OrderState::Filled);
    // the other order: read cancelled first, then the fill that beat it
    gate::place(&w.app, &w.book, &order("o2", None), &Asker::Person, t0()).unwrap().unwrap();
    w.fake.set("o2", BrokerStatus::Cancelled, "0");
    gate::read_back(&w.app, &w.book, "o2", t0()).unwrap();
    w.fake.set("o2", BrokerStatus::Filled, "10");
    assert_eq!(gate::read_back(&w.app, &w.book, "o2", t0()).unwrap().state, OrderState::Filled);
    w.state_is_the_log();
}

fn sale(qty: &str) -> OrderRequest {
    let mut o = order("sale-1", None);
    o.side = Side::Sell;
    o.kind = OrderKind::Market;
    o.limit_price = None;
    o.quantity = d(qty);
    o.request = json!({"externalId": "sale-1", "orderType": "SELL_QUANTITY"});
    o
}

#[test]
fn a_sale_from_the_ticket_goes_out_only_once_the_stops_cancel_is_confirmed() {
    let _g = crate::tests_common::guard();
    let (w, b) = World::armed(World::stop("95"), None);
    let stop = w.exit(&b).unwrap().request.id;
    // the fake confirms a cancel at once when read after it
    let fake = w.fake.clone();
    let s = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        fake.set(&s, BrokerStatus::Cancelled, "0");
    });
    let got = crate::orders::sell(&w.app, &w.book, &sale("10")).unwrap();
    assert!(got.ok, "{got:?}");
    let creates = w.fake.0.lock().unwrap().creates.clone();
    assert_eq!(creates.last().map(String::as_str), Some("sale-1"));
    assert_eq!(w.fake.0.lock().unwrap().orders[&stop].status, BrokerStatus::Cancelled, "the stop was gone before the sale went out");
    assert_eq!((w.phase(&b), w.bracket_of(&b).outcome.as_deref()), (Phase::Ended, Some("sold from the ticket")));
    w.state_is_the_log();
}

#[test]
fn a_sale_whose_stop_cancel_is_not_confirmed_sells_nothing_and_the_stop_rests_again() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), None);
    w.app.orders.sale_wait.store(1, std::sync::atomic::Ordering::SeqCst);
    let got = crate::orders::sell(&w.app, &w.book, &sale("10")).unwrap();
    assert!(!got.ok && got.error.as_deref().is_some_and(|e| e.starts_with("Nothing was sold")), "{got:?}");
    assert!(!w.fake.0.lock().unwrap().creates.contains(&"sale-1".to_string()), "nothing sold");
    assert_eq!(w.phase(&b), Phase::Guarding);
    // the cancel lands later: the stop goes back
    let stop = w.exit(&b).unwrap().request.id;
    w.fake.set(&stop, BrokerStatus::Cancelled, "0");
    w.later(5);
    w.settle();
    assert_eq!(w.working_exits().len(), 1, "a stop rests again");
    w.state_is_the_log();
}

#[test]
fn a_sale_refused_puts_the_stop_back() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), None);
    let stop = w.exit(&b).unwrap().request.id;
    let fake = w.fake.clone();
    let s = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        fake.set(&s, BrokerStatus::Cancelled, "0");
        fake.then(Behaviour::Refuse("Market closed", None));
    });
    let got = crate::orders::sell(&w.app, &w.book, &sale("10")).unwrap();
    assert!(!got.ok, "{got:?}");
    assert_eq!(w.phase(&b), Phase::Guarding);
    w.later(5);
    w.settle();
    assert_eq!(w.working_exits().len(), 1, "the stop rests again");
    w.state_is_the_log();
}

#[test]
fn part_of_the_shares_sold_from_the_ticket_leaves_the_stop_on_the_rest() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), None);
    let stop = w.exit(&b).unwrap().request.id;
    let fake = w.fake.clone();
    let s = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        fake.set(&s, BrokerStatus::Cancelled, "0");
    });
    assert!(crate::orders::sell(&w.app, &w.book, &sale("4")).unwrap().ok);
    w.later(5);
    w.settle();
    assert_eq!((w.phase(&b), w.bracket_of(&b).quantity), (Phase::Guarding, d("6")));
    let working = w.working_exits();
    assert_eq!(working.len(), 1);
    assert_eq!(working[0].1.request["quantity"].as_f64(), Some(6.0));
    w.state_is_the_log();
}

#[test]
fn with_orders_off_a_ticket_with_legs_is_recorded_and_no_bracket_waits() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    w.app.set_orders_live(false);
    let asked = crate::orders::TicketOrder { order: order("o1", None), stop: World::stop("95"), target: Some(d("110")) };
    let got = crate::orders::entry(&w.app, &w.book, asked).unwrap();
    assert_eq!((got.ok, got.status.as_deref(), got.bracket_id.as_deref()), (true, Some("dry"), None));
    assert!(w.book.live_brackets().unwrap().is_empty(), "no bracket waits on a fill that cannot come");
    assert_eq!(w.book.order("o1").unwrap().unwrap().fold.state, OrderState::Dry);
    assert!(w.fake.0.lock().unwrap().creates.is_empty(), "nothing left the gate");
}

#[test]
fn a_ticket_with_legs_writes_its_bracket_before_the_entry_goes_out() {
    let _g = crate::tests_common::guard();
    let w = World::new();
    let asked = crate::orders::TicketOrder { order: order("o1", None), stop: World::stop("95"), target: Some(d("110")) };
    let got = crate::orders::entry(&w.app, &w.book, asked).unwrap();
    let bid = got.bracket_id.clone().expect("a bracket");
    let sb = w.book.bracket(&bid).unwrap().unwrap();
    assert!(sb.created_at <= w.book.order("o1").unwrap().unwrap().created_at);
    assert_eq!(w.book.order("o1").unwrap().unwrap().request.bracket, Some((bid, OrderRole::Entry)));
    // refused at once: the bracket ends with it
    let asked = crate::orders::TicketOrder { order: order("o2", None), stop: World::stop("95"), target: None };
    w.fake.then(Behaviour::Refuse("Insufficient funds", None));
    let got = crate::orders::entry(&w.app, &w.book, asked).unwrap();
    assert!(!got.ok);
    assert_eq!(w.book.bracket(got.bracket_id.as_deref().unwrap()).unwrap().unwrap().bracket.phase, Phase::Ended);
    w.state_is_the_log();
}

#[test]
fn a_bracket_that_sends_more_than_its_cap_stops_and_the_header_says_so() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(Some(StopLeg { level: d("95"), trail: Some(Trail::Pct(d("5"))), high: None }), None);
    // a price running up a step a check: each is a cancel and a new stop
    let mut price = d("100");
    for _ in 0..12 {
        price = price.checked_mul(d("1.02")).unwrap().round(2, bagholder_core::Rounding::HalfUp);
        w.bid = Some(price);
        if let Some(x) = w.exit(&b) {
            if x.fold.state == OrderState::Cancelling {
                w.fake.set(&x.request.id, BrokerStatus::Cancelled, "0");
            }
        }
        w.later(2);
        w.tick();
    }
    assert_eq!(w.phase(&b), Phase::Halted);
    let failures = crate::orders::order_failures(&w.app);
    assert!(failures.iter().any(|f| f.contains("The bracket on SHOP is stopped")), "{failures:?}");
    // stopped: nothing more is sent
    let before = w.fake.0.lock().unwrap().creates.len();
    w.later(120);
    w.tick();
    assert_eq!(w.fake.0.lock().unwrap().creates.len(), before);
    w.state_is_the_log();
}

#[test]
fn a_quote_older_than_fifteen_seconds_by_its_own_time_is_not_acted_on() {
    let now: Timestamp = "2026-09-28T14:00:00Z".parse().unwrap();
    let q = |at: &str| crate::orders::TicketQuoteDetail { market_status: "OPEN".into(), last: Some(94.0), bid: Some(94.0), quoted_as_of: at.into(), ..Default::default() };
    assert!(crate::orders::quoted(&q("2026-09-28T13:59:50Z"), now).tape.is_some());
    let stale = crate::orders::quoted(&q("2026-09-28T13:59:40Z"), now);
    assert!(stale.tape.is_none() && stale.problem.is_some(), "{stale:?}");
    let unstated = crate::orders::quoted(&q(""), now);
    assert!(unstated.tape.is_none() && unstated.problem.is_some());
    let closed = crate::orders::quoted(&crate::orders::TicketQuoteDetail { market_status: "CLOSED".into(), ..q("2026-09-28T13:00:00Z") }, now);
    assert!(closed.tape.is_none() && closed.problem.is_none(), "a closed market is not a failure");
}

#[test]
fn nothing_fires_on_a_stale_quote() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), None);
    *w.app.orders.seam.stop_allowed.lock().unwrap() = Some(false);
    let _ = &mut w;
    let before = w.fake.0.lock().unwrap().creates.len();
    let mut quotes = w.quotes();
    quotes.insert("sec".into(), Quoted { open: true, tape: None, problem: Some("Wealthsimple's quote is from long ago, not current".into()) });
    crate::orders::bracket_tick(&w.app, Some(quotes), w.now).unwrap();
    assert_eq!(w.fake.0.lock().unwrap().creates.len(), before);
    assert_eq!(w.phase(&b), Phase::Guarding);
}

#[test]
fn an_exit_resting_with_no_live_bracket_holding_it_is_cancelled() {
    let _g = crate::tests_common::guard();
    let (mut w, b) = World::armed(World::stop("95"), None);
    // an exit of the bracket's that it no longer holds (as after a crash between placing and recording)
    let mut stray = order("stray", Some((b.as_str(), OrderRole::Target)));
    stray.side = Side::Sell;
    w.book.write_order(&stray, false, &Asker::Engine, w.now).unwrap();
    w.book.order_event("stray", &Asker::Engine, w.now, &bagholder_core::order::OrderEvent::Accepted { broker_id: "ws-x".into() }).unwrap().unwrap();
    w.later(5);
    w.tick();
    assert_eq!(w.fake.0.lock().unwrap().cancels, vec!["stray".to_string()]);
}
