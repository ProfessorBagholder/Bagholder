//! Carrying the earlier app's orders and brackets into the book, once
//! (`docs/plans/stage-4-execution.md`, "Migration"): each enters with an `imported`
//! first event that states where it stood and keeps the old row as it was, so an
//! order still working at Wealthsimple and a bracket still guarding a position are
//! followed from the first check after the upgrade. The file is snapshotted first,
//! and the old tables are dropped once the book holds every row; a row the import
//! cannot read stops it, naming the row, and the old tables stay.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use bagholder_book::orders::{BracketPlace, OrderRequest};
use bagholder_book::Book;
use bagholder_core::bracket::{BracketEvent, ExitRole, Phase, StopLeg, Trail};
use bagholder_core::order::{OrderEvent, OrderKind, OrderRole, OrderState, Side, TimeInForce};
use bagholder_core::{Currency, Dec};
use bagholder_store::orders::{Bracket as OldBracket, BracketStatus, Order as OldOrder, OrderStatus, OrderType, Role, SlKind, SlMode, Source, TrailUnit};

/// What the import carried.
#[derive(Debug, Default, PartialEq)]
pub struct Carried {
    pub snapshot: Option<PathBuf>,
    pub orders: usize,
    pub brackets: usize,
    /// Orders Wealthsimple reported from its own app: not the app's, so not carried
    /// (the feed finds any still pending).
    pub left_to_the_feed: usize,
}

fn dec(v: Option<f64>, what: &str, id: &str) -> Result<Option<Dec>, String> {
    match v {
        None => Ok(None),
        Some(x) if !x.is_finite() => Err(format!("{id}: {what} is not a number")),
        // a float's shortest text is the decimal it was written from
        Some(x) => Dec::parse(&format!("{x}")).map(Some).map_err(|e| format!("{id}: {what} {x}: {e}")),
    }
}

fn instant(t: &str, what: &str, id: &str) -> Result<jiff::Timestamp, String> {
    t.parse().map_err(|e: jiff::Error| format!("{id}: {what} {t:?}: {e}"))
}

/// When a row last changed: its own time, or when it was written where it states none.
fn changed(t: &str, created: jiff::Timestamp, id: &str) -> Result<jiff::Timestamp, String> {
    if t.trim().is_empty() {
        Ok(created)
    } else {
        instant(t, "updated_at", id)
    }
}

fn old_state(o: &OldOrder) -> Result<OrderState, String> {
    let filled = o.filled_qty.is_some_and(|q| q > 0.0);
    Ok(match o.status {
        OrderStatus::Dry => OrderState::Dry,
        // it may or may not have reached Wealthsimple: read back before anything else
        OrderStatus::Sending => OrderState::Unconfirmed,
        OrderStatus::Sent | OrderStatus::Pending if filled => OrderState::PartlyFilled,
        OrderStatus::Sent | OrderStatus::Pending => OrderState::Pending,
        OrderStatus::Cancelling => OrderState::Cancelling,
        OrderStatus::Filled => OrderState::Filled,
        OrderStatus::Cancelled => OrderState::Cancelled,
        OrderStatus::Expired => OrderState::Expired,
        OrderStatus::Rejected => OrderState::Rejected,
        OrderStatus::Failed => OrderState::Failed,
        OrderStatus::Unset => return Err(format!("{}: an order with no status", o.id)),
    })
}

fn old_kind(k: OrderType, id: &str) -> Result<OrderKind, String> {
    Ok(match k {
        OrderType::Market => OrderKind::Market,
        OrderType::Limit => OrderKind::Limit,
        OrderType::Stop => OrderKind::Stop,
        OrderType::StopLimit => OrderKind::StopLimit,
        OrderType::Unset => return Err(format!("{id}: an order with no type")),
    })
}

