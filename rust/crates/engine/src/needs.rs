//! The facts the figures use (`docs/plans/stage-3a-sources.md`, "The command"):
//! which currencies the engine converts and from which day, which instruments'
//! closes it values and over which days, and which payers are held. The readers
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
    /// Each instrument whose close a figure uses, and the first and last day it
    /// does: the days it is held (to today while it is held), and the expiry day
    /// of a contract that expired open, for its underlying.
    pub closes: BTreeMap<InstrumentId, (Date, Date)>,
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

    fn close(&mut self, i: InstrumentId, from: Date, to: Date) {
        let e = self.closes.entry(i).or_insert((from, to));
        *e = (e.0.min(from), e.1.max(to));
    }
}

/// The days a holding's units are not zero, as spans of first and last day; the
/// last span runs to today while it is held.
fn held_spans(units: &BTreeMap<Date, bagholder_core::Dec>, today: Date) -> Vec<(Date, Date)> {
    let mut out = Vec::new();
    let mut open: Option<Date> = None;
    for (day, q) in units {
        match (open, q.is_zero()) {
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
    // each holding, valued at its close in its own currency over the days it is held
    for ((_, i), units) in &matched.units {
        for (from, to) in held_spans(units, today) {
            n.close(*i, from, to);
            if let Some(c) = currency(i) {
                n.rate(c, from);
            }
            // a coin is closed in its USD market where its own pair has none
            if kind(i) == Some(InstrumentKind::Crypto) {
                n.rate(Currency::USD, from);
            }
            if kind(i) == Some(InstrumentKind::Security) && to >= today {
                n.payers.insert(*i);
            }
            // a contract held into its expiry: its underlying's close that day
            // says whether it expired worthless
            if let Some(terms) = ledger.instruments.get(i).and_then(|x| x.terms.as_ref()) {
                if terms.expiry >= from && terms.expiry <= to.min(today) {
                    n.close(terms.underlying, terms.expiry, terms.expiry);
                }
            }
        }
    }
    // a close stored in another currency than its instrument's is converted on its day
    for (i, closes) in &inputs.market.closes {
        let Some((from, to)) = n.closes.get(i).copied() else { continue };
        for (day, m) in closes.range(from..=to) {
            if Some(m.currency) != currency(i) {
                n.rate(m.currency, *day);
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
