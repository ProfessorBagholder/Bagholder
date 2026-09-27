//! What the book paid out and what it pays (`SPEC.md` §2 Distribution and
//! Distribution rate).
//!
//! A cashflow row is one dividend, interest payment, withholding tax or interest
//! charge, in the currency it was paid, with its CAD value on its day; a stock
//! dividend is a dividend row paid in units. A payer's rate is its per-unit
//! amount and its payments per year, both from the payer's own record (its
//! fund company's publication, its own announcement): the cash per unit of the
//! latest distribution gone ex, and the schedule the payer states. Neither is
//! ever worked out from the payments received or assumed; until the payer's
//! record is read, the figure waits on it.

use std::collections::BTreeMap;

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::transaction::Kind;
use bagholder_core::{AccountId, Dec, InstrumentId, Money, Rounding, SourceName, TransactionId};

use crate::fx::to_cad;
use crate::gap::{Fig, Gap, Gaps};
use crate::input::Inputs;
use crate::ledger::Matched;

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
    pub per: Option<Fig<Money>>,
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
        // the units the source states it was paid on (a payment moves no units)
        let qty = t.paid_on.filter(|q| q.is_positive());
        let per: Option<Fig<Money>> = qty.map(|q| Ok(Money::new(cash.amount.abs().div_rounded(q, crate::trades::PRICE_PLACES, Rounding::HalfEven)?, cash.currency)));
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
}

/// Where its payments per year came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrequencySource {
    Stated(SourceName),
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
    /// of the last known one; without a record, the ex-date the listing's quote
    /// reports and the day of the last payment received.
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

/// How a distribution whose source does not state its form was paid, as the
/// record shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Found {
    Cash,
    Units,
    NotYet,
}

/// Business days a paid distribution takes to post after its pay date, at most.
const POSTS_WITHIN: i64 = 2;

/// The last day a distribution paid on `pay` posts by: `POSTS_WITHIN` business
/// days after it.
pub(crate) fn posted_by(pay: Date) -> Option<Date> {
    let mut day = pay;
    let mut left = POSTS_WITHIN;
    while left > 0 {
        day = day.tomorrow().ok()?;
        if !matches!(day.weekday(), bagholder_core::jiff::civil::Weekday::Saturday | bagholder_core::jiff::civil::Weekday::Sunday) {
            left -= 1;
        }
    }
    Some(day)
}

/// The ex-date of the declared distribution a payment is for: the one the
/// payer's record lists whose pay date the payment posted on or within
/// `POSTS_WITHIN` business days of; the market's record beside it only where
/// the payer's lists none. None where no listed distribution matches, or where
/// those that match name different ex-dates.
pub fn entitled_ex(inputs: &Inputs, r: &CashRow) -> Option<Date> {
    let read = inputs.facts.declared.get(&r.instrument?)?;
    let matching = |list: &[crate::input::Declared]| -> Vec<Date> {
        let mut exes: Vec<Date> = list.iter().filter(|d| d.pay_date.is_some_and(|p| p <= r.day && posted_by(p).is_some_and(|by| r.day <= by))).map(|d| d.ex_date).collect();
        exes.sort();
        exes.dedup();
        exes
    };
    let mut exes = matching(&read.items);
    if exes.is_empty() {
        exes = matching(&read.market);
    }
    match exes.as_slice() {
        [ex] => Some(*ex),
        _ => None,
    }
}

/// The form of a distribution its source lists without saying (`SPEC.md` §2,
/// Distribution rate): cash where a dividend was paid on the instrument between
/// its ex-date and the next; units where none had posted two business days after
/// its pay date in an account that held the instrument on its ex-date (a
/// distribution paid in units that are then consolidated posts no row); not
/// known yet otherwise.
fn found_form(d: &crate::input::Declared, next_ex: Option<Date>, paid: &[&CashRow], held_on_ex: bool, today: Date) -> Found {
    if paid.iter().any(|r| r.day >= d.ex_date && next_ex.is_none_or(|n| r.day < n)) {
        return Found::Cash;
    }
    let posted_by = d.pay_date.and_then(posted_by);
    match posted_by {
        Some(by) if held_on_ex && today > by => Found::Units,
        _ => Found::NotYet,
    }
}

