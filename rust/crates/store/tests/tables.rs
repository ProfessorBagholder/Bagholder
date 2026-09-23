//! Margin, securities, trade groups and notes, NAV, the watchlist and the tile
//! row.

mod common;
use bagholder_store::{admin, feeds, rows, schema, tables};
use bagholder_store::tables::LegacyNote;
use bagholder_model::input::{TileRef, TradeGroup};
use common::*;

use serde_json::json;
use std::collections::{BTreeMap, HashSet};

#[test]
fn test_margin_rows_round_trip_and_clear_with_the_synced_data() {
    let d = db();
    tables::replace_margin(&d.conn, &typed_rows(&[
        json!({"accountId": "acct-1", "buyingPower": 6817.33, "currency": "CAD"}),
        json!({"accountId": "acct-2", "buyingPower": null, "currency": "CAD", "unavailable": "UnavailableSecurities (2 securities)"}),
        json!({"accountId": "", "buyingPower": 1.0}),
    ]), "2026-09-10T14:00:00Z").unwrap();
    let rows = tables::margin(&d.conn).unwrap();
    let got: Vec<(String, Option<f64>, String)> = rows.iter().map(|r| (r.account_id.clone(), r.buying_power, r.unavailable.clone())).collect();
    assert_eq!(got, vec![
        ("acct-1".to_string(), Some(6817.33), String::new()),
        ("acct-2".to_string(), None, "UnavailableSecurities (2 securities)".to_string()),
    ]);
    assert!(rows.iter().all(|r| !r.fetched_at.is_empty()));
    // the data version moving with a new reading is the server crate's `data_version`
    tables::replace_margin(&d.conn, &typed_rows(&[json!({"accountId": "acct-1", "buyingPower": 6900.0, "currency": "CAD"})]), "2026-09-10T15:00:00Z").unwrap();
    assert_eq!(tables::margin(&d.conn).unwrap()[0].buying_power, Some(6900.0));
    admin::clear_synced_data(&d.conn, true, true).unwrap();
    assert_eq!(tables::margin(&d.conn).unwrap(), Vec::new());
}

#[test]
fn test_snapshot_includes_securities() {
    let d = db();
    admin::upsert_securities(&d.conn, &typed_rows(&[json!({"id": "sec-s-ch", "symbol": "CH", "name": "Charbone Corporation", "primaryExchange": "TSX Venture Exchange", "primaryMic": "XTSV", "currency": "CAD"})]), "2026-09-10T14:00:00Z").unwrap();
    let secs = admin::list_securities(&d.conn).unwrap();
    assert_eq!(secs.len(), 1);
    assert_eq!(secs[0].id, "sec-s-ch");
    assert_eq!(secs[0].name, "Charbone Corporation");
    assert_eq!(secs[0].primary_mic, "XTSV");
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
    let raw: Vec<TradeGroup> = bagholder_model::lenient::rows(&groups);
    let saved = tables::save_trade_groups(&d.conn, &raw).unwrap();
    assert_eq!(saved.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), vec!["g_one", "g_two"]);
    assert_eq!(saved[0].members, vec!["a|b|1.00000000".to_string(), "c|d|2.00000000".to_string()]);
    assert_eq!(saved[1].members, vec!["x".to_string(), "y".to_string()]);
    assert!(saved[0].locked);
    assert!(saved[1].locked);
    assert_eq!(tables::trade_groups(&d.conn).unwrap(), saved);
}

#[test]
fn test_trade_groups_rejects_non_list() {
    let d = db();
    tables::set_meta(&d.conn, "trade_groups", "{}").unwrap();
    assert_eq!(tables::trade_groups(&d.conn).unwrap(), Vec::<TradeGroup>::new());
    assert_eq!(tables::save_trade_groups(&d.conn, &[]).unwrap(), Vec::<TradeGroup>::new());
}

