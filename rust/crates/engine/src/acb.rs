//! The tax cost of each ETF held outside registered plans (brief 82, PR 3;
//! `SPEC.md` `tax-cost-of-an-etf-1` to `tax-cost-of-an-etf-10`). One holding per fund: one person's units of
//! it across all their non-registered accounts, at average cost, in Canadian
//! dollars, each event on its own day. Derived whole on every read from the book,
//! the facts and what the tax inputs state; nothing is kept. Nothing is
//! estimated either: a figure that cannot be stated exactly leaves its year
//! incomplete, saying why, and every year after it waits on it.
//!
//! Trades' per-account first-in first-out lots are not touched: this is a
//! separate reading of the same rows.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::account::{AccountType, Registration};
use bagholder_core::jiff::civil::{Date, Weekday};
use bagholder_core::transaction::{Kind, Transaction};
use bagholder_core::{AccountId, Currency, Dec, DecError, InstrumentId, Money, Rounding, TransactionId};

use crate::input::Inputs;

/// Where a fund lists. A Canadian-listed fund's distributions move its cost; a
/// US-listed one's never do, whatever the fund labels them (`tax-cost-of-an-etf-8`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Listed {
    Canada,
    UnitedStates,
}

/// One slip's figures for a fund and a year, in dollars as the slip prints them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slip {
    /// Box 42 on a T3, in CRA's sign: positive lowers the cost, negative raises it.
    pub return_of_capital: Dec,
    /// The income it allocates, in actual amounts (not grossed up), every box.
    pub income: Dec,
    /// The foreign tax it states paid.
    pub foreign_tax: Dec,
}

/// What a source states per unit for one of a fund's record dates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerUnit {
    pub return_of_capital: Dec,
    pub non_cash: Dec,
    /// Whose figure it is, as the ledger's row names it.
    pub source: String,
}

/// What the ledger reads beside the book and the facts, each handed in.
#[derive(Clone, Debug, Default)]
pub struct TaxInputs {
    /// The ETFs, by where each lists. A security not here is not an ETF.
    pub funds: BTreeMap<InstrumentId, Listed>,
    /// Each year's slips for a fund, one per account it was issued for.
    pub slips: BTreeMap<(InstrumentId, i16), Vec<Slip>>,
    /// Per fund, per record date.
    pub per_unit: BTreeMap<InstrumentId, BTreeMap<Date, PerUnit>>,
    /// The day a year's non-cash distribution is dated, where its fund states
    /// one (a per-distribution field, or its year-end release).
    pub non_cash_on: BTreeMap<(InstrumentId, i16), Date>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Bought,
    Sold,
    /// Units moved in from outside the holding at a cost the person entered.
    MovedIn,
    ReturnOfCapital,
    NonCash,
    /// The cost went below zero: that much is a gain that day, the cost zero.
    BelowZero,
    /// A superficial loss added to the cost of the units that replaced them.
    LossAdded,
}

/// Where a row's amount comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Book(TransactionId),
    /// Entered by the person (`what-you-enter-3`, `what-you-enter-9`).
    Entered(TransactionId),
    /// The fund's own per-unit figure for that record date.
    Fund(String),
    /// The year's slips, placed in proportion to the person's own cash (`tax-cost-of-an-etf-5`).
    Slip,
}

/// One row of a holding's ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub day: Date,
    pub event: Event,
    /// The units it moved, signed.
    pub units: Dec,
    /// What it did to the cost, signed, in CAD.
    pub cad: Dec,
    pub units_after: Dec,
    pub cost_after: Dec,
    /// The Bank of Canada rate it took, where it converted an amount.
    pub rate: Option<Dec>,
    pub source: Source,
}

