//! Margin, securities, trade groups and notes, NAV, the watchlist and the tile
//! row (tests/test_store.py).

mod common;
use bagholder_store::{admin, feeds, schema, snapshot, tables};
use common::*;
use serde_json::{json, Value};
use std::collections::HashSet;

#[test]
fn test_margin_rows_round_trip_and_clear_with_the_synced_data() {
    let d = db();
    tables::replace_margin(&d.conn, &[
        json!({"accountId": "acct-1", "buyingPower": 6817.33, "currency": "CAD"}),
        json!({"accountId": "acct-2", "buyingPower": null, "currency": "CAD", "unavailable": "UnavailableSecurities (2 securities)"}),
        json!({"accountId": "", "buyingPower": 1.0}),
    ], "2026-09-10T14:00:00Z").unwrap();
    let snap = d.snapshot();
    let rows = snap["margin"].as_array().unwrap();
    let got: Vec<(Value, Value, Value)> = rows.iter().map(|r| (r["accountId"].clone(), r["buyingPower"].clone(), r["unavailable"].clone())).collect();
    assert_eq!(got, vec![
        (json!("acct-1"), json!(6817.33), json!("")),
        (json!("acct-2"), Value::Null, json!("UnavailableSecurities (2 securities)")),
    ]);
    assert!(rows.iter().all(|r| !r["fetchedAt"].as_str().unwrap_or("").is_empty()));
    // the data version moving with a new reading is the server crate's `data_version`
    tables::replace_margin(&d.conn, &[json!({"accountId": "acct-1", "buyingPower": 6900.0, "currency": "CAD"})], "2026-09-10T15:00:00Z").unwrap();
    assert_eq!(d.snapshot()["margin"][0]["buyingPower"], json!(6900.0));
    admin::clear_synced_data(&d.conn, true, true).unwrap();
    assert_eq!(d.snapshot()["margin"], json!([]));
}

#[test]
fn test_snapshot_includes_securities() {
    let d = db();
    admin::upsert_securities(&d.conn, &[json!({"id": "sec-s-ch", "symbol": "CH", "name": "Charbone Corporation", "primaryExchange": "TSX Venture Exchange", "primaryMic": "XTSV", "currency": "CAD"})], "2026-09-10T14:00:00Z").unwrap();
    let snap = d.snapshot();
    let secs = snap["securities"].as_array().unwrap();
    assert_eq!(secs.len(), 1);
    assert_eq!(secs[0]["id"], "sec-s-ch");
    assert_eq!(secs[0]["name"], "Charbone Corporation");
    assert_eq!(secs[0]["primaryMic"], "XTSV");
}

#[test]
fn test_trade_groups_roundtrip() {
    let d = db();
    let groups = json!([
        {"id": "g_one", "locked": true, "members": ["a|b|1.00000000", "c|d|2.00000000"]},
        {"id": "g_one", "locked": false, "members": ["dup"]},
        {"id": "", "members": ["x"]},
        {"id": "g_empty", "members": []},
        {"id": "g_two", "locked": 1, "members": ["x", "x", "y"]},
    ]);
    let saved = tables::save_trade_groups(&d.conn, Some(&groups)).unwrap();
    assert_eq!(saved.iter().map(|g| g["id"].as_str().unwrap()).collect::<Vec<_>>(), vec!["g_one", "g_two"]);
    assert_eq!(saved[0]["members"], json!(["a|b|1.00000000", "c|d|2.00000000"]));
    assert_eq!(saved[1]["members"], json!(["x", "y"]));
    assert_eq!(saved[0]["locked"], json!(true));
    assert_eq!(saved[1]["locked"], json!(true));
    assert_eq!(d.snapshot()["tradeGroups"], Value::Array(saved.clone()));
    assert_eq!(tables::trade_groups(&d.conn).unwrap(), saved);
}

#[test]
fn test_trade_groups_rejects_non_list() {
    let d = db();
    tables::set_meta(&d.conn, "trade_groups", "{}").unwrap();
    assert_eq!(tables::trade_groups(&d.conn).unwrap(), Vec::<Value>::new());
    assert_eq!(tables::save_trade_groups(&d.conn, None).unwrap(), Vec::<Value>::new());
}

