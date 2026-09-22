//! The generation counters: a counter moves when its rows change and only then
//! (docs/architecture.md, rules 3 and 4). A write that changes nothing is not a
//! change, so it must not send every open page to fetch the book again.

use serde_json::json;

use bagholder_store::{feeds, gens, market, open_db, relabel, tables};

fn db() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_db(&dir.path().join("bagholder.db")).unwrap();
    relabel::ensure(&conn).unwrap();
    (dir, conn)
}

fn gen(conn: &rusqlite::Connection, name: &str) -> i64 {
    *gens::all(conn).unwrap().get(name).unwrap_or_else(|| panic!("no counter named {}", name))
}

fn margin_row(power: f64) -> serde_json::Value {
    json!({"accountId": "a1", "buyingPower": power, "currency": "CAD", "unavailable": ""})
}

#[test]
fn test_every_counter_exists_from_the_start() {
    let (_d, conn) = db();
    let all = gens::all(&conn).unwrap();
    for name in gens::names() {
        assert!(all.contains_key(name), "{}", name);
    }
}

#[test]
fn test_margin_read_again_and_found_the_same_is_not_a_change() {
    let (_d, conn) = db();
    tables::replace_margin(&conn, &[margin_row(1000.0)], "2026-09-20T10:00:00Z").unwrap();
    let after_first = gen(&conn, "margin");
    assert!(after_first > 0, "the first figures are a change");
    tables::replace_margin(&conn, &[margin_row(1000.0)], "2026-09-20T10:05:00Z").unwrap();
    assert_eq!(gen(&conn, "margin"), after_first, "the same figures five minutes later move nothing");
    let stamp: String = conn.query_row("SELECT fetched_at FROM margin", [], |r| r.get(0)).unwrap();
    assert_eq!(stamp, "2026-09-20T10:05:00Z", "though when they were read is kept");
    tables::replace_margin(&conn, &[margin_row(900.0)], "2026-09-20T10:10:00Z").unwrap();
    assert!(gen(&conn, "margin") > after_first, "a different figure is a change");
}

#[test]
fn test_accounts_and_balances_replaced_with_themselves_are_not_a_change() {
    let (_d, conn) = db();
    let accounts = [json!({"id": "a1", "nickname": "Trading", "unifiedAccountType": "SELF_DIRECTED_TFSA", "currency": "CAD", "status": "open", "type": "tfsa", "netLiquidationValue": 5000.0})];
    let balances = [json!({"accountId": "a1", "custodianAccountId": "c1", "securityId": "sec-1", "quantity": 10.0})];
    tables::replace_accounts(&conn, &accounts).unwrap();
    tables::replace_balances(&conn, &balances).unwrap();
    let (a, b) = (gen(&conn, "accounts"), gen(&conn, "balances"));
    tables::replace_accounts(&conn, &accounts).unwrap();
    tables::replace_balances(&conn, &balances).unwrap();
    assert_eq!((gen(&conn, "accounts"), gen(&conn, "balances")), (a, b));
    let mut moved = accounts.clone();
    moved[0]["netLiquidationValue"] = json!(5100.0);
    tables::replace_accounts(&conn, &moved).unwrap();
    assert!(gen(&conn, "accounts") > a, "a net liquidation value that moved is a change");
}

#[test]
fn test_news_and_a_universe_read_again_unchanged_are_not_a_change() {
    let (_d, conn) = db();
    let story = [feeds::NewsItem {
        id: "n1".into(), headline: "QNC files its quarter".into(), source: "Newswire".into(), url: "https://example.test/n1".into(),
        published_at: "2026-09-19T12:00:00Z".into(), summary: String::new(), kind: feeds::NewsKind::Story, via: feeds::Feed::Tmx,
    }];
    feeds::replace_news(&conn, "QNC", "TSX-V", &story, "2026-09-20T10:00:00Z").unwrap();
    let n = gen(&conn, "news");
    feeds::replace_news(&conn, "QNC", "TSX-V", &story, "2026-09-20T10:15:00Z").unwrap();
    assert_eq!(gen(&conn, "news"), n);

    let rows = [json!({"symbol": "AAA", "name": "Aaa Corp", "value": 10.0, "percentChange": 1.5, "sector": "Energy", "country": "CA"})];
    feeds::replace_universe(&conn, "tsx60", &rows, "2026-09-20T10:00:00Z").unwrap();
    let u = gen(&conn, "universes");
    feeds::replace_universe(&conn, "tsx60", &rows, "2026-09-20T10:30:00Z").unwrap();
    assert_eq!(gen(&conn, "universes"), u);
    let mut ticked = rows.clone();
    ticked[0]["percentChange"] = json!(1.6);
    feeds::replace_universe(&conn, "tsx60", &ticked, "2026-09-20T11:00:00Z").unwrap();
    assert!(gen(&conn, "universes") > u);
}

