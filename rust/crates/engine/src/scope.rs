//! What one filter set produces (`SPEC.md` §4 and §5): the tiles, tables and
//! cards over the trades, positions, payments and accounts in scope. Every sum
//! is exact and in CAD; a member whose figure is not stated is left out of the
//! sum and counted, so each tile says how many it left out.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::account::{AccountKind, AccountStatus, AccountType};
use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::ToSpan;
use bagholder_core::journal::Grade;
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
use crate::trades::{TradeFig, TradeKey};

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

    pub fn in_dates(&self, today: Date, day: Date) -> bool {
        if let Some((lo, hi)) = self.bounds(today) {
            return lo <= day && day <= hi;
        }
        match &self.dates {
            Dates::Years(ys) if !ys.is_empty() => ys.contains(&day.year()),
            _ => true,
        }
    }

    /// The filters in force the Cashflow tab does not read.
    pub fn unread_by_cashflow(&self) -> Vec<&'static str> {
        [
            ("grade", !self.grades.is_empty()),
            ("tag", !self.tags.is_empty()),
            ("kind", !self.kinds.is_empty()),
            ("exchange", !self.venues.is_empty()),
            ("side", !self.sides.is_empty()),
            ("result", !self.outcomes.is_empty()),
            ("price", self.price.is_some()),
            ("hold", self.hold.is_some()),
            ("pnl", self.pnl.is_some()),
            ("qty", self.qty.is_some()),
        ]
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(n, _)| n)
        .collect()
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

fn outcome(t: &TradeFig) -> Option<Outcome> {
    let p = t.pnl_cad.as_ref().ok()?;
    Some(if p.amount.is_positive() {
        Outcome::Win
    } else if p.amount.is_negative() {
        Outcome::Loss
    } else {
        Outcome::Breakeven
    })
}

pub fn trade_matches(f: &Filters, inputs: &Inputs, t: &TradeFig) -> bool {
    let today = inputs.clock.today;
    if !f.accounts.is_empty() && !f.accounts.contains(&t.account) {
        return false;
    }
    if !chosen(f, inputs, &t.instruments) || !searched(inputs, &t.instruments, &f.search) {
        return false;
    }
    if !f.grades.is_empty() && !f.grades.contains(&t.journal.grade) {
        return false;
    }
    if !f.tags.is_empty() {
        let tagged = if t.journal.tags.is_empty() { f.tags.contains("") } else { t.journal.tags.iter().any(|x| f.tags.contains(x)) };
        if !tagged {
            return false;
        }
    }
    if !f.kinds.is_empty() && !f.kinds.contains(&t.kind) {
        return false;
    }
    if !f.venues.is_empty() && !venue_of(inputs, t.instrument).is_some_and(|v| f.venues.contains(&v)) {
        return false;
    }
    if !f.sides.is_empty() && !f.sides.contains(&t.direction) {
        return false;
    }
    if !f.outcomes.is_empty() && !outcome(t).is_some_and(|o| f.outcomes.contains(&o)) {
        return false;
    }
    let bound = |b: Option<Bound>, v: Option<Dec>| match (b, v) {
        (None, _) => true,
        (Some(b), Some(v)) => b.keeps(v),
        (Some(_), None) => false,
    };
    if !bound(f.price, t.entry.as_ref().ok().copied())
        || !bound(f.hold, Some(Dec::from_int(t.hold_days)))
        || !bound(f.pnl, t.pnl_cad.as_ref().ok().map(|m| m.amount))
        || !bound(f.qty, t.qty.as_ref().ok().copied())
    {
        return false;
    }
    f.in_dates(today, t.closed_on)
}

pub fn position_matches(f: &Filters, inputs: &Inputs, p: &PositionFig) -> bool {
    (f.accounts.is_empty() || f.accounts.contains(&p.account))
        && chosen(f, inputs, &[p.instrument])
        && searched(inputs, &[p.instrument], &f.search)
        && (f.kinds.is_empty() || f.kinds.contains(&p.kind))
        && (f.venues.is_empty() || venue_of(inputs, p.instrument).is_some_and(|v| f.venues.contains(&v)))
}

/// A sum of the stated members, and how many were left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partial {
    pub total: Money,
    pub left_out: usize,
}