impl Row {
    /// The cost per unit after the row, where units are held.
    pub fn per_unit(&self) -> Option<Dec> {
        if self.units_after.is_positive() {
            self.cost_after.div_rounded(self.units_after, 4, Rounding::HalfEven).ok()
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GainKind {
    Sale,
    /// The cost went below zero (CRA: line 13200 with a zero on line 13199).
    BelowZero,
}

/// A gain or loss for Schedule 3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gain {
    pub kind: GainKind,
    pub day: Date,
    pub transaction: Option<TransactionId>,
    pub units: Dec,
    pub proceeds: Dec,
    pub cost: Dec,
    pub outlays: Dec,
    /// Proceeds less cost and outlays, the denied part of a loss taken back.
    pub gain: Dec,
    /// The part of a loss denied as superficial, as a positive amount.
    pub denied: Dec,
}

/// Why a year is not complete.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Missing {
    /// Distributions were paid that year and no slip for it is in.
    Slip,
    /// A distribution row matches no declared distribution's record date.
    RecordDate(TransactionId),
    Rate { currency: Currency, day: Date },
    /// The fund's per-unit figures and the slip differ by more than a cent per
    /// distribution, by this much (the slip less the figures).
    SlipDiffers(Dec),
    /// Units moved in from outside the holding with no cost entered.
    MovedInCost(TransactionId),
    /// Units moved out to a registered plan or another person: a disposition
    /// whose value is not stated.
    MovedOut(TransactionId),
    /// A loss's superficial-loss period has not ended: it ends that day.
    PeriodOpen(Date),
    /// An account whose plan is not known holds the fund.
    AccountPlan(AccountId),
    /// A corporate event on the fund, which this ledger does not read.
    CorporateEvent(TransactionId),
    /// A sale of more units than the holding holds.
    Units(TransactionId),
    /// An earlier year is not complete, and this year's cost carries it.
    EarlierYear(i16),
    /// A figure too large to state.
    Arithmetic,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Year {
    pub gains: Vec<Gain>,
    pub missing: BTreeSet<Missing>,
}

impl Year {
    pub fn complete(&self) -> bool {
        self.missing.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holding {
    pub fund: InstrumentId,
    pub listed: Listed,
    pub rows: Vec<Row>,
    pub years: BTreeMap<i16, Year>,
    pub units: Dec,
    pub cost: Dec,
}

impl Holding {
    /// The last year complete with every year before it, if any (`tax-cost-of-an-etf-9`).
    pub fn complete_through(&self) -> Option<i16> {
        self.years.iter().take_while(|(_, y)| y.complete()).last().map(|(y, _)| *y)
    }
}

/// T+1 from these trade dates, T+2 before (`tax-cost-of-an-etf-2`).
fn t_plus_one_from(listed: Listed) -> Date {
    match listed {
        Listed::Canada => Date::constant(2024, 5, 27),
        Listed::UnitedStates => Date::constant(2024, 5, 28),
    }
}

/// The superficial-loss period's reach each side of a sale's settlement (`tax-cost-of-an-etf-7`).
const PERIOD_DAYS: i64 = 30;

/// A day's settlement: as the row states it, else its trade date plus the
/// business days of its time.
pub fn settlement(t: &Transaction, listed: Listed, holidays: &BTreeSet<Date>) -> Date {
    if let Some(d) = t.settle_date {
        return d;
    }
    let mut n = if t.trade_date >= t_plus_one_from(listed) { 1 } else { 2 };
    let mut d = t.trade_date;
    while n > 0 {
        d = match d.tomorrow() {
            Ok(x) => x,
            Err(_) => return d,
        };
        if !matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday) && !holidays.contains(&d) {
            n -= 1;
        }
    }
    d
}

fn cents(d: Dec) -> Dec {
    d.round(2, Rounding::HalfEven)
}

fn year_of(d: Date) -> i16 {
    d.year()
}

/// Where an account stands for the ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// Non-registered, held alone: part of the holding.
    Holding,
    /// A registered plan: counts for the superficial-loss rule alone.
    Registered,
    /// Held jointly: not in.
    Joint,
    Unknown,
}

fn place(inputs: &Inputs, account: AccountId) -> Place {
    match inputs.ledger.accounts.get(&account).map(|a| &a.account.account_type) {
        Some(AccountType::Known { joint: true, .. }) => Place::Joint,
        Some(AccountType::Known { registration: Registration::Unregistered, .. }) => Place::Holding,
        Some(AccountType::Known { .. }) => Place::Registered,
        _ => Place::Unknown,
    }
}

/// One step of the walk, in the order a day takes them: sales first, then a
/// return of capital, then a non-cash distribution, then purchases (`tax-cost-of-an-etf-4`).
#[derive(Clone, Debug)]
enum Step {
    Sell { t: TransactionId, units: Dec, proceeds: Dec, outlays: Dec, rate: Option<Dec> },
    Roc { amount: Dec, source: Source },
    NonCash { amount: Dec, source: Source },
    Buy { t: TransactionId, units: Dec, cost: Dec, rate: Option<Dec>, entered: bool },
}

impl Step {
    fn rank(&self) -> u8 {
        match self {
            Step::Sell { .. } => 0,
            Step::Roc { .. } => 1,
            Step::NonCash { .. } => 2,
            Step::Buy { .. } => 3,
        }
    }
}

/// Every ETF's holding, the funds in `tax.funds` that a non-registered account
/// of the person's has held.
pub fn holdings(inputs: &Inputs, tax: &TaxInputs) -> Vec<Holding> {
    let mut out = Vec::new();
    for (fund, listed) in &tax.funds {
        let rows: Vec<&Transaction> = inputs.ledger.transactions.of_instrument(*fund).collect();
        if !rows.iter().any(|t| place(inputs, t.account) == Place::Holding) {
            continue;
        }
        match holding(inputs, tax, *fund, *listed, &rows) {
            Ok(h) => out.push(h),
            Err(_) => {
                let mut years = BTreeMap::new();
                for t in &rows {
                    years.entry(year_of(t.trade_date)).or_insert_with(Year::default).missing.insert(Missing::Arithmetic);
                }
                out.push(Holding { fund: *fund, listed: *listed, rows: vec![], years, units: Dec::ZERO, cost: Dec::ZERO });
            }
        }
    }
    out
}

struct Cad<'a> {
    inputs: &'a Inputs,
}

impl Cad<'_> {
    /// An amount in CAD on a day, to the cent: CAD itself, else at the Bank of
    /// Canada's rate in effect that day (`tax-cost-of-an-etf-2`).
    fn of(&self, m: Money, day: Date, missing: &mut Vec<(i16, Missing)>, year: i16) -> Result<(Dec, Option<Dec>), DecError> {
        if m.currency == Currency::CAD {
            return Ok((cents(m.amount), None));
        }
        match crate::fx::rate(&self.inputs.facts.rates, &self.inputs.clock, m.currency, day) {
            Ok(r) => Ok((cents(m.amount.checked_mul(r)?), Some(r))),
            Err(_) => {
                missing.push((year, Missing::Rate { currency: m.currency, day }));
                Ok((Dec::ZERO, None))
            }
        }
    }
}

