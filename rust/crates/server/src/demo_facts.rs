//! `bagholder demo-facts <data folder>`: the made-up book (`store/src/bin/demo_book.rs`)
//! carried into a book, with the facts and prices its engine reads written beside
//! it (`docs/plans/stage-3c-switch.md`, open question 1): the Bank's rates for the
//! book's span, each account's stated value and cash, a quote for each listing,
//! each payer's declared record and schedule, and the benchmarks' trackers. So
//! the browser tests and the screenshots, which run offline, have figures to
//! show. Nothing in it is anyone's; every value is made up here.

use std::path::Path;

use bagholder_book::statements::AccountDay;
use bagholder_book::Book;
use bagholder_core::account::AccountRef;
use bagholder_core::jiff::civil::{Date, Weekday};
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::transaction::Kind;
use bagholder_core::{Broker, Currency, Dec, InstrumentId, Money, SourceName};
use bagholder_sources::cache::{MarketCache, StoredQuote};
use bagholder_sources::contract::Benchmark;

/// The demo's today and the moment it was read, as `demo_book.rs` states them.
const TODAY: &str = "2026-09-08";
const SYNCED: &str = "2026-09-08T20:05:00Z";

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn weekday(d: Date) -> bool {
    !matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday)
}

fn days(from: Date, to: Date) -> impl Iterator<Item = Date> {
    let mut d = Some(from);
    std::iter::from_fn(move || {
        let now = d?;
        if now > to {
            return None;
        }
        d = now.tomorrow().ok();
        Some(now)
    })
}

/// A made-up value that moves smoothly with the day: `base` ± `swing`, to `places`.
fn wave(base: f64, swing: f64, d: Date, period: f64, places: u32) -> Dec {
    let n = (d - Date::constant(2024, 1, 1)).get_days() as f64;
    let v = base + swing * (n / period).sin() + swing * 0.3 * (n / (period / 3.7)).cos();
    Dec::parse(&format!("{v:.p$}", p = places as usize)).expect("a made-up value is a decimal")
}

