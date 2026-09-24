//! Yahoo's chart, TMX and FRED, read from recorded real replies: closes stored as
//! traded, levels and distributions exactly as stated, and every wrong copy
//! refused.

mod common;

use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec};
use bagholder_sources::adapters::{fred, tmx, yahoo};
use bagholder_sources::outcome::{Outcome, OutcomeKind};

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn t(s: &str) -> Timestamp {
    s.parse().unwrap()
}

const Y: &str = "yahoo";
const TMX: &str = "tmx";
const FRED: &str = "fred";

#[test]
fn the_recorded_shapes_are_the_answers_union() {
    common::shape_is_the_answers_union("yahoo-chart.paths", Y, "", yahoo::KEYED);
    common::shape_is_the_answers_union("tmx-quote.paths", TMX, "quote-", &[]);
    common::shape_is_the_answers_union("tmx-dividends.paths", TMX, "dividends-", &[]);
    common::shape_is_the_answers_union("tmx-series.paths", TMX, "series-", &[]);
}

fn chart(name: &str, symbol: &str, now: &str) -> yahoo::Chart {
    match yahoo::parse(&common::json(Y, name), symbol, t(now)) {
        Outcome::Answered(c) => c,
        other => panic!("{name}: {other:?}"),
    }
}

fn close(c: &yahoo::Chart, d: Date) -> Option<Dec> {
    c.closes.iter().find(|(day, _)| *day == d).map(|(_, v)| *v)
}

#[test]
fn closes_are_stored_as_traded_with_the_replys_own_splits_undone() {
    let nvda = chart("NVDA-2024-05-28-2024-06-14.json", "NVDA", "2026-09-24T04:00:00Z");
    assert_eq!(nvda.currency, Currency::USD);
    assert_eq!(nvda.splits, vec![yahoo::Split { day: date(2024, 6, 10), numerator: dec("10"), denominator: dec("1") }]);
    // before the 10:1 split of 2024-06-10 the adjusted close is multiplied back, exactly
    assert_eq!(close(&nvda, date(2024, 5, 28)), Some(dec("1139.010009765625")));
    // on and after the split day the close is as written
    let on = close(&nvda, date(2024, 6, 10)).unwrap();
    assert!(on < dec("200"), "{on}");
    // the dividend after the split is as declared
    assert_eq!(nvda.dividends, vec![(date(2024, 6, 11), dec("0.01"))]);
    assert!(nvda.closes.windows(2).all(|w| w[0].0 < w[1].0));
}

#[test]
fn a_canadian_listing_and_a_fund_that_has_paid_nothing() {
    let enb = chart("ENB.TO-2026-08-03-2026-09-24.json", "ENB.TO", "2026-09-24T04:00:00Z");
    assert_eq!(enb.currency, Currency::CAD);
    assert_eq!(enb.dividends, vec![(date(2026, 8, 14), dec("0.97"))]);
    assert_eq!(close(&enb, date(2026, 8, 4)), Some(dec("75.08999633789062")));
    let wqtm = chart("WQTM-2025-10-01-2026-09-24.json", "WQTM", "2026-09-24T04:00:00Z");
    assert!(wqtm.dividends.is_empty());
    assert!(!wqtm.closes.is_empty());
    let gspc = chart("GSPC-2016-01-04-2016-02-01.json", "^GSPC", "2026-09-24T04:00:00Z");
    assert_eq!(gspc.closes.first().map(|c| c.0), Some(date(2016, 1, 4)));
}

#[test]
fn the_quote_is_the_regular_price_at_its_own_time_and_a_session_counts_once_closed() {
    let spy = chart("SPY-range-1d.json", "SPY", "2026-09-24T04:00:00Z");
    assert_eq!(spy.quote.price, dec("767.81"));
    assert_eq!(spy.quote.at, t("2026-09-23T20:00:00Z"));
    assert_eq!(spy.quote.change_pct, Some(dec("-0.72")));
    assert_eq!(spy.closes, vec![(date(2026, 9, 23), dec("767.8099975585938"))]);
}

