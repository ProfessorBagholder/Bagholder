//! The bracket engine (`SPEC.md` §6, Brackets after the fill): every five seconds
//! while the app runs connected, each live bracket reads back what it is waiting on,
//! takes its steps (`bagholder_core::bracket::decide`) against the broker's quote,
//! records each in its log, and sends what a step asks for through the gate. One
//! bracket at a time takes its steps (`gate::bracket_lock`), whoever asks: the
//! engine, the ticket or the Orders panel.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bagholder_book::orders::{OrderRequest, StoredBracket};
use bagholder_book::Book;
use bagholder_core::bracket::{self, Bracket, BracketEvent, Exit, ExitRole, Phase, Request, Seen, StateKey, Step, Tape};
use bagholder_core::order::{Asker, OrderEvent, OrderFold, OrderKind, OrderRole, OrderState, Side, TimeInForce};
use bagholder_core::Dec;
use jiff::{SignedDuration, Timestamp};

use super::gate::{self, Held};
use super::{log, ticket_session, TicketQuoteDetail};

/// Why the brackets' quote cannot be acted on, this check, for the header (said
/// once the next check finds it too, `Confirmed`); `None` once it can.
fn quote_problem(app: &App, problem: Option<String>) {
    if app.orders.quote_problem.lock().unwrap_or_else(|e| e.into_inner()).check(problem) {
        app.events.signal();
    }
}
use crate::app::App;

pub const BRACKET_POLL_SEC: u64 = 5;
/// A quote older than this by its own time is not acted on.
pub const QUOTE_FRESH_SEC: i64 = 15;
/// Wealthsimple lets a good-till-cancelled order lapse ninety days after it is placed.
pub const GTC_DAYS: i64 = 90;
/// Steps a bracket takes in one check at most: a guard against a loop in the rules,
/// which they never reach.
const STEPS_PER_CHECK: usize = 12;

/// What the engine read of a listing's quote: the tape while the market is open and
/// the quote current, and whether the market is open.
#[derive(Clone, Debug, Default)]
pub struct Quoted {
    pub open: bool,
    pub tape: Option<Tape>,
    /// Why a quote the market being open should have given is not used.
    pub problem: Option<String>,
}

fn dec_of(x: Option<f64>) -> Option<Dec> {
    x.filter(|v| v.is_finite()).and_then(|v| Dec::parse(&format!("{v}")).ok())
}

/// A quote as the engine reads it (`SPEC.md` §6: the bid, the last): used only while
/// its market is open and only when its own time is current.
pub fn quoted(q: &TicketQuoteDetail, now: Timestamp) -> Quoted {
    let open = q.market_status.eq_ignore_ascii_case("OPEN");
    if !open {
        return Quoted { open, tape: None, problem: None };
    }
    let Some(last) = dec_of(q.last) else {
        return Quoted { open, tape: None, problem: Some("Wealthsimple's quote has no last price".into()) };
    };
    let at: Option<Timestamp> = q.quoted_as_of.parse().ok();
    match at {
        Some(t) if now.duration_since(t) <= SignedDuration::from_secs(QUOTE_FRESH_SEC) => Quoted { open, tape: Some(Tape { last, bid: dec_of(q.bid) }), problem: None },
        Some(t) => Quoted { open, tape: None, problem: Some(format!("Wealthsimple's quote is from {t}, not current")) },
        None => Quoted { open, tape: None, problem: Some(format!("Wealthsimple's quote states no time it is from ({:?})", q.quoted_as_of)) },
    }
}

