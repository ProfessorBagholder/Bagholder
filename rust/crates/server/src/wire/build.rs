//! The figures document built from the engine (`figures::Figures`), for one
//! set of filters.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::instrument::{InstrumentKind, RefScheme};
use bagholder_core::journal::Grade;
use bagholder_core::{AccountId, InstrumentId, Money};
use bagholder_engine::cashflow::{CashRow, Payment};
use bagholder_engine::input::Inputs;
use bagholder_engine::ledger::Direction;
use bagholder_engine::positions::PositionFig;
use bagholder_engine::scope::{self, CashTile, Filters, Missing, PaidLabel, Scoped};
use bagholder_engine::trades::{TradeFig, TradeKey, TradeStatus};
use bagholder_engine::Engine;

use super::figures::*;
use super::{Dec, Fig};

/// What the page calls each instrument and account, read from the book once per
/// record change (`Names::load`): the engine carries no broker ids.
#[derive(Clone, Debug, Default)]
pub struct Names {
    /// The broker's id for each instrument, which an order names.
    pub security: BTreeMap<InstrumentId, String>,
    /// The broker's id for each account, which an order names.
    pub account: BTreeMap<bagholder_core::AccountId, String>,
}

impl Names {
    pub fn load(book: &bagholder_book::Book, inputs: &Inputs) -> Result<Names, String> {
        let mut security = BTreeMap::new();
        for id in inputs.ledger.instruments.keys() {
            let refs = book.instrument_refs(*id).map_err(|e| e.to_string())?;
            if let Some(r) = refs.into_iter().find(|r| matches!(r.scheme, RefScheme::BrokerSecurity(_))) {
                security.insert(*id, r.value);
            }
        }
        let mut account = BTreeMap::new();
        for (id, info) in &inputs.ledger.accounts {
            let refs = book.account_refs(*id).map_err(|e| e.to_string())?;
            if let Some(r) = refs.into_iter().find(|r| r.broker == info.broker) {
                account.insert(*id, r.value);
            }
        }
        Ok(Names { security, account })
    }
}

fn dec(m: &Money) -> Dec {
    Dec(m.amount)
}

fn fig_money(f: &bagholder_engine::gap::Fig<Money>) -> Fig<Dec> {
    Fig::of(f, dec)
}

fn fig_dec(f: &bagholder_engine::gap::Fig<bagholder_core::Dec>) -> Fig<Dec> {
    Fig::of(f, |d| Dec(*d))
}

fn partial(p: &scope::Partial) -> Partial {
    Partial { total: fig_money(&p.total), left_out: p.left_out }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn month_key(year: i16, month: i8) -> String {
    format!("{year:04}-{month:02}")
}

/// `Sep '26`.
fn month_label(year: i16, month: i8) -> String {
    format!("{} '{:02}", MONTHS[(month as usize).clamp(1, 12) - 1], year.rem_euclid(100))
}

/// The page's word for what an instrument is.
fn kind_word(k: InstrumentKind) -> &'static str {
    match k {
        InstrumentKind::OptionContract => "Options",
        InstrumentKind::Crypto => "Crypto",
        InstrumentKind::Future => "Futures",
        _ => "Shares",
    }
}

/// What the page shows of an instrument: its symbol, name and venue now.
struct Shown {
    symbol: String,
    name: String,
    exchange: String,
}

fn shown(inputs: &Inputs, i: InstrumentId) -> Shown {
    let info = inputs.ledger.instruments.get(&i);
    let name = info.and_then(|x| x.current_name());
    let crypto = info.is_some_and(|x| x.instrument.kind == InstrumentKind::Crypto);
    Shown {
        symbol: name.map(|n| n.symbol.clone()).unwrap_or_default(),
        name: name.and_then(|n| n.name.clone()).unwrap_or_default(),
        exchange: if crypto { "Crypto".into() } else { name.and_then(|n| n.venue_name.clone().or_else(|| n.venue_mic.clone())).unwrap_or_default() },
    }
}

/// An account's name: the person's for it, else what it is.
fn account_name(inputs: &Inputs, a: AccountId) -> String {
    use bagholder_core::account::{AccountKind, AccountType, Registration};
    let Some(info) = inputs.ledger.accounts.get(&a) else { return String::new() };
    if let Some(n) = info.account.nickname.as_deref().filter(|n| !n.trim().is_empty()) {
        return n.trim().to_string();
    }
    match &info.account.account_type {
        AccountType::Known { registration, kind, .. } => match registration {
            Registration::Unregistered => match kind {
                AccountKind::Cash => "Cash".into(),
                AccountKind::Margin => "Margin".into(),
                AccountKind::Crypto => "Crypto".into(),
                other => {
                    let w = other.as_str().replace('-', " ");
                    let mut c = w.chars();
                    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
                }
            },
            r => match r {
                Registration::GroupRrsp => "Group RRSP".into(),
                other => other.as_str().to_ascii_uppercase(),
            },
        },
        AccountType::Unrecognised(t) => t.clone(),
    }
}

