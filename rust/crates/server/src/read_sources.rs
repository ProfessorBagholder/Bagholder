//! `bagholder read-sources <book folder> [--cache <market.db>] [--now <instant>]`
//! and `bagholder source-health [--cache <market.db>] [--now <instant>]`
//! (`docs/plans/stage-3a-sources.md`, "The command").
//!
//! `read-sources` asks the engine what the figures use (its needs), runs every
//! reader that is due for it once, and asks again: what one read settles can
//! make another fact needed (an expired contract's underlying's close decides
//! whether it is still held), so the passes go on until the needs stop changing.
//! Each source's outcomes of the run are printed. `source-health` prints each
//! source's state and its last outcome of each kind.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bagholder_book::Book;
use bagholder_core::instrument::{InstrumentKind, RefScheme};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::InstrumentId;
use bagholder_engine::input::{Clock, Inputs, Market, Quote, QuoteSource};
use bagholder_engine::needs::FactNeeds;
use bagholder_engine::Engine;
use bagholder_net::{Limiter, Net, SystemClock};
use bagholder_sources::cache::MarketCache;
use bagholder_sources::contract::Listing;
use bagholder_sources::needs::{CloseNeed, Needs, PayerNeed};
use bagholder_sources::read::Ctx;
use bagholder_sources::{health, market, payers, rates};

use crate::engine_inputs;

/// How many times the needs are worked out again after a pass of reads, at most.
const PASSES: usize = 4;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The market as the cache holds it: its quotes, its closes and the benchmarks.
pub fn market_from_cache(cache: &MarketCache, book: &Book) -> Result<Market, String> {
    let mut m = Market::default();
    let kinds: BTreeMap<InstrumentId, InstrumentKind> = book.instruments().map_err(err)?.into_iter().map(|i| (i.id, i.kind)).collect();
    for q in cache.quotes().map_err(err)? {
        let source = match kinds.get(&q.instrument) {
            Some(InstrumentKind::Crypto) => QuoteSource::Crypto,
            Some(InstrumentKind::OptionContract) => QuoteSource::OptionChain,
            _ => QuoteSource::Listing,
        };
        m.quotes.insert(q.instrument, Quote { price: q.price, change: q.change, change_pct: q.change_pct, at: Some(q.quoted_at), source });
    }
    m.closes = cache.closes().map_err(err)?;
    for (b, levels) in cache.benchmarks().map_err(err)? {
        m.benchmarks.insert(b.key().to_string(), levels);
    }
    Ok(m)
}

/// An instrument as the sources ask for it: its kind and currency, its symbol and
/// venue now, and the ways to ask for it the book holds.
fn listing(book: &Book, id: InstrumentId) -> Result<Option<Listing>, String> {
    let i = book.instrument(id).map_err(err)?;
    let names = book.names(id).map_err(err)?;
    let Some(now) = names.last() else { return Ok(None) };
    let mut routes: BTreeMap<RefScheme, Vec<String>> = BTreeMap::new();
    for r in book.instrument_refs(id).map_err(err)? {
        if !r.identifies() {
            routes.entry(r.scheme).or_default().push(r.value);
        }
    }
    Ok(Some(Listing { id, kind: i.kind, currency: i.currency, symbol: now.symbol.clone(), venue_mic: now.venue_mic.clone(), routes }))
}

/// The engine's needs, as the sources read them.
fn needs_of(book: &Book, n: &FactNeeds) -> Result<Needs, String> {
    let mut out = Needs { benchmarks_from: n.first_day, ..Needs::default() };
    out.rates = n.rates.iter().map(|(c, d)| rates::Need { currency: *c, oldest: *d }).collect();
    for (id, (from, to)) in &n.closes {
        if let Some(listing) = listing(book, *id)? {
            out.closes.push(CloseNeed { listing, from: *from, to: *to });
        }
    }
    for id in &n.payers {
        if let Some(listing) = listing(book, *id)? {
            let name = book.names(*id).map_err(err)?.last().and_then(|n| n.name.clone());
            out.payers.push(PayerNeed { listing, name });
        }
    }
    Ok(out)
}