/// The bracket's current exit as the broker last stated it.
pub fn exit_of(book: &Book, b: &Bracket) -> Result<Option<Exit>, String> {
    let Some((role, id)) = &b.exit else { return Ok(None) };
    let Some(o) = book.order(id).map_err(|e| e.to_string())? else { return Ok(None) };
    let log = book.order_log(id).map_err(|e| e.to_string())?;
    let cancel_asked_at = log.iter().filter(|l| l.refused.is_none() && matches!(l.event, OrderEvent::CancelAsked)).map(|l| l.at).max();
    let expires_at = o.expires_at.or_else(|| (o.request.time_in_force == TimeInForce::UntilCancel).then(|| o.created_at + SignedDuration::from_hours(24 * GTC_DAYS)));
    let mut fold = o.fold;
    if *role == ExitRole::Sale {
        // one sale from the ticket can hold the shares of several brackets: what it
        // filled is theirs in turn, by the brackets' ids, each up to its own part
        let mut left = fold.filled;
        for sb in book.live_brackets().map_err(|e| e.to_string())?.iter().filter(|x| x.bracket.exit.as_ref() == Some(&(ExitRole::Sale, id.clone()))) {
            let part = sb.bracket.exit_at.map(|(_, q)| q).unwrap_or(Dec::ZERO);
            let mine = if left < part { left } else { part };
            if sb.bracket.exit_at == b.exit_at && sb.bracket == *b {
                fold.filled = mine;
                break;
            }
            left = left.checked_sub(mine).unwrap_or(Dec::ZERO);
        }
    }
    Ok(Some(Exit { order_id: id.clone(), role: *role, fold, cancel_asked: cancel_asked_at.is_some(), cancel_asked_at, price: o.stated_price, quantity: o.stated_quantity, expires_at }))
}

/// How things stand for the bracket's shares, which a refusal that depends on them
/// is asked again on (`docs/decisions.md` 2026-10-04): the market open, the account's
/// units as the newest statement states them, and the orders working on the shares.
pub(crate) fn state_key(app: &Arc<App>, book: &Book, sb: &StoredBracket, open: bool) -> StateKey {
    use bagholder_core::account::AccountRef;
    use bagholder_core::instrument::{RefScheme, Reference};
    let ws = bagholder_core::Broker::named("wealthsimple");
    let units = (|| {
        let account = book.account_by_ref(&AccountRef::new(ws.clone(), sb.place.broker_account.clone())).ok()??;
        let instrument = book.instrument_by_ref(&Reference::new(RefScheme::BrokerSecurity(ws.clone()), sb.place.broker_security.clone())).ok()??;
        book.units_reads(account, instrument, Timestamp::UNIX_EPOCH).ok()?.into_iter().next().map(|(_, q)| q.unwrap_or(Dec::ZERO))
    })();
    let elsewhere = app.orders.elsewhere.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|e| e.ended_at.is_none() && e.account == sb.place.broker_account && e.security == sb.place.broker_security).count();
    let own = book.orders_in_flight().map(|o| o.iter().filter(|o| o.request.broker_account == sb.place.broker_account && o.request.broker_security == sb.place.broker_security).count()).ok();
    let working = own.map(|n| u32::try_from(n + elsewhere).unwrap_or(u32::MAX));
    StateKey { open, units, working }
}

/// The bracket's entry.
fn entry_of(book: &Book, id: &str) -> Result<Option<(String, OrderFold)>, String> {
    Ok(book
        .orders_of_bracket(id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|o| o.request.bracket.as_ref().is_some_and(|(_, r)| *r == OrderRole::Entry))
        .map(|o| (o.request.id, o.fold)))
}

/// When the bracket armed.
fn armed_at(book: &Book, id: &str) -> Result<Option<Timestamp>, String> {
    Ok(book.bracket_log(id).map_err(|e| e.to_string())?.iter().find(|l| l.refused.is_none() && matches!(l.event, BracketEvent::Armed { .. })).map(|l| l.at))
}

