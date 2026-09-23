//! What the book's tests share: a book in a temporary folder, and a mapping whose
//! payload spells out its legs, so each test states exactly what a source said.
#![allow(dead_code)]

use bagholder_book::mapping::{Draft, InstrumentDraft, MapContext, Mapped, Mapping, NameDraft, OptionDraft};
use bagholder_book::records::Incoming;
use bagholder_book::Book;
use bagholder_core::account::{AccountRef, AccountStatus, AccountType};
use bagholder_core::instrument::{InstrumentKind, OptionRight, RefScheme, Reference};
use bagholder_core::record::Problem;
use bagholder_core::transaction::{Effect, Kind};
use bagholder_core::{AccountId, Broker, ConnectionId, Currency, Dec, Leg, Money, SourceName};
use serde_json::{json, Value};

pub fn at(s: &str) -> jiff::Timestamp {
    s.parse().unwrap()
}

pub fn t0() -> jiff::Timestamp {
    at("2026-09-23T12:00:00Z")
}

pub fn d(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

pub fn cad(s: &str) -> Money {
    Money::new(d(s), Currency::CAD)
}

pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub book: Book,
    pub connection: ConnectionId,
}

impl Fixture {
    pub fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let (book, _) = Book::open_in(dir.path(), "test", t0()).unwrap();
        let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", t0()).unwrap();
        Fixture { dir, book, connection }
    }

    /// An account known by the broker ids `refs`.
    /// The opening a transaction is: it and the instrument it moves.
    pub fn opening_of(&self, t: bagholder_core::TransactionId) -> bagholder_core::journal::Opening {
        let instrument = self.book.transaction(&t).unwrap().and_then(|x| x.instrument).expect("a transaction that moves an instrument");
        bagholder_core::journal::Opening { transaction: t, instrument }
    }

    /// The opening a record's `trade` leg is.
    pub fn opens(&self, r: bagholder_core::RecordId) -> bagholder_core::journal::Opening {
        self.opening_of(bagholder_core::TransactionId::new(r, bagholder_core::Leg::named("trade")))
    }

    pub fn account(&self, refs: &[&str]) -> AccountId {
        let refs: Vec<AccountRef> = refs.iter().map(|r| AccountRef::new(Broker::named("wealthsimple"), *r)).collect();
        let t = AccountType::Known { kind: bagholder_core::account::AccountKind::Cash, registration: bagholder_core::account::Registration::Tfsa, managed: false, joint: false };
        self.book.add_account(self.connection, &refs, &t, AccountStatus::Open, Some("Trading"), t0()).unwrap()
    }

    pub fn incoming<'a>(&self, key: &'a str, payload: &'a str) -> Incoming<'a> {
        Incoming { connection: Some(self.connection), source_key: key, payload, refs: vec![] }
    }

    pub fn store(&self, m: &Spelled, key: &str, payload: &Value) -> bagholder_book::records::Stored {
        let text = payload.to_string();
        self.book.store(m, &self.incoming(key, &text), t0()).unwrap()
    }
}

/// The mapping the tests use: the payload is `{"legs": [...], "problems": [...]}`
/// and each leg says what it is. `version` is what the test needs it to be.
pub struct Spelled {
    pub source: &'static str,
    pub version: u32,
}

impl Spelled {
    pub fn v(version: u32) -> Spelled {
        Spelled { source: "test-broker", version }
    }
}

