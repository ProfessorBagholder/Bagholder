//! Changing a resting order, and a leg of a bracket (`SPEC.md` §4, Orders: Edit).

use std::collections::HashMap;
use std::sync::Arc;

use bagholder_core::bracket::{self, Phase, StopLeg, Trail};
use bagholder_core::order::{Asker, OrderEvent, OrderKind, OrderRole, OrderState};
use bagholder_core::Dec;
use jiff::Timestamp;

use super::gate::{self, Held};
use super::preview::tick;
use super::{brackets, orders_live, OrderActionAnswer, PageDec, ORDERS_OFF};
use crate::app::App;

fn book_of(app: &Arc<App>) -> Result<bagholder_book::Book, String> {
    app.figures.get().ok_or("The book is not open.")?.book()
}

/// What the order stands at now: the broker's own statement where it made one, else
/// the newest change asked for that was not refused, else what it was placed with.
fn standing(o: &bagholder_book::orders::StoredOrder, log: &[bagholder_book::orders::Logged<OrderEvent>]) -> (Dec, Option<Dec>) {
    let mut qty = o.request.quantity;
    let mut limit = o.request.limit_price;
    for l in log.iter().filter(|l| l.refused.is_none()) {
        if let OrderEvent::ModifyAsked { limit_price, quantity } = &l.event {
            qty = quantity.unwrap_or(qty);
            limit = limit_price.or(limit);
        }
    }
    if let Some(q) = o.stated_quantity {
        qty = q;
    }
    // a limit order's stated price is its limit; a stop limit's is its stop
    if o.request.kind == OrderKind::Limit {
        limit = o.stated_price.or(limit);
    }
    (qty, limit)
}

/// `POST /api/order/modify`: change a resting order's size or its limit. A stop order
/// cannot be changed at Wealthsimple, only cancelled and placed again.
pub fn modify_order(app: &Arc<App>, order_id: &str, quantity: &PageDec, limit_price: &PageDec) -> OrderActionAnswer {
    match modify(app, order_id, quantity, limit_price) {
        Ok(a) => a,
        Err(e) => OrderActionAnswer::err(e),
    }
}

fn modify(app: &Arc<App>, order_id: &str, quantity: &PageDec, limit_price: &PageDec) -> Result<OrderActionAnswer, String> {
    let book = book_of(app)?;
    let Some(o) = book.order(order_id).map_err(|e| e.to_string())? else { return modify_elsewhere(app, order_id, quantity, limit_price) };
    if o.request.bracket.as_ref().is_some_and(|(_, r)| *r != OrderRole::Entry) {
        return Ok(OrderActionAnswer::err("That order is a bracket's; change the bracket."));
    }
    if !matches!(o.fold.state, OrderState::Pending | OrderState::PartlyFilled) {
        return Ok(OrderActionAnswer::err("That order is not open."));
    }
    if o.request.kind == OrderKind::Stop {
        return Ok(OrderActionAnswer::err("A stop order cannot be changed; cancel it and place another."));
    }
    let q = quantity.0.clone().map_err(|t| format!("Shares {t:?} is not a number."))?;
    let lp = limit_price.0.clone().map_err(|t| format!("The limit price {t:?} is not a number."))?.map(tick);
    if q.is_some_and(|x| !x.is_positive()) {
        return Ok(OrderActionAnswer::err("Shares must be more than zero."));
    }
    if lp.is_some_and(|x| !x.is_positive()) {
        return Ok(OrderActionAnswer::err("A limit price must be more than zero."));
    }
    let limit_type = matches!(o.request.kind, OrderKind::Limit | OrderKind::StopLimit);
    let log = book.order_log(order_id).map_err(|e| e.to_string())?;
    let (now_qty, now_limit) = standing(&o, &log);
    // only what differs from the order as it rests is asked for
    let new_limit = lp.filter(|p| limit_type && Some(*p) != now_limit);
    let new_quantity = q.filter(|x| *x != now_qty);
    if new_limit.is_none() && new_quantity.is_none() {
        return Ok(OrderActionAnswer::already(order_id));
    }
    if !orders_live(app) {
        return Ok(OrderActionAnswer::err(ORDERS_OFF));
    }
    match gate::modify(app, &book, order_id, new_limit, new_quantity, &Asker::Person, Timestamp::now())? {
        Ok(_) => {}
        Err(Held::Dry) => return Ok(OrderActionAnswer::err(ORDERS_OFF)),
        Err(held) => return Ok(OrderActionAnswer::err(format!("Not sent: {held:?}."))),
    }
    let last = book.order_log(order_id).map_err(|e| e.to_string())?.pop();
    if let Some(bagholder_book::orders::Logged { event: OrderEvent::ModifyRefused { why }, .. }) = last {
        return Ok(OrderActionAnswer::err(if why.is_empty() { "Wealthsimple refused the change.".to_string() } else { format!("Wealthsimple refused the change: {why}") }));
    }
    super::ask_read(app);
    Ok(OrderActionAnswer::accepted(order_id))
}

/// `POST /api/bracket/adjust`: move a leg of a live bracket, give a trailing stop
/// another trail, or take a leg off. A leg that rests at Wealthsimple at another level
/// is cancelled and placed again at the new one by the bracket's next step; a bracket
/// left with no leg ends.
pub fn adjust_bracket(app: &Arc<App>, bracket_id: &str, leg: &str, price: &PageDec, trail: &PageDec, remove: bool) -> OrderActionAnswer {
    match adjust(app, bracket_id, leg, price, trail, remove) {
        Ok(a) => a,
        Err(e) => OrderActionAnswer::err(e),
    }
}

