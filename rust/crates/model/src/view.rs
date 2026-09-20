//! One filter object applied to the whole base: `portfolio_view`,
//! `cashflow_view` and `build_view`.
//!
//! Per-instrument figures stay in the instrument's own currency; everything
//! that adds instruments together is CAD.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::activity::{Flag, Kind};
use crate::base::Base;
use crate::dates::shift_date;
use crate::exposure::exposure_slices;
use crate::filters::{clean_filters, date_bounds, in_date_scope, position_matches, trade_matches, Filters, BENCHMARK_LABELS};
use crate::fx::to_cad;
use crate::nav::{annualized, drawdown, yearly_returns, Point};
use crate::stats::{by_symbol, grade_buckets, metrics, month_label, monthly, payments_per_year, review_queue, GRADES};
use crate::value::FSum;
use crate::wire::{
    Allocation, BenchmarkRef, Cashflow, CashflowHolding, CashflowMonth, CashflowRow, CashflowTile, EquityBlock, ListingInfo, MarketDates, Options, Ordered, Payment, Portfolio, Position, PositionsSummary, Priced,
    RateSource, Trade, TradeDetail, View,
};

/// A position's market value as it counts toward the book: a short owes it.
fn signed_mv(p: &Position) -> f64 {
    if p.short { -p.mv } else { p.mv }
}