/// What `now` needs that `before` did not: a currency new or needed from an
/// earlier day, an instrument's closes new or over a wider span, a payer new, and
/// the benchmarks only when the oldest day moved earlier.
fn new_in(now: &FactNeeds, before: &FactNeeds) -> FactNeeds {
    FactNeeds {
        rates: now.rates.iter().filter(|(c, d)| before.rates.get(c).is_none_or(|b| *d < b)).map(|(c, d)| (*c, *d)).collect(),
        closes: now.closes.iter().filter(|(i, (f, t))| before.closes.get(i).is_none_or(|(bf, bt)| f < bf || t > bt)).map(|(i, s)| (*i, *s)).collect(),
        payers: now.payers.difference(&before.payers).copied().collect(),
        first_day: now.first_day.filter(|d| before.first_day.is_none_or(|b| *d < b)),
    }
}

fn engine(book: &Book, cache: &MarketCache, clock: Clock) -> Result<Engine, String> {
    let ledger = engine_inputs::ledger(book)?;
    let facts = engine_inputs::facts(book)?;
    let market = market_from_cache(cache, book)?;
    Ok(Engine::build(Inputs { ledger, facts, market, clock }))
}

fn clock(now: Timestamp) -> Result<Clock, String> {
    let bank = TimeZone::get("America/Toronto").map_err(err)?;
    let home = TimeZone::system();
    let today = now.to_zoned(home.clone()).date();
    Ok(Clock { today, now, home, bank })
}

/// Run every due reader until the needs stop changing; the report lists each
/// source's outcomes of the run.
pub fn read_sources(book_dir: &Path, cache_path: &Path, now: Timestamp) -> Result<String, String> {
    let at = Timestamp::now();
    let (book, _) = Book::open_in(book_dir, crate::app::APP_VERSION, at).map_err(err)?;
    let (cache, _) = MarketCache::open(cache_path, crate::app::APP_VERSION, at).map_err(err)?;
    let net = Net::new(Arc::new(SystemClock), Arc::new(Limiter::new()));
    let clock = clock(now)?;
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now, bank: &clock.bank };
    let mut last: Option<FactNeeds> = None;
    let mut passes = 0;
    let mut unread: Vec<(InstrumentId, String)> = Vec::new();
    while passes < PASSES {
        let n = engine(&book, &cache, clock.clone())?.needs();
        if last.as_ref() == Some(&n) {
            break;
        }
        // what this run has not asked for yet: a read that failed is not asked
        // again in the same run, only on its source's own schedule
        let needs = needs_of(&book, &last.as_ref().map_or_else(|| n.clone(), |l| new_in(&n, l)))?;
        rates::read(&ctx, &needs.rates).map_err(err)?;
        market::read_closes(&ctx, &needs.closes).map_err(err)?;
        if let Some(from) = needs.benchmarks_from {
            market::read_benchmarks(&ctx, from).map_err(err)?;
        }
        payers::run::read(&ctx, &needs.payers).map_err(err)?;
        unread = payers::run::unread(&needs.payers).iter().map(|p| (p.listing.id, p.listing.symbol.clone())).collect();
        last = Some(n);
        passes += 1;
    }
    let mut out = String::new();
    let _ = writeln!(out, "{passes} pass(es) of reads");
    // each payer held, and the source that reads it
    if let Some(n) = &last {
        let held = needs_of(&book, n)?;
        let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for p in &held.payers {
            let source = payers::adapter_for(p).map_or_else(|| "no source".to_string(), |a| a.source().to_string());
            by.entry(source).or_default().push(p.listing.symbol.clone());
        }
        for (source, symbols) in by {
            let _ = writeln!(out, "payers held, read by {source}: {}", symbols.join(", "));
        }
    }
    for source in cache.sources().map_err(err)? {
        let rows: Vec<_> = cache.outcomes(&source).map_err(err)?.into_iter().filter(|o| o.at >= at.min(now)).collect();
        if rows.is_empty() {
            continue;
        }
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for r in &rows {
            *counts.entry(r.outcome.as_str()).or_default() += 1;
        }
        let counts: Vec<String> = counts.iter().map(|(k, n)| format!("{n} {k}")).collect();
        let _ = writeln!(out, "{source}: {}", counts.join(", "));
        for r in rows.iter().filter(|r| r.outcome != bagholder_sources::outcome::OutcomeKind::Answered) {
            let what = r.instrument.map(|i| i.to_string()).unwrap_or_default();
            let _ = writeln!(out, "  {} {} {what} {}", r.outcome.as_str(), r.kind.as_str(), r.detail);
        }
        for r in rows.iter().filter_map(|r| r.shape_change.as_ref()) {
            let _ = writeln!(out, "  shape changed: {r}");
        }
    }
    // a held security no source reads matters where a figure waits on its payer:
    // one that has paid something (a security that never paid shows no rate)
    let e = engine(&book, &cache, clock)?;
    let waiting: Vec<String> = unread.into_iter().filter(|(id, _)| e.figures().payers.contains_key(id)).map(|(id, symbol)| format!("{symbol} ({id})")).collect();
    if !waiting.is_empty() {
        let _ = writeln!(out, "payers whose figures wait and that no source reads: {}", waiting.join(", "));
    }
    Ok(out)
}

