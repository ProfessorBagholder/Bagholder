//! The bracket's rules (`SPEC.md` §6, Brackets) against a fake broker that
//! answers, refuses, loses answers and fills in parts. Every send is checked
//! against the invariant that nothing is placed while an exit is in flight.

use bagholder_core::bracket::{self, Bracket, BracketEvent, Exit, ExitRole, Phase, RefusalClass, Request, Seen, StateKey, StopLeg, Tape, Trail};
use bagholder_core::order::{BrokerStatus, OrderEvent, OrderFold, OrderState, Reading};
use bagholder_core::Dec;
use jiff::{SignedDuration, Timestamp};

fn d(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

/// What the fake broker does with the next request.
#[derive(Clone, Debug, PartialEq)]
enum Answer {
    Accept,
    Refuse(&'static str, Option<&'static str>),
    /// The answer is lost: the order may or may not be at the broker.
    Lose,
}

struct World {
    b: Bracket,
    now: Timestamp,
    open: bool,
    tape: Option<Tape>,
    entry: OrderFold,
    exit: Option<Exit>,
    next: u32,
    answer: Answer,
    placed: Vec<(ExitRole, Option<Dec>, Dec)>,
    cancels: Vec<String>,
    resizes: Vec<Dec>,
    closed_elsewhere: Option<String>,
    key: StateKey,
    sale_running: bool,
}

impl World {
    fn new(stop: Option<StopLeg>, target: Option<&str>, native: bool) -> World {
        let now: Timestamp = "2026-09-28T15:00:00Z".parse().unwrap();
        let mut b = Bracket::of(&[(now, BracketEvent::Created { quantity: d("10"), stop, target: target.map(d) })]).unwrap();
        b.native = native;
        let entry = OrderFold::of(&[OrderEvent::Written { dry: false }, OrderEvent::Accepted { broker_id: "entry".into() }]).unwrap();
        World { b, now, open: true, tape: None, entry, exit: None, next: 0, answer: Answer::Accept, placed: vec![], cancels: vec![], resizes: vec![], closed_elsewhere: None, key: StateKey::default(), sale_running: false }
    }

    fn stop(level: &str) -> Option<StopLeg> {
        Some(StopLeg { level: d(level), trail: None, high: None })
    }

    fn quote(&mut self, bid: &str) {
        self.tape = Some(Tape { last: d(bid), bid: Some(d(bid)) });
    }

    fn fill_entry(&mut self, qty: &str, avg: &str) {
        self.entry.apply(&OrderEvent::Read(Reading::of(BrokerStatus::Filled, d(qty), Some(d(avg))))).unwrap();
    }

    fn later(&mut self, secs: i64) {
        self.now = self.now + SignedDuration::from_secs(secs);
    }

    /// One tick: steps until nothing, or until a request went out.
    fn tick(&mut self) {
        for _ in 0..16 {
            let step = {
                let seen = Seen { now: self.now, open: self.open, tape: self.tape, entry: Some(&self.entry), exit: self.exit.as_ref(), closed_elsewhere: self.closed_elsewhere.clone(), stop_allowed: self.b.native, key: self.key.clone(), sale_running: self.sale_running, entry_order: Some("entry") };
                bracket::decide(&self.b, &seen)
            };
            if step.is_nothing() {
                return;
            }
            for e in &step.events {
                self.b.apply(self.now, e).unwrap_or_else(|why| panic!("{why}: {e:?} from {:?}", self.b.phase));
                if let BracketEvent::CancelAsked { order_id } = e {
                    let x = self.exit.as_mut().expect("a cancel for the exit held");
                    assert_eq!(&x.order_id, order_id);
                    x.fold.apply(&OrderEvent::CancelAsked).unwrap();
                    x.cancel_asked = true;
                    x.cancel_asked_at = Some(self.now);
                }
            }
            if self.b.exit.is_none() {
                self.exit = None;
            }
            if let Some(r) = step.request {
                self.send(r);
                return;
            }
        }
        panic!("a tick that never settles: {:?}", self.b);
    }

    fn send(&mut self, r: Request) {
        match r {
            Request::Place { role, price, quantity } => {
                let request = Request::Place { role, price, quantity };
                assert!(self.exit.as_ref().map_or(true, |x| !x.fold.state.in_flight()), "placed while an exit is in flight: {:?}", self.exit);
                assert!(quantity.is_positive());
                self.placed.push((role, price, quantity));
                self.next += 1;
                let id = format!("x{}", self.next);
                let mut fold = OrderFold::of(&[OrderEvent::Written { dry: false }]).unwrap();
                match self.answer.clone() {
                    Answer::Refuse(why, code) => {
                        let class = bracket::classify(false, code, why);
                        self.b.apply(self.now, &BracketEvent::Refused { why: why.into(), code: code.map(String::from), role: Some(role), class, key: self.key.clone() }).unwrap();
                        return;
                    }
                    Answer::Accept => {
                        fold.apply(&OrderEvent::Accepted { broker_id: id.clone() }).unwrap();
                    }
                    Answer::Lose => {
                        fold.apply(&OrderEvent::Unclear { why: "the connection dropped".into() }).unwrap();
                    }
                }
                self.b.apply(self.now, &bracket::placed(&request, &id).unwrap()).unwrap();
                self.exit = Some(Exit { order_id: id, role, fold, cancel_asked: false, cancel_asked_at: None, price, quantity: Some(quantity), expires_at: Some(self.now + SignedDuration::from_hours(24 * 90)) });
            }
            Request::Cancel { order_id } => {
                if let Some(x) = self.exit.as_mut().filter(|x| x.order_id == order_id) {
                    x.cancel_asked_at = Some(self.now);
                }
                self.cancels.push(order_id)
            }
            Request::Resize { order_id, quantity } => {
                let x = self.exit.as_mut().expect("a resize of the exit held");
                assert_eq!(x.order_id, order_id);
                x.quantity = Some(quantity);
                self.resizes.push(quantity);
                self.b.apply(self.now, &BracketEvent::Resized { quantity }).unwrap();
            }
        }
    }

    fn broker(&mut self, status: BrokerStatus, filled: &str) {
        let x = self.exit.as_mut().expect("an exit at the broker");
        x.fold.apply(&OrderEvent::Read(Reading::of(status, d(filled), Some(d("1"))))).unwrap();
    }

    fn role(&self) -> Option<ExitRole> {
        self.exit.as_ref().map(|x| x.role)
    }
}

#[test]
fn a_fill_arms_the_bracket_for_what_filled_and_places_the_stop_at_once() {
    let mut w = World::new(World::stop("95"), Some("110"), true);
    w.tick();
    assert_eq!((w.b.phase, w.placed.len()), (Phase::Waiting, 0), "nothing before the entry fills");
    w.fill_entry("8", "100");
    w.tick();
    w.tick();
    assert_eq!(w.b.phase, Phase::Guarding);
    assert_eq!(w.placed, vec![(ExitRole::Stop, Some(d("95")), d("8"))], "the stop for the 8 that filled");
}

#[test]
fn an_entry_that_ends_unfilled_ends_the_bracket() {
    let mut w = World::new(World::stop("95"), None, true);
    w.entry.apply(&OrderEvent::Read(Reading::of(BrokerStatus::Cancelled, Dec::ZERO, None))).unwrap();
    w.tick();
    assert_eq!((w.b.phase, w.b.outcome.as_deref()), (Phase::Ended, Some("entry cancelled")));
}

fn armed(stop: Option<StopLeg>, target: Option<&str>, native: bool) -> World {
    let mut w = World::new(stop, target, native);
    w.fill_entry("10", "100");
    w.quote("100");
    w.tick();
    w.tick();
    w
}

#[test]
fn a_watched_stop_fires_a_market_sell_on_the_bid_and_its_fill_ends_the_bracket() {
    let mut w = armed(World::stop("95"), None, false);
    assert!(w.placed.is_empty(), "the broker takes no stop order for it: watched here");
    w.quote("95.5");
    w.tick();
    assert!(w.placed.is_empty());
    w.quote("94.9");
    w.tick();
    assert_eq!((w.b.phase, w.role()), (Phase::Firing, Some(ExitRole::Market)));
    w.broker(BrokerStatus::Filled, "10");
    w.tick();
    assert_eq!((w.b.phase, w.b.outcome.as_deref()), (Phase::Ended, Some("stopped")));
}

#[test]
fn the_target_waits_for_the_stops_cancel_to_be_confirmed_and_then_rests_with_the_stop_watched() {
    let mut w = armed(World::stop("95"), Some("110"), true);
    assert_eq!(w.role(), Some(ExitRole::Stop));
    w.quote("110");
    w.tick();
    assert_eq!((w.b.phase, w.cancels.len()), (Phase::ToTarget, 1));
    w.tick();
    w.tick();
    assert_eq!(w.placed.len(), 1, "no target while the stop's cancel is not confirmed: the stop stays in force");
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.tick();
    assert_eq!((w.b.phase, w.role()), (Phase::Target, Some(ExitRole::Target)));
    assert_eq!(w.placed.last().unwrap().1, Some(d("110")));
    w.broker(BrokerStatus::Filled, "10");
    w.tick();
    assert_eq!((w.b.phase, w.b.outcome.as_deref()), (Phase::Ended, Some("target")));
}

fn at_target() -> World {
    let mut w = armed(World::stop("95"), Some("110"), true);
    w.quote("110");
    w.tick();
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.tick();
    assert_eq!(w.b.phase, Phase::Target);
    w
}

#[test]
fn the_stop_level_reached_while_the_limit_rests_cancels_it_then_sells_at_market() {
    let mut w = at_target();
    w.quote("94");
    w.tick();
    assert_eq!(w.b.phase, Phase::ToMarket);
    w.tick();
    assert_eq!(w.placed.last().unwrap().0, ExitRole::Target, "no market sell until the limit's cancel is confirmed");
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.tick();
    assert_eq!((w.b.phase, w.role()), (Phase::Firing, Some(ExitRole::Market)));
}

#[test]
fn a_percent_under_the_target_the_limit_gives_way_to_the_stop_again() {
    let mut w = at_target();
    w.quote("109");
    w.tick();
    assert_eq!(w.b.phase, Phase::Target, "within a percent: the limit stays");
    w.quote("108.8");
    w.tick();
    assert_eq!(w.b.phase, Phase::BackToStop);
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.tick();
    assert_eq!((w.b.phase, w.role()), (Phase::Guarding, Some(ExitRole::Stop)));
    assert_eq!(w.placed.last().unwrap().1, Some(d("95")));
}

#[test]
fn a_trailing_stop_follows_the_high_by_half_a_percent_or_more_by_cancel_and_new_order() {
    let trail = Some(StopLeg { level: d("95"), trail: Some(Trail::Pct(d("5"))), high: None });
    let mut w = armed(trail, None, true);
    assert_eq!(w.placed[0].1, Some(d("95.00")), "armed from the fill's price: 100 less 5%");
    w.quote("100.3");
    w.tick();
    assert!(w.cancels.is_empty(), "95.29 is under half a percent above 95");
    w.quote("101");
    w.tick();
    w.tick();
    assert_eq!(w.b.stop.unwrap().level, d("95.95"));
    assert_eq!(w.cancels.len(), 1, "the resting stop is cancelled");
    w.tick();
    assert_eq!(w.placed.len(), 1, "and not placed again before the cancel is confirmed");
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.tick();
    assert_eq!(w.placed.last().unwrap(), &(ExitRole::Stop, Some(d("95.95")), d("10")));
}

#[test]
fn a_refusal_that_can_never_succeed_is_never_sent_again_and_the_stop_watched_here_fires_at_once() {
    let mut w = World::new(World::stop("95"), None, true);
    w.fill_entry("10", "100");
    w.answer = Answer::Refuse("Stop price has too many decimal places. Max allowed: 2", None);
    w.tick();
    w.tick();
    assert_eq!(w.b.refused.as_ref().map(|r| r.class), Some(RefusalClass::Invalid));
    for _ in 0..50 {
        w.later(3600);
        w.key.working = Some(w.key.working.unwrap_or(0) + 1);
        w.tick();
    }
    assert_eq!(w.placed.len(), 1, "never sent again, whatever changes");
    // the level crossed: a market sell at once, not held back by the stop's refusal
    w.answer = Answer::Accept;
    w.quote("94.90");
    w.tick();
    assert_eq!(w.placed.last().unwrap().0, ExitRole::Market);
}

#[test]
fn a_refusal_that_depends_on_how_things_stand_is_asked_again_only_when_they_change() {
    let mut w = World::new(World::stop("95"), None, true);
    w.fill_entry("10", "100");
    w.answer = Answer::Refuse("not enough shares", Some("NOT_ENOUGH_SHARES"));
    w.tick();
    w.tick();
    assert_eq!((w.b.phase, w.b.refused.as_ref().map(|r| r.class)), (Phase::Guarding, Some(RefusalClass::State)), "the shares may be tied up in another order: not ended");
    for _ in 0..20 {
        w.later(3600);
        w.tick();
    }
    assert_eq!(w.placed.len(), 1, "no timer sends it again");
    // the other order on the shares ends: asked again once
    w.key.working = Some(0);
    w.answer = Answer::Accept;
    w.tick();
    assert_eq!((w.placed.len(), w.role()), (2, Some(ExitRole::Stop)));
}

#[test]
fn an_answer_not_on_record_is_asked_again_on_each_change_at_most_three_times_then_only_watched() {
    let mut w = World::new(World::stop("95"), None, true);
    w.fill_entry("10", "100");
    w.answer = Answer::Refuse("something Wealthsimple never said before", None);
    w.tick();
    w.tick();
    for n in 0..6 {
        w.later(60);
        w.tick();
        assert_eq!(w.placed.len(), (n.min(2) + 1) as usize, "unchanged: nothing more");
        w.key.open = !w.key.open;
        w.tick();
    }
    assert_eq!(w.placed.len(), 3, "three tries, then the stop is only watched");
}

#[test]
fn a_send_with_no_answer_is_sent_again_at_the_next_check() {
    let mut w = World::new(World::stop("95"), None, true);
    w.fill_entry("10", "100");
    w.answer = Answer::Refuse(bracket::SESSION_LAPSED, None);
    w.tick();
    assert_eq!((w.placed.len(), w.b.refused.as_ref().map(|r| r.class)), (1, Some(RefusalClass::NoAnswer)));
    w.answer = Answer::Accept;
    w.later(5);
    w.tick();
    assert_eq!((w.placed.len(), w.role()), (2, Some(ExitRole::Stop)));
}

#[test]
fn a_fired_market_sell_never_waits_on_a_refused_stop() {
    for why in ["not enough shares", "something Wealthsimple never said before", "Stop price has too many decimal places. Max allowed: 2"] {
        let mut w = World::new(World::stop("95"), None, true);
        w.fill_entry("10", "100");
        w.answer = Answer::Refuse(why, None);
        w.tick();
        w.tick();
        w.answer = Answer::Accept;
        w.later(1);
        w.quote("94");
        w.tick();
        assert_eq!(w.placed.last().unwrap().0, ExitRole::Market, "{why}: the market sell went out a second after the refusal");
    }
}

#[test]
fn an_exit_cancelled_at_the_broker_without_the_app_asking_is_told_and_the_stop_watched_here() {
    let mut w = armed(World::stop("95"), None, true);
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    assert_eq!((w.b.phase, w.b.off_broker), (Phase::Guarding, true), "not an outcome: the bracket lives");
    let placed = w.placed.len();
    w.later(60);
    w.tick();
    assert_eq!(w.placed.len(), placed, "nothing placed at the broker again until the person acts");
    w.quote("94");
    w.tick();
    assert_eq!(w.placed.last().unwrap().0, ExitRole::Market, "the level is watched here and fires");
}

#[test]
fn a_lost_answer_places_nothing_more_until_the_read_back_settles_it() {
    let mut w = World::new(World::stop("95"), None, true);
    w.fill_entry("10", "100");
    w.answer = Answer::Lose;
    w.tick();
    w.tick();
    assert_eq!(w.exit.as_ref().unwrap().fold.state, OrderState::Unconfirmed);
    w.answer = Answer::Accept;
    for _ in 0..3 {
        w.later(60);
        w.tick();
    }
    assert_eq!(w.placed.len(), 1, "never a second stop while the first may rest");
    // the broker has it
    w.broker(BrokerStatus::Open, "0");
    w.tick();
    assert_eq!((w.placed.len(), w.exit.as_ref().unwrap().fold.state), (1, OrderState::Pending));
}

#[test]
fn a_lost_answer_the_broker_has_no_record_of_is_placed_again_exactly_once() {
    let mut w = World::new(World::stop("95"), None, true);
    w.fill_entry("10", "100");
    w.answer = Answer::Lose;
    w.tick();
    w.tick();
    w.answer = Answer::Accept;
    w.broker(BrokerStatus::NotFound, "0");
    w.tick();
    w.tick();
    w.tick();
    assert_eq!(w.placed.len(), 2);
    assert_eq!(w.exit.as_ref().unwrap().fold.state, OrderState::Pending);
}

#[test]
fn an_ended_bracket_is_closing_until_its_exits_cancel_is_confirmed() {
    let mut w = armed(World::stop("95"), None, true);
    let step = bracket::end(w.exit.as_ref(), "cancelled by the user");
    for e in &step.events {
        w.b.apply(w.now, e).unwrap();
        if let BracketEvent::CancelAsked { .. } = e {
            let x = w.exit.as_mut().unwrap();
            x.fold.apply(&OrderEvent::CancelAsked).unwrap();
            x.cancel_asked = true;
        }
    }
    assert_eq!(w.b.phase, Phase::Closing);
    w.tick();
    assert_eq!(w.b.phase, Phase::Closing, "nothing confirmed yet");
    // the broker refuses the cancel: it is sent again on the next check
    w.exit.as_mut().unwrap().fold.apply(&OrderEvent::CancelRefused { why: "try again".into() }).unwrap();
    w.tick();
    assert_eq!(w.exit.as_ref().unwrap().fold.state, OrderState::Cancelling);
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.tick();
    assert_eq!(w.b.phase, Phase::Ended);
}

#[test]
fn a_sale_from_the_ticket_clears_the_stop_first_and_a_sale_that_does_not_go_out_puts_it_back() {
    let mut w = armed(World::stop("95"), None, true);
    w.sale_running = true;
    w.b.apply(w.now, &BracketEvent::SaleAsked { quantity: d("10") }).unwrap();
    w.tick();
    assert_eq!(w.cancels.len(), 1);
    assert!(!w.b.clear_for_sale(), "not before the cancel is confirmed");
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    assert!(w.b.clear_for_sale());
    w.b.apply(w.now, &BracketEvent::SaleDropped { why: "refused".into() }).unwrap();
    w.tick();
    assert_eq!((w.b.phase, w.role()), (Phase::Guarding, Some(ExitRole::Stop)), "never left with neither a stop nor a sale");
}

#[test]
fn a_part_sold_from_the_ticket_leaves_the_stop_on_the_rest() {
    let mut w = armed(World::stop("95"), None, true);
    w.sale_running = true;
    w.b.apply(w.now, &BracketEvent::SaleAsked { quantity: d("4") }).unwrap();
    w.tick();
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.b.apply(w.now, &BracketEvent::Sold { quantity: d("4") }).unwrap();
    w.tick();
    assert_eq!(w.placed.last().unwrap(), &(ExitRole::Stop, Some(d("95")), d("6")));
}

#[test]
fn a_market_sell_rejected_after_it_was_accepted_puts_the_stop_back() {
    let mut w = armed(World::stop("95"), None, false);
    w.quote("94");
    w.tick();
    assert_eq!(w.b.phase, Phase::Firing);
    w.broker(BrokerStatus::Rejected, "0");
    w.quote("96");
    w.tick();
    w.tick();
    assert_eq!(w.b.phase, Phase::Guarding, "out of firing, the stop watched again");
}

#[test]
fn a_part_filled_exit_that_expires_at_the_close_is_placed_again_for_the_rest() {
    let mut w = armed(World::stop("95"), None, true);
    w.broker(BrokerStatus::Open, "3");
    w.broker(BrokerStatus::Expired, "3");
    w.tick();
    w.tick();
    assert_eq!(w.placed.last().unwrap(), &(ExitRole::Stop, Some(d("95")), d("7")));
}

#[test]
fn a_level_or_quantity_changed_by_hand_at_the_broker_is_followed_and_the_persons_own_edit_is_placed() {
    let mut w = armed(World::stop("95"), None, true);
    w.exit.as_mut().unwrap().price = Some(d("93"));
    w.tick();
    assert_eq!(w.b.stop.unwrap().level, d("93"), "followed, not cancelled");
    assert!(w.cancels.is_empty());
    // the person moves it from the panel: the resting stop is placed again there
    let step = bracket::adjust(w.exit.as_ref(), World::stop("97"), None);
    for e in &step.events {
        w.b.apply(w.now, e).unwrap();
    }
    w.tick();
    assert_eq!(w.cancels.len(), 1);
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.tick();
    assert_eq!(w.placed.last().unwrap().1, Some(d("97")));
}

#[test]
fn a_resting_exit_near_the_brokers_ninety_days_is_placed_again_at_the_same_level() {
    let mut w = armed(World::stop("95"), None, true);
    w.later(83 * 86400 + 1);
    w.open = true;
    w.tick();
    assert!(w.cancels.is_empty(), "under seven days left but the market is open: not yet");
    w.open = false;
    w.tick();
    assert_eq!(w.cancels.len(), 1);
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.tick();
    assert_eq!(w.placed.last().unwrap().1, Some(d("95")));
}

#[test]
fn a_position_closed_elsewhere_ends_only_a_bracket_with_nothing_resting() {
    let mut w = armed(World::stop("95"), None, false);
    w.closed_elsewhere = Some("sold: 10 shares in the activity feed".into());
    w.tick();
    assert_eq!(w.b.phase, Phase::Ended);
}

/// A small generator with a fixed seed: the same run every time.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

#[test]
fn under_any_run_of_broker_answers_and_prices_no_exit_is_placed_while_one_is_in_flight_and_every_tick_settles() {
    let mut rng = Lcg(7);
    for run in 0..400 {
        let trail = rng.next(2) == 0;
        let stop = Some(StopLeg { level: d("95"), trail: trail.then(|| Trail::Pct(d("5"))), high: None });
        let target = if rng.next(3) > 0 { Some("110") } else { None };
        let mut w = World::new(stop, target, rng.next(4) > 0);
        w.fill_entry("10", "100");
        for _ in 0..120 {
            w.answer = match rng.next(10) {
                0 => Answer::Refuse("refused", None),
                1 => Answer::Lose,
                _ => Answer::Accept,
            };
            let bid = 90 + rng.next(25);
            w.quote(&format!("{bid}.{}", rng.next(100)));
            w.open = rng.next(5) > 0;
            if w.exit.is_some() {
                let x = w.exit.as_ref().unwrap();
                let (state, filled) = (x.fold.state, x.fold.filled);
                match rng.next(12) {
                    0 if state.in_flight() => w.broker(BrokerStatus::Cancelled, &filled.to_text()),
                    1 if state.in_flight() => w.broker(BrokerStatus::Filled, &w.b.quantity.to_text()),
                    2 if state.in_flight() => w.broker(BrokerStatus::Expired, &filled.to_text()),
                    3 if state.in_flight() => w.broker(BrokerStatus::Rejected, &filled.to_text()),
                    4 if matches!(state, OrderState::Sending | OrderState::Unconfirmed) => w.broker(BrokerStatus::NotFound, "0"),
                    5 if matches!(state, OrderState::Sending | OrderState::Unconfirmed) => w.broker(BrokerStatus::Open, "0"),
                    6 if state == OrderState::Cancelling => {
                        w.exit.as_mut().unwrap().fold.apply(&OrderEvent::CancelRefused { why: "no".into() }).unwrap();
                    }
                    _ => {}
                }
            }
            w.later(5 + rng.next(400) as i64);
            w.tick();
            // the stop is never left neither resting nor watched while the bracket guards
            // (a refusal this very tick is answered at the next)
            if w.b.phase == Phase::Guarding && w.b.native && !w.b.off_broker && w.b.stop.is_some() && w.exit.is_none() && w.b.may_send(ExitRole::Stop, &w.key) && w.b.may_send(ExitRole::Market, &w.key) && w.tape.is_some() && w.b.refused.as_ref().is_none_or(|r| r.at < w.now) {
                panic!("run {run}: guarding with nothing resting and nothing placed: {:?}", w.b);
            }
            if w.b.phase == Phase::Ended {
                break;
            }
        }
    }
}

#[test]
fn a_position_is_gone_only_on_a_sale_of_every_share_or_two_statements_without_it_after_one_with_it() {
    use bagholder_core::bracket::closed_by_reads;
    let t = |s: &str| -> Timestamp { s.parse().unwrap() };
    let mut w = armed(World::stop("95"), None, false);
    assert!(closed_by_reads(&w.b, d("10"), None).0.unwrap().starts_with("sold: 10"));
    assert_eq!(closed_by_reads(&w.b, d("4"), None), (None, None), "a part sold is not the position gone");
    // not seen held yet: a statement without it says nothing
    assert_eq!(closed_by_reads(&w.b, Dec::ZERO, Some((t("2026-10-02T00:00:00Z"), false))), (None, None));
    let (why, e) = closed_by_reads(&w.b, Dec::ZERO, Some((t("2026-10-02T00:00:00Z"), true)));
    assert!(why.is_none());
    w.b.apply(w.now, &e.unwrap()).unwrap();
    // the first statement without it: noted, never an end
    let (why, e) = closed_by_reads(&w.b, Dec::ZERO, Some((t("2026-10-03T00:00:00Z"), false)));
    assert!(why.is_none());
    w.b.apply(w.now, &e.unwrap()).unwrap();
    // the same statement again is still one
    assert_eq!(closed_by_reads(&w.b, Dec::ZERO, Some((t("2026-10-03T00:00:00Z"), false))), (None, None));
    // a second, later one without it: gone
    assert!(closed_by_reads(&w.b, Dec::ZERO, Some((t("2026-10-04T00:00:00Z"), false))).0.unwrap().starts_with("position gone: two reads"));
    // a statement listing it again forgets the miss
    let (_, e) = closed_by_reads(&w.b, Dec::ZERO, Some((t("2026-10-04T00:00:00Z"), true)));
    w.b.apply(w.now, &e.unwrap()).unwrap();
    assert_eq!(w.b.missed_at, None);
}

impl World {
    /// The ticket's sale went out for `qty` of the bracket's shares: the bracket holds it.
    fn sale_sent(&mut self, qty: &str) {
        let mut fold = OrderFold::of(&[OrderEvent::Written { dry: false }]).unwrap();
        fold.apply(&OrderEvent::Accepted { broker_id: "sale".into() }).unwrap();
        self.b.apply(self.now, &BracketEvent::SaleSent { order_id: "sale".into(), quantity: d(qty) }).unwrap();
        self.exit = Some(Exit { order_id: "sale".into(), role: ExitRole::Sale, fold, cancel_asked: false, cancel_asked_at: None, price: Some(d("101")), quantity: Some(d(qty)), expires_at: None });
    }

    fn entry_reads(&mut self, status: BrokerStatus, filled: &str) {
        self.entry.apply(&OrderEvent::Read(Reading::of(status, d(filled), Some(d("100"))))).unwrap();
    }
}

#[test]
fn a_halted_bracket_sends_nothing_more_but_still_sells_when_its_stop_level_is_reached() {
    let mut w = armed(World::stop("95"), None, false);
    w.b.apply(w.now, &BracketEvent::Halted { why: "cap".into() }).unwrap();
    w.quote("97");
    w.tick();
    let before = w.placed.len();
    w.quote("94.50");
    w.tick();
    assert_eq!((w.placed.len(), w.placed.last().unwrap().0), (before + 1, ExitRole::Market));
}

#[test]
fn a_sale_from_the_ticket_this_run_is_not_sending_gives_the_bracket_back_its_stop() {
    // the app stopped between asking for the sale and sending it
    let mut w = armed(World::stop("95"), None, true);
    w.b.apply(w.now, &BracketEvent::SaleAsked { quantity: d("10") }).unwrap();
    w.sale_running = false;
    w.tick();
    assert_eq!((w.b.phase, w.role()), (Phase::Guarding, Some(ExitRole::Stop)), "on the first check after a start, its stop still resting");

    // the stop's cancel was confirmed before the app stopped: the stop is placed again
    let mut w = armed(World::stop("95"), None, true);
    w.sale_running = true;
    w.b.apply(w.now, &BracketEvent::SaleAsked { quantity: d("10") }).unwrap();
    w.tick();
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.sale_running = false;
    w.tick();
    w.tick();
    assert_eq!((w.b.phase, w.placed.last().cloned()), (Phase::Guarding, Some((ExitRole::Stop, Some(d("95")), d("10")))));
}

#[test]
fn a_ticket_sale_ends_the_bracket_only_when_it_fills() {
    let mut w = armed(World::stop("95"), None, true);
    w.sale_running = true;
    w.b.apply(w.now, &BracketEvent::SaleAsked { quantity: d("10") }).unwrap();
    w.tick();
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.sale_sent("10");
    w.sale_running = false;
    w.tick();
    assert_eq!(w.b.phase, Phase::Selling, "accepted is not sold");
    w.broker(BrokerStatus::Filled, "10");
    w.tick();
    assert_eq!((w.b.phase, w.b.outcome.as_deref()), (Phase::Ended, Some("sold from the ticket")));
}

#[test]
fn a_ticket_sale_that_expires_or_fills_in_part_puts_the_stop_back_on_what_is_left() {
    for (status, filled, left) in [(BrokerStatus::Expired, "0", "10"), (BrokerStatus::Cancelled, "0", "10"), (BrokerStatus::Expired, "4", "6")] {
        let mut w = armed(World::stop("95"), None, true);
        w.sale_running = true;
        w.b.apply(w.now, &BracketEvent::SaleAsked { quantity: d("10") }).unwrap();
        w.tick();
        w.broker(BrokerStatus::Cancelled, "0");
        w.tick();
        w.sale_sent("10");
        w.sale_running = false;
        if filled != "0" {
            w.broker(BrokerStatus::Open, filled);
        }
        w.broker(status, filled);
        w.tick();
        w.tick();
        assert_eq!((w.b.phase, w.placed.last().cloned()), (Phase::Guarding, Some((ExitRole::Stop, Some(d("95")), d(left)))), "{status:?} with {filled} sold");
    }
}

#[test]
fn the_stop_level_reached_while_a_ticket_sale_rests_cancels_it_and_sells_at_market() {
    let mut w = armed(World::stop("95"), None, true);
    w.sale_running = true;
    w.b.apply(w.now, &BracketEvent::SaleAsked { quantity: d("10") }).unwrap();
    w.tick();
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    w.sale_sent("10");
    w.sale_running = false;
    w.quote("94");
    w.tick();
    assert_eq!((w.b.phase, w.cancels.last().map(String::as_str)), (Phase::ToMarket, Some("sale")));
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    assert_eq!(w.placed.last().unwrap(), &(ExitRole::Market, None, d("10")));
}

#[test]
fn an_entry_filling_in_parts_arms_on_its_first_fill_and_the_stop_follows_by_a_change_in_place() {
    let mut w = World::new(World::stop("95"), None, true);
    w.entry_reads(BrokerStatus::Open, "3");
    w.tick();
    w.tick();
    assert_eq!((w.b.phase, w.placed.last().cloned()), (Phase::Guarding, Some((ExitRole::Stop, Some(d("95")), d("3")))), "armed for what filled");
    w.entry_reads(BrokerStatus::Open, "5");
    w.tick();
    w.tick();
    assert_eq!((w.b.quantity, w.resizes.clone()), (d("5"), vec![d("5")]), "grown, and the stop changed in place, not cancelled");
    assert!(w.cancels.is_empty());
    // more fills inside the rest: one change for them, after it
    w.entry_reads(BrokerStatus::Open, "6");
    w.tick();
    w.entry_reads(BrokerStatus::Open, "8");
    w.tick();
    assert_eq!(w.resizes.len(), 1, "coalesced within the rest");
    w.later(bracket::RESIZE_SECONDS);
    w.tick();
    assert_eq!(w.resizes.last(), Some(&d("8")));
    // the entry ends: its quantity is final and the stop matches it at once
    w.entry_reads(BrokerStatus::Filled, "10");
    w.tick();
    w.tick();
    assert_eq!((w.b.quantity, w.resizes.last()), (d("10"), Some(&d("10"))));
}

#[test]
fn a_stop_that_fires_while_the_entry_still_works_cancels_what_is_left_of_the_entry() {
    let mut w = World::new(World::stop("95"), None, false);
    w.entry_reads(BrokerStatus::Open, "3");
    w.tick();
    w.quote("94");
    w.tick();
    assert_eq!(w.placed.last().unwrap().0, ExitRole::Market);
    w.tick();
    assert_eq!(w.cancels.last().map(String::as_str), Some("entry"), "nothing more is bought behind a stop that has sold");
}

#[test]
fn a_cancel_the_broker_has_not_confirmed_is_asked_again_after_its_rest() {
    let mut w = armed(World::stop("95"), None, true);
    w.b.apply(w.now, &BracketEvent::Ended { outcome: "cancelled by the user".into() }).unwrap();
    w.tick();
    assert_eq!(w.cancels.len(), 1);
    w.broker(BrokerStatus::Open, "0");
    w.later(bracket::CANCEL_REASK_SECONDS - 1);
    w.tick();
    assert_eq!(w.cancels.len(), 1, "not before its rest");
    w.later(1);
    w.tick();
    assert_eq!(w.cancels.len(), 2, "asked again");
}
