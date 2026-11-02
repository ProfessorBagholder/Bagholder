//! What a written option was held against when it was written
//! (`docs/plans/trade-side-and-coverage.md`, SPEC.md §2 Trade): a short call is
//! covered by units of its underlying held long in the same account, a short put
//! secured by that account's cash in its currency. Each contract is judged once,
//! from Wealthsimple's positions read as of its opening day (`days_held`, brief
//! 79): the journal's lots are never an amount, and the ledger keeps no running
//! cash. The read is as of the day's close, so a contract written and covered
//! within one day is judged by that close.

use std::collections::BTreeMap;

use bagholder_core::instrument::{InstrumentKind, OptionRight};
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{AccountId, Currency, Dec, InstrumentId, Rounding, TransactionId};

use crate::input::Inputs;
use crate::ledger::{Direction, Matched};
use crate::trades::{TradeFig, TradeKey};

// Whether the read's cash line already leaves out the cash a secured put holds
// while it is open is not known (Wealthsimple's `TRADING` balance is net of its
// holds, brief 16; its positions read's cash line is not documented). So a put is
// secured where the day's cash covers it either way, uncovered where the cash is
// below nothing (borrowed), and not told in between: the read can't say.

/// A short option trade's coverage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// Every contract was judged: `covered` of `of` contracts were covered (a
    /// call) or secured (a put). A covered call's shares' trade, the one open in
    /// its account at that day's close (`against`): none where the shares came
    /// with no row the journal holds, or its contracts were written against
    /// different ones.
    Read { right: OptionRight, covered: Dec, of: Dec, against: Option<TradeKey> },
    /// Not every contract can be judged yet: the opening day's read is not in
    /// (`day`), or the contract's size is not stated (no day).
    Waiting { right: OptionRight, day: Option<Date> },
    /// A put the day's cash neither covers nor leaves short: the read can't say
    /// whether that cash is net of the put's own hold.
    Unclear { right: OptionRight },
}

impl Coverage {
    pub fn right(&self) -> OptionRight {
        match self {
            Coverage::Read { right, .. } | Coverage::Waiting { right, .. } | Coverage::Unclear { right } => *right,
        }
    }

    /// Every contract covered or secured.
    pub fn whole(&self) -> bool {
        matches!(self, Coverage::Read { covered, of, .. } if covered == of)
    }
}

/// One opening of a short contract: the contracts one fill wrote, in pieces as
/// they closed (or are still open).
struct Opening {
    trade: TradeKey,
    account: AccountId,
    instrument: InstrumentId,
    by: TransactionId,
    day: Date,
    at: Option<Timestamp>,
    qty: Dec,
    /// What the contracts brought in, in the option's currency, where known.
    premium: Option<Dec>,
    /// Each piece's contracts and the day it closed (none while open).
    pieces: Vec<(Dec, Option<Date>)>,
}

/// What an earlier opening still holds against its account: units of an
/// underlying, or cash in a currency, until a day.
struct Held {
    on: Holding,
    amount: Dec,
    until: Option<Date>,
}

#[derive(PartialEq, Eq)]
enum Holding {
    Units(InstrumentId),
    Cash(Currency),
}

