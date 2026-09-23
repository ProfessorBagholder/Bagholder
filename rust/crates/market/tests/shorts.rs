//! Short selling, which regulator answers for a
//! listing, what each report says, and the figure the app derives from them.

use bagholder_market::shorts::{self, CaPositionRow, CaVolumeRow, Position, UsVolumeRow};
use bagholder_store::feeds::{ShortMarket, VolumeSpan};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

const US_FILE: &str = "Date|Symbol|ShortVolume|ShortExemptVolume|TotalVolume|Market\r\n\
20260914|A|380591.095732|11|631970.726692|B,Q,N\r\n\
20260914|GME|2250985.897562|1942|3542062.253804|B,Q,N\r\n\
20260914|NOVOL|0|0|0|Q\r\n\
20260914|SHORT\r\n";

const CA_CSV: &str = "Security,Company Name,Listing Market,Short Sale Trades,% Total Trades,Short Traded Volume,% Total Traded Volume,Short Traded Value,% Total Traded Value\r\n\
QNC,Quantum Emotion Corp.,TSXV,6364,33.528,1197633,21.319,3453172,21.028\r\n\
TGIF,1933 Industries Inc.,CSE,12,1.5,5000,0,20,0\r\n";

const TODAY: &str = "2026-09-15";

fn ca_grid() -> Vec<Vec<Value>> {
    vec![
        vec![json!(""), json!(""), json!(""), json!(""), json!("")],
        vec![json!("Security Issue Name"), json!("Security Symbol"), json!("Exchange Code"), json!("No.Shares"), json!("Net Change")],
        vec![json!("QUANTUM EMOTION CORP."), json!("QNC"), json!("TSXV"), json!(2667164.0), json!(64077.0)],
        vec![json!("1933 INDUSTRIES INC."), json!("TGIF"), json!("CSE"), json!(72000.0), json!(68990.0)],
        vec![json!("ROW WITH NO SHARES"), json!("NIL"), json!("TSX"), json!(""), json!("")],
        vec![json!("SHORT ROW"), json!("OOPS")],
    ]
}

/// The caches are process-wide; tests that touch them
/// take turns.
fn serial() -> MutexGuard<'static, ()> {
    static M: Mutex<()> = Mutex::new(());
    let g = M.lock().unwrap_or_else(|e| e.into_inner());
    shorts::clear_files();
    shorts::clear_floats();
    g
}

fn conn() -> rusqlite::Connection {
    let c = rusqlite::Connection::open_in_memory().unwrap();
    bagholder_store::schema::init_schema(&c).unwrap();
    c
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{} != {}", a, b);
}

// --- RoutingTest ---------------------------------------------------------------

#[test]
fn test_each_market_goes_to_the_regulator_that_publishes_for_it() {
    assert_eq!(shorts::market_of("GME", "NYSE", "USD"), Some(ShortMarket::Us));
    assert_eq!(shorts::market_of("AAPL", "NASDAQ", "USD"), Some(ShortMarket::Us));
    assert_eq!(shorts::market_of("QNC", "TSX-V", "CAD"), Some(ShortMarket::Ca));
    assert_eq!(shorts::market_of("TGIF", "CSE", "CAD"), Some(ShortMarket::Ca));
    assert_eq!(shorts::market_of("HBIX", "Cboe Canada", "CAD"), Some(ShortMarket::Ca));
}

#[test]
fn test_a_venue_the_book_does_not_name_follows_the_currency_as_the_quotes_do() {
    assert_eq!(shorts::market_of("SHOP", "", "CAD"), Some(ShortMarket::Ca));
    assert_eq!(shorts::market_of("F", "", "USD"), Some(ShortMarket::Us));
}

#[test]
fn test_nothing_is_claimed_for_an_instrument_no_one_reports() {
    assert_eq!(shorts::market_of("BTC", "Crypto", "USD"), None);
    assert_eq!(shorts::market_of("SPX", "Index", "USD"), None);
    assert_eq!(shorts::market_of("ES", "CME", "USD"), None);
    assert_eq!(shorts::market_of("AAPL  260117C00150000", "NASDAQ", "USD"), None);
    assert_eq!(shorts::market_of("", "NYSE", "USD"), None);
}

// --- ReportDateTest ------------------------------------------------------------