#[test]
fn a_wrong_chart_writes_nothing_and_an_unknown_symbol_is_not_carried() {
    let read = |name: &str, symbol: &str| yahoo::parse(&common::json(Y, name), symbol, t("2026-09-24T04:00:00Z"));
    assert!(matches!(read("wrong-shape-NVDA-close-as-text.json", "NVDA"), Outcome::Mismatch(m) if m.path == "chart.result[0].indicators.quote[0].close[0]"));
    assert!(matches!(read("wrong-meaning-NVDA-another-symbol.json", "NVDA"), Outcome::Meaning(w) if w.contains("AMD")));
    assert!(matches!(read("wrong-meaning-NVDA-split-of-zero.json", "NVDA"), Outcome::Meaning(w) if w.contains("split")));
    let reply = bagholder_net::Reply { status: 404, url: "u".into(), headers: vec![], body: common::read(Y, "ZZZQX-status-404.json").into_bytes(), received_at: t("2026-09-24T00:00:00Z") };
    assert_eq!(bagholder_sources::ask::status(reply, &[404]).kind(), OutcomeKind::NotCarried);
}

fn quote(name: &str, form: &str) -> Outcome<tmx::TmxQuote> {
    tmx::parse_quote(&common::json(TMX, name), form)
}

#[test]
fn a_tmx_quote_states_its_venue_and_schedule() {
    let Outcome::Answered(q) = quote("quote-QCN.json", "QCN") else { panic!() };
    assert_eq!((q.price, q.change, q.currency, q.per_year), (dec("218.31"), Some(dec("-3.48")), Currency::CAD, Some(4)));
    assert_eq!(q.exchange_name, "Toronto Stock Exchange");
    assert!(bagholder_sources::venue::tmx_venue_matches("", &q.exchange_name));
    assert_eq!(quote("quote-ZZZQX-unknown.json", "ZZZQX").kind(), OutcomeKind::NotCarried);
    assert_eq!(quote("quote-ENB-CNX-another-venue.json", "ENB:CNX").kind(), OutcomeKind::NotCarried);
    assert!(matches!(quote("wrong-shape-quote-ENB-price-null.json", "ENB"), Outcome::Mismatch(m) if m.path == "data.getQuoteBySymbol.price"));
    assert!(matches!(quote("wrong-shape-quote-ENB-schedule-unknown.json", "ENB"), Outcome::Mismatch(m) if m.why.contains("Fortnightly")));
    assert!(matches!(quote("wrong-meaning-quote-ENB-another-symbol.json", "ENB"), Outcome::Meaning(w) if w.contains("TRP")));
}

#[test]
fn tmx_distributions_are_cash_when_paid_on_a_date_and_in_units_otherwise() {
    let Outcome::Answered(rows) = tmx::parse_dividends(&common::json(TMX, "dividends-QCN.json"), "QCN") else { panic!() };
    assert_eq!(rows.len(), 40);
    let on = |d: Date| rows.iter().find(|r| r.ex_date == d).copied().unwrap();
    let latest = on(date(2026, 9, 21));
    assert_eq!((latest.cash, latest.in_units, latest.pay_date, latest.record_date), (dec("1.13892"), None, Some(date(2026, 9, 28)), Some(date(2026, 9, 21))));
    // a year-end distribution paid in units: no pay date
    let units = on(date(2023, 12, 28));
    assert_eq!((units.cash, units.in_units), (Dec::ZERO, Some(dec("0.35705"))));
    // a year-end row of nothing
    let nothing = on(date(2025, 12, 31));
    assert_eq!((nothing.cash, nothing.in_units), (Dec::ZERO, None));
    // an unknown symbol lists nothing: whether it is one is the quote's to say
    assert!(matches!(tmx::parse_dividends(&common::json(TMX, "dividends-ZZZQX-unknown.json"), "ZZZQX"), Outcome::Answered(r) if r.is_empty()));
    assert!(matches!(tmx::parse_dividends(&common::json(TMX, "wrong-shape-dividends-QCN-amount-as-text.json"), "QCN"), Outcome::Mismatch(m) if m.path == "data.dividends.dividends[0].amount"));
    assert!(matches!(tmx::parse_dividends(&common::json(TMX, "wrong-meaning-dividends-QCN-paid-before-ex.json"), "QCN"), Outcome::Meaning(w) if w.contains("before")));
}