fn trade_id(t: &TradeFig) -> String {
    match &t.key {
        TradeKey::Group(g) => g.to_string(),
        TradeKey::Trip(k) => t.trade.map(|id| id.to_string()).unwrap_or_else(|| format!("{}|{}", k.opening, k.instrument)),
    }
}

fn key_id(figs: &BTreeMap<&TradeKey, &TradeFig>, k: &TradeKey) -> String {
    figs.get(k).map(|t| trade_id(t)).unwrap_or_default()
}

/// A holding's id: its account, instrument and direction.
pub fn position_id(p: &PositionFig) -> String {
    format!("{}:{}:{}", p.account, p.instrument, if p.direction == Direction::Short { "short" } else { "long" })
}

fn grade_word(g: Option<Grade>) -> String {
    g.map(|g| g.as_str().to_string()).unwrap_or_default()
}

fn underlying_symbol(inputs: &Inputs, i: InstrumentId) -> String {
    shown(inputs, scope::underlying_of(inputs, i)).symbol
}

fn trade(inputs: &Inputs, names: &Names, t: &TradeFig, position: Option<String>) -> Trade {
    let s = shown(inputs, t.instrument);
    Trade {
        id: trade_id(t),
        status: if t.status == TradeStatus::Closed { Status::Closed } else { Status::Open },
        position,
        underlying: underlying_symbol(inputs, t.instrument),
        symbol: s.symbol,
        name: s.name,
        exchange: s.exchange,
        kind: kind_word(t.kind).into(),
        currency: t.currency.as_str().into(),
        account: account_name(inputs, t.account),
        account_id: t.account.to_string(),
        instrument: t.instrument.to_string(),
        security: names.security.get(&t.instrument).cloned().unwrap_or_default(),
        side: if t.direction == Direction::Short { "COVER" } else { "SELL" }.into(),
        qty: fig_dec(&t.qty),
        entry: fig_dec(&t.entry),
        exit: t.exit.as_ref().map(fig_dec),
        entry_date: t.opened_on.to_string(),
        exit_date: t.closed_on.map(|d| d.to_string()),
        last_date: t.last_on.to_string(),
        hold_days: t.hold_days,
        pnl: fig_money(&t.pnl),
        pnl_cad: fig_money(&t.pnl_cad),
        pnl_pct: t.pnl_pct(),
        fees: fig_money(&t.fees),
        flags: t.flags.iter().map(|f| f.to_string()).collect(),
        grade: grade_word(t.journal.grade),
        thesis: t.journal.thesis.clone(),
        tags: t.journal.tags.clone(),
    }
}

fn ratio(d: bagholder_core::Dec) -> f64 {
    d.to_f64()
}

fn position(inputs: &Inputs, names: &Names, p: &PositionFig, trade: String) -> Position {
    let s = shown(inputs, p.instrument);
    let last = Fig::of(&p.mark, |m| Dec(m.price));
    let percent_change = p.mark.as_ref().ok().and_then(|m| m.change_pct).map(|c| ratio(c) / 100.0);
    Position {
        id: position_id(p),
        trade,
        underlying: underlying_symbol(inputs, p.instrument),
        symbol: s.symbol,
        name: s.name,
        exchange: s.exchange,
        kind: kind_word(p.kind).into(),
        currency: p.currency.as_str().into(),
        account: account_name(inputs, p.account),
        account_id: p.account.to_string(),
        instrument: p.instrument.to_string(),
        security: names.security.get(&p.instrument).cloned().unwrap_or_default(),
        short: p.direction == Direction::Short,
        qty: fig_dec(&p.qty),
        avg: fig_dec(&p.avg),
        cost: fig_money(&p.book),
        last,
        mv: fig_money(&p.market),
        unreal: fig_money(&p.unrealized),
        unreal_pct: match (&p.unrealized, &p.book) {
            (Ok(u), Ok(b)) => bagholder_engine::stat::money_ratio(*u, *b),
            _ => None,
        },
        day_change: Fig::of(&p.day_change, |d| d.as_ref().map(dec)),
        percent_change,
        opened: p.opened_on.to_string(),
        held: Fig::of(&p.held_days, |d| *d),
        grade: grade_word(p.journal.grade),
        thesis: p.journal.thesis.clone(),
        tags: p.journal.tags.clone(),
        gaps: p.gaps.words().into_iter().map(String::from).collect(),
        flags: p.flags.iter().map(|f| f.to_string()).collect(),
    }
}