#[test]
fn test_positions_are_reported_on_the_fifteenth_and_the_last_day() {
    assert_eq!(shorts::position_dates("2026-09-15", 4), vec!["2026-09-15", "2026-08-31", "2026-08-15", "2026-07-31"]);
}

#[test]
fn test_a_date_still_to_come_is_never_asked_for() {
    assert_eq!(shorts::position_dates("2026-09-03", 2), vec!["2026-08-31", "2026-08-15"]);
}

#[test]
fn test_the_turn_of_the_year_steps_back_into_december() {
    assert_eq!(shorts::position_dates("2026-01-05", 2), vec!["2025-12-31", "2025-12-15"]);
}

#[test]
fn test_volume_periods_are_the_two_halves_of_each_month() {
    let p = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(
        shorts::volume_periods("2026-09-15", 3),
        vec![p("2026-09-01", "2026-09-15"), p("2026-08-16", "2026-08-31"), p("2026-08-01", "2026-08-15")]
    );
}

#[test]
fn test_the_daily_file_is_only_looked_for_on_weekdays() {
    assert_eq!(shorts::trading_days("2026-09-15", 4), vec!["2026-09-15", "2026-09-14", "2026-09-11", "2026-09-10"]);
}

// --- ParseTest -----------------------------------------------------------------

#[test]
fn test_the_daily_us_file_gives_the_short_part_of_each_symbols_volume() {
    let rows = shorts::parse_us_volume(US_FILE);
    assert_eq!(rows["GME"], UsVolumeRow { short_volume: 2250985.897562, total_volume: 3542062.253804 });
    assert!(!rows.contains_key("NOVOL"));
    assert!(!rows.contains_key("SHORT"));
    assert!(!rows.contains_key("Symbol"));
}

#[test]
fn test_the_canadian_position_report_gives_shares_short_and_the_change() {
    let rows = shorts::parse_ca_positions(&ca_grid());
    assert_eq!(rows["QNC"], CaPositionRow { venue: "TSXV".into(), shares: 2667164.0, change: Some(64077.0), name: "QUANTUM EMOTION CORP.".into() });
    assert_eq!(rows["TGIF"].venue, "CSE");
    assert!(!rows.contains_key("NIL"));
    assert!(!rows.contains_key("OOPS"));
    assert!(!rows.contains_key("Security Symbol"));
}

#[test]
fn test_the_canadian_volume_report_gives_the_short_share_of_trading() {
    let rows = shorts::parse_ca_volume(CA_CSV).unwrap();
    assert_eq!(rows["QNC"].short_volume, 1197633.0);
    assert_eq!(rows["QNC"].volume_pct, Some(21.319));
    approx(rows["QNC"].total_volume.unwrap(), 1197633.0 / 21.319 * 100.0);
    assert!(rows["TGIF"].total_volume.is_none());
}

#[test]
fn test_a_row_is_only_used_for_the_venue_it_was_filed_under() {
    assert!(shorts::venue_fits("TSXV", "TSX-V"));
    assert!(shorts::venue_fits("AQL", "Cboe Canada"));
    assert!(shorts::venue_fits("TSX", ""));
    assert!(!shorts::venue_fits("TSX", "CSE"));
}

// --- ListingTest ---------------------------------------------------------------

#[test]
fn test_the_newest_settlement_finra_has_is_the_one_shown() {
    let answered = json!([
        {"settlementDate": "2026-08-14", "currentShortPositionQuantity": 54036583, "previousShortPositionQuantity": 53736062, "changePreviousNumber": 300521},
        {"settlementDate": "2026-08-31", "currentShortPositionQuantity": 56990026, "previousShortPositionQuantity": 54036583, "changePreviousNumber": 2953443, "averageDailyVolumeQuantity": 5864237},
        "not a row"
    ]);
    let out = shorts::parse_us_position(&answered);
    assert_eq!(
        (out.as_of.as_str(), out.shares, out.previous, out.change, out.previous_of.as_str(), out.average_volume),
        ("2026-08-31", Some(56990026.0), Some(54036583.0), Some(2953443.0), "2026-08-14", Some(5864237.0))
    );
}

#[test]
fn test_a_symbol_finra_does_not_carry_answers_nothing_rather_than_guessing() {
    assert_eq!(shorts::parse_us_position(&json!([])), Position::default());
}

