//! The quote pipeline, pinned end to end: each source's parser, what the store
//! keeps of its answer, and the fields beyond the stored price that a caller
//! reads (a listing's name and venue). The answers are held in
//! `golden/quote_records.json`, numbers compared as numbers, so a change of
//! representation must leave every stored quote as it was.
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-market --test quote_records`,
//! and read the diff.

use bagholder_market::{parse, quotes, tmx};
use bagholder_store::market;
use serde_json::{json, Map, Value};

fn norm(v: Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.into_iter().map(norm).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, norm(v))).collect::<Map<_, _>>()),
        v => v,
    }
}

const NOW: &str = "2026-09-22T15:00:00Z";

/// The parse, what a caller reads beside the price, and the stored row.
struct Pipe {
    conn: rusqlite::Connection,
    out: Map<String, Value>,
}

impl Pipe {
    fn new() -> Pipe {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        bagholder_store::schema::init_schema(&conn).unwrap();
        Pipe { conn, out: Map::new() }
    }

    fn put<T: serde::Serialize>(&mut self, name: &str, source: &str, parsed: Option<T>, store: impl Fn(&rusqlite::Connection, &str, &T)) {
        let cell = match parsed {
            None => Value::Null,
            Some(q) => {
                let sym = name.to_uppercase();
                store(&self.conn, &sym, &q);
                let seen = serde_json::to_value(&q).unwrap();
                let mut stored = market::quotes(&self.conn).unwrap().remove(&sym).map_or(Value::Null, |q| serde_json::to_value(q).unwrap());
                if let Value::Object(m) = &mut stored {
                    m.remove("source");
                    m.remove("fetchedAt");
                }
                let beside: Map<String, Value> = ["name", "exchange", "currency"]
                    .iter()
                    .filter_map(|k| seen.get(*k).filter(|v| !v.is_null() && *v != "").map(|v| (k.to_string(), v.clone())))
                    .collect();
                json!({"source": source, "stored": stored, "beside": beside})
            }
        };
        self.out.insert(name.to_string(), norm(cell));
    }
}

fn store_rec(conn: &rusqlite::Connection, sym: &str, q: &quotes::SourceQuote, source: &str) {
    market::upsert_quote(conn, sym, &q.quote, source, NOW).unwrap();
}

fn answers() -> Value {
    let mut p = Pipe::new();
    let s = |source: &'static str| move |c: &rusqlite::Connection, sym: &str, q: &quotes::SourceQuote| store_rec(c, sym, q, source);

    // TMX
    let tmx_full = json!({"data": {"getQuoteBySymbol": {
        "price": 4.87, "priceChange": -0.03, "percentChange": -0.612, "prevClose": 4.9, "currency": "CAD",
        "dividendAmount": 0.15, "dividendFrequency": "Monthly", "exDividendDate": "2026-09-30T00:00:00.000",
        "name": "Reddy Fund", "exchangeName": "Toronto Stock Exchange"}}});
    p.put("tmx_full", "tmx", tmx::parse_tmx_quote(&tmx_full), s("tmx"));
    let tmx_text = json!({"data": {"getQuoteBySymbol": {
        "price": "1.25", "priceChange": null, "percentChange": "", "prevClose": "1.2", "currency": null,
        "dividendAmount": null, "dividendFrequency": null, "exDividendDate": null, "name": "", "exchangeName": "TSX Venture Exchange"}}});
    p.put("tmx_text", "tmx", tmx::parse_tmx_quote(&tmx_text), s("tmx"));
    let tmx_no_price = json!({"data": {"getQuoteBySymbol": {"price": null, "name": "Halted", "exchangeName": "CSE"}}});
    p.put("tmx_no_price", "tmx", tmx::parse_tmx_quote(&tmx_no_price), s("tmx"));
    p.put("tmx_empty", "tmx", tmx::parse_tmx_quote(&json!({"data": {"getQuoteBySymbol": {}}})), s("tmx"));
    p.put("tmx_null", "tmx", tmx::parse_tmx_quote(&json!({"data": {"getQuoteBySymbol": null}})), s("tmx"));
    p.put("tmx_nothing", "tmx", tmx::parse_tmx_quote(&Value::Null), s("tmx"));

    // Cboe Canada
    let ca = |d: Value| parse::parse_cboe_ca_quote(&json!({"data": d}).to_string());
    p.put("cboe_ca_live", "cboe_ca", ca(json!({"last": 12.5, "prev_close": 12.0, "change": 0.5, "change_pct": 4.1667, "company_name": "Apple CDR"})), s("cboe_ca"));
    p.put("cboe_ca_closed", "cboe_ca", ca(json!({"last": 0, "prev_close": 12.0, "change": "0", "change_pct": null, "company_name": ""})), s("cboe_ca"));
    p.put("cboe_ca_text", "cboe_ca", ca(json!({"last": "3.10", "prev_close": "", "change": "0.1", "change_pct": "3.33"})), s("cboe_ca"));
    p.put("cboe_ca_none", "cboe_ca", ca(json!({"last": 0, "prev_close": null})), s("cboe_ca"));
    p.put("cboe_ca_bad", "cboe_ca", parse::parse_cboe_ca_quote("not json"), s("cboe_ca"));
    p.put("cboe_ca_empty", "cboe_ca", parse::parse_cboe_ca_quote(""), s("cboe_ca"));

    // Cboe options
    let chain = parse::parse_cboe_options(&json!({"data": {"options": [
        {"option": "AAPL260918C00200000", "bid": 5.1, "ask": 5.3, "last_trade_price": 5.0, "prev_day_close": 4.8},
        {"option": "AAPL260918P00150000", "bid": 0, "ask": 0.2, "last_trade_price": 0.15, "prev_day_close": 0.1},
        {"option": "AAPL260918P00100000", "bid": 0, "ask": 0, "last_trade_price": 0, "prev_day_close": 0.05},
        {"option": "AAPL260918P00050000", "bid": null, "ask": null, "last_trade_price": null, "prev_day_close": null},
        {"option": "AAPL260918C00300000", "bid": "1.0", "ask": "1.2", "last_trade_price": "", "prev_day_close": 0},
        "not a row"
    ]}}).to_string());
    let mut codes: Vec<&String> = chain.keys().collect();
    codes.sort();
    p.out.insert("options_chain_codes".into(), json!(codes));
    for code in ["AAPL260918C00200000", "AAPL260918P00150000", "AAPL260918P00100000", "AAPL260918P00050000", "AAPL260918C00300000", "MISSING"] {
        p.put(&format!("option_{}", code), "cboe_options", chain.get(code).and_then(parse::option_mark), s("cboe_options"));
    }

    // Coinbase
    p.put("coinbase_ccy", "coinbase", parse::parse_coinbase_rec(r#"{"data":{"amount":"64250.12","currency":"CAD"}}"#, "BTC-CAD"), s("coinbase"));
    p.put("coinbase_pair", "coinbase", parse::parse_coinbase_rec(r#"{"data":{"amount":3100.5}}"#, "ETH-USD"), s("coinbase"));
    p.put("coinbase_zero", "coinbase", parse::parse_coinbase_rec(r#"{"data":{"amount":"0"}}"#, "BTC-CAD"), s("coinbase"));
    p.put("coinbase_none", "coinbase", parse::parse_coinbase_rec(r#"{"data":{}}"#, "BTC-CAD"), s("coinbase"));
    p.put("coinbase_bad", "coinbase", parse::parse_coinbase_rec("", "BTC-CAD"), s("coinbase"));
    let spot = parse::parse_coinbase_rec(r#"{"data":{"amount":"110"}}"#, "SOL-CAD").map(|r| quotes::with_prev_close(r, 100.0));
    p.put("coinbase_with_prev", "coinbase", spot, s("coinbase"));

    // Yahoo
    let y = |meta: Value| quotes::parse_yahoo_quote(&json!({"chart": {"result": [{"meta": meta}]}}).to_string());
    p.put("yahoo_full", "yahoo_quote", y(json!({"regularMarketPrice": 190.5, "chartPreviousClose": 188.0, "previousClose": 150.0, "currency": "USD", "shortName": "Apple Inc.", "longName": "Apple Incorporated", "exchangeName": "NMS"})), s("yahoo_quote"));
    p.put("yahoo_prev_only", "yahoo_quote", y(json!({"regularMarketPrice": 10, "previousClose": 8, "longName": "Long Only"})), s("yahoo_quote"));
    p.put("yahoo_prev_zero", "yahoo_quote", y(json!({"regularMarketPrice": 10, "chartPreviousClose": 0})), s("yahoo_quote"));
    p.put("yahoo_no_price", "yahoo_quote", y(json!({"chartPreviousClose": 5})), s("yahoo_quote"));
    p.put("yahoo_not_object", "yahoo_quote", y(json!(3)), s("yahoo_quote"));
    p.put("yahoo_empty", "yahoo_quote", quotes::parse_yahoo_quote(""), s("yahoo_quote"));

    // A price feed that says nothing of dividends keeps what the store knew;
    // one that names them replaces them.
    p.put("kept_dividend", "tmx", tmx::parse_tmx_quote(&tmx_full), s("tmx"));
    p.put("kept_dividend", "yahoo_quote", y(json!({"regularMarketPrice": 5.0, "chartPreviousClose": 4.87})), s("yahoo_quote"));
    let tmx_new_div = json!({"data": {"getQuoteBySymbol": {"price": 5.0, "dividendAmount": 0.2, "dividendFrequency": "Quarterly", "exDividendDate": "2026-12-30"}}});
    p.put("replaced_dividend", "tmx", tmx::parse_tmx_quote(&tmx_full), s("tmx"));
    p.put("replaced_dividend", "tmx", tmx::parse_tmx_quote(&tmx_new_div), s("tmx"));

    Value::Object(p.out)
}

#[test]
fn test_every_quote_source_is_stored_as_it_was() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/quote_records.json");
    let have = norm(answers());
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
        return;
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/quote_records.json")).unwrap();
    for (k, v) in want.as_object().unwrap() {
        assert_eq!(&have[k], v, "{} is not what it was", k);
    }
    assert_eq!(have.as_object().unwrap().len(), want.as_object().unwrap().len());
}

#[test]
fn test_the_stamp_is_the_time_of_the_write() {
    let p = Pipe::new();
    store_rec(&p.conn, "abc", &tmx::parse_tmx_quote(&json!({"data": {"getQuoteBySymbol": {"price": 1.0}}})).unwrap(), "tmx");
    let q = market::quotes(&p.conn).unwrap().remove("ABC").unwrap();
    assert_eq!(q.fetched_at, NOW);
    assert_eq!(q.source, "tmx");
}