/// Each source's state and its last outcome of each kind.
pub fn source_health(cache_path: &Path, now: Timestamp) -> Result<String, String> {
    let (cache, _) = MarketCache::open(cache_path, crate::app::APP_VERSION, Timestamp::now()).map_err(err)?;
    health_report(&cache, now)
}

fn health_report(cache: &MarketCache, now: Timestamp) -> Result<String, String> {
    let mut out = String::new();
    for source in cache.sources().map_err(err)? {
        let rows = cache.outcomes(&source).map_err(err)?;
        let _ = writeln!(out, "{source}: {}", health::state(&rows, now));
        for (kind, r) in health::last_of_each(&rows) {
            let _ = writeln!(out, "  {} {} {} {}", kind.as_str(), r.at, r.kind.as_str(), r.detail);
        }
    }
    Ok(out)
}

struct Args {
    positional: Vec<String>,
    cache: Option<PathBuf>,
    now: Option<Timestamp>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args { positional: vec![], cache: None, now: None };
    let mut it = args.iter();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--cache" => a.cache = Some(PathBuf::from(it.next().ok_or("--cache names a file")?)),
            "--now" => {
                let v = it.next().ok_or("--now names an instant")?;
                a.now = Some(v.parse().map_err(|e| format!("{v} is not an instant: {e}"))?);
            }
            _ => a.positional.push(x.clone()),
        }
    }
    Ok(a)
}

pub fn cli_read(args: &[String]) -> i32 {
    let usage = "usage: bagholder read-sources <book folder> [--cache <market.db>] [--now <instant>]";
    let a = match parse(args) {
        Ok(a) if a.positional.len() == 1 => a,
        Ok(_) => {
            eprintln!("{usage}");
            return 2;
        }
        Err(e) => {
            eprintln!("{e}\n{usage}");
            return 2;
        }
    };
    let book = PathBuf::from(&a.positional[0]);
    let cache = a.cache.unwrap_or_else(|| book.join("market.db"));
    match read_sources(&book, &cache, a.now.unwrap_or_else(Timestamp::now)) {
        Ok(report) => {
            print!("{report}");
            0
        }
        Err(e) => {
            eprintln!("the read failed: {e}");
            1
        }
    }
}

