//! Live quotes, one run over recorded real replies: each market asked of its
//! own source, each quote kept with the time its source states, a TMX form it
//! answers for written back to the book, and a spot price stamped with its
//! reply's date less the age its origin allows.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use bagholder_book::Book;
use bagholder_core::instrument::{InstrumentKind, RefScheme};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, InstrumentId, Money};
use bagholder_sources::cache::MarketCache;
use bagholder_sources::contract::{DataKind, Listing};
use bagholder_sources::outcome::{Outcome, OutcomeKind};
use bagholder_sources::quotes;
use bagholder_sources::read::Ctx;

fn t(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn id(n: u8) -> InstrumentId {
    InstrumentId::parse(&format!("0192a000-0000-7000-8000-0000000000{n:02}")).unwrap()
}

fn listing(n: u8, kind: InstrumentKind, currency: Currency, symbol: &str, mic: Option<&str>) -> Listing {
    Listing { id: id(n), kind, currency, symbol: symbol.into(), venue_mic: mic.map(str::to_string), routes: BTreeMap::new() }
}

const TMX: &str = "https://app-money.tmx.com/graphql";

#[test]
fn each_market_is_quoted_by_its_own_source_with_the_time_it_states() {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-24T04:40:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    for (n, kind, c) in [(1, "security", "CAD"), (2, "security", "CAD"), (3, "security", "USD"), (4, "crypto", "CAD"), (5, "crypto", "USD"), (6, "security", "CAD"), (7, "security", "CAD")] {
        common::instrument_in_book(&dir.path().join("book.db"), id(n), kind, c);
    }
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let recorded = Arc::new(
        common::Recorded::new()
            .with_body(TMX, "\"symbol\":\"ENB\"", 200, "tmx", "quote-ENB.json")
            // TMX's answer for the TSX listing, served to the CSE form: another listing
            .with_body(TMX, "\"symbol\":\"ENB:CNX\"", 200, "tmx", "quote-ENB.json")
            .with("https://www-api.cboe.com/ca/equities/securities-1/HBIX/quote/", 200, "cboe-canada", "quote-HBIX.json")
            .with("https://query1.finance.yahoo.com/v8/finance/chart/SPY?range=1d&interval=1d", 200, "yahoo", "SPY-range-1d.json")
            .with("https://api.coinbase.com/v2/prices/BTC-CAD/spot", 200, "coinbase", "spot-BTC-CAD.json")
            .with("https://api.exchange.coinbase.com/products/BTC-USD/ticker", 200, "coinbase-exchange", "ticker-BTC-USD.json")
            // the coins' last closes: the CAD market is none of the Exchange's, and the
            // USD market's answer holds none of these days
            .with_prefix("https://api.exchange.coinbase.com/products/BTC-CAD/candles", 404, "coinbase-exchange", "candles-BTC-CAD-status-404.json")
            .with_prefix("https://api.exchange.coinbase.com/products/BTC-USD/candles", 200, "coinbase-exchange", "candles-BTC-USD-2026-09-01-2026-09-05.json"),
    );
    let net = common::net(&recorded, "2026-09-24T04:40:00Z");
    let zone = TimeZone::get("America/Toronto").unwrap();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    let listings = [
        listing(1, InstrumentKind::Security, Currency::CAD, "ENB", Some("XTSE")),
        listing(2, InstrumentKind::Security, Currency::CAD, "HBIX", Some("NEOE")),
        listing(3, InstrumentKind::Security, Currency::USD, "SPY", Some("ARCX")),
        listing(4, InstrumentKind::Crypto, Currency::CAD, "BTC", None),
        listing(5, InstrumentKind::Crypto, Currency::USD, "BTC", None),
        listing(6, InstrumentKind::Security, Currency::CAD, "ENB", Some("XCNQ")),
        // a listing the book holds in CAD, answered in USD: another listing
        listing(7, InstrumentKind::Security, Currency::CAD, "SPY", Some("ARCX")),
    ];
    quotes::read_quotes(&ctx, &listings).unwrap();
    let got: BTreeMap<InstrumentId, _> = cache.quotes().unwrap().into_iter().map(|q| (q.instrument, q)).collect();
    let q = |n: u8| (got[&id(n)].source.to_string(), got[&id(n)].price, got[&id(n)].quoted_at, got[&id(n)].allowance.as_secs());
    assert_eq!(q(1), ("tmx".into(), Money::new(dec("67.47"), Currency::CAD), t("2026-09-24T04:20:30Z"), 0));
    assert_eq!(q(2), ("cboe-canada".into(), Money::new(dec("7.2600"), Currency::CAD), t("2026-09-23T20:00:00Z"), 0));
    assert_eq!(q(3), ("yahoo".into(), Money::new(dec("767.81"), Currency::USD), t("2026-09-23T20:00:00Z"), 0));
    // the spot price states no time: its reply's date less the origin's sixty seconds
    assert_eq!(q(4), ("coinbase".into(), Money::new(dec("118318.2"), Currency::CAD), t("2026-09-24T04:32:41Z"), 60));
    assert_eq!(q(5), ("coinbase-exchange".into(), Money::new(dec("83895.98"), Currency::USD), t("2026-09-24T04:33:45.647605322Z"), 0));
    // ENB on the CSE: TMX named another venue for the CSE form, so the other
    // Canadian forms are asked, and the bare form's answer on the TSX is kept and
    // remembered, but not written to the book, whose venue it is not
    assert_eq!(q(6).1, Money::new(dec("67.47"), Currency::CAD));
    let tmx_rows = cache.outcomes(&bagholder_core::SourceName::named("tmx")).unwrap();
    assert!(tmx_rows.iter().any(|o| o.instrument == Some(id(6)) && o.outcome == OutcomeKind::NotCarried && o.kind == DataKind::Quote && o.detail.starts_with("ENB:CNX")));
    assert_eq!(cache.winner(id(6), DataKind::Quote).unwrap().map(|w| w.1), Some("ENB".to_string()));
    assert!(!got.contains_key(&id(7)));
    let yahoo_rows = cache.outcomes(&bagholder_core::SourceName::named("yahoo")).unwrap();
    assert!(yahoo_rows.iter().any(|o| o.instrument == Some(id(7)) && o.outcome == OutcomeKind::Meaning && o.detail.contains("USD")));
    // the form TMX answered for on the book's venue is written back as a route
    let routes = |n: u8| book.instrument_refs(id(n)).unwrap().into_iter().filter(|r| r.scheme == RefScheme::TmxForm).map(|r| r.value).collect::<Vec<_>>();
    assert_eq!(routes(1), vec!["ENB".to_string()]);
    assert!(routes(6).is_empty());
}

/// An index, a future, a rate and a currency pair of the app's directory are
/// quoted by Yahoo's chart under the code the directory gives each (its `yahoo`
/// route), each with the day's change in points from the previous close the
/// chart states; one with no such code is not asked for, and says so.
#[test]
fn the_directory_s_instruments_are_quoted_by_yahoo_under_their_own_code() {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-26T20:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    let kinds = [(11, InstrumentKind::Index, Currency::USD, "SPX", "^GSPC"), (12, InstrumentKind::Future, Currency::USD, "GC", "GC=F"), (13, InstrumentKind::Rate, Currency::USD, "ZQ", "ZQ=F"), (14, InstrumentKind::CurrencyPair, Currency::CAD, "USDCAD", "CAD=X"), (15, InstrumentKind::Index, Currency::USD, "NDX", "")];
    for (n, kind, c, _, _) in kinds {
        common::instrument_in_book(&dir.path().join("book.db"), id(n), kind.as_str(), c.as_str());
    }
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let chart = |code: &str| format!("https://query1.finance.yahoo.com/v8/finance/chart/{}?range=1d&interval=1d", code.replace('^', "%5E").replace('=', "%3D"));
    let recorded = Arc::new(
        common::Recorded::new()
            .with(&chart("^GSPC"), 200, "yahoo", "^GSPC-range-1d.json")
            .with(&chart("GC=F"), 200, "yahoo", "GC=F-range-1d.json")
            .with(&chart("ZQ=F"), 200, "yahoo", "ZQ=F-range-1d.json")
            .with(&chart("CAD=X"), 200, "yahoo", "CAD=X-range-1d.json"),
    );
    let net = common::net(&recorded, "2026-09-26T20:00:00Z");
    let zone = TimeZone::get("America/Toronto").unwrap();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    let listings: Vec<Listing> = kinds
        .iter()
        .map(|(n, kind, c, symbol, code)| {
            let mut l = listing(*n, *kind, *c, symbol, None);
            if !code.is_empty() {
                l.routes.insert(RefScheme::Yahoo, vec![code.to_string()]);
            }
            l
        })
        .collect();
    quotes::read_quotes(&ctx, &listings).unwrap();
    let got: BTreeMap<InstrumentId, _> = cache.quotes().unwrap().into_iter().map(|q| (q.instrument, q)).collect();
    let q = |n: u8| (got[&id(n)].source.to_string(), got[&id(n)].price, got[&id(n)].change, got[&id(n)].change_pct, got[&id(n)].quoted_at);
    assert_eq!(q(11), ("yahoo".into(), Money::new(dec("7743.41"), Currency::USD), Some(dec("39.28")), Some(dec("0.51")), Timestamp::from_second(1790368797).unwrap()));
    assert_eq!(q(12), ("yahoo".into(), Money::new(dec("4321.2"), Currency::USD), Some(dec("23.2")), Some(dec("0.54")), Timestamp::from_second(1790369999).unwrap()));
    assert_eq!(q(13), ("yahoo".into(), Money::new(dec("95.965"), Currency::USD), Some(dec("0.015")), got[&id(13)].change_pct, Timestamp::from_second(1790369970).unwrap()));
    assert_eq!(q(14), ("yahoo".into(), Money::new(dec("1.4141"), Currency::CAD), Some(dec("0.0005")), Some(dec("0.034")), Timestamp::from_second(1790447707).unwrap()));
    assert!(!got.contains_key(&id(15)));
    let yahoo_rows = cache.outcomes(&bagholder_core::SourceName::named("yahoo")).unwrap();
    assert!(yahoo_rows.iter().any(|o| o.instrument == Some(id(15)) && o.outcome == OutcomeKind::NotCarried && o.detail.contains("no code")));
    assert_eq!(recorded.asked.lock().unwrap().len(), 4, "only the four with a code were asked for");
}

/// A coin's day change is against the close of the last completed UTC day on its
/// Coinbase Exchange market: its own pair's, else the USD market's in CAD at the
/// Bank of Canada's rate of that day, or the last one published within the week
/// before it (a weekend's close takes Friday's rate); the closes asked of the
/// Exchange once a day and kept.
#[test]
fn a_coin_s_day_change_is_against_the_last_completed_utc_day() {
    let dir = tempfile::tempdir().unwrap();
    let at = t("2026-09-06T12:00:00Z");
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", at).unwrap();
    common::instrument_in_book(&dir.path().join("book.db"), id(4), "crypto", "CAD");
    common::instrument_in_book(&dir.path().join("book.db"), id(5), "crypto", "USD");
    // the Bank's rate on Friday the 4th; Saturday the 5th has none
    let friday: bagholder_core::jiff::civil::Date = "2026-09-04".parse().unwrap();
    book.store_rates(Currency::USD, &[(friday, dec("1.3800"))], (friday, friday), &bagholder_core::SourceName::named("bank-of-canada"), at).unwrap();
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let recorded = Arc::new(
        common::Recorded::new()
            .with("https://api.coinbase.com/v2/prices/BTC-CAD/spot", 200, "coinbase", "spot-BTC-CAD.json")
            .with("https://api.exchange.coinbase.com/products/BTC-USD/ticker", 200, "coinbase-exchange", "ticker-BTC-USD.json")
            .with_prefix("https://api.exchange.coinbase.com/products/BTC-CAD/candles", 404, "coinbase-exchange", "candles-BTC-CAD-status-404.json")
            .with_prefix("https://api.exchange.coinbase.com/products/BTC-USD/candles", 200, "coinbase-exchange", "candles-BTC-USD-2026-09-01-2026-09-05.json"),
    );
    let net = common::net(&recorded, "2026-09-06T12:00:00Z");
    let zone = TimeZone::get("America/Toronto").unwrap();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: at, bank: &zone };
    let listings = [listing(4, InstrumentKind::Crypto, Currency::CAD, "BTC", None), listing(5, InstrumentKind::Crypto, Currency::USD, "BTC", None)];
    quotes::read_quotes(&ctx, &listings).unwrap();
    let got: BTreeMap<InstrumentId, _> = cache.quotes().unwrap().into_iter().map(|q| (q.instrument, q)).collect();
    // USD: 83,895.98 against Saturday's close of 79,831.57
    assert_eq!((got[&id(5)].change, got[&id(5)].change_pct), (Some(dec("4064.41")), Some(dec("5.0912"))));
    // CAD: Saturday's close of 79,831.57 at Friday's 1.38, the last rate the Bank
    // published: 110,167.5666
    assert_eq!((got[&id(4)].change, got[&id(4)].change_pct), (Some(dec("8150.6334")), Some(dec("7.3984"))));
    // asked once today: a second read asks the Exchange for no candles
    let candles = || recorded.asked.lock().unwrap().iter().filter(|u| u.contains("/candles")).count();
    let before = candles();
    quotes::read_quotes(&ctx, &listings).unwrap();
    assert_eq!(candles(), before, "the closes are kept, not asked again");
}

