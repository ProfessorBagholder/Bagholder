//! Trades (`SPEC.md` §2 Trade): the closed part of each round trip, or of each
//! group the person saved, with its figures in the instrument's own currency and
//! its P&L in CAD, each leg converted on its own day.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::journal::{JournalEntry, JournalSubject};
use bagholder_core::{AccountId, Currency, Dec, GroupId, InstrumentId, Money, Rounding, TradeId, TransactionId};

use crate::fx::to_cad;
use crate::gap::{Fig, Gaps};
use crate::identity::Identity;
use crate::input::Inputs;
use crate::ledger::{multiplier, Closer, Direction, Matched, Slice, TripKey};
use crate::stat::Ratio;

/// Places an average price is kept to; it is rounded again where it is shown.
pub const PRICE_PLACES: u32 = 12;

/// What a trade is known by: a round trip's trade, or a saved group.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TradeKey {
    /// A round trip, by its key; its trade id once the book has given one.
    Trip(TripKey),
    Group(GroupId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeFig {
    pub key: TradeKey,
    /// The trade id (for a round trip) once the book has one.
    pub trade: Option<TradeId>,
    /// The round trips it is made of: one, or a group's.
    pub trips: Vec<TripKey>,
    pub account: AccountId,
    /// The instrument it is named after: the last contract of a chain.
    pub instrument: InstrumentId,
    pub instruments: Vec<InstrumentId>,
    pub kind: InstrumentKind,
    pub currency: Currency,
    pub direction: Direction,
    /// Units matched.
    pub qty: Fig<Dec>,
    pub opened_on: Date,
    pub closed_on: Date,
    pub opened_at: Option<Timestamp>,
    pub closed_at: Option<Timestamp>,
    /// Calendar days from the first opening to the last close.
    pub hold_days: i64,
    /// Per unit, quantity-weighted: the entry value ÷ (units × multiplier).
    pub entry: Fig<Dec>,
    pub exit: Fig<Dec>,
    /// The entry value: what the P&L percentage is over.
    pub basis: Fig<Money>,
    pub pnl: Fig<Money>,
    pub pnl_cad: Fig<Money>,
    pub fees: Fig<Money>,
    pub fees_cad: Fig<Money>,
    pub slices: Vec<Slice>,
    /// Every transaction that opened or closed part of it.
    pub fills: BTreeSet<TransactionId>,
    pub opened: BTreeSet<TransactionId>,
    pub closed: BTreeSet<TransactionId>,
    /// The marks of its slices (`reward`, `deposited`, `rolled`, …).
    pub flags: BTreeSet<&'static str>,
    pub journal: JournalEntry,
    /// A saved group: shown as one trade and never regrouped.
    pub locked: bool,
}

impl TradeFig {
    /// The P&L's percentage of the entry basis.
    pub fn pnl_pct(&self) -> Option<Ratio> {
        match (&self.pnl, &self.basis) {
            (Ok(p), Ok(b)) => crate::stat::money_ratio(*p, *b),
            _ => None,
        }
    }

    /// What the figures of this trade wait on.
    pub fn gaps(&self) -> Gaps {
        let mut g = Gaps::none();
        for f in [&self.pnl, &self.pnl_cad, &self.fees] {
            if let Err(e) = f {
                g.merge(e);
            }
        }
        if let Err(e) = &self.qty {
            g.merge(e);
        }
        g
    }
}

fn sum_money(currency: Currency, items: impl IntoIterator<Item = Fig<Money>>) -> Fig<Money> {
    let mut total = Money::zero(currency);
    let mut gaps = Gaps::none();
    for m in items {
        match m {
            Ok(v) if gaps.is_empty() => total = total.add_to_fit(v)?,
            Ok(_) => {}
            Err(g) => gaps.merge(&g),
        }
    }
    gaps.or(total)
}

/// One slice's P&L in CAD: each value and fee converted on its own day.
fn slice_pnl_cad(inputs: &Inputs, s: &Slice) -> Fig<Money> {
    let rates = &inputs.facts.rates;
    let clock = &inputs.clock;
    let mut gaps = s.taint.clone();
    let parts = [
        s.entry.clone().and_then(|m| to_cad(rates, clock, m, s.opened_on)),
        s.exit.clone().and_then(|m| to_cad(rates, clock, m, s.closed_on)),
        to_cad(rates, clock, s.entry_fee, s.opened_on),
        to_cad(rates, clock, s.exit_fee, s.closed_on),
    ];
    for p in &parts {
        if let Err(g) = p {
            gaps.merge(g);
        }
    }
    if !gaps.is_empty() {
        return Err(gaps);
    }
    let [Ok(entry), Ok(exit), Ok(ef), Ok(xf)] = parts else { unreachable!("every part is stated") };
    let gross = match s.direction {
        Direction::Long => exit.checked_sub(entry)?,
        Direction::Short => entry.checked_sub(exit)?,
    };
    Ok(gross.checked_sub(ef)?.checked_sub(xf)?)
}

fn fees_cad(inputs: &Inputs, s: &Slice) -> Fig<Money> {
    let rates = &inputs.facts.rates;
    let clock = &inputs.clock;
    let a = to_cad(rates, clock, s.entry_fee, s.opened_on)?;
    let b = to_cad(rates, clock, s.exit_fee, s.closed_on)?;
    Ok(a.add_to_fit(b)?)
}

/// The trade made of these slices.
#[allow(clippy::too_many_arguments)]
fn collapse(inputs: &Inputs, matched: &Matched, key: TradeKey, trade: Option<TradeId>, trips: Vec<TripKey>, slices: Vec<Slice>, journal: JournalEntry, locked: bool) -> Option<TradeFig> {
    let mut slices = slices;
    if slices.is_empty() {
        return None;
    }
    slices.sort_by(|a, b| (a.closed_on, a.closed_at, a.opened_on).cmp(&(b.closed_on, b.closed_at, b.opened_on)));
    let first_trip = matched.trips.get(&trips[0])?;
    let last = slices.last()?;
    let instrument = *first_trip.instruments.last().unwrap_or(&last.instrument);
    let info = inputs.ledger.instruments.get(&instrument);
    let kind = info.map(|i| i.instrument.kind).unwrap_or(InstrumentKind::Security);
    let currency = info.map(|i| i.instrument.currency).unwrap_or(Currency::CAD);
    let direction = slices[0].direction;
    let qty: Fig<Dec> = slices.iter().try_fold(Dec::ZERO, |a, s| a.add_to_fit(s.qty)).map_err(Gaps::from);
    let entries = sum_money(currency, slices.iter().map(|s| s.entry.clone()));
    let exits = sum_money(currency, slices.iter().map(|s| s.exit.clone()));
    // units × multiplier over the slices, each on its own contract
    let weight: Fig<Dec> = slices.iter().try_fold(Dec::ZERO, |a, s| {
        let m = multiplier(inputs.ledger.instruments.get(&s.instrument), s.instrument)?;
        Ok::<Dec, Gaps>(a.add_to_fit(s.qty.checked_mul(m)?)?)
    });
    let avg = |total: &Fig<Money>| -> Fig<Dec> {
        let (t, w) = (total.clone()?, weight.clone()?);
        if w.is_zero() {
            return Ok(Dec::ZERO);
        }
        Ok(t.amount.div_rounded(w, PRICE_PLACES, Rounding::HalfEven)?)
    };
    let fees = sum_money(currency, slices.iter().map(|s| Ok(s.entry_fee.add_to_fit(s.exit_fee)?)));
    let pnl = sum_money(currency, slices.iter().map(Slice::pnl));
    let pnl_cad = sum_money(Currency::CAD, slices.iter().map(|s| slice_pnl_cad(inputs, s)));
    let fees_cad = sum_money(Currency::CAD, slices.iter().map(|s| fees_cad(inputs, s)));
    let opened_on = slices.iter().map(|s| s.opened_on).min()?;
    let closed_on = slices.iter().map(|s| s.closed_on).max()?;
    let opened_at = slices.iter().filter_map(|s| s.opened_at).min();
    let closed_at = slices.iter().filter_map(|s| s.closed_at).max();
    let mut fills = BTreeSet::new();
    let mut opened = BTreeSet::new();
    let mut closed = BTreeSet::new();
    let mut flags = BTreeSet::new();
    let mut instruments = Vec::new();
    for k in &trips {
        if let Some(t) = matched.trips.get(k) {
            fills.extend(t.fills.iter().cloned());
            opened.extend(t.opened.iter().cloned());
            closed.extend(t.closed.iter().cloned());
            for i in &t.instruments {
                if !instruments.contains(i) {
                    instruments.push(*i);
                }
            }
        }
    }
    for s in &slices {
        flags.extend(s.flags.iter().map(|f| f.word()));
        if let Closer::Transaction(id) = &s.closed_by {
            closed.insert(id.clone());
        }
    }
    let account = matched.trips.get(trips.last()?).map(|t| t.account).unwrap_or(last.account);
    Some(TradeFig {
        key,
        trade,
        trips,
        account,
        instrument,
        instruments,
        kind,
        currency,
        direction,
        qty,
        opened_on,
        closed_on,
        opened_at,
        closed_at,
        hold_days: (closed_on - opened_on).get_days() as i64,
        entry: avg(&entries),
        exit: avg(&exits),
        basis: entries,
        pnl,
        pnl_cad,
        fees,
        fees_cad,
        slices,
        fills,
        opened,
        closed,
        flags,
        journal,
        locked,
    })
}

/// Every trade: the saved groups first, then each remaining round trip with
/// something closed, newest close first.
pub fn build_trades(inputs: &Inputs, matched: &Matched, identity: &Identity) -> Vec<TradeFig> {
    let journal = &inputs.ledger.journal;
    let trip_of_trade: BTreeMap<TradeId, &TripKey> = identity.trade_of.iter().map(|(k, t)| (*t, k)).collect();
    let mut used: BTreeSet<TripKey> = BTreeSet::new();
    let mut out = Vec::new();
    for g in &inputs.ledger.groups {
        let mut trips: Vec<TripKey> = Vec::new();
        for m in &g.members {
            if let Some(k) = trip_of_trade.get(m) {
                if !trips.contains(k) && !used.contains(*k) {
                    trips.push((*k).clone());
                }
            }
        }
        if trips.is_empty() {
            continue;
        }
        let slices: Vec<Slice> = trips.iter().filter_map(|k| matched.trips.get(k)).flat_map(|t| t.slices.iter().cloned()).collect();
        let entry = journal.get(&JournalSubject::Group(g.id)).cloned().unwrap_or_default();
        if let Some(t) = collapse(inputs, matched, TradeKey::Group(g.id), None, trips.clone(), slices, entry, true) {
            used.extend(trips);
            out.push(t);
        }
    }
    for (key, trip) in &matched.trips {
        if used.contains(key) || trip.slices.is_empty() {
            continue;
        }
        let trade = identity.trade_of.get(key).copied();
        let entry = trade.and_then(|t| journal.get(&JournalSubject::Trade(t)).cloned()).unwrap_or_default();
        if let Some(t) = collapse(inputs, matched, TradeKey::Trip(key.clone()), trade, vec![key.clone()], trip.slices.clone(), entry, false) {
            out.push(t);
        }
    }
    out.sort_by(|a, b| (b.closed_on, b.closed_at, &b.key).cmp(&(a.closed_on, a.closed_at, &a.key)));
    out
}