/// The Portfolio tiles, CAD aggregates over the accounts in scope.
///
/// Market value, cost basis and unrealized P&L come from the open positions in
/// scope at today's rate. Net asset value is the sum of Wealthsimple's net
/// liquidation value per account, margin used the negative cash balances per
/// currency, available margin the buying power of the margin accounts only --
/// every self-directed account answers that query with its cash to buy with,
/// which is not margin.
pub fn portfolio_view(base: &Base, f: &Filters, positions: &[&Position]) -> Portfolio {
    let cad = |amount: f64, currency: &str| to_cad(&base.fx, amount, currency, &base.today);

    let names = &f.lists.account;
    // closed accounts hold nothing and count for nothing here
    let accounts: Vec<&crate::wire::Account> = base.accounts.iter().filter(|a| a.status.to_lowercase() != "closed").filter(|a| names.is_empty() || names.contains(&a.name)).collect();
    let ids: BTreeSet<&str> = accounts.iter().map(|a| a.id.as_str()).collect();

    let mv: f64 = positions.iter().map(|p| cad(signed_mv(p), &p.currency)).fsum();
    let cost: f64 = positions.iter().map(|p| cad(p.cost.abs(), &p.currency)).fsum();
    let unreal: f64 = positions.iter().map(|p| cad(p.unreal, &p.currency)).fsum();
    let navs: Vec<f64> = accounts.iter().filter_map(|a| a.nav.map(|n| cad(n, &a.currency))).collect();

    // the negative cash balances are the margin drawn; the positive ones the cash
    let mut used: BTreeMap<String, f64> = BTreeMap::new();
    let mut cash_by: BTreeMap<String, f64> = BTreeMap::new();
    for b in base.balances.iter().filter(|b| ids.contains(b.account_id.as_str())) {
        let Some(currency) = base.cash_currencies.get(&b.security_id).filter(|c| !c.is_empty()) else { continue };
        if b.quantity < 0.0 {
            *used.entry(currency.clone()).or_insert(0.0) += -b.quantity;
        } else if b.quantity > 0.0 {
            *cash_by.entry(currency.clone()).or_insert(0.0) += b.quantity;
        }
    }
    let margin_used: f64 = used.iter().map(|(c, v)| cad(*v, c)).fsum();
    let cash: f64 = cash_by.iter().map(|(c, v)| cad(*v, c)).fsum();

    // the day's change: each quoted position's, over what those were worth at the previous close
    let quoted: Vec<(&Position, f64)> = positions.iter().filter_map(|p| p.day_change.map(|dc| (*p, dc))).collect();
    let day_change: Option<f64> = (!quoted.is_empty()).then(|| quoted.iter().map(|(p, dc)| cad(*dc, &p.currency)).fsum());
    let day_change_pct = day_change.and_then(|dc| {
        let before: f64 = quoted.iter().map(|(p, _)| cad(signed_mv(p), &p.currency)).fsum() - dc;
        (before != 0.0).then(|| dc / before)
    });

    // only a margin account's buying power is margin available
    let margin_accounts: Vec<&&crate::wire::Account> = accounts.iter().filter(|a| a.kind.to_uppercase().contains("MARGIN")).collect();
    let mut available: Vec<f64> = Vec::new();
    let mut unavailable: Vec<String> = Vec::new();
    for m in base.margin.iter() {
        let Some(account) = margin_accounts.iter().find(|a| a.id == m.account_id) else { continue };
        match m.buying_power {
            None => unavailable.push(account.name.clone()),
            Some(power) => available.push(cad(power, if m.currency.is_empty() { "CAD" } else { &m.currency })),
        }
    }
    unavailable.sort();

    let mut allocation: Vec<Allocation> = positions
        .iter()
        .map(|p| (p, cad(p.mv, &p.currency)))
        .filter(|(_, value)| *value > 0.0)
        .map(|(p, value)| Allocation { id: p.id.clone(), symbol: p.symbol.clone(), account: p.account.clone(), value, share: 0.0 })
        .collect();
    allocation.sort_by(|x, y| y.value.partial_cmp(&x.value).unwrap_or(std::cmp::Ordering::Equal));
    let allocated: f64 = allocation.iter().map(|x| x.value).fsum();
    for x in allocation.iter_mut() {
        x.share = if allocated != 0.0 { x.value / allocated } else { 0.0 };
    }

    let (sectors, regions) = exposure_slices(positions, &base.exposures, &cad);
    let nav_sum: f64 = navs.iter().fsum();
    Portfolio {
        allocation,
        sectors,
        regions,
        market_value: mv + 0.0,
        cost_basis: cost + 0.0,
        unrealized: unreal + 0.0,
        unrealized_pct: (cost != 0.0).then(|| unreal / cost),
        position_count: positions.len(),
        account_count: positions.iter().map(|p| p.account.as_str()).collect::<BTreeSet<_>>().len(),
        nav: (!navs.is_empty()).then_some(nav_sum),
        nav_accounts: navs.len(),
        margin_used: margin_used + 0.0,
        margin_used_by: used.iter().map(|(c, v)| (c.clone(), round2(*v))).collect(),
        margin_used_pct: (mv != 0.0).then(|| margin_used / mv),
        available_margin: (!available.is_empty()).then(|| available.iter().fsum()),
        available_margin_unavailable: unavailable,
        // the tiles a book without a margin account shows in the margin tiles' places
        has_margin: !margin_accounts.is_empty(),
        cash: cash + 0.0,
        cash_pct: (!navs.is_empty() && nav_sum != 0.0).then(|| cash / nav_sum),
        day_change,
        day_change_pct,
    }
}

/// Rounds to two places, half to even.
fn round2(v: f64) -> f64 {
    let scaled = v * 100.0;
    let r = scaled.round();
    let r = if (scaled - scaled.trunc()).abs() == 0.5 && r % 2.0 != 0.0 { r - scaled.signum() } else { r };
    r / 100.0
}

/// What a payer pays: per payment, how often, and where that was read.
struct Rate {
    per: f64,
    freq: i64,
    annual: f64,
    verified: bool,
    source: RateSource,
}

fn month_of(day: &str) -> String {
    day.chars().take(7).collect()
}

