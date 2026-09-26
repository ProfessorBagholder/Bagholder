//! A bracket's life as a state machine (`docs/architecture.md` §11,
//! `docs/plans/stage-4-execution.md`, `SPEC.md` §6 Brackets): the stop and the
//! target held for an entry, as phases with their allowed moves, and `decide`, the
//! one step the bracket takes from where it stands.
//!
//! The rules `SPEC.md` gives brackets are invariants here:
//! - at most one exit order is in flight for the bracket's shares (Wealthsimple holds
//!   one resting order per share), so nothing is placed while an exit is in flight;
//! - nothing is sent after a request whose answer is not yet known: an exit that is
//!   unconfirmed or being cancelled is waited for;
//! - while the bracket is live and no stop rests at the broker, the stop level is
//!   watched here and fires as a market sell;
//! - an exit that ends is cleared before anything else is decided, and a bracket that
//!   ends cancels what it has in flight and is `Closing` until nothing is.
//!
//! Nothing here reads a clock, a quote or the network: the caller hands in the time,
//! the quote, the entry and the exit as the broker last stated them, and sends the
//! request `decide` returns through the one gate.

use crate::order::{OrderFold, OrderState};
use crate::{Dec, Rounding};
use jiff::{SignedDuration, Timestamp};

text_enum! {
    /// Where a bracket stands.
    Phase "bracket phase" {
        /// The entry has not filled.
        Waiting = "waiting",
        /// Armed: the stop rests at the broker, or is watched here while none does.
        Guarding = "guarding",
        /// The target was reached: the stop's cancel is out, the target follows it.
        ToTarget = "to-target",
        /// The limit sell at the target rests (or is being placed); the stop is watched here.
        Target = "target",
        /// The price left the target: the limit's cancel is out, the stop follows it.
        BackToStop = "back-to-stop",
        /// The stop level was reached while the limit rested: its cancel is out, a market sell follows.
        ToMarket = "to-market",
        /// A market sell is out.
        Firing = "firing",
        /// A sale from the ticket: the exit's cancel is out, the sale follows it.
        ClosingForSale = "closing-for-sale",
        /// Ended: what it has in flight is being cancelled.
        Closing = "closing",
        /// Stopped for sending more orders than a bracket can: nothing is sent until the person acts.
        Halted = "halted",
        Ended = "ended",
    }
}

impl Phase {
    pub fn is_live(self) -> bool {
        self != Phase::Ended
    }
}

text_enum! {
    /// What an exit order is to its bracket.
    ExitRole "exit role" {
        /// A stop order resting at the broker.
        Stop = "stop",
        /// The limit sell at the target.
        Target = "target",
        /// A market sell: a watched stop fired.
        Market = "market",
    }
}

/// How a trailing stop trails the high.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trail {
    /// A percent of the high.
    Pct(Dec),
    /// An amount per share, in the listing's currency.
    Amount(Dec),
}

/// The stop leg: its level, and for a trailing stop how it trails and the high so far.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StopLeg {
    pub level: Dec,
    pub trail: Option<Trail>,
    pub high: Option<Dec>,
}

impl StopLeg {
    /// The level a trailing stop stands at under `high`, to the cent.
    pub fn trailed(trail: Trail, high: Dec) -> Dec {
        let distance = match trail {
            Trail::Pct(p) => high.checked_mul(p).and_then(|x| x.div_rounded(Dec::from_int(100), 10, Rounding::HalfEven)).unwrap_or(Dec::ZERO),
            Trail::Amount(a) => a,
        };
        high.checked_sub(distance).unwrap_or(Dec::ZERO).round(2, Rounding::HalfEven)
    }
}

/// Whether a trailing stop at `current` moves to `new`: at least half a percent above
/// it, and at least a cent (`SPEC.md` §6, Trailing).
pub fn trail_moves(new: Dec, current: Dec) -> bool {
    let half_pct = current.checked_mul(Dec::new(5, 3).expect("0.005")).unwrap_or(Dec::ZERO);
    let step = if half_pct > Dec::new(1, 2).expect("0.01") { half_pct } else { Dec::new(1, 2).expect("0.01") };
    new >= current.checked_add(step).unwrap_or(current)
}

