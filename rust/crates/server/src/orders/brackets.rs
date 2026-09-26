//! The bracket engine (`SPEC.md` §6, Brackets after the fill): every five seconds
//! while the app runs connected, each live bracket reads back what it is waiting on,
//! takes its steps (`bagholder_core::bracket::decide`) against the broker's quote,
//! records each in its log, and sends what a step asks for through the gate. One
//! bracket at a time takes its steps (`gate::bracket_lock`), whoever asks: the
//! engine, the ticket or the Orders panel.

use std::collections::HashMap;
use std::sync::Arc;

use bagholder_book::orders::{OrderRequest, StoredBracket};
use bagholder_book::Book;
use bagholder_core::bracket::{self, Bracket, BracketEvent, Exit, ExitRole, Phase, Request, Seen, Step, Tape};
use bagholder_core::order::{Asker, OrderEvent, OrderFold, OrderKind, OrderRole, OrderState, Side, TimeInForce};
use bagholder_core::Dec;
use jiff::{SignedDuration, Timestamp};
use serde_json::json;

use super::gate::{self, Held};
use super::{log, orders_can_run, ticket_session, TicketQuoteDetail};
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
    let cancel_asked = book.order_log(id).map_err(|e| e.to_string())?.iter().any(|l| l.refused.is_none() && matches!(l.event, OrderEvent::CancelAsked));
    let expires_at = o.expires_at.or_else(|| (o.request.time_in_force == TimeInForce::UntilCancel).then(|| o.created_at + SignedDuration::from_hours(24 * GTC_DAYS)));
    Ok(Some(Exit { order_id: id.clone(), role: *role, fold: o.fold, cancel_asked, price: o.stated_price, quantity: o.stated_quantity, expires_at }))
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
/// target, or a market sell, good till cancelled.
pub fn exit_order(sb: &StoredBracket, role: ExitRole, price: Option<Dec>, quantity: Dec) -> OrderRequest {
    let id = format!("order-{}", crate::app::uuid4());
    let (kind, order_role, limit, stop) = match role {
        ExitRole::Stop => (OrderKind::Stop, OrderRole::Stop, None, price),
        ExitRole::Target => (OrderKind::Limit, OrderRole::Target, price, None),
        ExitRole::Market => (OrderKind::Market, OrderRole::Market, None, None),
    };
    let mut request = json!({
        "canonicalAccountId": sb.place.broker_account,
        "externalId": id,
        "executionType": super::ws_execution(kind),
        "orderType": "SELL_QUANTITY",
        "quantity": quantity.to_f64(),
        "securityId": sb.place.broker_security,
        "timeInForce": "UNTIL_CANCEL",
    });
    if let Some(p) = limit {
        request["limitPrice"] = json!(p.to_f64());
    }
    if let Some(p) = stop {
        request["stopPrice"] = json!(p.to_f64());
    }
    OrderRequest {
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
        request,
    }
}

fn record(book: &Book, id: &str, asker: &Asker, now: Timestamp, e: &BracketEvent) -> Result<(), String> {
    if let Err(why) = book.bracket_event(id, asker, now, e).map_err(|e| e.to_string())? {
        log(&format!("bagholder bracket {id}: {why}"));
    }
    Ok(())
}