#[test]
fn an_index_series_is_each_sessions_level_oldest_first() {
    let span = (date(2026, 9, 1), date(2026, 9, 23));
    let Outcome::Answered(tsx) = tmx::parse_series(&common::json(TMX, "series-TSX-2026-09-01-2026-09-23.json"), "^TSX", span) else { panic!() };
    assert_eq!(tsx.last(), Some(&(date(2026, 9, 23), dec("35751.43"))));
    assert!(tsx.windows(2).all(|w| w[0].0 < w[1].0));
    assert!(tsx.iter().all(|(d, _)| *d != date(2026, 9, 7)), "Labour Day");
    let Outcome::Answered(old) = tmx::parse_series(&common::json(TMX, "series-TSX-1995-01-01-2000-12-31.json"), "^TSX", (date(1995, 1, 1), date(2000, 12, 31))) else { panic!() };
    assert!(old.is_empty(), "the series begins 2001-12-11");
    assert!(matches!(tmx::parse_series(&common::json(TMX, "wrong-meaning-series-TSX-oldest-first.json"), "^TSX", span), Outcome::Meaning(_)));
    assert!(matches!(tmx::parse_series(&common::json(TMX, "series-TSX-2026-09-01-2026-09-23.json"), "^TSX", (date(2026, 9, 10), date(2026, 9, 23))), Outcome::Meaning(w) if w.contains("outside")));
}

#[test]
fn fred_states_a_level_for_each_trading_day_and_none_for_a_holiday() {
    let Outcome::Answered(levels) = fred::parse(&common::read(FRED, "SP500.csv")) else { panic!() };
    assert_eq!(levels.first(), Some(&(date(2016, 9, 26), dec("2146.10"))));
    assert!(levels.iter().all(|(d, _)| *d != date(2016, 11, 24)), "Thanksgiving is listed with no value");
    assert_eq!(levels.len(), 2608 - 96);
    assert!(matches!(fred::parse(&common::read(FRED, "wrong-shape-SP500-header.csv")), Outcome::Mismatch(m) if m.path == "header"));
    assert!(matches!(fred::parse(&common::read(FRED, "wrong-meaning-SP500-negative.csv")), Outcome::Meaning(_)));
}

use bagholder_sources::adapters::{cboe_ca, coinbase};

const CB: &str = "coinbase";
const CBX: &str = "coinbase-exchange";
const CBOE: &str = "cboe-canada";

#[test]
fn the_coin_quotes_and_the_cboe_canada_quote_have_shapes_recorded() {
    common::shape_is_the_answers_union("coinbase-spot.paths", CB, "spot-", &[]);
    common::shape_is_the_answers_union("coinbase-exchange-ticker.paths", CBX, "ticker-", &[]);
    common::shape_is_the_answers_union("cboe-canada.paths", CBOE, "quote-", &[]);
}

