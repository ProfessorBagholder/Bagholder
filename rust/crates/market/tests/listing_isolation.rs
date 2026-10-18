//! Rule test 1 of plan `market-data-keyed-by-listing`: what a reader stores,
//! keeps or answers for one listing is never another listing's, for generated
//! pairs of listings that share a ticker (two venues of one country, a venue in
//! each country, a ticker with a venue suffix and one without, a coin and a
//! share, an option and its underlying), in either order of reads. Symbols and
//! venues are drawn from the venue table, never named. Nothing here reaches the
//! network: the process is offline, and each source a reader asks is stood in for.

use std::collections::HashMap;

use bagholder_core::listing::{CANADA_MICS, US_MICS};
use bagholder_core::ListingKey;
use bagholder_market::exposure::{self, hooks, Ctx, ShareClass};
use bagholder_market::news::{self, Ask, Clock, NetError, Readers, WireAnswer};
use bagholder_market::shorts::{self, CaPositionRow, CaVolumeRow};
use bagholder_store::bars::{DayBar, Ohlcv};
use bagholder_store::feeds::{Feed, NewsItem, NewsKind};
use rusqlite::Connection;

/// A small generator, seeded, so a failure is the same failure when run again.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) % n as u64) as usize
    }

    fn symbol(&mut self) -> String {
        let len = 1 + self.below(4);
        let mut s: String = (0..len).map(|_| (b'A' + self.below(26) as u8) as char).collect();
        if self.below(4) == 0 {
            s.push_str(".B");
        }
        s
    }
}

/// One side of a pair: the symbol as a source writes it, and the venue it is on.
#[derive(Clone, Debug)]
struct Side {
    symbol: String,
    venue: String,
}

impl Side {
    fn key(&self) -> ListingKey {
        ListingKey::on(&self.symbol, &self.venue).unwrap()
    }
}

/// Pairs of listings that share a ticker, of every kind the rule covers.
fn pairs(r: &mut Rng, n: usize) -> Vec<(Side, Side)> {
    let side = |symbol: &str, venue: &str| Side { symbol: symbol.to_string(), venue: venue.to_string() };
    let mut out = Vec::new();
    for _ in 0..n {
        let sym = r.symbol();
        let (a, b) = loop {
            let (a, b) = (CANADA_MICS[r.below(CANADA_MICS.len())], CANADA_MICS[r.below(CANADA_MICS.len())]);
            if a != b {
                break (a, b);
            }
        };
        let us = US_MICS[r.below(US_MICS.len())];
        out.push((side(&sym, a), side(&sym, b)));
        out.push((side(&sym, a), side(&sym, us)));
        // a venue suffix as a record may carry one (`ENB.TO`), beside another venue's listing of the ticker
        let other = if b == "XTSE" { a } else { b };
        out.push((side(&format!("{sym}.TO"), "XTSE"), side(&sym, other)));
        out.push((side(&sym, "CRYPTO"), side(&sym, us)));
        // the OTC market beside a US exchange: one ticker, two venues
        out.push((side(&sym, bagholder_core::listing::OTC_MICS[r.below(bagholder_core::listing::OTC_MICS.len())]), side(&sym, us)));
        out.push((side(&format!("{sym} 261218C00010000"), "OCC"), side(&sym, us)));
    }
    out
}

fn store() -> Connection {
    std::env::set_var("BAGHOLDER_OFFLINE", "1");
    let c = Connection::open_in_memory().unwrap();
    for m in &bagholder_sources::cache::MIGRATIONS {
        c.execute_batch(m.sql).unwrap();
    }
    c
}

/// Each listing's own mark, which no other listing's data carries.
fn mark(s: &Side) -> String {
    s.key().to_string()
}

/// Both orders of a pair.
fn both(a: &Side, b: &Side) -> [(Side, Side); 2] {
    [(a.clone(), b.clone()), (b.clone(), a.clone())]
}

#[test]
fn a_listings_bars_are_its_own() {
    let mut r = Rng(1);
    for (a, b) in pairs(&mut r, 20) {
        for (first, second) in both(&a, &b) {
            let c = store();
            let bars = |_: &Side, close: f64| vec![DayBar { date: "2026-09-15".into(), px: Ohlcv { open: None, high: None, low: None, close, volume: None } }];
            bagholder_store::market::upsert_price_history(&c, &first.key(), &bars(&first, 1.0), &mark(&first)).unwrap();
            bagholder_store::market::upsert_price_history(&c, &second.key(), &bars(&second, 2.0), &mark(&second)).unwrap();
            for (s, close) in [(&first, 1.0), (&second, 2.0)] {
                let got = bagholder_store::market::price_history(&c, &s.key(), "2026-09-01", "2026-09-30").unwrap();
                assert_eq!(got.iter().map(|b| b.px.close).collect::<Vec<_>>(), vec![close], "{s:?} beside {first:?} / {second:?}");
            }
        }
    }
}

