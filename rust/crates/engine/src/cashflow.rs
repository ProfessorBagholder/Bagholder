//! What the book paid out and what it pays (`SPEC.md` §2 Distribution and
//! Distribution rate).
//!
//! A cashflow row is one dividend, interest payment, withholding tax or interest
//! charge, in the currency it was paid, with its CAD value on its day; a stock
//! dividend is a dividend row paid in units. A payer's rate is its per-unit
//! amount and its payments per year: the fund's declared record first, then the
//! payments received; the frequency a source states first, then read from the
//! declared ex-dates, then from the payments; with none of these it is not
//! known, never assumed.

use std::collections::BTreeMap;

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::transaction::Kind;
use bagholder_core::{AccountId, Dec, InstrumentId, Money, Rounding, SourceName, TransactionId};

use crate::fx::to_cad;
use crate::gap::{Fig, Gap, Gaps};
use crate::input::{DistributionKind, Inputs};
use crate::ledger::Matched;
use crate::stat::payments_per_year;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Payment {
    Dividend,
    Interest,
    WithholdingTax,
    InterestCharge,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CashRow {
    pub id: TransactionId,
    pub day: Date,
    pub at: Option<Timestamp>,
    pub kind: Payment,
    pub account: AccountId,
    pub instrument: Option<InstrumentId>,
    /// Units the row states it paid on.
    pub qty: Option<Dec>,
    /// Per unit, where the row states its units.
    pub per: Option<Money>,
    /// In the currency it was paid.
    pub amount: Money,
    pub amount_cad: Fig<Money>,
    /// Paid in units (a stock dividend): how many.
    pub in_units: Option<Dec>,
}

pub fn build_cashflow(inputs: &Inputs, matched: &Matched) -> Vec<CashRow> {
    let rates = &inputs.facts.rates;
    let clock = &inputs.clock;
    let mut rows = Vec::new();
    for t in &inputs.ledger.transactions {
        let kind = match t.kind {
            Kind::Dividend => Payment::Dividend,
            Kind::Interest => Payment::Interest,
            Kind::WithholdingTax => Payment::WithholdingTax,
            Kind::InterestCharge => Payment::InterestCharge,
            _ => continue,
        };
        // a dividend of no cash is the broker's notice of one to come, not a payment
        let Some(cash) = t.cash.filter(|c| !c.amount.is_zero()) else { continue };
        let qty = t.quantity.filter(|q| !q.is_zero()).map(|q| q.abs());
        let per = qty.and_then(|q| cash.amount.abs().div_rounded(q, crate::trades::PRICE_PLACES, Rounding::HalfEven).ok()).map(|v| Money::new(v, cash.currency));
        rows.push(CashRow {
            id: t.id.clone(),
            day: t.trade_date,
            at: t.occurred_at,
            kind,
            account: t.account,
            instrument: t.instrument,
            qty,
            per,
            amount: cash,
            amount_cad: to_cad(rates, clock, cash, t.trade_date),
            in_units: None,
        });
    }
    for s in &matched.stock_dividends {
        rows.push(CashRow {
            id: s.transaction.clone(),
            day: s.day,
            at: None,
            kind: Payment::Dividend,
            account: s.account,
            instrument: Some(s.instrument),
            qty: None,
            per: None,
            amount: s.value,
            amount_cad: to_cad(rates, clock, s.value, s.day),
            in_units: Some(s.units),
        });
    }
    rows.sort_by(|a, b| (b.day, b.at, &b.id).cmp(&(a.day, a.at, &a.id)));
    rows
}

/// Where a payer's rate came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RateSource {
    Declared(SourceName),
    Payments,
}

/// Where its payments per year came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrequencySource {
    Stated(SourceName),
    DeclaredDates,
    Payments,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PayerRate {
    pub instrument: InstrumentId,
    /// Per unit, in the currency it is paid in.
    pub per: Fig<Money>,
    pub source: Option<RateSource>,
    pub per_year: Fig<u32>,
    pub frequency_source: Option<FrequencySource>,
    /// The ex-date and pay date of the next distribution still to be paid, else
    /// of the last known one.
    pub next_ex: Option<Date>,
    pub next_pay: Option<Date>,
}

impl PayerRate {
    /// Per unit × payments per year.
    pub fn annual_per_unit(&self) -> Fig<Money> {
        let per = self.per.clone();
        let n = self.per_year.clone();
        crate::gap::both(per, n, |p, n| Ok(p.times(Dec::from_int(n as i64))?))
    }
}

/// Every instrument that has paid a dividend or has a declared record: its rate.
pub fn payer_rates(inputs: &Inputs, rows: &[CashRow]) -> BTreeMap<InstrumentId, PayerRate> {
    let today = inputs.clock.today;
    let mut paid: BTreeMap<InstrumentId, Vec<&CashRow>> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.kind == Payment::Dividend && r.in_units.is_none()) {
        if let Some(i) = r.instrument {
            paid.entry(i).or_default().push(r);
        }
    }
    let mut instruments: Vec<InstrumentId> = paid.keys().copied().collect();
    instruments.extend(inputs.facts.declared.keys().copied());
    instruments.sort();
    instruments.dedup();
    let mut out = BTreeMap::new();
    for i in instruments {
        let payments = paid.get(&i).cloned().unwrap_or_default();
        let read = inputs.facts.declared.get(&i);
        let regular: Vec<_> = read.map(|r| r.items.iter().filter(|d| d.kind == DistributionKind::Regular).collect()).unwrap_or_default();
        // the rate: the latest regular distribution that has gone ex, else the
        // latest payment that states its units
        let latest = regular.iter().filter(|d| d.ex_date <= today).max_by_key(|d| d.ex_date);
        let (per, source) = match (latest, read) {
            (Some(d), Some(r)) => (Ok(d.amount), Some(RateSource::Declared(r.source.clone()))),
            _ => match payments.iter().filter(|r| r.per.is_some()).max_by_key(|r| (r.day, r.at)) {
                Some(r) => (Ok(r.per.expect("filtered")), Some(RateSource::Payments)),
                None => (Err(Gaps::of(Gap::DistributionUnknown(i))), None),
            },
        };
        // payments per year: stated, then the declared ex-dates, then the payments
        let stated = inputs.facts.frequencies.get(&i);
        let from_declared = payments_per_year(&regular.iter().map(|d| d.ex_date).collect::<Vec<_>>());
        let from_paid = payments_per_year(&payments.iter().map(|r| r.day).collect::<Vec<_>>());
        let (per_year, frequency_source) = match (stated, from_declared, from_paid) {
            (Some(s), _, _) => (Ok(s.value), Some(FrequencySource::Stated(s.source.clone()))),
            (None, Some(n), _) => (Ok(n), Some(FrequencySource::DeclaredDates)),
            (None, None, Some(n)) => (Ok(n), Some(FrequencySource::Payments)),
            _ => (Err(Gaps::of(Gap::FrequencyUnknown(i))), None),
        };
        // the next distribution still to be paid, whether or not it has gone ex
        let due = |d: &&&crate::input::Declared| d.pay_date.unwrap_or(d.ex_date);
        let mut all: Vec<_> = regular.iter().collect();
        all.sort_by_key(|d| (due(d), d.ex_date));
        let next = all.iter().find(|d| due(d) >= today).or(all.last());
        let (next_ex, next_pay) = match next {
            Some(d) => (Some(d.ex_date), d.pay_date),
            None => (None, payments.iter().map(|r| r.day).max()),
        };
        out.insert(i, PayerRate { instrument: i, per, source, per_year, frequency_source, next_ex, next_pay });
    }
    out
}