#[test]
fn a_spot_price_is_stamped_with_its_replys_date_less_what_its_cache_allows() {
    let headers = common::read(CB, "spot-BTC-CAD.json.headers");
    let header = |name: &str| headers.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()));
    let (date, cache) = (header("date"), header("cache-control"));
    let Outcome::Answered(s) = coinbase::parse_spot(&common::json(CB, "spot-BTC-CAD.json"), "BTC", Currency::CAD, date.as_deref(), cache.as_deref()) else { panic!() };
    assert_eq!((s.price, s.at, s.allowance), (dec("118318.2"), t("2026-09-24T04:33:41Z"), std::time::Duration::ZERO));
    // a cache that allows sixty seconds: the price may be that old
    let Outcome::Answered(s) = coinbase::parse_spot(&common::json(CB, "spot-BTC-CAD.json"), "BTC", Currency::CAD, date.as_deref(), Some("public, max-age=60")) else { panic!() };
    assert_eq!((s.at, s.allowance), (t("2026-09-24T04:32:41Z"), std::time::Duration::from_secs(60)));
    // with no date, the price has no time and is refused
    assert!(matches!(coinbase::parse_spot(&common::json(CB, "spot-BTC-CAD.json"), "BTC", Currency::CAD, None, None), Outcome::Meaning(w) if w.contains("no time")));
    let Outcome::Answered(f) = coinbase::parse_spot(&common::json(CB, "spot-FARTCOIN-CAD.json"), "FARTCOIN", Currency::CAD, date.as_deref(), None) else { panic!() };
    assert_eq!(f.price, dec("0.2606720647230226346175"));
    assert!(matches!(coinbase::parse_spot(&common::json(CB, "wrong-shape-spot-amount-as-number.json"), "BTC", Currency::CAD, date.as_deref(), None), Outcome::Mismatch(m) if m.path == "data.amount"));
    assert!(matches!(coinbase::parse_spot(&common::json(CB, "wrong-meaning-spot-another-pair.json"), "BTC", Currency::CAD, date.as_deref(), None), Outcome::Meaning(w) if w.contains("ETH")));
}

#[test]
fn a_coins_close_is_each_ended_utc_days_close() {
    let v = common::json(CBX, "candles-BTC-USD-2026-09-01-2026-09-05.json");
    let Outcome::Answered(days) = coinbase::parse_candles(&v, "BTC-USD", t("2026-09-24T00:00:00Z")) else { panic!() };
    assert_eq!(days.first(), Some(&(date(2026, 9, 1), dec("77398.69"))));
    assert_eq!(days.len(), 5);
    // at noon on the 5th, the 5th has not ended
    let Outcome::Answered(early) = coinbase::parse_candles(&v, "BTC-USD", t("2026-09-05T12:00:00Z")) else { panic!() };
    assert_eq!(early.last().map(|d| d.0), Some(date(2026, 9, 4)));
    assert!(matches!(coinbase::parse_candles(&common::json(CBX, "wrong-shape-candles-five-values.json"), "BTC-USD", t("2026-09-24T00:00:00Z")), Outcome::Mismatch(_)));
    assert!(matches!(coinbase::parse_candles(&common::json(CBX, "wrong-meaning-candles-oldest-first.json"), "BTC-USD", t("2026-09-24T00:00:00Z")), Outcome::Meaning(_)));
    let Outcome::Answered(tick) = coinbase::parse_ticker(&common::json(CBX, "ticker-BTC-USD.json"), "BTC-USD") else { panic!() };
    assert!(tick.price > Dec::ZERO && tick.at > t("2026-09-24T00:00:00Z"));
}

#[test]
fn a_cboe_canada_quote_is_its_last_price_at_its_trade_time() {
    let Outcome::Answered(q) = cboe_ca::parse(&common::json(CBOE, "quote-MAXQ.json"), "MAXQ") else { panic!() };
    assert_eq!((q.price, q.at, q.change), (dec("0.4400"), t("2026-09-23T20:00:00Z"), Some(dec("-0.0150"))));
    assert!(matches!(cboe_ca::parse(&common::json(CBOE, "wrong-shape-quote-MAXQ-last-null.json"), "MAXQ"), Outcome::Mismatch(m) if m.path == "data.last"));
    assert!(matches!(cboe_ca::parse(&common::json(CBOE, "wrong-meaning-quote-MAXQ-another-symbol.json"), "MAXQ"), Outcome::Meaning(w) if w.contains("HBIX")));
}