#[test]
fn test_a_quote_moves_its_own_counter_and_no_other() {
    let (_d, conn) = db();
    market::upsert_quote(&conn, "QNC", &bagholder_store::market::QuoteRecord { price: Some(1.75), ..Default::default() }, "tmx", "2026-09-20T10:00:00Z").unwrap();
    let before = gens::all(&conn).unwrap();
    market::upsert_quote(&conn, "QNC", &bagholder_store::market::QuoteRecord { price: Some(1.76), ..Default::default() }, "tmx", "2026-09-20T10:01:00Z").unwrap();
    let after = gens::all(&conn).unwrap();
    for (name, n) in &after {
        if name == "quotes" {
            assert!(n > &before[name], "the price moved");
        } else {
            assert_eq!(n, &before[name], "{} did not", name);
        }
    }
    market::upsert_quote(&conn, "QNC", &bagholder_store::market::QuoteRecord { price: Some(1.76), ..Default::default() }, "tmx", "2026-09-20T10:02:00Z").unwrap();
    assert_eq!(gens::all(&conn).unwrap()["quotes"], after["quotes"], "the same price read again is the same quote");
}

#[test]
fn test_an_edit_in_place_is_seen() {
    // the count and the newest date stay as they were: the old fingerprint missed this
    let (_d, conn) = db();
    conn.execute("INSERT INTO activities(id, transaction_date, symbol, quantity) VALUES ('x1', '2026-01-05', 'AAA', 10)", []).unwrap();
    let n = gen(&conn, "activities");
    conn.execute("UPDATE activities SET quantity = 12 WHERE id = 'x1'", []).unwrap();
    assert!(gen(&conn, "activities") > n);
    let n = gen(&conn, "activities");
    conn.execute("UPDATE activities SET quantity = 12 WHERE id = 'x1'", []).unwrap();
    assert_eq!(gen(&conn, "activities"), n, "set to what it already was: not a change");
}

#[test]
fn test_the_journal_and_the_tiles_have_counters_and_other_meta_has_none() {
    let (_d, conn) = db();
    let before = gens::all(&conn).unwrap();
    tables::set_meta(&conn, "journal_v2", "{\"rt:1\":{\"grade\":\"A\"}}").unwrap();
    tables::set_meta(&conn, "market_tiles", "[\"SPY\"]").unwrap();
    tables::set_meta(&conn, "news_fetched:QNC|TSX-V", "2026-09-20T10:00:00Z").unwrap();
    let after = gens::all(&conn).unwrap();
    assert!(after["journal"] > before["journal"]);
    assert!(after["tiles"] > before["tiles"]);
    let moved: Vec<&String> = after.iter().filter(|(k, v)| before[*k] != **v).map(|(k, _)| k).collect();
    assert_eq!(moved, ["journal", "tiles"], "a stamp the model never reads moves nothing");
    tables::set_meta(&conn, "journal_v2", "{\"rt:1\":{\"grade\":\"A\"}}").unwrap();
    assert_eq!(gens::all(&conn).unwrap()["journal"], after["journal"], "saved as it already was");
}

#[test]
fn test_a_column_added_later_is_compared_once_the_triggers_are_remade() {
    let (_d, conn) = db();
    conn.execute_batch("ALTER TABLE watchlist ADD COLUMN note TEXT").unwrap();
    conn.execute("INSERT INTO watchlist(symbol, exchange, name, currency, added_at) VALUES ('AAA', 'TSX', 'Aaa', 'CAD', '2026-09-20')", []).unwrap();
    relabel::ensure(&conn).unwrap(); // a start: the triggers are made again from the columns as they stand
    let n = gen(&conn, "watchlist");
    conn.execute("UPDATE watchlist SET note = 'watching the quarter' WHERE symbol = 'AAA'", []).unwrap();
    assert!(gen(&conn, "watchlist") > n);
}
