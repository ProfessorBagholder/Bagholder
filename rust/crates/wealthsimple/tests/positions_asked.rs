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
