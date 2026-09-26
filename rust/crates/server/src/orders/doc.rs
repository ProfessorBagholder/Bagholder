//! The `orders` document (`SPEC.md` §4, Orders): every card the Orders panel shows,
//! built here with its amounts as exact decimal text and its legs' words, so the page
//! only lays out and formats what it is sent.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bagholder_book::orders::{StoredBracket, StoredOrder};
use bagholder_book::Book;
use bagholder_core::bracket::{Bracket, BracketEvent, ExitRole, Phase, Trail};
use bagholder_core::order::{OrderKind, OrderRole, OrderState, Side, TimeInForce};
use bagholder_core::Dec;
use bagholder_diff_derive::Diff;
use serde::Serialize;
use ts_rs::TS;

use super::{log, orders_live, units_of, Elsewhere};
use crate::app::App;
use crate::wire::{Dec as Text, Fig};

/// The ended orders and brackets the panel is sent, newest first: its own paging.
/// Every order and bracket still live is sent, however many.
pub const PAGE: u32 = 200;

/// The tab a card stands under.
fn tab_of(s: OrderState) -> &'static str {
    match s {
        OrderState::Filled => "filled",
        OrderState::Cancelled | OrderState::Expired | OrderState::Rejected | OrderState::Failed | OrderState::Dry => "cancelled",
        _ => "pending",
    }
}

/// A leg of a bracket, as its row shows it.
#[derive(Clone, Debug, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = key)]
pub struct Leg {
    /// `sl` or `tp`.
    pub key: String,
    pub quantity: Text,
    /// The level: the stop's (a trailing stop's current one) or the target.
    pub level: Text,
    /// A trailing stop's trail: a percent, or an amount per share.
    pub trail_pct: Option<Text>,
    pub trail_amount: Option<Text>,
    /// The exit's fill, when this leg is how the bracket ended.
    pub filled: Option<Filled>,
    /// `Placing`, `Cancelling`, `Retrying · <reason>`, `Watching`, `Filled`,
    /// `Cancelled`, `Off`, or nothing.
    pub note: String,
    /// Quantity × level × the contract size; a fill's own amount once it filled.
    pub amount: Fig<Text>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct Filled {
    pub quantity: Text,
    pub average: Option<Text>,
}

/// An order the panel shows: one of the app's (never a bracket's own exit), or one
/// Wealthsimple reports that was placed elsewhere.
#[derive(Clone, Debug, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = id)]
pub struct OrderCard {
    pub id: String,
    /// The broker's id for the account.
    pub account: String,
    pub exchange: String,
    pub symbol: String,
    /// `buy` or `sell`.
    pub side: String,
    /// `market`, `limit`, `stop` or `stop-limit`.
    pub kind: String,
    /// `day` or `until-cancel`; none when Wealthsimple has not said.
    pub tif: Option<String>,
    pub quantity: Text,
    pub limit_price: Option<Text>,
    pub stop_price: Option<Text>,
    /// Where it stands (`bagholder_core::order::OrderState`).
    pub state: String,
    pub filled: Text,
    pub average: Option<Text>,
    /// Why it was refused, failed or is not confirmed, in the words recorded.
    pub why: Option<String>,
    /// Its value in the instrument's currency: the quantity at its price, what it
    /// filled for once filled; none for a market order not filled.
    pub value: Option<Fig<Text>>,
    /// The value is the fill's price guessed at: a market order's.
    pub approx: bool,
    /// `pending`, `filled` or `cancelled`.
    pub tab: String,
    pub at: String,
    /// Edit and Cancel act on it; Edit is dimmed where Wealthsimple takes no change.
    pub live: bool,
    pub editable: bool,
    /// The legs of the bracket waiting for this order to fill.
    pub legs: Vec<Leg>,
}

/// An armed bracket, as its own card.
#[derive(Clone, Debug, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = id)]
pub struct BracketCard {
    pub id: String,
    pub account: String,
    pub exchange: String,
    pub symbol: String,
    pub tab: String,
    /// When it armed while live; when it ended after.
    pub at: String,
    pub live: bool,
    /// What was paid for the shares under it.
    pub value: Fig<Text>,
    pub legs: Vec<Leg>,
    /// On the Cancelled tab: `Cancelled` when the person ended it, `Off` otherwise.
    pub end_word: Option<String>,
    /// What its editor starts from.
    pub stop_level: Option<Text>,
    pub trail_pct: Option<Text>,
    pub trail_amount: Option<Text>,
    pub target: Option<Text>,
}

/// The `orders` document: what the Orders panel is sent, and sent again as it changes.
#[derive(Clone, Debug, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct OrdersDoc {
    pub ok: bool,
    /// Whether orders are sent at all (`BAGHOLDER_DRY_ORDERS` turns them off).
    pub live: bool,
    pub refreshed_at: Option<String>,
    pub orders: Vec<OrderCard>,
    pub brackets: Vec<BracketCard>,
    /// Why the document could not be read, when it could not.
    pub error: Option<String>,
}