pub fn cli_health(args: &[String]) -> i32 {
    let usage = "usage: bagholder source-health --cache <market.db> [--now <instant>]";
    let a = match parse(args) {
        Ok(Args { positional, cache: Some(c), now }) if positional.is_empty() => (c, now),
        Ok(_) => {
            eprintln!("{usage}");
            return 2;
        }
        Err(e) => {
            eprintln!("{e}\n{usage}");
            return 2;
        }
    };
    match source_health(&a.0, a.1.unwrap_or_else(Timestamp::now)) {
        Ok(report) => {
            print!("{report}");
            0
        }
        Err(e) => {
            eprintln!("the health could not be read: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use bagholder_core::jiff::SignedDuration;
    use bagholder_core::SourceName;
    use bagholder_sources::cache::OutcomeRow;
    use bagholder_sources::contract::DataKind;
    use bagholder_sources::outcome::OutcomeKind;

    use super::*;

    #[test]
    fn a_later_pass_asks_only_what_became_needed() {
        use bagholder_core::jiff::civil::date;
        use bagholder_core::Currency;
        let i = |n: u8| InstrumentId::parse(&format!("0192a000-0000-7000-8000-0000000000{n:02}")).unwrap();
        let before = FactNeeds {
            rates: [(Currency::USD, date(2024, 1, 2))].into(),
            closes: [(i(1), (date(2024, 1, 2), date(2026, 9, 24)))].into(),
            payers: [i(1)].into(),
            first_day: Some(date(2024, 1, 2)),
        };
        let now = FactNeeds {
            rates: [(Currency::USD, date(2024, 1, 2)), (Currency::parse("EUR").unwrap(), date(2025, 3, 3))].into(),
            closes: [(i(1), (date(2024, 1, 2), date(2026, 9, 24))), (i(2), (date(2026, 6, 19), date(2026, 6, 19)))].into(),
            payers: [i(1), i(3)].into(),
            first_day: Some(date(2024, 1, 2)),
        };
        let todo = new_in(&now, &before);
        assert_eq!(todo.rates, [(Currency::parse("EUR").unwrap(), date(2025, 3, 3))].into());
        assert_eq!(todo.closes, [(i(2), (date(2026, 6, 19), date(2026, 6, 19)))].into());
        assert_eq!(todo.payers, [i(3)].into());
        assert_eq!(todo.first_day, None);
        // a currency needed from an earlier day, a wider span: asked again
        let mut earlier = now.clone();
        earlier.rates.insert(Currency::USD, date(2023, 5, 1));
        earlier.closes.insert(i(1), (date(2023, 5, 1), date(2026, 9, 24)));
        let todo = new_in(&earlier, &now);
        assert_eq!(todo.rates.keys().collect::<Vec<_>>(), vec![&Currency::USD]);
        assert_eq!(todo.closes.keys().collect::<Vec<_>>(), vec![&i(1)]);
        assert!(new_in(&now, &now) == FactNeeds::default());
    }

    #[test]
    fn source_health_prints_each_state_and_each_last_outcome() {
        let dir = std::env::temp_dir().join(format!("bh-health-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let now: Timestamp = "2026-09-24T20:00:00Z".parse().unwrap();
        let (cache, _) = MarketCache::open(&dir.join("market.db"), crate::app::APP_VERSION, now).unwrap();
        let row = |source: &'static str, outcome: OutcomeKind, minutes: i64, shape: Option<&str>| OutcomeRow {
            source: SourceName::named(source),
            host: "example.com".into(),
            kind: DataKind::DailyClose,
            instrument: None,
            outcome,
            detail: format!("{} {source}", outcome.as_str()),
            shape_change: shape.map(str::to_string),
            at: now - SignedDuration::from_mins(minutes),
        };
        for r in [
            row("alpha", OutcomeKind::Answered, 5, None),
            row("bravo", OutcomeKind::Answered, 10, None),
            row("bravo", OutcomeKind::Refused, 5, None),
            row("charlie", OutcomeKind::Mismatch, 5, None),
            row("delta", OutcomeKind::Answered, 5, Some("gone: $.price")),
            row("echo", OutcomeKind::NotCarried, 5, None),
        ] {
            cache.record(&r).unwrap();
        }
        let report = health_report(&cache, now).unwrap();
        for line in ["alpha: working", "bravo: refusing", "charlie: failing", "delta: shape-changed", "echo: unasked", "  refused ", "  answered ", "  mismatch ", "  not-carried "] {
            assert!(report.contains(line), "{line:?} not in:\n{report}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
