//! Which trade id each round trip carries (`docs/plans/stage-2-engine.md`, "Trade
//! identity").
//!
//! A round trip takes the id of the trade anchored on its earliest opening that
//! has one. The engine assigns nothing: it says which round trips have no trade
//! yet (for the book to open one on their key) and which trades a correction
//! joined into another's round trip (for the book to orphan, the journal kept).

use std::collections::BTreeMap;

use bagholder_core::journal::Anchor;
use bagholder_core::TradeId;

use crate::input::Ledger;
use crate::ledger::{Matched, TripKey};

/// What the book has to do for every round trip to carry a trade id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    /// Each round trip's trade.
    pub trade_of: BTreeMap<TripKey, TradeId>,
    /// Round trips none of whose openings has a trade: the book opens one on the key.
    pub needs_trade: Vec<TripKey>,
    /// A trade whose round trip a correction joined into another's: the trade
    /// that keeps the round trip, for the reason the book records.
    pub joined: Vec<(TradeId, TradeId)>,
}

/// The trade anchored on each opening.
fn anchors(ledger: &Ledger) -> BTreeMap<TripKey, TradeId> {
    let mut out = BTreeMap::new();
    for t in &ledger.trades {
        if let Anchor::Opening(ref o) = t.anchor {
            out.insert(TripKey { opening: o.transaction.clone(), instrument: o.instrument }, t.id);
        }
    }
    out
}

pub fn identify(ledger: &Ledger, matched: &Matched) -> Identity {
    let anchored = anchors(ledger);
    let mut out = Identity::default();
    for (key, trip) in &matched.trips {
        let mut found = trip.openings.iter().filter_map(|o| anchored.get(o).copied());
        match found.next() {
            Some(keeps) => {
                out.trade_of.insert(key.clone(), keeps);
                for other in found {
                    if other != keeps {
                        out.joined.push((other, keeps));
                    }
                }
            }
            None => out.needs_trade.push(key.clone()),
        }
    }
    out
}