/// Wealthsimple's codes for refusing a sale of shares the account does not hold.
pub const NO_SHARES: [&str; 2] = ["BALANCE_INSUFFICIENT_SHARES", "NOT_ENOUGH_SHARES"];

/// The rests before an exit refused is tried again: a minute, five, fifteen, then
/// hourly (`SPEC.md` §6, Failures).
pub const RETRY_SECONDS: [i64; 4] = [60, 300, 900, 3600];

/// Within a week of the broker's ninety days a resting exit is placed again while
/// the market is closed, and within two days in any session (`SPEC.md` §6, Ninety days).
pub const ROLL_SECONDS: i64 = 7 * 86400;
pub const ROLL_LAST_SECONDS: i64 = 2 * 86400;

/// How far under the target the price goes before the limit gives way to the stop.
pub fn back_off(target: Dec) -> Dec {
    target.checked_mul(Dec::new(99, 2).expect("0.99")).unwrap_or(target)
}

/// One thing that happened to a bracket.
#[derive(Clone, Debug, PartialEq)]
pub enum BracketEvent {
    Created { quantity: Dec, stop: Option<StopLeg>, target: Option<Dec> },
    /// The entry filled: armed for what filled; `native` when the broker takes stop orders for it.
    Armed { quantity: Dec, high: Option<Dec>, native: bool },
    /// The entry ended with nothing filled.
    EntryEnded { why: String },
    /// A new high, and the stop level under it.
    Trailed { level: Dec, high: Dec },
    /// A level or quantity changed by hand at the broker, followed.
    Adopted { level: Option<Dec>, target: Option<Dec>, quantity: Option<Dec> },
    /// The person changed the legs from the Orders panel.
    Adjusted { stop: Option<StopLeg>, target: Option<Dec> },
    /// An exit went out (the broker has it, or its answer is being read back), at
    /// this price for this quantity.
    Placed { role: ExitRole, order_id: String, price: Option<Dec>, quantity: Dec },
    /// An exit the broker would not take.
    Refused { why: String, code: Option<String> },
    /// The current exit's cancel was sent.
    CancelAsked { order_id: String },
    /// The current exit ended at the broker; what it filled is no longer the bracket's.
    Cleared { filled: Dec },
    /// A move to another phase.
    Moved { to: Phase },
    /// It ends: why.
    Ended { outcome: String },
    /// Nothing is in flight any more: done.
    Done,
    /// Stopped by the guard: why.
    Halted { why: String },
    /// A sale from the ticket asked for; the bracket clears the way.
    SaleAsked { quantity: Dec },
    /// The ticket's sale went out: those shares are no longer the bracket's.
    Sold { quantity: Dec },
    /// The ticket's sale did not go out: the bracket guards again.
    SaleDropped { why: String },
}

impl BracketEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            BracketEvent::Created { .. } => "created",
            BracketEvent::Armed { .. } => "armed",
            BracketEvent::EntryEnded { .. } => "entry-ended",
            BracketEvent::Trailed { .. } => "trailed",
            BracketEvent::Adopted { .. } => "adopted",
            BracketEvent::Adjusted { .. } => "adjusted",
            BracketEvent::Placed { .. } => "placed",
            BracketEvent::Refused { .. } => "refused",
            BracketEvent::CancelAsked { .. } => "cancel-asked",
            BracketEvent::Cleared { .. } => "cleared",
            BracketEvent::Moved { .. } => "moved",
            BracketEvent::Ended { .. } => "ended",
            BracketEvent::Done => "done",
            BracketEvent::Halted { .. } => "halted",
            BracketEvent::SaleAsked { .. } => "sale-asked",
            BracketEvent::Sold { .. } => "sold",
            BracketEvent::SaleDropped { .. } => "sale-dropped",
        }
    }
}