fn holding(inputs: &Inputs, tax: &TaxInputs, fund: InstrumentId, listed: Listed, rows: &[&Transaction]) -> Result<Holding, DecError> {
    let holidays = &inputs.facts.rates.holidays;
    let cad = Cad { inputs };
    let mut missing: Vec<(i16, Missing)> = Vec::new();
    let settle = |t: &Transaction| settlement(t, listed, holidays);
    let in_holding = |a: AccountId| place(inputs, a) == Place::Holding;
    let links = &inputs.ledger.transfer_links;
    let other_side = |t: &Transaction| -> Option<AccountId> {
        let id = links.received_by(&t.id).or_else(|| links.sent_by(&t.id))?;
        inputs.ledger.transactions.get(id).map(|o| o.account)
    };

    // every unit the person holds of the fund, by settlement, in any account of
    // theirs: the superficial-loss rule counts registered plans too
    let mut all_units: Vec<(Date, Dec)> = Vec::new();
    // purchases in any account of theirs, by settlement
    let mut bought_any: Vec<(Date, Dec)> = Vec::new();
    // the holding's own units, by settlement, for each record date
    let mut held_units: Vec<(Date, Dec)> = Vec::new();
    let mut steps: Vec<(Date, Step, usize)> = Vec::new();
    // the holding's cash distributions: (record date, CAD, its transaction)
    let mut paid: Vec<(Date, Dec, TransactionId)> = Vec::new();

    for (n, t) in rows.iter().enumerate() {
        let p = place(inputs, t.account);
        let year = year_of(t.trade_date);
        match p {
            Place::Joint => continue,
            Place::Unknown => {
                missing.push((year, Missing::AccountPlan(t.account)));
                continue;
            }
            _ => {}
        }
        let qty = t.quantity.unwrap_or(Dec::ZERO);
        let day = settle(t);
        let moves_units = matches!(t.kind, Kind::Buy | Kind::Sell | Kind::TransferIn | Kind::TransferOut);
        if moves_units && !qty.is_zero() {
            all_units.push((day, qty));
            if t.kind == Kind::Buy && qty.is_positive() {
                bought_any.push((day, qty));
            }
        }
        if p != Place::Holding {
            continue;
        }
        let year = year_of(day);
        match t.kind {
            Kind::Buy if qty.is_positive() => {
                let Some(cash) = t.cash else {
                    missing.push((year, Missing::Units(t.id.clone())));
                    continue;
                };
                let (c, rate) = cad.of(cash.neg(), day, &mut missing, year)?;
                held_units.push((day, qty));
                steps.push((day, Step::Buy { t: t.id.clone(), units: qty, cost: c, rate, entered: false }, n));
            }
            Kind::Sell if qty.is_negative() => {
                let Some(cash) = t.cash else {
                    missing.push((year, Missing::Units(t.id.clone())));
                    continue;
                };
                // cash is what the sale received, its commission already out: the
                // proceeds are both, the commission an outlay (`tax-cost-of-an-etf-3`)
                let fee = t.fee.unwrap_or(Money::zero(cash.currency));
                let (received, rate) = cad.of(cash, day, &mut missing, year)?;
                let (outlays, _) = cad.of(fee, day, &mut missing, year)?;
                let proceeds = received.checked_add(outlays)?;
                held_units.push((day, qty));
                steps.push((day, Step::Sell { t: t.id.clone(), units: qty.neg(), proceeds, outlays, rate }, n));
            }
            Kind::TransferIn | Kind::TransferOut if !qty.is_zero() => {
                // a move between the person's own non-registered accounts leaves the holding as it is
                if other_side(t).is_some_and(in_holding) {
                    continue;
                }
                if qty.is_negative() {
                    missing.push((year, Missing::MovedOut(t.id.clone())));
                    held_units.push((day, qty));
                    continue;
                }
                // the cost the person entered (`what-you-enter-3`), in CAD on the day acquired
                let entered = inputs.facts.adjustments.get(&t.id).and_then(|a| a.legs.iter().find_map(|l| l.cost.map(|c| (c, l.acquired))));
                match entered {
                    Some((cost, acquired)) => {
                        let on = acquired.unwrap_or(day);
                        let (c, rate) = cad.of(cost, on, &mut missing, year)?;
                        held_units.push((day, qty));
                        steps.push((day, Step::Buy { t: t.id.clone(), units: qty, cost: c, rate, entered: true }, n));
                    }
                    None => {
                        missing.push((year, Missing::MovedInCost(t.id.clone())));
                        held_units.push((day, qty));
                        steps.push((day, Step::Buy { t: t.id.clone(), units: qty, cost: Dec::ZERO, rate: None, entered: true }, n));
                    }
                }
            }
            Kind::CorporateEvent => missing.push((year_of(t.trade_date), Missing::CorporateEvent(t.id.clone()))),
            // a US-listed fund's distributions never touch its cost (`tax-cost-of-an-etf-8`)
            Kind::Dividend if listed == Listed::Canada => {
                let Some(cash) = t.cash.filter(|c| c.amount.is_positive()) else { continue };
                match record_date(inputs, fund, t) {
                    Some(rd) => {
                        let (c, _) = cad.of(cash, t.trade_date, &mut missing, year_of(rd))?;
                        paid.push((rd, c, t.id.clone()));
                    }
                    None => missing.push((year_of(t.trade_date), Missing::RecordDate(t.id.clone()))),
                }
            }
            _ => {}
        }
    }
    held_units.sort();
    all_units.sort();
    bought_any.sort();
    let held_on = |d: Date| -> Result<Dec, DecError> { held_units.iter().take_while(|(s, _)| *s <= d).try_fold(Dec::ZERO, |a, (_, q)| a.checked_add(*q)) };

    // each year's return of capital and non-cash distribution (`tax-cost-of-an-etf-4`, `tax-cost-of-an-etf-5`)
    let mut by_year: BTreeMap<i16, Vec<(Date, Dec, TransactionId)>> = BTreeMap::new();
    for p in paid {
        by_year.entry(year_of(p.0)).or_default().push(p);
    }
    let figures = tax.per_unit.get(&fund);
    for (year, mut rows) in by_year {
        rows.sort();
        let slips = tax.slips.get(&(fund, year));
        let slip = slips.map(|s| -> Result<Slip, DecError> {
            s.iter().try_fold(Slip::default(), |a, x| {
                Ok(Slip { return_of_capital: a.return_of_capital.checked_add(x.return_of_capital)?, income: a.income.checked_add(x.income)?, foreign_tax: a.foreign_tax.checked_add(x.foreign_tax)? })
            })
        });
        let slip = slip.transpose()?;
        // the person's own entry for a distribution (`what-you-enter-9`), else the fund's figure
        let entered_roc = |t: &TransactionId| inputs.facts.adjustments.get(t).and_then(|a| a.legs.iter().find_map(|l| l.cash_per_unit.map(|c| c.amount)));
        let per: Vec<Option<(Dec, Dec, Source)>> = rows
            .iter()
            .map(|(rd, _, t)| match entered_roc(t) {
                Some(roc) => Some((roc, figures.and_then(|f| f.get(rd)).map_or(Dec::ZERO, |f| f.non_cash), Source::Entered(t.clone()))),
                None => figures.and_then(|f| f.get(rd)).map(|f| (f.return_of_capital, f.non_cash, Source::Fund(f.source.clone()))),
            })
            .collect();
        if per.iter().all(Option::is_some) {
            // per record date, the units the holding held that day, the slip the check
            let mut rocs = Vec::new();
            for ((rd, _, _), f) in rows.iter().zip(&per) {
                let (roc, nc, source) = f.clone().expect("all present");
                let units = held_on(*rd)?;
                rocs.push((*rd, cents(roc.checked_mul(units)?), source.clone()));
                if !nc.is_zero() {
                    steps.push((*rd, Step::NonCash { amount: cents(nc.checked_mul(units)?), source }, usize::MAX));
                }
            }
            if let Some(s) = &slip {
                let total = rocs.iter().try_fold(Dec::ZERO, |a, r| a.checked_add(r.1))?;
                let differs = s.return_of_capital.checked_sub(total)?;
                let allowed = Dec::new(1, 2)?.checked_mul(Dec::from_int(rocs.len() as i64))?;
                if differs.abs() > allowed {
                    missing.push((year, Missing::SlipDiffers(differs)));
                } else if let Some(last) = rocs.last_mut() {
                    // a difference of rounding goes on the year's last distribution
                    last.1 = last.1.checked_add(differs)?;
                }
            } else {
                missing.push((year, Missing::Slip));
            }
            for (rd, amount, source) in rocs {
                steps.push((rd, Step::Roc { amount, source }, usize::MAX));
            }
        } else if let Some(s) = slip {
            // the slip's box 42 placed by the person's own cash on each record
            // date, exact under one annual ratio; the last takes what rounding
            // left. A distribution the person entered a return of capital for
            // keeps theirs, and the rest of the box is placed among the others.
            let mut rest = s.return_of_capital;
            let mut open = Vec::new();
            for (row, f) in rows.iter().zip(&per) {
                match f {
                    Some((roc, _, source @ Source::Entered(_))) => {
                        let amount = cents(roc.checked_mul(held_on(row.0)?)?);
                        rest = rest.checked_sub(amount)?;
                        steps.push((row.0, Step::Roc { amount, source: source.clone() }, usize::MAX));
                    }
                    _ => open.push(row),
                }
            }
            let cash = open.iter().try_fold(Dec::ZERO, |a, r| a.checked_add(r.1))?;
            let mut placed = Dec::ZERO;
            for (i, (rd, c, _)) in open.iter().enumerate() {
                let share = if i + 1 == open.len() { rest.checked_sub(placed)? } else { rest.mul_div_rounded(*c, cash, 2, Rounding::HalfEven)? };
                placed = placed.checked_add(share)?;
                steps.push((*rd, Step::Roc { amount: share, source: Source::Slip }, usize::MAX));
            }
            let cash = rows.iter().try_fold(Dec::ZERO, |a, r| a.checked_add(r.1))?;
            // non-cash: what the slip allocates, less foreign tax, less the cash
            // paid on its record dates, plus the capital returned (`tax-cost-of-an-etf-5`)
            let net = s.income.checked_sub(s.foreign_tax)?.checked_sub(cash)?;
            let non_cash = net.checked_add(s.return_of_capital)?;
            if !non_cash.is_zero() {
                let on = tax.non_cash_on.get(&(fund, year)).copied().unwrap_or(rows.last().expect("a year has rows").0);
                steps.push((on, Step::NonCash { amount: cents(non_cash), source: Source::Slip }, usize::MAX));
            }
        } else {
            missing.push((year, Missing::Slip));
        }
    }

    steps.sort_by(|a, b| (a.0, a.1.rank(), a.2).cmp(&(b.0, b.1.rank(), b.2)));
    let today = inputs.clock.today;
    let units_at = |list: &[(Date, Dec)], d: Date| -> Result<Dec, DecError> { list.iter().take_while(|(s, _)| *s <= d).try_fold(Dec::ZERO, |a, (_, q)| a.checked_add(*q)) };
    let mut units = Dec::ZERO;
    let mut cost = Dec::ZERO;
    let mut out_rows = Vec::new();
    let mut years: BTreeMap<i16, Year> = BTreeMap::new();
    for (day, step, _) in steps {
        let year = year_of(day);
        years.entry(year).or_default();
        match step {
            Step::Buy { t, units: q, cost: c, rate, entered } => {
                units = units.checked_add(q)?;
                cost = cost.checked_add(c)?;
                let source = if entered { Source::Entered(t) } else { Source::Book(t) };
                out_rows.push(Row { day, event: if entered { Event::MovedIn } else { Event::Bought }, units: q, cad: c, units_after: units, cost_after: cost, rate, source });
            }
            Step::Sell { t, units: q, proceeds, outlays, rate } => {
                if q > units {
                    missing.push((year, Missing::Units(t.clone())));
                }
                let sold = q.min(units);
                let part = if sold == units { cost } else if units.is_positive() { cost.mul_div_rounded(sold, units, 2, Rounding::HalfEven)? } else { Dec::ZERO };
                units = units.checked_sub(sold)?;
                cost = cost.checked_sub(part)?;
                out_rows.push(Row { day, event: Event::Sold, units: sold.neg(), cad: part.neg(), units_after: units, cost_after: cost, rate, source: Source::Book(t.clone()) });
                let mut gain = proceeds.checked_sub(part)?.checked_sub(outlays)?;
                let mut denied = Dec::ZERO;
                if gain.is_negative() {
                    // a superficial loss (`tax-cost-of-an-etf-7`): units bought in the 61 days around the
                    // sale's settlement and held at its end, in any account of the person's
                    let first = day.checked_sub(bagholder_core::jiff::Span::new().days(PERIOD_DAYS)).unwrap_or(day);
                    let last = day.checked_add(bagholder_core::jiff::Span::new().days(PERIOD_DAYS)).unwrap_or(day);
                    if last >= today {
                        missing.push((year, Missing::PeriodOpen(last)));
                    }
                    let bought = bought_any.iter().filter(|(s, _)| *s >= first && *s <= last).try_fold(Dec::ZERO, |a, (_, b)| a.checked_add(*b))?;
                    let held_end = units_at(&all_units, last)?;
                    let substitute = q.min(bought).min(held_end);
                    if substitute.is_positive() {
                        denied = gain.neg().mul_div_rounded(substitute, q, 2, Rounding::HalfEven)?;
                        gain = gain.checked_add(denied)?;
                        // what the holding's own accounts hold of it is added to its cost;
                        // what a registered plan holds is gone for good
                        let own_end = units_at(&held_units, last)?;
                        let added = if own_end >= held_end { denied } else { denied.mul_div_rounded(own_end.max(Dec::ZERO), held_end, 2, Rounding::HalfEven)? };
                        if !added.is_zero() {
                            cost = cost.checked_add(added)?;
                            out_rows.push(Row { day, event: Event::LossAdded, units: Dec::ZERO, cad: added, units_after: units, cost_after: cost, rate: None, source: Source::Book(t.clone()) });
                        }
                    }
                }
                years.entry(year).or_default().gains.push(Gain { kind: GainKind::Sale, day, transaction: Some(t), units: sold, proceeds, cost: part, outlays, gain, denied });
            }
            Step::Roc { amount, source } => {
                cost = cost.checked_sub(amount)?;
                out_rows.push(Row { day, event: Event::ReturnOfCapital, units: Dec::ZERO, cad: amount.neg(), units_after: units, cost_after: cost, rate: None, source: source.clone() });
                if cost.is_negative() {
                    // below zero: a gain that day, the cost then zero (`tax-cost-of-an-etf-6`)
                    let g = cost.neg();
                    cost = Dec::ZERO;
                    out_rows.push(Row { day, event: Event::BelowZero, units: Dec::ZERO, cad: g, units_after: units, cost_after: cost, rate: None, source });
                    years.entry(year).or_default().gains.push(Gain { kind: GainKind::BelowZero, day, transaction: None, units: Dec::ZERO, proceeds: Dec::ZERO, cost: Dec::ZERO, outlays: Dec::ZERO, gain: g, denied: Dec::ZERO });
                }
            }
            Step::NonCash { amount, source } => {
                cost = cost.checked_add(amount)?;
                out_rows.push(Row { day, event: Event::NonCash, units: Dec::ZERO, cad: amount, units_after: units, cost_after: cost, rate: None, source });
            }
        }
    }
    for (y, m) in missing {
        years.entry(y).or_default().missing.insert(m);
    }
    // a year after one not complete carries its cost
    let mut first_open = None;
    for (y, entry) in years.iter_mut() {
        if let Some(f) = first_open {
            entry.missing.insert(Missing::EarlierYear(f));
        } else if !entry.complete() {
            first_open = Some(*y);
        }
    }
    Ok(Holding { fund, listed, rows: out_rows, years, units, cost })
}

/// A distribution row's record date: the declared distribution it pays, matched
/// by its pay date, where one states both.
fn record_date(inputs: &Inputs, fund: InstrumentId, t: &Transaction) -> Option<Date> {
    let declared = inputs.facts.declared.get(&fund)?;
    declared.items.iter().filter(|d| d.pay_date == Some(t.trade_date)).find_map(|d| d.record_date)
}