/// The largest first, up to `cap`, the rest folded into `Other (n)`; a slice
/// named `last` (the unclassified) drawn after the rest.
fn slices(mut items: Vec<(String, Money, f64, Option<String>)>, cap: usize, last: Option<&str>) -> Vec<Slice> {
    let kept_last = last.and_then(|l| items.iter().position(|x| x.0 == l)).map(|i| items.remove(i));
    items.sort_by(|a, b| b.1.amount.cmp(&a.1.amount));
    let rest = if items.len() > cap { items.split_off(cap) } else { Vec::new() };
    let mut out: Vec<Slice> = items.into_iter().map(|(label, value, share, id)| Slice { label, value: Fig::Stated(dec(&value)), share, id }).collect();
    if !rest.is_empty() {
        let n = rest.len();
        let value = match rest.iter().try_fold(Money::zero(rest[0].1.currency), |a, x| Money::add_to_fit(a, x.1)) {
            Ok(v) => Fig::Stated(dec(&v)),
            Err(e) => Fig::Waits { gaps: vec![format!("arithmetic: {e}")] },
        };
        out.push(Slice { label: format!("Other ({n})"), value, share: rest.iter().map(|x| x.2).sum(), id: None });
    }
    if let Some((label, value, share, id)) = kept_last {
        out.push(Slice { label, value: Fig::Stated(dec(&value)), share, id });
    }
    out
}

fn kpi(k: &scope::Kpi) -> Kpi {
    Kpi {
        realized: fig_money(&k.realized),
        realized_left_out: k.realized_left_out,
        count: k.count,
        left_out: k.left_out,
        wins: k.wins,
        losses: k.losses,
        breakeven: k.breakeven,
        win_rate: k.win_rate,
        gross_win: fig_money(&k.gross_win),
        gross_loss: fig_money(&k.gross_loss),
        profit_factor: k.profit_factor.clone().into_wire(),
        profit_factor_infinite: k.profit_factor_infinite,
        expectancy: Fig::of(&k.expectancy, |m| m.as_ref().map(dec)),
        avg_win: Fig::of(&k.avg_win, |m| m.as_ref().map(dec)),
        avg_loss: Fig::of(&k.avg_loss, |m| m.as_ref().map(dec)),
    }
}

trait IntoWire<T> {
    fn into_wire(self) -> Fig<T>;
}

impl<T> IntoWire<T> for bagholder_engine::gap::Fig<T> {
    fn into_wire(self) -> Fig<T> {
        match self {
            Ok(v) => Fig::Stated(v),
            Err(g) => Fig::Waits { gaps: g.words().into_iter().map(String::from).collect() },
        }
    }
}

const BENCHMARKS: [(&str, &str); 3] = [("SP500", "S&P 500"), ("TSX", "S&P/TSX"), ("TX60", "TSX 60")];

fn missing_word(m: Missing) -> &'static str {
    match m {
        Missing::GradeAndThesis => "no grade or thesis",
        Missing::Grade => "no grade",
        Missing::Thesis => "no thesis",
    }
}

fn paid_label(l: &PaidLabel) -> String {
    match l {
        PaidLabel::Year(y) => y.to_string(),
        PaidLabel::YearToDate(y) => format!("{y} YTD"),
        PaidLabel::AllTime => "All time".into(),
        PaidLabel::LastTwelveMonths => "Last 12 months".into(),
    }
}

fn options(inputs: &Inputs, trades: &[TradeFig], positions: &[PositionFig]) -> Options {
    let accounts: Vec<AccountOption> = inputs.ledger.accounts.keys().map(|a| AccountOption { id: a.to_string(), name: account_name(inputs, *a) }).collect();
    let used: BTreeSet<InstrumentId> = trades.iter().flat_map(|t| t.instruments.iter().copied()).chain(positions.iter().map(|p| p.instrument)).collect();
    // a contract's underlying too: the by-symbol rows and the symbol filter name a contract's trades by it
    let underlyings: BTreeSet<InstrumentId> = used.iter().filter_map(|i| inputs.ledger.instruments.get(i)?.terms.as_ref().map(|t| t.underlying)).filter(|u| !used.contains(u)).collect();
    let option = |i: &InstrumentId| {
        let info = inputs.ledger.instruments.get(i)?;
        let s = shown(inputs, *i);
        Some(InstrumentOption { id: i.to_string(), symbol: s.symbol, name: s.name, exchange: s.exchange, kind: kind_word(info.instrument.kind).into(), currency: info.instrument.currency.as_str().into() })
    };
    let traded: Vec<InstrumentOption> = used.iter().filter_map(option).collect();
    let sorted = |v: BTreeSet<String>| v.into_iter().collect::<Vec<_>>();
    let tags = sorted(trades.iter().flat_map(|t| t.journal.tags.iter().cloned()).collect());
    let exchanges = sorted(traded.iter().map(|i| i.exchange.clone()).filter(|e| !e.is_empty()).collect());
    let kinds: Vec<String> = ["Shares", "Options", "Crypto", "Futures"].iter().filter(|k| traded.iter().any(|i| i.kind == **k)).map(|k| k.to_string()).collect();
    let mut instruments = traded;
    instruments.extend(underlyings.iter().filter_map(option));
    let mut years: Vec<String> = trades.iter().filter_map(|t| t.closed_on).map(|d| d.year().to_string()).collect::<BTreeSet<_>>().into_iter().collect();
    years.reverse();
    Options {
        accounts,
        instruments,
        tags,
        exchanges,
        kinds,
        grades: ["A", "B", "C", "F", "Ungraded"].iter().map(|s| s.to_string()).collect(),
        sides: ["SELL", "COVER"].iter().map(|s| s.to_string()).collect(),
        results: ["Winners", "Losers", "Breakeven"].iter().map(|s| s.to_string()).collect(),
        years,
    }
}

