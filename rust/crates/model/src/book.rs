//! `build_book`: the matched book.
//!
//! Every activity normalized, with the shares an assignment delivered and the
//! expiries the broker never posted added, then the FIFO match. It depends on
//! the activity rows, the securities and the day only, so a quote tick can
//! reuse it.

use serde_json::Value;
use std::collections::HashMap;

use crate::fifo::{match_fifo_in_place, Matched};
use crate::normalize::normalize_activities;
use crate::securities::Securities;
use crate::synth::{synthesize_assignment_shares, synthesize_expiries};
use crate::value::field_s;

pub struct Book {
    pub activities: Vec<Value>,
    pub acts_by_id: HashMap<String, Value>,
    pub securities: Securities,
    pub fifo: Matched,
    pub raw_count: usize,
}

pub fn build_book(snapshot: &Value, today: &str) -> Book {
    let raw_acts: Vec<Value> = snapshot
        .get("activities")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut acts = normalize_activities(&raw_acts);

    let sec_rows: Vec<Value> = snapshot
        .get("securities")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let securities = Securities::new(&sec_rows);

    let delivered = synthesize_assignment_shares(&acts, &|sid| securities.underlying_id(sid));
    if !delivered.is_empty() {
        acts.extend(delivered);
    }
    let mut fifo = match_fifo_in_place(&mut acts);

    let synthetic = synthesize_expiries(&fifo.open, today);
    if !synthetic.is_empty() {
        acts.extend(synthetic);
        fifo = match_fifo_in_place(&mut acts);
    }

    let mut acts_by_id = HashMap::new();
    for a in &acts {
        acts_by_id.insert(field_s(a, "id"), a.clone());
    }
    Book { activities: acts, acts_by_id, securities, fifo, raw_count: raw_acts.len() }
}
