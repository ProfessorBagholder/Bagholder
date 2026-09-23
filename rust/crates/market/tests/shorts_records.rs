//! Short selling, pinned end to end: each regulator's parser, the per-listing
//! lookups built on them, the float and fund-count lookups, `finish` for a US
//! and a Canadian record, and what the store keeps and reads back. The
//! answers are held in `golden/shorts_records.json`, so a change of
//! representation must leave every figure as it was.
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-market --test shorts_records`,
//! and read the diff.

use bagholder_market::shorts::{self, CaPositionRow, Position};
use bagholder_store::feeds::ShortMarket;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

fn norm(v: Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.into_iter().map(norm).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, norm(v))).collect::<Map<_, _>>()),
        v => v,
    }
}

fn cell<T: serde::Serialize + ?Sized>(x: &T) -> Value {
    norm(serde_json::to_value(x).unwrap())
}

const TODAY: &str = "2026-09-15";

const US_FILE: &str = "Date|Symbol|ShortVolume|ShortExemptVolume|TotalVolume|Market\r\n\
20260914|A|380591.095732|11|631970.726692|B,Q,N\r\n\
20260914|GME|2250985.897562|1942|3542062.253804|B,Q,N\r\n\
20260914|NOVOL|0|0|0|Q\r\n\
20260914|SHORT\r\n";

const CA_CSV: &str = "Security,Company Name,Listing Market,Short Sale Trades,% Total Trades,Short Traded Volume,% Total Traded Volume,Short Traded Value,% Total Traded Value\r\n\
QNC,Quantum Emotion Corp.,TSXV,6364,33.528,1197633,21.319,3453172,21.028\r\n\
TGIF,1933 Industries Inc.,CSE,12,1.5,5000,0,20,0\r\n";

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

fn directory() -> String {
    json!({"data": [
        {"symbol": "HBIX", "name": "HARVEST BITCOIN ENHANCED INCOME ETF", "security": "etf", "marketcap": 44091000.0, "last": 6.39},
        {"symbol": "BCBN", "name": "A COMPANY", "security": "equity", "marketcap": 100876283.0, "last": 1.0},
        {"symbol": "NOPR", "name": "NO PRICE ETF", "security": "etf", "marketcap": 500.0, "last": 0.0},
        {"symbol": "ODDS", "name": "NOT A WHOLE COUNT ETF", "security": "etf", "marketcap": 100.0, "last": 3.0}
    ]})
    .to_string()
}

fn conn() -> rusqlite::Connection {
    let c = rusqlite::Connection::open_in_memory().unwrap();
    bagholder_store::schema::init_schema(&c).unwrap();
    c
}

fn rows_on(grid: Result<Vec<Vec<Value>>, String>) -> HashMap<String, CaPositionRow> {
    grid.map(|g| shorts::parse_ca_positions(&g)).unwrap_or_default()
}

/// The Canadian position report for a few dated files, as
/// `test_the_canadian_run_reads_one_file_per_reporting_date` in `shorts.rs`
/// exercises.
fn grids(day: &str) -> Result<Vec<Vec<Value>>, String> {
    let shares = match day {
        "2026-09-15" => 400.0,
        "2026-08-31" => 300.0,
        "2026-08-15" => 200.0,
        "2026-07-31" => 100.0,
        _ => return Err("no report".into()),
    };
    Ok(vec![vec![json!(""), json!("QNC"), json!("TSXV"), json!(shares), json!(0.0)]])
}

fn answers() -> Value {
    // every process-global cache starts clear, and the golden test is the
    // only #[test] in this file, so nothing else can have warmed them
    shorts::clear_files();
    shorts::clear_floats();

    let mut out = Map::new();

    // --- the regulators' own parsers -------------------------------------
    out.insert("parse_us_volume".into(), cell(&shorts::parse_us_volume(US_FILE)));
    out.insert("parse_ca_positions".into(), cell(&shorts::parse_ca_positions(&ca_grid())));
    out.insert("parse_ca_volume_ok".into(), cell(&shorts::parse_ca_volume(CA_CSV).unwrap()));
    // a bare CR not followed by LF or CR, inside an unquoted field, is what
    // the CSV reader refuses
    out.insert("parse_ca_volume_err".into(), cell(&shorts::parse_ca_volume("Security\rX,Y").unwrap_err()));
    out.insert("parse_cboe_directory".into(), cell(&shorts::parse_cboe_directory(&directory())));
    out.insert("parse_cboe_directory_not_json".into(), cell(&shorts::parse_cboe_directory("not json")));

    let us_answered = json!([
        {"settlementDate": "2026-08-14", "currentShortPositionQuantity": 54036583, "previousShortPositionQuantity": 53736062, "changePreviousNumber": 300521},
        {"settlementDate": "2026-08-31", "currentShortPositionQuantity": 56990026, "previousShortPositionQuantity": 54036583, "changePreviousNumber": 2953443, "averageDailyVolumeQuantity": 5864237},
        "not a row"
    ]);
    out.insert("parse_us_position".into(), cell(&shorts::parse_us_position(&us_answered)));
    out.insert("parse_us_position_empty".into(), cell(&shorts::parse_us_position(&json!([]))));

    // --- per-listing lookups over a fixed file ----------------------------
    let us_rows = shorts::parse_us_volume(US_FILE);
    out.insert("us_volume_from_present".into(), cell(&shorts::us_volume_from("2026-09-14", &us_rows, "GME")));
    out.insert("us_volume_from_missing".into(), cell(&shorts::us_volume_from("2026-09-14", &us_rows, "NOPE")));

    let ca_pos_rows = shorts::parse_ca_positions(&ca_grid());
    out.insert("ca_position_from_present".into(), cell(&shorts::ca_position_from("2026-08-31", &ca_pos_rows, "QNC", "TSX-V", TODAY)));
    out.insert("ca_position_from_venue_mismatch".into(), cell(&shorts::ca_position_from("2026-08-31", &ca_pos_rows, "QNC", "CSE", TODAY)));
    out.insert("ca_position_from_missing".into(), cell(&shorts::ca_position_from("2026-08-31", &ca_pos_rows, "NOPE", "TSX-V", TODAY)));

    let ca_vol_rows = shorts::parse_ca_volume(CA_CSV).unwrap();
    let qnc_row = ca_vol_rows.get("QNC").cloned();
    let tgif_row = ca_vol_rows.get("TGIF").cloned();
    out.insert("ca_volume_from_present".into(), cell(&shorts::ca_volume_from("2026-09-01/2026-09-15", qnc_row.as_ref(), "TSX-V", || panic!("traded asked while the report carries the listing"))));
    out.insert("ca_volume_from_absent_with_traded".into(), cell(&shorts::ca_volume_from("2026-09-01/2026-09-15", None, "TSX-V", || Some(1519546.0))));
    out.insert("ca_volume_from_absent_without_traded".into(), cell(&shorts::ca_volume_from("2026-09-01/2026-09-15", None, "TSX-V", || None)));
    out.insert("ca_volume_from_venue_mismatch".into(), cell(&shorts::ca_volume_from("2026-09-01/2026-09-15", tgif_row.as_ref(), "TSX-V", || panic!("traded asked though a row was found"))));

    // --- the run of Canadian reports --------------------------------------
    let series = shorts::ca_series_with("QNC", "TSX-V", "2026-08-31", TODAY, shorts::SERIES, |d| rows_on(grids(d)));
    out.insert("ca_series_with".into(), cell(&series));
    let series_other_venue = shorts::ca_series_with("QNC", "CSE", "2026-08-31", TODAY, shorts::SERIES, |d| rows_on(grids(d)));
    out.insert("ca_series_with_other_venue".into(), cell(&series_other_venue));

    // --- fund unit counts --------------------------------------------------
    out.insert("fund_units_hbix".into(), cell(&shorts::fund_units_with("HBIX", "CBOE CANADA", "CAD", |_| None, |s| shorts::cboe_units_with(s, || Some(directory())))));
    out.insert("fund_units_bcbn_company".into(), cell(&shorts::fund_units_with("BCBN", "CBOE CANADA", "CAD", |_| None, |s| shorts::cboe_units_with(s, || Some(directory())))));
    out.insert("fund_units_nopr_no_price".into(), cell(&shorts::fund_units_with("NOPR", "CBOE CANADA", "CAD", |_| None, |s| shorts::cboe_units_with(s, || Some(directory())))));
    out.insert("fund_units_odds_not_whole".into(), cell(&shorts::fund_units_with("ODDS", "CBOE CANADA", "CAD", |_| None, |s| shorts::cboe_units_with(s, || Some(directory())))));
    out.insert("fund_units_other_venue".into(), cell(&shorts::fund_units_with("HBIX", "TSX", "CAD", |_| Some(0.0), |_| panic!("asked the venue for another one's listing"))));
    out.insert("fund_units_tmx_has_it".into(), cell(&shorts::fund_units_with("XYZ", "CBOE CANADA", "CAD", |_| Some(4200.0), |_| panic!("asked the venue when TMX already had a count"))));

    // --- finish: a US record and a Canadian one, with and without trend ---
    let c = conn();
    let us_with_issuer = Position { shares: Some(56990026.0), as_of: "2026-08-31".into(), average_volume: Some(5864237.0), issuer: "GameStop Corp.".into(), ..Position::default() };
    let out_us_full = shorts::finish(&c, us_with_issuer, None, "GME", "NYSE", false, ShortMarket::Us, |_| vec![], |issuer| { assert_eq!(issuer, "GameStop Corp."); Some(400000000.0) });
    out.insert("finish_us_with_issuer_and_float".into(), cell(&out_us_full));

    let us_bare = Position { shares: Some(50.0), as_of: "2026-08-31".into(), ..Position::default() };
    let out_us_bare = shorts::finish(&c, us_bare, None, "RKLB", "NASDAQ", true, ShortMarket::Us, |_| panic!("a US record never asks for a series"), |_| None);
    out.insert("finish_us_no_issuer_no_float".into(), cell(&out_us_bare));

    // a Canadian record built the way `for_listing` builds one, with the
    // market's own trading days seeded so the average and days-to-cover are
    // real figures rather than blanks
    for day in ["2026-09-02", "2026-09-03", "2026-09-04", "2026-09-08", "2026-09-09", "2026-09-10", "2026-09-11"] {
        bagholder_store::tables::upsert_benchmark_prices(&c, Some(&json!({day: 100.0})), "TSX").unwrap();
    }
    let ca_position = shorts::ca_position_with("QNC", "TSX-V", TODAY, || shorts::ca_position_file_with(TODAY, |_| Ok(ca_grid())));
    let ca_volume = shorts::ca_volume_with("QNC", "TSX-V", || shorts::ca_volume_file_with(TODAY, |_| Ok(CA_CSV.to_string())), |_| None);
    let ca_position_for_trend_false = ca_position.clone();
    let ca_volume_for_trend_false = ca_volume.clone();
    let out_ca_trend = shorts::finish(&c, ca_position, ca_volume, "QNC", "TSX-V", true, ShortMarket::Ca, |asof| shorts::ca_series_with("QNC", "TSX-V", asof, TODAY, shorts::SERIES, |d| rows_on(grids(d))), |issuer| { assert_eq!(issuer, "QUANTUM EMOTION CORP."); Some(212448707.0) });
    out.insert("finish_ca_trend".into(), cell(&out_ca_trend));
    let out_ca_no_trend = shorts::finish(&c, ca_position_for_trend_false, ca_volume_for_trend_false, "QNC", "TSX-V", false, ShortMarket::Ca, |_| panic!("trend is off: the series is never asked for"), |_| Some(212448707.0));
    out.insert("finish_ca_no_trend".into(), cell(&out_ca_no_trend));

    // --- store round trip ---------------------------------------------------
    let store_conn = conn();
    bagholder_store::feeds::save_shorts(&store_conn, &out_us_full, "2026-09-15T12:00:00Z", 1).unwrap();
    let read_back = bagholder_store::feeds::shorts_for(&store_conn, "GME", "NYSE").unwrap().unwrap();
    out.insert("store_shorts_for_after_save".into(), cell(&read_back));

    bagholder_store::feeds::save_shorts(&store_conn, &out_ca_trend, "2026-09-15T12:00:00Z", 1).unwrap();
    // a second save whose record carries no "series" keeps the series
    // already stored, rather than clearing it
    let mut ca_rec_no_series = out_ca_trend.clone();
    ca_rec_no_series.series = None;
    bagholder_store::feeds::save_shorts(&store_conn, &ca_rec_no_series, "2026-09-15T12:30:00Z", 2).unwrap();
    let qnc_after_reserve = bagholder_store::feeds::shorts_for(&store_conn, "QNC", "TSX-V").unwrap().unwrap();
    out.insert("store_series_kept_when_a_later_save_omits_it".into(), cell(&qnc_after_reserve));

    let mut all = bagholder_store::feeds::all_shorts(&store_conn).unwrap();
    all.sort_by(|a, b| a.shorts.symbol.cmp(&b.shorts.symbol));
    out.insert("store_all_shorts".into(), cell(&all));

    Value::Object(out)
}

/// Pinned answers for `shorts.rs`'s parsers, lookups, `finish`, and the
/// store round trip. To bless a change:
/// `BAGHOLDER_BLESS=1 cargo test -p bagholder-market --test shorts_records`.
#[test]
fn test_shorts_are_derived_and_stored_as_they_were() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/shorts_records.json");
    let have = norm(answers());
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
        return;
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/shorts_records.json")).unwrap();
    for (k, v) in want.as_object().unwrap() {
        assert_eq!(&have[k], v, "{} is not what it was", k);
    }
    assert_eq!(have.as_object().unwrap().len(), want.as_object().unwrap().len());
}
