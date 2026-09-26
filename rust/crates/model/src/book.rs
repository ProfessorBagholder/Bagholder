//! `build_book`: the matched book.
//!
//! Every activity normalized, with the shares an assignment delivered and the
//! expiries the broker never posted added, then the FIFO match. It depends on
//! the activity rows, the securities and the day only, so a quote tick can
//! reuse it.

use std::collections::HashMap;

use crate::activity::{Activity, RawActivity};
use crate::fifo::{match_fifo_in_place, Matched};
use crate::input::LastFill;
use crate::normalize::normalize_all;
use crate::securities::Securities;
use crate::synth::{synthesize_assignment_shares, synthesize_expiries};

pub struct Book {
    /// The working rows: the stored ones normalized, the derived ones added, and
    /// what the match inferred about each written onto it.
    pub activities: Vec<Activity>,
    by_id: HashMap<String, usize>,
    pub securities: Securities,
    /// The match as it comes: native amounts only. What a closed slice is worth
    /// in CAD depends on the FX table, which is not the book's to read.
    pub fifo: Matched,
    /// The last fill price of each symbol, what an open position is marked at
    /// until a quote says otherwise.
    pub last_prices: HashMap<String, LastFill>,
    pub raw_count: usize,
}

impl Book {
    /// A working row by its id.
    pub fn activity(&self, id: &str) -> Option<&Activity> {
        self.by_id.get(id).map(|i| &self.activities[*i])
    }
}

pub fn build_book(rows: &[RawActivity], securities: Securities, today: &str) -> Book {
    let mut acts = normalize_all(rows);
    acts.extend(synthesize_assignment_shares(&acts, &|security_id| securities.underlying_id(security_id)));
    let mut fifo = match_fifo_in_place(&mut acts);

    let expired = synthesize_expiries(&fifo.open, today);
    if !expired.is_empty() {
        acts.extend(expired);
        fifo = match_fifo_in_place(&mut acts);
    }
    Book::of(acts, fifo, securities, rows.len())
}

impl Book {
    /// The book of working rows already matched.
    pub fn of(activities: Vec<Activity>, fifo: Matched, securities: Securities, raw_count: usize) -> Book {
        // a later row with the same id stands for it
        let by_id = activities.iter().enumerate().map(|(i, a)| (a.id.clone(), i)).collect();
        let last_prices = crate::trades::last_fill_prices(&activities);
        Book { activities, by_id, securities, fifo, last_prices, raw_count }
    }
}
