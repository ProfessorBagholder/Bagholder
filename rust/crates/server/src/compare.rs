//! `bagholder compare-figures <old database> <book folder> [today]`: the old model
//! and the new engine on the same data, every figure that differs listed with
//! what the new engine says of it (`docs/plans/stage-2-engine.md`, "The
//! comparison on the person's data").
//!
//! The book is the one `import-book` made from the old database; it is derived
//! again with the import's current mapping. The market data (the USD rate, the
//! funds' declared distributions, the quotes and the benchmarks) is read from the
//! old store as a stand-in for the facts stage 3's readers will write, and is
//! named as such in the report. Both databases are read from copies; the book
//! folder given is written (its trades are opened), so it must be a copy too.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use rusqlite::Connection;

use bagholder_book::import::mapping::ImportMapping;
use bagholder_book::Book;
use bagholder_core::instrument::InstrumentKind;
use bagholder_core::journal::Opening;
use bagholder_core::{Currency, Dec, InstrumentId, Money, RecordId, TransactionId};
use bagholder_engine::input::{Clock, Declared, DeclaredRead, DistributionKind, Inputs, Market, Quote, QuoteSource, Sourced};
use bagholder_engine::scope::Filters;
use bagholder_engine::trades::TradeKey;
use bagholder_engine::{Change, Engine};

use crate::engine_inputs::{self, old_store_source};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A float the old store kept, as the decimal it prints as: a stand-in value
/// only, never written to the book.
fn dec(v: f64) -> Option<Dec> {
    if !v.is_finite() {
        return None;
    }
    Dec::parse(&format!("{v}")).ok()
}

