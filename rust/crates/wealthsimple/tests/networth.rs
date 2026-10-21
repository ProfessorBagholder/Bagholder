//! An outside account's balance by day: asked of Wealthsimple's net worth feature
//! over that account alone, a range as its own page asks one, and read from the
//! recorded reply (made-up figures) as that account's balance is stated.

use std::sync::{Arc, Mutex};

use bagholder_core::json::{self, Value};
use bagholder_core::{Currency, Dec};
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

fn day(s: &str) -> jiff::civil::Date {
    s.parse().unwrap()
}

fn reply() -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple-networth/FetchIdentityNetWorthFinancials-1.json")).unwrap()
}

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    match v {
        Value::Object(m) => m.get(key).unwrap_or_else(|| panic!("no {key}")),
        _ => panic!("not an object"),
    }
}

#[test]
fn an_outside_accounts_days_are_asked_over_it_alone_for_the_range_given() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let net = Net::answered_by(Arc::new(ManualClock::at("2026-10-20T12:00:00Z".parse().unwrap())), Arc::new(Limiter::new()), Box::new(Recording(seen.clone(), reply())));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("session.json"), r#"{"access_token":"a1","refresh_token":"r1","client_id":"c","identity_canonical_id":"identity-1","expires_at":"2026-10-20T13:00:00Z","wssdi":"device-1"}"#).unwrap();
    let mut client = bagholder_wealthsimple::client::Client::new(&net, bagholder_wealthsimple::session::SessionFile { path: dir.path().join("session.json") });
    let nodes = client.net_worth_history(&[], &["loc-1".to_string()], day("2026-09-20"), day("2026-10-20")).unwrap().expect("read");
    assert_eq!(nodes.len(), 5);
    let asked = seen.lock().unwrap();
    assert_eq!(asked.len(), 1);
    let body = json::parse(&asked[0]).unwrap();
    assert_eq!(field(&body, "operationName"), &Value::String("FetchIdentityNetWorthFinancials".into()));
    let vars = field(&body, "variables");
    let text = |k: &str| match field(vars, k) {
        Value::String(s) => s.clone(),
        other => panic!("{k}: {other:?}"),
    };
    // the range as the page asks one: its first and last days, every day of it
    assert_eq!((text("startDate"), text("endDate")), ("2026-09-20".to_string(), "2026-10-20".to_string()));
    assert_eq!(field(vars, "first"), &json::parse("31").unwrap());
    assert_eq!(field(vars, "externalFinancialEntityIds"), &json::parse(r#"["loc-1"]"#).unwrap());
    assert_eq!(field(vars, "accountIds"), &json::parse("[]").unwrap(), "no account of Wealthsimple's own");
    assert_eq!((text("currency"), text("accountScope")), ("CAD".to_string(), "OWN".to_string()));

    // the reply read as the line of credit's balance by day: owed positive
    let loc = bagholder_book::debts::OutsideDebt { entity_id: "loc-1".into(), kind: "LineOfCredit".into(), name: "Line of credit".into(), institution: "Bank".into(), last4: None, balance: Dec::parse("1100.25").unwrap(), currency: Currency::CAD, as_of: None };
    let days = bagholder_wealthsimple::read::outside_days(&nodes, &loc).unwrap();
    let got: Vec<(String, String)> = days.iter().map(|d| (d.day.to_string(), d.amount.round(2, bagholder_core::dec::Rounding::HalfEven).to_string())).collect();
    assert_eq!(got, [("2026-10-13", "1200"), ("2026-10-14", "1200"), ("2026-10-15", "1150.5"), ("2026-10-16", "1150.5"), ("2026-10-19", "1100.25")].map(|(a, b)| (a.to_string(), b.to_string())));
}