/// The bracket's current exit, as its own events make it and as the broker last
/// stated it.
#[derive(Clone, Debug, PartialEq)]
pub struct Exit {
    pub order_id: String,
    pub role: ExitRole,
    pub fold: OrderFold,
    /// The app asked for its cancel at some point.
    pub cancel_asked: bool,
    /// The price and quantity the broker states for it now: another than the
    /// bracket knows it at (`Bracket::exit_at`) is a change made by hand at the
    /// broker, which the bracket follows.
    pub price: Option<Dec>,
    pub quantity: Option<Dec>,
    /// When the broker lets it lapse.
    pub expires_at: Option<Timestamp>,
}

/// A bracket as its events so far make it.
#[derive(Clone, Debug, PartialEq)]
pub struct Bracket {
    pub phase: Phase,
    pub quantity: Dec,
    pub stop: Option<StopLeg>,
    pub target: Option<Dec>,
    /// The broker takes stop orders for the listing: the stop rests there. Otherwise it is watched here.
    pub native: bool,
    /// The current exit's order id.
    pub exit: Option<(ExitRole, String)>,
    /// The price and quantity the current exit stands at, as the bracket knows it:
    /// what it was placed with, or what was followed from a change by hand.
    pub exit_at: Option<(Option<Dec>, Dec)>,
    /// Refusals since the last exit went out.
    pub attempts: u32,
    pub refused_at: Option<Timestamp>,
    /// The newest refusal, halt or ending, in the words recorded.
    pub why: Option<String>,
    pub outcome: Option<String>,
    /// A ticket sale waiting for the way to clear.
    pub sale: Option<Dec>,
}

impl Bracket {
    /// The fold of a bracket's log, each event with the time it was recorded. An
    /// event that is not a move from where it falls is skipped, as when it was recorded.
    pub fn of(events: &[(Timestamp, BracketEvent)]) -> Option<Bracket> {
        let (first, rest) = events.split_first()?;
        let BracketEvent::Created { quantity, stop, target } = &first.1 else { return None };
        let mut b = Bracket { phase: Phase::Waiting, quantity: *quantity, stop: *stop, target: *target, native: false, exit: None, exit_at: None, attempts: 0, refused_at: None, why: None, outcome: None, sale: None };
        for (at, e) in rest {
            let _ = b.apply(*at, e);
        }
        Some(b)
    }