/// Each short option trade's coverage, judged in the order the contracts were
/// written, account by account.
pub fn coverages<'a>(inputs: &Inputs, matched: &Matched, trades: impl IntoIterator<Item = &'a TradeFig>) -> BTreeMap<TradeKey, Coverage> {
    let trades: Vec<&TradeFig> = trades.into_iter().collect();
    let mut openings: Vec<Opening> = Vec::new();
    for t in trades.iter().copied().filter(|t| written(t)) {
        let mut by: BTreeMap<(TransactionId, InstrumentId), Opening> = BTreeMap::new();
        let mut add = |instrument: InstrumentId, id: &TransactionId, day: Date, at: Option<Timestamp>, qty: Dec, premium: Option<Dec>, closed: Option<Date>| {
            let o = by.entry((id.clone(), instrument)).or_insert_with(|| Opening { trade: t.key.clone(), account: t.account, instrument, by: id.clone(), day, at, qty: Dec::ZERO, premium: Some(Dec::ZERO), pieces: Vec::new() });
            o.qty = o.qty.checked_add(qty).unwrap_or(o.qty);
            o.premium = match (o.premium, premium) {
                (Some(a), Some(b)) => a.checked_add(b).ok(),
                _ => None,
            };
            o.pieces.push((qty, closed));
        };
        for s in &t.slices {
            add(s.instrument, &s.opened_by, s.opened_on, s.opened_at, s.qty, s.entry.as_ref().ok().map(|m| m.amount.abs()), Some(s.closed_on));
        }
        for k in &t.trips {
            if let Some(trip) = matched.trips.get(k) {
                for (i, l) in matched.lots_of(trip) {
                    add(i, &l.opened_by, l.day, l.at, l.qty, l.value.as_ref().ok().map(|m| m.amount.abs()), None);
                }
            }
        }
        openings.extend(by.into_values().filter(|o| o.qty.is_positive()));
    }
    openings.sort_by(|a, b| (a.account, a.day, a.at, &a.by).cmp(&(b.account, b.day, b.at, &b.by)));
    // per trade: its right, the contracts covered and written, the first opening
    // that could not be judged, and whether a put's cash could not tell
    let mut per: BTreeMap<TradeKey, (OptionRight, Dec, Dec, Option<Option<Date>>, bool)> = BTreeMap::new();
    // each covered call's shares' trade: none once two of its openings disagree
    let mut against: BTreeMap<TradeKey, Option<TradeKey>> = BTreeMap::new();
    let mut holding: BTreeMap<AccountId, Vec<Held>> = BTreeMap::new();
    for o in &openings {
        let Some(terms) = inputs.ledger.instruments.get(&o.instrument).and_then(|i| i.terms.as_ref()) else { continue };
        let entry = per.entry(o.trade.clone()).or_insert((terms.right, Dec::ZERO, Dec::ZERO, None, false));
        entry.2 = entry.2.checked_add(o.qty).unwrap_or(entry.2);
        let held = holding.entry(o.account).or_default();
        // what earlier contracts of the account still hold at this day's close
        held.retain(|h| h.until.is_none_or(|u| u > o.day));
        let Some(size) = terms.multiplier else {
            entry.3.get_or_insert(None);
            continue;
        };
        let Some(read) = inputs.market.brokers.get(&o.account).and_then(|b| b.days_held.get(&o.day)) else {
            entry.3.get_or_insert(Some(o.day));
            continue;
        };
        let currency = inputs.ledger.instruments.get(&o.instrument).map(|i| i.instrument.currency);
        let (on, per_contract, available) = match terms.right {
            OptionRight::Call => {
                let on = Holding::Units(terms.underlying);
                let long = read.held.get(&terms.underlying).copied().filter(|u| u.is_positive()).unwrap_or(Dec::ZERO);
                let taken = sum(held.iter().filter(|h| h.on == on).map(|h| h.amount));
                (on, size, long.checked_sub(taken).unwrap_or(Dec::ZERO))
            }
            OptionRight::Put => {
                let Some(currency) = currency else { continue };
                let on = Holding::Cash(currency);
                let Some(premium) = o.premium else {
                    entry.3.get_or_insert(Some(o.day));
                    continue;
                };
                // strike × size × contracts less the premium received (43198629134235)
                let whole = terms.strike.checked_mul(size).and_then(|v| v.checked_mul(o.qty)).and_then(|v| v.checked_sub(premium)).unwrap_or(Dec::ZERO);
                let per_contract = whole.div_rounded(o.qty, 8, Rounding::AwayFromZero).unwrap_or(whole);
                let cash = read.cash.get(&currency).copied().unwrap_or(Dec::ZERO);
                let available = cash.checked_sub(sum(held.iter().filter(|h| h.on == on).map(|h| h.amount))).unwrap_or(Dec::ZERO);
                // cash at or above nothing that doesn't cover the hold may be cash
                // the hold was already taken from: not told
                if available < whole && !cash.is_negative() {
                    entry.4 = true;
                    continue;
                }
                (on, per_contract, available)
            }
        };
        let covered = if per_contract.is_positive() && available.is_positive() {
            available.div_rounded(per_contract, 0, Rounding::TowardZero).unwrap_or(Dec::ZERO).min(o.qty)
        } else if per_contract.is_positive() {
            Dec::ZERO
        } else {
            o.qty
        };
        entry.1 = entry.1.checked_add(covered).unwrap_or(entry.1);
        if terms.right == OptionRight::Call && covered.is_positive() {
            // the shares' trade open in the account at that day's close: one round
            // trip per account and instrument is open at a time (trade-1)
            let shares = trades.iter().find(|s| s.account == o.account && s.direction == Direction::Long && s.instruments.contains(&terms.underlying) && s.opened_on <= o.day && s.closed_on.is_none_or(|c| c > o.day)).map(|s| s.key.clone());
            match against.get(&o.trade) {
                None => {
                    against.insert(o.trade.clone(), shares);
                }
                Some(was) if *was != shares => {
                    against.insert(o.trade.clone(), None);
                }
                Some(_) => {}
            }
        }
        // each piece holds its share of what the covered contracts hold, until it closed
        for (qty, until) in &o.pieces {
            let share = covered.mul_div_rounded(*qty, o.qty, 8, Rounding::HalfEven).unwrap_or(Dec::ZERO);
            if share.is_positive() {
                held.push(Held { on: match &on { Holding::Units(i) => Holding::Units(*i), Holding::Cash(c) => Holding::Cash(*c) }, amount: share.checked_mul(per_contract).unwrap_or(Dec::ZERO), until: *until });
            }
        }
    }
    per.into_iter()
        .map(|(k, (right, covered, of, waiting, unclear))| {
            let c = match (waiting, unclear) {
                (Some(day), _) => Coverage::Waiting { right, day },
                (None, true) => Coverage::Unclear { right },
                (None, false) => Coverage::Read { right, covered, of, against: against.remove(&k).flatten() },
            };
            (k, c)
        })
        .collect()
}

