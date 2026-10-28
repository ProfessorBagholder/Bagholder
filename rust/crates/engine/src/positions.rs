//! Open positions (`SPEC.md` §2 Position): what a broker states one account
//! holds of one instrument, one way, valued (`crate::stated`, brief 79), and the
//! journal's round trips open in it. A position's trade is the trade of the round
//! trip its first lot belongs to, so a note on it is the note on the trade it
//! becomes.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::Date;
use bagholder_core::journal::{JournalEntry, JournalSubject};
use bagholder_core::{AccountId, Currency, Dec, InstrumentId, Money, Rounding, TradeId, TransactionId};

use crate::gap::{Fig, Gap, Gaps};
use crate::identity::Identity;
use crate::input::{Clock, Inputs, Market, QuoteSource, Rates};
use crate::ledger::{Direction, Lot, Matched, TripKey};
use crate::trades::PRICE_PLACES;

/// Where a position's price comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PriceSource {
    /// A live quote from the instrument's own kind's source.
    Quote,
    /// The last stored daily close, on this day.
    Close(Date),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub price: Dec,
    pub source: PriceSource,
    /// The day's change per unit and in percent, from a quote that states them.
    pub change: Option<Dec>,
    pub change_pct: Option<Dec>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PositionFig {
    /// The round trip of its first lot: its id, and the journal the holding
    /// shows; none where the journal holds no lot of it.
    pub key: Option<TripKey>,
    pub trade: Option<TradeId>,
    /// Every round trip open in it, oldest lot first, `key` first. A holding
    /// can carry several (a deposited coin beside bought ones, shares delivered
    /// into a holding): each becomes its own trade as it closes.
    pub trips: Vec<TripKey>,
    pub account: AccountId,
    pub instrument: InstrumentId,
    pub kind: InstrumentKind,
    pub currency: Currency,
    pub direction: Direction,
    /// The units its broker states it holds, exact (a short's as many as it owes).
    pub qty: Fig<Dec>,
    pub multiplier: Fig<Dec>,
    /// What its broker states the units cost (long) or brought in (short), in
    /// the currency it states (`crate::stated`).
    pub book: Fig<Money>,
    /// The currency of `book`, `avg` and `unrealized`.
    pub book_currency: Currency,
    /// What the journal's lots in it paid in fees.
    pub fees: Fig<Money>,
    /// Book ÷ (qty × multiplier).
    pub avg: Fig<Dec>,
    pub mark: Fig<Mark>,
    /// Qty × price × multiplier.
    pub market: Fig<Money>,
    /// Market − book for a long, book − market for a short, in `book_currency`.
    pub unrealized: Fig<Money>,
    /// Qty × the day's change × multiplier, reversed for a short.
    pub day_change: Fig<Option<Money>>,
    pub book_cad: Fig<Money>,
    pub market_cad: Fig<Money>,
    pub unrealized_cad: Fig<Money>,
    pub day_change_cad: Fig<Option<Money>>,
    /// Quantity-weighted days since each of the journal's lots in it was
    /// opened; none where it holds none.
    pub held_days: Option<Fig<i64>>,
    pub opened_on: Option<Date>,
    pub lots: Vec<Lot>,
    pub fills: BTreeSet<TransactionId>,
    /// What the holding waits on.
    pub gaps: Gaps,
    pub flags: BTreeSet<&'static str>,
    pub journal: JournalEntry,
}

fn fits(kind: InstrumentKind, source: QuoteSource) -> bool {
    match kind {
        InstrumentKind::Crypto => source == QuoteSource::Crypto,
        InstrumentKind::OptionContract => source == QuoteSource::OptionChain,
        _ => source == QuoteSource::Listing,
    }
}

/// The instrument's price: its quote from its own kind's source, else its last
/// stored close, else none. The person's own fill price is never a market price.
pub fn mark_of(inputs: &Inputs, instrument: InstrumentId, kind: InstrumentKind, currency: Currency) -> Fig<Mark> {
    mark_in(&inputs.market, &inputs.facts.rates, &inputs.clock, instrument, kind, currency)
}

/// `mark_of`, from the market's prices and the rates alone.
pub fn mark_in(market: &Market, rates: &Rates, clock: &Clock, instrument: InstrumentId, kind: InstrumentKind, currency: Currency) -> Fig<Mark> {
    if let Some(q) = market.quotes.get(&instrument).filter(|q| fits(kind, q.source) && q.price.currency == currency) {
        return Ok(Mark { price: q.price.amount, source: PriceSource::Quote, change: q.change, change_pct: q.change_pct });
    }
    if let Some((day, close)) = market.closes.get(&instrument).and_then(|c| c.range(..=clock.today).next_back()) {
        // a close in another currency is converted at its own day's rate
        let price = crate::fx::convert(rates, clock, *close, currency, *day)?;
        return Ok(Mark { price, source: PriceSource::Close(*day), change: None, change_pct: None });
    }
    Err(Gaps::of(Gap::PriceUnknown(instrument)))
}

/// The instrument's price at the close of `day`: its last stored close on or
/// before it (the engine's walk-back for today and for rates, `SPEC.md` §5), with
/// that session's move from the close before it. A close in another currency is
/// converted at its own day's rate.
pub fn close_mark(inputs: &Inputs, instrument: InstrumentId, currency: Currency, day: Date) -> Fig<Mark> {
    close_mark_in(&inputs.market.closes, &inputs.facts.rates, &inputs.clock, instrument, currency, day)
}