impl Partial {
    fn of<'a>(items: impl IntoIterator<Item = &'a Fig<Money>>) -> Result<Partial, Gaps> {
        let mut total = Money::zero(Currency::CAD);
        let mut left_out = 0;
        for i in items {
            match i {
                Ok(m) => total = total.add_to_fit(*m)?,
                Err(_) => left_out += 1,
            }
        }
        Ok(Partial { total, left_out })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Kpi {
    /// Over the trades whose CAD P&L is stated.
    pub realized: Money,
    pub count: usize,
    /// Trades in scope whose CAD P&L is not stated (a deposited coin, a rate
    /// waiting, a contract size not stated…).
    pub left_out: usize,
    pub wins: usize,
    pub losses: usize,
    pub breakeven: usize,
    pub win_rate: Option<Ratio>,
    pub gross_win: Money,
    pub gross_loss: Money,
    /// None with no losses and some wins: infinite.
    pub profit_factor: Option<Ratio>,
    pub profit_factor_infinite: bool,
    pub expectancy: Option<Money>,
    pub avg_win: Option<Money>,
    pub avg_loss: Option<Money>,
    pub fees: Money,
    pub avg_hold: Option<Ratio>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonthBar {
    pub year: i16,
    pub month: i8,
    pub value: Money,
    pub count: usize,
    pub trades: Vec<TradeKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnderlyingRow {
    pub underlying: InstrumentId,
    pub pnl: Money,
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
    pub pnl: Money,
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
    pub share: Ratio,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Portfolio {
    pub positions: Vec<usize>,
    pub market_value: Partial,
    pub cost_basis: Partial,
    pub unrealized: Partial,
    pub unrealized_pct: Option<Ratio>,
    pub account_count: usize,
    /// Σ the broker's stated net value of the open accounts in scope.
    pub net_value: Option<Money>,
    pub net_value_accounts: usize,
    /// The negative cash balances the broker states, per currency, shown positive.
    pub margin_used_by: BTreeMap<Currency, Dec>,
    pub margin_used: Fig<Money>,
    pub margin_used_pct: Option<Ratio>,
    pub available_margin: Option<Money>,
    /// Margin accounts whose buying power the broker could not state, and why.
    pub margin_unavailable: Vec<(AccountId, String)>,
    pub has_margin: bool,
    pub cash: Fig<Money>,
    pub cash_pct: Option<Ratio>,
    pub day_change: Option<Partial>,
    pub day_change_pct: Option<Ratio>,
    pub allocation: Vec<Allocation>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CashTile {
    Paid { label: PaidLabel, total: Partial, count: usize, per_paying_month: Option<Money> },
    Margin { margin_used: Fig<Money>, interest_per_month: Option<Money>, interest_months: usize },
    Yield { yield_on_cost: Option<Ratio>, projected_per_month: Money, earned: Money, book: Money, left_out: usize },
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
    pub unread_filters: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EquityBlock {
    pub series: Vec<Day>,
    pub years: Vec<YearReturn>,
    pub annualized: Annualized,
    pub drawdown: Drawdown,
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
}

fn money_sum(items: impl IntoIterator<Item = Money>) -> Money {
    items.into_iter().fold(Money::zero(Currency::CAD), |a, b| a.add_to_fit(b).unwrap_or(a))
}

fn avg_money(total: Money, n: usize) -> Option<Money> {
    (n > 0).then(|| total.amount.div_rounded(Dec::from_int(n as i64), crate::trades::PRICE_PLACES, bagholder_core::Rounding::HalfEven).ok().map(|v| Money::new(v, total.currency))).flatten()
}

fn kpi(trades: &[&TradeFig]) -> Kpi {
    let stated: Vec<(&TradeFig, Money)> = trades.iter().filter_map(|t| t.pnl_cad.as_ref().ok().map(|p| (*t, *p))).collect();
    let wins: Vec<Money> = stated.iter().map(|(_, p)| *p).filter(|p| p.amount.is_positive()).collect();
    let losses: Vec<Money> = stated.iter().map(|(_, p)| *p).filter(|p| p.amount.is_negative()).collect();
    let gross_win = money_sum(wins.iter().copied());
    let gross_loss = money_sum(losses.iter().copied()).neg();
    let realized = money_sum(stated.iter().map(|(_, p)| *p));
    let n = stated.len();
    let fees = money_sum(stated.iter().filter_map(|(t, _)| t.fees_cad.as_ref().ok().copied()));
    Kpi {
        realized,
        count: n,
        left_out: trades.len() - n,
        wins: wins.len(),
        losses: losses.len(),
        breakeven: n - wins.len() - losses.len(),
        win_rate: count_ratio(wins.len(), n),
        profit_factor: if !gross_loss.amount.is_zero() {
            money_ratio(gross_win, gross_loss)
        } else if gross_win.amount.is_positive() {
            None
        } else {
            Some(0.0)
        },
        profit_factor_infinite: gross_loss.amount.is_zero() && gross_win.amount.is_positive(),
        expectancy: avg_money(realized, n),
        avg_win: avg_money(gross_win, wins.len()),
        avg_loss: avg_money(gross_loss.neg(), losses.len()),
        gross_win,
        gross_loss,
        fees,
        avg_hold: count_ratio(stated.iter().map(|(t, _)| t.hold_days.max(0) as usize).sum(), n),
    }
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
) -> Scoped {
    let today = inputs.clock.today;
    let in_scope: Vec<&TradeFig> = trades.iter().filter(|t| trade_matches(f, inputs, t)).collect();
    let stated: Vec<&&TradeFig> = in_scope.iter().filter(|t| t.pnl_cad.is_ok()).collect();

    // monthly P&L by close month
    let mut months: BTreeMap<(i16, i8), MonthBar> = BTreeMap::new();
    for t in &stated {
        let p = *t.pnl_cad.as_ref().expect("stated");
        let bar = months.entry((t.closed_on.year(), t.closed_on.month())).or_insert(MonthBar { year: t.closed_on.year(), month: t.closed_on.month(), value: Money::zero(Currency::CAD), count: 0, trades: vec![] });
        bar.value = bar.value.add_to_fit(p).unwrap_or(bar.value);
        bar.count += 1;
        bar.trades.push(t.key.clone());
    }

    // by underlying, largest gain first
    let mut by: BTreeMap<InstrumentId, (Money, usize, usize, i64, usize, Vec<TradeKey>)> = BTreeMap::new();
    for t in &stated {
        let p = *t.pnl_cad.as_ref().expect("stated");
        let e = by.entry(underlying_of(inputs, t.instrument)).or_insert((Money::zero(Currency::CAD), 0, 0, 0, 0, vec![]));
        e.0 = e.0.add_to_fit(p).unwrap_or(e.0);
        e.1 += 1;
        e.2 += usize::from(p.amount.is_positive());
        e.3 += t.hold_days;
        e.4 += t.slices.len();
        e.5.push(t.key.clone());
    }
    let mut by_underlying: Vec<UnderlyingRow> = by
        .into_iter()
        .map(|(u, (pnl, n, wins, hold, legs, keys))| UnderlyingRow { underlying: u, pnl, count: n, legs, win_rate: count_ratio(wins, n), avg_hold: count_ratio(hold.max(0) as usize, n), trades: keys })
        .collect();
    by_underlying.sort_by(|a, b| b.pnl.amount.cmp(&a.pnl.amount));

    let grades = [Grade::A, Grade::B, Grade::C, Grade::F]
        .into_iter()
        .map(|g| {
            let rows: Vec<&&&TradeFig> = stated.iter().filter(|t| t.journal.grade == Some(g)).collect();
            GradeBucket { grade: g, count: rows.len(), pnl: money_sum(rows.iter().map(|t| *t.pnl_cad.as_ref().expect("stated"))), trades: rows.iter().map(|t| t.key.clone()).collect() }
        })
        .collect();
    let ungraded = in_scope.iter().filter(|t| t.journal.grade.is_none()).count();
    let mut queue: Vec<(&TradeFig, Missing)> = in_scope
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

    let portfolio = portfolio(f, inputs, positions);
    let cashflow = cashflow(f, inputs, positions, cash_rows, rates, &portfolio);
    let equity = equity_block(f, equity, benchmarks, today);
    Scoped {
        kpi: kpi(&in_scope),
        trades: in_scope.iter().map(|t| t.key.clone()).collect(),
        monthly: months.into_values().collect(),
        by_underlying,
        grades,
        ungraded,
        queue: queue.into_iter().map(|(t, m)| (t.key.clone(), m)).collect(),
        portfolio,
        cashflow,
        equity,
    }
}

fn open_accounts_in_scope<'a>(f: &Filters, inputs: &'a Inputs) -> Vec<&'a crate::input::AccountInfo> {
    inputs.ledger.accounts.values().filter(|a| a.account.status == AccountStatus::Open).filter(|a| f.accounts.is_empty() || f.accounts.contains(&a.account.id)).collect()
}

fn is_margin(a: &crate::input::AccountInfo) -> bool {
    matches!(a.account.account_type, AccountType::Known { kind: AccountKind::Margin, .. })
}

fn portfolio(f: &Filters, inputs: &Inputs, positions: &[PositionFig]) -> Portfolio {
    let rates = &inputs.facts.rates;
    let clock = &inputs.clock;
    let idx: Vec<usize> = positions.iter().enumerate().filter(|(_, p)| position_matches(f, inputs, p)).map(|(i, _)| i).collect();
    let signed_market = |p: &PositionFig| -> Fig<Money> { p.market_cad.clone().map(|m| if p.direction == Direction::Short { m.neg() } else { m }) };
    let market: Vec<Fig<Money>> = idx.iter().map(|i| signed_market(&positions[*i])).collect();
    let cost: Vec<Fig<Money>> = idx.iter().map(|i| positions[*i].book_cad.clone().map(|m| m.abs())).collect();
    let unreal: Vec<Fig<Money>> = idx.iter().map(|i| positions[*i].unrealized_cad.clone()).collect();
    let market_value = Partial::of(&market).unwrap_or(Partial { total: Money::zero(Currency::CAD), left_out: market.len() });
    let cost_basis = Partial::of(&cost).unwrap_or(Partial { total: Money::zero(Currency::CAD), left_out: cost.len() });
    let unrealized = Partial::of(&unreal).unwrap_or(Partial { total: Money::zero(Currency::CAD), left_out: unreal.len() });

    let accounts = open_accounts_in_scope(f, inputs);
    let mut navs = Vec::new();
    let mut used: BTreeMap<Currency, Dec> = BTreeMap::new();
    let mut cash_by: BTreeMap<Currency, Dec> = BTreeMap::new();
    let mut available = Vec::new();
    let mut unavailable = Vec::new();
    for a in &accounts {
        let Some(b) = inputs.market.brokers.get(&a.account.id) else { continue };
        if let Some(n) = b.net_value_now {
            navs.push(Money::new(n, Currency::CAD));
        }
        for (c, v) in &b.cash {
            let slot = if v.is_negative() { used.entry(*c).or_insert(Dec::ZERO) } else { cash_by.entry(*c).or_insert(Dec::ZERO) };
            *slot = slot.add_to_fit(v.abs()).unwrap_or(*slot);
        }
        if is_margin(a) {
            match &b.buying_power {
                Some(Ok(p)) => available.push(Money::new(*p, Currency::CAD)),
                Some(Err(why)) => unavailable.push((a.account.id, why.clone())),
                None => {}
            }
        }
    }
    let to_cad_sum = |by: &BTreeMap<Currency, Dec>| -> Fig<Money> {
        let mut total = Money::zero(Currency::CAD);
        for (c, v) in by {
            total = total.add_to_fit(live_to_cad(rates, clock, Money::new(*v, *c))?)?;
        }
        Ok(total)
    };
    let margin_used = to_cad_sum(&used);
    let cash = to_cad_sum(&cash_by);
    let net_value = (!navs.is_empty()).then(|| money_sum(navs.iter().copied()));
    let quoted: Vec<(usize, Money)> = idx.iter().filter_map(|i| positions[*i].day_change_cad.as_ref().ok().and_then(|d| d.map(|m| (*i, m)))).collect();
    let day_change = (!quoted.is_empty()).then(|| Partial { total: money_sum(quoted.iter().map(|(_, m)| *m)), left_out: idx.len() - quoted.len() });
    let day_change_pct = day_change.as_ref().and_then(|dc| {
        let now = money_sum(quoted.iter().filter_map(|(i, _)| signed_market(&positions[*i]).ok()));
        let before = now.checked_sub(dc.total).ok()?;
        money_ratio(dc.total, before)
    });
    let mut allocation: Vec<Allocation> = idx
        .iter()
        .filter_map(|i| positions[*i].market_cad.as_ref().ok().filter(|m| m.amount.is_positive()).map(|m| Allocation { position: *i, value: *m, share: 0.0 }))
        .collect();
    allocation.sort_by(|a, b| b.value.amount.cmp(&a.value.amount));
    let allocated = money_sum(allocation.iter().map(|a| a.value));
    for a in allocation.iter_mut() {
        a.share = money_ratio(a.value, allocated).unwrap_or(0.0);
    }
    let account_count = idx.iter().map(|i| positions[*i].account).collect::<BTreeSet<_>>().len();
    Portfolio {
        unrealized_pct: money_ratio(unrealized.total, cost_basis.total),
        margin_used_pct: margin_used.as_ref().ok().and_then(|m| money_ratio(*m, market_value.total)),
        cash_pct: match (&cash, &net_value) {
            (Ok(c), Some(n)) => money_ratio(*c, *n),
            _ => None,
        },
        positions: idx,
        market_value,
        cost_basis,
        unrealized,
        account_count,
        net_value_accounts: navs.len(),
        net_value,
        margin_used_by: used,
        margin_used,
        available_margin: (!available.is_empty()).then(|| money_sum(available.iter().copied())),
        margin_unavailable: unavailable,
        has_margin: accounts.iter().any(|a| is_margin(a)),
        cash,
        day_change,
        day_change_pct,
        allocation,
    }
}

fn cashflow(f: &Filters, inputs: &Inputs, positions: &[PositionFig], rows: &[CashRow], rates: &BTreeMap<InstrumentId, PayerRate>, portfolio: &Portfolio) -> Cashflow {
    let today = inputs.clock.today;
    let live = |m: Money| live_to_cad(&inputs.facts.rates, &inputs.clock, m);
    let in_accounts = |a: &AccountId| f.accounts.is_empty() || f.accounts.contains(a);
    let in_instruments = |i: Option<InstrumentId>| match i {
        Some(i) => chosen(f, inputs, &[i]) && searched(inputs, &[i], &f.search),
        None => f.instruments.is_empty() && f.search.is_empty(),
    };
    let everything: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| in_accounts(&r.account) && in_instruments(r.instrument) && f.in_dates(today, r.day)).map(|(i, _)| i).collect();
    let dividends: Vec<usize> = everything.iter().copied().filter(|i| rows[*i].kind == Payment::Dividend).collect();
    let other: Vec<usize> = everything.iter().copied().filter(|i| rows[*i].kind != Payment::Dividend).collect();
    let partial = |ix: &mut dyn Iterator<Item = usize>| -> Partial {
        let figs: Vec<Fig<Money>> = ix.map(|i| rows[i].amount_cad.clone()).collect();
        Partial::of(&figs).unwrap_or(Partial { total: Money::zero(Currency::CAD), left_out: figs.len() })
    };

    // the chart: every month from the first payment to the current month (or the date filter's end)
    let mut months: Vec<CashMonth> = Vec::new();
    if let Some(first) = dividends.iter().map(|i| rows[*i].day).min() {
        let end = match f.bounds(today) {
            Some((_, hi)) => hi.min(today),
            None => match &f.dates {
                Dates::Years(ys) if !ys.is_empty() => Date::new(*ys.iter().max().expect("nonempty"), 12, 31).map(|d| d.min(today)).unwrap_or(today),
                _ => today,
            },
        };
        let last = dividends.iter().map(|i| rows[*i].day).max().unwrap_or(first).max(end);
        let (mut y, mut m) = (first.year(), first.month());
        while (y, m) <= (last.year(), last.month()) {
            let in_month = |i: &usize| rows[*i].day.year() == y && rows[*i].day.month() == m;
            let paid: Vec<usize> = dividends.iter().copied().filter(in_month).collect();
            let charges: Vec<usize> = other.iter().copied().filter(|i| rows[*i].kind == Payment::InterestCharge && in_month(i)).collect();
            let mut interest = partial(&mut charges.iter().copied());
            interest.total = interest.total.neg();
            months.push(CashMonth { year: y, month: m, count: paid.len(), distributions: partial(&mut paid.iter().copied()), interest });
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
        CashTile::Paid { per_paying_month: avg_money(total.total, months_paid), total, count: ix.len(), label }
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
        let charges: Vec<usize> = other.iter().copied().filter(|i| rows[*i].kind == Payment::InterestCharge).collect();
        let charged: BTreeSet<(i16, i8)> = charges.iter().map(|i| (rows[*i].day.year(), rows[*i].day.month())).collect();
        let total = partial(&mut charges.iter().copied()).total.neg();
        tiles.push(CashTile::Margin { margin_used: portfolio.margin_used.clone(), interest_per_month: avg_money(total, charged.len()), interest_months: charged.len() });
    } else {
        let since = today.checked_sub(365.days()).unwrap_or(Date::MIN);
        let ix: Vec<usize> = dividends.iter().copied().filter(|i| rows[*i].day > since && rows[*i].day <= today).collect();
        let n = ix.iter().map(|i| (rows[*i].day.year(), rows[*i].day.month())).collect::<BTreeSet<_>>().len();
        tiles.push(paid_tile(PaidLabel::LastTwelveMonths, ix, n));
    }

    // the income holdings: each open long position in a paying instrument
    let for_yoc: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.kind == Payment::Dividend && in_accounts(&r.account)).map(|(i, _)| i).collect();
    let trailing_from = today.checked_sub(365.days()).unwrap_or(Date::MIN);
    let holdings: Vec<IncomeHolding> = positions
        .iter()
        .enumerate()
        .filter(|(_, p)| p.direction == Direction::Long && in_accounts(&p.account) && in_instruments(Some(p.instrument)))
        .filter_map(|(pi, p)| rates.get(&p.instrument).map(|r| (pi, p, r.clone())))
        .map(|(pi, p, rate)| {
            // what this holding paid: its instrument, into its own account
            let paid = |keep: &dyn Fn(&CashRow) -> bool| partial(&mut for_yoc.iter().copied().filter(|i| rows[*i].instrument == Some(p.instrument) && rows[*i].account == p.account && keep(&rows[*i])));
            let annual: Fig<Money> = rate.annual_per_unit().and_then(|a| Ok(a.times(p.qty)?));
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
    let earned = money_sum(rated.iter().map(|h| h.trailing_year.total));
    let annual_total = per_month.times(Dec::from_int(12)).unwrap_or(per_month);
    tiles.push(CashTile::Yield { yield_on_cost: money_ratio(annual_total, book), projected_per_month: per_month, earned, book, left_out: holdings.len() - rated.len() });

    Cashflow {
        total: partial(&mut dividends.iter().copied()),
        interest: partial(&mut other.iter().copied().filter(|i| rows[*i].kind == Payment::Interest)),
        withholding: partial(&mut other.iter().copied().filter(|i| rows[*i].kind == Payment::WithholdingTax)),
        tiles,
        months,
        holdings,
        rows: dividends,
        other,
        unread_filters: f.unread_by_cashflow(),
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

/// The equity series of the accounts in scope, and what it says. A day is in
/// the series when every account in scope that has begun has a value that day;
/// its return is the value-weighted return of the accounts that formed one.
fn equity_block(f: &Filters, equity: &BTreeMap<AccountId, AccountEquity>, benchmarks: &BTreeMap<String, crate::stat::benchmark::Levels>, today: Date) -> EquityBlock {
    let accounts: Vec<&AccountEquity> = equity.values().filter(|e| f.accounts.is_empty() || f.accounts.contains(&e.account)).collect();
    let mut values: BTreeMap<Date, (Dec, usize, Option<Dec>)> = BTreeMap::new();
    for e in &accounts {
        for p in &e.points {
            let v = values.entry(p.day).or_insert((Dec::ZERO, 0, Some(Dec::ZERO)));
            v.0 = v.0.add_to_fit(p.value).unwrap_or(v.0);
            v.1 += 1;
            v.2 = match (v.2, p.flow) {
                (Some(a), Some(b)) => a.add_to_fit(b).ok(),
                _ => None,
            };
        }
    }
    let begun = |d: Date| accounts.iter().filter(|e| e.points.first().is_some_and(|p| p.day <= d)).count();
    let complete: BTreeMap<Date, (Dec, Option<Dec>)> = values.into_iter().filter(|(d, (_, n, _))| *n == begun(*d)).map(|(d, (v, _, flow))| (d, (v, flow))).collect();
    let per_account: Vec<&[(Date, Ratio, Dec)]> = accounts.iter().map(|e| e.returns.as_slice()).collect();
    let series = returns::combine(&complete, &per_account);
    let benchmark = benchmarks.get(&f.benchmark);
    let years = returns::yearly_returns(&series, today, benchmark);
    EquityBlock { annualized: returns::annualized(&years), drawdown: returns::drawdown(&series), years, series }
}