/// The market as the old store holds it, keyed to the book's instruments by
/// their current symbol (a symbol two instruments share keys neither).
fn old_market(old: &Connection, book: &Book, ledger: &bagholder_engine::input::Ledger) -> Result<(Market, bagholder_engine::input::Rates, BTreeMap<InstrumentId, DeclaredRead>, BTreeMap<InstrumentId, Sourced<u32>>), String> {
    let mut by_symbol: BTreeMap<String, Vec<InstrumentId>> = BTreeMap::new();
    for (id, info) in &ledger.instruments {
        if let Some(n) = info.current_name() {
            by_symbol.entry(n.symbol.clone()).or_default().push(*id);
        }
    }
    let one = |symbol: &str| -> Option<InstrumentId> {
        match by_symbol.get(symbol).map(|v| v.as_slice()) {
            Some([only]) => Some(*only),
            _ => None,
        }
    };
    let mut market = Market::default();
    let mut frequencies = BTreeMap::new();
    let mut stmt = old.prepare("SELECT symbol, price, price_change, percent_change, source, dividend_frequency FROM quotes").map_err(err)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<f64>>(1)?, r.get::<_, Option<f64>>(2)?, r.get::<_, Option<f64>>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?)))
        .map_err(err)?;
    for row in rows {
        let (symbol, price, change, pct, source, freq) = row.map_err(err)?;
        let Some(i) = one(&symbol) else { continue };
        let info = &ledger.instruments[&i];
        let source = match (source.as_deref().unwrap_or(""), info.instrument.kind) {
            ("coinbase", _) => QuoteSource::Crypto,
            ("cboe_options", _) => QuoteSource::OptionChain,
            ("", InstrumentKind::Crypto) => QuoteSource::Crypto,
            ("", InstrumentKind::OptionContract) => QuoteSource::OptionChain,
            _ => QuoteSource::Listing,
        };
        if let Some(p) = price.and_then(dec) {
            market.quotes.insert(i, Quote { price: Money::new(p, info.instrument.currency), change: change.and_then(dec), change_pct: pct.and_then(dec), at: None, source });
        }
        let per_year = match freq.as_deref().map(str::to_lowercase).as_deref() {
            Some("monthly") => Some(12),
            Some("quarterly") => Some(4),
            Some("semi-annual" | "semiannual" | "semi-annually") => Some(2),
            Some("annual" | "annually") => Some(1),
            Some("weekly") => Some(52),
            Some("bi-weekly" | "biweekly") => Some(26),
            _ => None,
        };
        if let Some(n) = per_year {
            frequencies.insert(i, Sourced { value: n, source: old_store_source() });
        }
    }
    let mut closes: BTreeMap<InstrumentId, BTreeMap<bagholder_core::jiff::civil::Date, Dec>> = BTreeMap::new();
    let mut stmt = old.prepare("SELECT symbol, date, close FROM price_history").map_err(err)?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?))).map_err(err)? {
        let (symbol, date, close) = row.map_err(err)?;
        let (Some(i), Ok(d), Some(c)) = (one(&symbol), date.get(..10).unwrap_or("").parse(), dec(close)) else { continue };
        closes.entry(i).or_default().insert(d, c);
    }
    market.closes = closes;
    let mut stmt = old.prepare("SELECT symbol, date, close FROM benchmark_prices").map_err(err)?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?))).map_err(err)? {
        let (symbol, date, close) = row.map_err(err)?;
        let (Ok(d), Some(c)) = (date.get(..10).unwrap_or("").parse(), dec(close)) else { continue };
        market.benchmarks.entry(symbol).or_default().insert(d, c);
    }
    let mut rates = bagholder_engine::input::Rates::default();
    let mut stmt = old.prepare("SELECT date, rate FROM fx_rates WHERE pair = 'USDCAD' ORDER BY date").map_err(err)?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))).map_err(err)? {
        let (date, rate) = row.map_err(err)?;
        if let (Ok(d), Some(r)) = (date.parse(), dec(rate)) {
            rates.by_currency.entry(Currency::USD).or_default().insert(d, r);
        }
    }
    if let Some(s) = rates.by_currency.get(&Currency::USD) {
        // the old table was appended from reads of the Bank's series; it keeps
        // no record of them. A span is taken as read only where its stored days
        // lie no further apart than the Bank's longest closure (Christmas, Boxing
        // Day and a weekend: five days from one published day to the next), so a
        // longer hole is a rate missing, never a run of holidays
        let at = bagholder_core::jiff::Timestamp::now();
        let mut spans: Vec<bagholder_engine::input::Read> = Vec::new();
        for d in s.keys().copied() {
            match spans.last_mut() {
                Some(r) if (d - r.last).get_days() <= 5 => r.last = d,
                _ => spans.push(bagholder_engine::input::Read { first: d, last: d, at }),
            }
        }
        rates.covered.insert(Currency::USD, spans);
        rates.published.insert(Currency::USD);
    }
    let mut declared: BTreeMap<InstrumentId, Vec<Declared>> = BTreeMap::new();
    let mut stmt = old.prepare("SELECT symbol, ex_date, pay_date, amount, currency FROM distributions").map_err(err)?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, f64>(3)?, r.get::<_, Option<String>>(4)?))).map_err(err)? {
        let (symbol, ex, pay, amount, currency) = row.map_err(err)?;
        let Some(i) = one(&symbol) else { continue };
        let currency = currency.as_deref().and_then(|c| Currency::parse(c).ok()).unwrap_or(ledger.instruments[&i].instrument.currency);
        let (Ok(ex), Some(a)) = (ex.get(..10).unwrap_or("").parse(), dec(amount)) else { continue };
        declared.entry(i).or_default().push(Declared {
            ex_date: ex,
            record_date: None,
            pay_date: pay.as_deref().and_then(|p| p.get(..10)).and_then(|p| p.parse().ok()),
            amount: Money::new(a, currency),
            kind: DistributionKind::Regular,
        });
    }
    let read_at = bagholder_core::jiff::Timestamp::UNIX_EPOCH;
    let declared = declared.into_iter().map(|(i, items)| (i, DeclaredRead { read_at, source: old_store_source(), items })).collect();
    let _ = book;
    Ok((market, rates, declared, frequencies))
}

