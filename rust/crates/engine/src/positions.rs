//! Open positions (`SPEC.md` §2 Position): the lots still held in one account and
//! instrument, one way. A position's id is the trade id of the round trip its
//! first lot belongs to, so a note on it is the note on the trade it becomes.

use std::collections::BTreeSet;

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::Date;
use bagholder_core::journal::{JournalEntry, JournalSubject};
use bagholder_core::{AccountId, Currency, Dec, InstrumentId, Money, Rounding, TradeId, TransactionId};

use crate::fx::live_to_cad;
use crate::gap::{Fig, Gap, Gaps};
use crate::identity::Identity;
use crate::input::{Inputs, QuoteSource};
use crate::ledger::{multiplier, Direction, Lot, Matched, TripKey};
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
    /// The round trip of its first lot.
    pub key: TripKey,
    pub trade: Option<TradeId>,
    pub account: AccountId,
    pub instrument: InstrumentId,
    pub kind: InstrumentKind,
    pub currency: Currency,
    pub direction: Direction,
    pub qty: Dec,
    pub multiplier: Fig<Dec>,
    /// Σ lot value: what the open units cost (long) or brought in (short).
    pub book: Fig<Money>,
    pub fees: Money,
    /// Book ÷ (qty × multiplier).
    pub avg: Fig<Dec>,
    pub mark: Fig<Mark>,
    /// Qty × price × multiplier.
    pub market: Fig<Money>,
    /// Market − book for a long, book − market for a short.
    pub unrealized: Fig<Money>,
    /// Qty × the day's change × multiplier, reversed for a short.
    pub day_change: Fig<Option<Money>>,
    pub book_cad: Fig<Money>,
    pub market_cad: Fig<Money>,
    pub unrealized_cad: Fig<Money>,
    pub day_change_cad: Fig<Option<Money>>,
    /// Quantity-weighted days since each lot was opened.
    pub held_days: i64,
    pub opened_on: Date,
    pub lots: Vec<Lot>,
    pub fills: BTreeSet<TransactionId>,
    /// What the holding waits on.
    pub gaps: Gaps,
    pub flags: BTreeSet<&'static str>,
    pub journal: JournalEntry,
    /// What the broker says the account holds of it now.
    pub broker_qty: Option<Dec>,
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
    if let Some(q) = inputs.market.quotes.get(&instrument).filter(|q| fits(kind, q.source) && q.price.currency == currency) {
        return Ok(Mark { price: q.price.amount, source: PriceSource::Quote, change: q.change, change_pct: q.change_pct });
    }
    let today = inputs.clock.today;
    if let Some((day, close)) = inputs.market.closes.get(&instrument).and_then(|c| c.range(..=today).next_back()) {
        // a close in another currency is converted at its own day's rate
        let price = crate::fx::convert(&inputs.facts.rates, &inputs.clock, *close, currency, *day)?;
        return Ok(Mark { price, source: PriceSource::Close(*day), change: None, change_pct: None });
    }
    Err(Gaps::of(Gap::PriceUnknown(instrument)))
}

fn lot_sum(currency: Currency, lots: &[Lot]) -> Fig<Money> {
    let mut total = Money::zero(currency);
    let mut gaps = Gaps::none();
    for l in lots {
        match &l.value {
            Ok(v) if gaps.is_empty() => total = total.add_to_fit(*v)?,
            Ok(_) => {}
            Err(g) => gaps.merge(g),
        }
    }
    gaps.or(total)
}

