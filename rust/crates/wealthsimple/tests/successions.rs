//! An id Wealthsimple retires by a corporate action, the listing trading on
//! under a new id with no event row (`docs/architecture.md` §5, "Matching across
//! sources"): Wealthsimple's `status` of the old security says it, and the two
//! ids are one instrument. The replies are one account's November from the
//! owner's history (`tests/replies/wealthsimple-pull`), with a stock bought under
//! its old id and sold under its new one, shaped as Wealthsimple sends them.

use std::path::{Path, PathBuf};

use bagholder_book::Book;
use bagholder_broker::pull::pull;
use bagholder_core::account::AccountRef;
use bagholder_core::instrument::{RefScheme, Reference};
use bagholder_core::json::{self, Value};
use bagholder_core::{Broker, Dec, InstrumentId};
use bagholder_wealthsimple::adapter::Wealthsimple;
use bagholder_wealthsimple::replay::Replay;

const OLD: &str = "sec-s-5e0a1d2b3c4f4a6b8c9d0e1f2a3b4c5d";
const NEW: &str = "sec-s-9f8e7d6c5b4a4f3e8d2c1b0a9f8e7d6c";

fn at(s: &str) -> jiff::Timestamp {
    s.parse().unwrap()
}

fn replies() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple-pull")
}

fn parse(text: &str) -> Value {
    json::parse(text).unwrap()
}

fn obj<'a>(v: &'a mut Value, key: &str) -> &'a mut Value {
    let Value::Object(m) = v else { panic!("not an object at {key}") };
    m.get_mut(key).unwrap_or_else(|| panic!("no {key}"))
}

fn list(v: &mut Value) -> &mut Vec<Value> {
    let Value::Array(a) = v else { panic!("not a list") };
    a
}

fn set(v: &mut Value, key: &str, to: &str) {
    let Value::Object(m) = v else { panic!("not an object") };
    m.insert(key.to_string(), Value::String(to.to_string()));
}

fn edit_file(dir: &Path, name: &str, f: impl FnOnce(&mut Value)) {
    let path = dir.join(name);
    let mut v = parse(&std::fs::read_to_string(&path).unwrap());
    f(&mut v);
    std::fs::write(&path, v.canonical()).unwrap();
}

fn security(id: &str, name: &str, status: &str) -> Value {
    parse(&format!(
        r#"{{"currency": "CAD", "id": "{id}", "optionDetails": null, "securityType": "EQUITY", "status": "{status}",
            "stock": {{"name": "{name}", "primaryExchange": "TSX-V", "primaryMic": "XTSX", "symbol": "CH"}}}}"#
    ))
}

