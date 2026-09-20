//! Round trips: `build_trades` and `collapse_trade`.
//!
//! A trade is a position going from flat to open and back to flat. Partial
//! exits are legs of the same trade, and the id is stable from the first fill
//! so a journal entry survives later exits.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::activity::{Activity, Direction, Flag, Kind, Side};
use crate::book::Book;
use crate::clock::when_parts;
use crate::dates::days_between;
use crate::fifo::{trade_side, Slice};
use crate::input::{Journal, LastFill, TradeGroup};
use crate::symbols::{option_multiplier, underlying_symbol};
use crate::value::{fmt8, FSum};
use crate::wire::{ExitSide, Fill, Leg, Tally, Trade};

/// What a saved group names its members by.
pub fn slice_member_key(t: &Slice) -> String {
    if !t.buy_activity_id.is_empty() && !t.sell_activity_id.is_empty() {
        return [t.buy_activity_id.clone(), t.sell_activity_id.clone(), fmt8(t.quantity)].join("|");
    }
    t.id.clone()
}

/// The legacy page's group id: FNV-1a over the sorted member keys.
pub fn group_id_for_keys(keys: &[String]) -> String {
    let mut sorted: Vec<&String> = keys.iter().collect();
    sorted.sort();
    let joined = sorted.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
    let mut h: u32 = 2166136261;
    for ch in joined.chars() {
        h ^= ch as u32;
        h = h.wrapping_mul(16777619);
    }
    format!("g_{:x}_{}", h, keys.len())
}

fn leg(s: &Slice) -> Leg {
    Leg {
        key: slice_member_key(s),
        qty: s.quantity,
        entry: s.entry_price,
        exit: s.exit_price,
        entry_date: s.entry_date.clone(),
        exit_date: s.exit_date.clone(),
        pnl: s.pnl,
        pnl_cad: s.pnl_cad,
        fees: s.commission,
        buy_activity_id: s.buy_activity_id.clone(),
        sell_activity_id: s.sell_activity_id.clone(),
        flags: s.flags.clone(),
    }
}

/// One broker fill as the page prints it.
pub fn fill_row(a: &Activity) -> Fill {
    let (day, clock) = when_parts(a.when());
    let side = trade_side(a);
    let qty = a.quantity.abs();
    Fill {
        id: a.id.clone(),
        when: a.when().to_string(),
        date: if day.is_empty() { a.transaction_date.clone() } else { day },
        time: clock,
        side,
        sub: a.activity_sub_type.clone(),
        qty: if side == Some(Side::Sell) { -qty } else { qty },
        price: a.unit_price,
        amount: a.net_cash_amount,
        fees: a.commission,
        currency: a.currency.clone(),
        flags: a.flags.clone(),
    }
}