    /// Apply one event. A refused event changes nothing.
    pub fn apply(&mut self, at: Timestamp, e: &BracketEvent) -> Result<(), String> {
        use Phase::*;
        let refuse = |b: &Bracket| Err(format!("{} is not a move from {}", e.kind(), b.phase.as_str()));
        match e {
            BracketEvent::Created { .. } => return refuse(self),
            BracketEvent::Armed { quantity, high, native } => {
                if self.phase != Waiting {
                    return refuse(self);
                }
                self.quantity = *quantity;
                self.native = *native;
                if let (Some(stop), Some(high)) = (self.stop.as_mut(), high) {
                    if let Some(trail) = stop.trail {
                        stop.high = Some(*high);
                        stop.level = StopLeg::trailed(trail, *high);
                    }
                }
                self.phase = Guarding;
            }
            BracketEvent::EntryEnded { why } => {
                if self.phase != Waiting {
                    return refuse(self);
                }
                self.outcome = Some(why.clone());
                self.phase = Ended;
            }
            BracketEvent::Trailed { level, high } => match self.stop.as_mut() {
                Some(stop) if self.phase.is_live() && self.phase != Waiting => {
                    stop.level = *level;
                    stop.high = Some(*high);
                }
                _ => return refuse(self),
            },
            BracketEvent::Adopted { level, target, quantity } => {
                if !self.phase.is_live() {
                    return refuse(self);
                }
                if let Some((price, qty)) = self.exit_at.as_mut() {
                    if level.is_some() || target.is_some() {
                        *price = level.or(*target);
                    }
                    if let Some(q) = quantity {
                        *qty = *q;
                    }
                }
                if let (Some(l), Some(stop)) = (level, self.stop.as_mut()) {
                    stop.level = *l;
                }
                if let Some(t) = target {
                    self.target = Some(*t);
                }
                if let Some(q) = quantity {
                    self.quantity = *q;
                }
            }
            BracketEvent::Adjusted { stop, target } => {
                if !self.phase.is_live() || matches!(self.phase, Closing | ClosingForSale) {
                    return refuse(self);
                }
                self.stop = *stop;
                self.target = *target;
                // the person acting on a halted bracket starts it again
                if self.phase == Halted {
                    self.phase = Guarding;
                    self.attempts = 0;
                    self.refused_at = None;
                }
            }
            BracketEvent::Placed { role, order_id, price, quantity } => {
                if self.exit.is_some() || !self.phase.is_live() || self.phase == Waiting {
                    return refuse(self);
                }
                self.exit = Some((*role, order_id.clone()));
                self.exit_at = Some((*price, *quantity));
                self.attempts = 0;
                self.refused_at = None;
                self.why = None;
                self.phase = match (self.phase, role) {
                    (Guarding | ToTarget | Target, ExitRole::Target) => Target,
                    (_, ExitRole::Market) => Firing,
                    (p, _) => p,
                };
            }
            BracketEvent::Refused { why, code } => {
                if !self.phase.is_live() {
                    return refuse(self);
                }
                self.attempts += 1;
                self.refused_at = Some(at);
                self.why = Some(why.clone());
                if code.as_deref().is_some_and(|c| NO_SHARES.contains(&c.to_uppercase().as_str())) {
                    self.outcome = Some("the shares are not there".into());
                    self.phase = if self.exit.is_some() { Closing } else { Ended };
                } else if self.phase == Firing && self.exit.is_none() {
                    // the market sell did not go out: the stop guards again
                    self.phase = Guarding;
                }
            }
            BracketEvent::CancelAsked { order_id } => {
                if self.exit.as_ref().map(|(_, id)| id) != Some(order_id) {
                    return refuse(self);
                }
            }
            BracketEvent::Cleared { filled } => {
                let Some((role, _)) = self.exit.take() else { return refuse(self) };
                self.exit_at = None;
                self.quantity = self.quantity.checked_sub(*filled).unwrap_or(self.quantity);
                // a market sell that ended without selling everything: the stop guards the rest
                if self.phase == Firing && role == ExitRole::Market {
                    self.phase = Guarding;
                }
            }
            BracketEvent::Moved { to } => {
                let ok = matches!(
                    (self.phase, to),
                    (Guarding, ToTarget) | (Guarding, Target)
                        | (ToTarget, Target) | (ToTarget, Guarding)
                        | (Target, ToMarket) | (Target, BackToStop) | (Target, Guarding)
                        | (BackToStop, Guarding) | (ToMarket, Firing) | (ToMarket, Guarding)
                        | (Firing, Guarding)
                );
                if !ok {
                    return refuse(self);
                }
                self.phase = *to;
            }
            BracketEvent::Ended { outcome } => {
                if matches!(self.phase, Closing | Ended) {
                    return refuse(self);
                }
                self.outcome = Some(outcome.clone());
                self.sale = None;
                self.phase = if self.exit.is_some() { Closing } else { Ended };
            }
            BracketEvent::Done => {
                if self.phase != Closing || self.exit.is_some() {
                    return refuse(self);
                }
                self.phase = Ended;
            }
            BracketEvent::Halted { why } => {
                if !self.phase.is_live() || matches!(self.phase, Closing | Waiting) {
                    return refuse(self);
                }
                self.why = Some(why.clone());
                self.phase = Halted;
            }
            BracketEvent::SaleAsked { quantity } => {
                if matches!(self.phase, Waiting | Closing | Ended | ClosingForSale) {
                    return refuse(self);
                }
                self.sale = Some(*quantity);
                self.phase = ClosingForSale;
            }
            BracketEvent::Sold { quantity } => {
                if self.phase != ClosingForSale {
                    return refuse(self);
                }
                self.sale = None;
                self.quantity = self.quantity.checked_sub(*quantity).unwrap_or(Dec::ZERO);
                if self.quantity.is_positive() {
                    self.phase = Guarding;
                } else {
                    self.outcome = Some("sold from the ticket".into());
                    self.phase = if self.exit.is_some() { Closing } else { Ended };
                }
            }
            BracketEvent::SaleDropped { why } => {
                if self.phase != ClosingForSale {
                    return refuse(self);
                }
                self.sale = None;
                self.why = Some(why.clone());
                self.phase = Guarding;
            }
        }
        Ok(())
    }

