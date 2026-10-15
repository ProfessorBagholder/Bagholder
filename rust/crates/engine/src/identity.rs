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

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::journal::Anchor;
use bagholder_core::{InstrumentId, TradeId};


use crate::input::Ledger;
use crate::ledger::{Matched, TripKey};

/// What the book has to do for every round trip to carry a trade id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    /// Each round trip's trade.
    pub trade_of: BTreeMap<TripKey, TradeId>,
    /// Each trade's round trip: `trade_of` the other way.
    pub trip_of: BTreeMap<TradeId, TripKey>,
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

/// The trade anchored on each opening of `instruments`' (the last in the book's
/// order where two are).
fn anchors(ledger: &Ledger, instruments: &BTreeSet<InstrumentId>) -> BTreeMap<TripKey, TradeId> {
    let mut out = BTreeMap::new();
    for i in instruments {
        for t in ledger.trades.on(*i) {
            if let Anchor::Opening(ref o) = t.anchor {
                out.insert(TripKey { opening: o.transaction.clone(), instrument: o.instrument }, t.id);
            }
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
    let instruments: BTreeSet<InstrumentId> = matched.trips_of.keys().copied().chain(ledger.trades.iter().filter_map(|t| crate::input::Keyed::instrument(t))).collect();
    let mut out = identify_in(ledger, matched, &instruments);
    // a trade anchored on nothing the ledger holds an opening of is unclaimed:
    // every trade is on one of `instruments`, or orphaned
    out.in_book_order(ledger);
    out
}

/// The identity of the round trips keyed on `instruments` and of the trades
/// anchored on them: what a family's change moves (`crate::engine`).
pub fn identify_in(ledger: &Ledger, matched: &Matched, instruments: &BTreeSet<InstrumentId>) -> Identity {
    let anchored = anchors(ledger, instruments);
    let mut out = Identity::default();
    let trips = || instruments.iter().flat_map(|i| matched.trips_of.get(i).into_iter().flatten()).filter_map(|k| matched.trips.get(k));
    for trip in trips() {
        let key = &trip.key;
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
    let openings: BTreeSet<&TripKey> = trips().flat_map(|t| t.openings.iter()).collect();
    // in the book's order, so of two trades that would move to one round trip the
    // one made first takes it
    let mut trades: Vec<&bagholder_core::journal::Trade> = instruments.iter().flat_map(|i| ledger.trades.on(*i)).collect();
    trades.sort_by_key(|t| ledger.trades.rank(&t.id).cloned());
    for t in trades {
        let Anchor::Opening(ref o) = t.anchor else { continue };
        let key = TripKey { opening: o.transaction.clone(), instrument: o.instrument };
        if openings.contains(&key) || anchored.get(&key) != Some(&t.id) {
            continue;
        }
        let mut holding = trips().filter(|trip| trip.fills.contains(&o.transaction) && trip.instruments.contains(&o.instrument));
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
    out.trip_of = out.trade_of.iter().map(|(k, t)| (*t, k.clone())).collect();
    out.in_book_order(ledger);
    out
}

/// The identity of the round trips `trips` and of the trades `named`, with every
/// trade anchored on a fill of one of `trips`: what a change that reached only
/// those round trips moves (a continued match, a trade or its journal changed;
/// `docs/plans/stage-3-a-change-on-a-deep-history.md`, C). Every round trip a
/// trade of these can claim is one of `trips` (the caller's reach), so this is
/// what `identify_in` gives them.
pub fn identify_trips(ledger: &Ledger, matched: &Matched, trips: &BTreeSet<TripKey>, named: &BTreeSet<TradeId>) -> (Identity, BTreeSet<TradeId>) {
    let mut out = Identity::default();
    // the trades: the ones named, and every one anchored on a fill of a round trip reached
    let mut trades: BTreeSet<TradeId> = named.clone();
    for k in trips {
        for f in matched.trips.get(k).into_iter().flat_map(|t| t.fills.iter()) {
            trades.extend(ledger.trades.anchored_on(f).map(|t| t.id));
        }
    }
    // the trade anchored on an opening (the last in the book's order where two are)
    let anchored = |o: &TripKey| -> Option<TradeId> {
        ledger.trades.anchored_on(&o.opening).filter(|t| matches!(&t.anchor, Anchor::Opening(x) if x.instrument == o.instrument)).last().map(|t| t.id)
    };
    for k in trips {
        let Some(trip) = matched.trips.get(k) else { continue };
        if managed(ledger, trip.account) {
            continue;
        }
        let mut found = trip.openings.iter().filter_map(anchored);
        match found.next() {
            Some(keeps) => {
                out.trade_of.insert(k.clone(), keeps);
                for other in found {
                    if other != keeps {
                        out.joined.push((other, keeps));
                    }
                }
            }
            None => out.needs_trade.push(k.clone()),
        }
    }
    // in the book's order, so of two trades that would move to one round trip the
    // one made first takes it
    let mut ordered: Vec<&bagholder_core::journal::Trade> = trades.iter().filter_map(|t| ledger.trades.get(t)).collect();
    ordered.sort_by_key(|t| ledger.trades.rank(&t.id).cloned());
    for t in ordered {
        let Anchor::Opening(ref o) = t.anchor else { continue };
        let key = TripKey { opening: o.transaction.clone(), instrument: o.instrument };
        // an opening of some round trip, or not the trade that keeps its opening
        let opens = matched.trips_by_fill.get(&o.transaction).into_iter().flatten().any(|k| matched.trips.get(k).is_some_and(|tr| tr.openings.contains(&key)));
        if opens || anchored(&key) != Some(t.id) {
            continue;
        }
        let mut holding = matched.trips_by_fill.get(&o.transaction).into_iter().flatten().filter_map(|k| matched.trips.get(k)).filter(|trip| trip.instruments.contains(&o.instrument));
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
    out.trip_of = out.trade_of.iter().map(|(k, t)| (*t, k.clone())).collect();
    out.in_book_order(ledger);
    (out, trades)
}

impl Identity {
    /// The lists in one order, however the identity was put together: the round
    /// trips needing a trade by key (the order the book opens them in), the
    /// joined by trade, the moved and the unclaimed in the book's order.
    pub fn in_book_order(&mut self, ledger: &Ledger) {
        self.needs_trade.sort();
        self.needs_trade.dedup();
        self.joined.sort();
        self.joined.dedup();
        self.moved.sort_by_key(|(t, k)| (ledger.trades.rank(t).cloned(), k.clone()));
        self.unclaimed.sort_by_key(|t| (ledger.trades.rank(t).cloned(), *t));
    }

    /// What concerns the round trips `trips` and the trades `trades` taken out, and
    /// `fresh`, their identity again (`identify_trips`), put in.
    pub fn replace_trips(&mut self, ledger: &Ledger, trips: &BTreeSet<TripKey>, trades: &BTreeSet<TradeId>, fresh: Identity) {
        self.trade_of.retain(|k, t| !trips.contains(k) && !trades.contains(t));
        self.trip_of.retain(|t, k| !trips.contains(k) && !trades.contains(t));
        self.needs_trade.retain(|k| !trips.contains(k));
        self.joined.retain(|(o, k)| !(trades.contains(o) || trades.contains(k)));
        self.moved.retain(|(t, k)| !trips.contains(k) && !trades.contains(t));
        self.unclaimed.retain(|t| !trades.contains(t));
        let Identity { trade_of, trip_of, needs_trade, joined, moved, unclaimed } = fresh;
        self.trade_of.extend(trade_of);
        self.trip_of.extend(trip_of);
        self.needs_trade.extend(needs_trade);
        self.joined.extend(joined);
        self.moved.extend(moved);
        self.unclaimed.extend(unclaimed);
        self.in_book_order(ledger);
    }

    /// What concerns `instruments` taken out (the round trips keyed on them, and
    /// the trades anchored on them or gone from the ledger) and `fresh`, their
    /// identity again, put in.
    pub fn replace(&mut self, ledger: &Ledger, instruments: &BTreeSet<InstrumentId>, fresh: Identity) {
        let on = |t: &TradeId| ledger.trades.get(t).is_none_or(|t| crate::input::Keyed::instrument(t).is_none_or(|i| instruments.contains(&i)));
        self.trade_of.retain(|k, _| !instruments.contains(&k.instrument));
        self.trip_of.retain(|_, k| !instruments.contains(&k.instrument));
        self.needs_trade.retain(|k| !instruments.contains(&k.instrument));
        self.joined.retain(|(o, k)| !(on(o) || on(k)));
        self.moved.retain(|(_, k)| !instruments.contains(&k.instrument));
        self.unclaimed.retain(|t| !on(t));
        let Identity { trade_of, trip_of, needs_trade, joined, moved, unclaimed } = fresh;
        self.trade_of.extend(trade_of);
        self.trip_of.extend(trip_of);
        self.needs_trade.extend(needs_trade);
        self.joined.extend(joined);
        self.moved.extend(moved);
        self.unclaimed.extend(unclaimed);
        self.in_book_order(ledger);
    }
}
