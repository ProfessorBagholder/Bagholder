//! An order's life as a state machine (`docs/architecture.md` §11,
//! `docs/plans/stage-4-execution.md`): its states, what can happen to it, and the
//! only moves allowed. An order's state is the fold of what happened to it, in the
//! order it was recorded; nothing sets a state directly.
//!
//! A move happens only on the broker's own answer, a read-back of the order, or a
//! definite failure before anything left the app. An answer that is not the
//! broker's (a timeout, a dropped connection, a reply that cannot be read) leaves a
//! sent order `Unconfirmed` until a read-back settles it (FIX's order status
//! request; *Order State Changes*, scenario G.1). A cancel asked for is `Cancelling`
//! until the broker settles it, and a fill read meanwhile wins (scenario B.1.c).
//! A read older than what is already known never moves the state back.

use crate::Dec;

text_enum! {
    /// Where an order stands.
    OrderState "order state" {
        /// Written and not sent: orders are off.
        Dry = "dry",
        /// Written; the request is leaving the app. A crash leaves this behind.
        Sending = "sending",
        /// Sent, and the answer was not the broker's: the read-back settles it.
        Unconfirmed = "unconfirmed",
        /// The broker has it, nothing filled.
        Pending = "pending",
        /// The broker has it, part filled, the rest working.
        PartlyFilled = "partly-filled",
        Filled = "filled",
        /// A cancel was asked for and the broker has not settled it.
        Cancelling = "cancelling",
        Cancelled = "cancelled",
        Expired = "expired",
        /// Refused by the broker.
        Rejected = "rejected",
        /// Failed before anything left the app, or the broker has no record of it.
        Failed = "failed",
    }
}

impl OrderState {
    /// With the broker, or possibly with it: it can fill, and nothing more may be
    /// sent for what it stands for until it is settled.
    pub fn in_flight(self) -> bool {
        matches!(self, OrderState::Sending | OrderState::Unconfirmed | OrderState::Pending | OrderState::PartlyFilled | OrderState::Cancelling)
    }

    /// Nothing more will happen to it at the broker.
    pub fn is_final(self) -> bool {
        !self.in_flight()
    }
}

text_enum! {
    /// Which way an order trades.
    Side "side" { Buy = "buy", Sell = "sell" }
}

text_enum! {
    /// How an order is priced.
    OrderKind "order type" { Market = "market", Limit = "limit", Stop = "stop", StopLimit = "stop-limit" }
}

text_enum! {
    /// How long an order works: the day, or until cancelled (the broker's own limit applies).
    TimeInForce "time in force" { Day = "day", UntilCancel = "until-cancel" }
}

text_enum! {
    /// What an order is to its bracket: the entry, or one of its exits.
    OrderRole "order role" { Entry = "entry", Stop = "stop", Target = "target", Market = "market" }
}

/// Who asked for something to be done to an order or a bracket.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Asker {
    /// The person, from the page.
    Person,
    /// Bagholder's own bracket engine.
    Engine,
    /// An agent the person authorised, by the name it was given.
    Agent(String),
}

impl Asker {
    pub fn to_text(&self) -> String {
        match self {
            Asker::Person => "person".into(),
            Asker::Engine => "engine".into(),
            Asker::Agent(name) => format!("agent:{name}"),
        }
    }

    pub fn parse(s: &str) -> Result<Asker, crate::text_enum::UnknownWord> {
        match s {
            "person" => Ok(Asker::Person),
            "engine" => Ok(Asker::Engine),
            _ => match s.strip_prefix("agent:") {
                Some(name) if !name.is_empty() => Ok(Asker::Agent(name.to_string())),
                _ => Err(crate::text_enum::UnknownWord { what: "asker", word: s.to_string() }),
            },
        }
    }
}

/// What the broker says of an order when it is read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrokerStatus {
    /// Working: nothing filled, or part filled with the rest working.
    Open,
    Filled,
    Cancelled,
    Expired,
    Rejected,
    /// The broker has no order by the app's id.
    NotFound,
}

/// A read-back of an order: its status and how much has filled, at what average,
/// and the price, quantity and expiry the broker states for it now.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub status: BrokerStatus,
    pub filled: Dec,
    pub average: Option<Dec>,
    pub price: Option<Dec>,
    pub quantity: Option<Dec>,
    pub expires_at: Option<jiff::Timestamp>,
}

impl Reading {
    /// A reading of status and fill alone.
    pub fn of(status: BrokerStatus, filled: Dec, average: Option<Dec>) -> Reading {
        Reading { status, filled, average, price: None, quantity: None, expires_at: None }
    }
}