/// A coin's glance carries its day change as a held coin's quote does, in the
/// currency it is glanced in.
#[test]
fn a_coin_s_glance_has_its_day_change_in_its_own_currency() {
    let recorded = Arc::new(
        common::Recorded::new()
            .with("https://api.coinbase.com/v2/prices/BTC-CAD/spot", 200, "coinbase", "spot-BTC-CAD.json")
            .with_prefix("https://api.exchange.coinbase.com/products/BTC-CAD/candles", 404, "coinbase-exchange", "candles-BTC-CAD-status-404.json")
            .with_prefix("https://api.exchange.coinbase.com/products/BTC-USD/candles", 200, "coinbase-exchange", "candles-BTC-USD-2026-09-01-2026-09-05.json"),
    );
    let net = common::net(&recorded, "2026-09-06T12:00:00Z");
    let friday: bagholder_core::jiff::civil::Date = "2026-09-04".parse().unwrap();
    let rates = BTreeMap::from([(Currency::USD, BTreeMap::from([(friday, dec("1.3800"))]))]);
    let of = quotes::GlanceOf { kind: InstrumentKind::Crypto, currency: Currency::CAD, symbol: "BTC".into(), venue_mic: None, yahoo: None };
    let Outcome::Answered(g) = quotes::glance(&net, t("2026-09-06T12:00:00Z"), &of, &rates) else { panic!("a glance") };
    assert_eq!(g, quotes::Glance { price: Money::new(dec("118318.2"), Currency::CAD), change: Some(dec("8150.6334")), change_pct: Some(dec("7.3984")) });
}