fn t(d: Dec) -> Text {
    Text(d)
}

/// `a × b × c`, or the gap it waits on.
fn times(parts: &[Dec], per: Option<Dec>) -> Fig<Text> {
    let Some(per) = per else { return Fig::Waits { gaps: vec!["multiplier-unstated".into()] } };
    let mut v = per;
    for p in parts {
        match v.checked_mul(*p) {
            Ok(x) => v = x,
            Err(_) => return Fig::Waits { gaps: vec!["arithmetic".into()] },
        }
    }
    Fig::Stated(t(v))
}

/// What the book calls a Wealthsimple security now and the venue it trades on.
struct Names<'a> {
    book: &'a Book,
    app: &'a Arc<App>,
    seen: HashMap<String, (Option<String>, String, Option<Dec>)>,
}

impl Names<'_> {
    /// (the book's symbol for it, its venue, shares per unit)
    fn of(&mut self, security: &str) -> (Option<String>, String, Option<Dec>) {
        if let Some(v) = self.seen.get(security) {
            return v.clone();
        }
        use bagholder_core::instrument::{RefScheme, Reference};
        let r = Reference::new(RefScheme::BrokerSecurity(bagholder_core::Broker::named("wealthsimple")), security);
        let named = self.book.instrument_by_ref(&r).and_then(|i| i.map(|i| self.book.names(i)).transpose());
        let (symbol, venue) = match named {
            Ok(Some(n)) => {
                let last = n.last();
                (last.map(|n| n.symbol.clone()), last.and_then(|n| n.venue_name.clone().or_else(|| n.venue_mic.clone())).unwrap_or_default())
            }
            Ok(None) => (None, String::new()),
            Err(e) => {
                log(&format!("bagholder orders: what the book calls {security} could not be read: {e}"));
                (None, String::new())
            }
        };
        let units = units_of(self.app, self.book, security).unwrap_or_else(|e| {
            log(&format!("bagholder orders: the size of {security} could not be read: {e}"));
            None
        });
        let v = (symbol, venue, units);
        self.seen.insert(security.to_string(), v.clone());
        v
    }
}

fn kind_word(k: OrderKind) -> String {
    k.as_str().to_string()
}

fn side_word(s: Side) -> String {
    s.as_str().to_string()
}

/// An order's value: what it filled for once filled, else the quantity at its price.
fn order_value(kind: OrderKind, state: OrderState, quantity: Dec, filled: Dec, average: Option<Dec>, limit: Option<Dec>, stop: Option<Dec>, per: Option<Dec>) -> (Option<Fig<Text>>, bool) {
    if state == OrderState::Filled {
        if let Some(avg) = average {
            return (Some(times(&[if filled.is_positive() { filled } else { quantity }, avg], per)), false);
        }
    }
    let price = match kind {
        OrderKind::Market => average,
        OrderKind::Stop => stop,
        OrderKind::Limit | OrderKind::StopLimit => limit,
    };
    match price.filter(|p| p.is_positive()) {
        Some(p) => (Some(times(&[quantity, p], per)), kind == OrderKind::Market),
        None => (None, false),
    }
}