/// The open positions, in the order of their holdings; `only` one instrument's.
pub fn build_positions(inputs: &Inputs, matched: &Matched, identity: &Identity, only: Option<InstrumentId>) -> Vec<PositionFig> {
    let today = inputs.clock.today;
    let rates = &inputs.facts.rates;
    let clock = &inputs.clock;
    let mut out = Vec::new();
    for ((account, instrument), book) in &matched.books {
        if only.is_some_and(|o| o != *instrument) {
            continue;
        }
        for direction in [Direction::Long, Direction::Short] {
            let lots: Vec<Lot> = book.lots.iter().filter(|l| l.direction == direction).cloned().collect();
            let Some(first) = lots.first() else { continue };
            let info = inputs.ledger.instruments.get(instrument);
            let kind = info.map(|i| i.instrument.kind).unwrap_or(InstrumentKind::Security);
            let currency = info.map(|i| i.instrument.currency).unwrap_or(Currency::CAD);
            let mult = multiplier(info, *instrument);
            let qty = lots.iter().fold(Dec::ZERO, |a, l| a.add_to_fit(l.qty).unwrap_or(a));
            let fees = lots.iter().fold(Money::zero(currency), |a, l| a.add_to_fit(l.fee).unwrap_or(a));
            let taint = book.taint.clone();
            let with_taint = |f: Fig<Money>| -> Fig<Money> {
                if taint.is_empty() {
                    f
                } else {
                    let mut g = taint.clone();
                    if let Err(e) = &f {
                        g.merge(e);
                    }
                    Err(g)
                }
            };
            let book_value = with_taint(lot_sum(currency, &lots));
            let units: Fig<Dec> = mult.clone().and_then(|m| Ok(qty.checked_mul(m)?));
            let avg = match (&book_value, &units) {
                (Ok(b), Ok(u)) if !u.is_zero() => b.amount.div_rounded(*u, PRICE_PLACES, Rounding::HalfEven).map_err(Gaps::from),
                (Ok(_), Ok(_)) => Ok(Dec::ZERO),
                (Err(g), _) | (_, Err(g)) => Err(g.clone()),
            };
            let mark = mark_of(inputs, *instrument, kind, currency);
            let market = match (&mark, &units) {
                (Ok(m), Ok(u)) => with_taint(m.price.mul_to_fit(*u).map(|v| Money::new(v, currency)).map_err(Gaps::from)),
                (Err(g), _) | (_, Err(g)) => Err(g.clone()),
            };
            let unrealized = match (&market, &book_value) {
                (Ok(mk), Ok(bk)) => match direction {
                    Direction::Long => mk.checked_sub(*bk).map_err(Gaps::from),
                    Direction::Short => bk.checked_sub(*mk).map_err(Gaps::from),
                },
                (Err(g), Ok(_)) | (Ok(_), Err(g)) => Err(g.clone()),
                (Err(a), Err(b)) => {
                    let mut g = a.clone();
                    g.merge(b);
                    Err(g)
                }
            };
            let day_change: Fig<Option<Money>> = match (&mark, &units) {
                (Ok(m), Ok(u)) => match m.change.map(|c| c.mul_to_fit(*u)) {
                    Some(Ok(v)) => {
                        let v = if direction == Direction::Short { v.neg() } else { v };
                        if taint.is_empty() {
                            Ok(Some(Money::new(v, currency)))
                        } else {
                            Err(taint.clone())
                        }
                    }
                    Some(Err(e)) => Err(Gaps::from(e)),
                    None => Ok(None),
                },
                (Err(g), _) | (_, Err(g)) => Err(g.clone()),
            };
            let cad = |f: &Fig<Money>| f.clone().and_then(|m| live_to_cad(rates, clock, m));
            let day_change_cad = match &day_change {
                Ok(Some(m)) => live_to_cad(rates, clock, *m).map(Some),
                Ok(None) => Ok(None),
                Err(g) => Err(g.clone()),
            };
            // quantity-weighted days held
            let weighted = lots.iter().try_fold(Dec::ZERO, |a, l| a.add_to_fit(l.qty.checked_mul(Dec::from_int((today - l.day).get_days() as i64))?));
            let held_days = match weighted.and_then(|w| w.div_rounded(qty, 0, Rounding::HalfEven)) {
                Ok(d) => d.to_text().parse::<i64>().unwrap_or(0),
                Err(_) => 0,
            };
            let key = first.trip.clone();
            let trade = identity.trade_of.get(&key).copied();
            let journal = trade.and_then(|t| inputs.ledger.journal.get(&JournalSubject::Trade(t)).cloned()).unwrap_or_default();
            let mut fills = BTreeSet::new();
            let mut flags = BTreeSet::new();
            for l in &lots {
                if let Some(t) = matched.trips.get(&l.trip) {
                    fills.extend(t.fills.iter().cloned());
                }
                flags.extend(l.flags.iter().map(|f| f.word()));
            }
            let mut gaps = taint.clone();
            for f in [&book_value, &market] {
                if let Err(g) = f {
                    gaps.merge(g);
                }
            }
            out.push(PositionFig {
                key,
                trade,
                account: *account,
                instrument: *instrument,
                kind,
                currency,
                direction,
                qty,
                multiplier: mult,
                book_cad: cad(&book_value),
                market_cad: cad(&market),
                unrealized_cad: cad(&unrealized),
                book: book_value,
                fees,
                avg,
                mark,
                market,
                unrealized,
                day_change,
                day_change_cad,
                held_days,
                opened_on: lots.iter().map(|l| l.day).min().unwrap_or(today),
                fills,
                gaps,
                flags,
                journal,
                broker_qty: inputs.market.brokers.get(account).and_then(|b| b.held.get(instrument)).copied(),
                lots,
            });
        }
    }
    out
}