#[test]
fn test_a_whole_market_file_is_read_once_and_used_for_every_listing() {
    let _g = serial();
    let calls = Cell::new(0);
    let get = |_: &str| -> Result<String, String> {
        calls.set(calls.get() + 1);
        Ok(US_FILE.to_string())
    };
    let first = shorts::us_volume_with("GME", || shorts::us_volume_file_with(TODAY, get)).unwrap();
    let second = shorts::us_volume_with("A", || shorts::us_volume_file_with(TODAY, get)).unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(first.volume_of, "2026-09-15");
    approx(first.volume_pct.unwrap(), 2250985.897562 / 3542062.253804 * 100.0);
    assert_eq!(second.volume_span, VolumeSpan::Day);
}

#[test]
fn test_a_file_that_will_not_answer_keeps_what_was_already_read() {
    let _g = serial();
    shorts::us_volume_with("GME", || shorts::us_volume_file_with(TODAY, |_| Ok(US_FILE.to_string())));
    shorts::age_file("us_volume", shorts::FILE_HOURS * 3600 + 1);
    let kept = shorts::us_volume_with("GME", || shorts::us_volume_file_with(TODAY, |_| Err("down".to_string()))).unwrap();
    assert_eq!(kept.short_volume, Some(2250985.897562));
}

/// `for_listing` for a Canadian listing on fixed files.
fn ca_listing(c: &rusqlite::Connection, sym: &str, exchange: &str, trend: bool) -> bagholder_store::feeds::Shorts {
    let position = shorts::ca_position_with(sym, exchange, TODAY, || shorts::ca_position_file_with(TODAY, |_| Ok(ca_grid())));
    let volume = shorts::ca_volume_with(sym, exchange, || shorts::ca_volume_file_with(TODAY, |_| Ok(CA_CSV.to_string())), |_| None);
    shorts::finish(c, position, volume, sym, exchange, trend, ShortMarket::Ca, |_| vec![], |_| None)
}

#[test]
fn test_a_canadian_listing_reads_both_of_its_reports() {
    let _g = serial();
    let c = conn();
    let rec = ca_listing(&c, "QNC", "TSX-V", false);
    assert_eq!(rec.shares, Some(2667164.0));
    assert_eq!(rec.previous, Some(2603087.0));
    assert_eq!(rec.as_of, "2026-09-15");
    assert_eq!(rec.volume_of, "2026-09-01/2026-09-15");
    assert_eq!(rec.volume_span, Some(VolumeSpan::Period));
    assert_eq!(rec.previous_of, "2026-08-31");
}

#[test]
fn test_a_listing_filed_under_another_venue_is_not_read_as_this_one() {
    let _g = serial();
    let c = conn();
    let rec = ca_listing(&c, "QNC", "CSE", false);
    assert!(rec.shares.is_none());
    assert_eq!(rec.market, ShortMarket::Ca);
}

#[test]
fn test_nothing_is_read_for_an_instrument_no_one_reports() {
    let c = conn();
    assert_eq!(shorts::for_listing(&c, "BTC", "Crypto", "USD", TODAY, false, ""), None);
}

#[test]
fn test_days_to_cover_uses_the_volume_of_the_listings_own_market() {
    let c = conn();
    let us = bagholder_store::feeds::Shorts { market: ShortMarket::Us, shares: Some(56990026.0), average_volume: Some(5864237.0), ..Default::default() };
    assert_eq!(shorts::average_volume(&c, &us), Some(5864237.0));
    assert_eq!(shorts::days_to_cover(&c, &us), Some(9.7));
}

#[test]
fn test_the_canadian_average_counts_only_the_days_the_market_traded() {
    let c = conn();
    for day in ["2026-08-17", "2026-08-18", "2026-08-19", "2026-08-20", "2026-08-21"] {
        bagholder_store::tables::upsert_benchmark_prices(&c, Some(&json!({day: 100.0})), "TSX").unwrap();
    }
    let ca = bagholder_store::feeds::Shorts { market: ShortMarket::Ca, shares: Some(2667164.0), total_volume: Some(5000000.0), volume_of: "2026-08-16/2026-08-31".into(), ..Default::default() };
    let days = bagholder_store::tables::benchmark_days(&c, "TSX", "2026-08-16", "2026-08-31").unwrap();
    assert_eq!(days, 5);
    approx(shorts::average_volume(&c, &ca).unwrap(), 5000000.0 / days as f64);
    // no calendar stored: nothing is guessed
    assert_eq!(shorts::average_volume(&conn(), &ca), None);
}