/// The book's accounts, as the page and the order ticket name them.
pub fn accounts(inputs: &Inputs, names: &Names) -> Vec<Account> {
    use bagholder_core::account::{AccountKind, AccountType};
    inputs
        .ledger
        .accounts
        .iter()
        .map(|(id, info)| Account {
            id: id.to_string(),
            name: account_name(inputs, *id),
            broker_account: names.account.get(id).cloned(),
            status: info.account.status.as_str().into(),
            // an account the ticket can place with: the one broker orders go to, self-directed, trading securities
            tradable: info.broker == bagholder_core::Broker::named("wealthsimple") && matches!(info.account.account_type, AccountType::Known { managed: false, kind: AccountKind::Cash | AccountKind::Margin, .. }),
            margin: matches!(info.account.account_type, AccountType::Known { kind: AccountKind::Margin, .. }),
            nav: inputs.market.brokers.get(id).and_then(|b| b.net_value.iter().next_back().map(|(_, v)| Dec(*v))),
        })
        .collect()
}

/// Everything the page shows of the book under `filters`.
pub fn build(engine: &Engine, names: &Names, filters: &Filters, base: &bagholder_model::base::Base) -> Figures {
    let inputs = engine.inputs();
    let figs = engine.figures();
    let scoped: Scoped = engine.scope(filters);
    let by_key: BTreeMap<&TradeKey, &TradeFig> = figs.trades.iter().map(|t| (&t.key, t)).collect();
    let ids = |ks: &[TradeKey]| ks.iter().map(|k| key_id(&by_key, k)).collect::<Vec<_>>();

    // each open trade's holding, and each holding's trade
    let mut holding_of: BTreeMap<String, String> = BTreeMap::new();
    let mut trade_of: BTreeMap<String, String> = BTreeMap::new();
    for p in figs.positions {
        if let Some(t) = figs.trades.iter().find(|t| matches!(&t.key, TradeKey::Trip(k) if *k == p.key)) {
            holding_of.insert(trade_id(t), position_id(p));
            trade_of.insert(position_id(p), trade_id(t));
        }
    }
    let trades: Vec<Trade> = scoped.trades.iter().filter_map(|k| by_key.get(k)).map(|t| trade(inputs, names, t, holding_of.get(&trade_id(t)).cloned().filter(|_| t.status != TradeStatus::Closed))).collect();
    let positions: Vec<Position> = scoped.portfolio.positions.iter().map(|i| &figs.positions[*i]).map(|p| position(inputs, names, p, trade_of.get(&position_id(p)).cloned().unwrap_or_default())).collect();

    let today = inputs.clock.today;
    let benchmark = BENCHMARKS.iter().find(|(k, _)| *k == filters.benchmark).map(|(k, l)| BenchmarkRef { key: k.to_string(), label: l.to_string() }).unwrap_or(BenchmarkRef { key: filters.benchmark.clone(), label: String::new() });

    let e = &scoped.equity;
    let mut equity_gaps: Vec<String> = e.gaps.words().into_iter().map(String::from).collect();
    if e.series.iter().any(|d| d.exact.is_none()) {
        // a day whose accounts' values add past what a decimal holds is not drawn
        equity_gaps.push("arithmetic".into());
    }
    let equity = Equity {
        series: e.series.iter().filter_map(|d| d.exact.map(|v| Point { d: d.day.to_string(), v: Dec(v) })).collect(),
        drawdown: Drawdown { pct: e.drawdown.pct, abs: e.drawdown.abs.and_then(bagholder_engine::stat::returns::cents).map(Dec), at: e.drawdown.at.map(|d| d.to_string()) },
        annualized: Annualized { rate: e.annualized.rate, count: e.annualized.count },
        gaps: equity_gaps,
    };
    let years = e.years.iter().map(|y| YearRow { year: y.year.to_string(), r: y.r, sp_r: y.benchmark }).collect();

    let pf = &scoped.portfolio;
    let allocation = slices(
        pf.allocation.iter().map(|a| {
            let p = &figs.positions[a.position];
            (shown(inputs, p.instrument).symbol, a.value, a.share.clone().unwrap_or(0.0), Some(position_id(p)))
        }).collect(),
        10,
        None,
    );
    let portfolio = Portfolio {
        position_count: pf.positions.len(),
        market_value: partial(&pf.market_value),
        cost_basis: partial(&pf.cost_basis),
        unrealized: partial(&pf.unrealized),
        unrealized_pct: pf.unrealized_pct.clone().into_wire(),
        nav: pf.net_value.as_ref().map(fig_money),
        nav_accounts: pf.net_value_accounts,
        has_margin: pf.has_margin,
        margin_used: fig_money(&pf.margin_used),
        margin_used_pct: pf.margin_used_pct.clone().into_wire(),
        available_margin: pf.available_margin.as_ref().map(fig_money),
        available_margin_unavailable: pf.margin_unavailable.iter().map(|(a, _)| account_name(inputs, *a)).collect(),
        cash: fig_money(&pf.cash),
        cash_pct: pf.cash_pct.clone().into_wire(),
        day_change: pf.day_change.as_ref().map(partial),
        day_change_pct: pf.day_change_pct.clone().into_wire(),
        allocation,
    };

    let cashflow = cashflow(inputs, &figs, &scoped, today);

    let accounts = accounts(inputs, names);
    let nav_total = {
        let navs: Vec<bagholder_core::Dec> = accounts.iter().filter_map(|a| a.nav.map(|d| d.0)).collect();
        (!navs.is_empty()).then(|| match navs.into_iter().try_fold(bagholder_core::Dec::ZERO, |a, b| a.checked_add(b)) {
            Ok(total) => Fig::Stated(Dec(total)),
            Err(e) => Fig::Waits { gaps: vec![format!("arithmetic: {e}")] },
        })
    };

    let context = super::context::context(base, &positions);
    Figures {
        waiting: figs
            .matched
            .waiting
            .iter()
            .map(|(t, w)| {
                let s = shown(inputs, w.instrument);
                Waiting {
                    transaction: t.to_string(),
                    what: match w.what {
                        bagholder_engine::ledger::Wanted::CostOfArrival => "cost-of-arrival",
                        bagholder_engine::ledger::Wanted::Event => "event",
                    }
                    .into(),
                    account: w.account.to_string(),
                    account_name: account_name(inputs, w.account),
                    instrument: w.instrument.to_string(),
                    symbol: s.symbol,
                    currency: inputs.ledger.instruments.get(&w.instrument).map(|i| i.instrument.currency.as_str().to_string()).unwrap_or_default(),
                    day: w.day.to_string(),
                    units: w.units.map(Dec),
                }
            })
            .collect(),
        markets: context.markets,
        sectors: context.sectors,
        regions: context.regions,
        today: today.to_string(),
        activity_count: inputs.ledger.transactions.len(),
        options: options(inputs, figs.trades, figs.positions),
        kpi: kpi(&scoped.kpi),
        equity,
        years,
        benchmark,
        monthly: scoped.monthly.iter().map(|m| MonthlyBar { key: month_key(m.year, m.month), label: month_label(m.year, m.month), value: fig_money(&m.value), count: m.count, trade_ids: ids(&m.trades) }).collect(),
        by_symbol: scoped.by_underlying.iter().map(|r| BySymbolRow { id: r.underlying.to_string(), symbol: shown(inputs, r.underlying).symbol, pnl: fig_money(&r.pnl), n: r.count, win_rate: r.win_rate, avg_hold: r.avg_hold, trade_ids: ids(&r.trades) }).collect(),
        grades: Grades {
            buckets: scoped.grades.iter().map(|g| GradeBucket { grade: g.grade.as_str().into(), n: g.count, pnl: fig_money(&g.pnl), trade_ids: ids(&g.trades) }).collect(),
            graded: scoped.grades.iter().map(|g| g.count).sum(),
        },
        queue: scoped
            .queue
            .iter()
            .filter_map(|(k, m)| by_key.get(k).map(|t| QueueRow { id: trade_id(t), symbol: shown(inputs, t.instrument).symbol, date: t.closed_on.unwrap_or(t.last_on).to_string(), pnl: fig_money(&t.pnl_cad), missing: missing_word(*m).into() }))
            .collect(),
        trades,
        positions,
        portfolio,
        cashflow,
        accounts,
        nav_total,
    }
}