fn tmx_world(at: &str) -> (tempfile::TempDir, Book, MarketCache) {
    let dir = tempfile::tempdir().unwrap();
    let (book, _) = Book::open(&dir.path().join("book.db"), "test", t(at)).unwrap();
    for n in 1..=4 {
        common::instrument_in_book(&dir.path().join("book.db"), id(n), "security", "CAD");
    }
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", t(at)).unwrap();
    (dir, book, cache)
}

/// A listing whose venue TMX's form does not carry is asked under the other
/// Canadian forms in turn, each answer checked against the venue its form names;
/// the form that answers is asked first from then on. A listing no form answers
/// for is asked again under its first form only, until the next day. A listing
/// whose record names no venue known here is quoted as its currency's market's.
#[test]
fn tmx_is_asked_the_other_canadian_forms_and_remembers_what_answered() {
    let at = "2026-09-24T04:40:00Z";
    let (_dir, book, cache) = tmx_world(at);
    let recorded = Arc::new(
        common::Recorded::new()
            .with_body(TMX, "\"symbol\":\"ENB:CNX\"", 200, "tmx", "quote-ENB-CNX-unknown.json")
            .with_body(TMX, "\"symbol\":\"ENB\"", 200, "tmx", "quote-ENB.json")
            .with_body(TMX, "ZZZQX", 200, "tmx", "quote-ZZZQX-unknown.json"),
    );
    let zone = TimeZone::get("America/Toronto").unwrap();
    let listings = [
        // the book says the CSE, where TMX knows no ENB
        listing(1, InstrumentKind::Security, Currency::CAD, "ENB", Some("XCNQ")),
        // no venue known here: a CAD listing is Canadian, asked bare first
        listing(2, InstrumentKind::Security, Currency::CAD, "ENB", Some("XTSV-OTC")),
        listing(3, InstrumentKind::Security, Currency::CAD, "ZZZQX", None),
    ];
    let run = |now: &str| {
        let net = common::net(&recorded, now);
        let ctx = Ctx { book: &book, cache: &cache, net: &net, now: t(now), bank: &zone };
        let before = recorded.asked.lock().unwrap().len();
        quotes::read_quotes(&ctx, &listings).unwrap();
        recorded.asked.lock().unwrap().len() - before
    };
    // ENB: ENB:CNX, then ENB; the second: ENB; ZZZQX: its three forms
    assert_eq!(run(at), 2 + 1 + 3);
    let got: BTreeMap<InstrumentId, _> = cache.quotes().unwrap().into_iter().map(|q| (q.instrument, q)).collect();
    assert_eq!(got[&id(1)].price, Money::new(dec("67.47"), Currency::CAD));
    assert_eq!(got[&id(2)].price, Money::new(dec("67.47"), Currency::CAD));
    assert!(!got.contains_key(&id(3)));
    assert_eq!(cache.winner(id(1), DataKind::Quote).unwrap().map(|w| w.1), Some("ENB".to_string()));
    // not the book's venue's form: the book learns no route from it
    assert!(book.instrument_refs(id(1)).unwrap().iter().all(|r| r.scheme != RefScheme::TmxForm));
    // later that day: the winners first, and ZZZQX under its first form alone
    assert_eq!(run("2026-09-24T15:00:00Z"), 1 + 1 + 1);
    // the next day ZZZQX's forms are all asked again
    assert_eq!(run("2026-09-25T15:00:00Z"), 1 + 1 + 3);
}

