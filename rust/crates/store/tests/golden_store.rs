//! Golden (characterisation) test for `bagholder-store` ahead of the
//! `serde_json::Value` -> typed-struct conversion: CSV parsing and import
//! (every layout `detect_format` recognises, every date shape `parse_date`
//! accepts, every statement code `map_statement_type` maps), the
//! Wealthsimple-mapped merge (insert / link / revise / skip), the CSV and
//! typed-in merge, folder watching, the plain tables (meta, NAV, FX and
//! benchmark series, trade groups and notes, JSON text), and the admin
//! surface (securities, the journal, the tile row, the pull-window clock,
//! and `clear_synced_data`). Inputs live under `tests/golden/` (`csv/*.csv`,
//! `mapped.json`); answers are pinned in `tests/golden/store.json`, compared
//! key by key (a mismatch names the key), then by key count so nothing is
//! silently dropped from either side.
//!
//! Row ids are not pinned: CSV import assigns them from the crate's own
//! `uuid4()` (no injectable generator on that path), which is random by
//! design, so every activities-table dump in this file scrubs `id` and sorts
//! by content instead of storage order. Where an id's *identity* matters
//! (the revision test's "the stored id survives revision"), that is checked
//! with a plain `assert_eq!` in the test body, not through the golden.
//!
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-store --test golden_store`,
//! then read the diff in `tests/golden/store.json` before committing it.

use bagholder_model::input::{TileRef, TradeGroup};
use bagholder_store::tables::LegacyNote;
use bagholder_store::{activities, admin, book, csvimport, feeds, merge, rows, tables};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::cell::Cell;
use std::collections::BTreeMap;

/// A `Value` object read as a lenient reader would, before it is stored: a
/// value that is not a number, or does not spell one, drops its entry.
fn lenient_num_map(v: &Value) -> BTreeMap<String, f64> {
    match v {
        Value::Object(m) => m.iter().filter_map(|(k, x)| bagholder_model::lenient::opt_num(x).map(|n| (k.clone(), n))).collect(),
        _ => BTreeMap::new(),
    }
}

fn norm(v: Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.into_iter().map(norm).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, norm(v))).collect::<Map<_, _>>()),
        v => v,
    }
}

/// Recursively blanks every `id` field, so a randomly generated uuid never
/// enters the golden.
fn scrub_ids(v: &mut Value) {
    match v {
        Value::Object(m) => {
            if m.contains_key("id") {
                m.insert("id".to_string(), json!("<id>"));
            }
            for (_, x) in m.iter_mut() {
                scrub_ids(x);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(scrub_ids),
        _ => {}
    }
}

/// As `scrub_ids`, for the wall-clock stamps `scan_folder`/`status` write
/// themselves (`stamp()` in `csvimport.rs`), which are real "now" and so
/// never the same twice.
fn scrub_scan_times(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for key in ["scannedAt", "lastScan"] {
                if let Some(x) = m.get_mut(key) {
                    if x.is_string() && x.as_str() != Some("") {
                        *x = json!("<time>");
                    }
                }
            }
            for (_, x) in m.iter_mut() {
                scrub_scan_times(x);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(scrub_scan_times),
        _ => {}
    }
}

fn sort_key(a: &Value) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        a.get("transactionDate").and_then(|v| v.as_str()).unwrap_or(""),
        a.get("bookId").and_then(|v| v.as_str()).unwrap_or(""),
        a.get("symbol").and_then(|v| v.as_str()).unwrap_or(""),
        a.get("quantity").map(|v| v.to_string()).unwrap_or_default(),
        a.get("netCashAmount").map(|v| v.to_string()).unwrap_or_default(),
        a.get("description").and_then(|v| v.as_str()).unwrap_or(""),
    )
}

/// Every activity row, ids scrubbed, sorted by content rather than by
/// storage order -- deterministic whether the row's id came from a random
/// uuid (CSV import) or a counter (`Db::new_id`).
fn dump_activities(conn: &Connection) -> Value {
    let mut rows: Vec<Value> = activities::all_activities(conn).unwrap().iter().map(|r| serde_json::to_value(r).unwrap()).collect();
    rows.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    let mut v = Value::Array(rows);
    scrub_ids(&mut v);
    v
}

fn wsrow(v: Value) -> activities::ActivityRow {
    serde_json::from_value(v).unwrap()
}

fn deterministic_id() -> impl Fn() -> String {
    let n = Cell::new(0u64);
    move || {
        n.set(n.get() + 1);
        format!("00000000-0000-4000-8000-{:012}", n.get())
    }
}

fn fresh_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    bagholder_store::relabel::ensure(&conn).unwrap();
    conn
}

fn csv_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/csv").join(name)
}

fn read_csv(name: &str) -> String {
    std::fs::read_to_string(csv_path(name)).unwrap_or_else(|e| panic!("{}: {}", name, e))
}

const CSV_FILES: &[&str] = &[
    "canonical_basic.csv",
    "wealthsimple_monthly_statement.csv",
    "legacy_basic.csv",
    "statement_quirks.csv",
    "date_shapes.csv",
    "statement_type_codes.csv",
    "unparseable_row.csv",
    "unknown_format.csv",
    "TFSACAD-2024-08-01.csv",
];

// --------------------------------------------------------------------------
// pure helpers: no database at all
// --------------------------------------------------------------------------

fn helpers_scenario(out: &mut Map<String, Value>) {
    let headers = json!({
        "spaced": csvimport::normalize_header("  Transaction   Date  "),
        "quoted": csvimport::normalize_header("'Symbol'"),
        "bom": csvimport::normalize_header("\u{feff}Amount"),
        "dashed": csvimport::normalize_header("Net-Cash Amount"),
        "empty": csvimport::normalize_header(""),
    });
    out.insert("helpers/normalize_header".into(), headers);

    let numbers: Vec<(&str, f64)> = vec![
        ("1,234.56", csvimport::parse_number("1,234.56")),
        ("$1,234.56", csvimport::parse_number("$1,234.56")),
        ("(1,234.56)", csvimport::parse_number("(1,234.56)")),
        ("(0)", csvimport::parse_number("(0)")),
        ("-", csvimport::parse_number("-")),
        ("\u{2014}", csvimport::parse_number("\u{2014}")),
        ("n/a", csvimport::parse_number("n/a")),
        ("N/A", csvimport::parse_number("N/A")),
        ("", csvimport::parse_number("")),
        ("nan", csvimport::parse_number("nan")),
        ("inf", csvimport::parse_number("inf")),
        ("CAD 12.50", csvimport::parse_number("CAD 12.50")),
        ("  42  ", csvimport::parse_number("  42  ")),
        ("junk$$", csvimport::parse_number("junk$$")),
    ];
    out.insert(
        "helpers/parse_number".into(),
        json!(numbers.into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<_, _>>()),
    );

    let dates: Vec<(&str, String)> = vec![
        ("2024-05-01", csvimport::parse_date("2024-05-01")),
        ("2024-05-01T10:00:00Z", csvimport::parse_date("2024-05-01T10:00:00Z")),
        ("2024/05/02", csvimport::parse_date("2024/05/02")),
        ("2024.05.03", csvimport::parse_date("2024.05.03")),
        ("04-May-2024", csvimport::parse_date("04-May-2024")),
        ("May-05-2024", csvimport::parse_date("May-05-2024")),
        ("May 05, 2024", csvimport::parse_date("May 05, 2024")),
        ("06/07/2024", csvimport::parse_date("06/07/2024")),
        ("13/02/2024", csvimport::parse_date("13/02/2024")),
        ("45410", csvimport::parse_date("45410")),
        ("999999", csvimport::parse_date("999999")),
        ("not-a-date", csvimport::parse_date("not-a-date")),
        ("", csvimport::parse_date("")),
        ("   ", csvimport::parse_date("   ")),
    ];
    out.insert(
        "helpers/parse_date".into(),
        json!(dates.into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<_, _>>()),
    );

    let footer: Vec<(&str, bool)> = vec![
        ("As of 2024-01-31", csvimport::is_footer_line("As of 2024-01-31")),
        ("as of 2024-01-31 balances", csvimport::is_footer_line("as of 2024-01-31 balances")),
        ("2024-01-31 as of", csvimport::is_footer_line("2024-01-31 as of")),
        ("", csvimport::is_footer_line("")),
    ];
    out.insert(
        "helpers/is_footer_line".into(),
        json!(footer.into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<_, _>>()),
    );

    let fmt = |hs: &[&str]| csvimport::detect_format(&hs.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    let formats = json!({
        "canonical": fmt(&["transaction_date", "activity_type", "activity_sub_type", "net_cash_amount", "unit_price"]),
        "canonical_minimal": fmt(&["transaction_date", "symbol"]),
        "statement": fmt(&["date", "transaction", "description", "amount"]),
        "legacy_full": fmt(&["date", "action", "symbol", "quantity", "price", "amount"]),
        "legacy_minimal": fmt(&["date", "action"]),
        "unknown": fmt(&["foo", "bar", "baz"]),
        "empty": fmt(&[]),
    });
    out.insert("helpers/detect_format".into(), formats);

    let cat = |t: &str, s: &str| csvimport::categorize(t, s);
    let categorize = json!({
        "fx_exchange": cat("FxExchange", ""),
        "fx_lower": cat("fx", ""),
        "expired": cat("OptionExpired", ""),
        "exercised": cat("", "Exercise"),
        "trade_type": cat("Trade", ""),
        "buy_sub": cat("", "BUY"),
        "sell_sub": cat("", "sell"),
        "dividend": cat("Dividend", ""),
        "deposit": cat("Deposit", ""),
        "withdrawal": cat("Withdrawal", ""),
        "interest": cat("Interest", ""),
        "fee": cat("Fee", ""),
        "transfer": cat("Transfer", ""),
        "other": cat("Unknown", ""),
    });
    out.insert("helpers/categorize".into(), categorize);

    let inst = |d: &str| { let (s, n) = csvimport::extract_instrument(d); json!({"symbol": s, "name": n}) };
    let instruments = json!({
        "colon": inst("AAPL: Apple Inc buy"),
        "dash": inst("AAPL - Apple Inc"),
        "colon_dash": inst("AAPL - Apple Inc: 10 shares at $150.00 per share"),
        "spaced_only": inst("Apple Inc Common Stock"),
        "ticker_only": inst("AAPL"),
        "empty": inst(""),
        "weird": inst("...:::"),
    });
    out.insert("helpers/extract_instrument".into(), instruments);

    let parsed = |d: &str| serde_json::to_value(csvimport::parse_statement_description(d)).unwrap();
    out.insert("helpers/parse_statement_description".into(), json!({
        "shares": parsed("AAPL - Apple Inc: 10 shares at $150.00 per share (executed at 2024-05-01)"),
        "contracts": parsed("AAPL 260918C00200000: 2 contracts at $1.50 per share"),
        "no_fill": parsed("AAPL - Apple Inc: Dividend"),
        "negative_shares": parsed("AAPL - Apple Inc: -5 shares at $10.00 per share"),
        "empty": parsed(""),
    }));

    let stype = |c: &str, d: &str| { let (a, s, cat) = csvimport::map_statement_type(c, d); json!({"activityType": a, "activitySubType": s, "category": cat}) };
    out.insert("helpers/map_statement_type".into(), json!({
        "div": stype("DIV", "Dividend payment"),
        "cont": stype("CONT", "Contribution"),
        "wd": stype("WD", "Withdrawal"),
        "interest": stype("INTEREST", "Interest paid"),
        "trfin": stype("TRFIN", "Transfer in"),
        "fx": stype("FX", "FX conversion"),
        "fee": stype("FEE", "Monthly fee"),
        "buy": stype("BUY", "AAA: 1 shares at $10.00 per share"),
        "sell": stype("SELL", "AAA: 1 shares at $10.00 per share"),
        "expir": stype("EXPIR", "Option expired"),
        "roc": stype("ROC", "Return of capital"),
        "stkdis": stype("STKDIS", "Stock dividend AAA"),
        "loan": stype("LOAN", "Securities loan"),
        "unknown_code": stype("ZZZZUNKNOWN", "Some unknown code"),
        "blank_code_dividend_word": stype("", "your dividend has arrived"),
        "blank_code_buy_word": stype("", "you bought some shares"),
        "blank_code_blank_desc": stype("", ""),
    }));

    out.insert("helpers/book_id_from_file_name".into(), json!({
        "dated": csvimport::book_id_from_file_name("TFSACAD-2024-08-01.csv"),
        "plain": csvimport::book_id_from_file_name("RRSPUSD.csv"),
        "path": csvimport::book_id_from_file_name("/some/dir/MARGINCAD.csv"),
        "no_match": csvimport::book_id_from_file_name("export.csv"),
    }));

    out.insert("helpers/is_junk_name".into(), json!({
        "dotunderscore": csvimport::is_junk_name("._export.csv"),
        "macosx": csvimport::is_junk_name("__MACOSX/export.csv"),
        "normal": csvimport::is_junk_name("export.csv"),
    }));
    out.insert("helpers/is_csv_name".into(), json!({
        "csv": csvimport::is_csv_name("export.csv"),
        "CSV": csvimport::is_csv_name("export.CSV"),
        "txt": csvimport::is_csv_name("export.txt"),
    }));

    out.insert("helpers/looks_like_homemade_id".into(), json!({
        "empty": activities::looks_like_homemade_id(""),
        "pipe": activities::looks_like_homemade_id("a|b|1.0"),
        "manual_prefix": activities::looks_like_homemade_id("Manual-123"),
        "broker_id": activities::looks_like_homemade_id("E00201234567"),
    }));
    out.insert("helpers/is_real_account".into(), json!({
        "empty": activities::is_real_account(""),
        "tilde": activities::is_real_account("~fake"),
        "manual": activities::is_real_account("manual"),
        "cad_upper": activities::is_real_account("CAD"),
        "real": activities::is_real_account("acct-123"),
    }));
    out.insert("helpers/round_qty".into(), json!({
        "plain": activities::round_qty(1.123456789),
        "none": activities::round_qty(0.0),
        "string": activities::round_qty(2.5),
    }));

    let row_of = |v: Value| -> activities::ActivityRow { serde_json::from_value(v).unwrap() };
    let side_row = |sub: &str, ty: &str, qty: f64| {
        activities::trade_side(&activities::ActivityRow { activity_sub_type: sub.into(), activity_type: ty.into(), quantity: qty, ..Default::default() })
    };
    out.insert("helpers/trade_side".into(), json!({
        "buy_sub": side_row("BUY", "Trade", 5.0),
        "sell_sub": side_row("SELL", "Trade", -5.0),
        "no_sub_positive_qty": side_row("", "Trade", 5.0),
        "no_sub_negative_qty": side_row("", "Trade", -5.0),
    }));

    let fmk = |row: &activities::ActivityRow, inc: bool| { let (d, a, s, q, p, c) = activities::field_match_key(row, inc); json!([d, a, s, q, p, c]) };
    let lmk = |row: &activities::ActivityRow, inc: bool| { let (s, side, q, p, d, a) = activities::link_match_key(row, inc); json!([s, side, q, p, d, a]) };
    let sample = row_of(json!({"transactionDate": "2024-05-01", "accountId": "acct-1", "symbol": " aaa ", "quantity": 10.0, "unitPrice": 25.5, "netCashAmount": -255.0, "activitySubType": "BUY", "activityType": "Trade"}));
    out.insert("helpers/field_match_key".into(), json!({
        "with_account": fmk(&sample, true),
        "without_account": fmk(&sample, false),
        "fake_account_still_included_if_asked": fmk(&row_of(json!({"transactionDate": "2024-05-01", "accountId": "manual", "symbol": "AAA", "quantity": 1.0})), true),
    }));
    out.insert("helpers/link_match_key".into(), json!({
        "with_account": lmk(&sample, true),
        "without_account": lmk(&sample, false),
    }));

    out.insert("helpers/canonical_from_row".into(), json!({
        "wealthsimple_clean": activities::canonical_from_row(&row_of(json!({"canonicalId": "ws-1", "source": "wealthsimple"})), "wealthsimple"),
        "wealthsimple_homemade": activities::canonical_from_row(&row_of(json!({"canonicalId": "a|b", "source": "wealthsimple"})), "wealthsimple"),
        "wealthsimple_falls_back_to_id": activities::canonical_from_row(&row_of(json!({"id": "ws-old-1", "source": "wealthsimple"})), "wealthsimple"),
        "wealthsimple_nothing": activities::canonical_from_row(&row_of(json!({"source": "wealthsimple"})), "wealthsimple"),
        "not_wealthsimple": activities::canonical_from_row(&row_of(json!({"canonicalId": "ws-1"})), "csv"),
    }));

    // What `insert_columns` used to hand back: all 27 columns, keyed by
    // column name, for an assigned id and canonical id override -- rebuilt
    // here from a normalized `ActivityRow` now that the store itself only
    // ever needs the 19-column `Revisable` view of a row.
    fn full_columns(row: activities::ActivityRow, assigned_id: &str, canonical_id: Option<&str>) -> Map<String, Value> {
        let n = row.normalized();
        let mut out = Map::new();
        out.insert("id".into(), json!(assigned_id));
        out.insert("canonical_id".into(), canonical_id.map(|c| json!(c)).unwrap_or(Value::Null));
        out.insert("occurred_at".into(), json!(n.occurred_at));
        out.insert("transaction_date".into(), json!(n.transaction_date));
        out.insert("settlement_date".into(), json!(n.settlement_date));
        out.insert("account_id".into(), json!(n.account_id));
        out.insert("book_id".into(), json!(n.book_id));
        out.insert("fifo_id".into(), json!(n.fifo_id));
        out.insert("account_type".into(), json!(n.account_type));
        out.insert("activity_type".into(), json!(n.activity_type));
        out.insert("activity_sub_type".into(), json!(n.activity_sub_type));
        out.insert("description".into(), json!(n.description));
        out.insert("direction".into(), json!(n.direction));
        out.insert("symbol".into(), json!(n.symbol));
        out.insert("name".into(), json!(n.name));
        out.insert("currency".into(), json!(n.currency));
        out.insert("quantity".into(), json!(n.quantity));
        out.insert("unit_price".into(), json!(n.unit_price));
        out.insert("commission".into(), json!(n.commission));
        out.insert("net_cash_amount".into(), json!(n.net_cash_amount));
        out.insert("category".into(), json!(n.category));
        out.insert("balance".into(), n.balance.map(|b| json!(b)).unwrap_or(Value::Null));
        out.insert("source".into(), json!(n.source));
        out.insert("raw_type".into(), json!(n.raw_type));
        out.insert("aft_type".into(), json!(n.aft_type));
        out.insert("counter_symbol".into(), json!(n.counter_symbol));
        out.insert("security_id".into(), n.security_id.map(|s| json!(s)).unwrap_or(Value::Null));
        out
    }

    let cols = full_columns(
        row_of(json!({"occurredAt": "2024-05-01T10:00:00Z", "transactionDate": "2024-05-01", "accountId": "acct-1", "symbol": "AAA", "quantity": 10.0, "unitPrice": 25.5, "netCashAmount": -255.0, "activityType": "Trade", "activitySubType": "BUY", "source": "wealthsimple", "securityId": "sec-1"})),
        "assigned-1",
        Some("cid-1"),
    );
    out.insert("helpers/insert_columns".into(), Value::Object(cols));

    let cols_defaults = full_columns(row_of(json!({"transactionDate": "2024-05-01"})), "assigned-2", None);
    out.insert("helpers/insert_columns_defaults".into(), Value::Object(cols_defaults));

    out.insert("helpers/json_text".into(), json!({
        "float_one": tables::json_text(&json!(1.0)),
        "float_tenth": tables::json_text(&json!(0.1)),
        "big": tables::json_text(&json!(1e21)),
        "unicode": tables::json_text(&json!("caf\u{e9} \u{1f600}")),
        "nested": tables::json_text(&json!({"b": 1, "a": [1, 2.5, null, "x"]})),
    }));
    out.insert("helpers/json_text_sorted".into(), json!({
        "key_order": tables::json_text_sorted(&json!({"b": 1, "a": 2, "c": {"z": 1, "y": 2}})),
    }));
}

// --------------------------------------------------------------------------
// csv: parse + import, per fixture file
// --------------------------------------------------------------------------

/// Before the typed conversion, `map_statement`/`map_canonical`/`map_legacy`
/// each built a `json!` literal naming only the fields that source ever sets;
/// a typed `ActivityRow` serializes every column. `parse_csv`'s golden pins
/// that narrower, per-format shape, so it is rebuilt here from the typed row
/// rather than widened.
fn strip_unset_csv_fields(report: &mut Value) {
    let fmt = report.get("format").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let keep: &[&str] = match fmt.as_str() {
        "statement" => &[
            "id", "occurredAt", "transactionDate", "settlementDate", "accountId", "bookId", "accountType",
            "activityType", "activitySubType", "description", "direction", "symbol", "name", "currency",
            "quantity", "unitPrice", "commission", "netCashAmount", "category", "balance", "source",
        ],
        "canonical" | "legacy" => &[
            "id", "occurredAt", "transactionDate", "settlementDate", "accountId", "accountType", "activityType",
            "activitySubType", "description", "direction", "symbol", "name", "currency", "quantity", "unitPrice",
            "commission", "netCashAmount", "category", "source",
        ],
        _ => return,
    };
    if let Some(Value::Array(acts)) = report.get_mut("activities") {
        for a in acts.iter_mut() {
            if let Value::Object(m) = a {
                *m = m.iter().filter(|(k, _)| keep.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
            }
        }
    }
}

fn csv_scenario(out: &mut Map<String, Value>, conn: &Connection) {
    for name in CSV_FILES {
        let text = read_csv(name);
        let mut parsed = match csvimport::parse_csv(&text, name) {
            Ok(v) => serde_json::to_value(&v).unwrap(),
            Err(e) => json!({"error": e}),
        };
        strip_unset_csv_fields(&mut parsed);
        scrub_ids(&mut parsed);
        out.insert(format!("csv/{}/parse", name), parsed);

        let mut imported = match csvimport::import_text(conn, name, &text) {
            Ok(v) => serde_json::to_value(&v).unwrap(),
            Err(e) => json!({"error": e}),
        };
        scrub_ids(&mut imported);
        out.insert(format!("csv/{}/import", name), imported);
        out.insert(format!("csv/{}/activities_after", name), dump_activities(conn));
    }
    // re-importing the same file a second time: everything a duplicate
    let text = read_csv("canonical_basic.csv");
    let mut again = serde_json::to_value(&csvimport::import_text(conn, "canonical_basic.csv", &text).unwrap()).unwrap();
    scrub_ids(&mut again);
    out.insert("csv/canonical_basic.csv/reimport".into(), again);
}

// --------------------------------------------------------------------------
// merge: wealthsimple-mapped rows (insert / link / revise / skip), csv/typed
// merge, and the small readers around them
// --------------------------------------------------------------------------

fn merge_scenario(out: &mut Map<String, Value>, conn: &Connection) {
    let mapped: Value = serde_json::from_str(
        &std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/mapped.json")).unwrap(),
    )
    .unwrap();
    let get = |k: &str| -> activities::ActivityRow { serde_json::from_value(mapped[k].clone()).unwrap() };
    let new_id = deterministic_id();

    // fresh insert
    let r1 = merge::apply_wealthsimple_mapped(conn, &[get("fresh")], &new_id).unwrap();
    out.insert("merge/apply/fresh".into(), json!(r1));

    // link: an unlinked local row (from the CSV import) matches exactly
    let link_matches_before = merge::find_link_candidates(conn, &get("link_candidate")).unwrap();
    out.insert("merge/find_link_candidates/before".into(), dump_activities_slice(&link_matches_before));
    let r_link = merge::apply_wealthsimple_mapped(conn, &[get("link_candidate")], &new_id).unwrap();
    out.insert("merge/apply/link".into(), json!(r_link));

    // revision: v1 inserts, v2 revises the same canonical id
    let r_rev1 = merge::apply_wealthsimple_mapped(conn, &[get("revision_v1")], &new_id).unwrap();
    let id_before = {
        let rows = activities::all_activities(conn).unwrap();
        rows.iter().find(|a| a.canonical_id.as_deref() == Some("ws-002")).unwrap().id.clone()
    };
    let r_rev2 = merge::apply_wealthsimple_mapped(conn, &[get("revision_v2")], &new_id).unwrap();
    let id_after = {
        let rows = activities::all_activities(conn).unwrap();
        rows.iter().find(|a| a.canonical_id.as_deref() == Some("ws-002")).unwrap().id.clone()
    };
    assert_eq!(id_before, id_after, "a revision keeps the stored row's own id");
    out.insert("merge/apply/revision_v1".into(), json!(r_rev1));
    out.insert("merge/apply/revision_v2".into(), json!(r_rev2));
    // revising with the identical row again changes nothing
    let r_rev2_again = merge::apply_wealthsimple_mapped(conn, &[get("revision_v2")], &new_id).unwrap();
    out.insert("merge/apply/revision_replay".into(), json!(r_rev2_again));

    // a homemade-looking canonical id and a null canonical id are both never applied
    let r_home = merge::apply_wealthsimple_mapped(conn, &[get("homemade_id")], &new_id).unwrap();
    out.insert("merge/apply/homemade_id".into(), json!(r_home));
    let r_null = merge::apply_wealthsimple_mapped(conn, &[get("null_canonical")], &new_id).unwrap();
    out.insert("merge/apply/null_canonical".into(), json!(r_null));

    // idempotent: applying the whole batch again
    let whole: Vec<activities::ActivityRow> = ["fresh", "link_candidate", "revision_v1", "revision_v2", "homemade_id", "null_canonical"]
        .iter()
        .map(|k| get(k))
        .collect();
    let r_again = merge::apply_wealthsimple_mapped(conn, &whole, &new_id).unwrap();
    out.insert("merge/apply/replay_whole_batch".into(), json!(r_again));

    out.insert("merge/activities_after_apply".into(), dump_activities(conn));

    // merge_local_rows: hand-entered rows, one a duplicate of what is already stored,
    // and a default row -- the typed equivalent of the falsy/non-object rows a JSON
    // input used to carry, which merge_local_rows still drops without counting
    let row_of = |v: Value| -> activities::ActivityRow { serde_json::from_value(v).unwrap() };
    let hand = vec![
        row_of(json!({"transactionDate": "2024-10-01", "occurredAt": "2024-10-01", "accountId": "manual", "symbol": "HAND", "quantity": 3.0, "unitPrice": 5.0, "netCashAmount": -15.0, "activityType": "Trade", "activitySubType": "BUY", "source": "manual"})),
        row_of(json!({"transactionDate": "2024-10-01", "occurredAt": "2024-10-01", "accountId": "manual", "symbol": "HAND", "quantity": 3.0, "unitPrice": 5.0, "netCashAmount": -15.0, "activityType": "Trade", "activitySubType": "BUY", "source": "manual"})),
        row_of(json!({"transactionDate": "2024-10-02", "occurredAt": "2024-10-02", "accountId": "", "symbol": "HAND2", "quantity": 1.0, "unitPrice": 2.0, "netCashAmount": -2.0, "activityType": "Trade", "activitySubType": "BUY", "source": "csv"})),
        activities::ActivityRow::default(),
    ];
    let merged = merge::merge_local_rows(conn, &hand, &new_id).unwrap();
    out.insert("merge/merge_local_rows".into(), json!({"added": merged.added, "duplicates": merged.duplicates, "ok": merged.ok}));
    // running the identical batch again: everything now a duplicate
    let merged_again = merge::merge_local_rows(conn, &hand, &new_id).unwrap();
    out.insert(
        "merge/merge_local_rows_replay".into(),
        json!({"added": merged_again.added, "duplicates": merged_again.duplicates}),
    );

    // insert_local directly
    let saved = activities::insert_local(
        conn,
        &row_of(json!({"transactionDate": "2024-10-05", "occurredAt": "2024-10-05", "accountId": "manual", "symbol": "DIRECT", "quantity": 1.0, "unitPrice": 1.0, "netCashAmount": -1.0, "activityType": "Trade", "activitySubType": "BUY", "source": "wealthsimple", "canonicalId": "should-be-dropped"})),
        &new_id,
    )
    .unwrap();
    out.insert("merge/insert_local".into(), serde_json::to_value(&saved).unwrap());

    out.insert("merge/activities_final".into(), dump_activities(conn));
    out.insert("merge/all_activities_count".into(), json!(activities::activity_count(conn).unwrap()));
    let mut ids = activities::canonical_ids(conn).unwrap();
    ids.sort();
    out.insert("merge/canonical_ids".into(), json!(ids));

    // activity_by_id for two ids, one real one missing
    let some_id = activities::all_activities(conn).unwrap()[0].id.clone();
    out.insert("merge/activity_by_id/found".into(), {
        let mut v = serde_json::to_value(activities::activity_by_id(conn, &some_id).unwrap().unwrap()).unwrap();
        scrub_ids(&mut v);
        v
    });
    out.insert("merge/activity_by_id/missing".into(), json!(activities::activity_by_id(conn, "no-such-id").unwrap()));
    out.insert(
        "merge/by_id_reexport_matches".into(),
        json!(merge::by_id(conn, &some_id).unwrap().is_some()),
    );

    // revisable_view of a row
    out.insert(
        "merge/revisable_view".into(),
        {
            let mut v = serde_json::to_value(bagholder_store::activities::Revisable::of(&get("revision_v2").normalized())).unwrap();
            v.as_object_mut().unwrap().remove("security_id");
            v
        },
    );
}

fn dump_activities_slice(rows: &[activities::ActivityRow]) -> Value {
    let mut rows: Vec<Value> = rows.iter().map(|r| serde_json::to_value(r).unwrap()).collect();
    rows.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    let mut v = Value::Array(rows);
    scrub_ids(&mut v);
    v
}

// --------------------------------------------------------------------------
// folder watching, against a temp directory holding the fixture CSVs
// --------------------------------------------------------------------------

fn redact(v: &Value, dir: &str) -> Value {
    match v {
        Value::String(s) => json!(s.replace(dir, "<dir>")),
        Value::Array(a) => Value::Array(a.iter().map(|x| redact(x, dir)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), redact(x, dir))).collect()),
        other => other.clone(),
    }
}

fn folder_scenario(out: &mut Map<String, Value>) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_string_lossy().into_owned();
    for name in ["canonical_basic.csv", "legacy_basic.csv"] {
        std::fs::copy(csv_path(name), tmp.path().join(name)).unwrap();
    }
    // a junk file and a non-csv file must be ignored
    std::fs::write(tmp.path().join("._junk.csv"), "x").unwrap();
    std::fs::write(tmp.path().join("notes.txt"), "x").unwrap();
    std::fs::write(tmp.path().join("empty.csv"), "").unwrap();

    fn tv<T: serde::Serialize>(v: &T) -> Value {
        serde_json::to_value(v).unwrap()
    }

    let conn = fresh_conn();
    let set = csvimport::set_watch_folder(&conn, &dir).unwrap();
    out.insert("folder/set_watch_folder".into(), redact(&tv(&set), &dir));
    out.insert("folder/watch_folder".into(), json!(csvimport::watch_folder(&conn).unwrap().replace(&dir, "<dir>")));

    let listed = csvimport::list_csv_files(&dir);
    let listed: Vec<Value> = listed.iter().map(|v| redact(&tv(v), &dir)).collect();
    out.insert("folder/list_csv_files".into(), json!(listed));

    let mut first = redact(&tv(&csvimport::scan_folder(&conn, None, false).unwrap()), &dir);
    scrub_scan_times(&mut first);
    out.insert("folder/scan_folder/first".into(), first);
    let mut status1 = redact(&tv(&csvimport::status(&conn).unwrap()), &dir);
    scrub_scan_times(&mut status1);
    out.insert("folder/status/after_first_scan".into(), status1);

    let mut unchanged = redact(&tv(&csvimport::scan_folder(&conn, None, false).unwrap()), &dir);
    scrub_scan_times(&mut unchanged);
    out.insert("folder/scan_folder/second_unchanged".into(), unchanged);

    let mut forced = redact(&tv(&csvimport::scan_folder(&conn, None, true).unwrap()), &dir);
    scrub_scan_times(&mut forced);
    out.insert("folder/scan_folder/forced".into(), forced);

    out.insert("folder/activities_after_scan".into(), dump_activities(&conn));

    csvimport::clear_watch_folder(&conn).unwrap();
    out.insert("folder/watch_folder_after_clear".into(), json!(csvimport::watch_folder(&conn).unwrap()));
    let mut status_cleared = tv(&csvimport::status(&conn).unwrap());
    scrub_scan_times(&mut status_cleared);
    out.insert("folder/status_after_clear".into(), status_cleared);

    // no folder set at all
    let no_folder = csvimport::scan_folder(&conn, None, false).unwrap();
    out.insert("folder/scan_folder/no_folder_set".into(), tv(&no_folder));
    // a folder that does not exist
    let missing = csvimport::scan_folder(&conn, Some("/no/such/folder/anywhere"), false).unwrap();
    out.insert("folder/scan_folder/missing_folder".into(), tv(&missing));
    // set_watch_folder with an empty path, and a path that is not a directory
    out.insert("folder/set_watch_folder/empty".into(), tv(&csvimport::set_watch_folder(&conn, "").unwrap()));
    let file_path = tmp.path().join("canonical_basic.csv");
    out.insert(
        "folder/set_watch_folder/not_a_directory".into(),
        redact(&tv(&csvimport::set_watch_folder(&conn, file_path.to_str().unwrap()).unwrap()), &dir),
    );
}

// --------------------------------------------------------------------------
// tables: meta, nav history, fx/benchmark series, trade groups/notes
// --------------------------------------------------------------------------

fn tables_scenario(out: &mut Map<String, Value>) {
    let conn = fresh_conn();

    out.insert("tables/get_meta/default".into(), json!(tables::get_meta(&conn, "no-such-key", "fallback").unwrap()));
    tables::set_meta(&conn, "k1", "v1").unwrap();
    tables::set_meta(&conn, "k1", "v2").unwrap();
    out.insert("tables/get_meta/overwritten".into(), json!(tables::get_meta(&conn, "k1", "").unwrap()));

    // NAV: identity-wide ("") and one account, via replace_nav
    let points = vec![
        json!({"accountId": "", "date": "2024-01-01", "equity": 1000.0, "currency": "CAD", "netDeposits": 100.0}),
        json!({"accountId": "", "date": "2024-01-02", "equity": 1010.0, "currency": "CAD"}),
        json!({"accountId": "acct-x", "date": "2024-01-01", "equity": 500.0, "currency": "USD", "netDeposits": 0.0}),
        json!({"accountId": "", "date": "", "equity": 1.0, "currency": "CAD"}), // no date: dropped
        json!({"accountId": "", "date": "2024-01-03", "equity": null, "currency": "CAD"}), // no equity: dropped
    ];
    tables::replace_nav(&conn, &common_typed_nav(&points)).unwrap();
    let (nav_identity, nav_by_account) = rows::nav(&conn).unwrap();
    out.insert("tables/nav/identity".into(), json!(nav_identity));
    out.insert("tables/nav/by_account".into(), json!(nav_by_account));
    let mut lasts: Vec<(String, String)> = tables::nav_last_dates(&conn).unwrap().into_iter().collect();
    lasts.sort();
    out.insert("tables/nav_last_dates".into(), json!(lasts));

    // clean_date_map: junk keys and values, as a lenient reader would parse them first
    let dirty_raw = json!({
        "2024-01-01": 1.5,
        "2024-01-02": "2.5",
        "2024-01-02T00:00:00Z": 9.0,
        "not-a-date": 3.0,
        "2024-01-03": 0.0,
        "2024-01-04": -1.0,
        "2024-01-05": "junk",
        "2024-01-06": null,
    });
    let dirty = lenient_num_map(&dirty_raw);
    out.insert("tables/clean_date_map".into(), json!(tables::clean_date_map(&dirty)));
    out.insert("tables/clean_date_map/none".into(), json!(tables::clean_date_map(&BTreeMap::new())));

    let n1 = tables::upsert_fx_rates(&conn, &dirty, tables::FX_PAIR).unwrap();
    out.insert("tables/upsert_fx_rates/inserted".into(), json!(n1));
    out.insert("tables/fx_rates".into(), json!(tables::fx_rates(&conn, tables::FX_PAIR).unwrap()));
    out.insert("tables/fx_last_date".into(), json!(tables::fx_last_date(&conn, tables::FX_PAIR).unwrap()));
    // insert or ignore: a repeat write with a different value never rewrites the day
    tables::upsert_fx_rates(&conn, &lenient_num_map(&json!({"2024-01-01": 99.0})), tables::FX_PAIR).unwrap();
    out.insert("tables/fx_rates/insert_or_ignore".into(), json!(tables::fx_rates(&conn, tables::FX_PAIR).unwrap().get("2024-01-01")));
    out.insert("tables/fx_last_date/missing_pair".into(), json!(tables::fx_last_date(&conn, "NOPAIR").unwrap()));

    let n2 = tables::upsert_benchmark_prices(&conn, &dirty, tables::BENCHMARK_SYMBOL).unwrap();
    out.insert("tables/upsert_benchmark_prices/inserted".into(), json!(n2));
    out.insert("tables/benchmark_prices".into(), json!(tables::benchmark_prices(&conn, tables::BENCHMARK_SYMBOL).unwrap()));
    out.insert(
        "tables/benchmark_days".into(),
        json!(tables::benchmark_days(&conn, tables::BENCHMARK_SYMBOL, "2024-01-01", "2024-01-06").unwrap()),
    );
    out.insert("tables/benchmark_last_date".into(), json!(tables::benchmark_last_date(&conn, tables::BENCHMARK_SYMBOL).unwrap()));

    // trade groups: junk inputs, as a lenient reader would parse them first
    let groups_raw = json!([
        {"id": "g1", "locked": true, "members": ["a|b|1.00000000", "c|d|2.00000000", "a|b|1.00000000"]},
        {"id": "g1", "members": ["dup-id-ignored"]},
        {"id": "", "members": ["x"]},
        {"id": "g_empty", "members": []},
        {"id": "g2", "locked": 1, "members": ["x", "x", "y"]},
        {"id": "g3", "members": "not-an-array"},
        "not-an-object",
    ]);
    let groups: Vec<TradeGroup> = bagholder_model::lenient::rows(&groups_raw);
    let saved_groups = tables::save_trade_groups(&conn, &groups).unwrap();
    out.insert("tables/save_trade_groups".into(), json!(saved_groups));
    out.insert("tables/trade_groups".into(), json!(tables::trade_groups(&conn).unwrap()));
    out.insert("tables/clean_trade_groups/direct".into(), json!(tables::clean_trade_groups(&groups)));
    out.insert("tables/clean_trade_groups/none".into(), json!(tables::clean_trade_groups(&[])));

    let notes_raw = json!({
        "trade-1": {"thesis": "good setup", "tag": "swing", "grade": "A"},
        "trade-2": {"thesis": "", "tag": "", "grade": "Z"},
        "trade-3": {"thesis": "", "tag": "", "grade": ""},
        "  ": {"thesis": "blank key trimmed away"},
        "trade-4": "not-an-object",
    });
    let notes: BTreeMap<String, LegacyNote> = bagholder_model::lenient::objmap(&notes_raw);
    let saved_notes = tables::save_trade_notes(&conn, &notes).unwrap();
    out.insert("tables/save_trade_notes".into(), json!(saved_notes));
    out.insert("tables/trade_notes".into(), json!(tables::trade_notes(&conn).unwrap()));
    out.insert("tables/clean_trade_notes/direct".into(), json!(tables::clean_trade_notes(&notes)));
    out.insert("tables/clean_trade_notes/none".into(), json!(tables::clean_trade_notes(&BTreeMap::new())));

    // json_text / json_text_sorted through the store's own writer, on richer values
    let richer = json!({"z": 1.0, "a": [0.1, 1e21, "caf\u{e9}", null, true, false], "m": {"nested": "value"}});
    out.insert("tables/json_text/richer".into(), json!(tables::json_text(&richer)));
    out.insert("tables/json_text_sorted/richer".into(), json!(tables::json_text_sorted(&richer)));

    // accounts / balances / margin readers, via the typed writers
    tables::replace_accounts(&conn, &common_typed_accounts()).unwrap();
    tables::replace_balances(&conn, &common_typed_balances()).unwrap();
    tables::replace_margin(&conn, &common_typed_margin(), "2026-09-22T00:00:00Z").unwrap();
    out.insert("tables/accounts".into(), json!(tables::accounts(&conn).unwrap()));
    out.insert("tables/balances".into(), json!(tables::balances(&conn).unwrap()));
    out.insert("tables/margin".into(), json!(tables::margin(&conn).unwrap()));
}

fn common_typed_nav(rows: &[Value]) -> Vec<bagholder_store::broker::NavPoint> {
    rows.iter().map(|v| serde_json::from_value(v.clone()).unwrap()).collect()
}
fn common_typed_accounts() -> Vec<bagholder_store::broker::Account> {
    serde_json::from_value(json!([
        {"id": "acct-a", "nickname": "A", "unifiedAccountType": "SelfDirected", "currency": "CAD", "status": "open", "type": "SelfDirected", "netLiquidationValue": 1000.0, "marginAccountId": ""},
        {"id": "", "nickname": "dropped, no id"},
    ]))
    .unwrap()
}
fn common_typed_balances() -> Vec<bagholder_store::broker::Balance> {
    serde_json::from_value(json!([
        {"accountId": "acct-a", "custodianAccountId": "cust-1", "securityId": "sec-1", "quantity": 10.0},
    ]))
    .unwrap()
}
fn common_typed_margin() -> Vec<bagholder_store::broker::Margin> {
    serde_json::from_value(json!([
        {"accountId": "acct-a", "buyingPower": 500.0, "currency": "CAD"},
        {"accountId": "", "buyingPower": 1.0}, // dropped, no account id
    ]))
    .unwrap()
}

// --------------------------------------------------------------------------
// admin: securities, journal, tiles, the pull-window clock, clear_synced_data
// --------------------------------------------------------------------------

/// Mountain time is UTC-6 on these summer days (same trick as
/// `tests/activities.rs`'s `mountain` helper).
fn mountain(y: i64, m: u32, d: u32, hh: i64, mm: i64) -> i64 {
    bagholder_model::dates::to_days(y, m, d) * 86400 + (hh + 6) * 3600 + mm * 60
}

fn admin_scenario(out: &mut Map<String, Value>) {
    let conn = fresh_conn();

    let secs: Vec<bagholder_model::securities::Security> = serde_json::from_value(json!([
        {"id": "sec-a", "symbol": "AAA", "name": "AAA Inc", "primaryExchange": "TSX", "primaryMic": "XTSE", "currency": "CAD"},
        {"id": "sec-b", "symbol": "BBB", "name": "BBB Inc", "primaryExchange": "TSXV", "primaryMic": "XTSX", "currency": "CAD", "underlyingId": "sec-a"},
        {"id": "  ", "symbol": "IGNORED"}, // blank id, dropped
    ]))
    .unwrap();
    admin::upsert_securities(&conn, &secs, "2026-09-22T00:00:00Z").unwrap();
    out.insert("admin/list_securities".into(), json!(admin::list_securities(&conn).unwrap()));
    out.insert(
        "admin/missing_security_ids".into(),
        json!(admin::missing_security_ids(&conn, &["sec-a".into(), "sec-c".into(), "sec-a".into(), "  ".into()]).unwrap()),
    );
    out.insert("admin/missing_security_ids/empty_input".into(), json!(admin::missing_security_ids(&conn, &[]).unwrap()));

    out.insert("admin/needs_security_id_backfill/before".into(), json!(admin::needs_security_id_backfill(&conn).unwrap()));
    let new_id = deterministic_id();
    merge::apply_wealthsimple_mapped(
        &conn,
        &[wsrow(json!({"canonicalId": "backfill-1", "transactionDate": "2024-01-01", "occurredAt": "2024-01-01", "accountId": "acct-1", "symbol": "AAA", "quantity": 1.0, "unitPrice": 1.0, "netCashAmount": -1.0, "activityType": "Trade", "activitySubType": "BUY", "source": "wealthsimple"}))],
        &new_id,
    )
    .unwrap();
    out.insert("admin/needs_security_id_backfill/after_wealthsimple_row_no_sid".into(), json!(admin::needs_security_id_backfill(&conn).unwrap()));
    merge::apply_wealthsimple_mapped(
        &conn,
        &[wsrow(json!({"canonicalId": "backfill-2", "transactionDate": "2024-01-02", "occurredAt": "2024-01-02", "accountId": "acct-1", "symbol": "BBB", "quantity": 1.0, "unitPrice": 1.0, "netCashAmount": -1.0, "activityType": "Trade", "activitySubType": "BUY", "source": "wealthsimple", "securityId": "sec-b"}))],
        &new_id,
    )
    .unwrap();
    // one row still has no security id, so the flag stays true
    out.insert("admin/needs_security_id_backfill/mixed".into(), json!(admin::needs_security_id_backfill(&conn).unwrap()));

    // journal
    let entries: bagholder_model::input::Journal = bagholder_model::lenient::objmap(&json!({
        "trade-1": {"thesis": "solid thesis", "grade": "b", "tags": "swing,earnings"},
        "trade-2": {"thesis": "", "grade": "", "tags": []},
        "  ": {"thesis": "blank key"},
        "trade-3": {"thesis": "dup tags", "tags": ["x", "x", "y"]},
    })).into_iter().collect();
    out.insert("admin/save_journal".into(), json!(admin::save_journal(&conn, &entries).unwrap()));
    let entry_of = |v: Value| -> bagholder_model::input::JournalEntry { serde_json::from_value(v).unwrap() };
    out.insert("admin/save_journal_entry/set".into(), json!(admin::save_journal_entry(&conn, "trade-4", Some(&entry_of(json!({"thesis": "new entry", "grade": "A", "tags": ["z"]})))).unwrap()));
    out.insert("admin/save_journal_entry/delete".into(), json!(admin::save_journal_entry(&conn, "trade-1", None).unwrap()));
    out.insert("admin/save_journal_entry/blank_key_noop".into(), json!(admin::save_journal_entry(&conn, "  ", Some(&entry_of(json!({"thesis": "x"})))).unwrap()));
    out.insert("admin/snapshot_journal".into(), json!(admin::journal(&conn).unwrap()));
    let junk_conn = fresh_conn();
    tables::set_meta(&junk_conn, tables::JOURNAL_META, r#"{"k1": {"thesis": "", "grade": "F", "tags": []}, "k2": "not-an-object", "k3": {"thesis": "", "grade": "nope", "tags": []}}"#).unwrap();
    out.insert("admin/clean_journal/junk".into(), json!(admin::journal(&junk_conn).unwrap()));

    // tiles
    let tiles_in: Vec<TileRef> = bagholder_model::lenient::rows(&json!([
        {"symbol": " aaa ", "exchange": "tsx"},
        {"symbol": "", "exchange": "x"}, // no symbol, dropped
        {"symbol": "bbb"},
        "not-an-object",
    ]));
    let saved_tiles = admin::save_tiles(&conn, &tiles_in).unwrap();
    out.insert("admin/save_tiles".into(), json!(saved_tiles));
    out.insert("admin/tiles_part".into(), json!(rows::tiles(&conn).unwrap()));
    let empty_conn = fresh_conn();
    out.insert("admin/tiles_from/empty".into(), json!(rows::tiles(&empty_conn).unwrap()));
    tables::set_meta(&empty_conn, rows::TILES_META, "not json").unwrap();
    out.insert("admin/tiles_from/bad_json".into(), json!(rows::tiles(&empty_conn).unwrap()));
    tables::set_meta(&empty_conn, rows::TILES_META, "{}").unwrap();
    out.insert("admin/tiles_from/not_array".into(), json!(rows::tiles(&empty_conn).unwrap()));

    // pull window clock
    out.insert("admin/seconds_until_pull_window".into(), json!({
        "monday_before_open": admin::seconds_until_pull_window(mountain(2026, 8, 31, 13, 59)),
        "monday_at_window": admin::seconds_until_pull_window(mountain(2026, 8, 31, 14, 0)),
        "friday_after_window": admin::seconds_until_pull_window(mountain(2026, 9, 4, 15, 0)),
        "saturday": admin::seconds_until_pull_window(mountain(2026, 9, 5, 8, 0)),
        "sunday_evening": admin::seconds_until_pull_window(mountain(2026, 9, 6, 20, 0)),
    }));
    let due = |t| admin::activity_pull_due(&conn, t).unwrap();
    out.insert("admin/activity_pull_due".into(), json!({
        "before_window": due(mountain(2026, 8, 31, 13, 59)),
        "at_window": due(mountain(2026, 8, 31, 14, 0)),
        "weekend": due(mountain(2026, 8, 29, 15, 0)),
    }));
    admin::mark_activity_pulled(&conn, "2026-08-31T20:05:00Z").unwrap();
    out.insert("admin/activity_pull_due/after_mark_same_day".into(), json!(due(mountain(2026, 8, 31, 15, 0))));
    out.insert("admin/activity_pull_due/after_mark_next_day".into(), json!(due(mountain(2026, 9, 1, 14, 0))));
}

/// `clear_synced_data`, for each of the four keep-journal/keep-market
/// combinations, each starting from the same seeded state.
fn clear_synced_data_scenario(out: &mut Map<String, Value>) {
    for (keep_journal, keep_market) in [(false, false), (true, false), (false, true), (true, true)] {
        let conn = fresh_conn();
        let new_id = deterministic_id();
        merge::apply_wealthsimple_mapped(
            &conn,
            &[wsrow(json!({"canonicalId": "clear-1", "transactionDate": "2024-01-01", "occurredAt": "2024-01-01", "accountId": "acct-1", "symbol": "AAA", "quantity": 1.0, "unitPrice": 1.0, "netCashAmount": -1.0, "activityType": "Trade", "activitySubType": "BUY", "source": "wealthsimple"}))],
            &new_id,
        )
        .unwrap();
        tables::replace_accounts(&conn, &common_typed_accounts()).unwrap();
        tables::replace_balances(&conn, &common_typed_balances()).unwrap();
        tables::replace_margin(&conn, &common_typed_margin(), "2026-09-22T00:00:00Z").unwrap();
        tables::replace_nav(&conn, &common_typed_nav(&[json!({"accountId": "", "date": "2024-01-01", "equity": 100.0, "currency": "CAD"})])).unwrap();
        tables::upsert_fx_rates(&conn, &lenient_num_map(&json!({"2024-01-01": 1.35})), tables::FX_PAIR).unwrap();
        tables::upsert_benchmark_prices(&conn, &lenient_num_map(&json!({"2024-01-01": 5000.0})), tables::BENCHMARK_SYMBOL).unwrap();
        let groups: Vec<TradeGroup> = bagholder_model::lenient::rows(&json!([{"id": "g1", "members": ["m1"]}]));
        tables::save_trade_groups(&conn, &groups).unwrap();
        let notes: BTreeMap<String, LegacyNote> = bagholder_model::lenient::objmap(&json!({"t1": {"thesis": "x", "grade": "A"}}));
        tables::save_trade_notes(&conn, &notes).unwrap();
        let journal: bagholder_model::input::Journal = bagholder_model::lenient::objmap(&json!({"t1": {"thesis": "journal entry", "grade": "A"}})).into_iter().collect();
        admin::save_journal(&conn, &journal).unwrap();
        tables::set_meta(&conn, "synced_at", "2026-09-22T00:00:00Z").unwrap();
        tables::set_meta(&conn, "last_activity_pull", "2026-09-22T00:00:00Z").unwrap();
        tables::set_meta(&conn, "security_id_backfill_done", "1").unwrap();

        admin::clear_synced_data(&conn, keep_journal, keep_market).unwrap();
        let b = book::book(&conn).unwrap();
        let key = format!("admin/clear_synced_data/keep_journal={}/keep_market={}", keep_journal, keep_market);
        out.insert(format!("{}/activities", key), json!(b.activities));
        out.insert(format!("{}/accounts", key), json!(b.accounts));
        out.insert(format!("{}/balances", key), json!(b.balances));
        out.insert(format!("{}/margin", key), json!(tables::margin(&conn).unwrap()));
        out.insert(format!("{}/navHistory", key), json!(b.nav_history));
        out.insert(format!("{}/tradeGroups", key), json!(b.trade_groups));
        out.insert(format!("{}/notes", key), json!(b.notes));
        out.insert(format!("{}/journal", key), json!(admin::journal(&conn).unwrap()));
        out.insert(format!("{}/fx_rates", key), json!(tables::fx_rates(&conn, tables::FX_PAIR).unwrap()));
        out.insert(format!("{}/benchmark_prices", key), json!(tables::benchmark_prices(&conn, tables::BENCHMARK_SYMBOL).unwrap()));
        out.insert(format!("{}/synced_at", key), json!(tables::get_meta(&conn, "synced_at", "").unwrap()));
        out.insert(format!("{}/security_id_backfill_done", key), json!(tables::get_meta(&conn, "security_id_backfill_done", "").unwrap()));
    }
}

// --------------------------------------------------------------------------
// exposures and universes as stored, and the book export
// --------------------------------------------------------------------------

fn snapshot_scenario(out: &mut Map<String, Value>) {
    let conn = fresh_conn();
    out.insert("snapshot/exposures_part".into(), json!(feeds::all_exposures(&conn).unwrap()));
    out.insert("snapshot/universes_part".into(), json!(feeds::stored_universes(&conn).unwrap()));
}

// --------------------------------------------------------------------------
// book: the stored rows as they are, for `GET /api/book` and for export
// --------------------------------------------------------------------------

fn book_scenario(out: &mut Map<String, Value>) {
    let conn = fresh_conn();
    let new_id = deterministic_id();
    merge::apply_wealthsimple_mapped(
        &conn,
        &[wsrow(json!({"canonicalId": "snap-1", "transactionDate": "2024-01-01", "occurredAt": "2024-01-01", "accountId": "acct-a", "symbol": "AAA", "quantity": 1.0, "unitPrice": 10.0, "netCashAmount": -10.0, "activityType": "Trade", "activitySubType": "BUY", "source": "wealthsimple"}))],
        &new_id,
    )
    .unwrap();
    tables::replace_accounts(&conn, &common_typed_accounts()).unwrap();
    tables::replace_balances(&conn, &common_typed_balances()).unwrap();
    tables::replace_margin(&conn, &common_typed_margin(), "2026-09-22T00:00:00Z").unwrap();
    tables::replace_nav(&conn, &common_typed_nav(&[
        json!({"accountId": "", "date": "2024-01-01", "equity": 100.0, "currency": "CAD"}),
        json!({"accountId": "acct-a", "date": "2024-01-01", "equity": 50.0, "currency": "CAD"}),
    ]))
    .unwrap();
    let groups: Vec<TradeGroup> = bagholder_model::lenient::rows(&json!([{"id": "g1", "members": ["m1"]}]));
    tables::save_trade_groups(&conn, &groups).unwrap();
    let journal: bagholder_model::input::Journal = bagholder_model::lenient::objmap(&json!({"snap-1": {"thesis": "x", "grade": "A"}})).into_iter().collect();
    admin::save_journal(&conn, &journal).unwrap();
    let tiles_in: Vec<TileRef> = bagholder_model::lenient::rows(&json!([{"symbol": "AAA", "exchange": "TSX"}]));
    admin::save_tiles(&conn, &tiles_in).unwrap();
    let secs: Vec<bagholder_model::securities::Security> = serde_json::from_value(json!([
        {"id": "sec-a", "symbol": "AAA", "name": "AAA Inc", "primaryExchange": "TSX", "primaryMic": "XTSE", "currency": "CAD"},
    ]))
    .unwrap();
    admin::upsert_securities(&conn, &secs, "2026-09-22T00:00:00Z").unwrap();
    tables::set_meta(&conn, "synced_at", "2026-09-22T00:00:00Z").unwrap();

    let mut b = json!(book::book(&conn).unwrap());
    scrub_ids(&mut b);
    out.insert("book/book".into(), b);

    out.insert("book/distinct_activity_account_ids".into(), json!(book::distinct_activity_account_ids(&conn).unwrap()));
    out.insert("book/distinct_security_ids".into(), json!(book::distinct_security_ids(&conn).unwrap()));
}

// --------------------------------------------------------------------------

fn answers() -> Value {
    let mut out: Map<String, Value> = Map::new();
    helpers_scenario(&mut out);

    let csv_conn = fresh_conn();
    csv_scenario(&mut out, &csv_conn);
    merge_scenario(&mut out, &csv_conn);

    folder_scenario(&mut out);
    tables_scenario(&mut out);
    admin_scenario(&mut out);
    clear_synced_data_scenario(&mut out);
    snapshot_scenario(&mut out);
    book_scenario(&mut out);
    Value::Object(out)
}

#[test]
fn test_store_behaviour_is_pinned() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/store.json");
    let have = norm(answers());
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
        return;
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/store.json")).unwrap();
    for (k, v) in want.as_object().unwrap() {
        assert_eq!(&have[k], v, "{} is not what it was", k);
    }
    assert_eq!(have.as_object().unwrap().len(), want.as_object().unwrap().len());
}

#[test]
fn test_running_twice_is_deterministic() {
    let a = norm(answers());
    let b = norm(answers());
    assert_eq!(a, b, "the golden must not depend on run order or generated ids");
}
