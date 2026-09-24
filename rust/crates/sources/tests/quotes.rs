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
use bagholder_sources::outcome::OutcomeKind;
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
    for (n, kind, c) in [(1, "security", "CAD"), (2, "security", "CAD"), (3, "security", "USD"), (4, "crypto", "CAD"), (5, "crypto", "USD"), (6, "security", "CAD")] {
        common::instrument_in_book(&dir.path().join("book.db"), id(n), kind, c);
    }
    let (cache, _) = MarketCache::open(&dir.path().join("market.db"), "test", at).unwrap();
    let recorded = Arc::new(
        common::Recorded::new()
            .with_body(TMX, "\"symbol\":\"ENB\"", 200, "tmx", "quote-ENB.json")
            // TMX answers ENB:CNX with the TSX listing: not the one asked
            .with_body(TMX, "\"symbol\":\"ENB:CNX\"", 200, "tmx", "quote-ENB-CNX-another-venue.json")
            .with("https://www-api.cboe.com/ca/equities/securities-1/HBIX/quote/", 200, "cboe-canada", "quote-HBIX.json")
            .with("https://query1.finance.yahoo.com/v8/finance/chart/SPY?range=1d&interval=1d", 200, "yahoo", "SPY-range-1d.json")
            .with("https://api.coinbase.com/v2/prices/BTC-CAD/spot", 200, "coinbase", "spot-BTC-CAD.json")
            .with("https://api.exchange.coinbase.com/products/BTC-USD/ticker", 200, "coinbase-exchange", "ticker-BTC-USD.json"),
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
    // ENB on the CSE: TMX named another venue, so nothing is kept and no form is learned
    assert!(!got.contains_key(&id(6)));
    let tmx_rows = cache.outcomes(&bagholder_core::SourceName::named("tmx")).unwrap();
    assert!(tmx_rows.iter().any(|o| o.instrument == Some(id(6)) && o.outcome == OutcomeKind::NotCarried && o.kind == DataKind::Quote));
    // the form TMX answered for on the book's venue is written back as a route
    let routes = |n: u8| book.instrument_refs(id(n)).unwrap().into_iter().filter(|r| r.scheme == RefScheme::TmxForm).map(|r| r.value).collect::<Vec<_>>();
    assert_eq!(routes(1), vec!["ENB".to_string()]);
    assert!(routes(6).is_empty());
}