/// The November replies, and `CH`: 1,000 bought under the old id on the 3rd,
/// 400 sold under the new one on the 17th, 600 held under the new one on the
/// 18th. The old security's status is `old_status`.
fn capture(old_status: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for e in std::fs::read_dir(replies()).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "json") {
            std::fs::copy(&p, dir.path().join(p.file_name().unwrap())).unwrap();
        }
    }
    edit_file(dir.path(), "edited-securities.json", |v| {
        let all = list(obj(obj(v, "data"), "securities"));
        all.push(security(OLD, "Charbone Hydrogen Corp", old_status));
        all.push(security(NEW, "Charbone Corp.", "TRADING"));
    });
    edit_file(dir.path(), "edited-activity-november.json", |v| {
        let edges = list(obj(obj(obj(v, "data"), "activityFeedItems"), "edges"));
        let buy = edges.iter().find(|e| matches!(&e, Value::Object(m) if matches!(m.get("node"), Some(Value::Object(n)) if n.get("type") == Some(&Value::String("DIY_BUY".into()))))).unwrap().clone();
        let row = |key: &str, ty: &str, id: &str, qty: &str, amount: &str, when: &str| {
            let mut e = buy.clone();
            let n = obj(&mut e, "node");
            for (k, to) in [("canonicalId", key), ("externalCanonicalId", key), ("type", ty), ("securityId", id), ("assetQuantity", qty), ("amount", amount), ("assetSymbol", "CH"), ("occurredAt", when)] {
                set(n, k, to);
            }
            set(obj(n, "security"), "id", id);
            e
        };
        edges.push(row("anon-order-ch-1", "DIY_BUY", OLD, "1000.0000", "150.00", "2025-11-03T15:00:00.000000+00:00"));
        edges.push(row("anon-order-ch-2", "DIY_SELL", NEW, "400.0000", "72.00", "2025-11-17T15:00:00.000000+00:00"));
    });
    edit_file(dir.path(), "positions@anon-tfsa-1@2025-11-18.json", |v| {
        let accounts = list(obj(obj(v, "data"), "accounts"));
        let edges = list(obj(obj(obj(obj(&mut accounts[0], "financials"), "current"), "positionsAsOfDate"), "edges"));
        edges.push(parse(&format!(r#"{{"node": {{"direction": "LONG", "quantity": "600", "security": {{"id": "{NEW}"}}}}}}"#)));
    });
    dir
}

struct Pulled {
    _home: tempfile::TempDir,
    book: Book,
    old: InstrumentId,
    new: InstrumentId,
    account: bagholder_core::AccountId,
}

fn pulled(old_status: &str) -> Pulled {
    let replies = capture(old_status);
    let home = tempfile::tempdir().unwrap();
    let now = at("2025-11-19T20:00:00Z");
    let (book, _) = Book::open_in(home.path(), "test", now).unwrap();
    let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", now).unwrap();
    let mut ws = Wealthsimple::new(Replay::read(replies.path()).unwrap().taken_before_statements());
    let report = pull(&book, &mut ws, connection, "2025-11-19".parse().unwrap(), now, &mut |_| {}).unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    let by = |id: &str| book.instrument_by_ref(&Reference::new(RefScheme::BrokerSecurity(Broker::named("wealthsimple")), id)).unwrap().unwrap();
    let (old, new) = (by(OLD), by(NEW));
    let account = book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1")).unwrap().unwrap();
    Pulled { _home: home, book, old, new, account }
}

fn held(p: &Pulled, i: InstrumentId) -> Option<Dec> {
    bagholder_book::records::positions(&p.book.transactions().unwrap()).unwrap().get(&(p.account, i)).copied()
}

fn stated(p: &Pulled, i: InstrumentId) -> Option<Dec> {
    p.book.stated(p.account).unwrap().units.expect("the units stated").1.get(&i).copied()
}

#[test]
fn an_id_retired_by_a_corporate_action_and_the_listing_s_new_id_are_one_holding() {
    let p = pulled("CORPORATE_ACTION");
    assert_eq!(p.old, p.new, "one instrument under both ids");
    // the sale under the new id comes off what was bought under the old
    assert_eq!(held(&p, p.old), Some(Dec::parse("600").unwrap()));
    // and what the book holds is what Wealthsimple states it holds
    assert_eq!(stated(&p, p.old), held(&p, p.old));
    let names = p.book.names(p.old).unwrap();
    assert_eq!(names.iter().map(|n| (n.symbol.as_str(), n.venue_mic.as_deref())).collect::<Vec<_>>(), vec![("CH", Some("XTSX"))]);
    // a pull that finds nothing new leaves it so
    let problems: Vec<_> = p.book.problems().unwrap().into_iter().filter(|(_, pr)| pr.detail.contains(OLD) || pr.detail.contains(NEW)).collect();
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn the_same_symbol_venue_and_currency_without_the_retirement_stated_stay_two() {
    // a ticker can be another company's: only Wealthsimple's word that the old
    // id was retired by a corporate action joins them
    let p = pulled("TRADING");
    assert_ne!(p.old, p.new);
    assert_eq!(held(&p, p.old), Some(Dec::parse("1000").unwrap()));
    assert_eq!(held(&p, p.new), Some(Dec::parse("-400").unwrap()));
    assert_eq!(stated(&p, p.new), Some(Dec::parse("600").unwrap()));
    assert_eq!(stated(&p, p.old), None);
}