/// Every instrument that has paid a dividend or has a declared record: its rate.
pub fn payer_rates(inputs: &Inputs, rows: &[CashRow], matched: &Matched) -> BTreeMap<InstrumentId, PayerRate> {
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
        // every row of the payer's record counts; what the figures are is income
        // paid, so a row reinvested whole pays nothing and is passed over
        let cash: Vec<_> = read.map(|r| r.items.iter().filter(|d| d.amount.amount.is_positive()).collect()).unwrap_or_default();
        // the rate: the cash per unit of the latest distribution gone ex, from the
        // payer's own record and nothing else. A row whose source does not state
        // its form counts as cash once the record shows it paid in cash, is passed
        // over once it shows it paid in units, and holds the rate until then.
        let mut gone: Vec<&&crate::input::Declared> = cash.iter().filter(|d| d.ex_date <= today).collect();
        gone.sort_by_key(|d| std::cmp::Reverse(d.ex_date));
        let accounts: Vec<AccountId> = matched.units.keys().filter(|(_, x)| *x == i).map(|(a, _)| *a).collect();
        let held_on = |day: Date| accounts.iter().any(|a| day.yesterday().ok().is_some_and(|before| matched.units_on(*a, i, before).is_ok_and(|q| q.is_positive())));
        let mut latest: Result<Option<&crate::input::Declared>, Gap> = Ok(None);
        for (n, d) in gone.iter().enumerate() {
            let use_it = match d.form {
                bagholder_core::distribution::Form::Stated => true,
                bagholder_core::distribution::Form::Unstated => {
                    let next_ex = if n == 0 { None } else { Some(gone[n - 1].ex_date) };
                    match found_form(d, next_ex, &payments, held_on(d.ex_date), today) {
                        Found::Cash => true,
                        Found::Units => false,
                        Found::NotYet => {
                            latest = Err(Gap::FormUnstated(i));
                            break;
                        }
                    }
                }
            };
            if use_it {
                latest = Ok(Some(d));
                break;
            }
        }
        let (per, source) = match (latest, read) {
            (Err(g), Some(_)) => (Err(Gaps::of(g)), None),
            (Ok(Some(d)), Some(r)) => (Ok(d.amount), Some(RateSource::Declared(r.source.clone()))),
            (Ok(None), Some(_)) => (Err(Gaps::of(Gap::NoDistributionYet(i))), None),
            (_, None) => (Err(Gaps::of(Gap::PayerNotRead(i))), None),
        };
        // payments per year: the payer's own statement, never worked out; a
        // record read that states none waits on a schedule no source states
        let (per_year, frequency_source) = match (inputs.facts.frequencies.get(&i), read) {
            (Some(s), _) => (Ok(s.value), Some(FrequencySource::Stated(s.source.clone()))),
            (None, Some(_)) => (Err(Gaps::of(Gap::ScheduleUnstated(i))), None),
            (None, None) => (Err(Gaps::of(Gap::PayerNotRead(i))), None),
        };
        // the next distribution still to be paid, whether or not it has gone ex:
        // the payer's record's; where it lists none, the market's record beside
        // it (a company's publication lists a declaration days after the
        // exchange does), counting only what it lists after the payer's latest;
        // else the payer's last
        let due = |d: &&crate::input::Declared| d.pay_date.unwrap_or(d.ex_date);
        let mut all: Vec<&crate::input::Declared> = cash.to_vec();
        all.sort_by_key(|d| (due(d), d.ex_date));
        let latest_listed = all.iter().map(|d| d.ex_date).max();
        let mut announced: Vec<&crate::input::Declared> = read.map(|r| r.market.iter().filter(|d| d.amount.amount.is_positive() && latest_listed.is_none_or(|l| d.ex_date > l)).collect()).unwrap_or_default();
        announced.sort_by_key(|d| (due(d), d.ex_date));
        let next = all.iter().find(|d| due(d) >= today).or_else(|| announced.iter().find(|d| due(d) >= today)).or(all.last());
        // without a record: the ex-date the listing's quote reports, and the day
        // of the last payment received
        let (next_ex, next_pay) = match next {
            Some(d) => (Some(d.ex_date), d.pay_date),
            None => (inputs.market.ex_dividends.get(&i).copied(), payments.iter().map(|r| r.day).max()),
        };
        out.insert(i, PayerRate { instrument: i, per, source, per_year, frequency_source, next_ex, next_pay });
    }
    out
}