fn cashflow(inputs: &Inputs, figs: &bagholder_engine::engine::Figures, scoped: &Scoped, today: bagholder_core::jiff::civil::Date) -> Cashflow {
    let c = &scoped.cashflow;
    let tiles = c
        .tiles
        .iter()
        .map(|t| match t {
            CashTile::Paid { label, total, per_paying_month, .. } => CashflowTile::Paid { label: paid_label(label), total: partial(total), per_month: Fig::of(per_paying_month, |m| m.as_ref().map(dec)) },
            CashTile::Margin { margin_used, interest_per_month, .. } => CashflowTile::Margin { label: "Margin used".into(), margin_used: fig_money(margin_used), interest_per_month: Fig::of(interest_per_month, |m| m.as_ref().map(dec)) },
            CashTile::Yield { yield_on_cost, projected_per_month, .. } => CashflowTile::Yield { label: "Yield on cost".into(), yield_on_cost: yield_on_cost.clone().into_wire(), projected: fig_money(projected_per_month) },
        })
        .collect();
    let months = c
        .months
        .iter()
        .map(|m| {
            let net = match (&m.distributions.total, &m.interest.total) {
                (Ok(d), Ok(i)) => d.amount.checked_sub(i.amount).map(|n| Fig::Stated(Dec(n))).unwrap_or(Fig::Waits { gaps: vec!["arithmetic".into()] }),
                (Err(g), _) | (_, Err(g)) => Fig::Waits { gaps: g.words().into_iter().map(String::from).collect() },
            };
            CashflowMonth { key: month_key(m.year, m.month), label: month_label(m.year, m.month), value: fig_money(&m.distributions.total), count: m.count, interest: fig_money(&m.interest.total), net }
        })
        .collect();
    let twelve = bagholder_core::Dec::from_int(12);
    let holdings: Vec<CashflowHolding> = c
        .holdings
        .iter()
        .map(|h| {
            let p = &figs.positions[h.position];
            let per_month = h.annual.as_ref().map_err(|g| g.clone()).and_then(|a| a.amount.div_rounded(twelve, 2, bagholder_core::Rounding::HalfEven).map_err(|e| bagholder_engine::gap::Gaps::of(bagholder_engine::gap::Gap::Arithmetic(e.to_string()))));
            CashflowHolding {
                id: position_id(p),
                symbol: shown(inputs, p.instrument).symbol,
                account: account_name(inputs, p.account),
                currency: p.currency.as_str().into(),
                qty: fig_dec(&p.qty),
                avg: fig_dec(&p.avg),
                cost: fig_money(&p.book),
                mv: fig_money(&p.market),
                per: fig_money(&h.rate.per),
                ytd: partial(&h.ytd),
                all: partial(&h.all_time),
                next_ex_date: h.rate.next_ex.map(|d| d.to_string()),
                next_pay_date: h.rate.next_pay.map(|d| d.to_string()),
                ex_past: h.rate.next_ex.is_some_and(|d| d < today),
                pay_past: h.rate.next_pay.is_some_and(|d| d < today),
                annual: fig_money(&h.annual),
                per_month: fig_dec(&per_month),
                yoc: h.yield_on_cost.clone().into_wire(),
                current_yield: h.current_yield.clone().into_wire(),
            }
        })
        .collect();
    // the pie: each holding's projected income a month in CAD, largest first
    let stated: Vec<(String, Money, Option<String>)> = c
        .holdings
        .iter()
        .filter_map(|h| {
            let p = &figs.positions[h.position];
            h.projected_per_month_cad.as_ref().ok().filter(|m| m.amount.is_positive()).map(|m| (shown(inputs, p.instrument).symbol, *m, Some(position_id(p))))
        })
        .collect();
    let total = stated.iter().try_fold(Money::zero(bagholder_core::Currency::CAD), |a, x| Money::add_to_fit(a, x.1));
    // the pie's total counts the holdings whose projection waits
    let income_total = Partial {
        total: match &total {
            Ok(t) => Fig::Stated(dec(t)),
            Err(e) => Fig::Waits { gaps: vec![format!("arithmetic: {e}")] },
        },
        left_out: c.holdings.iter().filter(|h| h.projected_per_month_cad.is_err()).count(),
    };
    let mut stated = stated;
    stated.sort_by(|a, b| b.1.amount.cmp(&a.1.amount));
    let income: Vec<Slice> = stated
        .iter()
        .map(|(symbol, m, id)| Slice { label: symbol.clone(), value: Fig::Stated(dec(m)), share: total.as_ref().ok().and_then(|t| bagholder_engine::stat::money_ratio(*m, *t)).unwrap_or(0.0), id: id.clone() })
        .collect();
    let rows = c
        .rows
        .iter()
        .map(|i| &figs.cash[*i])
        .map(|r: &CashRow| CashflowRow {
            id: r.id.to_string(),
            date: r.day.to_string(),
            symbol: r.instrument.map(|i| shown(inputs, i).symbol).unwrap_or_else(|| if r.kind == Payment::Interest { "Cash".into() } else { "—".into() }),
            account: account_name(inputs, r.account),
            qty: r.qty.map(Dec),
            per: r.per.as_ref().map(fig_money),
            amount: dec(&r.amount),
            currency: r.amount.currency.as_str().into(),
        })
        .collect();
    Cashflow { tiles, months, holdings, income, income_total, rows, skipped_filters: c.unread_filters.iter().map(|s| s.to_string()).collect() }
}