/// What the book says of the bracket's position since it armed: the units sold in
/// its account since then, and the newest statement of the account's units, with
/// whether it lists the position.
fn since_armed(book: &Book, app: &Arc<App>, sb: &StoredBracket, armed: Timestamp) -> Result<Option<(Dec, Option<(Timestamp, bool)>)>, String> {
    use bagholder_core::account::AccountRef;
    use bagholder_core::instrument::{RefScheme, Reference};
    use bagholder_core::transaction::Kind;
    let Some(f) = app.figures.get() else { return Ok(None) };
    let e = |e: bagholder_book::BookError| e.to_string();
    let ws = bagholder_core::Broker::named("wealthsimple");
    let Some(account) = book.account_by_ref(&AccountRef::new(ws.clone(), sb.place.broker_account.clone())).map_err(e)? else { return Ok(None) };
    let Some(instrument) = book.instrument_by_ref(&Reference::new(RefScheme::BrokerSecurity(ws), sb.place.broker_security.clone())).map_err(e)? else { return Ok(None) };
    let zone = book.zone().map_err(e)?.map(|z| z.zone);
    let sold = f
        .read(|eng| {
            eng.inputs()
                .ledger
                .transactions
                .iter()
                .filter(|t| t.account == account && t.instrument == Some(instrument) && t.kind == Kind::Sell)
                // an instant after the arming; a row with only a day, a day after the arming's
                .filter(|t| match (t.occurred_at, &zone) {
                    (Some(at), _) => at > armed,
                    (None, Some(z)) => t.trade_date > armed.to_zoned(z.clone()).date(),
                    (None, None) => false,
                })
                .filter_map(|t| t.quantity)
                .try_fold(Dec::ZERO, |sum, q| sum.checked_add(q.abs()))
        })
        .ok_or("the figures are not built yet")?
        .map_err(|e| e.to_string())?;
    let newest = book.units_reads(account, instrument, armed).map_err(e)?.into_iter().next().map(|(at, held)| (at, held.is_some_and(|h| h.is_positive())));
    Ok(Some((sold, newest)))
}

/// The sell a bracket places for its exit: a stop at the level, the limit at the
/// target, or a market sell, good till cancelled. A ticket's sale is the person's
/// order, never placed by a bracket.
pub fn exit_order(sb: &StoredBracket, role: ExitRole, price: Option<Dec>, quantity: Dec) -> Option<OrderRequest> {
    let id = format!("order-{}", crate::app::uuid4());
    let (kind, order_role, limit, stop) = match role {
        ExitRole::Stop => (OrderKind::Stop, OrderRole::Stop, None, price),
        ExitRole::Target => (OrderKind::Limit, OrderRole::Target, price, None),
        ExitRole::Market => (OrderKind::Market, OrderRole::Market, None, None),
        ExitRole::Sale => return None,
    };
    let mut o = OrderRequest {
        id,
        broker: sb.place.broker.clone(),
        broker_account: sb.place.broker_account.clone(),
        broker_security: sb.place.broker_security.clone(),
        symbol: sb.place.symbol.clone(),
        currency: sb.place.currency,
        side: Side::Sell,
        kind,
        quantity,
        limit_price: limit,
        stop_price: stop,
        time_in_force: TimeInForce::UntilCancel,
        bracket: Some((sb.place.id.clone(), order_role)),
        request: serde_json::Value::Null,
    };
    o.request = gate::request_record(&o);
    Some(o)
}

fn record(book: &Book, id: &str, asker: &Asker, now: Timestamp, e: &BracketEvent) -> Result<(), String> {
    if let Err(why) = book.bracket_event(id, asker, now, e).map_err(|e| e.to_string())? {
        log(&format!("bagholder bracket {id}: {why}"));
    }
    Ok(())
}

