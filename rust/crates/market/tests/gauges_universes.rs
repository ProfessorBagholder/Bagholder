//! The published gauges and the market universes, pinned from the publisher's
//! answer to what the store keeps and reads back. The answers are held in
//! `golden/gauges_universes.json`, so a change of representation must leave
//! every reading and every tile as it was.
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-market --test gauges_universes`,
//! and read the diff.

use bagholder_market::{fear, universes};
use bagholder_store::feeds as sf;
use serde_json::{json, Map, Value};

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

fn cnn() -> Value {
    json!({
        "fear_and_greed": {"score": 28.6571428571429, "rating": "fear", "timestamp": "2026-09-15T19:59:51-04:00",
                           "previous_close": 31.0571428571429, "previous_1_week": 39.142857142857146,
                           "previous_1_month": null, "previous_1_year": "64.45714285714287"},
        "fear_and_greed_historical": {"data": [{"x": 1789516791000.0, "y": 28.6571428571429, "rating": "fear"},
                                               {"x": 1757980800000.0, "y": 64.37142857142858, "rating": "greed"},
                                               {"x": 1757894400000.0, "y": null},
                                               {"y": 50}]},
        "market_momentum_sp125": {"score": 22.8, "rating": "extreme fear", "data": []},
        "market_momentum_sp500": {"score": 99.0, "rating": "extreme greed", "data": []},
        "stock_price_strength": {"score": 1, "rating": "", "data": []},
        "stock_price_breadth": {"score": null, "rating": "extreme fear"},
        "put_call_options": {"score": "32.2", "rating": "FEAR", "data": []},
        "market_volatility_vix_50": {"score": 50, "rating": "neutral", "data": []},
        "junk_bond_demand": {"score": 58.6, "data": []},
        "safe_haven_demand": {"score": 31, "rating": "fear", "data": []},
    })
}

fn crypto() -> Value {
    let mut data = vec![];
    for i in 0..40 {
        data.push(json!({"value": format!("{}", 20 + i), "value_classification": if i % 3 == 0 { "" } else { "Fear" }, "timestamp": format!("{}", 1789516800 - i * 86400)}));
    }
    data.push(json!({"value": null, "timestamp": "1786000000"}));
    json!({"data": data})
}

fn screener() -> Value {
    json!({"data": {"rows": [
        {"symbol": "NVDA ", "name": " NVIDIA Corporation Common Stock", "lastsale": "$219.41", "pctchange": "0.808%", "marketCap": "5,350,000,000,000.00", "sector": "Technology", "country": "United States"},
        {"symbol": "JPM", "name": "JP Morgan", "lastsale": "$210.00", "pctchange": "-0.5%", "marketCap": "600000000000.00", "sector": "Finance", "country": "United States"},
        {"symbol": "TSM", "name": "Taiwan Semiconductor", "lastsale": 250, "pctchange": "1.2%", "marketCap": 1300000000000u64, "sector": "Technology", "country": "Taiwan"},
        {"symbol": "SHOP", "name": "Shopify", "lastsale": "$130.00", "pctchange": "3.3%", "marketCap": "170000000000.00", "sector": "Technology", "country": "Canada"},
        {"symbol": "XYZ", "name": "No cap", "lastsale": "N/A", "pctchange": "N/A", "marketCap": "", "sector": "Miscellaneous", "country": "United States"},
        {"symbol": "ABC", "name": "No country", "lastsale": "$2.00", "pctchange": "-0.0%", "marketCap": "100.00", "sector": "Telecommunications", "country": ""},
        {"symbol": "", "name": "No symbol", "marketCap": "5"},
        "not a row",
        {"symbol": "EQL", "name": "Equal cap", "lastsale": "$1", "pctchange": "1%", "marketCap": "600000000000.00", "sector": "Finance", "country": "United States"},
    ]}})
}

fn answers() -> Value {
    let mut out = Map::new();
    let stocks = fear::parse_stocks(&cnn());
    let coin = fear::parse_crypto(&crypto());
    out.insert("stocks".into(), cell(&stocks));
    out.insert("crypto".into(), cell(&coin));
    out.insert("stocks_no_score".into(), cell(&fear::parse_stocks(&json!({"fear_and_greed": {"rating": "fear"}}))));
    out.insert("crypto_empty".into(), cell(&fear::parse_crypto(&json!({"data": [{"value": null}]}))));

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    bagholder_store::schema::init_schema(&conn).unwrap();
    const NOW: &str = "2026-09-22T15:00:00Z";
    sf::save_gauge(&conn, "Stocks", stocks.as_ref().unwrap(), NOW, 1).unwrap();
    sf::save_gauge(&conn, "crypto", coin.as_ref().unwrap(), NOW, 1).unwrap();
    out.insert("stored_stocks".into(), cell(&sf::gauge(&conn, "stocks").unwrap()));
    out.insert("stored_crypto".into(), cell(&sf::gauge(&conn, " CRYPTO").unwrap()));
    out.insert("stored_missing".into(), cell(&sf::gauge(&conn, "gold").unwrap()));

    let rows = universes::parse_screener(&screener());
    out.insert("screener".into(), cell(&rows));
    let us = universes::us_rows(&rows, 3);
    let intl = universes::intl_rows(&rows, 5);
    out.insert("us".into(), cell(&us));
    out.insert("intl".into(), cell(&intl));
    out.insert("constituents".into(), cell(&universes::parse_constituents(&json!({"data": {"constituents": [
        {"symbol": "RY", "quotedMarketValue": 398317400940u64, "longName": "Royal Bank of Canada", "weight": 9.823, "exchange": "TSX"},
        {"symbol": "TD ", "quotedMarketValue": "150000000000", "shortName": "TD Bank", "weight": null, "exchange": " TSX"},
        {"weight": 1},
    ]}}))));
    out.insert("tile_quote".into(), cell(&universes::parse_tile_quote(&json!({"data": {"getQuoteBySymbol": {"symbol": "RY", "name": " Royal Bank", "price": 180.1, "percentChange": 0.42, "sector": "Financial Services"}}}))));
    out.insert("tile_quote_blank".into(), cell(&universes::parse_tile_quote(&json!({"data": {"getQuoteBySymbol": {"name": "X", "percentChange": null, "sector": ""}}}))));
    out.insert("tile_quote_none".into(), cell(&universes::parse_tile_quote(&json!({"data": {"getQuoteBySymbol": {}}}))));

    sf::replace_universe(&conn, "us", &us, NOW).unwrap();
    sf::replace_universe(&conn, "intl", &intl, NOW).unwrap();
    let tile = |symbol: &str, name: &str, value: f64, percent_change: Option<f64>, sector: &str, country: &str| bagholder_model::input::UniverseRow {
        symbol: symbol.into(), name: name.into(), value, percent_change, sector: sector.into(), country: country.into(),
    };
    let ry = universes::Constituent { symbol: "RY".into(), name: "Royal Bank of Canada".into(), weight: 9.823, cap: 1.0, exchange: "TSX".into() };
    let td = universes::Constituent { symbol: "TD".into(), name: "TD Bank".into(), weight: 0.0, cap: 150000000000.0, exchange: "TSX".into() };
    let ry_quote = universes::TileQuote { percent_change: Some(0.42), sector: "Financials".into(), name: String::new() };
    let ca = vec![
        universes::canada_tile(&ry, Some(&ry_quote)),
        universes::canada_tile(&td, None),
        tile("", "dropped", 1.0, None, "", ""),
    ];
    out.insert("canada_tiles".into(), cell(&ca));
    sf::replace_universe(&conn, "ca", &ca, NOW).unwrap();
    let read: Vec<(String, Vec<Value>)> = bagholder_store::rows::universes(&conn)
        .unwrap()
        .0
        .into_iter()
        .map(|(k, rows)| (k, rows.iter().map(|r| json!({"symbol": r.symbol, "name": r.name, "value": r.value, "percentChange": r.percent_change, "sector": r.sector, "country": r.country})).collect()))
        .collect();
    out.insert("stored_universes".into(), cell(&read));
    out.insert("snapshot_universes".into(), cell(&bagholder_store::feeds::stored_universes(&conn).unwrap()));
    Value::Object(out)
}

#[test]
fn test_every_gauge_and_universe_is_what_it_was() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/gauges_universes.json");
    let have = norm(answers());
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
        return;
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/gauges_universes.json")).unwrap();
    for (k, v) in want.as_object().unwrap() {
        assert_eq!(&have[k], v, "{} is not what it was", k);
    }
    assert_eq!(have.as_object().unwrap().len(), want.as_object().unwrap().len());
}