fn sum(items: impl Iterator<Item = Dec>) -> Dec {
    items.fold(Dec::ZERO, |a, b| a.checked_add(b).unwrap_or(a))
}

/// A short option: the trades a coverage is judged for.
pub fn written(t: &TradeFig) -> bool {
    t.direction == Direction::Short && t.kind == InstrumentKind::OptionContract
}

/// Each trade's coverage set from `coverages`.
pub fn assign(inputs: &Inputs, matched: &Matched, trades: &mut [TradeFig]) {
    let mut c = coverages(inputs, matched, trades.iter());
    for t in trades.iter_mut() {
        t.coverage = c.remove(&t.key);
    }
}

/// The opening days whose positions read a contract's coverage waits on.
pub fn days_waited(trades: &[TradeFig]) -> impl Iterator<Item = Date> + '_ {
    trades.iter().filter_map(|t| match &t.coverage {
        Some(Coverage::Waiting { day: Some(d), .. }) => Some(*d),
        _ => None,
    })
}

/// The shares' trade a covered call was written against, and the part of the
/// call's contracts covered (`covered` of `of`).
pub fn written_against(t: &TradeFig) -> Option<(&TradeKey, Dec, Dec)> {
    match &t.coverage {
        Some(Coverage::Read { right: OptionRight::Call, covered, of, against: Some(k) }) if covered.is_positive() && of.is_positive() => Some((k, *covered, *of)),
        _ => None,
    }
}

/// A trade's P&L as the statistics count it (SPEC.md trade-76): a covered call
/// counts inside its shares' position, never as a trade of its own; its
/// uncovered part, if any, stays its own.
#[derive(Clone, Debug, PartialEq)]
pub struct Counted<'a> {
    pub trade: &'a TradeFig,
    pub pnl: crate::gap::Fig<bagholder_core::Money>,
    pub pnl_cad: crate::gap::Fig<bagholder_core::Money>,
    /// The covered calls written against it, when it is a shares' trade.
    pub calls: Vec<&'a TradeFig>,
}