pub fn write(home: &Path) -> Result<String, String> {
    let at: Timestamp = SYNCED.parse().map_err(err)?;
    let today: Date = TODAY.parse().map_err(err)?;
    if !home.join(bagholder_book::BOOK_FILE).exists() {
        crate::legacy_import::import(&home.join(crate::figures::OLD_FILE), home, at)?;
    }
    let (book, _) = Book::open_in(home, crate::app::APP_VERSION, at).map_err(err)?;
    let (cache, _) = MarketCache::open(&home.join(crate::figures::CACHE_FILE), crate::app::APP_VERSION, at).map_err(err)?;
    // the zone the browser tests' pages run in, as a page states it: the figures are
    // built from the first start, before any page has opened
    book.state_zone("America/Toronto", at).map_err(err)?;
    let bank = SourceName::named("bank-of-canada");
    let demo = SourceName::named("demo");

    // the Bank's rate for every currency the book holds but CAD, each business day of its span
    let first = book.transactions().map_err(err)?.iter().map(|t| t.trade_date).min().unwrap_or(today);
    let span_from = first.checked_sub(SignedDuration::from_hours(24 * 30)).map_err(err)?;
    // the rates reach the day the demo is run on, so a value held today converts
    let rates_to = Timestamp::now().to_zoned(bagholder_core::jiff::tz::TimeZone::get("America/Toronto").map_err(err)?).date().max(today);
    let currencies: std::collections::BTreeSet<Currency> = book.instruments().map_err(err)?.iter().map(|i| i.currency).filter(|c| *c != Currency::CAD).collect();
    for c in &currencies {
        let rates: Vec<(Date, Dec)> = days(span_from, rates_to).filter(|d| weekday(*d)).map(|d| (d, wave(1.37, 0.018, d, 45.0, 4))).collect();
        book.store_rates(*c, &rates, (span_from, rates_to), &bank, at).map_err(err)?;
        book.store_rate_series(&[bagholder_book::facts::RateSeries { currency: *c, source: bank.clone(), first_day: span_from, last_day: rates_to, ended: false }], at).map_err(err)?;
    }

    // each contract's size, as the made-up contracts state it
    drop(book);
    rusqlite::Connection::open(home.join(bagholder_book::BOOK_FILE))
        .and_then(|c| c.execute("UPDATE option_terms SET multiplier = '100', source = 'demo' WHERE multiplier IS NULL", []))
        .map_err(err)?;
    let (book, _) = Book::open_in(home, crate::app::APP_VERSION, at).map_err(err)?;

    // each account's value and net deposits, a share of the book's, and its cash
    let old = rusqlite::Connection::open(home.join(crate::figures::OLD_FILE)).map_err(err)?;
    let nav: Vec<(String, f64, f64)> = old
        .prepare("SELECT date, equity, net_deposits FROM nav_history WHERE equity IS NOT NULL ORDER BY date")
        .map_err(err)?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, Option<f64>>(2)?.unwrap_or(0.0))))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let ws = Broker::named("wealthsimple");
    let connection = book.connections().map_err(err)?.into_iter().find(|c| c.broker == ws).map(|c| c.id).ok_or("the demo book has no Wealthsimple connection")?;
    let shares = [("acct-tfsa", 0.38), ("acct-rrsp", 0.41), ("acct-trading", 0.08), ("acct-crypto", 0.13)];
    let cash = [("acct-tfsa", "4210.35", Currency::CAD), ("acct-rrsp", "1875.00", Currency::CAD), ("acct-trading", "-18240.60", Currency::USD), ("acct-crypto", "312.40", Currency::CAD)];
    for (ws_id, share) in shares {
        let Some(account) = book.account_by_ref(&AccountRef::new(ws.clone(), ws_id)).map_err(err)? else { continue };
        let read = book.broker_read(connection, &format!("history:{ws_id}"), at).map_err(err)?;
        let cad = |v: f64| Money::new(Dec::parse(&format!("{:.2}", v * share)).expect("a value to the cent"), Currency::CAD);
        let account_days: Vec<AccountDay> = nav.iter().filter_map(|(d, v, dep)| Some(AccountDay { day: d.parse().ok()?, net_value: cad(*v), net_deposits: cad(*dep) })).collect();
        book.store_account_days(account, &account_days, &read).map_err(err)?;
        if let Some((_, amount, currency)) = cash.iter().find(|c| c.0 == ws_id) {
            let read = book.broker_read(connection, &format!("balances:{ws_id}"), at).map_err(err)?;
            book.store_cash(account, at, &[(*currency, Dec::parse(amount).map_err(err)?)].into_iter().collect(), &read).map_err(err)?;
        }
        book.note_activity_read(account, at, true).map_err(err)?;
    }

    // a quote for each listing the book holds a record of, from its demo bars
    let mut quoted = 0;
    for i in book.instruments().map_err(err)? {
        let Some(symbol) = book.names(i.id).map_err(err)?.last().map(|n| n.symbol.clone()) else { continue };
        let closes: Vec<f64> = old
            .prepare("SELECT close FROM price_history WHERE symbol = ? AND date <= ? ORDER BY date DESC LIMIT 2")
            .map_err(err)?
            .query_map(rusqlite::params![symbol, TODAY], |r| r.get::<_, f64>(0))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        let (Some(last), prev) = (closes.first(), closes.get(1)) else { continue };
        let price = Dec::parse(&format!("{last:.2}")).map_err(err)?;
        let change = prev.map(|p| Dec::parse(&format!("{:.2}", last - p))).transpose().map_err(err)?;
        let change_pct = prev.filter(|p| **p != 0.0).map(|p| Dec::parse(&format!("{:.2}", (last / p - 1.0) * 100.0))).transpose().map_err(err)?;
        cache
            .store_quote(&StoredQuote { instrument: i.id, source: demo.clone(), price: Money::new(price, i.currency), change, change_pct, quoted_at: at, allowance: std::time::Duration::ZERO, received_at: at })
            .map_err(err)?;
        quoted += 1;
    }

    // each payer's record: every dividend it paid, declared a week before it was paid,
    // quarterly as the demo pays
    let mut paid: std::collections::BTreeMap<InstrumentId, Vec<(Date, Dec, Currency)>> = Default::default();
    let all = book.transactions().map_err(err)?;
    // units the account held of the instrument at the end of `day`, from its buys and sales
    let held = |account: bagholder_core::AccountId, i: InstrumentId, day: Date| -> Dec {
        all.iter()
            .filter(|t| t.account == account && t.instrument == Some(i) && t.trade_date <= day && t.kind != Kind::Dividend)
            .filter_map(|t| t.quantity)
            .fold(Dec::ZERO, |a, q| a.checked_add(q).unwrap_or(a))
    };
    for t in &all {
        if t.kind != Kind::Dividend {
            continue;
        }
        let (Some(i), Some(cash)) = (t.instrument, t.cash) else { continue };
        let qty = held(t.account, i, t.trade_date);
        if !qty.is_positive() {
            continue;
        }
        let per = cash.amount.div_rounded(qty, 6, bagholder_core::Rounding::HalfEven).map_err(err)?;
        paid.entry(i).or_default().push((t.trade_date, per, cash.currency));
    }
    for (i, rows) in &paid {
        let items: Vec<bagholder_book::facts::DeclaredRow> = rows
            .iter()
            .filter_map(|(pay, per, currency)| {
                let ex = pay.checked_sub(SignedDuration::from_hours(24 * 7)).ok()?;
                Some(bagholder_book::facts::DeclaredRow { form: bagholder_core::distribution::Form::Stated, ex_date: ex, record_date: Some(ex), pay_date: Some(*pay), amount: Money::new(*per, *currency), reinvested: None })
            })
            .collect();
        book.store_declared(*i, &items, &demo, at).map_err(err)?;
        book.store_frequency(*i, 4, &demo, Some(today), at).map_err(err)?;
    }

    // the benchmarks' trackers, each a steady climb
    for (b, base) in [(Benchmark::Sp500, 470.0), (Benchmark::Tsx, 32.0), (Benchmark::Tx60, 33.0)] {
        let closes: Vec<(Date, Dec)> = days(span_from, today)
            .filter(|d| weekday(*d))
            .map(|d| {
                let n = (d - span_from).get_days() as f64;
                (d, Dec::parse(&format!("{:.2}", base * (1.0 + n * 0.0004) * (1.0 + 0.02 * (n / 23.0).sin()))).expect("a close to the cent"))
            })
            .collect();
        cache.store_benchmark_closes(b, &closes, &demo, at).map_err(err)?;
    }

    Ok(format!("demo facts: {} currencies, {} quotes, {} payers\n", currencies.len(), quoted, paid.len()))
}

pub fn cli(args: &[String]) -> i32 {
    let [home] = args else {
        eprintln!("usage: bagholder demo-facts <data folder holding the demo's bagholder.db>");
        return 2;
    };
    match write(Path::new(home)) {
        Ok(report) => {
            print!("{report}");
            0
        }
        Err(e) => {
            eprintln!("the demo's facts could not be written: {e}");
            1
        }
    }
}
