//! What every store test starts from: a fresh database in a temporary home,
//! through `ensure`.
#![allow(dead_code)]

use rusqlite::Connection;
use serde_json::{json, Value};
use std::cell::Cell;
use std::path::PathBuf;

pub struct Db {
    pub dir: tempfile::TempDir,
    pub conn: Connection,
    n: Cell<u64>,
}

impl Db {
    pub fn path(&self) -> PathBuf {
        self.dir.path().join("bagholder.db")
    }
    /// A fresh id each call, shaped like a uuid.
    pub fn new_id(&self) -> impl Fn() -> String + '_ {
        move || {
            let n = self.n.get() + 1;
            self.n.set(n);
            format!("00000000-0000-4000-8000-{:012}", n)
        }
    }
    pub fn reopen(&mut self) {
        self.conn = Connection::open(self.path()).unwrap();
    }
    pub fn ensure(&self) {
        bagholder_store::relabel::ensure(&self.conn).unwrap();
    }
    pub fn snapshot(&self) -> Value {
        bagholder_store::snapshot::snapshot(&self.conn, true).unwrap()
    }
    pub fn activities(&self) -> Vec<Value> {
        self.snapshot()["activities"].as_array().unwrap().clone()
    }
    pub fn apply(&self, rows: &[Value]) -> bagholder_store::merge::Applied {
        let id = self.new_id();
        let rows = typed_rows::<bagholder_store::activities::ActivityRow>(rows);
        bagholder_store::merge::apply_wealthsimple_mapped(&self.conn, &rows, &id).unwrap()
    }
    pub fn insert_local(&self, row: Value) -> Value {
        let id = self.new_id();
        let row: bagholder_store::activities::ActivityRow = typed(row);
        serde_json::to_value(bagholder_store::activities::insert_local(&self.conn, &row, &id).unwrap()).unwrap()
    }
    pub fn count(&self) -> i64 {
        bagholder_store::activities::activity_count(&self.conn).unwrap()
    }
}

/// An empty home, the database not yet created.
pub fn bare() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let conn = Connection::open(dir.path().join("bagholder.db")).unwrap();
    Db { dir, conn, n: Cell::new(0) }
}

pub fn db() -> Db {
    let d = bare();
    d.ensure();
    d
}

pub fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{} != {}", a, b);
}

pub fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {}", v))
}

/// A Wealthsimple trade as the ws crate's mapper writes it.
pub fn ws_row() -> Value {
    json!({"canonicalId": "ws-cid-aaa-001", "occurredAt": "2024-06-15T13:45:22.123Z", "transactionDate": "2024-06-15", "settlementDate": "2024-06-15", "accountId": "acct-1", "bookId": "acct-1", "fifoId": "acct-1", "accountType": "", "activityType": "Trade", "activitySubType": "BUY", "description": "Buy 10 AAA @ 10", "direction": "DEBIT", "symbol": "AAA", "name": "AAA", "currency": "CAD", "quantity": 10.0, "unitPrice": 10.0, "commission": 0.0, "netCashAmount": -100.0, "category": "trade", "balance": null, "source": "wealthsimple", "rawType": "DIY_BUY", "aftType": "", "counterSymbol": "", "securityId": null})
}

/// A `json!` literal read as the typed row a writer now takes; every writer's
/// row type is lenient, so this never fails on a shape a test itself wrote.
pub fn typed<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

/// As `typed`, for a list of rows.
pub fn typed_rows<T: serde::de::DeserializeOwned>(rows: &[Value]) -> Vec<T> {
    rows.iter().map(|v| typed(v.clone())).collect()
}

pub fn with(mut row: Value, over: Value) -> Value {
    for (k, v) in over.as_object().unwrap() {
        row[k] = v.clone();
    }
    row
}