    /// Whether a refused exit may be tried again at `now`.
    pub fn may_retry(&self, now: Timestamp) -> bool {
        match (self.attempts, self.refused_at) {
            (0, _) | (_, None) => true,
            (n, Some(t)) => {
                let wait = RETRY_SECONDS[(n as usize).min(RETRY_SECONDS.len()) - 1];
                now.duration_since(t) >= SignedDuration::from_secs(wait)
            }
        }
    }

    /// The ticket's sale may go out: the bracket's way is clear.
    pub fn clear_for_sale(&self) -> bool {
        self.phase == Phase::ClosingForSale && self.exit.is_none()
    }
}

/// What the broker's quote says, as the engine reads it: only a current quote while
/// the market is open is handed in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tape {
    pub last: Dec,
    pub bid: Option<Dec>,
}

impl Tape {
    /// What a level is compared with: the bid, the price a sell gets, else the last.
    pub fn trigger(&self) -> Dec {
        self.bid.unwrap_or(self.last)
    }
}

/// What the bracket is handed to decide on.
#[derive(Clone, Debug)]
pub struct Seen<'a> {
    pub now: Timestamp,
    /// Whether the market for the listing is open now.
    pub open: bool,
    /// The current quote, while the market is open.
    pub tape: Option<Tape>,
    /// The entry, while the bracket waits for it.
    pub entry: Option<&'a OrderFold>,
    /// The current exit.
    pub exit: Option<&'a Exit>,
    /// Why the position is gone, when the book shows it closed elsewhere.
    pub closed_elsewhere: Option<String>,
}

/// A request for the gate.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// A sell for the bracket's shares: a stop at `price`, a limit at `price`, or at market.
    Place { role: ExitRole, price: Option<Dec>, quantity: Dec },
    Cancel { order_id: String },
}

/// One step: what to record, and at most one request to send.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Step {
    pub events: Vec<BracketEvent>,
    pub request: Option<Request>,
}

impl Step {
    fn nothing() -> Step {
        Step::default()
    }

    fn record(events: Vec<BracketEvent>) -> Step {
        Step { events, request: None }
    }

    fn send(events: Vec<BracketEvent>, request: Request) -> Step {
        Step { events, request: Some(request) }
    }

    pub fn is_nothing(&self) -> bool {
        self.events.is_empty() && self.request.is_none()
    }
}

/// The outcome an exit's fill ends a bracket with.
fn filled_outcome(role: ExitRole) -> &'static str {
    match role {
        ExitRole::Target => "target",
        ExitRole::Stop | ExitRole::Market => "stopped",
    }
}

/// Whether a resting exit is to be placed again before the broker lets it lapse.
pub fn roll_due(exit: &Exit, now: Timestamp, open: bool) -> bool {
    if !matches!(exit.fold.state, OrderState::Pending | OrderState::PartlyFilled) || exit.cancel_asked {
        return false;
    }
    let Some(expires) = exit.expires_at else { return false };
    let left = expires.duration_since(now);
    if left > SignedDuration::from_secs(ROLL_SECONDS) {
        return false;
    }
    left <= SignedDuration::from_secs(ROLL_LAST_SECONDS) || !open
}

/// End the bracket: cancel what it has in flight (an exit whose own answer is not
/// known yet is cancelled once it is, from `Closing`).
pub fn end(exit: Option<&Exit>, outcome: &str) -> Step {
    let ended = BracketEvent::Ended { outcome: outcome.into() };
    match exit {
        Some(x) if x.fold.state.in_flight() && x.fold.state != OrderState::Cancelling && !waiting_on(x) => {
            Step::send(vec![ended, BracketEvent::CancelAsked { order_id: x.order_id.clone() }], Request::Cancel { order_id: x.order_id.clone() })
        }
        _ => Step::record(vec![ended]),
    }
}