#[test]
fn test_no_position_or_no_volume_leaves_days_to_cover_unsaid() {
    let c = conn();
    let a = bagholder_store::feeds::Shorts { market: ShortMarket::Us, shares: None, average_volume: Some(10.0), ..Default::default() };
    assert_eq!(shorts::days_to_cover(&c, &a), None);
    let b = bagholder_store::feeds::Shorts { market: ShortMarket::Us, shares: Some(10.0), average_volume: None, ..Default::default() };
    assert_eq!(shorts::days_to_cover(&c, &b), None);
    let d = bagholder_store::feeds::Shorts { market: ShortMarket::Ca, total_volume: None, volume_of: "2026-08-16/2026-08-31".into(), ..Default::default() };
    assert_eq!(shorts::average_volume(&c, &d), None);
}

// --- SeriesTest ----------------------------------------------------------------

#[test]
fn test_every_settlement_finra_answered_with_is_kept_oldest_first() {
    let answered = json!([
        {"settlementDate": "2026-08-31", "currentShortPositionQuantity": 3},
        {"settlementDate": "2026-07-31", "currentShortPositionQuantity": 1},
        {"settlementDate": "2026-08-14", "currentShortPositionQuantity": 2},
        {"settlementDate": "2026-06-30", "currentShortPositionQuantity": null}
    ]);
    let series = shorts::parse_us_position(&answered).series.unwrap();
    let dates: Vec<&str> = series.iter().map(|p| p.date.as_str()).collect();
    let shares: Vec<f64> = series.iter().map(|p| p.shares).collect();
    assert_eq!(dates, vec!["2026-07-31", "2026-08-14", "2026-08-31"]);
    assert_eq!(shares, vec![1.0, 2.0, 3.0]);
}

fn rows_on(grid: Result<Vec<Vec<Value>>, String>) -> HashMap<String, CaPositionRow> {
    grid.map(|g| shorts::parse_ca_positions(&g)).unwrap_or_default()
}

#[test]
fn test_the_canadian_run_reads_one_file_per_reporting_date() {
    let grids = |day: &str| -> Result<Vec<Vec<Value>>, String> {
        let shares = match day {
            "2026-08-31" => 300.0,
            "2026-08-15" => 200.0,
            "2026-07-31" => 100.0,
            _ => return Err("no report".into()),
        };
        Ok(vec![vec![json!(""), json!("QNC"), json!("TSXV"), json!(shares), json!(0.0)]])
    };
    let series = shorts::ca_series_with("QNC", "TSX-V", "2026-08-31", TODAY, shorts::SERIES, |d| rows_on(grids(d)));
    let got: Vec<(String, f64)> = series.iter().map(|p| (p.date.clone(), p.shares)).collect();
    assert_eq!(got, vec![("2026-07-31".to_string(), 100.0), ("2026-08-15".to_string(), 200.0), ("2026-08-31".to_string(), 300.0)]);
}

#[test]
fn test_a_report_after_the_one_on_show_is_not_drawn() {
    let s = shorts::ca_series_with("QNC", "TSX-V", "2026-07-31", TODAY, shorts::SERIES, |_| rows_on(Err("none".into())));
    assert!(s.is_empty());
}

#[test]
fn test_a_listing_on_another_venue_is_not_drawn_into_this_ones_run() {
    let grid = || Ok(vec![vec![json!(""), json!("QNC"), json!("CSE"), json!(300.0), json!(0.0)]]);
    let s = shorts::ca_series_with("QNC", "TSX-V", "2026-08-31", TODAY, shorts::SERIES, |_| rows_on(grid()));
    assert!(s.is_empty());
}

#[test]
fn test_the_canadian_run_is_only_read_when_it_is_asked_for() {
    let _g = serial();
    let c = conn();
    let asked = Cell::new(false);
    let position = shorts::ca_position_with("QNC", "TSX-V", TODAY, || shorts::ca_position_file_with(TODAY, |_| Ok(ca_grid())));
    let quiet = shorts::finish(&c, position, None, "QNC", "TSX-V", false, ShortMarket::Ca, |_| { asked.set(true); vec![] }, |_| None);
    assert!(quiet.series.is_none());
    assert!(!asked.get());
}