#[test]
fn test_trade_notes_roundtrip() {
    let d = db();
    let saved = tables::save_trade_notes(&d.conn, Some(&json!({
        "g_one": {"thesis": "scale in", "tag": "hold", "grade": "A"},
        "g_empty": {"thesis": "", "tag": "", "grade": ""},
        "g_bad": {"thesis": "x", "tag": "y", "grade": "Z"},
        "": {"thesis": "nope"},
    }))).unwrap();
    assert_eq!(saved["g_one"]["grade"], "A");
    assert_eq!(saved["g_one"]["tag"], "hold");
    assert!(!saved.contains_key("g_empty"));
    assert_eq!(saved["g_bad"]["grade"], "");
    assert_eq!(saved["g_bad"]["thesis"], "x");
    assert_eq!(d.snapshot()["notes"], Value::Object(saved.clone()));
    assert_eq!(tables::trade_notes(&d.conn).unwrap(), saved);
}

#[test]
fn test_nav_history_migrates_date_pk_to_account_date() {
    let mut d = db();
    {
        let c = rusqlite::Connection::open(d.path()).unwrap();
        c.execute_batch(
            "DROP TABLE nav_history;
             CREATE TABLE nav_history (date TEXT PRIMARY KEY, equity REAL, currency TEXT, net_deposits REAL);
             INSERT INTO nav_history (date, equity, currency, net_deposits) VALUES ('2024-01-02', 1000.0, 'CAD', 100.0);
             INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', '1');",
        ).unwrap();
    }
    d.reopen();
    d.ensure();
    let snap = d.snapshot();
    assert_eq!(tables::get_meta(&d.conn, "schema_version", "").unwrap(), schema::SCHEMA_VERSION.to_string());
    let nav = snap["navHistory"].as_array().unwrap();
    assert_eq!(nav.len(), 1);
    assert_eq!(nav[0]["date"], "2024-01-02");
    assert_eq!(f(&nav[0]["equity"]), 1000.0);
    assert_eq!(f(&nav[0]["netDeposits"]), 100.0);
    assert!(nav[0].get("accountId").is_none());
    assert_eq!(snap["navByAccount"], json!({}));
    let mut stmt = d.conn.prepare("PRAGMA table_info(nav_history)").unwrap();
    let pk: HashSet<String> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .filter(|(_, pk)| *pk != 0)
        .map(|(n, _)| n)
        .collect();
    assert_eq!(pk, ["account_id", "date"].iter().map(|s| s.to_string()).collect());
}

fn dates(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|p| p["date"].as_str().unwrap().to_string()).collect()
}

fn keys(v: &Value) -> HashSet<String> {
    v.as_object().unwrap().keys().cloned().collect()
}

#[test]
fn test_replace_nav_by_account_and_snapshot() {
    let d = db();
    tables::replace_nav(&d.conn, &[
        json!({"date": "2024-01-01", "equity": 10, "currency": "CAD", "netDeposits": 1, "accountId": ""}),
        json!({"date": "2024-01-02", "equity": 11, "net_deposits": 2}),
        json!({"date": "2024-01-01", "equity": 5, "currency": "CAD", "accountId": "TFSA"}),
        json!({"date": "2024-01-02", "equity": 6, "account_id": "TFSA", "netDeposits": 3}),
        json!({"date": "2024-01-01", "equity": 7, "accountId": "RRSP"}),
    ]).unwrap();
    let snap = d.snapshot();
    assert_eq!(dates(&snap["navHistory"]), vec!["2024-01-01", "2024-01-02"]);
    assert_eq!(f(&snap["navHistory"][0]["equity"]), 10.0);
    assert_eq!(keys(&snap["navByAccount"]), ["TFSA", "RRSP"].iter().map(|s| s.to_string()).collect());
    assert_eq!(f(&snap["navByAccount"]["TFSA"][0]["equity"]), 5.0);
    assert_eq!(f(&snap["navByAccount"]["TFSA"][1]["netDeposits"]), 3.0);
    assert!(snap["navByAccount"]["TFSA"][0].get("accountId").is_none());
    tables::replace_nav(&d.conn, &[
        json!({"date": "2024-06-01", "equity": 20, "accountId": ""}),
        json!({"date": "2024-06-01", "equity": 8, "accountId": "TFSA"}),
    ]).unwrap();
    let snap = d.snapshot();
    assert_eq!(dates(&snap["navHistory"]), vec!["2024-06-01"]);
    assert_eq!(keys(&snap["navByAccount"]), ["TFSA"].iter().map(|s| s.to_string()).collect());
}

