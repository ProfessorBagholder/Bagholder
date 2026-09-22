//! The model's inputs read straight from the columns (`rows`) are what the model
//! reads from the snapshot's JSON rows, part by part, for rows of every shape.

mod common;
use bagholder_model::base::Inputs;
use bagholder_store::{market, rows, snapshot, tables};
use common::*;

#[test]
fn test_every_input_reads_the_same_with_and_without_json_between() {
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
    tables::set_meta(&d.conn, snapshot::TILES_META, r#"[{"symbol":" ry ","exchange":"tsx"},{"symbol":""},7]"#).unwrap();

    let snap = snapshot::snapshot(&d.conn, false).unwrap();
    let through_json = Inputs::from_snapshot(&snap, &market::market_data(&d.conn).unwrap(), &snapshot::journal(&d.conn).unwrap());

    let c = &d.conn;
    assert_eq!(*through_json.securities, rows::securities(c).unwrap());
    assert_eq!(*through_json.accounts, rows::accounts(c).unwrap());
    assert_eq!(*through_json.balances, rows::balances(c).unwrap());
    assert_eq!(*through_json.margin, rows::margin(c).unwrap());
    let (nav, by_account) = rows::nav(c).unwrap();
    assert_eq!((&*through_json.nav, &*through_json.nav_by_account), (&nav, &by_account));
    assert_eq!(*through_json.exposures, rows::exposures(c).unwrap());
    assert_eq!(*through_json.watchlist, rows::watchlist(c).unwrap());
    assert_eq!(*through_json.news, rows::news(c).unwrap());
    assert_eq!(*through_json.universes, rows::universes(c).unwrap());
    assert_eq!(*through_json.distributions, rows::distributions(c).unwrap());
    assert_eq!(*through_json.quotes, rows::quotes(c).unwrap());
    assert_eq!(*through_json.fx, rows::fx(c, tables::FX_PAIR).unwrap());
    assert_eq!(*through_json.benchmark, rows::benchmark(c, tables::BENCHMARK_SYMBOL).unwrap());
    for sym in market::BENCHMARK_SYMBOLS {
        assert_eq!(through_json.benchmarks[sym], rows::benchmark(c, sym).unwrap());
    }
    assert_eq!(*through_json.groups, rows::groups(c).unwrap());
    assert_eq!(*through_json.journal, rows::journal(c).unwrap());
    assert_eq!(*through_json.tiles, rows::tiles(c).unwrap());

    // the rows are there to compare: none of the readings is empty
    assert_eq!((nav.len(), by_account.len(), rows::universes(c).unwrap().0.len()), (2, 2, 2));
    assert_eq!(rows::exposures(c).unwrap()["sec-1"].sectors, vec![("Tech".to_string(), 0.6), ("Energy".to_string(), 0.4)]);
    assert_eq!(rows::news(c).unwrap().iter().find(|n| n.id == "n2").unwrap().kind, "story");
    assert_eq!(rows::margin(c).unwrap()[1].currency, "CAD");
    assert_eq!(rows::groups(c).unwrap().len(), 1);
    assert_eq!(rows::tiles(c).unwrap().unwrap().len(), 1);

    // never saved is not saved empty
    tables::set_meta(c, snapshot::TILES_META, "").unwrap();
    assert_eq!(rows::tiles(c).unwrap(), None);
    assert_eq!(*Inputs::from_snapshot(&snapshot::snapshot(c, false).unwrap(), &serde_json::json!({}), &Default::default()).tiles, None);
}