/// A halted listing is answered with no price: the price read last stands, and
/// TMX's answer is no failure.
#[test]
fn a_halted_listing_keeps_the_price_read_last() {
    let at = "2026-09-24T04:40:00Z";
    let (_dir, book, cache) = tmx_world(at);
    let zone = TimeZone::get("America/Toronto").unwrap();
    let enb = [listing(1, InstrumentKind::Security, Currency::CAD, "ENB", Some("XTSE"))];
    let read = |name: &str, now: &str| {
        let recorded = Arc::new(common::Recorded::new().with(TMX, 200, "tmx", name));
        let net = common::net(&recorded, now);
        let ctx = Ctx { book: &book, cache: &cache, net: &net, now: t(now), bank: &zone };
        quotes::read_quotes(&ctx, &enb).unwrap();
    };
    read("quote-ENB.json", at);
    read("edited-quote-ENB-price-null.json", "2026-09-24T15:00:00Z");
    let q = cache.quotes().unwrap();
    assert_eq!((q.len(), q[0].price, q[0].quoted_at), (1, Money::new(dec("67.47"), Currency::CAD), t("2026-09-24T04:20:30Z")));
    let tmx_rows = cache.outcomes(&bagholder_core::SourceName::named("tmx")).unwrap();
    assert!(tmx_rows.iter().all(|o| o.outcome == OutcomeKind::Answered), "{tmx_rows:?}");
}

