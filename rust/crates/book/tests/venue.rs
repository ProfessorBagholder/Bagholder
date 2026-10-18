//! The venue an instrument is on (brief 43): the last one a record stated, as of
//! its own day; a record that states none says nothing about it, whichever writer
//! made it (the broker's pull, an import, a fill of the app's own order, a trade
//! entered by hand), and whatever day it is dated.

mod common;

use bagholder_book::orders::OrderRequest;
use bagholder_book::person::{Entry, Side as EntrySide, Traded};
use bagholder_core::listing::venue_of;
use bagholder_core::order::{Asker, BrokerStatus, OrderEvent, OrderKind, Reading, Side, TimeInForce};
use bagholder_core::{Currency, Money};
use common::*;
use serde_json::json;

/// A small generator, so a failing history is the same on every run.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

/// Venues as a record can state them: a code and its words, either alone or both.
const VENUES: [(&str, &str); 5] = [("XTSE", "TSX"), ("XTSX", "TSX-V"), ("XNAS", "NASDAQ"), ("XNYS", "NYSE"), ("XCNQ", "CSE")];

fn stated(rng: &mut Rng) -> (Option<String>, Option<String>) {
    let (mic, words) = VENUES[rng.below(VENUES.len() as u64) as usize];
    match rng.below(3) {
        0 => (Some(mic.into()), None),
        1 => (None, Some(words.into())),
        _ => (Some(mic.into()), Some(words.into())),
    }
}

fn share(symbol: &str, seen: &str, venue: &(Option<String>, Option<String>)) -> serde_json::Value {
    let mut v = json!({"refs": [["isin", "CA0000000009"], ["broker-security:wealthsimple", "sec-9"]], "kind": "security", "currency": "CAD", "symbol": symbol, "seen": seen});
    if let Some(m) = &venue.0 {
        v["mic"] = json!(m);
    }
    if let Some(w) = &venue.1 {
        v["venue"] = json!(w);
    }
    v
}

/// What a record stated of the instrument: its day, the order it was written in,
/// its symbol, and its venue as a code and in words (none, for a record stating none).
type Said = (String, usize, String, Option<(Option<String>, Option<String>)>);

/// The names the rule gives, written from the rule itself (brief 43), not from the
/// book's fold: the records in order of day, symbol and writing; one that states no
/// venue reads as on the venue the run before it is on, code and words; an unbroken
/// run of one symbol on one code is one name, its words the latest a record in it
/// stated, else those it was carried on. (symbol, code, words, first day, last day)
fn reference(said: &[Said]) -> Vec<(String, Option<String>, Option<String>, String, String)> {
    let mut order: Vec<&Said> = said.iter().collect();
    order.sort_by(|a, b| (&a.0, &a.2, a.1).cmp(&(&b.0, &b.2, b.1)));
    let mut runs: Vec<(String, Option<String>, Option<String>, String, String)> = Vec::new();
    for (day, _, symbol, venue) in order {
        let (mic, words) = match venue {
            Some(v) => v.clone(),
            None => runs.last().map(|r| (r.1.clone(), r.2.clone())).unwrap_or_default(),
        };
        match runs.last_mut() {
            Some(r) if r.0 == *symbol && r.1 == mic => {
                r.4 = day.clone();
                if words.is_some() {
                    r.2 = words;
                }
            }
            _ => runs.push((symbol.clone(), mic, words, day.clone(), day.clone())),
        }
    }
    runs
}