/// Record a step's events, and send what it asks for: who asked is on both. `key` is
/// how things stood, kept with a refusal that depends on them.
pub(crate) fn take_step(app: &Arc<App>, book: &Book, sb: &StoredBracket, step: &Step, asker: &Asker, now: Timestamp, key: &StateKey) -> Result<(), String> {
    for e in &step.events {
        record(book, &sb.place.id, asker, now, e)?;
        if let BracketEvent::OffBroker { role } = e {
            // a problem told: the bracket goes on, its stop watched here
            super::emit(app, "problems", &format!("bracket:{}:off-broker:{now}", sb.place.id), &format!("{} cancelled at Wealthsimple · {}", if *role == ExitRole::Target { "Take profit" } else { "Stop" }, sb.place.symbol), "Bagholder watches the stop level itself until you change or cancel the bracket");
        }
    }
    if let Some(r) = &step.request {
        let sb = book.bracket(&sb.place.id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no bracket {}", sb.place.id))?;
        send(app, book, &sb, r, asker, now, key)?;
    }
    Ok(())
}

/// Send what a step asks for, and record what came of it.
fn send(app: &Arc<App>, book: &Book, sb: &StoredBracket, request: &Request, asker: &Asker, now: Timestamp, key: &StateKey) -> Result<(), String> {
    let id = &sb.place.id;
    match request {
        Request::Place { role, price, quantity } => {
            let Some(o) = exit_order(sb, *role, *price, *quantity) else {
                log(&format!("bagholder bracket {id}: asked to place a {} it never places", role.as_str()));
                return Ok(());
            };
            match gate::place(app, book, &o, asker, now)? {
                Ok(fold) => {
                    let event = match fold.state {
                        OrderState::Rejected | OrderState::Failed => {
                            let why = fold.why.clone().unwrap_or_else(|| fold.state.as_str().into());
                            // failed: it never reached Wealthsimple's answer
                            let class = bracket::classify(fold.state == OrderState::Failed, fold.code.as_deref(), &why);
                            BracketEvent::Refused { why, code: fold.code.clone(), role: Some(*role), class, key: key.clone() }
                        }
                        _ => bracket::placed(request, &o.id).expect("a placement"),
                    };
                    log(&format!("bagholder bracket {id} for {}: {} {} {}", sb.place.symbol, role.as_str(), quantity, match &event {
                        BracketEvent::Refused { why, .. } => format!("refused: {why}"),
                        _ => format!("sent ({})", fold.state.as_str()),
                    }));
                    record(book, id, asker, now, &event)?;
                }
                Err(held) => held_back(app, book, sb, held, now)?,
            }
        }
        Request::Cancel { order_id } => match gate::cancel(app, book, order_id, asker, now)? {
            Ok(fold) => log(&format!("bagholder bracket {id} for {}: cancel of {order_id} sent ({})", sb.place.symbol, fold.state.as_str())),
            Err(held) => held_back(app, book, sb, held, now)?,
        },
        Request::Resize { order_id, quantity } => match gate::modify(app, book, order_id, None, Some(*quantity), asker, now)? {
            Ok(_) => {
                let refused = book.order_log(order_id).map_err(|e| e.to_string())?.last().is_some_and(|l| matches!(l.event, OrderEvent::ModifyRefused { .. }));
                if refused {
                    // a change the broker will not make: the exit is cancelled and placed
                    // again for the new quantity, the level watched here meanwhile
                    log(&format!("bagholder bracket {id} for {}: the change of {order_id} to {quantity} was refused; it is placed again", sb.place.symbol));
                    if let Err(held) = gate::cancel(app, book, order_id, asker, now)? {
                        held_back(app, book, sb, held, now)?;
                    }
                } else {
                    record(book, id, asker, now, &BracketEvent::Resized { quantity: *quantity })?;
                }
            }
            Err(held) => held_back(app, book, sb, held, now)?,
        },
    }
    Ok(())
}

fn held_back(app: &Arc<App>, book: &Book, sb: &StoredBracket, held: Held, now: Timestamp) -> Result<(), String> {
    match held {
        Held::Capped => {
            let why = format!("stopped: it sent {} orders in a minute", gate::BRACKET_CAP_PER_MINUTE);
            record(book, &sb.place.id, &Asker::Engine, now, &BracketEvent::Halted { why: why.clone() })?;
            super::emit(app, "problems", &format!("bracket:{}:halted", sb.place.id), &format!("Bracket stopped · {}", sb.place.symbol), &format!("It sent {} orders in a minute; nothing more is sent until you change or cancel it", gate::BRACKET_CAP_PER_MINUTE));
            log(&format!("bagholder bracket {} for {}: {why}", sb.place.id, sb.place.symbol));
        }
        Held::InFlight => log(&format!("bagholder bracket {} for {}: an exit is in flight; nothing more is placed", sb.place.id, sb.place.symbol)),
        Held::Dry => log(&format!("bagholder bracket {} for {}: orders are off; nothing is sent", sb.place.id, sb.place.symbol)),
        Held::NotNow(why) => log(&format!("bagholder bracket {} for {}: {why}", sb.place.id, sb.place.symbol)),
    }
    Ok(())
}

/// Whether the bracket's exit is read back on this check: whenever the broker may
/// still act on it, so its fill, a cancel or a change made by hand at the broker, and
/// the answer to a request, are all seen within a check (`SPEC.md` §6, Brackets).
fn awaited(x: &Exit) -> bool {
    x.fold.state.in_flight()
}

/// What stands in the way of a bracket's check, said in the header from the second
/// check of it in a row that meets it until a check goes through (`Confirmed`;
/// `SPEC.md` §4, the header); the log line stays beside it. `None` clears it.
pub(crate) fn trouble(app: &Arc<App>, id: &str, why: Option<String>) {
    let changed = {
        let mut t = app.orders.bracket_trouble.lock().unwrap_or_else(|e| e.into_inner());
        let cleared = why.is_none();
        let changed = t.entry(id.to_string()).or_default().check(why);
        if cleared {
            t.remove(id);
        }
        changed
    };
    if changed {
        app.events.signal();
    }
}

/// One bracket's check: what it waits on read back, then its steps until it has
/// nothing to do or a request is out. Taken under the bracket's lock. What went wrong
/// in it is said in the header until a check of the bracket goes through.
pub fn check_bracket(app: &Arc<App>, book: &Book, id: &str, quotes: &HashMap<String, Quoted>, now: Timestamp) -> Result<(), String> {
    let mut troubles = Vec::new();
    let checked = check_one(app, book, id, quotes, now, &mut troubles);
    if let Err(e) = &checked {
        troubles.push(format!("A bracket could not be checked: {e}"));
    }
    trouble(app, id, troubles.into_iter().next());
    checked
}

fn check_one(app: &Arc<App>, book: &Book, id: &str, quotes: &HashMap<String, Quoted>, now: Timestamp, troubles: &mut Vec<String>) -> Result<(), String> {
    let lock = gate::bracket_lock(app, id);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sb) = book.bracket(id).map_err(|e| e.to_string())? else { return Ok(()) };
    let mut said = |why: String| {
        log(&format!("bagholder bracket {id}: {why}"));
        troubles.push(format!("The bracket on {}: {why}", sb.place.symbol));
    };
    // what the bracket waits on is read from the broker first
    if super::orders_live(app) {
        // the entry while it works, armed or not: a bracket grows with its fills
        if let Some((entry, fold)) = entry_of(book, id)? {
            if fold.state.in_flight() {
                if let Err(e) = gate::read_back(app, book, &entry, now) {
                    said(format!("its entry could not be read back from Wealthsimple: {e}"));
                }
            }
        }
        if let Some(x) = exit_of(book, &sb.bracket)? {
            if awaited(&x) {
                if let Err(e) = gate::read_back(app, book, &x.order_id, now) {
                    said(format!("its exit could not be read back from Wealthsimple: {e}"));
                }
            }
        }
    }
    let quote = quotes.get(&sb.place.broker_security).cloned().unwrap_or_default();
    for _ in 0..STEPS_PER_CHECK {
        let Some(sb) = book.bracket(id).map_err(|e| e.to_string())? else { return Ok(()) };
        let b = &sb.bracket;
        let exit = exit_of(book, b)?;
        let entry_row = entry_of(book, id)?;
        let entry = entry_row.as_ref().map(|(_, f)| f.clone());
        let mut closed_elsewhere = None;
        if exit.is_none() && matches!(b.phase, Phase::Guarding | Phase::Target) {
            if let Some(armed) = armed_at(book, id)? {
                match since_armed(book, app, &sb, armed) {
                    Ok(Some((sold, newest))) => {
                        let (why, note) = bracket::closed_by_reads(b, sold, newest);
                        if let Some(note) = note {
                            record(book, id, &Asker::Engine, now, &note)?;
                            continue;
                        }
                        closed_elsewhere = why;
                    }
                    Ok(None) => {}
                    Err(e) => said(format!("what the book holds of it could not be read: {e}")),
                }
            }
        }
        let key = state_key(app, book, &sb, quote.open);
        let sale_running = app.orders.gate.selling.lock().unwrap_or_else(|e| e.into_inner()).contains(id);
        let seen = Seen { now, open: quote.open, tape: quote.tape, entry: entry.as_ref(), exit: exit.as_ref(), closed_elsewhere, key: key.clone(), sale_running, entry_order: entry_row.as_ref().map(|(id, _)| id.as_str()) };
        let step: Step = bracket::decide(b, &seen);
        if step.is_nothing() {
            return Ok(());
        }
        take_step(app, book, &sb, &step, &Asker::Engine, now, &key)?;
        if step.request.is_some() {
            return Ok(());
        }
    }
    said(format!("more than {STEPS_PER_CHECK} steps in one check; the rest wait for the next"));
    Ok(())
}