#[test]
fn test_trade_notes_roundtrip() {
    let d = db();
    let raw: BTreeMap<String, LegacyNote> = bagholder_model::lenient::objmap(&json!({
        "g_one": {"thesis": "scale in", "tag": "hold", "grade": "A"},
        "g_empty": {"thesis": "", "tag": "", "grade": ""},
        "g_bad": {"thesis": "x", "tag": "y", "grade": "Z"},
        "": {"thesis": "nope"},
    }));
    let saved = tables::save_trade_notes(&d.conn, &raw).unwrap();
    assert_eq!(saved["g_one"].grade, "A");
    assert_eq!(saved["g_one"].tag, "hold");
    assert!(!saved.contains_key("g_empty"));
    assert_eq!(saved["g_bad"].grade, "");
    assert_eq!(saved["g_bad"].thesis, "x");
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
    assert_eq!(tables::get_meta(&d.conn, "schema_version", "").unwrap(), schema::SCHEMA_VERSION.to_string());
    let (nav, by_account) = rows::nav(&d.conn).unwrap();
    assert_eq!(nav.len(), 1);
    assert_eq!(nav[0].date, "2024-01-02");
    assert_eq!(nav[0].equity, Some(1000.0));
    assert_eq!(nav[0].net_deposits, Some(100.0));
    assert!(by_account.is_empty());
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

#[test]
fn test_replace_nav_by_account_and_snapshot() {
    let d = db();
    tables::replace_nav(&d.conn, &typed_rows(&[
        json!({"date": "2024-01-01", "equity": 10, "currency": "CAD", "netDeposits": 1, "accountId": ""}),
        json!({"date": "2024-01-02", "equity": 11, "netDeposits": 2}),
        json!({"date": "2024-01-01", "equity": 5, "currency": "CAD", "accountId": "TFSA"}),
        json!({"date": "2024-01-02", "equity": 6, "accountId": "TFSA", "netDeposits": 3}),
        json!({"date": "2024-01-01", "equity": 7, "accountId": "RRSP"}),
    ])).unwrap();
    let (nav, by_account) = rows::nav(&d.conn).unwrap();
    assert_eq!(nav.iter().map(|p| p.date.clone()).collect::<Vec<_>>(), vec!["2024-01-01", "2024-01-02"]);
    assert_eq!(nav[0].equity, Some(10.0));
    assert_eq!(by_account.keys().cloned().collect::<HashSet<_>>(), ["TFSA", "RRSP"].iter().map(|s| s.to_string()).collect());
    assert_eq!(by_account["TFSA"][0].equity, Some(5.0));
    assert_eq!(by_account["TFSA"][1].net_deposits, Some(3.0));
    tables::replace_nav(&d.conn, &typed_rows(&[
        json!({"date": "2024-06-01", "equity": 20, "accountId": ""}),
        json!({"date": "2024-06-01", "equity": 8, "accountId": "TFSA"}),
    ])).unwrap();
    let (nav, by_account) = rows::nav(&d.conn).unwrap();
    assert_eq!(nav.iter().map(|p| p.date.clone()).collect::<Vec<_>>(), vec!["2024-06-01"]);
    assert_eq!(by_account.keys().cloned().collect::<HashSet<_>>(), ["TFSA"].iter().map(|s| s.to_string()).collect());
}

#[test]
fn test_upsert_nav_keeps_existing_days() {
    let d = db();
    tables::replace_nav(&d.conn, &typed_rows(&[
        json!({"date": "2024-01-01", "equity": 10, "accountId": ""}),
        json!({"date": "2024-01-01", "equity": 5, "accountId": "TFSA"}),
    ])).unwrap();
    tables::upsert_nav(&d.conn, &typed_rows(&[
        json!({"date": "2024-01-02", "equity": 11, "accountId": ""}),
        json!({"date": "2024-01-01", "equity": 6, "accountId": "TFSA"}),
    ])).unwrap();
    let (nav, by_account) = rows::nav(&d.conn).unwrap();
    assert_eq!(nav.iter().map(|p| p.date.clone()).collect::<Vec<_>>(), vec!["2024-01-01", "2024-01-02"]);
    assert_eq!(nav[1].equity, Some(11.0));
    assert_eq!(by_account["TFSA"][0].equity, Some(6.0));
    assert_eq!(json!(tables::nav_last_dates(&d.conn).unwrap()), json!({"": "2024-01-02", "TFSA": "2024-01-01"}));
}

/// WatchlistTest. The data-version assertion is the server crate's.
#[test]
fn test_add_list_remove() {
    let d = db();
    let row = feeds::add_watch(&d.conn, "shop", "tsx", "Shopify Inc.", "cad", "", "2026-09-11T14:00:00Z").unwrap().unwrap();
    assert_eq!(
        (row.symbol.as_str(), row.exchange.as_str(), row.name.as_str(), row.currency.as_str(), row.added_at.as_str()),
        ("SHOP", "TSX", "Shopify Inc.", "CAD", "2026-09-11T14:00:00Z")
    );
    feeds::add_watch(&d.conn, "NVDA", "NASDAQ", "", "USD", "", "2026-09-11T14:01:00Z").unwrap();
    let syms = |d: &Db| feeds::list_watchlist(&d.conn).unwrap().iter().map(|w| w.symbol.clone()).collect::<Vec<_>>();
    assert_eq!(syms(&d), vec!["SHOP", "NVDA"], "in the order they were added");
    let again = feeds::add_watch(&d.conn, "SHOP", "TSX", "", "", "", "2026-09-12T00:00:00Z").unwrap().unwrap();
    assert_eq!((again.added_at.as_str(), again.name.as_str()), ("2026-09-11T14:00:00Z", "Shopify Inc."), "adding a followed listing again keeps its place and its name");
    assert_eq!(feeds::add_watch(&d.conn, "NVDA", "NASDAQ", "NVIDIA Corp", "", "", "2026-09-12T00:00:00Z").unwrap().unwrap().name, "NVIDIA Corp", "a blank name is filled in");
    assert!(feeds::remove_watch(&d.conn, "shop", "tsx").unwrap());
    assert!(!feeds::remove_watch(&d.conn, "SHOP", "TSX").unwrap());
    assert_eq!(syms(&d), vec!["NVDA"]);
    assert_eq!(feeds::list_watchlist(&d.conn).unwrap()[0].symbol, "NVDA", "the snapshot carries it to the model");
}

/// TilesTest, the store half: the rows the model builds from the saved tiles
/// (model and server crates) are not reachable here.
#[test]
fn test_the_row_is_the_default_until_saved_and_then_what_was_saved() {
    let d = db();
    assert_eq!(rows::tiles(&d.conn).unwrap(), None, "never saved");
    let saved = admin::save_tiles(&d.conn, &[
        TileRef { symbol: " tnx ".into(), exchange: "index".into() },
        TileRef { symbol: " usdcad ".into(), exchange: "fx".into() },
        TileRef { symbol: "".into(), exchange: "x".into() },
    ]).unwrap();
    assert_eq!(saved, vec![
        TileRef { symbol: "TNX".into(), exchange: "INDEX".into() },
        TileRef { symbol: "USDCAD".into(), exchange: "FX".into() },
    ]);
    assert_eq!(rows::tiles(&d.conn).unwrap(), Some(saved));
    admin::save_tiles(&d.conn, &[]).unwrap();
    assert_eq!(rows::tiles(&d.conn).unwrap(), Some(vec![]), "an emptied row stays empty");
}

/// The sync loop sleeps until the pull window opens: the next weekday, 2:00 PM Mountain.
#[test]
fn test_the_next_pull_window_is_a_known_moment() {
    use bagholder_store::admin::seconds_until_pull_window;
    // Wednesday 2026-09-16, 10:00 Mountain: today's window, four hours on
    assert_eq!(seconds_until_pull_window(1_789_574_400), 4 * 3600);
    // Friday 2026-09-18, 15:00 Mountain: past today's, so Monday's, 71 hours on
    assert_eq!(seconds_until_pull_window(1_789_765_200), 71 * 3600);
    // part-way through a minute: to the second
    assert_eq!(seconds_until_pull_window(1_789_574_400 + 25), 4 * 3600 - 25);
}
