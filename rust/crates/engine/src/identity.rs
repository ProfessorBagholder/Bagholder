//! Which trade id each round trip carries (`docs/plans/stage-2-engine.md`, "Trade
//! identity").
//!
//! A round trip takes the id of the trade anchored on its earliest opening that
//! has one. The engine assigns nothing: it says which round trips have no trade
//! yet (for the book to open one on their key), which trades a correction moved
//! onto another opening of the same round trip (for the book to move), and which
//! trades a correction joined into another's round trip or left with no round
//! trip at all (for the book to orphan, the journal kept).
//!
//! A trade's id is kept for good (`SPEC.md` §2, Trade): a correction to the rows
//! behind it keeps the trade and its journal. So a trade whose anchor no longer
//! opens a round trip, but is still a fill of exactly one round trip that no
//! other trade holds, is that round trip's trade and moves to its opening; only
//! one with no such round trip is orphaned.

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
    /// Trades anchored on a transaction that opens no round trip any more but is
    /// still a fill of exactly one round trip no other trade holds (a correction
    /// made it a close, or ordered an earlier fill before it): the round trip the
    /// book moves each one to, its id and journal kept.
    pub moved: Vec<(TradeId, TripKey)>,
    /// Trades anchored on a transaction that opens no round trip any more and is
    /// a fill of no round trip it can move to (the transaction is gone, its round
    /// trip holds another trade, or it is a fill of more than one): the book
    /// orphans them, their notes kept for the person to re-attach.
    pub unclaimed: Vec<TradeId>,
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

/// Whether an account is managed by its broker: the trades the broker makes in
/// it are not the person's, and make no round trip (`docs/decisions.md`,
/// 2026-09-25). Its holdings, cash, income and value count as any account's.
pub fn managed(ledger: &Ledger, account: bagholder_core::AccountId) -> bool {
    ledger.accounts.get(&account).is_some_and(|a| matches!(a.account.account_type, bagholder_core::account::AccountType::Known { managed: true, .. }))
}

pub fn identify(ledger: &Ledger, matched: &Matched) -> Identity {
    let anchored = anchors(ledger);
    let mut out = Identity::default();
    for (key, trip) in &matched.trips {
        // a trip held in a managed account is no trade of the person's
        if managed(ledger, trip.account) {
            continue;
        }
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
    let openings: std::collections::BTreeSet<&TripKey> = matched.trips.values().flat_map(|t| t.openings.iter()).collect();
    // in the book's order, so of two trades that would move to one round trip the
    // one made first takes it
    for t in &ledger.trades {
        let Anchor::Opening(ref o) = t.anchor else { continue };
        let key = TripKey { opening: o.transaction.clone(), instrument: o.instrument };
        if openings.contains(&key) || anchored.get(&key) != Some(&t.id) {
            continue;
        }
        let mut holding = matched.trips.values().filter(|trip| trip.fills.contains(&o.transaction) && trip.instruments.contains(&o.instrument));
        let to = match (holding.next(), holding.next()) {
            (Some(trip), None) if !managed(ledger, trip.account) && !out.trade_of.contains_key(&trip.key) => Some(trip.key.clone()),
            _ => None,
        };
        match to {
            Some(k) => {
                out.needs_trade.retain(|n| *n != k);
                out.trade_of.insert(k.clone(), t.id);
                out.moved.push((t.id, k));
            }
            None => out.unclaimed.push(t.id),
        }
    }
    out
}
