//! The bracket's rules (`SPEC.md` §6, Brackets) against a fake broker that
//! answers, refuses, loses answers and fills in parts. Every send is checked
//! against the invariant that nothing is placed while an exit is in flight.

use bagholder_core::bracket::{self, Bracket, BracketEvent, Exit, ExitRole, Phase, Request, Seen, StopLeg, Tape, Trail};
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
    closed_elsewhere: Option<String>,
}

impl World {
    fn new(stop: Option<StopLeg>, target: Option<&str>, native: bool) -> World {
        let now: Timestamp = "2026-09-28T15:00:00Z".parse().unwrap();
        let mut b = Bracket::of(&[(now, BracketEvent::Created { quantity: d("10"), stop, target: target.map(d) })]).unwrap();
        b.native = native;
        let entry = OrderFold::of(&[OrderEvent::Written { dry: false }, OrderEvent::Accepted { broker_id: "entry".into() }]).unwrap();
        World { b, now, open: true, tape: None, entry, exit: None, next: 0, answer: Answer::Accept, placed: vec![], cancels: vec![], closed_elsewhere: None }
    }

    fn stop(level: &str) -> Option<StopLeg> {
        Some(StopLeg { level: d(level), trail: None, high: None })
    }

    fn quote(&mut self, bid: &str) {
        self.tape = Some(Tape { last: d(bid), bid: Some(d(bid)) });
    }

    fn fill_entry(&mut self, qty: &str, avg: &str) {
        self.entry.apply(&OrderEvent::Read(Reading { status: BrokerStatus::Filled, filled: d(qty), average: Some(d(avg)) })).unwrap();
    }

    fn later(&mut self, secs: i64) {
        self.now = self.now + SignedDuration::from_secs(secs);
    }

    /// One tick: steps until nothing, or until a request went out.
    fn tick(&mut self) {
        for _ in 0..16 {
            let step = {
                let seen = Seen { now: self.now, open: self.open, tape: self.tape, entry: Some(&self.entry), exit: self.exit.as_ref(), closed_elsewhere: self.closed_elsewhere.clone() };
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
                        self.b.apply(self.now, &BracketEvent::Refused { why: why.into(), code: code.map(String::from) }).unwrap();
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
                self.exit = Some(Exit { order_id: id, role, fold, cancel_asked: false, price, quantity: Some(quantity), expires_at: Some(self.now + SignedDuration::from_hours(24 * 90)) });
            }
            Request::Cancel { order_id } => self.cancels.push(order_id),
        }
    }

    fn broker(&mut self, status: BrokerStatus, filled: &str) {
        let x = self.exit.as_mut().expect("an exit at the broker");
        x.fold.apply(&OrderEvent::Read(Reading { status, filled: d(filled), average: Some(d("1")) })).unwrap();
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
    w.entry.apply(&OrderEvent::Read(Reading { status: BrokerStatus::Cancelled, filled: Dec::ZERO, average: None })).unwrap();
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
fn a_refused_exit_is_tried_again_after_a_minute_five_fifteen_then_hourly() {
    let mut w = World::new(World::stop("95"), None, true);
    w.fill_entry("10", "100");
    w.answer = Answer::Refuse("the market is closed", None);
    w.tick();
    w.tick();
    assert_eq!((w.placed.len(), w.b.attempts), (1, 1));
    for (wait, n) in [(59, 1), (1, 2), (299, 2), (1, 3), (899, 3), (1, 4), (3599, 4), (1, 5), (3600, 6)] {
        w.later(wait);
        w.tick();
        assert_eq!(w.placed.len(), n, "after {wait} s more");
    }
    w.answer = Answer::Accept;
    w.later(3600);
    w.tick();
    assert_eq!((w.b.attempts, w.role()), (0, Some(ExitRole::Stop)));
}

#[test]
fn a_refusal_for_shares_that_are_not_there_ends_the_bracket() {
    let mut w = World::new(World::stop("95"), None, true);
    w.fill_entry("10", "100");
    w.answer = Answer::Refuse("not enough shares", Some("NOT_ENOUGH_SHARES"));
    w.tick();
    w.tick();
    assert_eq!((w.b.phase, w.b.outcome.as_deref()), (Phase::Ended, Some("the shares are not there")));
}

#[test]
fn an_exit_cancelled_at_the_broker_by_hand_ends_the_bracket() {
    let mut w = armed(World::stop("95"), None, true);
    w.broker(BrokerStatus::Cancelled, "0");
    w.tick();
    assert_eq!((w.b.phase, w.b.outcome.as_deref()), (Phase::Ended, Some("stop cancelled at Wealthsimple by hand")));
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
            if w.b.phase == Phase::Guarding && w.b.native && w.b.stop.is_some() && w.exit.is_none() && w.b.may_retry(w.now) && w.tape.is_some() {
                panic!("run {run}: guarding with nothing resting and nothing placed: {:?}", w.b);
            }
            if w.b.phase == Phase::Ended {
                break;
            }
        }
    }
}