/// What a fill did, as its row shows it: an option's buy or sale opening or
/// closing, a share's buy or sale, or what else moved the position.
/// A fill's side, and what it did in its trade (SPEC §Trades, the executions): an
/// option's opens or closes (`BUY TO OPEN`, `SELL TO CLOSE`); a fill that closed one
/// trade and opened the next, of any kind, `(close + open)`; else bought or sold.
fn fill_words(tx: &bagholder_core::transaction::Transaction, option: bool, role: Option<bagholder_engine::ledger::FillRole>) -> (String, String) {
    use bagholder_core::transaction::Kind;
    let side = match tx.kind {
        Kind::Buy => "BUY",
        Kind::Sell => "SELL",
        _ => "",
    };
    if side.is_empty() {
        return (String::new(), tx.kind.as_str().replace('-', " ").to_ascii_uppercase());
    }
    let sub = match role.map(|r| (r.closed, r.opened)) {
        Some((true, true)) => format!("{side} (close + open)"),
        Some((true, false)) if option => format!("{side} TO CLOSE"),
        Some((false, true)) if option => format!("{side} TO OPEN"),
        _ => side.to_string(),
    };
    (side.to_string(), sub)
}

/// The book's holdings and trades whose symbol `named` accepts, for a listing's
/// page: a holding by its id, a trade with its fills.
pub fn listed(engine: &Engine, names: &Names, named: &dyn Fn(&str) -> bool) -> (Vec<crate::feeds::ListedRow>, Vec<crate::feeds::ListedRow>) {
    let inputs = engine.inputs();
    let figs = engine.figures();
    let positions = figs
        .positions
        .iter()
        .map(|p| position(inputs, names, p, String::new()))
        .filter(|p| named(&p.symbol))
        .map(|p| crate::feeds::ListedRow { id: p.id, symbol: p.symbol, exchange: p.exchange, currency: p.currency, kind: p.kind, name: p.name, security_id: p.security, fills: Vec::new() })
        .collect();
    let trades = figs
        .trades
        .iter()
        .map(|t| trade(inputs, names, t, None))
        .filter(|t| named(&t.symbol))
        .map(|t| {
            let fills = detail(engine, &t.id).map(|d| d.fills).unwrap_or_default();
            crate::feeds::ListedRow { id: t.id, symbol: t.symbol, exchange: t.exchange, currency: t.currency, kind: t.kind, name: t.name, security_id: t.security, fills }
        })
        .collect();
    (positions, trades)
}