#[test]
fn test_upsert_nav_keeps_existing_days() {
    let d = db();
    tables::replace_nav(&d.conn, &[
        json!({"date": "2024-01-01", "equity": 10, "accountId": ""}),
        json!({"date": "2024-01-01", "equity": 5, "accountId": "TFSA"}),
    ]).unwrap();
    tables::upsert_nav(&d.conn, &[
        json!({"date": "2024-01-02", "equity": 11, "accountId": ""}),
        json!({"date": "2024-01-01", "equity": 6, "accountId": "TFSA"}),
    ]).unwrap();
    let snap = d.snapshot();
    assert_eq!(dates(&snap["navHistory"]), vec!["2024-01-01", "2024-01-02"]);
    assert_eq!(f(&snap["navHistory"][1]["equity"]), 11.0);
    assert_eq!(f(&snap["navByAccount"]["TFSA"][0]["equity"]), 6.0);
    assert_eq!(Value::Object(tables::nav_last_dates(&d.conn).unwrap()), json!({"": "2024-01-02", "TFSA": "2024-01-01"}));
}

/// WatchlistTest. The data-version assertion is the server crate's.
#[test]
fn test_add_list_remove() {
    let d = db();
    let row = feeds::add_watch(&d.conn, "shop", "tsx", "Shopify Inc.", "cad", "", "2026-09-11T14:00:00Z").unwrap().unwrap();
    assert_eq!(
        (row["symbol"].as_str(), row["exchange"].as_str(), row["name"].as_str(), row["currency"].as_str(), row["addedAt"].as_str()),
        (Some("SHOP"), Some("TSX"), Some("Shopify Inc."), Some("CAD"), Some("2026-09-11T14:00:00Z"))
    );
    feeds::add_watch(&d.conn, "NVDA", "NASDAQ", "", "USD", "", "2026-09-11T14:01:00Z").unwrap();
    let syms = |d: &Db| feeds::list_watchlist(&d.conn).unwrap().iter().map(|w| w["symbol"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(syms(&d), vec!["SHOP", "NVDA"], "in the order they were added");
    let again = feeds::add_watch(&d.conn, "SHOP", "TSX", "", "", "", "2026-09-12T00:00:00Z").unwrap().unwrap();
    assert_eq!((again["addedAt"].as_str(), again["name"].as_str()), (Some("2026-09-11T14:00:00Z"), Some("Shopify Inc.")), "adding a followed listing again keeps its place and its name");
    assert_eq!(feeds::add_watch(&d.conn, "NVDA", "NASDAQ", "NVIDIA Corp", "", "", "2026-09-12T00:00:00Z").unwrap().unwrap()["name"], "NVIDIA Corp", "a blank name is filled in");
    assert!(feeds::remove_watch(&d.conn, "shop", "tsx").unwrap());
    assert!(!feeds::remove_watch(&d.conn, "SHOP", "TSX").unwrap());
    assert_eq!(syms(&d), vec!["NVDA"]);
    assert_eq!(d.snapshot()["watchlist"][0]["symbol"], "NVDA", "the snapshot carries it to the model");
}

/// TilesTest, the store half: the rows the model builds from the saved tiles
/// (model and server crates) are not reachable here.
#[test]
fn test_the_row_is_the_default_until_saved_and_then_what_was_saved() {
    let d = db();
    let tiles = |d: &Db| snapshot::tiles_from(&tables::get_meta(&d.conn, snapshot::TILES_META, "").unwrap());
    assert_eq!(tiles(&d), Value::Null, "never saved");
    assert_eq!(d.snapshot()["tiles"], Value::Null);
    let saved = admin::save_tiles(&d.conn, &[json!({"symbol": "tnx", "exchange": "index"}), json!({"symbol": "usdcad", "exchange": "fx"}), json!({"symbol": "", "exchange": "x"}), json!("junk")]).unwrap();
    assert_eq!(saved, json!([{"symbol": "TNX", "exchange": "INDEX"}, {"symbol": "USDCAD", "exchange": "FX"}]));
    assert_eq!(tiles(&d), saved);
    admin::save_tiles(&d.conn, &[]).unwrap();
    assert_eq!(tiles(&d), json!([]), "an emptied row stays empty");
}
