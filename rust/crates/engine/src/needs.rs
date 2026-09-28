//! The facts the figures use (`docs/plans/stage-3a-sources.md`, "The command";
//! `docs/plans/stage-3a-brief-06.md`): which currencies the engine converts and
//! from which day, which closes decide a contract's expiry, which instruments are
//! held today (their prices are quoted), and which payers are held. The readers
//! read what this names and nothing else, so it is worked out here, beside the
//! figures that use it, and not guessed by the caller.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::Date;
use bagholder_core::{Currency, InstrumentId};

use crate::input::Inputs;
use crate::ledger::Matched;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FactNeeds {
    /// Each currency converted to CAD, and the oldest day it is converted on.
    pub rates: BTreeMap<Currency, Date>,
    /// Each instrument whose close a figure uses, and the days it does: an
    /// underlying on the expiry day of each contract held into it, whose close
    /// says whether the contract expired worthless.
    pub closes: BTreeMap<InstrumentId, BTreeSet<Date>>,
    /// Each instrument held today in some account: its price is quoted.
    pub held: BTreeSet<InstrumentId>,
    /// Each security held now: its distributions and schedule are read.
    pub payers: BTreeSet<InstrumentId>,
    /// The person's oldest day: the span the benchmarks must cover.
    pub first_day: Option<Date>,
}

impl FactNeeds {
    fn rate(&mut self, c: Currency, day: Date) {
        if c == Currency::CAD {
            return;
        }
        let e = self.rates.entry(c).or_insert(day);
        *e = (*e).min(day);
    }

    fn close(&mut self, i: InstrumentId, day: Date) {
        self.closes.entry(i).or_default().insert(day);
    }
}

/// The days a holding's units are not zero, as spans of first and last day; the
/// last span runs to today while it is held.
fn held_spans(units: &BTreeMap<Date, crate::gap::Fig<bagholder_core::Dec>>, today: Date) -> Vec<(Date, Date)> {
    let mut out = Vec::new();
    let mut open: Option<Date> = None;
    for (day, q) in units {
        // a count that could not be summed is of units held
        match (open, q.as_ref().is_ok_and(|q| q.is_zero())) {
            (None, false) => open = Some(*day),
            // units recorded at the end of a day: the day it went to zero was held
            (Some(from), true) => {
                out.push((from, *day));
                open = None;
            }
            _ => {}
        }
    }
    if let Some(from) = open {
        out.push((from, today.max(from)));
    }
    out
}

pub fn fact_needs(inputs: &Inputs, matched: &Matched) -> FactNeeds {
    let today = inputs.clock.today;
    let ledger = &inputs.ledger;
    let mut n = FactNeeds::default();
    let kind = |i: &InstrumentId| ledger.instruments.get(i).map(|x| x.instrument.kind);
    let currency = |i: &InstrumentId| ledger.instruments.get(i).map(|x| x.instrument.currency);
    // every amount on the record, and every account's cash from its day on
    for t in &ledger.transactions {
        for m in [t.cash, t.price, t.fee].into_iter().flatten() {
            n.rate(m.currency, t.trade_date);
        }
        n.first_day = Some(n.first_day.map_or(t.trade_date, |d| d.min(t.trade_date)));
    }
    for b in inputs.market.brokers.values() {
        if let Some(d) = b.net_value.keys().next() {
            n.first_day = Some(n.first_day.map_or(*d, |f| f.min(*d)));
        }
    }
    // each holding: its currency converted from its first day held (its lots'
    // cost and value in CAD), quoted while held today, a security's payer read,
    // and a contract held into its expiry decided by its underlying's close that day
    for ((account, i), units) in &matched.units {
        let held_today = !matched.units_on(*account, *i, today).is_ok_and(|q| q.is_zero());
        for (from, to) in held_spans(units, today) {
            if let Some(c) = currency(i) {
                n.rate(c, from);
            }
            if held_today && to >= today {
                n.held.insert(*i);
                if kind(i) == Some(InstrumentKind::Security) {
                    n.payers.insert(*i);
                }
                // a holding priced by its last stored close in another currency
                // (a coin's USD market's), converted on that close's day
                if let Some((day, m)) = inputs.market.closes.get(i).and_then(|c| c.range(..=today).next_back()) {
                    if Some(m.currency) != currency(i) {
                        n.rate(m.currency, *day);
                    }
                }
            }
            if let Some(terms) = ledger.instruments.get(i).and_then(|x| x.terms.as_ref()) {
                if terms.expiry >= from && terms.expiry <= to.min(today) {
                    n.close(terms.underlying, terms.expiry);
                }
            }
        }
    }
    // a close stored in another currency than its instrument's is converted on its day
    for (i, closes) in &inputs.market.closes {
        let Some(days) = n.closes.get(i).cloned() else { continue };
        for (day, m) in closes.iter().filter(|(d, _)| days.contains(d)) {
            if Some(m.currency) != currency(i) {
                n.rate(m.currency, *day);
            }
        }
    }
    // a benchmark's tracker in another currency, converted from the person's
    // oldest day or its own first session, whichever is later
    if let Some(first) = n.first_day {
        for s in inputs.market.benchmarks.values() {
            if let Some(d) = s.closes.keys().find(|d| **d >= first) {
                n.rate(s.currency, *d);
            }
        }
    }
    // a payer's distributions in another currency, at the rate of today
    for (i, read) in &inputs.facts.declared {
        if n.payers.contains(i) {
            for d in &read.items {
                n.rate(d.amount.currency, today);
            }
        }
    }
    n
}