fn instrument(v: &Value) -> InstrumentDraft {
    let refs = v["refs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| Reference::new(RefScheme::parse(r[0].as_str().unwrap()).unwrap(), r[1].as_str().unwrap()))
        .collect();
    InstrumentDraft {
        refs,
        kind: InstrumentKind::parse(v["kind"].as_str().unwrap_or("security")).unwrap(),
        currency: Currency::parse(v["currency"].as_str().unwrap_or("CAD")).unwrap(),
        name: v.get("symbol").map(|s| NameDraft {
            symbol: s.as_str().unwrap().to_string(),
            venue_mic: v.get("mic").and_then(Value::as_str).map(str::to_string),
            venue_name: None,
            name: v.get("name").and_then(Value::as_str).map(str::to_string),
            seen: v["seen"].as_str().unwrap_or("2026-01-02").parse().unwrap(),
        }),
        option: v.get("option").map(|o| OptionDraft {
            underlying: Box::new(instrument(&o["underlying"])),
            expiry: o["expiry"].as_str().unwrap().parse().unwrap(),
            strike: d(o["strike"].as_str().unwrap()),
            right: OptionRight::parse(o["right"].as_str().unwrap()).unwrap(),
            multiplier: o.get("multiplier").and_then(Value::as_str).map(d),
        }),
    }
}

impl Mapping for Spelled {
    fn source(&self) -> SourceName {
        SourceName::named(self.source)
    }

    fn version(&self) -> u32 {
        self.version
    }

    fn map(&self, _ctx: &MapContext, payload: &str) -> Mapped {
        let v: Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(e) => return Mapped::unreadable(e.to_string()),
        };
        let Some(legs) = v.get("legs").and_then(Value::as_array) else { return Mapped::unreadable("no legs") };
        let money = |l: &Value, f: &str| l.get(f).and_then(Value::as_str).map(|a| Money::new(d(a), Currency::parse(l.get("currency").and_then(Value::as_str).unwrap_or("CAD")).unwrap()));
        let legs = legs
            .iter()
            .map(|l| Draft {
                leg: Leg::parse(l["leg"].as_str().unwrap_or("trade")).unwrap(),
                account: AccountRef::new(Broker::named("wealthsimple"), l["account"].as_str().unwrap()),
                occurred_at: l.get("at").and_then(Value::as_str).map(|s| s.parse().unwrap()),
                trade_date: l["date"].as_str().unwrap_or("2026-01-02").parse().unwrap(),
                settle_date: None,
                kind: Kind::parse(l["kind"].as_str().unwrap()).unwrap(),
                effect: l.get("effect").and_then(Value::as_str).map(|e| Effect::parse(e).unwrap()),
                instrument: l.get("instrument").map(instrument),
                quantity: l.get("quantity").and_then(Value::as_str).map(d),
                price: money(l, "price"),
                cash: money(l, "cash"),
                fee: money(l, "fee"),
                fx_rate: None,
            })
            .collect();
        let problems = v.get("problems").and_then(Value::as_array).map(|ps| ps.iter().map(|p| Problem::new(p.as_str().unwrap(), "stated by the test")).collect()).unwrap_or_default();
        // `{"adjustments": [{"leg", "applies_to": "<record>/<leg>", "legs": [{"from": [[scheme, value]], "to": …, …}]}]}`
        let refs = |v: Option<&Value>| -> Option<Vec<Reference>> {
            v.and_then(Value::as_array).map(|rs| rs.iter().map(|r| Reference::new(RefScheme::parse(r[0].as_str().unwrap()).unwrap(), r[1].as_str().unwrap())).collect())
        };
        let adjustments = v
            .get("adjustments")
            .and_then(Value::as_array)
            .map(|adjs| {
                adjs.iter()
                    .map(|a| bagholder_book::mapping::AdjustmentDraft {
                        leg: Leg::parse(a["leg"].as_str().unwrap_or("adjustment")).unwrap(),
                        applies_to: bagholder_core::TransactionId::parse(a["applies_to"].as_str().unwrap()).unwrap(),
                        legs: a["legs"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|l| bagholder_book::mapping::AdjustmentLegDraft {
                                from: refs(l.get("from")),
                                to: refs(l.get("to")),
                                units_per_unit: l.get("units_per_unit").and_then(Value::as_str).map(d),
                                cost_share: l.get("cost_share").and_then(Value::as_str).map(d),
                                cash_per_unit: money(l, "cash_per_unit"),
                                cost: money(l, "cost"),
                                acquired: l.get("acquired").and_then(Value::as_str).map(|s| s.parse().unwrap()),
                            })
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Mapped { legs, problems, adjustments }
    }
}

/// A share by its ISIN.
pub fn share(isin: &str, symbol: &str) -> Value {
    json!({"refs": [["isin", isin]], "kind": "security", "currency": "CAD", "symbol": symbol})
}

/// A buy of `qty` of `instrument` in `account` at `when`.
pub fn buy(account: &str, instrument: Value, qty: &str, cash: &str, when: &str) -> Value {
    json!({"leg": "trade", "account": account, "kind": "buy", "instrument": instrument, "quantity": qty, "cash": cash, "at": when, "date": &when[..10]})
}

pub fn sell(account: &str, instrument: Value, qty: &str, cash: &str, when: &str) -> Value {
    json!({"leg": "trade", "account": account, "kind": "sell", "instrument": instrument, "quantity": qty, "cash": cash, "at": when, "date": &when[..10]})
}

pub fn legs(legs: Vec<Value>) -> Value {
    json!({ "legs": legs })
}

/// Everything the book holds, as text: a test compares two of these to show a
/// write changed nothing.
pub fn everything(dir: &std::path::Path) -> String {
    let conn = rusqlite::Connection::open(dir.join("book.db")).unwrap();
    let mut out = String::new();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for t in tables {
        let mut stmt = conn.prepare(&format!("SELECT * FROM {t} ORDER BY 1, 2")).unwrap();
        let n = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            let cells: Vec<String> = (0..n).map(|i| format!("{:?}", r.get_ref(i).unwrap())).collect();
            out.push_str(&format!("{t}: {}\n", cells.join(" | ")));
        }
    }
    out
}