#[test]
fn the_venue_is_the_last_one_stated_and_a_record_stating_none_never_changes_it() {
    for seed in 1..=30u64 {
        let f = Fixture::new();
        f.account(&["a1"]);
        let account = f.book.accounts().unwrap()[0].id;
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        // the instrument, first named by the broker on a venue
        let first = stated(&mut rng);
        let r = f.store(&Spelled::v(1), "first", &legs(vec![buy("a1", share("ZQV", "2026-03-01", &first), "100", "-100", "2026-03-01T15:00:00Z")]));
        let id = f.book.transactions_of(r.record).unwrap()[0].instrument.unwrap();
        // what each record stated, by its day and the order it was written in
        let mut said: Vec<Said> = vec![("2026-03-01".into(), 0, "ZQV".into(), Some(first))];
        for n in 1..40usize {
            // any day of a span around the first: a record may be dated before what came earlier
            let day = format!("2026-0{}-{:02}", 2 + rng.below(3), 1 + rng.below(28));
            let at: jiff::Timestamp = format!("{day}T16:00:00Z").parse().unwrap();
            let venue = if rng.below(2) == 0 { Some(stated(&mut rng)) } else { None };
            let none = (None, None);
            // the ticker changes now and then, and back
            let symbol = if rng.below(4) == 0 { "ZQW" } else { "ZQV" };
            let writer = rng.below(4);
            let step = format!("seed {seed}, step {n}, writer {writer}, {day}, {venue:?}");
            match writer {
                // the broker's pull, and an import of the person's own records
                0 | 1 => {
                    let source = if writer == 0 { Spelled::v(1) } else { Spelled { source: "csv", version: 1 } };
                    f.store(&source, &format!("r{n}"), &legs(vec![buy("a1", share(symbol, &day, venue.as_ref().unwrap_or(&none)), "1", "-1", &format!("{day}T15:00:00Z"))]));
                }
                // a fill of the app's own order, on its listing's venue where the order states one
                2 => {
                    let (venue_mic, venue_name) = venue.clone().unwrap_or_default();
                    let id = format!("order-{n}");
                    let o = OrderRequest {
                        id: id.clone(),
                        broker: "wealthsimple".into(),
                        broker_account: "a1".into(),
                        broker_security: "sec-9".into(),
                        symbol: symbol.into(),
                        venue_mic,
                        venue_name,
                        currency: Currency::CAD,
                        side: Side::Buy,
                        kind: OrderKind::Market,
                        quantity: d("1"),
                        limit_price: None,
                        stop_price: None,
                        time_in_force: TimeInForce::Day,
                        bracket: None,
                        request: json!({}),
                    };
                    f.book.write_order(&o, false, &Asker::Person, at).unwrap();
                    f.book.order_event(&id, &Asker::Person, at, &OrderEvent::Accepted { broker_id: format!("ws-{n}") }).unwrap().unwrap();
                    f.book.order_event(&id, &Asker::Engine, at, &OrderEvent::Read(Reading::of(BrokerStatus::Filled, d("1"), Some(d("1"))))).unwrap().unwrap();
                }
                // a trade entered by hand on the instrument held: it names it by its reference alone
                _ => {
                    let day: jiff::civil::Date = day.parse().unwrap();
                    f.book.enter(&Entry::Trade { account, instrument: Traded::Held(id), day, side: EntrySide::Buy, quantity: d("1"), price: Money::new(d("1"), Currency::CAD), fee: None }, at).unwrap();
                }
            }
            // a fill is dated its own day in the book's zone, the others as stated:
            // all here are on `day`; a trade entered by hand on the instrument held
            // names it by its reference and states no name at all
            if writer != 3 {
                said.push((day, n, symbol.to_string(), venue));
            }
            // every name, against the rule's own
            let names: Vec<(String, Option<String>, Option<String>, String, String)> =
                f.book.names(id).unwrap().into_iter().map(|n| (n.symbol, n.venue_mic, n.venue_name, n.first_seen.to_string(), n.last_seen.to_string())).collect();
            assert_eq!(names, reference(&said), "{step}");
            // and the venue now is one the table knows: every venue stated here is
            let now = names.last().unwrap();
            assert!(venue_of(now.1.as_deref(), now.2.as_deref()).is_some(), "{step}: {names:?}");
        }
    }
}