/// One thing that happened to an order.
#[derive(Clone, Debug, PartialEq)]
pub enum OrderEvent {
    /// The order was written, before anything was sent: `dry` when orders are off.
    Written { dry: bool },
    /// It failed before anything left the app (no session, the gate refused).
    NotSent { why: String },
    /// The broker's own answer: it has the order, under its id.
    Accepted { broker_id: String },
    /// The broker's own answer: it refused the order.
    Refused { why: String, code: Option<String> },
    /// Sent, and what came back was not the broker's answer.
    Unclear { why: String },
    /// A read-back of the order.
    Read(Reading),
    /// A cancel was asked for and sent.
    CancelAsked,
    /// The broker refused the cancel.
    CancelRefused { why: String },
}

impl OrderEvent {
    /// The word the event is kept under.
    pub fn kind(&self) -> &'static str {
        match self {
            OrderEvent::Written { .. } => "written",
            OrderEvent::NotSent { .. } => "not-sent",
            OrderEvent::Accepted { .. } => "accepted",
            OrderEvent::Refused { .. } => "refused",
            OrderEvent::Unclear { .. } => "unclear",
            OrderEvent::Read(_) => "read",
            OrderEvent::CancelAsked => "cancel-asked",
            OrderEvent::CancelRefused { .. } => "cancel-refused",
        }
    }
}

/// An order as its events so far make it.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderFold {
    pub state: OrderState,
    /// The broker's own id, once it has said.
    pub broker_id: Option<String>,
    /// How much has filled, by the broker's newest reading; it only rises.
    pub filled: Dec,
    pub average: Option<Dec>,
    /// Why it was refused, failed or is unconfirmed, in the words recorded.
    pub why: Option<String>,
    pub code: Option<String>,
}

/// What applying an event did.
#[derive(Clone, Debug, PartialEq)]
pub enum Applied {
    /// The state moved.
    Moved { from: OrderState, to: OrderState },
    /// The state stayed; what is known of the order changed (a fill quantity, an id).
    Updated,
    /// The event told nothing new (a read older than what is known).
    Nothing,
}

/// Why an event was not applied: the state it met, and the event.
#[derive(Clone, Debug, PartialEq)]
pub struct NotAllowed {
    pub state: OrderState,
    pub event: &'static str,
    pub why: String,
}

impl std::fmt::Display for NotAllowed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not a move from {}: {}", self.event, self.state.as_str(), self.why)
    }
}

impl OrderFold {
    /// The order as its first event makes it: only `Written` can start one.
    pub fn start(first: &OrderEvent) -> Result<OrderFold, NotAllowed> {
        match first {
            OrderEvent::Written { dry } => Ok(OrderFold {
                state: if *dry { OrderState::Dry } else { OrderState::Sending },
                broker_id: None,
                filled: Dec::ZERO,
                average: None,
                why: None,
                code: None,
            }),
            other => Err(NotAllowed { state: OrderState::Sending, event: other.kind(), why: "an order starts by being written".into() }),
        }
    }

    /// The fold of a whole log. An event that is not allowed where it falls is
    /// skipped, as it was when it was recorded.
    pub fn of(events: &[OrderEvent]) -> Option<OrderFold> {
        let (first, rest) = events.split_first()?;
        let mut fold = OrderFold::start(first).ok()?;
        for e in rest {
            let _ = fold.apply(e);
        }
        Some(fold)
    }

    /// The state that working at the broker means, with what has filled.
    fn working(&self) -> OrderState {
        if self.filled.is_positive() {
            OrderState::PartlyFilled
        } else {
            OrderState::Pending
        }
    }

    fn to(&mut self, to: OrderState) -> Applied {
        let from = self.state;
        self.state = to;
        if from == to {
            Applied::Updated
        } else {
            Applied::Moved { from, to }
        }
    }

    fn refuse(&self, event: &OrderEvent, why: &str) -> NotAllowed {
        NotAllowed { state: self.state, event: event.kind(), why: why.into() }
    }

