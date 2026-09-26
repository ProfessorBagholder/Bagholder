//! The stand-in Wealthsimple server (`bagholder_ws::standin`), and a helper
//! the client's tests share.
#![allow(dead_code)]

pub use bagholder_ws::standin::*;
use serde_json::Value;

pub fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {}", v))
}