/// An old order as the book writes it, and where it stood.
fn order_of(o: &OldOrder, bracket: Option<(String, OrderRole)>) -> Result<(OrderRequest, OrderEvent, jiff::Timestamp, jiff::Timestamp), String> {
    let id = &o.id;
    let side = match o.side {
        bagholder_store::orders::Side::Buy => Side::Buy,
        bagholder_store::orders::Side::Sell => Side::Sell,
        bagholder_store::orders::Side::Unset => return Err(format!("{id}: an order with no side")),
    };
    let quantity = dec(o.quantity, "the quantity", id)?.filter(|q| q.is_positive()).ok_or_else(|| format!("{id}: no quantity"))?;
    let currency = Currency::parse(&o.currency.to_uppercase()).map_err(|_| format!("{id}: the currency {:?}", o.currency))?;
    let row = serde_json::to_string(o).map_err(|e| format!("{id}: {e}"))?;
    let first = OrderEvent::Imported {
        state: old_state(o)?,
        broker_id: Some(o.ws_order_id.clone()).filter(|w| !w.is_empty()),
        filled: dec(o.filled_qty, "the filled quantity", id)?.unwrap_or(Dec::ZERO),
        average: dec(o.avg_fill, "the average fill", id)?,
        why: Some(o.error.clone()).filter(|e| !e.is_empty()),
        row,
    };
    let request = OrderRequest {
        id: id.clone(),
        broker: "wealthsimple".into(),
        broker_account: o.account_id.clone(),
        broker_security: o.security_id.clone(),
        symbol: o.symbol.clone(),
        currency,
        side,
        kind: old_kind(o.kind, id)?,
        quantity,
        limit_price: dec(o.limit_price, "the limit price", id)?,
        stop_price: dec(o.stop_price, "the stop price", id)?,
        time_in_force: if o.tif.eq_ignore_ascii_case("UNTIL_CANCEL") { TimeInForce::UntilCancel } else { TimeInForce::Day },
        bracket,
        request: o.request.clone(),
    };
    let created = instant(&o.created_at, "created_at", id)?;
    Ok((request, first, created, changed(&o.updated_at, created, id)?))
}

/// An old bracket as the book writes it: its legs, and where it stood, with the one
/// exit of its own still at Wealthsimple as its current exit.
fn bracket_of(b: &OldBracket, exits: &[&OldOrder]) -> Result<(BracketPlace, BracketEvent, jiff::Timestamp, jiff::Timestamp), String> {
    let id = &b.id;
    let quantity = dec(b.quantity, "the quantity", id)?.filter(|q| q.is_positive()).ok_or_else(|| format!("{id}: no quantity"))?;
    let stop = match b.sl_kind {
        SlKind::Unset => None,
        kind => {
            let trail = match (kind, dec(b.sl_trail, "the trail", id)?) {
                (SlKind::Trail, Some(t)) if b.sl_trail_unit == TrailUnit::Amt => Some(Trail::Amount(t)),
                (SlKind::Trail, Some(t)) => Some(Trail::Pct(t)),
                (SlKind::Trail, None) => return Err(format!("{id}: a trailing stop with no trail")),
                _ => None,
            };
            let high = dec(b.high_water, "the high", id)?;
            let level = match (dec(b.sl_price, "the stop price", id)?, trail, high) {
                (Some(l), _, _) => l,
                (None, Some(t), Some(h)) => StopLeg::trailed(t, h),
                _ => return Err(format!("{id}: a stop with no level")),
            };
            Some(StopLeg { level, trail, high })
        }
    };
    let target = dec(b.tp_price, "the target", id)?;
    // the exit of its own the broker may still act on, as the bracket's current exit
    let live: Vec<&&OldOrder> = exits.iter().filter(|o| matches!(o.status, OrderStatus::Sending | OrderStatus::Sent | OrderStatus::Pending | OrderStatus::Cancelling)).collect();
    if live.len() > 1 {
        return Err(format!("{id}: {} exits of its own at Wealthsimple at once", live.len()));
    }
    let exit = live.first().map(|o| -> Result<(ExitRole, String), String> {
        let role = match (o.role, old_kind(o.kind, &o.id)?) {
            (_, OrderKind::Market) => ExitRole::Market,
            (Role::Target, _) | (_, OrderKind::Limit) => ExitRole::Target,
            _ => ExitRole::Stop,
        };
        Ok((role, o.id.clone()))
    });
    let exit = exit.transpose()?;
    let phase = match b.status {
        BracketStatus::Waiting => Phase::Waiting,
        BracketStatus::Armed => Phase::Guarding,
        BracketStatus::TargetPlaced => Phase::Target,
        BracketStatus::Firing => Phase::Firing,
        // the target's cancel out, a market sell to follow
        BracketStatus::Stopping => Phase::ToMarket,
        BracketStatus::Closing => Phase::Closing,
        BracketStatus::Done | BracketStatus::Cancelled => Phase::Ended,
        BracketStatus::Unset => return Err(format!("{id}: a bracket with no status")),
    };
    // an ended one keeps no exit; a waiting one has none yet
    let exit = if matches!(phase, Phase::Ended | Phase::Waiting) { None } else { exit };
    let currency = Currency::parse(&b.currency.to_uppercase()).map_err(|_| format!("{id}: the currency {:?}", b.currency))?;
    let place = BracketPlace { id: id.clone(), broker: "wealthsimple".into(), broker_account: b.account_id.clone(), broker_security: b.security_id.clone(), symbol: b.symbol.clone(), currency };
    let first = BracketEvent::Imported {
        phase,
        quantity,
        stop,
        target,
        native: b.sl_mode == SlMode::Native || (b.sl_mode == SlMode::Unset && b.sl_native),
        exit,
        attempts: u32::try_from(b.attempts.max(0)).unwrap_or(u32::MAX),
        why: Some(b.error.clone()).filter(|e| !e.is_empty()),
        outcome: Some(b.outcome.clone()).filter(|o| !o.is_empty()),
        seen_held: b.seen_held,
        row: serde_json::to_string(b).map_err(|e| format!("{id}: {e}"))?,
    };
    let created = instant(&b.created_at, "created_at", id)?;
    Ok((place, first, created, changed(&b.updated_at, created, id)?))
}

