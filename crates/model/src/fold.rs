//! `model.fold_option_rolls`: a same-day cover plus a new short on the same
//! underlying is one roll, so the cover's P&L folds into the far contract's
//! basis and the cover row goes away.
//!
//! A chain rolls more than once, and the covers are walked in date order over
//! the *live* rows: a cover that was itself the target of an earlier fold has
//! already had its basis adjusted, and the next fold must read that adjusted
//! value. The Python list holds references for the same reason, so the covers
//! here are indices into `closed` rather than copies of it.

use std::collections::HashSet;

use crate::fifo::{stable_trade_id, Lot, Slice};
use crate::symbols::{is_option_symbol, option_multiplier, underlying_symbol};

fn day_of(s: &str) -> String {
    s.chars().take(10).collect()
}

fn roll_book_slice(t: &Slice) -> String {
    let acct = if t.account.is_empty() { &t.account_type } else { &t.account };
    format!("{}::{}::{}", acct, t.currency, underlying_symbol(&t.symbol))
}

pub fn fold_option_rolls(closed: &mut Vec<Slice>, open_lots: &mut [Lot]) {
    if closed.is_empty() {
        return;
    }

    // Ordered once, before anything is mutated, as the Python list is.
    let mut covers: Vec<usize> = (0..closed.len())
        .filter(|i| closed[*i].open_direction == "SHORT" && is_option_symbol(&closed[*i].symbol))
        .collect();
    covers.sort_by(|i, j| {
        let (a, b) = (&closed[*i], &closed[*j]);
        (day_of(&a.entry_date), day_of(&a.exit_date), a.id.clone())
            .cmp(&(day_of(&b.entry_date), day_of(&b.exit_date), b.id.clone()))
    });

    let mut dropped: HashSet<usize> = HashSet::new();
    for ci in covers {
        if dropped.contains(&ci) {
            continue;
        }
        let cover = closed[ci].clone();
        let d = day_of(&cover.exit_date);
        if d.is_empty() {
            continue;
        }
        let under = underlying_symbol(&cover.symbol);
        if under.is_empty() || under == "—" {
            continue;
        }
        let ck = roll_book_slice(&cover);

        let mut closed_cands: Vec<usize> = (0..closed.len())
            .filter(|i| {
                let t = &closed[*i];
                *i != ci
                    && !dropped.contains(i)
                    && t.open_direction == "SHORT"
                    && is_option_symbol(&t.symbol)
                    && t.symbol != cover.symbol
                    && roll_book_slice(t) == ck
                    && day_of(&t.entry_date) == d
            })
            .collect();
        let mut open_cands: Vec<usize> = (0..open_lots.len())
            .filter(|i| {
                let l = &open_lots[*i];
                l.direction == "SHORT"
                    && is_option_symbol(&l.symbol)
                    && l.symbol != cover.symbol
                    && format!("{}::{}::{}", l.account_type, l.currency, underlying_symbol(&l.symbol)) == ck
                    && day_of(&l.date) == d
            })
            .collect();

        let use_closed = !closed_cands.is_empty();
        if !use_closed && open_cands.is_empty() {
            continue;
        }
        let cq = cover.quantity.abs();
        // nearest quantity to the cover's wins, then the symbol, as Python sorts
        if use_closed {
            closed_cands.sort_by(|i, j| {
                let (a, b) = (&closed[*i], &closed[*j]);
                ((a.quantity.abs() - cq).abs(), a.symbol.clone())
                    .partial_cmp(&((b.quantity.abs() - cq).abs(), b.symbol.clone()))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let idx = closed_cands[0];
            let qty = closed[idx].quantity.abs();
            if !(qty > 0.0) {
                continue;
            }
            let mult = option_multiplier(&closed[idx].symbol);
            let adj = cover.pnl / (qty * mult);
            let row = &mut closed[idx];
            row.entry_price += adj;
            let raw = if row.open_direction == "SHORT" {
                row.entry_price - row.exit_price
            } else {
                row.exit_price - row.entry_price
            } * qty
                * mult;
            row.pnl = raw - row.commission;
            row.pnl_cad = row.pnl;
            row.id = stable_trade_id(row);
            if !row.flags.iter().any(|f| f == "rolled") {
                row.flags.push("rolled".into());
            }
        } else {
            open_cands.sort_by(|i, j| {
                let (a, b) = (&open_lots[*i], &open_lots[*j]);
                ((a.qty.abs() - cq).abs(), a.symbol.clone())
                    .partial_cmp(&((b.qty.abs() - cq).abs(), b.symbol.clone()))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let idx = open_cands[0];
            let qty = open_lots[idx].qty.abs();
            if !(qty > 0.0) {
                continue;
            }
            let adj = cover.pnl / (qty * option_multiplier(&open_lots[idx].symbol));
            let row = &mut open_lots[idx];
            row.price += adj;
            if !row.flags.iter().any(|f| f == "rolled") {
                row.flags.push("rolled".into());
            }
        }
        dropped.insert(ci);
    }
    if !dropped.is_empty() {
        let mut i = 0;
        closed.retain(|_| {
            let keep = !dropped.contains(&i);
            i += 1;
            keep
        });
    }
}