/// A bracket's legs as its rows show them (`SPEC.md` §4, Orders, Bracket rows): no
/// word while a leg rests, is watched or waits for the fill; `Watching` on the stop
/// while the target's limit sell stands in for it; `Placing` and `Cancelling` while a
/// request is out, `Retrying · <reason>` after a refused one; at the end the leg that
/// exited reads its fill, the other `Cancelled`, and both `Off` when the bracket ended
/// without exiting.
pub fn legs(b: &Bracket, orders: &[StoredOrder], per: Option<Dec>) -> Vec<Leg> {
    let exit = b.exit.as_ref().and_then(|(_, id)| orders.iter().find(|o| o.request.id == *id));
    let exit_role = b.exit.as_ref().map(|(r, _)| *r);
    let exit_state = exit.map(|o| o.fold.state);
    let sending = matches!(exit_state, Some(OrderState::Sending | OrderState::Unconfirmed));
    let cancelling = exit_state == Some(OrderState::Cancelling);
    let retrying = || format!("Retrying · {}", b.why.clone().unwrap_or_default());
    let exited_by = match b.outcome.as_deref() {
        Some("stopped") => Some(ExitRole::Stop),
        Some("target") => Some(ExitRole::Target),
        _ => None,
    };
    let not_exited = if exited_by.is_some() { "Cancelled" } else { "Off" };
    // the fill of the exit that ended it, from the bracket's own orders
    let fill_of = |stop: bool| {
        orders.iter().rev().find(|o| {
            let role = o.request.bracket.as_ref().map(|(_, r)| *r);
            o.fold.state == OrderState::Filled && if stop { matches!(role, Some(OrderRole::Stop | OrderRole::Market)) } else { role == Some(OrderRole::Target) }
        })
    };
    let q = b.quantity;
    let mut out = Vec::new();
    for (key, level, is_stop) in [("sl", b.stop.map(|s| s.level), true), ("tp", b.target, false)] {
        let Some(level) = level else { continue };
        let mine = |r: Option<ExitRole>| if is_stop { matches!(r, Some(ExitRole::Stop | ExitRole::Market)) } else { r == Some(ExitRole::Target) };
        let note: String = match b.phase {
            Phase::Waiting | Phase::Halted => String::new(),
            Phase::Guarding if is_stop => match exit_role {
                Some(r) if mine(Some(r)) && cancelling => "Cancelling".into(),
                Some(r) if mine(Some(r)) && sending => "Placing".into(),
                Some(_) => String::new(),
                // no stop out: one that should rest is being placed, or tried again
                None if b.native => {
                    if b.attempts > 0 {
                        retrying()
                    } else {
                        "Placing".into()
                    }
                }
                None => String::new(),
            },
            Phase::Guarding => String::new(),
            Phase::ToTarget if is_stop => "Cancelling".into(),
            Phase::ToTarget => "Placing".into(),
            Phase::Target if is_stop => "Watching".into(),
            Phase::Target => match exit_role {
                Some(ExitRole::Target) if sending => "Placing".into(),
                Some(ExitRole::Target) if cancelling => "Cancelling".into(),
                Some(ExitRole::Target) => String::new(),
                _ if b.attempts > 0 => retrying(),
                _ => "Placing".into(),
            },
            Phase::BackToStop | Phase::ToMarket if is_stop => "Placing".into(),
            Phase::BackToStop | Phase::ToMarket => "Cancelling".into(),
            Phase::Firing if is_stop => {
                if b.attempts > 0 {
                    retrying()
                } else {
                    "Placing".into()
                }
            }
            Phase::Firing => String::new(),
            Phase::ClosingForSale => {
                if mine(exit_role) {
                    "Cancelling".into()
                } else {
                    String::new()
                }
            }
            Phase::Closing => {
                if exited_by.is_some_and(|r| (r == ExitRole::Stop) == is_stop) {
                    "Filled".into()
                } else if mine(exit_role) && exit.is_some_and(|o| o.fold.state.in_flight()) {
                    "Cancelling".into()
                } else {
                    not_exited.into()
                }
            }
            Phase::Ended => String::new(),
        };
        let mut leg = Leg {
            key: key.into(),
            quantity: t(q),
            level: t(level),
            trail_pct: None,
            trail_amount: None,
            filled: None,
            note,
            amount: times(&[q, level], per),
        };
        if is_stop {
            match b.stop.and_then(|s| s.trail) {
                Some(Trail::Pct(p)) => leg.trail_pct = Some(t(p)),
                Some(Trail::Amount(a)) => leg.trail_amount = Some(t(a)),
                None => {}
            }
        }
        if b.phase == Phase::Ended {
            let exited_here = exited_by.is_some_and(|r| (r == ExitRole::Stop) == is_stop);
            match (exited_here, fill_of(is_stop)) {
                (true, Some(f)) => {
                    let fq = if f.fold.filled.is_positive() { f.fold.filled } else { q };
                    leg.quantity = t(fq);
                    leg.filled = Some(Filled { quantity: t(fq), average: f.fold.average.map(t) });
                    if let Some(avg) = f.fold.average {
                        leg.amount = times(&[fq, avg], per);
                    }
                }
                (true, None) => leg.note = "Filled".into(),
                (false, _) => leg.note = not_exited.into(),
            }
            // an exit refused for shares that are not there: the reason on its leg
            if exited_by.is_none() && b.outcome.as_deref() == Some("the shares are not there") {
                let refused = orders.iter().rev().find(|o| o.fold.state == OrderState::Rejected && o.request.bracket.as_ref().is_some_and(|(_, r)| *r != OrderRole::Entry));
                let mine = refused.is_some_and(|o| {
                    let r = o.request.bracket.as_ref().map(|(_, r)| *r);
                    if is_stop { matches!(r, Some(OrderRole::Stop | OrderRole::Market)) } else { r == Some(OrderRole::Target) }
                });
                if mine {
                    leg.note = format!("Off · {}", refused.and_then(|o| o.fold.why.clone()).or_else(|| b.why.clone()).unwrap_or_else(|| "the shares are not there".into()));
                }
            }
        }
        out.push(leg);
    }
    out
}

