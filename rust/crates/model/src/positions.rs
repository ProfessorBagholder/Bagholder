//! Open positions: `build_positions`.
//!
//! The open lots rolled up per symbol, account, currency and direction. A
//! position and the trade it becomes when it closes share one journal entry,
//! because both are keyed by the round trip that opened it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::activity::{Direction, Kind};
use crate::book::Book;
use crate::dates::days_between;
use crate::fifo::Lot;
use crate::input::{AccountRow, BalanceRow, Journal, Quotes};
use crate::symbols::{option_multiplier, underlying_symbol};
use crate::value::FSum;
use crate::wire::{Fill, Mark, OpenLot, Position};

pub fn build_positions(book: &Book, balances: &[BalanceRow], accounts: &[AccountRow], journal: &Journal, today: &str, quotes: &Quotes) -> Vec<Position> {
    let mut ids_by_name: HashMap<String, HashSet<&str>> = HashMap::new();
    for account in accounts {
        ids_by_name.entry(account.name()).or_default().insert(&account.id);
    }
    let mut held_at_ws: HashMap<(&str, &str), f64> = HashMap::new();
    for b in balances {
        *held_at_ws.entry((&b.account_id, &b.security_id)).or_insert(0.0) += b.quantity;
    }

    type Key<'a> = (&'a str, &'a str, &'a str, Direction);
    let mut groups: HashMap<Key, Vec<&Lot>> = HashMap::new();
    let mut order: Vec<Key> = Vec::new();
    for lot in &book.fifo.open {
        let key = (lot.symbol.as_str(), lot.account_type.as_str(), lot.currency.as_str(), lot.direction);
        if !groups.contains_key(&key) {
            order.push(key);
        }
        groups.entry(key).or_default().push(lot);
    }

    let mut rows: Vec<Position> = Vec::new();
    for key in order {
        let mut lots = groups.remove(&key).unwrap();
        lots.sort_by(|a, b| (&a.date, &a.when).cmp(&(&b.date, &b.when)));
        let (symbol, account, currency, direction) = key;
        let first = lots[0];
        let mult = option_multiplier(symbol);
        let qty: f64 = lots.iter().map(|l| l.qty).fsum();
        if qty <= 1e-9 {
            continue;
        }
        let cost: f64 = lots.iter().map(|l| l.qty * l.price * mult).fsum();
        let fees: f64 = lots.iter().map(|l| l.commission).fsum();
        let security_id = lots.iter().map(|l| &l.security_id).find(|s| !s.is_empty()).cloned().unwrap_or_default();

        // marked at its own last fill until a quote that fits its kind says otherwise
        let mut last = match book.last_prices.get(symbol) {
            Some(fill) => fill.price,
            None => if qty != 0.0 { cost / (qty * mult) } else { 0.0 },
        };
        let mut price_source = Mark::Fill;
        let quote = quotes.get(symbol).filter(|q| q.fits(first.kind));
        if let Some(price) = quote.and_then(|q| q.price).filter(|p| *p != 0.0) {
            last = price;
            price_source = Mark::Quote;
        }

        let mv = qty * last * mult;
        let unreal = if direction == Direction::Long { mv - cost } else { cost - mv };
        let held: f64 = lots.iter().map(|l| l.qty * days_between(&l.date, today) as f64).fsum();

        let ws_qty = (!security_id.is_empty())
            .then(|| ids_by_name.get(account))
            .flatten()
            .map(|ids| ids.iter().filter_map(|id| held_at_ws.get(&(*id, security_id.as_str()))).collect::<Vec<_>>())
            .filter(|found| !found.is_empty())
            .map(|found| found.into_iter().sum());

        let legacy_id = format!("pos:{}|{}|{}", account, symbol, currency);
        let id = first.rt.clone().unwrap_or_else(|| legacy_id.clone());
        let said = journal.get(&id).or_else(|| journal.get(&legacy_id)).cloned().unwrap_or_default();

        let price_change = quote.and_then(|q| q.price_change);
        let mut fills: Vec<Fill> = lots.iter().filter_map(|l| book.activity(&l.activity_id)).map(crate::trades::fill_row).collect();
        fills.sort_by(|a, b| b.when.cmp(&a.when));

        rows.push(Position {
            id,
            symbol: symbol.to_string(),
            underlying: underlying_symbol(symbol),
            name: book.securities.name(&security_id, if first.name.is_empty() { symbol } else { &first.name }),
            exchange: if first.kind == Kind::Crypto { "Crypto".to_string() } else { book.securities.exchange(&security_id) },
            kind: first.kind,
            account: account.to_string(),
            account_id: first.account_id.clone(),
            currency: currency.to_string(),
            security_id,
            short: direction == Direction::Short,
            qty,
            mult: mult as i64,
            avg: if qty != 0.0 { cost / (qty * mult) } else { 0.0 },
            cost,
            fees,
            last,
            price_source,
            price_change,
            percent_change: quote.and_then(|q| q.percent_change),
            day_change: price_change.map(|pc| qty * pc * mult * if direction == Direction::Short { -1.0 } else { 1.0 }),
            mv,
            unreal,
            unreal_pct: (cost != 0.0).then(|| unreal / cost),
            held: if qty != 0.0 { round_half_even(held / qty) } else { 0 },
            opened: first.date.clone(),
            ws_qty,
            rt: first.rt.clone(),
            lots: lots
                .iter()
                .map(|l| OpenLot {
                    opened: l.date.clone(),
                    qty: l.qty,
                    price: l.price,
                    basis: l.qty * l.price * mult,
                    held: days_between(&l.date, today),
                    flags: l.flags.clone(),
                    activity_id: l.activity_id.clone(),
                })
                .collect(),
            fills: Some(Arc::new(fills)),
            grade: said.grade,
            thesis: said.thesis,
            tags: said.tags,
            alloc: 0.0,
        });
    }

    let book_cost: f64 = rows.iter().map(|r| r.cost.abs()).fsum();
    for r in rows.iter_mut() {
        r.alloc = if book_cost != 0.0 { r.cost.abs() / book_cost } else { 0.0 };
    }
    // stable, largest share first
    rows.sort_by(|a, b| b.alloc.partial_cmp(&a.alloc).unwrap_or(std::cmp::Ordering::Equal));
    rows
}

/// Round half to even, returning an integer.
fn round_half_even(v: f64) -> i64 {
    let r = v.round();
    if (v - v.trunc()).abs() == 0.5 && r % 2.0 != 0.0 {
        (r - v.signum()) as i64
    } else {
        r as i64
    }
}