/// `close_mark`, from the stored closes and the rates alone.
pub fn close_mark_in(closes: &BTreeMap<InstrumentId, BTreeMap<Date, Money>>, rates: &Rates, clock: &Clock, instrument: InstrumentId, currency: Currency, day: Date) -> Fig<Mark> {
    let closes = closes.get(&instrument);
    let mut last = closes.into_iter().flat_map(|c| c.range(..=day).rev());
    let Some((d, close)) = last.next() else { return Err(Gaps::of(Gap::PriceUnknown(instrument))) };
    let price = crate::fx::convert(rates, clock, *close, currency, *d)?;
    let (change, change_pct) = match last.next() {
        Some((d0, c0)) => {
            let before = crate::fx::convert(rates, clock, *c0, currency, *d0)?;
            let change = price.checked_sub(before)?;
            let pct = if before.is_zero() { None } else { Some(change.checked_mul(Dec::new(100, 0)?)?.div_rounded(before, PRICE_PLACES, Rounding::HalfEven)?) };
            (Some(change), pct)
        }
        None => (None, None),
    };
    Ok(Mark { price, source: PriceSource::Close(*d), change, change_pct })
}

/// The instrument's price now, as a holding of it is marked; none where none is known.
pub fn current_price(inputs: &Inputs, instrument: InstrumentId) -> Option<Dec> {
    let info = inputs.ledger.instruments.get(&instrument)?;
    mark_of(inputs, instrument, info.instrument.kind, info.instrument.currency).ok().map(|m| m.price)
}

/// The open positions: the holdings each broker states (`crate::stated`), in the
/// order of their accounts and instruments, each linked to the journal's round
/// trips open in it; only `account`'s, and only `instrument`'s, where given, so
/// a change to one builds only what it touches. Every amount is the broker's
/// (brief 79 §1): the journal gives a holding its trade, its journal entry, its
/// fills, when it was opened and the fees its lots paid, never an amount.
pub fn build_positions(inputs: &Inputs, matched: &Matched, identity: &Identity, account: Option<AccountId>, instrument: Option<InstrumentId>) -> Vec<PositionFig> {
    let sources = crate::stated::Sources::of(inputs);
    let stated = inputs.market.brokers.iter().filter(|(a, _)| account.is_none_or(|x| x == **a)).map(|(a, b)| crate::stated::Stated { account: *a, held: &b.held, book: &b.held_book });
    let held = match instrument {
        Some(i) => stated.filter_map(|s| crate::stated::holding_of(&sources, &s, i, None)).collect(),
        None => crate::stated::holdings(&sources, stated, None),
    };
    held.into_iter().map(|h| linked(inputs, Some(matched), Some(identity), h)).collect()
}

/// A holding with the journal's round trips open in it the same way: the trade
/// of its first lot, that trade's journal entry, the lots' fills and flags, when
/// they were opened and the fees they paid. A holding the journal holds no lot of,
/// or one of a past day (no journal given), has none of them.
pub(crate) fn linked(inputs: &Inputs, matched: Option<&Matched>, identity: Option<&Identity>, h: crate::stated::Holding) -> PositionFig {
    let today = inputs.clock.today;
    let lots: Vec<Lot> = matched.and_then(|m| m.books.get(&(h.account, h.instrument))).map(|b| b.lots.iter().filter(|l| l.direction == h.direction).cloned().collect()).unwrap_or_default();
    let key = lots.first().map(|l| l.trip.clone());
    let mut trips: Vec<TripKey> = Vec::new();
    for l in &lots {
        if !trips.contains(&l.trip) {
            trips.push(l.trip.clone());
        }
    }
    let trade = key.as_ref().and_then(|k| identity?.trade_of.get(k).copied());
    let journal = trade.and_then(|t| inputs.ledger.journal.get(&JournalSubject::Trade(t)).cloned()).unwrap_or_default();
    let mut fills = BTreeSet::new();
    let mut flags = BTreeSet::new();
    for l in &lots {
        if let Some(t) = matched.and_then(|m| m.trips.get(&l.trip)) {
            fills.extend(t.fills.iter().cloned());
        }
        flags.extend(l.flags.iter().map(|f| f.word()));
    }
    let fees: Fig<Money> = lots.iter().try_fold(Money::zero(h.currency), |a, l| a.add_to_fit(l.fee)).map_err(Gaps::from);
    // quantity-weighted days since each lot was opened
    let held_days: Option<Fig<i64>> = (!lots.is_empty()).then(|| {
        let open = lots.iter().try_fold(Dec::ZERO, |a, l| a.checked_add(l.qty)).map_err(Gaps::from)?;
        let weighted = lots.iter().try_fold(Dec::ZERO, |a, l| a.add_to_fit(l.qty.checked_mul(Dec::from_int((today - l.day).get_days() as i64))?))?;
        Ok(weighted.div_rounded(open, 0, Rounding::HalfEven)?.to_int()?)
    });
    PositionFig {
        key,
        trade,
        trips,
        account: h.account,
        instrument: h.instrument,
        kind: h.kind,
        currency: h.currency,
        direction: h.direction,
        qty: Ok(h.qty),
        multiplier: h.multiplier,
        book: h.book,
        book_currency: h.book_currency,
        fees,
        avg: h.avg,
        mark: h.mark,
        market: h.market,
        unrealized: h.unrealized,
        day_change: h.day_change,
        book_cad: h.book_cad,
        market_cad: h.market_cad,
        unrealized_cad: h.unrealized_cad,
        day_change_cad: h.day_change_cad,
        held_days,
        opened_on: lots.iter().map(|l| l.day).min(),
        lots,
        fills,
        gaps: h.gaps,
        flags,
        journal,
    }
}