// --- FloatTest -----------------------------------------------------------------

fn stats(value: Value) -> String {
    json!({"quoteSummary": {"result": [{"defaultKeyStatistics": {"floatShares": {"raw": value}}}]}}).to_string()
}

/// A fake session: the first answer whose mark is in the URL, else a 404.
struct Yahoo {
    answers: Vec<(&'static str, String, u16)>,
    asked: RefCell<Vec<String>>,
}

impl Yahoo {
    fn new(answers: Vec<(&'static str, String, u16)>) -> Self {
        Yahoo { answers, asked: RefCell::new(vec![]) }
    }
    fn get(&self, form: &str) -> Option<(u16, String)> {
        let url = shorts::YAHOO_STATS_URL.replacen("{}", form, 1).replacen("{}", "abc", 1);
        self.asked.borrow_mut().push(url.clone());
        for (mark, payload, status) in &self.answers {
            if url.contains(mark) {
                return Some((*status, payload.clone()));
            }
        }
        Some((404, "{}".to_string()))
    }
    fn asked_with(&self, mark: &str) -> usize {
        self.asked.borrow().iter().filter(|u| u.contains(mark)).count()
    }
}

fn float(y: &Yahoo, sym: &str, ex: &str, ccy: &str, name: &str) -> Option<f64> {
    shorts::float_shares_with(sym, ex, ccy, name, true, |f| y.get(f), || true, || {}, |_, _| None)
}

#[test]
fn test_the_float_is_read_under_the_venues_own_symbol() {
    let _g = serial();
    let y = Yahoo::new(vec![("QNC.V", stats(json!(212448707)), 200)]);
    assert_eq!(float(&y, "QNC", "TSX-V", "CAD", ""), Some(212448707.0));
    assert!(y.asked_with("QNC.V") > 0);
}

#[test]
fn test_a_form_the_source_does_not_carry_falls_to_the_next() {
    let _g = serial();
    let y = Yahoo::new(vec![("QNC.TO", "{}".into(), 404), ("QNC.V", stats(json!(1000.0)), 200)]);
    assert_eq!(float(&y, "QNC", "TSX-V", "CAD", ""), Some(1000.0));
}

#[test]
fn test_a_listing_with_no_float_published_reports_none() {
    let _g = serial();
    let y = Yahoo::new(vec![("HBIX", stats(Value::Null), 200)]);
    assert_eq!(float(&y, "HBIX", "Cboe Canada", "CAD", ""), None);
}

#[test]
fn test_it_is_read_once_and_kept() {
    let _g = serial();
    let y = Yahoo::new(vec![("GME", stats(json!(463550645)), 200)]);
    float(&y, "GME", "NYSE", "USD", "");
    float(&y, "GME", "NYSE", "USD", "");
    assert_eq!(y.asked_with("GME"), 1);
}

#[test]
fn test_without_the_browser_client_the_float_is_simply_unknown() {
    let _g = serial();
    let got = shorts::float_shares_with("GME", "NYSE", "USD", "", false, |_| panic!("asked"), || true, || {}, |_, _| None);
    assert_eq!(got, None);
}

#[test]
fn test_the_position_is_measured_against_the_float() {
    let c = conn();
    let position = Position { shares: Some(100.0), as_of: "2026-08-31".into(), ..Position::default() };
    let out = shorts::finish(&c, position, None, "GME", "NYSE", false, ShortMarket::Us, |_| vec![], |_| Some(400.0));
    assert_eq!(out.float, Some(400.0));
    assert_eq!(out.of_float, Some(25.0));
}

// --- FundFloatTest -------------------------------------------------------------

#[test]
fn test_a_company_with_no_float_published_is_never_given_its_share_count() {
    let _g = serial();
    let answer = json!({"quoteSummary": {"result": [{"defaultKeyStatistics": {"floatShares": null, "sharesOutstanding": {"raw": 500.0}}}]}}).to_string();
    let y = Yahoo::new(vec![("QNC", answer, 200)]);
    let got = shorts::float_shares_with("QNC", "TSX-V", "CAD", "Quantum eMotion Corp", true, |f| y.get(f), || true, || {}, |_, _| panic!("asked for units"));
    assert_eq!(got, None);
}

#[test]
fn test_a_fund_falls_back_to_the_units_the_exchange_publishes() {
    let _g = serial();
    let answer = json!({"quoteSummary": {"result": [{"defaultKeyStatistics": {"floatShares": null, "sharesOutstanding": null}}]}}).to_string();
    let y = Yahoo::new(vec![("RDDY", answer, 200)]);
    let got = shorts::float_shares_with("RDDY", "TSX", "CAD", "Harvest Reddit Enhanced High Income Shares ETF", true, |f| y.get(f), || true, || {}, |_, _| Some(20075000.0));
    assert_eq!(got, Some(20075000.0));
}

#[test]
fn test_a_us_fund_takes_the_count_from_the_same_answer_as_the_float() {
    let _g = serial();
    let answer = json!({"quoteSummary": {"result": [{"defaultKeyStatistics": {"floatShares": null, "sharesOutstanding": {"raw": 4000.0}}}]}}).to_string();
    let y = Yahoo::new(vec![("SPY", answer, 200)]);
    assert_eq!(float(&y, "SPY", "NYSE", "USD", "SPDR S&P 500 ETF Trust"), Some(4000.0));
}

#[test]
fn test_a_float_that_is_published_is_still_what_a_fund_is_measured_against() {
    let _g = serial();
    let answer = json!({"quoteSummary": {"result": [{"defaultKeyStatistics": {"floatShares": {"raw": 111.0}, "sharesOutstanding": {"raw": 999.0}}}]}}).to_string();
    let y = Yahoo::new(vec![("XIU", answer, 200)]);
    assert_eq!(float(&y, "XIU", "TSX", "CAD", "iShares S&P/TSX 60 Index ETF"), Some(111.0));
}

// --- SearchedListingTest -------------------------------------------------------

fn searched(exchange: &str, name: &str) -> (bagholder_store::feeds::Shorts, Vec<String>) {
    let _g = serial();
    let c = conn();
    let mut rows = HashMap::new();
    rows.insert("SHOP".to_string(), CaPositionRow { venue: "TSX".into(), shares: 7573829.0, change: Some(41206.0), name: "SHOPIFY INC. CL 'A' SV".into() });
    let position = shorts::ca_position_from("2026-08-31", &rows, "SHOP", exchange, TODAY);
    let seen = RefCell::new(vec![]);
    let out = shorts::finish(&c, position, None, "SHOP", exchange, false, ShortMarket::Ca, |_| vec![], |issuer| {
        // for_listing hands the book's own name over the report's
        let nm = if name.trim().is_empty() { issuer } else { name.trim() };
        seen.borrow_mut().push(nm.to_string());
        Some(1000000.0)
    });
    (out, seen.into_inner())
}

#[test]
fn test_the_report_names_the_venue_and_the_issuer_where_the_book_knows_neither() {
    let (rec, seen) = searched("", "");
    assert_eq!((rec.exchange.as_str(), rec.name.as_str()), ("TSX", "SHOPIFY INC. CL 'A' SV"));
    assert_eq!(seen, vec!["SHOPIFY INC. CL 'A' SV"]);
}

#[test]
fn test_the_book_own_name_for_a_listing_it_carries_is_the_one_the_float_is_read_under() {
    let (rec, seen) = searched("TSX", "Shopify Inc.");
    assert_eq!(rec.exchange, "TSX");
    assert_eq!(seen, vec!["Shopify Inc."]);
}

// --- CboeUnitsTest -------------------------------------------------------------

fn directory() -> String {
    json!({"data": [
        {"symbol": "HBIX", "name": "HARVEST BITCOIN ENHANCED INCOME ETF", "security": "etf", "marketcap": 44091000.0, "last": 6.39},
        {"symbol": "BCBN", "name": "A COMPANY", "security": "equity", "marketcap": 100876283.0, "last": 1.0},
        {"symbol": "NOPR", "name": "NO PRICE ETF", "security": "etf", "marketcap": 500.0, "last": 0.0},
        {"symbol": "ODDS", "name": "NOT A WHOLE COUNT ETF", "security": "etf", "marketcap": 100.0, "last": 3.0}
    ]})
    .to_string()
}

#[test]
fn test_the_count_is_the_capitalisation_over_the_price_for_the_venues_own_funds() {
    let _g = serial();
    let calls = Cell::new(0);
    // TMX carries no count for these
    let units = |sym: &str| {
        shorts::fund_units_with(sym, "CBOE CANADA", "CAD", |_| None, |s| shorts::cboe_units_with(s, || { calls.set(calls.get() + 1); Some(directory()) }))
    };
    assert_eq!(units("HBIX"), Some(6900000.0));
    assert_eq!(units("BCBN"), None, "a company's shares in issue are not its float");
    assert_eq!(units("NOPR"), None, "no price, no count");
    assert_eq!(units("ODDS"), None, "a count that is not whole is not the exchange's own");
    assert_eq!(calls.get(), 1, "one directory for every listing looked up in it");
}

#[test]
fn test_a_listing_on_another_venue_never_takes_a_count_from_this_one() {
    let got = shorts::fund_units_with("HBIX", "TSX", "CAD", |_| Some(0.0), |_| panic!("asked anyway"));
    assert_eq!(got, None);
}

#[test]
fn test_the_venue_is_asked_only_where_tmx_has_no_count() {
    let got = shorts::fund_units_with("XYZ", "CBOE CANADA", "CAD", |_| Some(4200.0), |_| panic!("asked anyway"));
    assert_eq!(got, Some(4200.0));
}

// --- FloatCacheTest ------------------------------------------------------------

#[test]
fn test_a_figure_is_read_once_and_kept() {
    let _g = serial();
    let y = Yahoo::new(vec![("GME", stats(json!(463550645)), 200)]);
    float(&y, "GME", "NYSE", "USD", "GameStop Corp.");
    float(&y, "GME", "NYSE", "USD", "GameStop Corp.");
    assert_eq!(y.asked_with("GME"), 1);
}

#[test]
fn test_a_lookup_that_answered_with_nothing_is_asked_again() {
    let _g = serial();
    let empty = Yahoo::new(vec![("ASTS", stats(Value::Null), 200)]);
    assert_eq!(float(&empty, "ASTS", "NASDAQ", "USD", "AST SpaceMobile Inc."), None);
    shorts::age_float("ASTS|NASDAQ", shorts::FLOAT_MISS_MIN * 60 + 1);
    let good = Yahoo::new(vec![("ASTS", stats(json!(266440743)), 200)]);
    assert_eq!(float(&good, "ASTS", "NASDAQ", "USD", "AST SpaceMobile Inc."), Some(266440743.0));
}

#[test]
fn test_a_figure_is_not_asked_again_that_soon() {
    let _g = serial();
    let y = Yahoo::new(vec![("GME", stats(json!(463550645)), 200)]);
    float(&y, "GME", "NYSE", "USD", "GameStop Corp.");
    shorts::age_float("GME|NYSE", shorts::FLOAT_MISS_MIN * 60 + 1);
    float(&y, "GME", "NYSE", "USD", "GameStop Corp.");
    assert_eq!(y.asked_with("GME"), 1);
}

// --- YahooPaceTest -------------------------------------------------------------

#[test]
fn test_each_lookup_takes_its_turn_and_leaves_the_next_slot() {
    let _g = serial();
    let y = Yahoo::new(vec![("GME", stats(json!(1.0)), 200)]);
    let turns = Cell::new(0);
    shorts::float_shares_with("GME", "NYSE", "USD", "GameStop Corp.", true, |f| y.get(f), || { turns.set(turns.get() + 1); true }, || {}, |_, _| None);
    assert_eq!(turns.get(), 1, "the lookup takes Yahoo's turn");
}

#[test]
fn test_nothing_is_asked_while_a_backoff_stands() {
    let _g = serial();
    let y = Yahoo::new(vec![("GME", stats(json!(1.0)), 200)]);
    let got = shorts::float_shares_with("GME", "NYSE", "USD", "GameStop Corp.", true, |f| y.get(f), || false, || {}, |_, _| None);
    assert_eq!(got, None);
    assert_eq!(y.asked_with("quoteSummary"), 0);
}

#[test]
fn test_a_refusal_starts_the_backoff_the_whole_app_honours() {
    let _g = serial();
    let y = Yahoo::new(vec![("GME", "{}".into(), 429)]);
    let backed = Cell::new(false);
    let got = shorts::float_shares_with("GME", "NYSE", "USD", "GameStop Corp.", true, |f| y.get(f), || true, || backed.set(true), |_, _| None);
    assert_eq!(got, None);
    assert!(backed.get());
}

// --- FloatFormTest -------------------------------------------------------------

#[test]
fn test_a_us_listing_with_no_currency_on_its_row_is_still_asked_for_as_one() {
    let _g = serial();
    let y = Yahoo::new(vec![("quoteSummary/ASTS?", stats(json!(266440743)), 200)]);
    assert_eq!(float(&y, "ASTS", "NASDAQ", "", "AST SpaceMobile Inc."), Some(266440743.0));
    assert!(y.asked.borrow().iter().all(|u| !u.contains(".TO") && !u.contains(".V")), "asked under a Canadian suffix");
}

#[test]
fn test_a_canadian_listing_with_no_currency_keeps_its_own_suffixes() {
    let _g = serial();
    let y = Yahoo::new(vec![("QNC.V", stats(json!(212448707)), 200)]);
    assert_eq!(float(&y, "QNC", "TSX-V", "", "Quantum eMotion Corp"), Some(212448707.0));
    assert!(y.asked_with("QNC.V") > 0);
}

// --- reading the version -------------------------------------------------------

#[test]
fn test_a_record_read_on_the_spot_names_its_listing() {
    let c = conn();
    let position = Position { shares: Some(1.0), as_of: "2026-08-31".into(), ..Position::default() };
    let out = shorts::finish(&c, position, None, "RKLB", "NASDAQ", false, ShortMarket::Us, |_| vec![], |_| None);
    assert_eq!(out.exchange, "NASDAQ");
}

// --- ReportOmitsListingTest ----------------------------------------------------

fn warm(rows: HashMap<String, CaVolumeRow>) {
    shorts::warm_ca_volume("ca_volume", "2026-08-16/2026-08-31", rows);
}

#[test]
fn test_a_listing_the_report_omits_reads_as_none_of_its_trading() {
    let _g = serial();
    warm(HashMap::new());
    let out = shorts::ca_volume_with("YES", "TSX-V", || panic!("read again"), |_| Some(1519546.0)).unwrap();
    assert_eq!(out.short_volume, Some(0.0));
    assert_eq!(out.volume_pct, Some(0.0));
    assert_eq!(out.total_volume, Some(1519546.0));
}

#[test]
fn test_a_listing_the_report_omits_and_the_exchange_has_no_volume_for_says_nothing() {
    let _g = serial();
    warm(HashMap::new());
    assert_eq!(shorts::ca_volume_with("YES", "TSX-V", || panic!("read again"), |_| None), None);
}

#[test]
fn test_a_listing_the_report_carries_is_read_from_the_report() {
    let _g = serial();
    let mut rows = HashMap::new();
    rows.insert("QNC".to_string(), CaVolumeRow { venue: "TSXV".into(), short_volume: 1197633.0, volume_pct: Some(21.319), total_volume: Some(5617679.0) });
    warm(rows);
    let out = shorts::ca_volume_with("QNC", "TSX-V", || panic!("read again"), |_| panic!("asked the exchange anyway")).unwrap();
    assert_eq!(out.volume_pct, Some(21.319));
}

#[test]
fn test_days_to_cover_follows_from_what_the_exchange_says_was_traded() {
    let c = conn();
    let prices: serde_json::Map<String, Value> = (17..28).map(|d| (format!("2026-08-{:02}", d), json!(100.0))).collect();
    bagholder_store::tables::upsert_benchmark_prices(&c, Some(&Value::Object(prices)), "TSX").unwrap();
    let rec = bagholder_store::feeds::Shorts { market: ShortMarket::Ca, shares: Some(17873.0), total_volume: Some(1519546.0), volume_of: "2026-08-16/2026-08-31".into(), ..Default::default() };
    let days = bagholder_store::tables::benchmark_days(&c, "TSX", "2026-08-16", "2026-08-31").unwrap();
    assert!(days > 0);
    approx(shorts::average_volume(&c, &rec).unwrap(), 1519546.0 / days as f64);
    assert!(shorts::days_to_cover(&c, &rec).is_some());
}