/// A trade's or holding's fills, by its id, for its page.
pub fn detail(engine: &Engine, id: &str) -> Option<Detail> {
    let figs = engine.figures();
    let fills = figs.trades.iter().find(|t| trade_id(t) == id).map(|t| &t.fills).or_else(|| figs.positions.iter().find(|p| position_id(p) == id).map(|p| &p.fills))?;
    let by_id: BTreeMap<&bagholder_core::TransactionId, &bagholder_core::transaction::Transaction> = engine.inputs().ledger.transactions.iter().map(|t| (&t.id, t)).collect();
    let mut rows: Vec<Fill> = fills
        .iter()
        .filter_map(|f| by_id.get(f))
        .map(|tx| {
            let option = tx.instrument.and_then(|i| engine.inputs().ledger.instruments.get(&i)).is_some_and(|i| i.instrument.kind == InstrumentKind::OptionContract);
            let (side, sub) = fill_words(tx, option, figs.matched.roles.get(&tx.id).copied());
            let currency = tx.cash.map(|c| c.currency).or(tx.price.map(|p| p.currency));
            Fill {
                id: tx.id.to_string(),
                when: tx.occurred_at.map(|t| t.to_string()),
                date: tx.trade_date.to_string(),
                side,
                sub,
                qty: match tx.quantity {
                    Some(q) => Fig::Stated(Dec(q)),
                    None => Fig::Waits { gaps: vec!["quantity-unstated".into()] },
                },
                price: fig_dec(&bagholder_engine::ledger::fill_price(tx, tx.instrument.and_then(|i| engine.inputs().ledger.instruments.get(&i)))),
                amount: match tx.cash {
                    Some(c) => Fig::Stated(Dec(c.amount)),
                    None => Fig::Waits { gaps: vec!["value-unstated".into()] },
                },
                currency: currency.map(|c| c.as_str().to_string()).unwrap_or_default(),
                flags: Vec::new(),
            }
        })
        .collect();
    rows.sort_by(|a, b| (&b.when, &b.date).cmp(&(&a.when, &a.date)));
    Some(Detail { id: id.to_string(), fills: rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JSON value's shape: each object's fields and theirs, each list by the
    /// shapes its rows take, each value by its type.
    fn shape(v: &serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match v {
            Value::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), shape(v))).collect()),
            Value::Array(a) => {
                let mut rows: Vec<Value> = a.iter().map(shape).collect();
                rows.sort_by_key(|r| r.to_string());
                rows.dedup();
                Value::Array(rows)
            }
            Value::String(_) => "string".into(),
            Value::Number(_) => "number".into(),
            Value::Bool(_) => "bool".into(),
            Value::Null => Value::Null,
        }
    }

    #[test]
    fn a_fill_says_what_it_did_in_its_trade() {
        use bagholder_core::transaction::{Kind, Transaction};
        use bagholder_engine::ledger::FillRole;
        let u = "0192a000-0000-7000-8000-00000000000";
        let tx = |kind: Kind| Transaction {
            id: bagholder_core::TransactionId::parse(&format!("{u}1/trade")).unwrap(),
            mapping: bagholder_core::MappingVersion { source: bagholder_core::SourceName::named("test"), version: 1 },
            account: AccountId::parse(&format!("{u}2")).unwrap(),
            occurred_at: None,
            trade_date: "2026-04-20".parse().unwrap(),
            settle_date: None,
            kind,
            effect: None,
            instrument: None,
            quantity: None,
            price: None,
            cash: None,
            fee: None,
            fx_rate: None,
        };
        let words = |kind, option, closed, opened| fill_words(&tx(kind), option, Some(FillRole { closed, opened })).1;
        assert_eq!(words(Kind::Buy, true, false, true), "BUY TO OPEN");
        assert_eq!(words(Kind::Sell, true, true, false), "SELL TO CLOSE");
        assert_eq!(words(Kind::Buy, true, true, true), "BUY (close + open)");
        assert_eq!(words(Kind::Sell, false, true, true), "SELL (close + open)");
        assert_eq!(words(Kind::Buy, false, false, true), "BUY", "shares and coins are bought and sold");
        assert_eq!(words(Kind::Sell, false, true, false), "SELL");
        assert_eq!(fill_words(&tx(Kind::Buy), true, None).1, "BUY", "a fill the match did not apply");
    }

    /// The document built from a real month of one account: every list's rows
    /// told apart by their ids, every open trade naming its holding, every
    /// holding its trade.
    #[test]
    fn the_document_from_a_real_month_names_every_row_by_its_id() {
        let _g = crate::tests_common::guard();
        let base = crate::tests_common::app().base().unwrap();
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let now: bagholder_core::jiff::Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        // a made-up quote for each holding: a tenth over what it cost a unit
        let cache = f.cache().unwrap();
        let held: Vec<(InstrumentId, bagholder_core::Currency, bagholder_core::Dec)> = f
            .read(|e| e.figures().positions.iter().filter_map(|p| p.avg.as_ref().ok().map(|a| (p.instrument, p.currency, *a))).collect())
            .unwrap();
        for (i, currency, avg) in &held {
            let price = avg.checked_mul(bagholder_core::Dec::parse("1.1").unwrap()).unwrap().round(2, bagholder_core::Rounding::HalfEven);
            let quote = bagholder_sources::cache::StoredQuote { instrument: *i, source: bagholder_core::SourceName::named("test"), price: Money::new(price, *currency), change: None, change_pct: None, quoted_at: now, allowance: std::time::Duration::ZERO, received_at: now };
            cache.store_quote(&quote).unwrap();
            f.price_changed(*i).unwrap();
        }
        let book = f.book().unwrap();
        let names = f.read(|e| Names::load(&book, e.inputs())).unwrap().unwrap();
        let doc = f.read(|e| build(e, &names, &Filters::default(), &base)).unwrap();
        assert!(!doc.trades.is_empty() && !doc.positions.is_empty());
        let unique = |ids: Vec<&str>| {
            let mut seen = std::collections::BTreeSet::new();
            ids.into_iter().all(|i| !i.is_empty() && seen.insert(i))
        };
        assert!(unique(doc.trades.iter().map(|t| t.id.as_str()).collect()));
        assert!(unique(doc.positions.iter().map(|p| p.id.as_str()).collect()));
        assert!(unique(doc.cashflow.rows.iter().map(|r| r.id.as_str()).collect()));
        for t in doc.trades.iter().filter(|t| t.status == Status::Open) {
            let p = t.position.as_deref().expect("an open trade names its holding");
            assert!(doc.positions.iter().any(|x| x.id == p && x.trade == t.id), "{p}");
        }
        for p in &doc.positions {
            assert!(!p.security.is_empty(), "{}: the broker's id an order names", p.symbol);
            assert!(!p.account.is_empty());
        }
        // the page's own tests read this document: kept beside them, the market's
        // context (which reads the day it is built on) left out
        let mut page = serde_json::to_value(&doc).unwrap();
        page["markets"] = serde_json::Value::Null;
        page["sectors"] = serde_json::json!([]);
        page["regions"] = serde_json::json!([]);
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/fixtures/figures_pulled_month.json");
        if std::env::var("BAGHOLDER_BLESS").is_ok_and(|v| v == "1") {
            std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
            std::fs::write(&fixture, serde_json::to_string_pretty(&page).unwrap() + "\n").unwrap();
        }
        // the ids are made new on every pull, so the fixture is held to the document's
        // shape: every field the server writes, of the type it writes, and no other
        let kept: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&fixture).unwrap_or_default()).unwrap_or_default();
        assert_eq!(shape(&kept), shape(&page), "web/src/lib/fixtures/figures_pulled_month.json is not the shape of the document the server builds: run with BAGHOLDER_BLESS=1 and review the diff");
        // money is text on the wire
        let json = serde_json::to_value(&doc).unwrap();
        assert!(json["positions"][0]["cost"].is_string() || json["positions"][0]["cost"]["gaps"].is_array());
    }
}
