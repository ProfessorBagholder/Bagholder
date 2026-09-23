//! The model's inputs read straight from the columns (`rows`), for rows of
//! every shape: what the store's writers leave behind is what its readers
//! come back with.

mod common;
use bagholder_store::{rows, tables};
use common::*;

#[test]
fn test_every_input_reads_cleanly_from_rows_of_every_shape() {
    let d = db();
    d.conn
        .execute_batch(
            r#"
        INSERT INTO securities(id,symbol,name,primary_exchange,primary_mic,currency,underlying_id,fetched_at) VALUES ('sec-1','QNC','Quantum','TSXV','XTSX','CAD',NULL,'t'), ('sec-2','',NULL,NULL,NULL,NULL,'sec-1',NULL);
        INSERT INTO accounts(id,nickname,unified_account_type,currency,status,type,net_liquidation_value) VALUES ('acct-1','Main','TFSA','CAD','open','ca_tfsa',1234.5), ('acct-2',NULL,NULL,NULL,NULL,NULL,NULL);
        INSERT INTO balances(account_id, custodian_account_id, security_id, quantity) VALUES ('acct-1','cust','sec-1',10), (NULL,NULL,NULL,NULL), ('acct-2','','sec-2',2.5);
        INSERT INTO margin(account_id,buying_power,currency,unavailable,fetched_at) VALUES ('acct-1',5000,'USD','',NULL), ('acct-2',NULL,NULL,'no margin','t');
        INSERT INTO nav_history(account_id,date,equity,currency,net_deposits) VALUES ('','2026-01-02',100,'CAD',90), ('','2026-01-03',NULL,NULL,NULL), ('acct-1','2026-01-02',50,'',NULL), ('acct-2','2026-01-02',7,'CAD',7);
        INSERT INTO exposures(key,sectors,countries,coverage,source,as_of,industry,error,fetched_at) VALUES ('sec-1','{"Tech":0.6,"Energy":"0.4"}','{"Canada":1}',1,'src','2026','ind','',NULL), ('sec-2','not json','',NULL,NULL,NULL,NULL,NULL,NULL), ('fund:X',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
        INSERT INTO watchlist(symbol,exchange,name,currency,security_id,added_at) VALUES ('ENB','TSX','Enbridge','CAD','sec-9','2026-01-01'), ('AAPL','',NULL,NULL,NULL,NULL);
        INSERT INTO news(id,symbol,exchange,source,headline,wire,url,published_at,fetched_at,kind,summary) VALUES ('n1','QNC','TSXV','src','Up','Newsfile','http://x','2026-01-02','t','release','sum'), ('n2','*','MARKET',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
        INSERT INTO universes(key,symbol,name,value,percent_change,sector,country,fetched_at) VALUES ('tsx','RY','Royal',150,1.2,'Financials','Canada','t'), ('tsx','TD',NULL,NULL,NULL,NULL,NULL,NULL), ('sp','AAPL','Apple',3000,-0.5,'Tech','US','t');
        INSERT INTO distributions(symbol,ex_date,pay_date,amount,currency,source) VALUES ('ZWC','2026-01-28','2026-02-05',0.1,'CAD','tmx'), ('ZWC','2025-12-29',NULL,0.1,NULL,'tmx'), ('HMAX','2026-01-30','',0.2,'CAD','issuer');
        INSERT INTO quotes(symbol, price, price_change, percent_change, source, ex_dividend_date) VALUES ('QNC',1.75,0.05,2.9,'tmx','2026-01-28'), ('BTC',NULL,NULL,NULL,NULL,NULL), ('ENB@TSX',60,NULL,NULL,'coinbase','');
        INSERT INTO fx_rates(pair,date,rate) VALUES ('USDCAD','2026-01-02',1.36), ('USDCAD','2026-01-03',1.37), ('EURCAD','2026-01-02',1.5);
        INSERT INTO benchmark_prices(symbol,date,close) VALUES ('TSX','2026-01-02',25000), ('SP500','2026-01-02',6000), ('TSX60','2026-01-03',1500);
        "#,
        )
        .unwrap();
    tables::set_meta(&d.conn, "trade_groups", r#"[{"id":"g1","members":["rt:a"," rt:b ","rt:a"]},{"id":"","members":["x"]},{"id":"g2","members":[]}]"#).unwrap();
    tables::set_meta(&d.conn, tables::JOURNAL_META, r#"{"rt:a":{"thesis":"t","tags":"a, b","grade":"b"},"rt:c":{"thesis":""},"  ":{"thesis":"x"}}"#).unwrap();
    tables::set_meta(&d.conn, rows::TILES_META, r#"[{"symbol":" ry ","exchange":"tsx"},{"symbol":""},7]"#).unwrap();

    let c = &d.conn;
    let (nav, by_account) = rows::nav(c).unwrap();
    assert_eq!((nav.len(), by_account.len(), rows::universes(c).unwrap().0.len()), (2, 2, 2));
    assert_eq!(rows::exposures(c).unwrap()["sec-1"].sectors, vec![("Tech".to_string(), 0.6), ("Energy".to_string(), 0.4)]);
    assert_eq!(rows::news(c).unwrap().iter().find(|n| n.id == "n2").unwrap().kind, "story");
    assert_eq!(rows::margin(c).unwrap()[1].currency, "CAD");
    assert_eq!(rows::groups(c).unwrap().len(), 1);
    assert_eq!(rows::groups(c).unwrap()[0].members, vec!["rt:a".to_string(), "rt:b".to_string()]);
    assert_eq!(rows::journal(c).unwrap().get("rt:a").map(|e| e.tags.clone()), Some(vec!["a".to_string(), "b".to_string()]));
    assert_eq!(rows::tiles(c).unwrap().unwrap().len(), 1);
    assert_eq!(rows::securities(c).unwrap().len(), 2);
    assert_eq!(rows::accounts(c).unwrap().len(), 2);
    assert_eq!(rows::balances(c).unwrap().len(), 3);
    assert_eq!(rows::watchlist(c).unwrap().len(), 2);
    assert_eq!(rows::distributions(c).unwrap()["ZWC"].len(), 2);
    assert_eq!(rows::quotes(c).unwrap().len(), 3);
    assert_eq!(rows::fx(c, tables::FX_PAIR).unwrap().len(), 2);
    assert_eq!(rows::benchmark(c, tables::BENCHMARK_SYMBOL).unwrap().len(), 1);

    // never saved is not saved empty
    tables::set_meta(c, rows::TILES_META, "").unwrap();
    assert_eq!(rows::tiles(c).unwrap(), None);
}