/// An exit of Bagholder's own in flight at the broker that no live bracket holds as
/// its current exit is cancelled: it would sell shares nothing watches.
fn sweep(app: &Arc<App>, book: &Book, now: Timestamp) -> Result<(), String> {
    for o in book.orders_in_flight().map_err(|e| e.to_string())? {
        let Some((bracket, role)) = &o.request.bracket else { continue };
        if *role == OrderRole::Entry || !matches!(o.fold.state, OrderState::Pending | OrderState::PartlyFilled) {
            continue;
        }
        let held = book.bracket(bracket).map_err(|e| e.to_string())?.is_some_and(|sb| sb.bracket.exit.as_ref().is_some_and(|(_, id)| *id == o.request.id));
        if held {
            continue;
        }
        log(&format!("bagholder bracket: {} for {} rests at Wealthsimple with no bracket holding it; cancelled", o.request.id, o.request.symbol));
        match gate::cancel(app, book, &o.request.id, &Asker::Engine, now)? {
            Ok(_) => {}
            // dry orders send nothing, an order no longer resting has nothing to cancel, and a
            // capped minute is asked again on the next sweep; a cancel is never held in flight
            Err(Held::Dry | Held::NotNow(_) | Held::Capped | Held::InFlight) => {}
        }
    }
    Ok(())
}