/// One group of slices as a single round trip.
pub fn collapse_trade(gid: &str, slices: &[Slice], locked: bool, book: &Book, journal: &Journal) -> Trade {
    let mut slices: Vec<&Slice> = slices.iter().collect();
    slices.sort_by_cached_key(|s| (s.exit_date.clone(), s.entry_date.clone(), slice_member_key(s)));
    let t0 = slices[0];

    let qty: f64 = slices.iter().map(|s| s.quantity).fsum();
    let entry_notional: f64 = slices.iter().map(|s| s.entry_price * s.quantity).fsum();
    let exit_notional: f64 = slices.iter().map(|s| s.exit_price * s.quantity).fsum();
    let pnl: f64 = slices.iter().map(|s| s.pnl).fsum();
    let pnl_cad: f64 = slices.iter().map(|s| s.pnl_cad).fsum();
    let fees: f64 = slices.iter().map(|s| s.commission).fsum();
    let fees_cad: f64 = slices.iter().map(|s| s.fees_cad.unwrap_or(s.commission)).fsum();
    let entry_date = slices.iter().map(|s| s.entry_date.clone()).min().unwrap_or_default();
    let exit_date = slices.iter().map(|s| s.exit_date.clone()).max().unwrap_or_default();
    let or_day = |when: &String, day: &String| if when.is_empty() { day.clone() } else { when.clone() };
    let entry_when = slices.iter().map(|s| or_day(&s.entry_when, &s.entry_date)).min().unwrap_or_default();
    let exit_when = slices.iter().map(|s| or_day(&s.exit_when, &s.exit_date)).max().unwrap_or_default();

    let mult = option_multiplier(&t0.symbol);
    let entry = if qty != 0.0 { entry_notional / qty } else { 0.0 };
    let exit = if qty != 0.0 { exit_notional / qty } else { t0.exit_price };
    let basis = (entry * qty * mult).abs();
    let security_id = slices.iter().map(|s| &s.security_id).find(|s| !s.is_empty()).cloned().unwrap_or_default();

    let mut ids: Vec<&String> = Vec::new();
    for s in &slices {
        for id in [&s.buy_activity_id, &s.sell_activity_id] {
            if !id.is_empty() && !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let mut fills: Vec<Fill> = ids.iter().filter_map(|id| book.activity(id)).map(fill_row).collect();

    // Label each fill by what it did in this trade, not by the broker's order
    // type: the open/close types are option language, while shares and crypto
    // are simply bought or sold.
    let opened_ids: HashSet<&String> = slices.iter().map(|s| &s.buy_activity_id).collect();
    let closed_ids: HashSet<&String> = slices.iter().map(|s| &s.sell_activity_id).collect();
    for f in fills.iter_mut() {
        let (opened, closed) = (opened_ids.contains(&f.id), closed_ids.contains(&f.id));
        let side = if f.side == Some(Side::Buy) { "BUY" } else { "SELL" };
        f.sub = match (t0.kind, opened, closed) {
            (Kind::Options, false, true) => format!("{} TO CLOSE", side),
            (Kind::Options, true, false) => format!("{} TO OPEN", side),
            (Kind::Options, false, false) => f.sub.clone(),
            (_, true, true) => format!("{} (close + open)", side),
            _ => side.to_string(),
        };
    }
    fills.sort_by(|a, b| b.when.cmp(&a.when));

    let opens = fills.iter().filter(|f| f.side == Some(t0.open_direction.opened_by())).count();
    let closes = fills.len() - opens;

    let mut flags: Vec<Flag> = slices.iter().flat_map(|s| s.flags.iter().cloned()).collect();
    flags.sort();
    flags.dedup();

    let said = journal.get(gid).cloned().unwrap_or_default();
    Trade {
        id: gid.to_string(),
        status: "closed",
        locked,
        symbol: t0.symbol.clone(),
        underlying: underlying_symbol(&t0.symbol),
        name: book.securities.name(&security_id, if t0.name.is_empty() { &t0.symbol } else { &t0.name }),
        exchange: if t0.kind == Kind::Crypto { "Crypto".to_string() } else { book.securities.exchange(&security_id) },
        kind: t0.kind,
        currency: t0.currency.clone(),
        account: t0.account.clone(),
        account_id: t0.account_id.clone(),
        security_id,
        side: if t0.open_direction == Direction::Long { ExitSide::Sell } else { ExitSide::Cover },
        open_direction: t0.open_direction,
        qty,
        mult: mult as i64,
        entry,
        exit,
        hold_days: days_between(&entry_date, &exit_date),
        entry_date,
        exit_date,
        entry_when,
        exit_when,
        pnl,
        pnl_cad,
        fees,
        fees_cad,
        pnl_pct: (basis > 0.0).then(|| pnl / basis),
        legs: Some(Arc::new(slices.iter().map(|s| leg(s)).collect())),
        leg_count: slices.len(),
        opened: Tally { qty, avg: entry, fills: opens },
        closed: Tally { qty, avg: exit, fills: closes },
        fills: Some(Arc::new(fills)),
        net_cash: pnl,
        flags,
        grade: said.grade,
        thesis: said.thesis,
        tags: said.tags,
    }
}

/// The saved manual groups first, then whatever is left grouped by round trip.
pub fn build_trades(closed: &[Slice], saved_groups: &[TradeGroup], book: &Book, journal: &Journal) -> Vec<Trade> {
    let by_key: HashMap<String, &Slice> = closed.iter().map(|s| (slice_member_key(s), s)).collect();
    let mut used: HashSet<String> = HashSet::new();
    let mut groups: Vec<(String, Vec<Slice>, bool)> = Vec::new();

    for saved in saved_groups {
        let mut members: Vec<Slice> = Vec::new();
        for key in &saved.members {
            if let Some(s) = by_key.get(key) {
                if used.insert(key.clone()) {
                    members.push((*s).clone());
                }
            }
        }
        if !members.is_empty() {
            let gid = if saved.id.is_empty() { group_id_for_keys(&members.iter().map(slice_member_key).collect::<Vec<_>>()) } else { saved.id.clone() };
            groups.push((gid, members, true));
        }
    }

    let mut by_rt: HashMap<String, Vec<Slice>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for s in closed {
        let key = slice_member_key(s);
        if used.contains(&key) {
            continue;
        }
        let mut rt = s.rt.clone().unwrap_or_else(|| format!("rt:{}", key));
        // a deposited (unknown-basis) leg stands as its own unscoreable trade
        if s.flags.contains(&Flag::BasisUnknown) {
            rt.push_str("|nobasis");
        }
        if !by_rt.contains_key(&rt) {
            order.push(rt.clone());
        }
        by_rt.entry(rt).or_default().push(s.clone());
    }
    for rt in order {
        let members = by_rt.remove(&rt).unwrap();
        groups.push((rt, members, false));
    }

    let mut trades: Vec<Trade> = groups.iter().map(|(gid, members, locked)| collapse_trade(gid, members, *locked, book, journal)).collect();
    trades.sort_by(|a, b| (&b.exit_date, &b.id).cmp(&(&a.exit_date, &a.id)));
    trades
}

/// Symbol -> the newest fill that carried a price.
pub fn last_fill_prices(activities: &[Activity]) -> HashMap<String, LastFill> {
    let mut ordered: Vec<&Activity> = activities.iter().filter(|a| a.category.is_fill() && a.unit_price > 0.0 && !a.symbol.is_empty()).collect();
    ordered.sort_by(|a, b| (&a.transaction_date, &a.occurred_at).cmp(&(&b.transaction_date, &b.occurred_at)));
    ordered.into_iter().map(|a| (a.symbol.clone(), LastFill { price: a.unit_price, date: a.transaction_date.clone() })).collect()
}