/// A listing's intraday bars, the source its bars last came from, and a timeframe
/// no source had for it, are its own: never another listing's of its ticker.
#[test]
fn a_listings_intraday_bars_bar_source_and_misses_are_its_own() {
    use bagholder_market::history;
    use bagholder_store::bars::TimeBar;
    let mut r = Rng(11);
    for (a, b) in pairs(&mut r, 20) {
        for (first, second) in both(&a, &b) {
            let c = store();
            let bars = |close: f64| vec![TimeBar { time: 1_789_000_000, px: Ohlcv { open: None, high: None, low: None, close, volume: None } }];
            bagholder_store::market::upsert_price_bars(&c, &first.key(), "1h", &bars(1.0), "test").unwrap();
            bagholder_store::market::upsert_price_bars(&c, &second.key(), "1h", &bars(2.0), "test").unwrap();
            for (s, close) in [(&first, 1.0), (&second, 2.0)] {
                let got = bagholder_store::market::price_bars(&c, &s.key(), "1h", 0, i64::MAX).unwrap();
                assert_eq!(got.iter().map(|b| b.px.close).collect::<Vec<_>>(), vec![close], "{s:?} beside {first:?} / {second:?}");
            }
            // a timeframe no source had for the first is no miss of the second's
            history::record_intraday_miss(&c, &first.key(), "1h", "2026-09-16T12:00:00Z").unwrap();
            let now = 1_789_560_300.0;
            assert!(history::intraday_missed_recently(&c, &first.key(), "1h", now).unwrap(), "{first:?}'s own miss");
            assert!(!history::intraday_missed_recently(&c, &second.key(), "1h", now).unwrap(), "{second:?} took {first:?}'s miss");
            // the source the first's bars came from is not the second's
            let keyed = |s: &Side| {
                let kind = match s.venue.as_str() {
                    "CRYPTO" => "Crypto",
                    "OCC" => "Options",
                    _ => "Shares",
                };
                bagholder_model::input::Listing::new(s.symbol.as_str(), s.venue.as_str(), "USD", kind).keyed()
            };
            if let (Ok(f), Ok(s)) = (keyed(&first), keyed(&second)) {
                history::remember_winner(&c, &f, "yahoo", "FIRST").unwrap();
                assert_eq!(history::bars_winner(&c, &f).unwrap(), Some(("yahoo".into(), "FIRST".into())));
                assert_eq!(history::bars_winner(&c, &s).unwrap(), None, "{second:?} took {first:?}'s bar source");
            }
        }
    }
}

#[test]
fn a_listings_news_is_its_own() {
    let mut r = Rng(2);
    let clock = Clock::at(bagholder_model::dates::to_days(2026, 9, 16) * 86400 + 12 * 3600);
    for (a, b) in pairs(&mut r, 10).into_iter().filter(|(a, _)| a.venue != "OCC") {
        for (first, second) in both(&a, &b) {
            let c = store();
            // the wire answers whatever listing it is asked for with that listing's own story
            let wire = |symbol: &str, exchange: &str, _: &Clock| {
                let id = format!("tmx:{}", ListingKey::on(symbol, exchange).map(|k| k.to_string()).unwrap_or_default());
                let item = NewsItem { id, headline: format!("{symbol} on {exchange}"), source: "Newsfile".into(), url: String::new(), published_at: "2026-09-15T12:00:00Z".into(), summary: String::new(), kind: NewsKind::Release, via: Feed::Tmx };
                (Feed::Tmx, Some(WireAnswer::of(vec![item])))
            };
            let extra = |_: Feed, _: &Ask| -> Result<Option<Vec<NewsItem>>, NetError> { Ok(Some(vec![])) };
            for s in [&first, &second] {
                news::read_listing(&c, &Readers { wire: &wire, extra: &extra }, &s.symbol, &s.venue, "", "", true, &clock, None).unwrap();
            }
            for s in [&first, &second] {
                let ids: Vec<String> = bagholder_store::feeds::news_for(&c, &s.key()).unwrap().into_iter().map(|n| n.id).collect();
                assert_eq!(ids, vec![format!("tmx:{}", mark(s))], "{s:?} beside {first:?} / {second:?}");
            }
        }
    }
}