/// The quotes of the listings whose brackets act on the price.
fn quotes_for(app: &Arc<App>, live: &[StoredBracket], now: Timestamp) -> HashMap<String, Quoted> {
    // a halted bracket still sells when its stop level is reached: its quote is read too
    let mut ids: Vec<String> = live.iter().filter(|b| !matches!(b.bracket.phase, Phase::Waiting | Phase::Closing | Phase::Ended)).map(|b| b.place.broker_security.clone()).collect();
    ids.sort();
    ids.dedup();
    let mut out = HashMap::new();
    if ids.is_empty() {
        quote_problem(app, None);
        return out;
    }
    let sess = match ticket_session(app) {
        Ok(s) => s,
        Err(e) => {
            // no one signed in is said by the connection; a login that cannot be read is said here
            quote_problem(app, if e == super::tools::NOT_CONNECTED { None } else { Some(e) });
            return out;
        }
    };
    match super::fetch_quotes(app, &sess, &ids) {
        Ok(q) => {
            let mut problems = Vec::new();
            for id in &ids {
                let got = match q.get(id) {
                    Some(q) => quoted(q, now),
                    None => Quoted { open: false, tape: None, problem: Some("Wealthsimple sent no quote".into()) },
                };
                if let Some(p) = &got.problem {
                    let symbol = live.iter().find(|b| b.place.broker_security == *id).map(|b| b.place.symbol.clone()).unwrap_or_default();
                    problems.push(format!("{p} for {symbol}"));
                }
                out.insert(id.clone(), got);
            }
            quote_problem(app, (!problems.is_empty()).then(|| format!("{}; brackets do not act on it.", problems.join("; "))));
        }
        Err(e) => quote_problem(app, Some(format!("Wealthsimple's quotes for the brackets could not be read: {}", super::err_text(&e)))),
    }
    out
}