/// When a bracket armed, from its log; one carried over from the earlier app already
/// armed counts from when it was created.
fn armed_at(book: &Book, sb: &StoredBracket) -> Result<Option<jiff::Timestamp>, String> {
    let log = book.bracket_log(&sb.place.id).map_err(|e| e.to_string())?;
    if let Some(BracketEvent::Imported { phase, .. }) = log.first().map(|l| &l.event) {
        if *phase != Phase::Waiting {
            return Ok(Some(sb.created_at));
        }
    }
    Ok(log.iter().find(|l| l.refused.is_none() && matches!(l.event, BracketEvent::Armed { .. })).map(|l| l.at))
}

fn bracket_card(sb: &StoredBracket, orders: &[StoredOrder], armed: jiff::Timestamp, names: &mut Names) -> BracketCard {
    let b = &sb.bracket;
    let (symbol, exchange, per) = names.of(&sb.place.broker_security);
    let entry = orders.iter().find(|o| o.request.bracket.as_ref().is_some_and(|(_, r)| *r == OrderRole::Entry));
    let value = match entry.and_then(|e| e.fold.average) {
        Some(avg) => times(&[b.quantity, avg], per),
        None => Fig::Waits { gaps: vec!["price-unknown".into()] },
    };
    let live = b.phase.is_live();
    let exited = matches!(b.outcome.as_deref(), Some("stopped" | "target"));
    let tab = if live {
        "pending"
    } else if exited {
        "filled"
    } else {
        "cancelled"
    };
    let end_word = (!live && !exited).then(|| if matches!(b.outcome.as_deref(), Some("cancelled by the user" | "both legs removed")) { "Cancelled" } else { "Off" }.to_string());
    let at = if live { armed } else { sb.updated_at };
    BracketCard {
        id: sb.place.id.clone(),
        account: sb.place.broker_account.clone(),
        exchange,
        symbol: symbol.unwrap_or_else(|| sb.place.symbol.clone()),
        tab: tab.into(),
        at: at.to_string(),
        live,
        value,
        legs: legs(b, orders, per),
        end_word,
        stop_level: b.stop.map(|s| t(s.level)),
        trail_pct: match b.stop.and_then(|s| s.trail) {
            Some(Trail::Pct(p)) => Some(t(p)),
            _ => None,
        },
        trail_amount: match b.stop.and_then(|s| s.trail) {
            Some(Trail::Amount(a)) => Some(t(a)),
            _ => None,
        },
        target: b.target.map(t),
    }
}