fn adjust(app: &Arc<App>, bracket_id: &str, leg: &str, price: &PageDec, trail: &PageDec, remove: bool) -> Result<OrderActionAnswer, String> {
    let book = book_of(app)?;
    let now = Timestamp::now();
    {
        let lock = gate::bracket_lock(app, bracket_id);
        let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
        let Some(sb) = book.bracket(bracket_id).map_err(|e| e.to_string())? else { return Ok(OrderActionAnswer::err("No such bracket.")) };
        let b = &sb.bracket;
        if !b.phase.is_live() || matches!(b.phase, Phase::Closing | Phase::ClosingForSale) {
            return Ok(OrderActionAnswer::err("That bracket is not live."));
        }
        // with orders off the bracket is not changed here either: its resting orders
        // could not be cancelled, and a bracket changed only here would say something
        // Wealthsimple does not hold
        if !orders_live(app) {
            return Ok(OrderActionAnswer::err(ORDERS_OFF));
        }
        let stop_leg = match leg.to_lowercase().as_str() {
            "sl" => true,
            "tp" => false,
            _ => return Ok(OrderActionAnswer::err("Which leg?")),
        };
        let positive = |v: &PageDec, name: &str| -> Result<Option<Dec>, String> { Ok(v.0.clone().map_err(|t| format!("{name} {t:?} is not a number."))?.filter(|x| x.is_positive())) };
        let (mut stop, mut target) = (b.stop, b.target);
        if remove {
            if stop_leg {
                stop = None;
            } else {
                target = None;
            }
        } else if stop_leg {
            let Some(current) = b.stop else { return Ok(OrderActionAnswer::err("This bracket has no stop loss.")) };
            stop = Some(match current.trail {
                Some(t) => {
                    let Some(distance) = positive(trail, "The trail")? else { return Ok(OrderActionAnswer::err("A trail is required.")) };
                    let t = match t {
                        Trail::Pct(_) => Trail::Pct(distance),
                        Trail::Amount(_) => Trail::Amount(distance),
                    };
                    // the level under the high so far; before the fill there is none yet
                    let level = current.high.map_or(current.level, |h| StopLeg::trailed(t, h));
                    StopLeg { level, trail: Some(t), high: current.high }
                }
                None => {
                    let Some(p) = positive(price, "The stop price")? else { return Ok(OrderActionAnswer::err("A stop price is required.")) };
                    StopLeg { level: tick(p), trail: None, high: None }
                }
            });
        } else {
            let Some(p) = positive(price, "The limit price")? else { return Ok(OrderActionAnswer::err("A limit price is required.")) };
            target = Some(tick(p));
        }
        let exit = brackets::exit_of(&book, b)?;
        let step = bracket::adjust(exit.as_ref(), stop, target);
        brackets::take_step(app, &book, &sb, &step, &Asker::Person, now)?;
        crate::app::log(&format!("bagholder bracket {bracket_id} for {}: {} {} by the user", sb.place.symbol, if stop_leg { "stop loss" } else { "take profit" }, if remove { "removed" } else { "moved" }));
    }
    // the step that follows the change (a resting leg cancelled to be placed again) goes out now
    brackets::check_bracket(app, &book, bracket_id, &HashMap::new(), now)?;
    Ok(OrderActionAnswer::accepted(bracket_id))
}

/// The person changes an order placed in Wealthsimple's own app, shown from the feed.
fn modify_elsewhere(app: &Arc<App>, order_id: &str, quantity: &PageDec, limit_price: &PageDec) -> Result<OrderActionAnswer, String> {
    let Some(e) = app.orders.elsewhere.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|e| e.id == order_id).cloned() else { return Ok(OrderActionAnswer::err("No such order.")) };
    if !matches!(e.state, OrderState::Pending | OrderState::PartlyFilled) {
        return Ok(OrderActionAnswer::err("That order is not open."));
    }
    if e.kind == OrderKind::Stop {
        return Ok(OrderActionAnswer::err("A stop order cannot be changed; cancel it and place another."));
    }
    let q = quantity.0.clone().map_err(|t| format!("Shares {t:?} is not a number."))?;
    let lp = limit_price.0.clone().map_err(|t| format!("The limit price {t:?} is not a number."))?.map(tick);
    if q.is_some_and(|x| !x.is_positive()) {
        return Ok(OrderActionAnswer::err("Shares must be more than zero."));
    }
    if lp.is_some_and(|x| !x.is_positive()) {
        return Ok(OrderActionAnswer::err("A limit price must be more than zero."));
    }
    let limit_type = matches!(e.kind, OrderKind::Limit | OrderKind::StopLimit);
    let new_limit = lp.filter(|p| limit_type && Some(*p) != e.limit_price);
    let new_quantity = q.filter(|x| *x != e.quantity);
    if new_limit.is_none() && new_quantity.is_none() {
        return Ok(OrderActionAnswer::already(order_id));
    }
    match gate::modify_elsewhere(app, order_id, new_limit, new_quantity) {
        Err(_) => Ok(OrderActionAnswer::err(ORDERS_OFF)),
        Ok(gate::Sent::Refused { why, .. }) | Ok(gate::Sent::NotSent { why }) => Ok(OrderActionAnswer::err(if why.is_empty() { "Wealthsimple refused the change.".to_string() } else { format!("Wealthsimple refused the change: {why}") })),
        Ok(_) => {
            if let Some(x) = app.orders.elsewhere.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().find(|x| x.id == order_id) {
                x.limit_price = new_limit.or(x.limit_price);
                x.quantity = new_quantity.unwrap_or(x.quantity);
            }
            super::ask_read(app);
            Ok(OrderActionAnswer::accepted(order_id))
        }
    }
}