/// `m` × `part` ÷ `whole`, to the cent.
fn part_of(m: &crate::gap::Fig<bagholder_core::Money>, part: Dec, whole: Dec) -> crate::gap::Fig<bagholder_core::Money> {
    let m = m.clone()?;
    Ok(bagholder_core::Money::new(m.amount.mul_div_rounded(part, whole, 2, Rounding::HalfEven)?, m.currency))
}

fn add(a: crate::gap::Fig<bagholder_core::Money>, b: crate::gap::Fig<bagholder_core::Money>) -> crate::gap::Fig<bagholder_core::Money> {
    crate::gap::both(a, b, |x, y| Ok(x.checked_add(y)?))
}

/// Each trade of `counted_from` as the statistics count it, with `all` the whole
/// book's trades (a covered call outside the filters still counts in its shares'
/// position). A wholly covered call linked to its shares is left out: it counts
/// there. A call's P&L adds to its shares' in the shares' currency where they
/// share it, as a covered call's does (the same underlying); in CAD always.
pub fn counted<'a>(counted_from: &[&'a TradeFig], all: &'a [TradeFig]) -> Vec<Counted<'a>> {
    let mut calls: BTreeMap<&TradeKey, Vec<&'a TradeFig>> = BTreeMap::new();
    for t in all {
        if let Some((k, _, _)) = written_against(t) {
            calls.entry(k).or_default().push(t);
        }
    }
    let shares_known: std::collections::BTreeSet<&TradeKey> = all.iter().map(|t| &t.key).collect();
    let mut out = Vec::with_capacity(counted_from.len());
    for t in counted_from.iter().copied() {
        let (mut pnl, mut pnl_cad) = (t.pnl.clone(), t.pnl_cad.clone());
        if let Some((k, covered, of)) = written_against(t) {
            if shares_known.contains(k) {
                if covered == of {
                    continue;
                }
                // the uncovered contracts stay a trade of their own
                let left = of.checked_sub(covered).unwrap_or(Dec::ZERO);
                pnl = part_of(&t.pnl, left, of);
                pnl_cad = part_of(&t.pnl_cad, left, of);
            }
        }
        let mine: Vec<&'a TradeFig> = calls.get(&t.key).cloned().unwrap_or_default();
        for c in &mine {
            let (_, covered, of) = written_against(c).expect("listed by it");
            if c.currency == t.currency {
                pnl = add(pnl, part_of(&c.pnl, covered, of));
            }
            pnl_cad = add(pnl_cad, part_of(&c.pnl_cad, covered, of));
        }
        out.push(Counted { trade: t, pnl, pnl_cad, calls: mine });
    }
    out
}

/// Each shares' trade with covered calls written against it: the calls, and the
/// position's P&L, its own and theirs as `counted` adds them. While the shares
/// are held, their own is what they realized so far and what the holding's
/// units would make now (`holdings`, its unrealized P&L), so a call bought back
/// at a loss while they rose never reads as the position's loss. Only those trades.
pub fn positions<'a>(all: &'a [TradeFig], holdings: &[crate::positions::PositionFig]) -> BTreeMap<&'a TradeKey, Counted<'a>> {
    let mut calls: BTreeMap<&TradeKey, Vec<&TradeFig>> = BTreeMap::new();
    for t in all {
        if let Some((k, _, _)) = written_against(t) {
            calls.entry(k).or_default().push(t);
        }
    }
    let mut out = BTreeMap::new();
    for t in all {
        let Some(mine) = calls.remove(&t.key) else { continue };
        let (mut pnl, mut pnl_cad) = (t.pnl.clone(), t.pnl_cad.clone());
        if let TradeKey::Trip(k) = &t.key {
            if let Some(h) = holdings.iter().find(|h| h.key.as_ref() == Some(k)) {
                pnl = add(pnl, h.unrealized.clone());
                pnl_cad = add(pnl_cad, h.unrealized_cad.clone());
            }
        }
        for c in &mine {
            let (_, covered, of) = written_against(c).expect("listed by it");
            if c.currency == t.currency {
                pnl = add(pnl, part_of(&c.pnl, covered, of));
            }
            pnl_cad = add(pnl_cad, part_of(&c.pnl_cad, covered, of));
        }
        out.insert(&t.key, Counted { trade: t, pnl, pnl_cad, calls: mine });
    }
    out
}