#[test]
fn a_listings_breakdown_is_its_own() {
    let mut r = Rng(3);
    for (a, b) in pairs(&mut r, 10).into_iter().filter(|(a, _)| a.venue != "OCC" && a.venue != "CRYPTO") {
        for (first, second) in both(&a, &b) {
            let c = store();
            hooks::clear();
            hooks::FALLBACK.with(|h| *h.borrow_mut() = Some(Box::new(|_, _, _| Ok(None))));
            // each listing is classified as its own sector, named for it
            hooks::CLASSIFY.with(|h| *h.borrow_mut() = Some(Box::new(|sym, ex, _| ShareClass { sector: ListingKey::on(sym, ex).unwrap().to_string(), industry: String::new(), country: String::new(), source: "stand-in".into() })));
            let pool = std::sync::Arc::new(bagholder_store::pool::Pool::new(&std::env::temp_dir().join("bh-isolation-none.db")));
            let ctx = Ctx { conn: &c, pool, today: "2026-09-16".into() };
            for s in [&first, &second] {
                exposure::share_exposure(&ctx, &s.symbol, &s.venue, "").unwrap();
            }
            for s in [&first, &second] {
                let rec = bagholder_store::feeds::exposure_record(&c, &bagholder_model::venues::share_key(&s.key())).unwrap().expect("its record");
                assert_eq!(rec.record.sectors.first_name(), Some(mark(s).as_str()), "{s:?} beside {first:?} / {second:?}");
            }
            hooks::clear();
        }
    }
}

#[test]
fn a_listings_short_interest_is_its_own() {
    let mut r = Rng(4);
    // the Canadian regulator's files, as read: one row for each listing of every pair, its own shares
    for (a, b) in pairs(&mut r, 10).into_iter().filter(|(a, b)| CANADA_MICS.contains(&a.venue.as_str()) && CANADA_MICS.contains(&b.venue.as_str())) {
        for (first, second) in both(&a, &b) {
            let c = store();
            let code = |s: &Side| shorts::CA_VENUE_NAMES.iter().find(|(_, w)| ListingKey::on(&s.symbol, w).is_ok_and(|k| k == s.key())).map(|(c, _)| *c);
            let (Some(cf), Some(cs)) = (code(&first), code(&second)) else { continue }; // Alpha files nothing of its own
            let shares = |s: &Side| if s.key() == first.key() { 1000.0 } else { 2000.0 };
            let mut position = HashMap::new();
            let mut volume = HashMap::new();
            for (s, cd) in [(&first, cf), (&second, cs)] {
                let k = shorts::ciro_key(&s.symbol, cd).unwrap();
                position.insert(k.clone(), CaPositionRow { venue: cd.into(), shares: shares(s), change: None, name: mark(s) });
                volume.insert(k, CaVolumeRow { venue: cd.into(), short_volume: shares(s), volume_pct: Some(10.0), total_volume: Some(shares(s) * 10.0) });
            }
            shorts::clear_files();
            shorts::ca_position_with(&first.symbol, &first.venue, "2026-09-16", || Ok(Some(("2026-09-15".to_string(), position)))).unwrap();
            shorts::warm_ca_volume("ca_volume", "2026-09-01/2026-09-15", volume);
            for s in [&first, &second] {
                let got = shorts::for_listing(&c, &s.symbol, &s.venue, "2026-09-16", false, "").unwrap().expect("its short interest");
                assert_eq!((got.shares, got.short_volume), (Some(shares(s)), Some(shares(s))), "{s:?} beside {first:?} / {second:?}");
            }
            shorts::clear_files();
        }
    }
}

/// No listing whose venue is none known is read, stored or classified: its
/// market data waits for its venue.
#[test]
fn a_listing_with_no_venue_known_is_read_of_no_source() {
    let mut r = Rng(5);
    let c = store();
    hooks::clear();
    hooks::CLASSIFY.with(|h| *h.borrow_mut() = Some(Box::new(|sym, ex, _| panic!("{sym} on {ex:?} was classified"))));
    hooks::ADAPTER.with(|h| *h.borrow_mut() = Some(Box::new(|_, sym, _, ex| panic!("{sym} on {ex:?} was looked through"))));
    let pool = std::sync::Arc::new(bagholder_store::pool::Pool::new(&std::env::temp_dir().join("bh-isolation-none.db")));
    let ctx = Ctx { conn: &c, pool, today: "2026-09-16".into() };
    for _ in 0..50 {
        let sym = r.symbol();
        for venue in ["", "LSE", "XLON", "London Stock Exchange"] {
            assert_eq!(exposure::share_exposure(&ctx, &sym, venue, "CAD").unwrap(), bagholder_store::feeds::ExposureRecord::default());
            assert_eq!(exposure::fund_exposure(&ctx, &sym, "Some ETF", venue, 0, &mut Vec::new()).unwrap(), None);
            assert_eq!(shorts::market_of(&sym, venue), None);
            assert!(bagholder_market::quotes::yahoo_forms(&bagholder_model::input::Listing::new(sym.as_str(), venue, "CAD", "")).is_empty());
            assert_eq!(bagholder_market::quotes::tmx_quote_symbol(&sym, venue), None);
            assert_eq!(news::source_for(&sym, venue), None);
        }
    }
    let kept: i64 = c.query_row("SELECT COUNT(*) FROM exposures", [], |r| r.get(0)).unwrap();
    let meta: i64 = c.query_row("SELECT COUNT(*) FROM meta WHERE key NOT LIKE 'carry%'", [], |r| r.get(0)).unwrap();
    assert_eq!((kept, meta), (0, 0), "nothing kept for a listing with no venue");
    hooks::clear();
}