    /// Apply one event: the move it makes, or why it is not one. A refused event
    /// changes nothing.
    pub fn apply(&mut self, event: &OrderEvent) -> Result<Applied, NotAllowed> {
        use OrderState::*;
        match event {
            OrderEvent::Written { .. } => Err(self.refuse(event, "an order is written once")),
            OrderEvent::NotSent { why } => match self.state {
                Sending => {
                    self.why = Some(why.clone());
                    Ok(self.to(Failed))
                }
                _ => Err(self.refuse(event, "only an order still leaving the app can fail before it left")),
            },
            OrderEvent::Accepted { broker_id } => match self.state {
                Sending | Unconfirmed => {
                    self.broker_id = Some(broker_id.clone());
                    self.why = None;
                    let to = self.working();
                    Ok(self.to(to))
                }
                // the same answer heard twice
                _ if self.broker_id.as_deref() == Some(broker_id.as_str()) => Ok(Applied::Nothing),
                // an id learnt from a read-back first, the answer arriving after
                Pending | PartlyFilled | Cancelling | Filled | Cancelled | Expired if self.broker_id.is_none() => {
                    self.broker_id = Some(broker_id.clone());
                    Ok(Applied::Updated)
                }
                _ => Err(self.refuse(event, "the broker already answered this order")),
            },
            OrderEvent::Refused { why, code } => match self.state {
                Sending | Unconfirmed => {
                    self.why = Some(why.clone());
                    self.code = code.clone();
                    Ok(self.to(Rejected))
                }
                _ => Err(self.refuse(event, "the broker already answered this order")),
            },
            OrderEvent::Unclear { why } => match self.state {
                Sending => {
                    self.why = Some(why.clone());
                    Ok(self.to(Unconfirmed))
                }
                // a second unclear answer changes nothing: the read-back settles it
                Unconfirmed => {
                    self.why = Some(why.clone());
                    Ok(Applied::Updated)
                }
                _ => Err(self.refuse(event, "the order was already answered")),
            },
            OrderEvent::CancelAsked => match self.state {
                Pending | PartlyFilled => Ok(self.to(Cancelling)),
                _ => Err(self.refuse(event, "only a working order can be cancelled")),
            },
            OrderEvent::CancelRefused { why } => match self.state {
                Cancelling => {
                    self.why = Some(why.clone());
                    let to = self.working();
                    Ok(self.to(to))
                }
                _ => Err(self.refuse(event, "no cancel is outstanding")),
            },
            OrderEvent::Read(r) => self.read(event, r),
        }
    }

