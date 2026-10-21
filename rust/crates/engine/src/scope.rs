//! What one filter set produces (`SPEC.md` §4 and §5): the tiles, tables and
//! cards over the trades, positions, payments and accounts in scope. Every sum
//! is exact and in CAD; a member whose figure is not stated is left out of the
//! sum and counted, so each tile says how many it left out.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::account::{AccountKind, AccountStatus, AccountType};
use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::ToSpan;
use bagholder_core::journal::{Grade, JournalEntry};
use bagholder_core::{AccountId, Currency, Dec, InstrumentId, Money};

use crate::cashflow::{CashRow, PayerRate, Payment};
use crate::equity::AccountEquity;
use crate::fx::{live_rate, live_to_cad};
use crate::gap::{Fig, Gap, Gaps};
use crate::input::Inputs;
use crate::ledger::Direction;
use crate::positions::PositionFig;
use crate::stat::returns::{self, Annualized, Day, Drawdown, YearReturn};
use crate::stat::{count_ratio, money_ratio, Ratio};
use crate::trades::{TradeFig, TradeKey, TradeStatus};

/// Which side of its bound a range keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    Above(Dec),
    Below(Dec),
}

impl Bound {
    fn keeps(self, v: Dec) -> bool {
        match self {
            Bound::Above(b) => v > b,
            Bound::Below(b) => v < b,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Day,
    Week,
    Month,
    Quarter,
    HalfYear,
    YearToDate,
    Year,
    FiveYears,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Dates {
    #[default]
    All,
    Preset(Preset),
    Years(BTreeSet<i16>),
    Range { from: Option<Date>, to: Option<Date> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    Win,
    Loss,
    Breakeven,
}

/// The filters (`SPEC.md` §5), naming Bagholder's own ids; an empty set keeps
/// everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filters {
    pub dates: Dates,
    pub accounts: BTreeSet<AccountId>,
    /// A trade matches when any instrument it held, or that instrument's
    /// underlying, is chosen.
    pub instruments: BTreeSet<InstrumentId>,
    /// Matches any name an instrument has had, or its underlying's.
    pub search: String,
    /// `None` inside the set keeps the ungraded.
    pub grades: BTreeSet<Option<Grade>>,
    /// The empty tag inside the set keeps the untagged.
    pub tags: BTreeSet<String>,
    pub kinds: BTreeSet<InstrumentKind>,
    /// Venue names, `Crypto` for a coin.
    pub venues: BTreeSet<String>,
    pub sides: BTreeSet<Direction>,
    pub outcomes: BTreeSet<Outcome>,
    pub price: Option<Bound>,
    pub hold: Option<Bound>,
    pub pnl: Option<Bound>,
    pub qty: Option<Bound>,
    /// The index the years are measured against.
    pub benchmark: String,
}

impl Filters {
    /// The span a date filter keeps, `None` when the years list or nothing does.
    pub fn bounds(&self, today: Date) -> Option<(Date, Date)> {
        match &self.dates {
            Dates::Range { from, to } => Some((from.unwrap_or(Date::MIN), to.unwrap_or(Date::MAX))),
            Dates::Preset(p) => {
                let days = match p {
                    Preset::Day => 1,
                    Preset::Week => 7,
                    Preset::Month => 30,
                    Preset::Quarter => 90,
                    Preset::HalfYear => 180,
                    Preset::Year => 365,
                    Preset::FiveYears => 1826,
                    Preset::YearToDate => return Date::new(today.year(), 1, 1).ok().map(|d| (d, today)),
                };
                Some((today.checked_sub(days.days()).unwrap_or(Date::MIN), today))
            }
            _ => None,
        }
    }

    /// The last day of dates that end before today: the day a past range's
    /// holdings are as of (`SPEC.md` §5). None while the dates reach today (no
    /// date filter, a preset, a range or a list of years that does).
    pub fn past_end(&self, today: Date) -> Option<Date> {
        let end = match &self.dates {
            Dates::Range { to: Some(to), .. } => *to,
            Dates::Years(ys) => Date::new(*ys.iter().max()?, 12, 31).ok()?,
            _ => return None,
        };
        (end < today).then_some(end)
    }

    /// Whether any day from `from` to `to` is in the dates chosen.
    pub fn overlaps(&self, today: Date, from: Date, to: Date) -> bool {
        if let Some((lo, hi)) = self.bounds(today) {
            return from <= hi && lo <= to;
        }
        match &self.dates {
            Dates::Years(ys) if !ys.is_empty() => ys.iter().any(|y| from.year() <= *y && *y <= to.year()),
            _ => true,
        }
    }

    pub fn in_dates(&self, today: Date, day: Date) -> bool {
        if let Some((lo, hi)) = self.bounds(today) {
            return lo <= day && day <= hi;
        }
        match &self.dates {
            Dates::Years(ys) if !ys.is_empty() => ys.contains(&day.year()),
            _ => true,
        }
    }
}

/// An instrument's underlying where it has one, else itself.
pub fn underlying_of(inputs: &Inputs, i: InstrumentId) -> InstrumentId {
    inputs.ledger.instruments.get(&i).and_then(|x| x.terms.as_ref()).map(|t| t.underlying).unwrap_or(i)
}

fn venue_of(inputs: &Inputs, i: InstrumentId) -> Option<String> {
    let info = inputs.ledger.instruments.get(&i)?;
    if info.instrument.kind == InstrumentKind::Crypto {
        return Some("Crypto".into());
    }
    info.current_name().and_then(|n| n.venue_name.clone())
}

fn searched(inputs: &Inputs, instruments: &[InstrumentId], needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let needle = needle.to_uppercase();
    instruments.iter().flat_map(|i| [*i, underlying_of(inputs, *i)]).any(|i| {
        inputs.ledger.instruments.get(&i).is_some_and(|info| {
            info.names.iter().any(|n| n.symbol.to_uppercase().contains(&needle) || n.name.as_deref().is_some_and(|x| x.to_uppercase().contains(&needle)))
        })
    })
}

fn chosen(f: &Filters, inputs: &Inputs, instruments: &[InstrumentId]) -> bool {
    f.instruments.is_empty() || instruments.iter().any(|i| f.instruments.contains(i) || f.instruments.contains(&underlying_of(inputs, *i)))
}

/// A closed trade's result, the one definition the Result filter, the
/// Dashboard's statistics and the by-symbol rows all read (`SPEC.md` §6
/// Dashboard): the sign of its P&L in its own currency, the figure its own page
/// shows. An open trade has none, since its closed-trade figures count once it
/// has closed (decision 2026-09-24), and neither has one whose P&L, or the CAD
/// amount its sums take, waits.
pub fn result(t: &TradeFig) -> Option<Outcome> {
    if t.status == TradeStatus::Open || t.pnl_cad.is_err() {
        return None;
    }
    sign_of(t.pnl.as_ref().ok()?)
}

fn sign_of(p: &Money) -> Option<Outcome> {
    Some(if p.amount.is_positive() {
        Outcome::Win
    } else if p.amount.is_negative() {
        Outcome::Loss
    } else {
        Outcome::Breakeven
    })
}

/// Whether a range keeps a value; a value not stated is never kept by one.
fn bound_keeps(b: Option<Bound>, v: Option<Dec>) -> bool {
    match (b, v) {
        (None, _) => true,
        (Some(b), Some(v)) => b.keeps(v),
        (Some(_), None) => false,
    }
}

/// The grade, tag and side filters, over a journal and a direction.
fn journal_matches(f: &Filters, journal: &JournalEntry, direction: Direction) -> bool {
    if !f.grades.is_empty() && !f.grades.contains(&journal.grade) {
        return false;
    }
    if !f.tags.is_empty() {
        let tagged = if journal.tags.is_empty() { f.tags.contains("") } else { journal.tags.iter().any(|x| f.tags.contains(x)) };
        if !tagged {
            return false;
        }
    }
    f.sides.is_empty() || f.sides.contains(&direction)
}

fn reads_journal(f: &Filters) -> bool {
    !f.grades.is_empty() || !f.tags.is_empty() || !f.sides.is_empty()
}

pub fn trade_matches(f: &Filters, inputs: &Inputs, t: &TradeFig) -> bool {
    let today = inputs.clock.today;
    if !f.accounts.is_empty() && !f.accounts.contains(&t.account) {
        return false;
    }
    if !chosen(f, inputs, &t.instruments) || !searched(inputs, &t.instruments, &f.search) {
        return false;
    }
    if !journal_matches(f, &t.journal, t.direction) {
        return false;
    }
    if !f.kinds.is_empty() && !f.kinds.contains(&t.kind) {
        return false;
    }
    if !f.venues.is_empty() && !venue_of(inputs, t.instrument).is_some_and(|v| f.venues.contains(&v)) {
        return false;
    }
    if !f.outcomes.is_empty() && !result(t).is_some_and(|o| f.outcomes.contains(&o)) {
        return false;
    }
    if !bound_keeps(f.price, t.entry.as_ref().ok().copied())
        || !bound_keeps(f.hold, Some(Dec::from_int(t.hold_days)))
        || !bound_keeps(f.pnl, t.pnl_cad.as_ref().ok().map(|m| m.amount))
        || !bound_keeps(f.qty, t.qty.as_ref().ok().copied())
    {
        return false;
    }
    // in scope when it was open at any time in the dates chosen
    f.overlaps(today, t.opened_on, t.closed_on.unwrap_or(today))
}

/// A closed trade whose close falls in the dates chosen: what the closed-trade
/// statistics count.
fn closed_in(f: &Filters, today: Date, t: &TradeFig) -> bool {
    t.closed_on.is_some_and(|d| f.in_dates(today, d))
}

/// The filters a holding's account and instrument answer.
fn holding_instrument_matches(f: &Filters, inputs: &Inputs, p: &PositionFig) -> bool {
    (f.accounts.is_empty() || f.accounts.contains(&p.account))
        && chosen(f, inputs, &[p.instrument])
        && searched(inputs, &[p.instrument], &f.search)
        && (f.kinds.is_empty() || f.kinds.contains(&p.kind))
        && (f.venues.is_empty() || venue_of(inputs, p.instrument).is_some_and(|v| f.venues.contains(&v)))
}

/// Every filter a holding has, each reading the value its own row shows: its
/// direction; the journal it shares with its trade; its result by the sign of
/// its unrealized P&L; Price by its average cost, Hold by its days held, P&L by
/// its unrealized P&L, Qty by its units; and the dates when it was held at some
/// time in them, so a range ending today narrows nothing.
pub fn position_matches(f: &Filters, inputs: &Inputs, p: &PositionFig) -> bool {
    let today = inputs.clock.today;
    holding_instrument_matches(f, inputs, p)
        && journal_matches(f, &p.journal, p.direction)
        && (f.outcomes.is_empty() || p.unrealized.as_ref().ok().and_then(sign_of).is_some_and(|o| f.outcomes.contains(&o)))
        && bound_keeps(f.price, p.avg.as_ref().ok().copied())
        && bound_keeps(f.hold, p.held_days.as_ref().ok().map(|d| Dec::from_int(*d)))
        && bound_keeps(f.pnl, p.unrealized.as_ref().ok().map(|m| m.amount))
        && bound_keeps(f.qty, p.qty.as_ref().ok().copied())
        && f.overlaps(today, p.opened_on, today)
}

/// A sum of the stated members, and how many were left out. The sum is the
/// failure when the stated members cannot be added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partial {
    pub total: Fig<Money>,
    pub left_out: usize,
}

impl Partial {
    fn of<'a>(items: impl IntoIterator<Item = &'a Fig<Money>>) -> Partial {
        let mut stated = Vec::new();
        let mut left_out = 0;
        for i in items {
            match i {
                Ok(m) => stated.push(*m),
                Err(_) => left_out += 1,
            }
        }
        Partial { total: money_sum(stated), left_out }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Kpi {
    /// Every sale's P&L in the dates chosen, open trades' included, whose CAD
    /// P&L is stated.
    pub realized: Fig<Money>,
    /// Sales in the dates chosen whose CAD P&L is not stated.
    pub realized_left_out: usize,
    /// Closed trades whose CAD P&L is stated.
    pub count: usize,
    /// Closed trades in scope whose CAD P&L is not stated (a deposited coin, a
    /// rate waiting, a contract size not stated…).
    pub left_out: usize,
    pub wins: usize,
    pub losses: usize,
    pub breakeven: usize,
    pub win_rate: Option<Ratio>,
    pub gross_win: Fig<Money>,
    pub gross_loss: Fig<Money>,
    /// None with no losses and some wins: infinite.
    pub profit_factor: Fig<Option<Ratio>>,
    pub profit_factor_infinite: bool,
    pub expectancy: Fig<Option<Money>>,
    pub avg_win: Fig<Option<Money>>,
    pub avg_loss: Fig<Option<Money>>,
    pub fees: Fig<Money>,
    pub avg_hold: Option<Ratio>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonthBar {
    pub year: i16,
    pub month: i8,
    pub value: Fig<Money>,
    pub count: usize,
    pub trades: Vec<TradeKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnderlyingRow {
    pub underlying: InstrumentId,
    pub pnl: Fig<Money>,
    pub count: usize,
    pub legs: usize,
    pub win_rate: Option<Ratio>,
    pub avg_hold: Option<Ratio>,
    pub trades: Vec<TradeKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GradeBucket {
    pub grade: Grade,
    pub count: usize,
    pub pnl: Fig<Money>,
    pub trades: Vec<TradeKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    GradeAndThesis,
    Grade,
    Thesis,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Allocation {
    pub position: usize,
    pub value: Money,
    pub share: Fig<Ratio>,
}

/// Where each of net worth's rows of the broker's own accounts is held: the
/// brokers of the accounts behind it, each as the person calls it
/// (`AccountInfo::broker_label`), once, in the accounts' order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeldAt {
    pub holdings: Vec<String>,
    pub invested_cash: Vec<String>,
    pub chequing_cash: Vec<String>,
    pub margin: Vec<String>,
}

impl HeldAt {
    fn note(list: &mut Vec<String>, label: &str) {
        if !list.iter().any(|l| l == label) {
            list.push(label.to_string());
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Portfolio {
    pub positions: Vec<usize>,
    pub market_value: Partial,
    pub cost_basis: Partial,
    pub unrealized: Partial,
    pub unrealized_pct: Fig<Option<Ratio>>,
    pub account_count: usize,
    /// Σ the broker's stated net value of the open accounts in scope.
    pub net_value: Option<Fig<Money>>,
    pub net_value_accounts: usize,
    /// The negative cash balances the broker states, per currency, shown positive.
    pub margin_used_by: BTreeMap<Currency, Fig<Dec>>,
    /// What is borrowed, as the broker states it now; none for a past day, which
    /// no broker statement describes.
    pub margin_used: Option<Fig<Money>>,
    /// The interest a month of today's borrowing costs at each margin account's
    /// rate (`month_interest`), in CAD; none for a past day.
    pub margin_interest_month: Option<Fig<Money>>,
    pub margin_used_pct: Fig<Option<Ratio>>,
    pub available_margin: Option<Fig<Money>>,
    /// Margin accounts whose buying power the broker could not state, and why.
    pub margin_unavailable: Vec<(AccountId, String)>,
    pub has_margin: bool,
    /// What is owed outside the accounts, in CAD, with each debt and its CAD
    /// value: the outside debts the broker's net worth feature states, under
    /// every account only (an outside debt belongs to none, brief 67), and what
    /// each account in scope other than a margin account owes in its cash
    /// (`owed_of`). None where no debt exists.
    pub owed: Option<(Fig<Money>, Vec<(crate::input::Debt, Fig<Money>)>)>,
    /// What is held outside the broker that is not a debt, each in CAD, under
    /// every account only: net worth's other assets.
    pub outside_assets: Vec<(crate::input::Debt, Fig<Money>)>,
    /// Net worth by day in scope, every day from the first anything in it is
    /// stated to today, whose point is `net_worth_now` (`net_worth_days`); none
    /// under a holding's filter. Part of the figures, so a new read of what it is
    /// worked out from moves the portfolio.
    pub net_worth_history: Vec<NetWorthDay>,
    /// Net worth's totals as its tiles show them: now, from the figures the
    /// Portfolio shows (the holdings, the cash, the margin, what is owed and held
    /// outside); under a range ending before today, its last day's.
    pub net_worth_now: Option<NetWorthDay>,
    pub cash: Fig<Money>,
    /// The cash in the investment accounts in scope, each currency's positive
    /// balance in CAD: what waits to be invested, beside the holdings. Not a
    /// chequing account's, which is no investment, nor a card's.
    pub invested_cash: Fig<Money>,
    /// The cash in the chequing (spending) accounts in scope.
    pub chequing_cash: Fig<Money>,
    /// Where the holdings, the cash, the chequing cash and the margin are held.
    pub held_at: HeldAt,
    /// The allocation's cash slice: the invested cash and its share of the
    /// holdings and it, where there is any and the scope is not a holding's.
    pub allocation_cash: Option<(Money, Fig<Ratio>)>,
    pub cash_pct: Fig<Option<Ratio>>,
    pub day_change: Option<Partial>,
    pub day_change_pct: Fig<Option<Ratio>>,
    pub allocation: Vec<Allocation>,
    /// The past day the holdings are as of, with them (`Engine::holdings`); none
    /// for the holdings now, which are the engine's own positions.
    pub past: Option<std::sync::Arc<crate::engine::Past>>,
}

impl Portfolio {
    /// The positions `positions` and `allocation` index: the past day's, or
    /// those now (`now`, the engine's).
    pub fn held<'a>(&'a self, now: &'a [PositionFig]) -> &'a [PositionFig] {
        match &self.past {
            Some(p) => &p.positions,
            None => now,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CashTile {
    Paid { label: PaidLabel, total: Partial, count: usize, per_paying_month: Fig<Option<Money>> },
    Margin { margin_used: Fig<Money>, interest_per_month: Fig<Option<Money>> },
    Yield { yield_on_cost: Fig<Option<Ratio>>, projected_per_month: Fig<Money>, earned: Fig<Money>, book: Fig<Money>, left_out: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaidLabel {
    Year(i16),
    YearToDate(i16),
    AllTime,
    LastTwelveMonths,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CashMonth {
    pub year: i16,
    pub month: i8,
    pub distributions: Partial,
    pub count: usize,
    /// The month's interest charges, shown positive.
    pub interest: Partial,
    /// The month's repayments of debts held outside the broker, shown positive.
    pub repayments: Partial,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IncomeHolding {
    pub position: usize,
    pub rate: PayerRate,
    pub ytd: Partial,
    pub trailing_year: Partial,
    pub all_time: Partial,
    /// Per unit × payments per year × units, in the currency paid.
    pub annual: Fig<Money>,
    /// That, a month, in CAD at the live rate.
    pub projected_per_month_cad: Fig<Money>,
    pub yield_on_cost: Fig<Ratio>,
    pub current_yield: Fig<Ratio>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cashflow {
    pub tiles: Vec<CashTile>,
    pub months: Vec<CashMonth>,
    pub holdings: Vec<IncomeHolding>,
    /// Dividends in scope, newest first (indexes into the cashflow rows).
    pub rows: Vec<usize>,
    /// Interest, withholding tax and interest charges in scope.
    pub other: Vec<usize>,
    pub total: Partial,
    pub interest: Partial,
    pub withholding: Partial,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EquityBlock {
    pub series: Vec<Day>,
    /// What the accounts' series in scope wait on.
    pub gaps: Gaps,
    pub years: Vec<YearReturn>,
    pub annualized: Annualized,
    pub drawdown: Drawdown,
}

/// The realized P&L in scope, as a running total by the day each part was
/// realized, in CAD: each sale in the dates of a trade the filters keep.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PnlCurve {
    /// Each day a part was realized, and the total to the end of it.
    pub days: Vec<(Date, Fig<Money>)>,
    /// Parts whose P&L waits on something, left out of every total.
    pub left_out: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scoped {
    pub trades: Vec<TradeKey>,
    pub kpi: Kpi,
    pub monthly: Vec<MonthBar>,
    pub by_underlying: Vec<UnderlyingRow>,
    pub grades: Vec<GradeBucket>,
    pub ungraded: usize,
    pub queue: Vec<(TradeKey, Missing)>,
    pub portfolio: Portfolio,
    pub cashflow: Cashflow,
    pub equity: EquityBlock,
    pub pnl_curve: PnlCurve,
}

/// Σ amounts in CAD: the failure when they cannot be added (a total too large
/// to hold), never a total that leaves a term out.
fn money_sum(items: impl IntoIterator<Item = Money>) -> Fig<Money> {
    Ok(items.into_iter().try_fold(Money::zero(Currency::CAD), Money::add_to_fit)?)
}

/// Σ figures in CAD: the first failure among them, or of adding them.
fn fig_sum(items: &[Fig<Money>]) -> Fig<Money> {
    let mut total = Money::zero(Currency::CAD);
    for m in items {
        total = total.add_to_fit(m.clone()?)?;
    }
    Ok(total)
}

/// The mean of `n` amounts totalling `total`; none of none.
/// A month's interest on `principal` at a yearly `rate` (a fraction), for the
/// month `day` is in, as Wealthsimple's web app works it out: compounded daily
/// over the month's days, the principal × ((1 + rate ÷ the year's days) ^ the
/// month's days − 1), to the cent.
pub fn month_interest(principal: Dec, rate: Dec, day: Date) -> Fig<Dec> {
    let daily = rate.div_rounded(Dec::from_int(i64::from(day.days_in_year())), 12, bagholder_core::Rounding::HalfEven)?;
    let step = Dec::from_int(1).checked_add(daily)?;
    let mut factor = Dec::from_int(1);
    for _ in 0..day.days_in_month() {
        factor = factor.mul_to_fit(step)?;
    }
    Ok(principal.mul_to_fit(factor.checked_sub(Dec::from_int(1))?)?.round(2, bagholder_core::Rounding::HalfEven))
}

fn avg_money(total: &Fig<Money>, n: usize) -> Fig<Option<Money>> {
    let total = total.clone()?;
    if n == 0 {
        return Ok(None);
    }
    let v = total.amount.div_rounded(Dec::from_int(n as i64), crate::trades::PRICE_PLACES, bagholder_core::Rounding::HalfEven)?;
    Ok(Some(Money::new(v, total.currency)))
}

/// `part ÷ whole` where both are stated; none where the whole is zero.
fn ratio_of(part: &Fig<Money>, whole: &Fig<Money>) -> Fig<Option<Ratio>> {
    crate::gap::both(part.clone(), whole.clone(), |p, w| Ok(money_ratio(p, w)))
}

/// The closed-trade statistics over `closed`, and the realized P&L over `parts`.
fn kpi(closed: &[&TradeFig], parts: &[&Fig<Money>]) -> Kpi {
    let trades = closed;
    // each with a result, and the CAD amount its sums take as it stands: a win
    // the currency turned negative in CAD stays negative in the gross wins
    let stated: Vec<(&TradeFig, Money, Outcome)> = trades.iter().filter_map(|t| Some((*t, *t.pnl_cad.as_ref().ok()?, result(t)?))).collect();
    let wins: Vec<Money> = stated.iter().filter(|(_, _, o)| *o == Outcome::Win).map(|(_, p, _)| *p).collect();
    let losses: Vec<Money> = stated.iter().filter(|(_, _, o)| *o == Outcome::Loss).map(|(_, p, _)| *p).collect();
    let gross_win = money_sum(wins.iter().copied());
    let gross_loss = money_sum(losses.iter().copied()).map(Money::neg);
    let closed_total = money_sum(stated.iter().map(|(_, p, _)| *p));
    let realized = money_sum(parts.iter().filter_map(|p| p.as_ref().ok().copied()));
    let realized_left_out = parts.iter().filter(|p| p.is_err()).count();
    let n = stated.len();
    let fees = money_sum(stated.iter().filter_map(|(t, _, _)| t.fees_cad.as_ref().ok().copied()));
    let profit_factor = crate::gap::both(gross_win.clone(), gross_loss.clone(), |w, l| {
        Ok(if !l.amount.is_zero() {
            money_ratio(w, l)
        } else if w.amount.is_positive() {
            None
        } else {
            Some(0.0)
        })
    });
    Kpi {
        count: n,
        left_out: trades.len() - n,
        wins: wins.len(),
        losses: losses.len(),
        breakeven: n - wins.len() - losses.len(),
        win_rate: count_ratio(wins.len(), n),
        profit_factor_infinite: matches!((&gross_win, &gross_loss), (Ok(w), Ok(l)) if l.amount.is_zero() && w.amount.is_positive()),
        profit_factor,
        expectancy: avg_money(&closed_total, n),
        avg_win: avg_money(&gross_win, wins.len()),
        avg_loss: avg_money(&gross_loss.clone().map(Money::neg), losses.len()),
        realized,
        realized_left_out,
        gross_win,
        gross_loss,
        fees,
        avg_hold: count_ratio(stated.iter().map(|(t, _, _)| t.hold_days.max(0) as usize).sum(), n),
    }
}

/// The dashboard's part of a filter set: everything that follows from the
/// trades in scope and the accounts' values.
#[derive(Clone, Debug, PartialEq)]
pub struct Dashboard {
    pub kpi: Kpi,
    pub monthly: Vec<MonthBar>,
    pub by_underlying: Vec<UnderlyingRow>,
    pub grades: Vec<GradeBucket>,
    pub ungraded: usize,
    pub queue: Vec<(TradeKey, Missing)>,
    pub equity: EquityBlock,
    pub pnl_curve: PnlCurve,
}

/// The trades a filter set shows, in the engine's order.
pub fn trades_in_scope(f: &Filters, inputs: &Inputs, trades: &[TradeFig]) -> Vec<TradeKey> {
    trades.iter().filter(|t| trade_matches(f, inputs, t)).map(|t| t.key.clone()).collect()
}

/// The dashboard under a filter set.
pub fn dashboard(f: &Filters, inputs: &Inputs, trades: &[TradeFig], equity: &BTreeMap<AccountId, AccountEquity>, benchmarks: &BTreeMap<String, crate::stat::benchmark::Levels>) -> Dashboard {
    let today = inputs.clock.today;
    let in_scope: Vec<&TradeFig> = trades.iter().filter(|t| trade_matches(f, inputs, t)).collect();
    // closed trades whose close is in the dates: what the statistics count
    let closed: Vec<&TradeFig> = in_scope.iter().copied().filter(|t| closed_in(f, today, t)).collect();
    let stated: Vec<&&TradeFig> = closed.iter().filter(|t| result(t).is_some()).collect();
    // each sale in the dates, of an open or a closed trade, on its own day
    let parts: Vec<(&TradeFig, &crate::trades::Realized)> = in_scope.iter().flat_map(|t| t.realized.iter().filter(|r| f.in_dates(today, r.day)).map(move |r| (*t, r))).collect();

    // monthly P&L by the month each sale was realized
    let mut months: BTreeMap<(i16, i8), (Vec<Money>, Vec<TradeKey>)> = BTreeMap::new();
    for (t, r) in &parts {
        let Ok(p) = &r.pnl_cad else { continue };
        let bar = months.entry((r.day.year(), r.day.month())).or_default();
        bar.0.push(*p);
        if !bar.1.contains(&t.key) {
            bar.1.push(t.key.clone());
        }
    }
    let monthly: Vec<MonthBar> = months.into_iter().map(|((year, month), (pnls, trades))| MonthBar { year, month, count: trades.len(), value: money_sum(pnls), trades }).collect();

    // by underlying, largest gain first: the P&L realized, the closed trades' counts
    let mut by: BTreeMap<InstrumentId, (Vec<Money>, usize, i64, usize, Vec<TradeKey>, usize)> = BTreeMap::new();
    for (t, r) in &parts {
        if let Ok(p) = &r.pnl_cad {
            by.entry(underlying_of(inputs, t.instrument)).or_default().0.push(*p);
        }
    }
    for t in &stated {
        let e = by.entry(underlying_of(inputs, t.instrument)).or_default();
        e.1 += usize::from(result(t) == Some(Outcome::Win));
        e.2 += t.hold_days;
        e.3 += t.slices.len();
        e.4.push(t.key.clone());
        e.5 += 1;
    }
    let mut by_underlying: Vec<UnderlyingRow> = by
        .into_iter()
        .map(|(u, (pnls, wins, hold, legs, keys, n))| UnderlyingRow { underlying: u, pnl: money_sum(pnls), count: n, legs, win_rate: count_ratio(wins, n), avg_hold: count_ratio(hold.max(0) as usize, n), trades: keys })
        .collect();
    // largest gain first; one whose total cannot be stated last
    by_underlying.sort_by(|a, b| match (&a.pnl, &b.pnl) {
        (Ok(x), Ok(y)) => y.amount.cmp(&x.amount),
        (a, b) => a.is_err().cmp(&b.is_err()),
    });

    let grades = [Grade::A, Grade::B, Grade::C, Grade::F]
        .into_iter()
        .map(|g| {
            let rows: Vec<&&&TradeFig> = stated.iter().filter(|t| t.journal.grade == Some(g)).collect();
            GradeBucket { grade: g, count: rows.len(), pnl: money_sum(rows.iter().map(|t| *t.pnl_cad.as_ref().expect("stated"))), trades: rows.iter().map(|t| t.key.clone()).collect() }
        })
        .collect();
    let ungraded = closed.iter().filter(|t| t.journal.grade.is_none()).count();
    let mut queue: Vec<(&TradeFig, Missing)> = closed
        .iter()
        .filter_map(|t| {
            let m = match (t.journal.grade.is_none(), t.journal.thesis.trim().is_empty()) {
                (true, true) => Missing::GradeAndThesis,
                (true, false) => Missing::Grade,
                (false, true) => Missing::Thesis,
                (false, false) => return None,
            };
            Some((*t, m))
        })
        .collect();
    queue.sort_by(|a, b| b.0.closed_on.cmp(&a.0.closed_on));

    let equity = equity_block(f, equity, benchmarks, today);
    let pnl_curve = pnl_curve(&parts.iter().map(|(_, r)| *r).collect::<Vec<_>>());
    Dashboard {
        pnl_curve,
        kpi: kpi(&closed, &parts.iter().map(|(_, r)| &r.pnl_cad).collect::<Vec<_>>()),
        monthly,
        by_underlying,
        grades,
        ungraded,
        queue: queue.into_iter().map(|(t, m)| (t.key.clone(), m)).collect(),
        equity,
    }
}

/// The holdings' part of a filter set.
pub fn portfolio_in_scope(f: &Filters, inputs: &Inputs, positions: &[PositionFig], past_days: PastDays) -> Portfolio {
    portfolio(f, inputs, positions, past_days)
}

/// The cashflow's part of a filter set, over the holdings' part.
#[allow(clippy::too_many_arguments)]
pub fn cashflow_in_scope(f: &Filters, inputs: &Inputs, trades: &[TradeFig], positions: &[PositionFig], cash_rows: &[CashRow], rates: &BTreeMap<InstrumentId, PayerRate>, portfolio: &Portfolio) -> Cashflow {
    cashflow(f, inputs, trades, positions, cash_rows, rates, portfolio)
}

/// Everything one filter set produces.
#[allow(clippy::too_many_arguments)]
pub fn scope(
    f: &Filters,
    inputs: &Inputs,
    trades: &[TradeFig],
    positions: &[PositionFig],
    cash_rows: &[CashRow],
    rates: &BTreeMap<InstrumentId, PayerRate>,
    equity: &BTreeMap<AccountId, AccountEquity>,
    benchmarks: &BTreeMap<String, crate::stat::benchmark::Levels>,
    past_days: PastDays,
) -> Scoped {
    let d = dashboard(f, inputs, trades, equity, benchmarks);
    let portfolio = portfolio(f, inputs, positions, past_days);
    let cashflow = cashflow(f, inputs, trades, positions, cash_rows, rates, &portfolio);
    Scoped {
        trades: trades_in_scope(f, inputs, trades),
        kpi: d.kpi,
        monthly: d.monthly,
        by_underlying: d.by_underlying,
        grades: d.grades,
        ungraded: d.ungraded,
        queue: d.queue,
        pnl_curve: d.pnl_curve,
        equity: d.equity,
        portfolio,
        cashflow,
    }
}

/// Each account's value now as its broker states it, in CAD at the live rate.
fn current_navs(inputs: &Inputs, accounts: &[&crate::input::AccountInfo]) -> Vec<Fig<Money>> {
    accounts.iter().filter_map(|a| inputs.market.brokers.get(&a.account.id)?.net_value_now).map(|n| live_to_cad(&inputs.facts.rates, &inputs.clock, n)).collect()
}

/// The net liquidation value of every open account now, in CAD: what an order's
/// position size is over (`SPEC.md` order-ticket-66), the same sum as the
/// portfolio's net value with no filter, waiting on the first account it
/// cannot put in CAD. None where no account states a value.
pub fn current_net_value(inputs: &Inputs) -> Option<Fig<Money>> {
    let navs = current_navs(inputs, &open_accounts_in_scope(&Filters::default(), inputs));
    (!navs.is_empty()).then(|| fig_sum(&navs))
}

fn open_accounts_in_scope<'a>(f: &Filters, inputs: &'a Inputs) -> Vec<&'a crate::input::AccountInfo> {
    inputs.ledger.accounts.values().filter(|a| a.account.status == AccountStatus::Open).filter(|a| f.accounts.is_empty() || f.accounts.contains(&a.account.id)).collect()
}

/// One day's net worth: everything held and everything owed at its close, in
/// CAD, each stated or waiting on what it cannot be stated without.
#[derive(Clone, Debug, PartialEq)]
pub struct NetWorthDay {
    pub day: Date,
    /// The broker's accounts at their value before what they owe (their value
    /// after it and what they owe), and, under every account, what is held
    /// outside them.
    pub assets: Fig<Money>,
    /// What each account owes in its cash (`owed_of`), the margin borrowed
    /// among it, and, under every account, what is owed outside.
    pub debts: Fig<Money>,
}

impl NetWorthDay {
    /// Everything held less everything owed.
    pub fn net(&self) -> Fig<Money> {
        crate::gap::both(self.assets.clone(), self.debts.clone(), |a, d| Ok(a.add_to_fit(d.neg())?))
    }
}

/// Whether the filters narrow to holdings (an instrument, a tag, a grade): a
/// holding has no debt and no net worth by day of its own.
pub fn holdings_filtered(f: &Filters) -> bool {
    !f.instruments.is_empty() || reads_journal(f)
}

/// `add` added to `total`, every gap of both kept.
fn add_fig(total: &mut Fig<Money>, add: &Fig<Money>) {
    let t = std::mem::replace(total, Ok(Money::zero(Currency::CAD)));
    *total = crate::gap::both(t, add.clone(), |t, a| Ok(t.add_to_fit(a)?));
}

/// One thing's statements by day, each held from its day until the next: what
/// it comes to on a day is its latest statement on or before it, and nothing
/// before its first; `until`, its last day counted where it stops (a closed
/// account), else every day after its last.
struct Held {
    stated: Vec<(Date, Fig<Money>)>,
    until: Option<Date>,
    next: usize,
}

impl Held {
    fn new(stated: Vec<(Date, Fig<Money>)>, until: Option<Date>) -> Held {
        Held { stated, until, next: 0 }
    }

    fn first(&self) -> Option<Date> {
        self.stated.first().map(|(d, _)| *d)
    }

    /// What it comes to on `day`; days are asked oldest first.
    fn on(&mut self, day: Date) -> Option<&Fig<Money>> {
        while self.next < self.stated.len() && self.stated[self.next].0 <= day {
            self.next += 1;
        }
        if self.next == 0 || self.until.is_some_and(|u| day > u) {
            return None;
        }
        Some(&self.stated[self.next - 1].1)
    }
}

/// An account's stated values by day, read in order as the days reach them: what
/// it comes to on a day is its latest statement on or before it, in CAD at the
/// rate of the day it was stated for, and nothing before its first; `until`, its
/// last day counted where it stops (a closed account).
struct Stated<'a> {
    first: Option<Date>,
    days: std::iter::Peekable<std::collections::btree_map::Iter<'a, Date, Money>>,
    held: Option<Fig<Money>>,
    until: Option<Date>,
}

impl Stated<'_> {
    /// What it comes to on `day`; days are asked oldest first.
    fn on(&mut self, day: Date, rates: &crate::input::Rates, clock: &crate::input::Clock) -> Option<&Fig<Money>> {
        while let Some((d, m)) = self.days.next_if(|(d, _)| **d <= day) {
            self.held = Some(crate::fx::to_cad(rates, clock, *m, *d));
        }
        if self.until.is_some_and(|u| day > u) {
            return None;
        }
        self.held.as_ref()
    }
}

/// Each account's cash per currency at each day's close, worked back from the
/// cash its broker states now through the transactions after the day: what the
/// broker states now less everything that moved since. A balance the book does
/// not reach back to (an account opened, or a card carried, before the first
/// transaction read) is held from the first day on, so every day is what the
/// account's statements say. An account whose broker states no cash now is
/// summed from nothing, the book being all there is of it. Days are asked
/// oldest first.
pub(crate) struct CashByDay {
    moves: Vec<(Date, AccountId, Money)>,
    next: usize,
    /// Each account's cash per currency, in the order of (account, currency).
    cash: Vec<((AccountId, Currency), Fig<Dec>)>,
}

impl CashByDay {
    /// Over the accounts `accounts` holds, in any order.
    pub(crate) fn new(inputs: &Inputs, accounts: &[AccountId]) -> CashByDay {
        let mut ids = accounts.to_vec();
        ids.sort_unstable();
        let has = |a: &AccountId| ids.binary_search(a).is_ok();
        let mut moves: Vec<(Date, AccountId, Money)> = Vec::with_capacity(inputs.ledger.transactions.len());
        moves.extend(inputs.ledger.transactions.iter().filter(|t| has(&t.account)).filter_map(|t| t.cash.map(|c| (t.trade_date, t.account, c))));
        moves.sort_by_key(|m| m.0);
        let stated = |a: &AccountId| inputs.market.brokers.get(a).filter(|b| b.as_of.is_some());
        // every balance there is: each stated now, and each a move is in
        let mut cash: Vec<((AccountId, Currency), Fig<Dec>)> = Vec::with_capacity(moves.len() + ids.len());
        for a in &ids {
            cash.extend(stated(a).into_iter().flat_map(|b| b.cash.keys()).map(|c| ((*a, *c), Ok(Dec::ZERO))));
        }
        cash.extend(moves.iter().map(|(_, a, m)| ((*a, m.currency), Ok(Dec::ZERO))));
        cash.sort_unstable_by_key(|(k, _)| *k);
        cash.dedup_by_key(|(k, _)| *k);
        // what each account held before its first transaction: the stated cash now
        // less every move, so that the moves bring it to the stated cash
        for ((a, c), v) in cash.iter_mut() {
            if let Some(b) = stated(a) {
                *v = Ok(b.cash.get(c).copied().unwrap_or(Dec::ZERO));
            }
        }
        for (_, a, m) in &moves {
            if stated(a).is_some() {
                let at = cash.binary_search_by_key(&(*a, m.currency), |(k, _)| *k).expect("every move's balance");
                if let Ok(v) = &cash[at].1 {
                    cash[at].1 = v.checked_sub(m.amount).map_err(Gaps::from);
                }
            }
        }
        CashByDay { moves, next: 0, cash }
    }

    /// Each account's cash per currency at `day`'s close, in the order of
    /// (account, currency).
    pub(crate) fn close(&mut self, day: Date) -> &[((AccountId, Currency), Fig<Dec>)] {
        while self.next < self.moves.len() && self.moves[self.next].0 <= day {
            let (_, a, m) = self.moves[self.next];
            let at = self.cash.binary_search_by_key(&(a, m.currency), |(k, _)| *k).expect("every move's balance");
            if let Ok(v) = &self.cash[at].1 {
                self.cash[at].1 = v.checked_add(m.amount).map_err(Gaps::from);
            }
            self.next += 1;
        }
        &self.cash
    }
}

/// What an account owes in one currency's cash balance: a card's or a line of
/// credit's balance, signed (a balance in credit owes less than nothing); any
/// other account's cash below zero, the margin borrowed in a margin account.
fn owed_of(kind: Option<AccountKind>, cash: Dec) -> Dec {
    match kind {
        Some(AccountKind::CreditCard) | Some(AccountKind::LineOfCredit) => cash.neg(),
        _ if cash.is_negative() => cash.neg(),
        _ => Dec::ZERO,
    }
}

fn kind_of(a: &crate::input::AccountInfo) -> Option<AccountKind> {
    match &a.account.account_type {
        AccountType::Known { kind, .. } => Some(*kind),
        _ => None,
    }
}

/// Net worth by day, a point for every day from the first anything in scope is
/// stated to today (`docs/plans/debts-held-outside-wealthsimple.md`). Each of the
/// broker's accounts counts its latest stated value on or before the day, at the
/// rate of the day it was stated for, from its first stated day, and a closed
/// account until its last; an open account whose value is never stated counts
/// its cash. What each owes at the day's close (`owed_of`, from `CashByDay`, at
/// the day's rate) is a debt, and added back to what the account holds: the same
/// parts, by the same rule, as today's point (`portfolio`), so a day on which
/// nothing moves ends where the next begins. Under every account, each thing held
/// or owed outside counts its latest statement on or before the day
/// (`Debts::history`, and its balance now from the day it was stated). Today is
/// `now`: the figures the tiles show. A day with a part that cannot be stated (a
/// rate not published) is kept, waiting on it.
pub fn net_worth_days(f: &Filters, inputs: &Inputs, now: &NetWorthDay) -> Vec<NetWorthDay> {
    let mut out = net_worth_past(&f.accounts, inputs);
    out.push(now.clone());
    out
}

/// Net worth by day over the accounts `in_scope` (every account where empty), each day
/// before today (`net_worth_days`). Nothing a quote or a close moves is among
/// what it is worked out from, so it is kept from one price change to the next
/// (`Engine::net_worth_past`).
pub fn net_worth_past(in_scope: &BTreeSet<AccountId>, inputs: &Inputs) -> Vec<NetWorthDay> {
    let debts = &inputs.facts.debts;
    let whole = in_scope.is_empty();
    let (rates, clock) = (&inputs.facts.rates, &inputs.clock);
    let today = clock.today;
    // the accounts the broker states, in the order of their ids, each with its
    // stated values, read where it stands, each in CAD at its own day's rate
    // as the days reach it; one whose value is never stated counts while it is open
    struct Account<'a> {
        id: AccountId,
        kind: Option<AccountKind>,
        values: Option<Stated<'a>>,
    }
    let mut accounts: Vec<Account> = Vec::with_capacity(inputs.ledger.accounts.len());
    accounts.extend(inputs
        .ledger
        .accounts
        .values()
        .filter(|a| whole || in_scope.contains(&a.account.id))
        .filter_map(|a| {
            let v = &inputs.market.brokers.get(&a.account.id)?.net_value;
            let values = match v.keys().next_back() {
                Some(last) => Some(Stated { first: v.keys().next().copied(), days: v.iter().peekable(), held: None, until: (a.account.status != AccountStatus::Open).then_some(*last) }),
                None if a.account.status == AccountStatus::Open => None,
                None => return None,
            };
            Some(Account { id: a.account.id, kind: kind_of(a), values })
        }));
    // each thing held or owed outside: its statements by day, then its balance
    // now from the day that balance was stated, where that is later
    let outside = |list: &[crate::input::Debt]| -> Vec<Held> {
        list.iter()
            .map(|d| {
                let mut stated: Vec<(Date, Fig<Money>)> = debts.history.get(&d.id).into_iter().flatten().map(|(day, m)| (*day, crate::fx::to_cad(rates, clock, *m, *day))).collect();
                let as_of = d.as_of.map(|t| t.to_zoned(clock.home.clone()).date()).unwrap_or(today).min(today);
                if stated.last().is_none_or(|(l, _)| *l < as_of) {
                    stated.push((as_of, crate::fx::to_cad(rates, clock, d.balance, as_of)));
                }
                // stated at nothing before it was first held: not yet held
                let begins = stated.iter().position(|(_, m)| !matches!(m, Ok(m) if m.amount.is_zero())).unwrap_or(stated.len());
                Held::new(stated.split_off(begins), None)
            })
            .collect()
    };
    let (mut held_out, mut owed_out) = if whole { (outside(&debts.assets), outside(&debts.owed)) } else { (Vec::new(), Vec::new()) };
    let first = accounts.iter().filter_map(|a| a.values.as_ref()?.first).chain(held_out.iter().chain(&owed_out).filter_map(Held::first)).min();
    let mut out = Vec::new();
    if let Some(first) = first.filter(|d| *d < today) {
        let ids: Vec<AccountId> = accounts.iter().map(|a| a.id).collect();
        let mut cash = CashByDay::new(inputs, &ids);
        // each account's debt and, never stated, what it holds, by its place in `accounts`
        let zero = || -> Fig<Money> { Ok(Money::zero(Currency::CAD)) };
        let mut parts: Vec<(Fig<Money>, Fig<Money>)> = accounts.iter().map(|_| (zero(), zero())).collect();
        let mut day = first;
        while day < today {
            for p in parts.iter_mut() {
                *p = (zero(), zero());
            }
            // each balance of the day's close to its account: the balances are in
            // the order of their accounts' ids, as `accounts` is
            let mut at = 0;
            for ((id, c), v) in cash.close(day) {
                while at < accounts.len() && accounts[at].id < *id {
                    at += 1;
                }
                let Some(a) = accounts.get(at).filter(|a| a.id == *id) else { continue };
                let (debt, held) = &mut parts[at];
                let in_cad = |d: Dec| crate::fx::to_cad(rates, clock, Money::new(d, *c), day);
                match v {
                    Ok(v) => {
                        let o = owed_of(a.kind, *v);
                        if !o.is_zero() {
                            add_fig(debt, &in_cad(o));
                        }
                        if a.values.is_none() && !v.is_zero() {
                            add_fig(held, &in_cad(*v));
                        }
                    }
                    Err(g) => {
                        add_fig(debt, &Err(g.clone()));
                        add_fig(held, &Err(g.clone()));
                    }
                }
            }
            let (mut assets, mut owed) = (zero(), zero());
            for (a, (debt, held)) in accounts.iter_mut().zip(parts.iter_mut()) {
                if let Some(s) = a.values.as_mut() {
                    match s.on(day, rates, clock) {
                        Some(v) => add_fig(held, v),
                        // before its first stated day, or closed: nothing of it
                        None => continue,
                    }
                }
                add_fig(held, debt);
                add_fig(&mut assets, held);
                add_fig(&mut owed, debt);
            }
            for h in held_out.iter_mut() {
                if let Some(v) = h.on(day) {
                    add_fig(&mut assets, v);
                }
            }
            for h in owed_out.iter_mut() {
                if let Some(v) = h.on(day) {
                    add_fig(&mut owed, v);
                }
            }
            out.push(NetWorthDay { day, assets, debts: owed });
            day = match day.tomorrow() {
                Ok(d) => d,
                Err(_) => break,
            };
        }
    }
    out
}

pub(crate) fn is_margin(a: &crate::input::AccountInfo) -> bool {
    matches!(a.account.account_type, AccountType::Known { kind: AccountKind::Margin, .. })
}

/// What is owed outside the accounts now (`Portfolio::owed`).
fn owed(f: &Filters, inputs: &Inputs, accounts: &[&crate::input::AccountInfo]) -> Option<(Fig<Money>, Vec<(crate::input::Debt, Fig<Money>)>)> {
    // a holding owes nothing; what is owed outside is no account's, so under
    // every account only; a card's balance with its account
    if holdings_filtered(f) {
        return None;
    }
    let (rates, clock) = (&inputs.facts.rates, &inputs.clock);
    let mut debts: Vec<crate::input::Debt> = if f.accounts.is_empty() { inputs.facts.debts.owed.clone() } else { Vec::new() };
    // what each account owes in its cash (`owed_of`): a card's or a line of
    // credit's balance, any other account's cash below zero; a margin account's is
    // the margin, a debt of its own
    for a in accounts {
        let Some(b) = inputs.market.brokers.get(&a.account.id) else { continue };
        if is_margin(a) {
            continue;
        }
        let kind = kind_of(a);
        let named = match kind {
            Some(AccountKind::CreditCard) => "CreditCard",
            Some(AccountKind::LineOfCredit) => "LineOfCredit",
            _ => "",
        };
        for (c, v) in &b.cash {
            let o = owed_of(kind, *v);
            if !o.is_zero() {
                debts.push(crate::input::Debt { id: format!("{}:{}", a.account.id, c), kind: named.into(), name: a.account.name(), institution: a.broker_label.clone(), balance: Money::new(o, *c), as_of: b.as_of });
            }
        }
    }
    if debts.is_empty() {
        return None;
    }
    let each: Vec<(crate::input::Debt, Fig<Money>)> = debts.into_iter().map(|d| {
        // as stated: owed positive, a debt in credit negative, which lowers what is owed
        let cad = live_to_cad(rates, clock, d.balance);
        (d, cad)
    }).collect();
    Some((fig_sum(&each.iter().map(|(_, m)| m.clone()).collect::<Vec<_>>()), each))
}

/// What the days before today come to (`net_worth_past`) under an account set,
/// asked only where the scope has net worth by day.
pub type PastDays<'a> = &'a dyn Fn(&BTreeSet<AccountId>) -> std::sync::Arc<Vec<NetWorthDay>>;

fn portfolio(f: &Filters, inputs: &Inputs, positions: &[PositionFig], past_days: PastDays) -> Portfolio {
    let rates = &inputs.facts.rates;
    let clock = &inputs.clock;
    let idx: Vec<usize> = positions.iter().enumerate().filter(|(_, p)| position_matches(f, inputs, p)).map(|(i, _)| i).collect();
    let signed_market = |p: &PositionFig| -> Fig<Money> { p.market_cad.clone().map(|m| if p.direction == Direction::Short { m.neg() } else { m }) };
    let market: Vec<Fig<Money>> = idx.iter().map(|i| signed_market(&positions[*i])).collect();
    let cost: Vec<Fig<Money>> = idx.iter().map(|i| positions[*i].book_cad.clone().map(|m| m.abs())).collect();
    let unreal: Vec<Fig<Money>> = idx.iter().map(|i| positions[*i].unrealized_cad.clone()).collect();
    let market_value = Partial::of(&market);
    let cost_basis = Partial::of(&cost);
    let unrealized = Partial::of(&unreal);

    let accounts = open_accounts_in_scope(f, inputs);
    // an account figure is never divided by a filtered sum: margin used is over
    // the market value of every holding in the accounts in scope
    let accounts_market = Partial::of(&positions.iter().filter(|p| f.accounts.is_empty() || f.accounts.contains(&p.account)).map(signed_market).collect::<Vec<_>>());
    let navs = current_navs(inputs, &accounts);
    let mut used: BTreeMap<Currency, Fig<Dec>> = BTreeMap::new();
    let mut interest: BTreeMap<Currency, Fig<Dec>> = BTreeMap::new();
    let mut cash_by: BTreeMap<Currency, Fig<Dec>> = BTreeMap::new();
    let mut invested_by: BTreeMap<Currency, Fig<Dec>> = BTreeMap::new();
    let mut chequing_by: BTreeMap<Currency, Fig<Dec>> = BTreeMap::new();
    let mut available = Vec::new();
    let mut unavailable = Vec::new();
    let mut unread = Vec::new();
    let mut held_at = HeldAt::default();
    for i in &idx {
        if let Some(a) = inputs.ledger.accounts.get(&positions[*i].account) {
            HeldAt::note(&mut held_at.holdings, &a.broker_label);
        }
    }
    for a in &accounts {
        let Some(b) = inputs.market.brokers.get(&a.account.id) else { continue };
        for (c, v) in &b.cash {
            if v.is_negative() && is_margin(a) {
                HeldAt::note(&mut held_at.margin, &a.broker_label);
            }
            // money borrowed is a margin account's negative cash; another
            // account's (a credit card's balance owed) is a debt, not margin
            let slot = match (v.is_negative(), is_margin(a)) {
                (true, true) => used.entry(*c).or_insert(Ok(Dec::ZERO)),
                (true, false) => continue,
                (false, _) => cash_by.entry(*c).or_insert(Ok(Dec::ZERO)),
            };
            if let Ok(total) = slot {
                *slot = total.checked_add(v.abs()).map_err(Gaps::from);
            }
            // a month of what it borrows, at its own rate in that currency
            if v.is_negative() && is_margin(a) {
                let month = match b.margin_rates.get(c) {
                    Some(rate) => month_interest(v.abs(), *rate, inputs.clock.today),
                    None => Err(Gaps::of(Gap::MarginRateUnread { account: a.account.id, currency: *c })),
                };
                let e = interest.entry(*c).or_insert(Ok(Dec::ZERO));
                *e = match (e.clone(), month) {
                    (Ok(t), Ok(m)) => t.checked_add(m).map_err(Gaps::from),
                    (Err(mut g), Err(h)) => {
                        g.merge(&h);
                        Err(g)
                    }
                    (Err(g), _) | (_, Err(g)) => Err(g),
                };
            }
            // a positive balance: a chequing account's is cash to spend, an
            // investment account's waits to be invested, a card's is neither
            if v.is_positive() {
                let kind = match &a.account.account_type {
                    AccountType::Known { kind, .. } => Some(*kind),
                    _ => None,
                };
                let to = match kind {
                    Some(AccountKind::Spending) => Some((&mut chequing_by, &mut held_at.chequing_cash)),
                    Some(AccountKind::CreditCard) | Some(AccountKind::LineOfCredit) => None,
                    _ => Some((&mut invested_by, &mut held_at.invested_cash)),
                };
                let to = to.map(|(to, at)| {
                    HeldAt::note(at, &a.broker_label);
                    to
                });
                if let Some(to) = to {
                    let e = to.entry(*c).or_insert(Ok(Dec::ZERO));
                    if let Ok(total) = e {
                        *e = total.checked_add(*v).map_err(Gaps::from);
                    }
                }
            }
        }
        if is_margin(a) {
            match &b.buying_power {
                Some(Ok(p)) => available.push(live_to_cad(rates, clock, *p)),
                Some(Err(why)) => unavailable.push((a.account.id, why.clone())),
                None => unread.push(a.account.id),
            }
        }
    }
    // a margin account in scope makes the figure one to state: it waits on any
    // whose buying power is not read yet
    for a in &accounts {
        if is_margin(a) && !inputs.market.brokers.contains_key(&a.account.id) {
            unread.push(a.account.id);
        }
    }
    let to_cad_sum = |by: &BTreeMap<Currency, Fig<Dec>>| -> Fig<Money> {
        let mut total = Money::zero(Currency::CAD);
        for (c, v) in by {
            total = total.add_to_fit(live_to_cad(rates, clock, Money::new(v.clone()?, *c))?)?;
        }
        Ok(total)
    };
    let margin_used = to_cad_sum(&used);
    let cash = to_cad_sum(&cash_by);
    let invested_cash = to_cad_sum(&invested_by);
    let chequing_cash = to_cad_sum(&chequing_by);
    let net_value = (!navs.is_empty()).then(|| fig_sum(&navs));
    let quoted: Vec<(usize, Money)> = idx.iter().filter_map(|i| positions[*i].day_change_cad.as_ref().ok().and_then(|d| d.map(|m| (*i, m)))).collect();
    let day_change = (!quoted.is_empty()).then(|| Partial { total: money_sum(quoted.iter().map(|(_, m)| *m)), left_out: idx.len() - quoted.len() });
    let day_change_pct: Fig<Option<Ratio>> = match &day_change {
        None => Ok(None),
        Some(dc) => {
            let now = money_sum(quoted.iter().filter_map(|(i, _)| signed_market(&positions[*i]).ok()));
            // a sum made to fit (`money_sum`) can carry the full 28 digits, so the
            // difference is made to fit too
            let before = crate::gap::both(now, dc.total.clone(), |n, d| Ok(n.add_to_fit(d.neg())?));
            ratio_of(&dc.total, &before)
        }
    };
    let mut allocation: Vec<Allocation> = idx
        .iter()
        .filter_map(|i| positions[*i].market_cad.as_ref().ok().filter(|m| m.amount.is_positive()).map(|m| Allocation { position: *i, value: *m, share: Ok(0.0) }))
        .collect();
    allocation.sort_by(|a, b| b.value.amount.cmp(&a.value.amount));
    // the cash waiting to be invested is part of what the accounts hold: a slice
    // of its own, unless the scope is a holding's
    let allocation_cash = match &invested_cash {
        Ok(c) if c.amount.is_positive() && !holdings_filtered(f) => Some(*c),
        _ => None,
    };
    let allocated = money_sum(allocation.iter().map(|a| a.value).chain(allocation_cash));
    for a in allocation.iter_mut() {
        // every value is positive, so their sum is never zero
        a.share = allocated.clone().map(|t| money_ratio(a.value, t).unwrap_or(0.0));
    }
    let allocation_cash = allocation_cash.map(|c| (c, allocated.clone().map(|t| money_ratio(c, t).unwrap_or(0.0))));
    let account_count = idx.iter().map(|i| positions[*i].account).collect::<BTreeSet<_>>().len();
    let owed = owed(f, inputs, &accounts);
    let outside_assets = if f.accounts.is_empty() && !holdings_filtered(f) { inputs.facts.debts.assets.iter().map(|d| (d.clone(), live_to_cad(&inputs.facts.rates, &inputs.clock, d.balance))).collect() } else { Vec::new() };
    // net worth now, from the figures the tiles show: today's point
    let now = {
        let mut assets: Fig<Money> = Ok(Money::zero(Currency::CAD));
        if !idx.is_empty() {
            add_fig(&mut assets, &market_value.total);
        }
        add_fig(&mut assets, &invested_cash);
        add_fig(&mut assets, &chequing_cash);
        for (_, v) in &outside_assets {
            add_fig(&mut assets, v);
        }
        let mut debts = margin_used.clone();
        if let Some((total, _)) = &owed {
            add_fig(&mut debts, total);
        }
        NetWorthDay { day: inputs.clock.today, assets, debts }
    };
    let net_worth_history = if holdings_filtered(f) { Vec::new() } else { past_days(&f.accounts).iter().chain(std::iter::once(&now)).filter(|n| f.in_dates(inputs.clock.today, n.day)).cloned().collect() };
    Portfolio {
        owed,
        outside_assets,
        net_worth_history,
        net_worth_now: Some(now),
        unrealized_pct: ratio_of(&unrealized.total, &cost_basis.total),
        margin_used_pct: ratio_of(&margin_used, &accounts_market.total),
        cash_pct: match &net_value {
            Some(n) => ratio_of(&cash, n),
            None => Ok(None),
        },
        positions: idx,
        market_value,
        cost_basis,
        unrealized,
        account_count,
        net_value_accounts: navs.len(),
        net_value,
        margin_used_by: used,
        margin_used: Some(margin_used),
        margin_interest_month: Some(to_cad_sum(&interest)),
        available_margin: match (unread.first(), available.is_empty()) {
            (Some(a), _) => Some(Err(Gaps::of(Gap::BuyingPowerUnread(*a)))),
            (None, false) => Some(fig_sum(&available)),
            (None, true) => None,
        },
        margin_unavailable: unavailable,
        has_margin: accounts.iter().any(|a| is_margin(a)),
        cash,
        day_change,
        day_change_pct,
        allocation,
        invested_cash,
        chequing_cash,
        held_at,
        allocation_cash,
        past: None,
    }
}

/// The Portfolio as of the close of a past day (`SPEC.md` §5): the positions
/// then, at that day's close and rate; what the accounts were worth that day,
/// as the broker stated it; the cash that day (`CashByDay`); nothing borrowed or
/// available to borrow, which the broker states only now.
pub fn portfolio_as_of(f: &Filters, inputs: &Inputs, past: std::sync::Arc<crate::engine::Past>, past_days: PastDays) -> Portfolio {
    let day = past.day;
    let mut p = portfolio(f, inputs, &past.positions, past_days);
    let accounts: Vec<AccountId> = inputs.ledger.accounts.keys().copied().filter(|a| f.accounts.is_empty() || f.accounts.contains(a)).collect();
    // each account's value that day: its last stated on or before it
    // each at the rate of the day it is that account's value for
    let navs: Vec<Fig<Money>> = accounts.iter().filter_map(|a| inputs.market.brokers.get(a)?.net_value.range(..=day).next_back().map(|(d, v)| crate::fx::to_cad(&inputs.facts.rates, &inputs.clock, *v, *d))).collect();
    p.net_value_accounts = navs.len();
    p.net_value = (!navs.is_empty()).then(|| fig_sum(&navs));
    // the cash that day, worked back from what the broker states now
    // (`CashByDay`), each currency's positive balance at that day's rate
    let by = CashByDay::new(inputs, &accounts).close(day).to_vec();
    let mut cash = Ok(Money::zero(Currency::CAD));
    let (mut invested, mut chequing) = (Ok(Money::zero(Currency::CAD)), Ok(Money::zero(Currency::CAD)));
    for ((a, c), v) in by {
        let kind = match inputs.ledger.accounts.get(&a).map(|i| &i.account.account_type) {
            Some(AccountType::Known { kind, .. }) => Some(*kind),
            _ => None,
        };
        let add = |total: Money, v: Dec| -> Fig<Money> {
            if !v.is_positive() {
                return Ok(total);
            }
            Ok(total.add_to_fit(crate::fx::to_cad(&inputs.facts.rates, &inputs.clock, Money::new(v, c), day)?)?)
        };
        cash = crate::gap::both(cash, v.clone(), add);
        // as now: a chequing account's cash is to spend, an investment account's waits to be invested
        match kind {
            Some(AccountKind::Spending) => chequing = crate::gap::both(chequing, v, add),
            Some(AccountKind::CreditCard) | Some(AccountKind::LineOfCredit) => {}
            _ => invested = crate::gap::both(invested, v, add),
        }
    }
    // the allocation's shares again, over that day's holdings and the cash waiting then
    let slice = match &invested {
        Ok(c) if c.amount.is_positive() && !holdings_filtered(f) => Some(*c),
        _ => None,
    };
    let allocated = money_sum(p.allocation.iter().map(|a| a.value).chain(slice));
    for a in p.allocation.iter_mut() {
        a.share = allocated.clone().map(|t| money_ratio(a.value, t).unwrap_or(0.0));
    }
    p.allocation_cash = slice.map(|c| (c, allocated.clone().map(|t| money_ratio(c, t).unwrap_or(0.0))));
    p.invested_cash = invested;
    p.chequing_cash = chequing;
    p.cash_pct = match &p.net_value {
        Some(n) => ratio_of(&cash, n),
        None => Ok(None),
    };
    p.cash = cash;
    p.margin_used_by = BTreeMap::new();
    p.margin_used = None;
    p.margin_interest_month = None;
    p.margin_used_pct = Ok(None);
    p.available_margin = None;
    // no statement describes a past day's debts
    p.owed = None;
    p.outside_assets = Vec::new();
    // net worth's totals: the range's last day's
    p.net_worth_now = p.net_worth_history.last().cloned();
    p.margin_unavailable = Vec::new();
    p.past = Some(past);
    p
}

#[allow(clippy::too_many_arguments)]
fn cashflow(f: &Filters, inputs: &Inputs, trades: &[TradeFig], positions: &[PositionFig], rows: &[CashRow], rates: &BTreeMap<InstrumentId, PayerRate>, portfolio: &Portfolio) -> Cashflow {
    let today = inputs.clock.today;
    let live = |m: Money| live_to_cad(&inputs.facts.rates, &inputs.clock, m);
    let in_accounts = |a: &AccountId| f.accounts.is_empty() || f.accounts.contains(a);
    // the symbol, search, kind and exchange filters, over a payment's instrument;
    // a payment on none (interest on cash) answers none of them
    let in_instruments = |i: Option<InstrumentId>| match i {
        Some(i) => {
            chosen(f, inputs, &[i])
                && searched(inputs, &[i], &f.search)
                && (f.kinds.is_empty() || inputs.ledger.instruments.get(&i).is_some_and(|x| f.kinds.contains(&x.instrument.kind)))
                && (f.venues.is_empty() || venue_of(inputs, i).is_some_and(|v| f.venues.contains(&v)))
        }
        None => f.instruments.is_empty() && f.search.is_empty() && f.kinds.is_empty() && f.venues.is_empty(),
    };
    // the grade, tag and side filters, over the round trip entitled to the
    // payment: the one open at the close of the session before the ex-date of
    // the declared distribution it is for (holding at the record date, which the
    // ex-date equals under T+1). A trip that opened before the ex-date and had
    // not closed before it held at that close: fills fall in sessions. A payment
    // no declared distribution matches is never placed by its pay date: it
    // answers no such filter.
    let entitled = |r: &CashRow| -> bool {
        if !reads_journal(f) {
            return true;
        }
        let (Some(i), Some(ex)) = (r.instrument, crate::cashflow::entitled_ex(inputs, r)) else { return false };
        trades.iter().any(|t| t.account == r.account && t.instruments.contains(&i) && t.opened_on < ex && t.closed_on.is_none_or(|c| c >= ex) && journal_matches(f, &t.journal, t.direction))
    };
    // a repayment of a debt held outside is the household's, whichever account the
    // money left: what the income in scope is set against, under any account filter
    let everything: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| (in_accounts(&r.account) || r.kind == Payment::Repayment) && in_instruments(r.instrument) && f.in_dates(today, r.day) && entitled(r)).map(|(i, _)| i).collect();
    let dividends: Vec<usize> = everything.iter().copied().filter(|i| rows[*i].kind == Payment::Dividend).collect();
    let other: Vec<usize> = everything.iter().copied().filter(|i| rows[*i].kind != Payment::Dividend).collect();
    let partial = |ix: &mut dyn Iterator<Item = usize>| -> Partial {
        let figs: Vec<Fig<Money>> = ix.map(|i| rows[i].amount_cad.clone()).collect();
        Partial::of(&figs)
    };

    // the chart: every month from the first payment to the current month (or the date filter's end)
    let mut months: Vec<CashMonth> = Vec::new();
    // the bars' own payments: distributions, interest charged and repayments
    let charted = |i: &usize| matches!(rows[*i].kind, Payment::Dividend | Payment::InterestCharge | Payment::Repayment);
    if let Some(first) = everything.iter().filter(|i| charted(i)).map(|i| rows[*i].day).min() {
        let end = match f.bounds(today) {
            Some((_, hi)) => hi.min(today),
            None => match &f.dates {
                Dates::Years(ys) if !ys.is_empty() => Date::new(*ys.iter().max().expect("nonempty"), 12, 31).map(|d| d.min(today)).unwrap_or(today),
                _ => today,
            },
        };
        let last = everything.iter().filter(|i| charted(i)).map(|i| rows[*i].day).max().unwrap_or(first).max(end);
        let (mut y, mut m) = (first.year(), first.month());
        while (y, m) <= (last.year(), last.month()) {
            let in_month = |i: &usize| rows[*i].day.year() == y && rows[*i].day.month() == m;
            let paid: Vec<usize> = dividends.iter().copied().filter(in_month).collect();
            let charges: Vec<usize> = other.iter().copied().filter(|i| rows[*i].kind == Payment::InterestCharge && in_month(i)).collect();
            let mut interest = partial(&mut charges.iter().copied());
            interest.total = interest.total.map(Money::neg);
            let repaid: Vec<usize> = other.iter().copied().filter(|i| rows[*i].kind == Payment::Repayment && in_month(i)).collect();
            let mut repayments = partial(&mut repaid.iter().copied());
            repayments.total = repayments.total.map(Money::neg);
            months.push(CashMonth { year: y, month: m, count: paid.len(), distributions: partial(&mut paid.iter().copied()), interest, repayments });
            if m == 12 {
                y += 1;
                m = 1;
            } else {
                m += 1;
            }
        }
    }

    let paying_months = |keep: &dyn Fn(&CashMonth) -> bool| months.iter().filter(|m| keep(m) && m.count > 0).count();
    let paid_tile = |label: PaidLabel, ix: Vec<usize>, months_paid: usize| {
        let total = partial(&mut ix.iter().copied());
        CashTile::Paid { per_paying_month: avg_money(&total.total, months_paid), total, count: ix.len(), label }
    };
    let this_year = today.year();
    let mut tiles = Vec::new();
    for y in [this_year - 2, this_year - 1, this_year] {
        let ix: Vec<usize> = dividends.iter().copied().filter(|i| rows[*i].day.year() == y).collect();
        let label = if y == this_year { PaidLabel::YearToDate(y) } else { PaidLabel::Year(y) };
        tiles.push(paid_tile(label, ix, paying_months(&|m| m.year == y)));
    }
    tiles.push(paid_tile(PaidLabel::AllTime, dividends.clone(), paying_months(&|_| true)));
    if portfolio.has_margin {
        // a month of today's borrowing at each account's rate; nothing where
        // nothing is borrowed, or for a past day
        let interest_per_month = match &portfolio.margin_interest_month {
            Some(Ok(m)) if m.amount.is_zero() => Ok(None),
            Some(m) => m.clone().map(Some),
            None => Ok(None),
        };
        tiles.push(CashTile::Margin { margin_used: portfolio.margin_used.clone().unwrap_or(Ok(Money::zero(Currency::CAD))), interest_per_month });
    } else {
        let since = today.checked_sub(365.days()).unwrap_or(Date::MIN);
        let ix: Vec<usize> = dividends.iter().copied().filter(|i| rows[*i].day > since && rows[*i].day <= today).collect();
        let n = ix.iter().map(|i| (rows[*i].day.year(), rows[*i].day.month())).collect::<BTreeSet<_>>().len();
        tiles.push(paid_tile(PaidLabel::LastTwelveMonths, ix, n));
    }

    // the income holdings: each open long position in a dividend-paying
    // instrument, one that has paid the book a dividend in any account, or whose
    // declared record lists a distribution of an amount above zero. A holding with a record
    // that lists none (a listing that has never paid) is not one.
    let mut payers: BTreeSet<InstrumentId> = rows.iter().filter(|r| r.kind == Payment::Dividend).filter_map(|r| r.instrument).collect();
    payers.extend(inputs.facts.declared.iter().filter(|(_, r)| r.items.iter().any(|d| d.amount.amount.is_positive())).map(|(i, _)| *i));
    let for_yoc: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.kind == Payment::Dividend && in_accounts(&r.account)).map(|(i, _)| i).collect();
    let trailing_from = today.checked_sub(365.days()).unwrap_or(Date::MIN);
    let holdings: Vec<IncomeHolding> = positions
        .iter()
        .enumerate()
        .filter(|(_, p)| p.direction == Direction::Long && payers.contains(&p.instrument) && in_accounts(&p.account) && in_instruments(Some(p.instrument)) && journal_matches(f, &p.journal, p.direction))
        .filter_map(|(pi, p)| rates.get(&p.instrument).map(|r| (pi, p, r.clone())))
        .map(|(pi, p, rate)| {
            // what the symbol paid: its instrument, into every account in scope,
            // whichever holds it now (SPEC.md §5, Cashflow Positions)
            let paid = |keep: &dyn Fn(&CashRow) -> bool| partial(&mut for_yoc.iter().copied().filter(|i| rows[*i].instrument == Some(p.instrument) && keep(&rows[*i])));
            let annual: Fig<Money> = crate::gap::both(rate.annual_per_unit(), p.qty.clone(), |a, q| Ok(a.times(q)?));
            // a payout in another currency than the holding's is taken at the
            // latest rate, as any live figure is
            let per_unit_annual = rate.annual_per_unit().and_then(|a| in_currency_live(inputs, a, p.currency));
            let over = |a: Money, of: Dec, what: &str| money_ratio(a, Money::new(of, p.currency)).ok_or_else(|| Gaps::of(Gap::Arithmetic(format!("{} has no {what} to be a yield of", p.instrument))));
            let yoc = crate::gap::both(per_unit_annual.clone(), p.avg.clone(), |a, avg| over(a, avg, "cost"));
            let cy = crate::gap::both(per_unit_annual, p.mark.clone(), |a, m| over(a, m.price, "price"));
            IncomeHolding {
                position: pi,
                ytd: paid(&|r| r.day.year() == this_year),
                trailing_year: paid(&|r| r.day > trailing_from),
                all_time: paid(&|_| true),
                projected_per_month_cad: annual.clone().and_then(live).and_then(|m| Ok(Money::new(m.amount.div_rounded(Dec::from_int(12), crate::trades::PRICE_PLACES, bagholder_core::Rounding::HalfEven)?, Currency::CAD))),
                annual,
                yield_on_cost: yoc,
                current_yield: cy,
                rate,
            }
        })
        .collect();
    let rated: Vec<&IncomeHolding> = holdings.iter().filter(|h| h.projected_per_month_cad.is_ok()).collect();
    let book = money_sum(rated.iter().filter_map(|h| positions[h.position].book_cad.as_ref().ok().copied()));
    let per_month = money_sum(rated.iter().filter_map(|h| h.projected_per_month_cad.as_ref().ok().copied()));
    // what the rated symbols paid in the trailing year, each symbol once however
    // many accounts hold it
    let rated_instruments: BTreeSet<InstrumentId> = rated.iter().map(|h| h.rate.instrument).collect();
    let earned: Fig<Money> = partial(&mut for_yoc.iter().copied().filter(|i| rows[*i].day > trailing_from && rows[*i].instrument.is_some_and(|x| rated_instruments.contains(&x)))).total;
    let annual_total: Fig<Money> = per_month.clone().and_then(|m| Ok(m.times(Dec::from_int(12))?));
    tiles.push(CashTile::Yield { yield_on_cost: ratio_of(&annual_total, &book), projected_per_month: per_month, earned, book, left_out: holdings.len() - rated.len() });

    Cashflow {
        total: partial(&mut dividends.iter().copied()),
        interest: partial(&mut other.iter().copied().filter(|i| rows[*i].kind == Payment::Interest)),
        withholding: partial(&mut other.iter().copied().filter(|i| rows[*i].kind == Payment::WithholdingTax)),
        tiles,
        months,
        holdings,
        rows: dividends,
        other,
    }
}

/// `amount` in `currency` at the latest rates the Bank has published.
fn in_currency_live(inputs: &Inputs, amount: Money, currency: Currency) -> Fig<Money> {
    if amount.currency == currency {
        return Ok(amount);
    }
    let cad = live_to_cad(&inputs.facts.rates, &inputs.clock, amount)?;
    let (per_unit, _) = live_rate(&inputs.facts.rates, &inputs.clock, currency)?;
    Ok(Money::new(cad.amount.div_rounded(per_unit, crate::trades::PRICE_PLACES, bagholder_core::Rounding::HalfEven)?, currency))
}

/// The equity series of the accounts in scope over the dates chosen, and what it
/// says. A day is in the series when every account in scope that has begun has
/// a value that day; its return is the value-weighted return of the accounts
/// that formed one. The series is built over the whole history (a return needs
/// the day before, and the pre-history floor is the whole history's), then the
/// curve, the drawdown and the years are taken over the dates chosen, a year cut
/// by a range over its days inside it.
fn equity_block(f: &Filters, equity: &BTreeMap<AccountId, AccountEquity>, benchmarks: &BTreeMap<String, crate::stat::benchmark::Levels>, today: Date) -> EquityBlock {
    let accounts: Vec<&AccountEquity> = equity.values().filter(|e| f.accounts.is_empty() || f.accounts.contains(&e.account)).collect();
    let points: Vec<&[crate::equity::DayValue]> = accounts.iter().map(|e| e.points.as_slice()).collect();
    let whole = returns::combine(&points);
    let floor = returns::floor(&whole);
    let benchmark = benchmarks.get(&f.benchmark);
    let years = returns::yearly_returns(&whole, today, benchmark, floor, |y| year_span(f, today, y));
    let series: Vec<Day> = whole.into_iter().filter(|d| f.in_dates(today, d.day)).collect();
    let mut gaps = Gaps::none();
    for e in &accounts {
        gaps.merge(&e.gaps);
    }
    EquityBlock { annualized: returns::annualized(&years), drawdown: returns::drawdown(&series, floor), years, series, gaps }
}

/// The calendar days of year `y` the dates chosen keep, first and last; none
/// when they keep none of it.
fn year_span(f: &Filters, today: Date, y: i16) -> Option<(Date, Date)> {
    let (jan1, dec31) = (Date::new(y, 1, 1).ok()?, Date::new(y, 12, 31).ok()?);
    if let Some((lo, hi)) = f.bounds(today) {
        let (first, last) = (jan1.max(lo), dec31.min(hi));
        return (first <= last).then_some((first, last));
    }
    match &f.dates {
        Dates::Years(ys) if !ys.is_empty() => ys.contains(&y).then_some((jan1, dec31)),
        _ => Some((jan1, dec31)),
    }
}

/// The running total of the realized parts by their days.
fn pnl_curve(parts: &[&crate::trades::Realized]) -> PnlCurve {
    let mut by_day: BTreeMap<Date, Vec<Money>> = BTreeMap::new();
    let mut left_out = 0;
    for r in parts {
        match &r.pnl_cad {
            Ok(p) => by_day.entry(r.day).or_default().push(*p),
            Err(_) => left_out += 1,
        }
    }
    let mut running: Fig<Money> = Ok(Money::zero(Currency::CAD));
    let days = by_day
        .into_iter()
        .map(|(day, pnls)| {
            running = running.clone().and_then(|r| money_sum(std::iter::once(r).chain(pnls)));
            (day, running.clone())
        })
        .collect();
    PnlCurve { days, left_out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cad(s: &str) -> Money {
        Money::new(Dec::parse(s).unwrap(), Currency::CAD)
    }

    const HUGE: &str = "70000000000000000000000000000";

    #[test]
    fn the_pnl_curve_runs_each_realized_part_on_its_own_day_and_leaves_out_what_waits() {
        let d = |s: &str| -> Date { s.parse().unwrap() };
        let part = |day: &str, p: Fig<Money>| crate::trades::Realized { day: d(day), pnl_cad: p };
        let waits = || Err(Gaps::of(crate::gap::Gap::Arithmetic("waits".into())));
        let parts = [part("2025-01-03", Ok(cad("10"))), part("2025-01-02", Ok(cad("-4"))), part("2025-01-03", Ok(cad("1.5"))), part("2025-01-05", waits())];
        let c = pnl_curve(&parts.iter().collect::<Vec<_>>());
        assert_eq!(c.days, vec![(d("2025-01-02"), Ok(cad("-4"))), (d("2025-01-03"), Ok(cad("7.5")))]);
        assert_eq!(c.left_out, 1);
        // a running total too large to hold is a gap from that day on, never a total missing a term
        let parts = [part("2025-01-02", Ok(cad(HUGE))), part("2025-01-03", Ok(cad(HUGE))), part("2025-01-04", Ok(cad("1")))];
        let c = pnl_curve(&parts.iter().collect::<Vec<_>>());
        assert!(c.days[0].1.is_ok() && c.days[1].1.is_err() && c.days[2].1.is_err(), "{:?}", c.days);
    }

    #[test]
    fn a_total_too_large_to_hold_is_a_gap_never_a_total_missing_a_term() {
        assert_eq!(money_sum([cad("1.25"), cad("2.5")]), Ok(cad("3.75")));
        let over = money_sum([cad(HUGE), cad(HUGE), cad("1")]);
        assert!(over.as_ref().is_err_and(|g| g.has_word("arithmetic")), "{over:?}");

        let p = Partial::of(&[Ok(cad(HUGE)), Err(Gaps::of(Gap::Arithmetic("a price".into()))), Ok(cad(HUGE))]);
        assert_eq!(p.left_out, 1);
        assert!(p.total.as_ref().is_err_and(|g| g.has_word("arithmetic")), "{p:?}");

        // what is worked out from such a total carries its gap too
        assert!(avg_money(&p.total, 2).is_err());
        assert!(ratio_of(&Ok(cad("1")), &p.total).is_err());
        assert_eq!(avg_money(&Ok(cad("3")), 0), Ok(None));
        assert_eq!(ratio_of(&Ok(cad("1")), &Ok(cad("4"))), Ok(Some(0.25)));
    }
}