/// A schedule word TMX states that the reader does not know leaves the price
/// standing; the payer's record, which reads the schedule, refuses it.
#[test]
fn an_unknown_schedule_word_fails_the_payers_record_and_not_the_price() {
    let at = "2026-09-24T04:40:00Z";
    let (_dir, book, cache) = tmx_world(at);
    let recorded = Arc::new(common::Recorded::new().with_body(TMX, "getQuoteBySymbol", 200, "tmx", "wrong-shape-quote-ENB-schedule-unknown.json"));
    let net = common::net(&recorded, at);
    let zone = TimeZone::get("America/Toronto").unwrap();
    let ctx = Ctx { book: &book, cache: &cache, net: &net, now: t(at), bank: &zone };
    let enb = listing(1, InstrumentKind::Security, Currency::CAD, "ENB", Some("XTSE"));
    quotes::read_quotes(&ctx, std::slice::from_ref(&enb)).unwrap();
    assert_eq!(cache.quotes().unwrap()[0].price, Money::new(dec("67.47"), Currency::CAD));
    use bagholder_sources::payers::Payer;
    let need = bagholder_sources::needs::PayerNeed { listing: enb, name: Some("Enbridge Inc.".into()) };
    let noted = bagholder_sources::payers::exchange::TmxRecord.read(&net, &need, t(at));
    assert!(matches!(noted.outcome, Outcome::Mismatch(m) if m.why.contains("Fortnightly")));
}