/// The person changes the legs. An exit resting at a level that changed is placed
/// again there by the next step (`stale`); a stop removed while it rests is cancelled
/// then; both legs gone ends the bracket.
pub fn adjust(exit: Option<&Exit>, stop: Option<StopLeg>, target: Option<Dec>) -> Step {
    if stop.is_none() && target.is_none() {
        return end(exit, "both legs removed");
    }
    Step::record(vec![BracketEvent::Adjusted { stop, target }])
}

/// Cancel the current exit, moving to `then` (or staying).
fn cancel(x: &Exit, then: Option<Phase>) -> Step {
    let mut events = Vec::new();
    if let Some(to) = then {
        events.push(BracketEvent::Moved { to });
    }
    events.push(BracketEvent::CancelAsked { order_id: x.order_id.clone() });
    Step::send(events, Request::Cancel { order_id: x.order_id.clone() })
}

/// A sell for the bracket's shares, when a refusal's rest is over.
fn place(b: &Bracket, now: Timestamp, role: ExitRole, price: Option<Dec>) -> Step {
    if !b.may_retry(now) {
        return Step::nothing();
    }
    Step::send(vec![], Request::Place { role, price, quantity: b.quantity })
}

/// The one step the bracket takes from where it stands. The caller records the
/// events, sends the request through the gate, records the gate's answer (`placed`,
/// `refused`, or the order's own events), and asks again until the step is nothing.
pub fn decide(b: &Bracket, seen: &Seen) -> Step {
    use Phase::*;
    if b.phase == Ended {
        return Step::nothing();
    }
    if b.phase == Waiting {
        return arm(b, seen);
    }
    // what became of the exit first
    let exit = seen.exit.filter(|x| b.exit.as_ref().is_some_and(|(_, id)| *id == x.order_id));
    if b.exit.is_some() && exit.is_none() {
        // the exit the bracket holds is not known yet (read back next): wait
        return Step::nothing();
    }
    if let Some(x) = exit {
        if let Some(step) = reconcile(b, x) {
            return step;
        }
    }
    if b.phase == Closing {
        return match exit {
            Some(x) if x.fold.state.in_flight() => keep_cancelling(x),
            Some(_) => Step::nothing(),
            None => Step::record(vec![BracketEvent::Done]),
        };
    }
    if b.phase == Halted {
        return Step::nothing();
    }
    // a position closed elsewhere is seen only with nothing resting to fill for it
    if exit.is_none() && !matches!(b.phase, ClosingForSale) {
        if let Some(why) = &seen.closed_elsewhere {
            return end(None, why);
        }
    }
    let level = b.stop.map(|s| s.level);
    let trigger = seen.tape.map(|t| t.trigger());
    let stop_hit = matches!((level, trigger), (Some(l), Some(t)) if t <= l);
    let at_target = matches!((b.target, trigger), (Some(tp), Some(t)) if t >= tp);
    // a trailing stop follows the high while it guards or while the limit rests
    if (b.phase == Guarding && !at_target) || b.phase == Target {
        if let (Some(stop), Some(tape)) = (b.stop, seen.tape) {
            if let (Some(trail), Some(high)) = (stop.trail, stop.high.or(Some(tape.last))) {
                let high = if tape.last > high { tape.last } else { high };
                if Some(high) != stop.high {
                    let new = StopLeg::trailed(trail, high);
                    let level = if trail_moves(new, stop.level) { new } else { stop.level };
                    // a stop resting at the old level is placed again at the new one (`stale`)
                    return Step::record(vec![BracketEvent::Trailed { level, high }]);
                }
            }
        }
    }
    match b.phase {
        Guarding => match exit {
            None => {
                if stop_hit {
                    return place(b, seen.now, ExitRole::Market, None);
                }
                if at_target {
                    return place(b, seen.now, ExitRole::Target, b.target);
                }
                if b.native && b.stop.is_some() {
                    return place(b, seen.now, ExitRole::Stop, level);
                }
                Step::nothing()
            }
            Some(x) if waiting_on(x) => Step::nothing(),
            Some(x) if x.fold.state == OrderState::Cancelling => Step::nothing(),
            Some(x) => {
                if at_target {
                    return cancel(x, Some(ToTarget));
                }
                if stale(b, x) || roll_due(x, seen.now, seen.open) {
                    return cancel(x, None);
                }
                Step::nothing()
            }
        },
        ToTarget | Target if b.target.is_none() => match exit {
            // the target was removed: the limit, if one rests, gives way to the stop
            Some(x) if x.role == ExitRole::Target => keep_cancelling(x),
            Some(_) => Step::nothing(),
            None => Step::record(vec![BracketEvent::Moved { to: Guarding }]),
        },
        ToTarget => match exit {
            Some(x) if x.fold.state == OrderState::Cancelling || waiting_on(x) => Step::nothing(),
            // the cancel was refused: asked again
            Some(x) => cancel(x, None),
            None => {
                if stop_hit {
                    return place(b, seen.now, ExitRole::Market, None);
                }
                if matches!((b.target, trigger), (Some(tp), Some(t)) if t < back_off(tp)) {
                    return Step::record(vec![BracketEvent::Moved { to: Guarding }]);
                }
                place(b, seen.now, ExitRole::Target, b.target)
            }
        },
        Target => match exit {
            None => {
                if stop_hit {
                    return place(b, seen.now, ExitRole::Market, None);
                }
                if matches!((b.target, trigger), (Some(tp), Some(t)) if t < back_off(tp)) {
                    return Step::record(vec![BracketEvent::Moved { to: Guarding }]);
                }
                place(b, seen.now, ExitRole::Target, b.target)
            }
            Some(x) if x.fold.state == OrderState::Cancelling || waiting_on(x) => Step::nothing(),
            Some(x) => {
                if stop_hit {
                    return cancel(x, Some(ToMarket));
                }
                if matches!((b.target, trigger), (Some(tp), Some(t)) if t < back_off(tp)) {
                    return cancel(x, Some(BackToStop));
                }
                if stale(b, x) || roll_due(x, seen.now, seen.open) {
                    return cancel(x, None);
                }
                Step::nothing()
            }
        },
        BackToStop => match exit {
            Some(x) if x.fold.state == OrderState::Cancelling || waiting_on(x) => Step::nothing(),
            Some(x) => cancel(x, None),
            None => Step::record(vec![BracketEvent::Moved { to: Guarding }]),
        },
        ToMarket => match exit {
            Some(x) if x.fold.state == OrderState::Cancelling || waiting_on(x) => Step::nothing(),
            Some(x) => cancel(x, None),
            None => place(b, seen.now, ExitRole::Market, None),
        },
        Firing => match exit {
            Some(_) => Step::nothing(),
            None => Step::record(vec![BracketEvent::Moved { to: Guarding }]),
        },
        ClosingForSale => match exit {
            Some(x) if x.fold.state.in_flight() => keep_cancelling(x),
            _ => Step::nothing(),
        },
        Waiting | Closing | Halted | Ended => Step::nothing(),
    }
}

