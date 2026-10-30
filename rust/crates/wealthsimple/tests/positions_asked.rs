//! The positions read is asked as Wealthsimple's own web app asks it:
//! `currency: "CAD"` with `currencyOverride: "MARKET"` (its release 0.3.720893,
//! chunk 652), so each position's `marketBookValue` is in the listing's own
//! currency (`SPEC.md` position-6, market-data-35).

use std::sync::{Arc, Mutex};

use bagholder_core::json::{self, Value};
use bagholder_net::{Ask, Limiter, ManualClock, Net, NetError, Transport};
use bagholder_wealthsimple::adapter::Source;

/// Answers every request with one reply and keeps each request's body.
struct Recording(Arc<Mutex<Vec<String>>>, String);

impl Transport for Recording {
    fn answer(&self, ask: &Ask) -> Result<bagholder_net::Answer, NetError> {
        self.0.lock().unwrap().push(String::from_utf8(ask.body.unwrap_or_default().to_vec()).unwrap());
        Ok((200, ask.url.to_string(), vec![], self.1.clone().into_bytes()))
    }
}

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    match v {
        Value::Object(m) => m.get(key).unwrap_or_else(|| panic!("no {key}")),
        _ => panic!("not an object"),
    }
}

#[test]
fn the_positions_read_is_asked_in_cad_with_the_market_override() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let reply = r#"{"data":{"accounts":[{"id":"a-1","financials":{"current":{"id":"c-1","positionsAsOfDate":{"edges":[]}}}}]}}"#;
    let net = Net::answered_by(Arc::new(ManualClock::at("2026-10-20T12:00:00Z".parse().unwrap())), Arc::new(Limiter::new()), Box::new(Recording(seen.clone(), reply.to_string())));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("session.json"), r#"{"access_token":"a1","refresh_token":"r1","client_id":"c","identity_canonical_id":"identity-1","expires_at":"2026-10-20T13:00:00Z","wssdi":"device-1"}"#).unwrap();
    let mut client = bagholder_wealthsimple::client::Client::new(&net, bagholder_wealthsimple::session::SessionFile { path: dir.path().join("session.json") });
    client.positions_of(&["a-1".to_string()], "2026-10-19".parse().unwrap()).unwrap();
    let asked = seen.lock().unwrap();
    assert_eq!(asked.len(), 1);
    let body = json::parse(&asked[0]).unwrap();
    assert_eq!(field(&body, "operationName"), &Value::String("FetchHoldingsExportPositionsAsOfDate".into()));
    let vars = field(&body, "variables");
    assert_eq!(field(vars, "currency"), &Value::String("CAD".into()));
    assert_eq!(field(vars, "currencyOverride"), &Value::String("MARKET".into()));
    assert_eq!(field(vars, "asOf"), &Value::String("2026-10-19".into()));
}

/// Today's holdings in the shape of `FetchHoldingsExportPositions`, built from a
/// recorded past-date reply's nodes (no reply of that document is recorded yet):
/// each node names its one account and its direction as `positionDirection`, as
/// Wealthsimple's web app reads them (release 0.3.726566, read 2026-10-30).
fn today_reply(nodes: &[Value], account: &str) -> String {
    let edges: Vec<Value> = nodes
        .iter()
        .map(|n| {
            let Value::Object(mut f) = n.clone() else { panic!() };
            let direction = f.remove("direction").unwrap();
            f.remove("closePrice");
            f.insert("positionDirection".into(), direction);
            f.insert("accounts".into(), Value::Array(vec![Value::Object([("id".to_string(), Value::String(account.into()))].into())]));
            Value::Object([("node".to_string(), Value::Object(f))].into())
        })
        .collect();
    let page = r#"{"hasNextPage":false,"endCursor":null}"#;
    format!(r#"{{"data":{{"identity":{{"id":"identity-1","financials":{{"current":{{"id":"c-1","positions":{{"edges":{},"pageInfo":{page},"totalCount":{}}}}}}}}}}}}}"#, Value::Array(edges).canonical(), nodes.len())
}

#[test]
fn today_s_holdings_are_asked_as_wealthsimple_s_web_app_asks_them_and_read_as_a_past_day_s() {
    // a recorded past day's positions: cash, a long and a short
    let recorded = json::parse(&std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple-pull/positions@anon-tfsa-1@2025-11-18.json")).unwrap()).unwrap();
    let nodes: Vec<Value> = bagholder_sources::reply::Node::root(&recorded).obj("data").unwrap().list("accounts").unwrap()[0].obj("financials").unwrap().obj("current").unwrap().obj("positionsAsOfDate").unwrap().list("edges").unwrap().iter().map(|e| e.obj("node").unwrap().value().clone()).collect();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let net = Net::answered_by(Arc::new(ManualClock::at("2026-10-30T14:06:00Z".parse().unwrap())), Arc::new(Limiter::new()), Box::new(Recording(seen.clone(), today_reply(&nodes, "a-1"))));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("session.json"), r#"{"access_token":"a1","refresh_token":"r1","client_id":"c","identity_canonical_id":"identity-1","expires_at":"2026-10-30T15:00:00Z","wssdi":"device-1"}"#).unwrap();
    let mut client = bagholder_wealthsimple::client::Client::new(&net, bagholder_wealthsimple::session::SessionFile { path: dir.path().join("session.json") });
    let today = client.positions_now(&["a-1".to_string(), "a-2".to_string()], "2026-10-30".parse().unwrap()).unwrap();
    // one request, never the past-date document, which Wealthsimple refuses for today
    let asked = seen.lock().unwrap();
    assert_eq!(asked.len(), 1);
    let body = json::parse(&asked[0]).unwrap();
    assert_eq!(field(&body, "operationName"), &Value::String("FetchHoldingsExportPositions".into()));
    let vars = field(&body, "variables");
    assert_eq!(field(vars, "identityId"), &Value::String("identity-1".into()));
    assert_eq!(field(vars, "currency"), &Value::String("CAD".into()));
    assert_eq!(field(vars, "currencyOverride"), &Value::String("MARKET".into()));
    assert_eq!(field(vars, "aggregated"), &Value::Bool(false));
    assert_eq!(field(vars, "accountIds"), &Value::Array(vec![Value::String("a-1".into()), Value::String("a-2".into())]));
    assert_eq!(field(vars, "first"), &Value::Number("500".into()));
    assert_eq!(field(field(vars, "filter"), "includeCash"), &Value::Bool(true));
    assert_eq!(field(field(vars, "filter"), "includeManaged"), &Value::Bool(true));
    // each account's nodes read exactly as the past day's are; an account holding nothing is stated empty
    let past = Value::Array(nodes);
    use bagholder_wealthsimple::read::{cash_held, units};
    assert_eq!(units(&today["a-1"]).unwrap(), units(&past).unwrap());
    assert_eq!(cash_held(&today["a-1"]).unwrap(), cash_held(&past).unwrap());
    assert!(!units(&today["a-1"]).unwrap().is_empty());
    assert_eq!(today["a-2"], Value::Array(vec![]));
}