fn order_card(o: &StoredOrder, waiting: Option<(&Bracket, &[StoredOrder])>, names: &mut Names) -> OrderCard {
    let r = &o.request;
    let (symbol, exchange, per) = names.of(&r.broker_security);
    let (value, approx) = order_value(r.kind, o.fold.state, r.quantity, o.fold.filled, o.fold.average, r.limit_price, r.stop_price, per);
    let working = matches!(o.fold.state, OrderState::Pending | OrderState::PartlyFilled);
    OrderCard {
        id: r.id.clone(),
        account: r.broker_account.clone(),
        exchange,
        symbol: symbol.unwrap_or_else(|| r.symbol.clone()),
        side: side_word(r.side),
        kind: kind_word(r.kind),
        tif: Some(match r.time_in_force {
            TimeInForce::Day => "day",
            TimeInForce::UntilCancel => "until-cancel",
        }
        .into()),
        quantity: t(o.stated_quantity.unwrap_or(r.quantity)),
        limit_price: r.limit_price.map(t),
        stop_price: r.stop_price.map(t),
        state: o.fold.state.as_str().into(),
        filled: t(o.fold.filled),
        average: o.fold.average.map(t),
        // a reading that could not be taken is the newest word on it
        why: o.refused.clone().or_else(|| o.fold.why.clone()),
        value,
        approx,
        tab: tab_of(o.fold.state).into(),
        at: o.created_at.to_string(),
        live: working,
        editable: working && r.kind != OrderKind::Stop,
        legs: waiting.map(|(b, orders)| legs(b, orders, per)).unwrap_or_default(),
    }
}

fn elsewhere_card(e: &Elsewhere, names: &mut Names) -> OrderCard {
    let (symbol, exchange, per) = names.of(&e.security);
    let (value, approx) = order_value(e.kind, e.state, e.quantity, e.filled, e.average, e.limit_price, e.stop_price, per);
    let working = matches!(e.state, OrderState::Pending | OrderState::PartlyFilled);
    OrderCard {
        id: e.id.clone(),
        account: e.account.clone(),
        exchange,
        symbol: symbol.unwrap_or_else(|| e.symbol.clone()),
        side: side_word(e.side),
        kind: kind_word(e.kind),
        tif: None,
        quantity: t(e.quantity),
        limit_price: e.limit_price.map(t),
        stop_price: e.stop_price.map(t),
        state: e.state.as_str().into(),
        filled: t(e.filled),
        average: e.average.map(t),
        why: None,
        value,
        approx,
        tab: tab_of(e.state).into(),
        at: e.created_at.to_string(),
        live: working,
        editable: working && e.kind != OrderKind::Stop,
        legs: Vec::new(),
    }
}