fn money(m: &Result<Money, bagholder_engine::gap::Gaps>) -> String {
    match m {
        Ok(v) => v.amount.round(2, bagholder_core::Rounding::HalfEven).to_text(),
        Err(g) => format!("— ({})", g.words().join(", ")),
    }
}

fn close_enough(old: f64, new: &Result<Money, bagholder_engine::gap::Gaps>) -> bool {
    match new {
        Ok(m) => (m.amount.to_f64() - old).abs() < 0.005,
        Err(_) => false,
    }
}

pub fn compare(old_path: &Path, book_dir: &Path, today: bagholder_core::jiff::civil::Date) -> Result<String, String> {
    let at = bagholder_core::jiff::Timestamp::now();
    let scratch = std::env::temp_dir().join(format!("bh-compare-{}", at.as_millisecond()));
    std::fs::create_dir_all(&scratch).map_err(err)?;
    bagholder_book::import::copy_database(old_path, &scratch.join("bagholder.db")).map_err(err)?;
    let old = bagholder_store::connect(&scratch).map_err(err)?;
    bagholder_store::schema::init_schema(&old).map_err(err)?;
    let today_text = today.to_string();
    let base = crate::model_cache::ModelCache::new().base(&old, &today_text).map_err(err)?;
    let view = bagholder_model::view::build_view(&base, None);

    let (book, _) = Book::open_in(book_dir, crate::app::APP_VERSION, at).map_err(err)?;
    let changes = book.rederive(&ImportMapping, at).map_err(err)?;
    let mut ledger = engine_inputs::ledger(&book)?;
    let mut facts = engine_inputs::facts(&book)?;
    let (market, rates, declared, frequencies) = old_market(&old, &book, &ledger)?;
    facts.rates = rates;
    facts.declared = declared;
    facts.frequencies = frequencies;
    let bank = bagholder_core::jiff::tz::TimeZone::get("America/Toronto").map_err(err)?;
    let home = bagholder_core::jiff::tz::TimeZone::system();
    // the end of the day in the Bank's zone: every rate of the day is out
    let now = today.at(23, 0, 0, 0).to_zoned(bank.clone()).map_err(err)?.timestamp();
    let clock = Clock { today, now, home, bank };
    ledger.trades = book.trades().map_err(err)?;
    let mut engine = Engine::build(Inputs { ledger, facts, market, clock });
    for key in engine.identity().needs_trade.clone() {
        book.open_trade(&Opening { transaction: key.opening.clone(), instrument: key.instrument }, None, at).map_err(err)?;
    }
    for (joined, keeps) in engine.identity().joined.clone() {
        book.orphan_joined(joined, keeps).map_err(err)?;
    }
    let unclaimed = engine.identity().unclaimed.clone();
    for trade in &unclaimed {
        book.orphan_unclaimed(*trade).map_err(err)?;
    }
    engine.apply(Change::Trades(book.trades().map_err(err)?));

    // the old row each transaction came from
    let keys = book.live_record_keys().map_err(err)?;
    let old_row = |t: &TransactionId| -> String { keys.get(&t.record).cloned().unwrap_or_default() };
    let figures = engine.figures();
    let mut out = String::new();
    writeln!(out, "Compared on {today}. Stand-ins read from the old store: the USD rate, declared distributions, stated frequencies (TMX's quote field), quotes, closes, benchmarks.").ok();
    writeln!(out, "Re-derived with the import mapping: {} transactions changed, {} added, {} removed.", changes.changed.len(), changes.added.len(), changes.removed.len()).ok();
    let mut changed_kinds: BTreeMap<String, usize> = BTreeMap::new();
    for id in &changes.changed {
        if let Some(t) = engine_inputs_transaction(&book, id) {
            *changed_kinds.entry(t.kind.as_str().to_string()).or_default() += 1;
        }
    }
    writeln!(out, "  changed, by kind now: {changed_kinds:?}").ok();

    // trades, by the old key: a round trip is named for the row that opened it
    let mut new_by_old: BTreeMap<String, &bagholder_engine::trades::TradeFig> = BTreeMap::new();
    for t in figures.trades {
        let key = match &t.key {
            TradeKey::Trip(k) => format!("rt:{}", old_row(&k.opening)),
            TradeKey::Group(g) => format!("group:{g}"),
        };
        new_by_old.insert(key, t);
    }
    let mut causes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut same = 0usize;
    let mut seen_new: BTreeSet<String> = BTreeSet::new();
    for o in &view.trades {
        let key = o.id.split('|').next().unwrap_or(&o.id).to_string();
        match new_by_old.get(&key) {
            None => {
                causes.entry("old trade with no new trade on the same opening".into()).or_default().push(format!("{} {} {} → {} pnl {:.2}", o.id, o.symbol, o.entry_date, o.exit_date, o.pnl));
            }
            Some(n) => {
                seen_new.insert(key.clone());
                let pnl_ok = close_enough(o.pnl, &n.pnl);
                let cad_ok = close_enough(o.pnl_cad, &n.pnl_cad);
                let qty_ok = n.qty.as_ref().is_ok_and(|q| (q.to_f64() - o.qty).abs() < 1e-6);
                if pnl_ok && cad_ok && qty_ok {
                    same += 1;
                    continue;
                }
                let cause = if !n.gaps().is_empty() {
                    format!("new figure waits: {}", n.gaps().words().join(", "))
                } else if !qty_ok {
                    "matched a different quantity".to_string()
                } else if !pnl_ok {
                    "a different P&L in its own currency".to_string()
                } else {
                    "a different P&L in CAD".to_string()
                };
                causes.entry(cause).or_default().push(format!(
                    "{} {} qty {} / {} pnl {:.2} / {} cad {:.2} / {} flags old {:?} new {:?}",
                    key,
                    o.symbol,
                    o.qty,
                    n.qty.as_ref().map(|q| q.to_text()).unwrap_or_else(|g| format!("— ({})", g.words().join(", "))),
                    o.pnl,
                    money(&n.pnl),
                    o.pnl_cad,
                    money(&n.pnl_cad),
                    o.flags,
                    n.flags
                ));
            }
        }
    }
    for (key, n) in &new_by_old {
        if !seen_new.contains(key) {
            causes.entry("new trade the old model did not have".into()).or_default().push(format!("{key} pnl {} flags {:?} gaps {:?}", money(&n.pnl), n.flags, n.gaps().words()));
        }
    }
    writeln!(out, "\nTrades: {} old, {} new, {} the same to the cent.", view.trades.len(), figures.trades.len(), same).ok();
    for (cause, items) in &causes {
        writeln!(out, "  {} — {}", cause, items.len()).ok();
        for i in items.iter().take(8) {
            writeln!(out, "      {i}").ok();
        }
    }

    // realized P&L per instrument, whatever the round trips' bounds: the
    // arithmetic, checked apart from where each model starts and ends a trade
    let mut old_by_symbol: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for o in &view.trades {
        let e = old_by_symbol.entry(o.symbol.clone()).or_default();
        e.0 += o.pnl;
        e.1 += 1;
    }
    let mut new_by_symbol: BTreeMap<String, (Option<Dec>, usize, BTreeSet<&'static str>)> = BTreeMap::new();
    for t in figures.trades {
        let symbol = figures_symbol(&engine, t.instrument);
        let e = new_by_symbol.entry(symbol).or_insert((Some(Dec::ZERO), 0, BTreeSet::new()));
        e.1 += 1;
        match &t.pnl {
            Ok(p) => e.0 = e.0.and_then(|x| x.checked_add(p.amount).ok()),
            Err(g) => {
                e.0 = None;
                e.2.extend(g.words());
            }
        }
    }
    let symbols: BTreeSet<&String> = old_by_symbol.keys().chain(new_by_symbol.keys()).collect();
    let (mut agree, mut differ) = (0usize, Vec::new());
    for s in symbols {
        let old = old_by_symbol.get(s).copied().unwrap_or((0.0, 0));
        let new = new_by_symbol.get(s).cloned().unwrap_or((Some(Dec::ZERO), 0, BTreeSet::new()));
        match new.0 {
            Some(n) if (n.to_f64() - old.0).abs() < 0.005 => agree += 1,
            Some(n) => differ.push(format!("{s}: old {:.2} over {} trades, new {} over {}", old.0, old.1, n.round(2, bagholder_core::Rounding::HalfEven).to_text(), new.1)),
            None => differ.push(format!("{s}: old {:.2} over {} trades, new waits ({})", old.0, old.1, new.2.iter().copied().collect::<Vec<_>>().join(", "))),
        }
    }
    writeln!(out, "\nRealized P&L by instrument: {agree} agree to the cent; {} differ.", differ.len()).ok();
    for d in &differ {
        writeln!(out, "      {d}").ok();
    }

    // positions, by the old key
    let mut new_pos: BTreeMap<String, &bagholder_engine::positions::PositionFig> = BTreeMap::new();
    for p in figures.positions {
        new_pos.insert(format!("rt:{}", old_row(&p.key.opening)), p);
    }
    let mut pos_causes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut pos_same = 0usize;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for o in &view.positions {
        let key = o.rt.clone().unwrap_or_else(|| o.id.clone());
        match new_pos.get(&key) {
            None => pos_causes.entry("old position with no new position on the same opening".into()).or_default().push(format!("{key} {} qty {} book {:.2}", o.symbol, o.qty, o.cost)),
            Some(n) => {
                seen.insert(key.clone());
                let qty_ok = (n.qty.to_f64() - o.qty).abs() < 1e-6;
                let book_ok = close_enough(o.cost.abs(), &n.book);
                if qty_ok && book_ok {
                    pos_same += 1;
                    continue;
                }
                let cause = if !n.gaps.is_empty() { format!("new position waits: {}", n.gaps.words().join(", ")) } else if !qty_ok { "a different quantity".into() } else { "a different book value".into() };
                pos_causes.entry(cause).or_default().push(format!("{key} {} qty {} / {} book {:.2} / {}", o.symbol, o.qty, n.qty, o.cost, money(&n.book)));
            }
        }
    }
    for (key, n) in &new_pos {
        if !seen.contains(key) {
            let symbol = figures_symbol(&engine, n.instrument);
            pos_causes.entry("new position the old model did not have".into()).or_default().push(format!("{key} {symbol} qty {} book {} gaps {:?}", n.qty, money(&n.book), n.gaps.words()));
        }
    }
    writeln!(out, "\nPositions: {} old, {} new, {} the same.", view.positions.len(), figures.positions.len(), pos_same).ok();
    for (cause, items) in &pos_causes {
        writeln!(out, "  {} — {}", cause, items.len()).ok();
        for i in items.iter().take(12) {
            writeln!(out, "      {i}").ok();
        }
    }

    // payments, by the old row
    let new_cash: BTreeMap<String, &bagholder_engine::cashflow::CashRow> = figures.cash.iter().map(|r| (old_row(&r.id), r)).collect();
    let mut cash_diff = Vec::new();
    let mut cash_same = 0;
    for o in view.cashflow.rows.iter().chain(view.cashflow.other.iter()) {
        match new_cash.get(&o.id) {
            None => cash_diff.push(format!("{} {} {:?} {:.2}: no new row", o.id, o.date, o.kind, o.amount)),
            Some(n) if close_enough(o.amount_cad, &n.amount_cad) && (n.amount.amount.to_f64() - o.amount).abs() < 0.005 => cash_same += 1,
            Some(n) => cash_diff.push(format!("{} {} {:?} {:.2} cad {:.2} / {} cad {}", o.id, o.date, o.kind, o.amount, o.amount_cad, n.amount.amount.to_text(), money(&n.amount_cad))),
        }
    }
    writeln!(out, "\nPayments: {} old, {} new, {} the same; {} differ.", view.cashflow.rows.len() + view.cashflow.other.len(), figures.cash.len(), cash_same, cash_diff.len()).ok();
    for d in cash_diff.iter().take(12) {
        writeln!(out, "      {d}").ok();
    }

    // the dashboard, unfiltered
    let scoped = engine.scope(&Filters { benchmark: "SP500".into(), ..Filters::default() });
    writeln!(out, "\nDashboard: realized old {:.2} new {} ({} trades, {} left out); count old {} new {}.", view.kpi.realized, scoped.kpi.realized.amount.round(2, bagholder_core::Rounding::HalfEven).to_text(), scoped.kpi.count, scoped.kpi.left_out, view.kpi.count, scoped.kpi.count).ok();
    writeln!(out, "Portfolio: market value old {:.2} new {} ({} left out); cost basis old {:.2} new {} ({} left out).", view.portfolio.market_value, scoped.portfolio.market_value.total.amount.round(2, bagholder_core::Rounding::HalfEven).to_text(), scoped.portfolio.market_value.left_out, view.portfolio.cost_basis, scoped.portfolio.cost_basis.total.amount.round(2, bagholder_core::Rounding::HalfEven).to_text(), scoped.portfolio.cost_basis.left_out).ok();
    writeln!(out, "Cashflow: all-time dividends old {:.2} new {} ({} left out).", view.cashflow.total, scoped.cashflow.total.total.amount.round(2, bagholder_core::Rounding::HalfEven).to_text(), scoped.cashflow.total.left_out).ok();

    // what the new engine could not apply
    let m = figures.matched;
    let mut waits: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (_, g) in &m.unapplied {
        for w in g.words() {
            *waits.entry(w).or_default() += 1;
        }
    }
    writeln!(out, "\nNot applied by the new engine: {:?}; sold or closed beyond what was held: {}; price and cash disagreeing: {}.", waits, m.beyond.len(), m.disagreements.len()).ok();
    for b in m.beyond.iter().take(12) {
        writeln!(out, "      beyond held: {} {} qty {}", old_row(&b.transaction), figures_symbol(&engine, b.instrument), b.qty.to_text()).ok();
    }
    let _ = std::fs::remove_dir_all(&scratch);
    let _: Option<RecordId> = None;
    Ok(out)
}

fn engine_inputs_transaction(book: &Book, id: &TransactionId) -> Option<bagholder_core::transaction::Transaction> {
    book.transaction(id).ok().flatten()
}

fn figures_symbol(engine: &Engine, i: InstrumentId) -> String {
    engine.inputs().ledger.instruments.get(&i).and_then(|x| x.current_name()).map(|n| n.symbol.clone()).unwrap_or_else(|| i.to_string())
}

/// `bagholder compare-figures <old database> <book folder> [YYYY-MM-DD]`.
pub fn cli(args: &[String]) -> i32 {
    let (old, book, today) = match args {
        [old, book] => (old, book, bagholder_core::jiff::Zoned::now().date()),
        [old, book, day] => match day.parse() {
            Ok(d) => (old, book, d),
            Err(e) => {
                eprintln!("{day} is not a day: {e}");
                return 2;
            }
        },
        _ => {
            eprintln!("usage: bagholder compare-figures <old database> <book folder> [YYYY-MM-DD]");
            return 2;
        }
    };
    match compare(Path::new(old), Path::new(book), today) {
        Ok(report) => {
            println!("{report}");
            0
        }
        Err(e) => {
            eprintln!("the comparison failed: {e}");
            1
        }
    }
}