/// The calendar a bracket's listing trades on, from the book's own record of it;
/// none for a listing the book does not hold or a venue with no calendar held.
fn venue(book: &Book, sb: &StoredBracket) -> Option<bagholder_core::sessions::Venue> {
    use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
    let ws = bagholder_core::Broker::named("wealthsimple");
    let id = book.instrument_by_ref(&Reference::new(RefScheme::BrokerSecurity(ws), sb.place.broker_security.clone())).ok()??;
    let option = book.instrument(id).ok()?.kind == InstrumentKind::OptionContract;
    let mic = book.current_name(id).ok()?.and_then(|n| n.venue_mic).unwrap_or_default();
    bagholder_core::sessions::venue_of(&mic, option)
}

/// Whether a bracket rests at this check (`docs/plans/stage-money.md`, part A): its
/// venue's session is closed by the exchange's calendar, it was closed at the bracket's
/// last check too, and nothing of it is in flight (an entry working, a request whose
/// answer is not known, a cancel out). A session opening or closing is a check of its
/// own, so a resting exit is read once at each; a date past the published calendar
/// never rests.
pub(crate) fn resting(app: &Arc<App>, book: &Book, sb: &StoredBracket, now: Timestamp) -> bool {
    let open = venue(book, sb).and_then(|v| bagholder_core::sessions::in_session(v, now));
    let before = app.orders.session_seen.lock().unwrap_or_else(|e| e.into_inner()).insert(sb.place.id.clone(), open);
    open == Some(false) && before.flatten() == Some(false) && settled(book, sb)
}

/// Whether a bracket stands still: guarding or holding its target, its entry not
/// working, and its exit resting at the broker or none, no cancel out.
fn settled(book: &Book, sb: &StoredBracket) -> bool {
    if !matches!(sb.bracket.phase, Phase::Guarding | Phase::Target) {
        return false;
    }
    let entry_working = entry_of(book, &sb.place.id).ok().flatten().is_some_and(|(_, f)| f.state.in_flight());
    let exit_settled = exit_of(book, &sb.bracket).ok().flatten().is_none_or(|x| matches!(x.fold.state, OrderState::Pending | OrderState::PartlyFilled) && !x.cancel_asked);
    !entry_working && exit_settled
}

/// Whether the order read-back leaves a bracket's resting exit alone now: its venue's
/// session is closed and the bracket stands still. The bracket's own check reads it
/// when the session opens and closes.
pub(crate) fn exit_rests(book: &Book, bracket: &str, now: Timestamp) -> bool {
    let Ok(Some(sb)) = book.bracket(bracket) else { return false };
    venue(book, &sb).and_then(|v| bagholder_core::sessions::in_session(v, now)) == Some(false) && settled(book, &sb)
}

