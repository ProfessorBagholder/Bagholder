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
    /// The match as it comes: native amounts only. What a closed slice is worth
    /// in CAD depends on the FX table, which is not the book's to read.
    pub fifo: Matched,
    /// The last fill price of each symbol, what an open position is marked at
    /// until a quote says otherwise.
    pub last_prices: serde_json::Map<String, Value>,
    pub raw_count: usize,
}

pub fn build_book(raw_acts: &[Value], sec_rows: &[Value], today: &str) -> Book {
    let mut acts = normalize_activities(raw_acts);
    let securities = Securities::new(sec_rows);

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
    let last_prices = crate::trades::last_fill_prices(&acts);
    Book { activities: acts, acts_by_id, securities, fifo, last_prices, raw_count: raw_acts.len() }
}
