//! Trades (`SPEC.md` §2 Trade): each round trip from its first fill until it is
//! flat, open while any of it is held, or each group the person saved, with its
//! figures in the instrument's own currency and its P&L in CAD, each sale
//! realized on its own day and each leg converted on its own day.

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
use crate::ledger::{multiplier, Closer, Direction, Lot, Matched, Slice, TripKey};
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

/// Whether any of a trade is still held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TradeStatus {
    Open,
    Closed,
}

/// One sale's P&L, realized on its day.
#[derive(Clone, Debug, PartialEq)]
pub struct Realized {
    pub day: Date,
    pub pnl_cad: Fig<Money>,
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
    pub status: TradeStatus,
    /// Units opened.
    pub qty: Fig<Dec>,
    pub opened_on: Date,
    /// The last close, once the trade is closed.
    pub closed_on: Option<Date>,
    /// The day of its latest fill or close: what the list is ordered by.
    pub last_on: Date,
    pub opened_at: Option<Timestamp>,
    pub closed_at: Option<Timestamp>,
    /// Calendar days from the first opening to the last close, or to today
    /// while open.
    pub hold_days: i64,
    /// Per unit, quantity-weighted over the units opened: the entry value ÷
    /// (units × multiplier).
    pub entry: Fig<Dec>,
    /// The same over the units closed so far; none before the first.
    pub exit: Option<Fig<Dec>>,
    /// The entry value of the units closed: what the P&L percentage is over.
    pub basis: Fig<Money>,
    /// Each sale's P&L in CAD on its own day; `pnl_cad` is their sum.
    pub realized: Vec<Realized>,
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
    pub fn is_closed(&self) -> bool {
        self.status == TradeStatus::Closed
    }

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

/// The trade made of these slices and these lots still open. None for a round
/// trip that went flat without a sale (`SPEC.md` §2 Trade): it is not a trade.
#[allow(clippy::too_many_arguments)]
fn collapse(inputs: &Inputs, matched: &Matched, key: TradeKey, trade: Option<TradeId>, trips: Vec<TripKey>, slices: Vec<Slice>, lots: Vec<(InstrumentId, &Lot)>, journal: JournalEntry, locked: bool) -> Option<TradeFig> {
    let mut slices = slices;
    if slices.is_empty() && lots.is_empty() {
        return None;
    }
    slices.sort_by(|a, b| (a.closed_on, a.closed_at, a.opened_on).cmp(&(b.closed_on, b.closed_at, b.opened_on)));
    let first_trip = matched.trips.get(&trips[0])?;
    let named = slices.last().map(|s| s.instrument).or_else(|| lots.last().map(|(i, _)| *i))?;
    let instrument = *first_trip.instruments.last().unwrap_or(&named);
    let info = inputs.ledger.instruments.get(&instrument);
    let kind = info.map(|i| i.instrument.kind).unwrap_or(InstrumentKind::Security);
    let currency = info.map(|i| i.instrument.currency).unwrap_or(Currency::CAD);
    let direction = slices.first().map(|s| s.direction).or_else(|| lots.first().map(|(_, l)| l.direction))?;
    let status = if lots.is_empty() { TradeStatus::Closed } else { TradeStatus::Open };
    let mult = |i: InstrumentId| multiplier(inputs.ledger.instruments.get(&i), i);
    // units opened: the units closed and the units still held
    let qty: Fig<Dec> = slices.iter().map(|s| s.qty).chain(lots.iter().map(|(_, l)| l.qty)).try_fold(Dec::ZERO, |a, q| a.checked_add(q)).map_err(Gaps::from);
    let closed_entries = sum_money(currency, slices.iter().map(|s| s.entry.clone()));
    let all_entries = sum_money(currency, slices.iter().map(|s| s.entry.clone()).chain(lots.iter().map(|(_, l)| l.value.clone())));
    let exits = sum_money(currency, slices.iter().map(|s| s.exit.clone()));
    // units × multiplier, each on its own contract
    let weigh = |parts: Vec<(Dec, InstrumentId)>| -> Fig<Dec> {
        parts.into_iter().try_fold(Dec::ZERO, |a, (q, i)| Ok::<Dec, Gaps>(a.add_to_fit(q.checked_mul(mult(i)?)?)?))
    };
    let closed_weight = weigh(slices.iter().map(|s| (s.qty, s.instrument)).collect());
    let all_weight = weigh(slices.iter().map(|s| (s.qty, s.instrument)).chain(lots.iter().map(|(i, l)| (l.qty, *i))).collect());
    let avg = |total: &Fig<Money>, weight: &Fig<Dec>| -> Fig<Dec> {
        let (t, w) = (total.clone()?, weight.clone()?);
        if w.is_zero() {
            return Ok(Dec::ZERO);
        }
        Ok(t.amount.div_rounded(w, PRICE_PLACES, Rounding::HalfEven)?)
    };
    let fees = sum_money(currency, slices.iter().map(|s| Ok(s.entry_fee.add_to_fit(s.exit_fee)?)).chain(lots.iter().map(|(_, l)| Ok(l.fee))));
    let pnl = sum_money(currency, slices.iter().map(Slice::pnl));
    let realized: Vec<Realized> = slices.iter().map(|s| Realized { day: s.closed_on, pnl_cad: slice_pnl_cad(inputs, s) }).collect();
    let pnl_cad = sum_money(Currency::CAD, realized.iter().map(|r| r.pnl_cad.clone()));
    let fees_cad = sum_money(Currency::CAD, slices.iter().map(|s| fees_cad(inputs, s)).chain(lots.iter().map(|(_, l)| to_cad(&inputs.facts.rates, &inputs.clock, l.fee, l.day))));
    let opened_on = slices.iter().map(|s| s.opened_on).chain(lots.iter().map(|(_, l)| l.day)).min()?;
    let last_close = slices.iter().map(|s| s.closed_on).max();
    let closed_on = match status {
        TradeStatus::Closed => last_close,
        TradeStatus::Open => None,
    };
    let last_on = last_close.into_iter().chain(lots.iter().map(|(_, l)| l.day)).max()?;
    let opened_at = slices.iter().filter_map(|s| s.opened_at).chain(lots.iter().filter_map(|(_, l)| l.at)).min();
    let closed_at = match status {
        TradeStatus::Closed => slices.iter().filter_map(|s| s.closed_at).max(),
        TradeStatus::Open => None,
    };
    let hold_end = closed_on.unwrap_or(inputs.clock.today);
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
    for (_, l) in &lots {
        flags.extend(l.flags.iter().map(|f| f.word()));
    }
    let account = matched.trips.get(trips.last()?).map(|t| t.account).or_else(|| slices.last().map(|s| s.account))?;
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
        status,
        qty,
        opened_on,
        closed_on,
        last_on,
        opened_at,
        closed_at,
        hold_days: (hold_end - opened_on).get_days() as i64,
        entry: avg(&all_entries, &all_weight),
        // none before a unit has closed (a return of capital realized closes none)
        exit: if slices.iter().all(|s| s.qty.is_zero()) { None } else { Some(avg(&exits, &closed_weight)) },
        basis: closed_entries,
        realized,
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

/// Every trade: the saved groups first, then each remaining round trip, open or
/// closed, newest activity first.
pub fn build_trades(inputs: &Inputs, matched: &Matched, identity: &Identity) -> Vec<TradeFig> {
    let journal = &inputs.ledger.journal;
    let mut open: BTreeMap<&TripKey, Vec<(InstrumentId, &Lot)>> = BTreeMap::new();
    for ((_, instrument), book) in &matched.books {
        for l in &book.lots {
            open.entry(&l.trip).or_default().push((*instrument, l));
        }
    }
    let lots_of = |trips: &[TripKey]| -> Vec<(InstrumentId, &Lot)> { trips.iter().flat_map(|k| open.get(k).cloned().unwrap_or_default()).collect() };
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
        if let Some(t) = collapse(inputs, matched, TradeKey::Group(g.id), None, trips.clone(), slices, lots_of(&trips), entry, true) {
            used.extend(trips);
            out.push(t);
        }
    }
    for (key, trip) in &matched.trips {
        // a trip held in a managed account is the broker's, not a trade
        if used.contains(key) || crate::identity::managed(&inputs.ledger, trip.account) {
            continue;
        }
        let trade = identity.trade_of.get(key).copied();
        let entry = trade.and_then(|t| journal.get(&JournalSubject::Trade(t)).cloned()).unwrap_or_default();
        let keys = vec![key.clone()];
        if let Some(t) = collapse(inputs, matched, TradeKey::Trip(key.clone()), trade, keys.clone(), trip.slices.clone(), lots_of(&keys), entry, false) {
            out.push(t);
        }
    }
    out.sort_by(|a, b| (b.last_on, b.closed_at, &b.key).cmp(&(a.last_on, a.closed_at, &a.key)));
    out
}