/// A resting exit that no longer stands where the bracket does: its level moved (a
/// trail, the person's edit) or the bracket's shares changed (a part sold): it is
/// cancelled and placed again.
fn stale(b: &Bracket, x: &Exit) -> bool {
    let (known_price, known_qty) = b.exit_at.unwrap_or((None, b.quantity));
    let stated = x.price.or(known_price);
    let level = match x.role {
        ExitRole::Stop => b.stop.map(|s| s.level),
        ExitRole::Target => b.target,
        ExitRole::Market => return false,
    };
    stated != level || x.quantity.unwrap_or(known_qty) != b.quantity
}

/// An exit whose own answer is not known yet: nothing is sent past it.
fn waiting_on(x: &Exit) -> bool {
    matches!(x.fold.state, OrderState::Sending | OrderState::Unconfirmed)
}

/// Cancel the exit, unless its cancel is already out or its answer is not known.
fn keep_cancelling(x: &Exit) -> Step {
    if x.fold.state == OrderState::Cancelling || waiting_on(x) {
        Step::nothing()
    } else {
        cancel(x, None)
    }
}

fn arm(b: &Bracket, seen: &Seen) -> Step {
    let Some(entry) = seen.entry else { return Step::nothing() };
    if entry.state.in_flight() || entry.state == OrderState::Dry {
        return Step::nothing();
    }
    if entry.filled.is_positive() {
        return Step::record(vec![BracketEvent::Armed { quantity: entry.filled, high: entry.average, native: b.native }]);
    }
    Step::record(vec![BracketEvent::EntryEnded { why: format!("entry {}", entry.state.as_str()) }])
}