fn build(app: &Arc<App>) -> Result<OrdersDoc, String> {
    let f = app.figures.get().ok_or("the book is not open")?;
    let book = f.book()?;
    let e = |e: bagholder_book::BookError| e.to_string();
    let mut names = Names { book: &book, app, seen: HashMap::new() };
    // every live bracket, and the newest ended ones
    let mut brackets: Vec<StoredBracket> = book.live_brackets().map_err(e)?;
    let mut seen: HashSet<String> = brackets.iter().map(|b| b.place.id.clone()).collect();
    for b in book.brackets_before(None, PAGE).map_err(e)? {
        if seen.insert(b.place.id.clone()) {
            brackets.push(b);
        }
    }
    // every order in flight, and the newest ended ones
    let mut orders: Vec<StoredOrder> = book.orders_in_flight().map_err(e)?;
    let mut seen: HashSet<String> = orders.iter().map(|o| o.request.id.clone()).collect();
    for o in book.orders_before(None, PAGE).map_err(e)? {
        if seen.insert(o.request.id.clone()) {
            orders.push(o);
        }
    }
    let mut by_bracket: HashMap<String, Vec<StoredOrder>> = HashMap::new();
    let mut waiting: HashMap<String, String> = HashMap::new();
    let mut bracket_cards = Vec::new();
    for sb in &brackets {
        let own = book.orders_of_bracket(&sb.place.id).map_err(e)?;
        if sb.bracket.phase == Phase::Waiting {
            if let Some(entry) = own.iter().find(|o| o.request.bracket.as_ref().is_some_and(|(_, r)| *r == OrderRole::Entry)) {
                waiting.insert(entry.request.id.clone(), sb.place.id.clone());
            }
        } else if let Some(armed) = armed_at(&book, sb)? {
            // a bracket that never armed was only legs on its entry's card
            bracket_cards.push(bracket_card(sb, &own, armed, &mut names));
        }
        by_bracket.insert(sb.place.id.clone(), own);
    }
    let mut cards = Vec::new();
    for o in &orders {
        // a bracket's own exits are never cards: its legs are
        if o.request.bracket.as_ref().is_some_and(|(_, r)| *r != OrderRole::Entry) {
            continue;
        }
        let w = waiting.get(&o.request.id).and_then(|bid| {
            let sb = brackets.iter().find(|b| b.place.id == *bid)?;
            Some((&sb.bracket, by_bracket.get(bid).map(Vec::as_slice).unwrap_or(&[])))
        });
        cards.push(order_card(o, w, &mut names));
    }
    for x in app.orders.elsewhere.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        cards.push(elsewhere_card(x, &mut names));
    }
    cards.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.id.cmp(&b.id)));
    bracket_cards.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.id.cmp(&b.id)));
    Ok(OrdersDoc {
        ok: true,
        live: orders_live(app),
        refreshed_at: app.orders.refreshed_at.lock().unwrap_or_else(|e| e.into_inner()).map(|t| t.to_string()),
        orders: cards,
        brackets: bracket_cards,
        error: None,
    })
}

pub fn orders_doc(app: &Arc<App>) -> OrdersDoc {
    build(app).unwrap_or_else(|why| OrdersDoc { ok: false, live: orders_live(app), refreshed_at: None, orders: Vec::new(), brackets: Vec::new(), error: Some(why) })
}

/// What the header's badge counts: the Orders panel's Pending cards (`SPEC.md` §4,
/// Orders), so the two agree -- every order still with the broker or being sent that
/// is not a bracket's own exit, and every armed bracket still live -- in the accounts
/// in scope, named by the broker's id for each (`None`: every account).
pub fn open_orders_count(app: &Arc<App>, accounts: Option<&HashSet<String>>) -> i64 {
    let within = |a: &str| accounts.is_none_or(|s| s.contains(a));
    let Some(f) = app.figures.get() else { return 0 };
    let count = || -> Result<i64, String> {
        let book = f.book()?;
        let e = |e: bagholder_book::BookError| e.to_string();
        let entries = book.orders_in_flight().map_err(e)?.iter().filter(|o| !o.request.bracket.as_ref().is_some_and(|(_, r)| *r != OrderRole::Entry) && within(&o.request.broker_account)).count();
        let armed = book.live_brackets().map_err(e)?.iter().filter(|b| b.bracket.phase != Phase::Waiting && within(&b.place.broker_account)).count();
        let elsewhere = app.orders.elsewhere.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|x| x.state.in_flight() && within(&x.account)).count();
        Ok((entries + armed + elsewhere) as i64)
    };
    count().unwrap_or_else(|e| {
        log(&format!("bagholder orders: the open orders could not be counted: {e}"));
        0
    })
}