pub fn cashflow_view(base: &Base, f: &Filters, positions_all: &[Position], margin_used: f64, has_margin: bool) -> Cashflow {
    let today = base.today.as_str();
    let accounts = &f.lists.account;
    let search = f.search.to_uppercase();
    let in_account = |account: &str| accounts.is_empty() || accounts.iter().any(|a| a == account);
    let searched = |symbol: &str| search.is_empty() || symbol.to_uppercase().contains(&search);
    let in_scope = |r: &CashflowRow| in_account(&r.account) && searched(&r.symbol) && (f.lists.symbol.is_empty() || f.lists.symbol.contains(&r.symbol)) && in_date_scope(f, today, &r.date);

    let everything: Vec<&CashflowRow> = base.cashflow.iter().filter(|r| in_scope(r)).collect();
    let recs: Vec<&CashflowRow> = everything.iter().copied().filter(|r| r.kind == Payment::Dividend).collect();

    // The chart runs to the current month (or the end of the date filter), with
    // an empty bar for a month that has not paid yet.
    let mut keys: Vec<String> = Vec::new();
    let mut bucket: HashMap<String, (f64, usize)> = HashMap::new();
    if !recs.is_empty() {
        let first = recs.iter().map(|r| month_of(&r.date)).min().unwrap();
        let mut last = recs.iter().map(|r| month_of(&r.date)).max().unwrap();
        let end_day = match date_bounds(f, today) {
            Some((_, hi)) => if hi.as_str() < today { hi } else { today.to_string() },
            None if !f.years.is_empty() => {
                let year_end = format!("{}-12-31", f.years.iter().max().unwrap());
                if year_end.as_str() < today { year_end } else { today.to_string() }
            }
            None => today.to_string(),
        };
        if month_of(&end_day) > last {
            last = month_of(&end_day);
        }
        let mut y: i64 = first[..4].parse().unwrap_or(0);
        let mut m: u32 = first[5..7].parse().unwrap_or(1);
        loop {
            let k = format!("{:04}-{:02}", y, m);
            if k > last {
                break;
            }
            bucket.insert(k.clone(), (0.0, 0));
            keys.push(k);
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
    }
    for r in &recs {
        if let Some(e) = bucket.get_mut(&month_of(&r.date)) {
            e.0 += r.amount_cad;
            e.1 += 1;
        }
    }
    let months: Vec<CashflowMonth> = keys.iter().map(|k| CashflowMonth { key: k.clone(), label: month_label(k), value: bucket[k].0, count: bucket[k].1 }).collect();

    let dividends = || base.cashflow.iter().filter(|r| r.kind == Payment::Dividend);
    let payers: BTreeSet<&str> = dividends().map(|r| r.symbol.as_str()).collect();
    let held: Vec<&Position> = positions_all.iter().filter(|p| payers.contains(p.symbol.as_str()) && !p.short && in_account(&p.account) && searched(&p.symbol)).collect();
    let for_yoc: Vec<&CashflowRow> = dividends().filter(|r| in_account(&r.account) && searched(&r.symbol)).collect();

    let last_paid = recs.first().map(|r| r.date.as_str()).unwrap_or(today);
    let cut = trailing_year_month(last_paid);
    let this_year: String = today.chars().take(4).collect();
    let paid_for = |sym: &str, keep: &dyn Fn(&CashflowRow) -> bool| -> f64 { for_yoc.iter().filter(|r| r.symbol == sym && keep(r)).map(|r| r.amount_cad).fsum() + 0.0 };

    // The fund's own declared record first: the latest distribution that has
    // gone ex, and payments per year from the gaps between its ex-dates, so a
    // schedule change shows at once. Never assumed from the instrument.
    let rate_for = |sym: &str| -> Option<Rate> {
        let declared = base.distributions.get(sym).map(|d| d.as_slice()).unwrap_or(&[]);
        // the latest that has gone ex; the first of those on one day
        let latest = declared.iter().filter(|d| d.ex_date.as_str() <= today).fold(None, |best: Option<&crate::input::Distribution>, d| match best {
            Some(b) if b.ex_date >= d.ex_date => Some(b),
            _ => Some(d),
        });
        if let Some(d) = latest {
            let freq = payments_per_year(&declared.iter().map(|d| d.ex_date.clone()).collect::<Vec<_>>());
            if let (true, Some(freq)) = (d.amount != 0.0, freq) {
                return Some(Rate { per: d.amount, freq, annual: d.amount * freq as f64, verified: true, source: RateSource::Declared });
            }
        }
        let paid: Vec<&&CashflowRow> = for_yoc.iter().filter(|r| r.symbol == sym).collect();
        let per = paid.iter().filter(|r| r.per.map_or(false, |p| p != 0.0)).fold(None, |best: Option<&&&CashflowRow>, r| match best {
            Some(b) if b.date >= r.date => Some(b),
            _ => Some(r),
        })?.per?;
        let freq = payments_per_year(&paid.iter().map(|r| r.date.clone()).collect::<Vec<_>>());
        let per_year = freq.unwrap_or(12);
        Some(Rate { per, freq: per_year, annual: per * per_year as f64, verified: freq.is_some(), source: RateSource::Payments })
    };

    // The next distribution still to be paid, whether or not it has gone ex,
    // else the last known one.
    let distribution_dates = |sym: &str| -> (String, String) {
        let pay_day = |d: &crate::input::Distribution| -> String { d.pay_date.chars().take(10).collect() };
        let due = |d: &crate::input::Distribution| -> (String, String) {
            let pay = pay_day(d);
            (if pay.is_empty() { d.ex_date.clone() } else { pay }, d.ex_date.clone())
        };
        let mut declared: Vec<&crate::input::Distribution> = base.distributions.get(sym).map(|d| d.iter().collect()).unwrap_or_default();
        declared.sort_by_cached_key(|d| due(d));
        match declared.iter().find(|d| due(d).0.as_str() >= today).or(declared.last()) {
            Some(d) => (d.ex_date.clone(), pay_day(d)),
            None => {
                let ex: String = base.quotes.get(sym).map(|q| q.ex_dividend_date.chars().take(10).collect()).unwrap_or_default();
                (ex, for_yoc.iter().filter(|r| r.symbol == sym).map(|r| r.date.clone()).max().unwrap_or_default())
            }
        }
    };

    let holdings: Vec<CashflowHolding> = held
        .iter()
        .map(|p| {
            let rate = rate_for(&p.symbol);
            let close = base.quotes.get(&p.symbol).filter(|q| q.fits(p.kind)).and_then(|q| q.price).filter(|px| *px > 0.0);
            let last = close.unwrap_or(p.last);
            let (ex, pay) = distribution_dates(&p.symbol);
            CashflowHolding {
                id: p.id.clone(),
                symbol: p.symbol.clone(),
                account: p.account.clone(),
                qty: p.qty,
                per: rate.as_ref().map(|r| r.per),
                freq: rate.as_ref().map(|r| r.freq),
                freq_verified: rate.as_ref().map_or(false, |r| r.verified),
                rate_source: rate.as_ref().map_or(RateSource::Unknown, |r| r.source),
                cost: p.cost,
                avg: p.avg,
                last,
                price_source: if close.is_some() { Priced::Close } else { Priced::Fill },
                ytd: paid_for(&p.symbol, &|r| r.date.starts_with(&this_year)),
                ttm: paid_for(&p.symbol, &|r| month_of(&r.date) >= cut),
                all: paid_for(&p.symbol, &|_| true),
                ex_past: !ex.is_empty() && ex.as_str() < today,
                pay_past: !pay.is_empty() && pay.as_str() < today,
                next_ex_date: ex,
                next_pay_date: pay,
                yob: rate.as_ref().map(|r| r.per * p.qty),
                annual: rate.as_ref().map(|r| r.annual * p.qty),
                yoc: rate.as_ref().filter(|_| p.avg != 0.0).map(|r| r.annual / p.avg),
                current_yield: rate.as_ref().filter(|_| last != 0.0).map(|r| r.annual / last),
            }
        })
        .collect();

    let rated: Vec<(&CashflowHolding, f64)> = holdings.iter().filter_map(|h| h.annual.map(|a| (h, a))).collect();
    let basis_all: f64 = rated.iter().map(|(h, _)| h.cost).fsum();
    let earned_all: f64 = rated.iter().map(|(h, _)| h.ttm).fsum();
    let annual_all: f64 = rated.iter().map(|(_, annual)| *annual).fsum();
    let total: f64 = recs.iter().map(|r| r.amount_cad).fsum();

    let paid_over = |label: String, rows: &[&CashflowRow], months_paid: usize| {
        let sum: f64 = rows.iter().map(|r| r.amount_cad).fsum();
        CashflowTile::Paid { label, total: sum + 0.0, per_month: sum / months_paid.max(1) as f64, count: rows.len() }
    };
    let this_yr: i64 = this_year.parse().unwrap_or(0);
    let mut tiles: Vec<CashflowTile> = Vec::new();
    for y in [this_yr - 2, this_yr - 1, this_yr] {
        let ys = y.to_string();
        let rows: Vec<&CashflowRow> = recs.iter().copied().filter(|r| r.date.starts_with(&ys)).collect();
        let months_paid = keys.iter().filter(|k| k.starts_with(&ys) && bucket[*k].1 > 0).count();
        tiles.push(paid_over(if y == this_yr { format!("{} YTD", y) } else { ys }, &rows, months_paid));
    }
    tiles.push(paid_over("All time".into(), &recs, keys.iter().filter(|k| bucket[*k].1 > 0).count()));

    if has_margin {
        // margin used is the Portfolio tab's figure; under it the average
        // margin interest per charged month
        let charges: Vec<&&CashflowRow> = everything.iter().filter(|r| r.kind == Payment::InterestCharge).collect();
        let charged_months: BTreeSet<String> = charges.iter().map(|r| month_of(&r.date)).collect();
        let charged: f64 = charges.iter().map(|r| -r.amount_cad).fsum();
        tiles.push(CashflowTile::Margin {
            label: "Margin used",
            margin_used,
            interest_per_month: if charged_months.is_empty() { 0.0 } else { charged / charged_months.len() as f64 },
            interest_months: charged_months.len(),
        });
    } else {
        // without a margin account: the trailing twelve months, averaged over
        // the months that paid
        let since = shift_date(today, -365);
        let window: Vec<&CashflowRow> = recs.iter().copied().filter(|r| r.date > since && r.date.as_str() <= today).collect();
        let months_paid = window.iter().map(|r| month_of(&r.date)).collect::<BTreeSet<_>>().len();
        tiles.push(paid_over("Last 12 months".into(), &window, months_paid));
    }
    tiles.push(CashflowTile::Yield { label: "Yield on cost", r#yield: (basis_all != 0.0).then(|| annual_all / basis_all), projected: annual_all / 12.0, earned: earned_all + 0.0, book: basis_all + 0.0 });

    let other: Vec<&CashflowRow> = everything.iter().copied().filter(|r| r.kind != Payment::Dividend).collect();
    Cashflow {
        tiles,
        months,
        holdings,
        total: total + 0.0,
        count: recs.len(),
        skipped_filters: f.unread_by_cashflow(),
        interest: other.iter().filter(|r| r.kind == Payment::Interest).map(|r| r.amount_cad).fsum(),
        withholding: other.iter().filter(|r| r.kind == Payment::WithholdingTax).map(|r| r.amount_cad).fsum(),
        rows: recs.into_iter().cloned().collect(),
        other: other.into_iter().cloned().collect(),
    }
}

/// Eleven months back from a date, as `YYYY-MM`.
fn trailing_year_month(day: &str) -> String {
    let y: i64 = day[..4].parse().unwrap_or(0);
    let m: i64 = day[5..7].parse().unwrap_or(1);
    let mut cm = m - 11;
    let mut cy = y;
    while cm <= 0 {
        cm += 12;
        cy -= 1;
    }
    format!("{:04}-{:02}", cy, cm)
}

/// Whose legs and fills a view carries. They are most of the payload, so the
/// page is sent them for the one trade or holding it has open.
#[derive(Clone, Copy, Debug)]
pub enum Detail<'a> {
    /// Every row's: the view as the model builds it, for the cases and the tests.
    All,
    /// No row's, or only the row with this id.
    Only(Option<&'a str>),
}

impl Detail<'_> {
    fn keeps(&self, id: &str) -> bool {
        match self {
            Detail::All => true,
            Detail::Only(open) => *open == Some(id),
        }
    }
}

/// The view with every row's detail.
pub fn build_view(base: &Base, filters: Option<&Value>) -> View {
    view_of(base, filters, Detail::All)
}

/// The view as the page is sent it.
pub fn view_of(base: &Base, filters: Option<&Value>, detail: Detail) -> View {
    let f = clean_filters(filters);
    let today = base.today.as_str();

    let trades: Vec<&Trade> = base.trades.iter().filter(|t| trade_matches(t, &f, today)).collect();
    // performance stats score only trades with a known entry basis: a deposited
    // (transferred-in) coin has no buy made here and cannot be scored
    let scored: Vec<&Trade> = trades.iter().copied().filter(|t| !t.flags.contains(&Flag::BasisUnknown)).collect();
    let positions: Vec<&Position> = base.positions.iter().filter(|p| position_matches(p, &f)).collect();

    let one_account = match f.lists.account.as_slice() {
        [name] => base.equity_by_account.get(name).map(|series| (series, name.clone())),
        _ => None,
    };
    let (series, series_label) = one_account.unwrap_or((&base.equity, "All accounts".to_string()));

    let no_benchmark = BTreeMap::new();
    let years = yearly_returns(series, base.benchmarks.get(&f.benchmark).unwrap_or(&no_benchmark), today);
    let portfolio = portfolio_view(base, &f, &positions);

    let mut shown: Vec<&Point> = match date_bounds(&f, today) {
        Some((lo, hi)) => series.iter().filter(|p| lo <= p.d && p.d <= hi).collect(),
        None if !f.years.is_empty() => series.iter().filter(|p| f.years.contains(&p.d.chars().take(4).collect::<String>())).collect(),
        None => series.iter().collect(),
    };
    if !shown.is_empty() {
        // the pre-history a chart should not start from
        let peak = shown.iter().map(|p| p.v).fold(f64::NEG_INFINITY, f64::max);
        let first = shown.iter().position(|p| p.v > peak * 0.01).unwrap_or(0);
        shown = shown.split_off(first);
    }

    let sorted = |mut values: Vec<String>| {
        values.sort();
        values.dedup();
        values
    };
    // what the ⌘K list shows beside each symbol: its name, exchange and kind,
    // from the rows that carry it (a name that is only the symbol counts as none)
    let mut listings: Ordered<ListingInfo> = Ordered::default();
    let rows_of_the_book = base.trades.iter().map(|t| (&t.symbol, &t.name, &t.exchange, t.kind, &t.currency)).chain(base.positions.iter().map(|p| (&p.symbol, &p.name, &p.exchange, p.kind, &p.currency)));
    for (symbol, name, exchange, kind, currency) in rows_of_the_book {
        let known = listings.entry(symbol, || ListingInfo { name: String::new(), exchange: String::new(), kind, currency: currency.clone() });
        if known.name.is_empty() && !name.is_empty() && name != symbol {
            known.name = name.clone();
        }
        if known.exchange.is_empty() && !exchange.is_empty() {
            known.exchange = exchange.clone();
        }
    }
    let mut year_options = sorted(base.trades.iter().filter(|t| !t.exit_date.is_empty()).map(|t| t.exit_date.chars().take(4).collect()).collect());
    year_options.reverse();
    let options = Options {
        accounts: sorted(base.trades.iter().map(|t| t.account.clone()).chain(base.positions.iter().map(|p| p.account.clone())).chain(base.cashflow.iter().map(|r| r.account.clone())).collect()),
        symbols: sorted(base.trades.iter().map(|t| t.symbol.clone()).chain(base.positions.iter().map(|p| p.symbol.clone())).collect()),
        listings,
        tags: sorted(base.trades.iter().flat_map(|t| t.tags.iter().cloned()).collect()),
        exchanges: sorted(base.trades.iter().map(|t| t.exchange.clone()).chain(base.positions.iter().map(|p| p.exchange.clone())).filter(|e| !e.is_empty()).collect()),
        kinds: Kind::ALL.into_iter().filter(|k| base.trades.iter().any(|t| t.kind == *k) || base.positions.iter().any(|p| p.kind == *k)).collect(),
        grades: GRADES.iter().copied().chain(["Ungraded"]).collect(),
        sides: ["SELL", "COVER"],
        results: ["Winners", "Losers", "Breakeven"],
        years: year_options,
    };

    let cashflow = cashflow_view(base, &f, &base.positions, portfolio.margin_used, portfolio.has_margin);
    let without_detail = |legs: &mut Option<_>, fills: &mut Option<_>, id: &str| {
        if !detail.keeps(id) {
            *legs = None;
            *fills = None;
        }
    };
    View {
        ok: true,
        today: base.today.clone(),
        synced_at: base.synced_at.clone(),
        currency: "CAD",
        market: MarketDates { fx_last: base.fx_last.clone(), benchmark_last: base.benchmark_last.clone() },
        options,
        kpi: metrics(&scored),
        equity: EquityBlock { label: series_label, series: shown.into_iter().cloned().collect(), drawdown: drawdown(series), annualized: annualized(&years) },
        years,
        benchmark: BenchmarkRef { label: BENCHMARK_LABELS.iter().find(|(k, _)| *k == f.benchmark).map(|(_, v)| *v).unwrap_or(""), key: f.benchmark.clone() },
        monthly: monthly(&scored),
        by_symbol: by_symbol(&scored),
        grades: grade_buckets(&trades),
        queue: review_queue(&trades),
        trade_count: trades.len(),
        trade_total: base.trades.len(),
        positions_summary: PositionsSummary {
            count: positions.len(),
            book: positions.iter().map(|p| p.cost.abs()).fsum() + 0.0,
            mv: positions.iter().map(|p| signed_mv(p)).fsum() + 0.0,
            unreal: positions.iter().map(|p| p.unreal).fsum() + 0.0,
        },
        markets: crate::markets::markets_view(base, &positions),
        trades: trades
            .into_iter()
            .map(|t| {
                let mut t = t.clone();
                without_detail(&mut t.legs, &mut t.fills, &t.id.clone());
                t
            })
            .collect(),
        positions: positions
            .into_iter()
            .map(|p| {
                let mut p = p.clone();
                if !detail.keeps(&p.id) {
                    p.fills = None;
                }
                p
            })
            .collect(),
        portfolio,
        cashflow,
        unmatched: base.book.fifo.unmatched.clone(),
        accounts: (*base.accounts).clone(),
        activity_count: base.activity_count,
        filters: f,
    }
}

/// The legs and fills of one trade or holding, by id.
pub fn trade_detail(base: &Base, id: &str) -> Option<TradeDetail> {
    let of_trade = base.trades.iter().find(|t| t.id == id).map(|t| (t.legs.clone(), t.fills.clone()));
    let of_holding = || base.positions.iter().find(|p| p.id == id).map(|p| (None, p.fills.clone()));
    let (legs, fills) = of_trade.or_else(of_holding)?;
    Some(TradeDetail { id: id.to_string(), legs: legs.unwrap_or_default(), fills: fills.unwrap_or_default() })
}