/// Send what a step asks for, and record what came of it.
fn send(app: &Arc<App>, book: &Book, sb: &StoredBracket, request: &Request, now: Timestamp) -> Result<(), String> {
    let id = &sb.place.id;
    match request {
        Request::Place { role, price, quantity } => {
            let o = exit_order(sb, *role, *price, *quantity);
            match gate::place(app, book, &o, &Asker::Engine, now)? {
                Ok(fold) => {
                    let event = match fold.state {
                        OrderState::Rejected | OrderState::Failed => BracketEvent::Refused { why: fold.why.clone().unwrap_or_else(|| fold.state.as_str().into()), code: fold.code.clone() },
                        _ => bracket::placed(request, &o.id).expect("a placement"),
                    };
                    log(&format!("bagholder bracket {id} for {}: {} {} {}", sb.place.symbol, role.as_str(), quantity, match &event {
                        BracketEvent::Refused { why, .. } => format!("refused: {why}"),
                        _ => format!("sent ({})", fold.state.as_str()),
                    }));
                    record(book, id, &Asker::Engine, now, &event)?;
                }
                Err(held) => held_back(app, book, sb, held, now)?,
            }
        }
        Request::Cancel { order_id } => match gate::cancel(app, book, order_id, &Asker::Engine, now)? {
            Ok(fold) => log(&format!("bagholder bracket {id} for {}: cancel of {order_id} sent ({})", sb.place.symbol, fold.state.as_str())),
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

/// Whether the broker's answer on the bracket's order is awaited: its request is out
/// and not answered, or the phase is waiting on it.
fn awaited(phase: Phase, x: &Exit) -> bool {
    x.fold.state.in_flight() && (matches!(x.fold.state, OrderState::Sending | OrderState::Unconfirmed | OrderState::Cancelling) || matches!(phase, Phase::ToTarget | Phase::BackToStop | Phase::ToMarket | Phase::Firing | Phase::ClosingForSale | Phase::Closing))
}

/// One bracket's check: what it waits on read back, then its steps until it has
/// nothing to do or a request is out. Taken under the bracket's lock.
pub fn check_bracket(app: &Arc<App>, book: &Book, id: &str, quotes: &HashMap<String, Quoted>, now: Timestamp) -> Result<(), String> {
    let lock = gate::bracket_lock(app, id);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sb) = book.bracket(id).map_err(|e| e.to_string())? else { return Ok(()) };
    // what the bracket waits on is read from the broker first
    if super::orders_live() {
        if sb.bracket.phase == Phase::Waiting {
            if let Some((entry, fold)) = entry_of(book, id)? {
                if fold.state.in_flight() {
                    if let Err(e) = gate::read_back(app, book, &entry, now) {
                        log(&format!("bagholder bracket {id}: the entry could not be read back: {e}"));
                    }
                }
            }
        } else if let Some(x) = exit_of(book, &sb.bracket)? {
            if awaited(sb.bracket.phase, &x) {
                if let Err(e) = gate::read_back(app, book, &x.order_id, now) {
                    log(&format!("bagholder bracket {id}: its exit could not be read back: {e}"));
                }
            }
        }
    }
    let quote = quotes.get(&sb.place.broker_security).cloned().unwrap_or_default();
    for _ in 0..STEPS_PER_CHECK {
        let Some(sb) = book.bracket(id).map_err(|e| e.to_string())? else { return Ok(()) };
        let b = &sb.bracket;
        let exit = exit_of(book, b)?;
        let entry = entry_of(book, id)?.map(|(_, f)| f);
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
                    Err(e) => log(&format!("bagholder bracket {id}: what the book holds could not be read: {e}")),
                }
            }
        }
        let stop_allowed = b.phase == Phase::Waiting && super::stop_allowed(app, &sb.place.broker_security);
        let seen = Seen { now, open: quote.open, tape: quote.tape, entry: entry.as_ref(), exit: exit.as_ref(), closed_elsewhere, stop_allowed };
        let step: Step = bracket::decide(b, &seen);
        if step.is_nothing() {
            return Ok(());
        }
        for e in &step.events {
            record(book, id, &Asker::Engine, now, e)?;
        }
        if let Some(r) = &step.request {
            let sb = book.bracket(id).map_err(|e| e.to_string())?.expect("the bracket");
            send(app, book, &sb, r, now)?;
            return Ok(());
        }
    }
    log(&format!("bagholder bracket {id}: more than {STEPS_PER_CHECK} steps in one check; the rest wait for the next"));
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
        let _ = gate::cancel(app, book, &o.request.id, &Asker::Engine, now)?;
    }
    Ok(())
}

/// The quotes of the listings whose brackets act on the price.
fn quotes_for(app: &Arc<App>, live: &[StoredBracket], now: Timestamp) -> HashMap<String, Quoted> {
    let mut ids: Vec<String> = live.iter().filter(|b| !matches!(b.bracket.phase, Phase::Waiting | Phase::Closing | Phase::Halted | Phase::Ended)).map(|b| b.place.broker_security.clone()).collect();
    ids.sort();
    ids.dedup();
    let mut out = HashMap::new();
    if ids.is_empty() {
        super::quote_problem(app, None);
        return out;
    }
    let Some(sess) = ticket_session(app) else { return out };
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
            super::quote_problem(app, (!problems.is_empty()).then(|| format!("{}; brackets do not act on it.", problems.join("; "))));
        }
        Err(e) => super::quote_problem(app, Some(format!("Wealthsimple's quotes for the brackets could not be read: {}", super::err_text(&e)))),
    }
    out
}

/// Every live bracket's check, then the sweep for exits nothing holds.
pub fn bracket_tick(app: &Arc<App>, quotes: Option<HashMap<String, Quoted>>, now: Timestamp) -> Result<usize, String> {
    let Some(f) = app.figures.get() else { return Ok(0) };
    let book = f.book()?;
    let live = book.live_brackets().map_err(|e| e.to_string())?;
    let quotes = quotes.unwrap_or_else(|| quotes_for(app, &live, now));
    for sb in &live {
        if let Err(e) = check_bracket(app, &book, &sb.place.id, &quotes, now) {
            log(&format!("bagholder bracket {}: {e}", sb.place.id));
        }
    }
    if super::orders_live() {
        sweep(app, &book, now)?;
    }
    Ok(live.len())
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
        if app.wait(std::time::Duration::from_secs(BRACKET_POLL_SEC)) {
            return;
        }
        if !orders_can_run(app) {
            continue;
        }
        if let Err(e) = bracket_tick(app, None, Timestamp::now()) {
            log(&format!("bagholder bracket: the check failed: {e}"));
        }
    }
}

/// The person cancels a bracket from the Orders panel: it ends, its exit cancelled.
pub fn cancel_bracket(app: &Arc<App>, id: &str, asker: &Asker) -> Result<(), String> {
    if !super::orders_live() {
        return Err(super::ORDERS_OFF.into());
    }
    let f = app.figures.get().ok_or("the book is not open")?;
    let book = f.book()?;
    let now = Timestamp::now();
    let lock = gate::bracket_lock(app, id);
    let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
    let sb = book.bracket(id).map_err(|e| e.to_string())?.ok_or("No such bracket.")?;
    if !sb.bracket.phase.is_live() || sb.bracket.phase == Phase::Closing {
        return Err("That bracket is not live.".into());
    }
    let exit = exit_of(&book, &sb.bracket)?;
    let step = bracket::end(exit.as_ref(), "cancelled by the user");
    for e in &step.events {
        record(&book, id, asker, now, e)?;
    }
    if let Some(r) = &step.request {
        send(app, &book, &sb, r, now)?;
    }
    Ok(())
}