/// Every live bracket's check, then the sweep for exits nothing holds.
pub fn bracket_tick(app: &Arc<App>, quotes: Option<HashMap<String, Quoted>>, now: Timestamp) -> Result<usize, String> {
    let Some(f) = app.figures.get() else { return Ok(0) };
    let book = f.book()?;
    let all = book.live_brackets().map_err(|e| e.to_string())?;
    // between sessions a bracket with nothing in flight reads nothing and asks for no quote
    let live: Vec<StoredBracket> = all.iter().filter(|sb| !resting(app, &book, sb, now)).cloned().collect();
    // a bracket no longer live has nothing standing in its way
    let gone: Vec<String> = app.orders.bracket_trouble.lock().unwrap_or_else(|e| e.into_inner()).keys().filter(|k| !all.iter().any(|sb| &sb.place.id == *k)).cloned().collect();
    for id in gone {
        trouble(app, &id, None);
    }
    let quotes = quotes.unwrap_or_else(|| quotes_for(app, &live, now));
    // each bracket on its own thread with its own book connection: one slow broker
    // call holds up only its own bracket (brief 15 §2)
    std::thread::scope(|scope| {
        for sb in &live {
            let quotes = &quotes;
            scope.spawn(move || {
                let checked = f.book().and_then(|book| check_bracket(app, &book, &sb.place.id, quotes, now));
                if let Err(e) = checked {
                    log(&format!("bagholder bracket {}: {e}", sb.place.id));
                    trouble(app, &sb.place.id, Some(format!("The bracket on {} could not be checked: {e}", sb.place.symbol)));
                }
            });
        }
    });
    if super::orders_live(app) {
        sweep(app, &book, now)?;
    }
    Ok(all.len())
}

/// Whether the engine has anything to do: a live bracket, or an exit of Bagholder's
/// own in flight that a check may have to cancel.
fn bracket_work(app: &Arc<App>) -> bool {
    let Some(f) = app.figures.get() else { return false };
    let Ok(book) = f.book() else { return true };
    let brackets = book.live_brackets().map(|l| !l.is_empty()).unwrap_or(true);
    brackets || book.orders_in_flight().map(|o| o.iter().any(|o| o.request.bracket.as_ref().is_some_and(|(_, r)| *r != OrderRole::Entry))).unwrap_or(true)
}

pub fn bracket_loop(app: &Arc<App>) {
    while app.events.park_until(app, || bracket_work(app)) {
        if app.wait(Duration::from_secs(BRACKET_POLL_SEC)) {
            return;
        }
        // a live bracket not checked is said from the second check missed in a row, with why
        let blocked = super::tools::watch_blocked(app).map(|why| format!("Brackets are not being watched: {why}."));
        if app.orders.watch_problem.lock().unwrap_or_else(|e| e.into_inner()).check(blocked.clone()) {
            app.events.signal();
        }
        if blocked.is_some() {
            continue;
        }
        if let Err(e) = bracket_tick(app, None, Timestamp::now()) {
            log(&format!("bagholder bracket: the check failed: {e}"));
        }
    }
}

/// `POST /api/bracket/cancel`: the person cancels a bracket from the Orders panel: it
/// ends, its exit cancelled.
pub fn cancel_bracket(app: &Arc<App>, id: &str) -> super::OrderActionAnswer {
    use super::OrderActionAnswer as A;
    let run = || -> Result<A, String> {
        let f = app.figures.get().ok_or("The book is not open.")?;
        let book = f.book()?;
        let now = Timestamp::now();
        let lock = gate::bracket_lock(app, id);
        let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
        let Some(sb) = book.bracket(id).map_err(|e| e.to_string())? else { return Ok(A::err("No such bracket.")) };
        if !sb.bracket.phase.is_live() || sb.bracket.phase == Phase::Closing {
            return Ok(A::err("That bracket is not live."));
        }
        if !super::orders_live(app) {
            return Ok(A::err(super::ORDERS_OFF));
        }
        let exit = exit_of(&book, &sb.bracket)?;
        let step = bracket::end(exit.as_ref(), "cancelled by the user");
        take_step(app, &book, &sb, &step, &Asker::Person, now, &StateKey::default())?;
        super::ask_read(app);
        Ok(A::accepted(id))
    };
    run().unwrap_or_else(A::err)
}