/// Bring the bracket in line with what became of its exit: a fill ends it, an end
/// clears it (and a cancel nobody here asked for ends the bracket), a price or a
/// quantity changed by hand is followed. `None` when there is nothing to do.
fn reconcile(b: &Bracket, x: &Exit) -> Option<Step> {
    use OrderState::*;
    use Phase::{Closing, ClosingForSale, Guarding, Target};
    match x.fold.state {
        Filled => {
            if b.phase == Closing {
                return Some(Step::record(vec![BracketEvent::Cleared { filled: x.fold.filled }]));
            }
            let mut events = vec![BracketEvent::Cleared { filled: x.fold.filled }];
            if b.phase == ClosingForSale {
                // it filled before its cancel landed: those shares are sold; the ticket's sale is dropped
                events.push(BracketEvent::SaleDropped { why: format!("the {} filled first", x.role.as_str()) });
            }
            events.push(BracketEvent::Ended { outcome: filled_outcome(x.role).into() });
            Some(Step::record(events))
        }
        Cancelled | Expired | Rejected | Failed => {
            let mut events = vec![BracketEvent::Cleared { filled: x.fold.filled }];
            if b.phase == Closing {
                return Some(Step::record(events));
            }
            match x.fold.state {
                Cancelled if !x.cancel_asked => events.push(BracketEvent::Ended { outcome: format!("{} cancelled at Wealthsimple by hand", x.role.as_str()) }),
                Rejected => events.push(BracketEvent::Refused { why: x.fold.why.clone().unwrap_or_else(|| "rejected".into()), code: x.fold.code.clone() }),
                _ => {}
            }
            Some(Step::record(events))
        }
        Pending | PartlyFilled if !x.cancel_asked && matches!(b.phase, Guarding | Target) => {
            // changed by hand: the broker states another than the app sent, and the
            // bracket does not stand there yet
            let (known_price, known_qty) = b.exit_at.unwrap_or((None, b.quantity));
            let by_hand = |theirs: Option<Dec>| theirs.is_some() && theirs != known_price;
            let (level, target) = match x.role {
                ExitRole::Stop if by_hand(x.price) => (x.price, None),
                ExitRole::Target if by_hand(x.price) => (None, x.price),
                _ => (None, None),
            };
            let quantity = x.quantity.filter(|q| q.is_positive() && *q != known_qty);
            if level.is_none() && target.is_none() && quantity.is_none() {
                return None;
            }
            Some(Step::record(vec![BracketEvent::Adopted { level, target, quantity }]))
        }
        _ => None,
    }
}

/// What the caller records after the gate answered a `Place`: the exit went out
/// (the broker's answer, or one being read back).
pub fn placed(request: &Request, order_id: &str) -> Option<BracketEvent> {
    match request {
        Request::Place { role, price, quantity } => Some(BracketEvent::Placed { role: *role, order_id: order_id.into(), price: *price, quantity: *quantity }),
        Request::Cancel { .. } => None,
    }
}

/// A bracket's tick in full against a stated world: steps until nothing, at most
/// `limit` of them (a guard against a loop in the rules, never reached by them).
pub fn settle<F>(b: &mut Bracket, seen: &Seen, limit: usize, mut send: F) -> Vec<Step>
where
    F: FnMut(&Bracket, &Request) -> Vec<BracketEvent>,
{
    let mut steps = Vec::new();
    for _ in 0..limit {
        let step = decide(b, seen);
        if step.is_nothing() {
            break;
        }
        for e in &step.events {
            let _ = b.apply(seen.now, e);
        }
        let sent = step.request.is_some();
        if let Some(r) = &step.request {
            for e in send(b, r) {
                let _ = b.apply(seen.now, &e);
            }
        }
        steps.push(step);
        // a request's answer is the broker's to give: nothing more this tick
        if sent {
            break;
        }
    }
    steps
}