    fn read(&mut self, event: &OrderEvent, r: &Reading) -> Result<Applied, NotAllowed> {
        use OrderState::*;
        if r.filled.is_negative() {
            return Err(self.refuse(event, "a negative filled quantity"));
        }
        if r.status == BrokerStatus::Filled && !r.filled.is_positive() {
            return Err(self.refuse(event, "filled with nothing filled"));
        }
        // what has filled never goes down: a lower reading is recorded, never applied
        if r.filled < self.filled {
            return Err(self.refuse(event, &format!("the broker's filled quantity went down, from {} to {}", self.filled, r.filled)));
        }
        let more = r.filled > self.filled;
        let settle = |f: &mut OrderFold| {
            if more {
                f.filled = r.filled;
                f.average = r.average;
            }
        };
        match (self.state, r.status) {
            (Dry, _) => Err(self.refuse(event, "a dry order was never sent")),
            (_, BrokerStatus::NotFound) => match self.state {
                // the create never reached the broker
                Sending | Unconfirmed => {
                    self.why = Some("the broker has no record of it".into());
                    Ok(self.to(Failed))
                }
                _ => Err(self.refuse(event, "the broker no longer finds an order it had")),
            },
            // a read that is not newer than a final state tells nothing, but a fill
            // read late on an ended order is booked
            (Filled | Cancelled | Expired | Rejected | Failed, s) => {
                let same = matches!((self.state, s), (Filled, BrokerStatus::Filled) | (Cancelled, BrokerStatus::Cancelled) | (Expired, BrokerStatus::Expired) | (Rejected, BrokerStatus::Rejected));
                // the fill wins over a cancel or an expiry it raced
                let fill_won = matches!((self.state, s), (Cancelled | Expired, BrokerStatus::Filled));
                if fill_won {
                    settle(self);
                    return Ok(self.to(Filled));
                }
                if (same || s == BrokerStatus::Open) && more && self.state != Failed && self.state != Rejected {
                    settle(self);
                    return Ok(Applied::Updated);
                }
                if same || s == BrokerStatus::Open {
                    return Ok(Applied::Nothing);
                }
                Err(self.refuse(event, "an ended order does not end again another way"))
            }
            (Sending | Unconfirmed | Pending | PartlyFilled | Cancelling, s) => {
                settle(self);
                let to = match s {
                    // a cancel outstanding stays so until the broker settles it
                    BrokerStatus::Open if self.state == Cancelling => Cancelling,
                    BrokerStatus::Open => self.working(),
                    BrokerStatus::Filled => Filled,
                    BrokerStatus::Cancelled => Cancelled,
                    BrokerStatus::Expired => Expired,
                    BrokerStatus::Rejected => Rejected,
                    BrokerStatus::NotFound => unreachable!("handled above"),
                };
                if matches!(self.state, Sending | Unconfirmed) {
                    self.why = None;
                }
                let applied = self.to(to);
                Ok(if applied == Applied::Updated && !more { Applied::Nothing } else { applied })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use OrderState::*;

    fn d(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }

    fn read(status: BrokerStatus, filled: &str) -> OrderEvent {
        OrderEvent::Read(Reading::of(status, d(filled), if filled == "0" { None } else { Some(d("10")) }))
    }

    /// An order brought to `state` by the shortest path of allowed events.
    fn at(state: OrderState) -> OrderFold {
        let sent = OrderEvent::Written { dry: false };
        let path: Vec<OrderEvent> = match state {
            Dry => vec![OrderEvent::Written { dry: true }],
            Sending => vec![sent],
            Unconfirmed => vec![sent, OrderEvent::Unclear { why: "timed out".into() }],
            Pending => vec![sent, OrderEvent::Accepted { broker_id: "o-1".into() }],
            PartlyFilled => vec![sent, OrderEvent::Accepted { broker_id: "o-1".into() }, read(BrokerStatus::Open, "4")],
            Filled => vec![sent, OrderEvent::Accepted { broker_id: "o-1".into() }, read(BrokerStatus::Filled, "10")],
            Cancelling => vec![sent, OrderEvent::Accepted { broker_id: "o-1".into() }, OrderEvent::CancelAsked],
            Cancelled => vec![sent, OrderEvent::Accepted { broker_id: "o-1".into() }, read(BrokerStatus::Cancelled, "0")],
            Expired => vec![sent, OrderEvent::Accepted { broker_id: "o-1".into() }, read(BrokerStatus::Expired, "0")],
            Rejected => vec![sent, OrderEvent::Refused { why: "no".into(), code: None }],
            Failed => vec![sent, OrderEvent::NotSent { why: "no session".into() }],
        };
        let f = OrderFold::of(&path).unwrap();
        assert_eq!(f.state, state, "the path to {state:?}");
        f
    }

    fn every_event() -> Vec<OrderEvent> {
        let mut v = vec![
            OrderEvent::Written { dry: false },
            OrderEvent::NotSent { why: "x".into() },
            OrderEvent::Accepted { broker_id: "o-2".into() },
            OrderEvent::Refused { why: "x".into(), code: None },
            OrderEvent::Unclear { why: "x".into() },
            OrderEvent::CancelAsked,
            OrderEvent::CancelRefused { why: "x".into() },
        ];
        for s in [BrokerStatus::Open, BrokerStatus::Filled, BrokerStatus::Cancelled, BrokerStatus::Expired, BrokerStatus::Rejected, BrokerStatus::NotFound] {
            v.push(read(s, "0"));
            v.push(read(s, "6"));
        }
        v
    }

    /// The table of moves, written out: (from, event, to). Anything not here from
    /// a state either tells nothing, only updates what is known, or is refused.
    fn allowed(from: OrderState, e: &OrderEvent) -> Option<OrderState> {
        use BrokerStatus as B;
        let r = |e: &OrderEvent| match e {
            OrderEvent::Read(r) => Some((r.status, r.filled.is_positive())),
            _ => None,
        };
        // what `at` has filled in each state; a reading below it is refused wherever it falls
        let known = match from {
            PartlyFilled => d("4"),
            Filled => d("10"),
            _ => Dec::ZERO,
        };
        if matches!(e, OrderEvent::Read(r) if r.filled < known) {
            return None;
        }
        match (from, e) {
            (Sending, OrderEvent::NotSent { .. }) => Some(Failed),
            (Sending | Unconfirmed, OrderEvent::Accepted { .. }) => Some(Pending),
            (Sending | Unconfirmed, OrderEvent::Refused { .. }) => Some(Rejected),
            (Sending, OrderEvent::Unclear { .. }) => Some(Unconfirmed),
            (Pending | PartlyFilled, OrderEvent::CancelAsked) => Some(Cancelling),
            (Cancelling, OrderEvent::CancelRefused { .. }) => Some(Pending),
            (Sending | Unconfirmed | Pending | PartlyFilled | Cancelling, e) if r(e).is_some() => match r(e).unwrap() {
                (B::NotFound, _) if matches!(from, Sending | Unconfirmed) => Some(Failed),
                (B::NotFound, _) => None,
                (B::Open, _) if from == Cancelling => Some(Cancelling),
                (B::Open, true) => Some(PartlyFilled),
                (B::Open, false) if from == PartlyFilled => None,
                (B::Open, false) => Some(Pending),
                (B::Filled, false) => None,
                (B::Filled, true) => Some(Filled),
                (B::Cancelled, _) => Some(Cancelled),
                (B::Expired, _) => Some(Expired),
                (B::Rejected, _) => Some(Rejected),
            },
            (Cancelled | Expired, e) if matches!(r(e), Some((B::Filled, true))) => Some(Filled),
            _ => None,
        }
    }

    #[test]
    fn every_move_in_the_table_is_made_and_every_other_event_moves_nothing() {
        for &state in OrderState::ALL {
            for e in every_event() {
                let before = at(state);
                let mut f = before.clone();
                let got = f.apply(&e);
                match allowed(state, &e) {
                    Some(to) if to != state => {
                        assert_eq!(got, Ok(Applied::Moved { from: state, to }), "{state:?} on {e:?}");
                        assert_eq!(f.state, to);
                    }
                    _ => {
                        assert_eq!(f.state, state, "{state:?} does not move on {e:?} ({got:?})");
                        if got.is_err() {
                            assert_eq!(f, before, "a refused event changes nothing: {state:?} on {e:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn an_answer_that_is_not_the_brokers_leaves_the_order_unconfirmed_until_read_back() {
        let mut f = at(Sending);
        f.apply(&OrderEvent::Unclear { why: "the connection dropped".into() }).unwrap();
        assert_eq!(f.state, Unconfirmed);
        assert!(f.state.in_flight(), "nothing more is sent for it");
        f.apply(&read(BrokerStatus::Open, "0")).unwrap();
        assert_eq!(f.state, Pending, "the read-back finds it at the broker: never failed");
        let mut g = at(Unconfirmed);
        g.apply(&read(BrokerStatus::NotFound, "0")).unwrap();
        assert_eq!(g.state, Failed, "the broker has no record: it never reached it");
    }

    #[test]
    fn a_fill_read_after_a_cancel_was_asked_wins_and_a_cancel_stays_asked_while_the_order_works() {
        let mut f = at(Cancelling);
        assert_eq!(f.apply(&read(BrokerStatus::Open, "3")).unwrap(), Applied::Updated);
        assert_eq!((f.state, f.filled), (Cancelling, d("3")));
        f.apply(&read(BrokerStatus::Filled, "10")).unwrap();
        assert_eq!((f.state, f.filled), (Filled, d("10")));
        // a cancel read first, then the fill that beat it
        let mut g = at(Cancelled);
        g.apply(&read(BrokerStatus::Filled, "10")).unwrap();
        assert_eq!(g.state, Filled);
    }

    #[test]
    fn a_read_older_than_what_is_known_never_moves_the_state_back() {
        let mut f = at(Filled);
        assert_eq!(f.apply(&read(BrokerStatus::Open, "10")).unwrap(), Applied::Nothing);
        assert_eq!(f.state, Filled);
        let mut g = at(PartlyFilled);
        assert!(g.apply(&read(BrokerStatus::Open, "2")).is_err(), "less filled than known is refused");
        assert_eq!((g.state, g.filled), (PartlyFilled, d("4")));
    }

    #[test]
    fn the_filled_quantity_only_rises_and_a_late_fill_on_an_ended_order_is_kept() {
        let mut f = at(Cancelled);
        f.apply(&read(BrokerStatus::Cancelled, "3")).unwrap();
        assert_eq!((f.state, f.filled), (Cancelled, d("3")), "part filled, then cancelled");
        let e = f.apply(&read(BrokerStatus::Cancelled, "1")).unwrap_err();
        assert!(e.why.contains("went down"), "{e}");
        assert_eq!(f.filled, d("3"));
    }

    #[test]
    fn the_same_answer_heard_twice_tells_nothing() {
        let mut f = at(Pending);
        assert_eq!(f.apply(&OrderEvent::Accepted { broker_id: "o-1".into() }).unwrap(), Applied::Nothing);
        assert_eq!(f.apply(&read(BrokerStatus::Open, "0")).unwrap(), Applied::Nothing);
    }

    #[test]
    fn in_flight_is_every_state_the_broker_may_still_act_on() {
        let flying: Vec<_> = OrderState::ALL.iter().copied().filter(|s| s.in_flight()).collect();
        assert_eq!(flying, vec![Sending, Unconfirmed, Pending, PartlyFilled, Cancelling]);
    }

    #[test]
    fn who_asked_reads_back_as_written() {
        for a in [Asker::Person, Asker::Engine, Asker::Agent("claude".into())] {
            assert_eq!(Asker::parse(&a.to_text()).unwrap(), a);
        }
        assert!(Asker::parse("agent:").is_err());
        assert!(Asker::parse("someone").is_err());
    }
}