/// Carry the earlier store's orders and brackets into the book once, then drop its
/// order tables, the file snapshotted beside the book's own snapshots first. Nothing
/// when they were carried already.
pub fn carry_orders(home: &Path, conn: &Connection, book: &Book, at: jiff::Timestamp) -> Result<Carried, String> {
    let e = |e: rusqlite::Error| e.to_string();
    if bagholder_store::schema::orders_moved(conn).map_err(e)? {
        return Ok(Carried::default());
    }
    let orders = bagholder_store::orders::typed::list_orders(conn, i64::MAX).map_err(e)?;
    let brackets = bagholder_store::orders::typed::list_brackets(conn, &[]).map_err(e)?;
    let mut carried = Carried::default();
    if !orders.is_empty() || !brackets.is_empty() {
        let dir = home.join("snapshots");
        std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        let snapshot = dir.join(format!("bagholder-before-the-orders-{}.db", at.as_millisecond()));
        bagholder_book::import::copy_database(&home.join("bagholder.db"), &snapshot).map_err(|e| e.to_string())?;
        carried.snapshot = Some(snapshot);
    }
    // which bracket each order belongs to: an entry by its id, an exit by its parent's
    let mut bracket_of_entry: HashMap<&str, &str> = HashMap::new();
    for b in &brackets {
        bracket_of_entry.insert(b.order_id.as_str(), b.id.as_str());
    }
    let own = |o: &&OldOrder| o.source == Source::Bagholder;
    // every row read first: one the import cannot read stops it before anything is written
    let mut new_brackets = Vec::new();
    for b in &brackets {
        if book.bracket(&b.id).map_err(|e| e.to_string())?.is_some() {
            continue;
        }
        let exits: Vec<&OldOrder> = orders.iter().filter(|o| o.parent_id == b.order_id && matches!(o.role, Role::Stop | Role::Target) && own(o)).collect();
        new_brackets.push(bracket_of(b, &exits)?);
    }
    let mut new_orders = Vec::new();
    for o in &orders {
        if !own(&o) {
            carried.left_to_the_feed += 1;
            continue;
        }
        if book.order(&o.id).map_err(|e| e.to_string())?.is_some() {
            continue;
        }
        let bracket = match o.role {
            Role::Stop | Role::Target => bracket_of_entry.get(o.parent_id.as_str()).map(|b| (b.to_string(), if o.role == Role::Stop { OrderRole::Stop } else { OrderRole::Target })),
            _ => bracket_of_entry.get(o.id.as_str()).map(|b| (b.to_string(), OrderRole::Entry)),
        };
        new_orders.push(order_of(o, bracket)?);
    }
    book.import_orders(&new_brackets, &new_orders).map_err(|e| format!("the earlier orders could not be carried into the book: {e}; the old tables are kept"))?;
    carried.brackets = new_brackets.len();
    carried.orders = new_orders.len();
    bagholder_store::schema::drop_order_tables(conn, &at.to_string()).map_err(e)?;
    Ok(carried)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bagholder_store::orders::typed;

    fn at() -> jiff::Timestamp {
        "2026-09-28T14:00:00Z".parse().unwrap()
    }

    fn old_order(v: serde_json::Value) -> OldOrder {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn live_orders_and_brackets_are_carried_once_and_followed_and_the_old_tables_go() {
        let home = tempfile::tempdir().unwrap();
        let conn = Connection::open(home.path().join("bagholder.db")).unwrap();
        bagholder_store::schema::init_schema(&conn).unwrap();
        let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, at()).unwrap();
        let t = "2026-09-20T14:00:00Z";
        let entry = old_order(serde_json::json!({"id": "order-e", "createdAt": t, "accountId": "acct", "securityId": "sec", "symbol": "SHOP", "currency": "USD", "side": "BUY", "type": "LIMIT", "quantity": 10, "limitPrice": 100.5, "tif": "DAY", "status": "filled", "wsOrderId": "ws-1", "filledQty": 10, "avgFill": 100.25, "source": "bagholder", "role": "entry", "request": {"externalId": "order-e"}}));
        let stop = old_order(serde_json::json!({"id": "order-s", "createdAt": t, "accountId": "acct", "securityId": "sec", "symbol": "SHOP", "currency": "USD", "side": "SELL", "type": "STOP", "quantity": 10, "stopPrice": 95, "tif": "UNTIL_CANCEL", "status": "pending", "wsOrderId": "ws-2", "source": "bagholder", "role": "stop", "parentId": "order-e", "request": {"externalId": "order-s"}}));
        let theirs = old_order(serde_json::json!({"id": "ws-feed", "createdAt": t, "accountId": "acct", "securityId": "sec", "symbol": "SHOP", "currency": "USD", "side": "BUY", "type": "LIMIT", "quantity": 1, "limitPrice": 90, "status": "pending", "source": "wealthsimple"}));
        let sending = old_order(serde_json::json!({"id": "order-x", "createdAt": t, "accountId": "acct", "securityId": "sec", "symbol": "SHOP", "currency": "USD", "side": "BUY", "type": "MARKET", "quantity": 2, "status": "sending", "source": "bagholder", "role": "entry"}));
        for o in [&entry, &stop, &theirs, &sending] {
            typed::insert_order(&conn, o, t).unwrap();
        }
        let b: OldBracket = serde_json::from_value(serde_json::json!({"id": "bracket-1", "orderId": "order-e", "createdAt": t, "accountId": "acct", "securityId": "sec", "symbol": "SHOP", "currency": "USD", "quantity": 10, "slKind": "stop", "slPrice": 95, "slOrderId": "order-s", "slMode": "native", "tpPrice": 110, "status": "armed", "attempts": 0, "armedAt": t})).unwrap();
        typed::insert_bracket(&conn, &b, t).unwrap();

        let carried = carry_orders(home.path(), &conn, &book, at()).unwrap();
        assert_eq!((carried.orders, carried.brackets, carried.left_to_the_feed), (3, 1, 1));
        assert!(carried.snapshot.as_ref().is_some_and(|p| p.exists()), "the file as it was is kept");
        let sb = book.bracket("bracket-1").unwrap().unwrap();
        assert_eq!((sb.bracket.phase, sb.bracket.exit.clone(), sb.bracket.stop.map(|s| s.level)), (Phase::Guarding, Some((ExitRole::Stop, "order-s".to_string())), Some(Dec::parse("95").unwrap())));
        let s = book.order("order-s").unwrap().unwrap();
        assert_eq!((s.fold.state, s.request.bracket.clone()), (OrderState::Pending, Some(("bracket-1".to_string(), OrderRole::Stop))));
        assert_eq!(book.order("order-e").unwrap().unwrap().fold.average, Some(Dec::parse("100.25").unwrap()));
        assert_eq!(book.order("order-x").unwrap().unwrap().fold.state, OrderState::Unconfirmed, "a row left sending is read back before anything else");
        let in_flight: Vec<String> = book.orders_in_flight().unwrap().into_iter().map(|o| o.request.id).collect();
        assert_eq!(in_flight, ["order-s", "order-x"], "followed from the first check");
        assert!(book.states_disagreeing().unwrap().is_empty());
        // gone from the old file, and not made again at the next start
        assert!(bagholder_store::schema::orders_moved(&conn).unwrap());
        bagholder_store::schema::init_schema(&conn).unwrap();
        let tables: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('orders','brackets')", [], |r| r.get(0)).unwrap();
        assert_eq!(tables, 0);
        assert_eq!(carry_orders(home.path(), &conn, &book, at()).unwrap(), Carried::default(), "once");
    }

    #[test]
    fn a_row_the_import_cannot_read_stops_it_and_the_old_tables_stay() {
        let home = tempfile::tempdir().unwrap();
        let conn = Connection::open(home.path().join("bagholder.db")).unwrap();
        bagholder_store::schema::init_schema(&conn).unwrap();
        let (book, _) = Book::open_in(home.path(), crate::app::APP_VERSION, at()).unwrap();
        let bad = old_order(serde_json::json!({"id": "order-1", "createdAt": "2026-09-20T14:00:00Z", "accountId": "acct", "securityId": "sec", "symbol": "SHOP", "currency": "USD", "side": "BUY", "type": "LIMIT", "quantity": 1, "limitPrice": 1, "status": "", "source": "bagholder"}));
        typed::insert_order(&conn, &bad, "2026-09-20T14:00:00Z").unwrap();
        let e = carry_orders(home.path(), &conn, &book, at()).unwrap_err();
        assert!(e.contains("order-1: an order with no status"), "{e}");
        assert!(!bagholder_store::schema::orders_moved(&conn).unwrap());
        assert!(book.order("order-1").unwrap().is_none(), "nothing half carried");
    }
}
