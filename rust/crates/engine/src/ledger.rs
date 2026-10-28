//! Lots and round trips on Bagholder's transactions (`docs/plans/stage-2-engine.md`,
//! "The ledger").
//!
//! Lots are matched first in first out per account and instrument, never per
//! symbol. A round trip ("trip") is a position going from flat, through open,
//! back to flat; its key is its opening transaction and instrument, and the book
//! gives it a trade id. What the record does not state is never worked out: a
//! quantity not stated, a multi-leg order without its legs, a contract size not
//! stated, a corporate event without its values each leave the holdings they
//! touch waiting (`Gap`), from that day on.
//!
//! A fill's value is its price × quantity × the contract's multiplier where the
//! source states a price, otherwise its cash less its fee (a purchase pays its
//! value and its fee; a sale receives its value less its fee). A lot carries the
//! value and the fee still open; a part of a lot or of a fill is its share by
//! quantity, to `SHARE_PLACES`, the last part taking what is left, so the parts
//! always add up to the whole exactly.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bagholder_core::instrument::{InstrumentKind, OptionRight};
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::transaction::{Effect, Kind, Transaction};
use bagholder_core::{AccountId, Currency, Dec, InstrumentId, Money, Rounding, TransactionId};

use crate::gap::{Fig, Gap, Gaps};
use crate::input::{AdjustmentLeg, Inputs, InstrumentInfo};

/// Places a pro-rata share is computed to before the last part takes the rest.
pub const SHARE_PLACES: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    Long,
    Short,
}

impl Direction {
    fn opposite(self) -> Direction {
        match self {
            Direction::Long => Direction::Short,
            Direction::Short => Direction::Long,
        }
    }
}

/// Marks a lot or a slice carries for the screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Flag {
    /// Opened by a staking reward, at no cost.
    Reward,
    /// Came into the account as a transfer of the asset, not a purchase here.
    Deposited,
    /// Part of a chain of contracts rolled one into the next.
    Rolled,
    /// Closed at zero on its expiry date by its terms, with nothing on the record.
    NoExpiryRecord,
    /// Closed or opened by an assignment or exercise.
    Assignment,
    /// Its quantity changed by a split or consolidation.
    Split,
    /// Moved to another instrument by a corporate event.
    Continued,
    /// Opened by a stock dividend or a spin-off.
    FromEvent,
    /// Moved in from another of the person's accounts.
    Transferred,
    /// Its cost is one the person entered (`SPEC.md` §2, What you enter: each
    /// shows as entered by you): an opening balance, a spin-off's share, capital
    /// returned.
    Entered,
    /// Written off at nothing as dust: what a fill left of a coin, worth less
    /// than the smallest order the broker takes (`crate::dust`).
    Dust,
    /// Units the activity feed never showed arriving, as Wealthsimple's positions
    /// read of the day before a transaction that took them states them held (brief
    /// 79, amended 2026-10-28): opened from that read, its cost the read's.
    Stated,
}

impl Flag {
    pub fn word(self) -> &'static str {
        match self {
            Flag::Reward => "reward",
            Flag::Deposited => "deposited",
            Flag::Rolled => "rolled",
            Flag::NoExpiryRecord => "no-expiry-record",
            Flag::Assignment => "assignment",
            Flag::Split => "split",
            Flag::Continued => "continued",
            Flag::FromEvent => "from-event",
            Flag::Transferred => "transferred",
            Flag::Entered => "entered",
            Flag::Dust => "dust",
            Flag::Stated => "stated",
        }
    }
}

/// A round trip's key: the transaction that opened it, and the instrument it
/// opened (an assignment opens the underlying's round trip while closing the
/// contract's, so the transaction alone is not enough).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TripKey {
    pub opening: TransactionId,
    pub instrument: InstrumentId,
}

/// What closed a slice.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Closer {
    Transaction(TransactionId),
    /// The contract's own expiry date, with nothing on the record.
    Expiry,
}

/// Quantity still open from one opening.
#[derive(Clone, Debug, PartialEq)]
pub struct Lot {
    pub trip: TripKey,
    pub opened_by: TransactionId,
    pub day: Date,
    pub at: Option<Timestamp>,
    pub direction: Direction,
    /// Always positive.
    pub qty: Dec,
    /// What the open quantity cost (long) or brought in (short), fees apart.
    pub value: Fig<Money>,
    /// The opening fee still attached to the open quantity.
    pub fee: Money,
    pub flags: BTreeSet<Flag>,
}

/// One closed piece of a lot, against one closing.
#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub trip: TripKey,
    pub account: AccountId,
    pub instrument: InstrumentId,
    pub direction: Direction,
    pub qty: Dec,
    pub opened_by: TransactionId,
    pub opened_on: Date,
    pub opened_at: Option<Timestamp>,
    pub closed_by: Closer,
    pub closed_on: Date,
    pub closed_at: Option<Timestamp>,
    pub entry: Fig<Money>,
    pub exit: Fig<Money>,
    pub entry_fee: Money,
    pub exit_fee: Money,
    pub flags: BTreeSet<Flag>,
    /// What the holding was waiting on when this closed.
    pub taint: Gaps,
}

impl Slice {
    /// Exit less entry for a long, entry less exit for a short, less both fees.
    pub fn pnl(&self) -> Fig<Money> {
        let mut gaps = self.taint.clone();
        for f in [&self.entry, &self.exit] {
            if let Err(g) = f {
                gaps.merge(g);
            }
        }
        let (Ok(entry), Ok(exit)) = (&self.entry, &self.exit) else { return Err(gaps) };
        if !gaps.is_empty() {
            return Err(gaps);
        }
        let gross = match self.direction {
            Direction::Long => exit.checked_sub(*entry)?,
            Direction::Short => entry.checked_sub(*exit)?,
        };
        Ok(gross.checked_sub(self.entry_fee)?.checked_sub(self.exit_fee)?)
    }
}

/// One round trip, possibly across contracts (a roll) or accounts (a linked
/// transfer), possibly still open.
#[derive(Clone, Debug, PartialEq)]
pub struct Trip {
    pub key: TripKey,
    /// Where it is held now, or was when it closed.
    pub account: AccountId,
    pub direction: Direction,
    /// Every instrument it held, in the order it first held them; the last is
    /// the one it is named after.
    pub instruments: Vec<InstrumentId>,
    pub slices: Vec<Slice>,
    /// Every transaction that opened or closed part of it.
    pub fills: BTreeSet<TransactionId>,
    pub opened: BTreeSet<TransactionId>,
    pub closed: BTreeSet<TransactionId>,
    /// Lots of it still open, across its books.
    pub open_lots: usize,
    /// Units it opened that left it with no sale: sent out of the person's
    /// accounts, moved to a round trip of their own in another of them, or taken
    /// by an event with nothing after it. Still units it opened (`SPEC.md` §2
    /// Trade, Qty).
    pub moved_out: Fig<Dec>,
    /// Every opening, in order: the candidates for its trade id.
    pub openings: Vec<TripKey>,
}

/// Units a transaction took out that the journal's own lots did not hold: the
/// activity feed never showed them arriving (a transfer in, a corporate action
/// it does not spell out, a coin's rounding). Wealthsimple's positions read of
/// the day before says what they were (brief 79, amended 2026-10-28).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Beyond {
    pub transaction: TransactionId,
    pub account: AccountId,
    pub instrument: InstrumentId,
    /// The units the journal's lots did not hold.
    pub qty: Dec,
    pub read: StatedRead,
}

/// What Wealthsimple's positions read of the day before a transaction says of
/// units the journal did not hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatedRead {
    /// That day's read states them: a lot of them was opened from it (`Flag::Stated`).
    Opened(Date),
    /// That day is not read yet: the holding waits on its read.
    Unread(Date),
    /// No read states them: the account is one no holdings are read of, or the
    /// read states fewer. They have no cost on record.
    Unstated,
}

impl StatedRead {
    /// The day of the read it rests on, where it rests on one.
    pub fn day(self) -> Option<Date> {
        match self {
            StatedRead::Opened(d) | StatedRead::Unread(d) => Some(d),
            StatedRead::Unstated => None,
        }
    }
}

/// An amount of a coin that was dust (`crate::dust`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dust {
    pub transaction: TransactionId,
    pub account: AccountId,
    pub instrument: InstrumentId,
    /// Always positive.
    pub qty: Dec,
    /// Taken beyond what was held (else written off from what was held).
    pub beyond: bool,
}

/// Units of a coin that arrived with no row of their own, at no cost: what a
/// row's rounding left out, or the last reward paid into a move out of staking
/// (`SPEC.md` §2, Crypto).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrived {
    /// The row that shows they were held.
    pub transaction: TransactionId,
    pub account: AccountId,
    pub instrument: InstrumentId,
    /// Always positive.
    pub qty: Dec,
    /// A reward paid into a move out of staking (else what rounding left out).
    pub reward: bool,
}

/// An event's units as the broker states them against its ratio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitsDisagree {
    pub transaction: TransactionId,
    pub account: AccountId,
    pub instrument: InstrumentId,
    pub stated: Dec,
    pub by_ratio: Dec,
}

/// Units of the issuer paid as a dividend: income at the adjustment's value.
#[derive(Clone, Debug, PartialEq)]
pub struct StockDividend {
    pub transaction: TransactionId,
    pub account: AccountId,
    pub instrument: InstrumentId,
    pub day: Date,
    pub units: Dec,
    pub value: Money,
}

/// The average cost of a holding's open units one way (`SPEC.md` §1 Position:
/// Book), as Wealthsimple, IBKR and the CRA state it: kept beside the lots by
/// the same code that changes them, never worked out again from them.
#[derive(Clone, Debug, PartialEq)]
pub struct Basis {
    /// What the open units cost (a long) or brought in (a short), fees in: an
    /// opening adds its value and its fee (a short's value less it), a removal
    /// takes its share by units, an event lowers or moves it. Nothing once the
    /// holding is flat.
    pub cost: Fig<Money>,
    /// What the holding has realized at average cost over its life: each
    /// close's proceeds less its fee and the cost it took off, and capital
    /// returned beyond the cost. No screen shows it, since a trade's P&L is
    /// matched first in first out (`SPEC.md` §1); over a holding that ends flat
    /// the two come to one total, which holds this to the lots.
    pub realized: Fig<Money>,
}

/// The lots of one account in one instrument.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Book {
    pub lots: VecDeque<Lot>,
    /// Each direction's average cost, from its first opening.
    pub basis: BTreeMap<Direction, Basis>,
    /// What this holding has been waiting on since a transaction it could not
    /// apply: every figure matched in it from then on carries it.
    pub taint: Gaps,
    /// The trip bought lots open into while any are open.
    trip: Option<TripKey>,
    /// The trip deposited lots open into while any are open.
    deposit_trip: Option<TripKey>,
}

/// What `close` left unmet.
#[derive(Clone, Debug)]
struct Closed {
    /// The quantity no open lot met.
    left: Dec,
    /// The part of the closing value and fee that is the unmet quantity's.
    value_left: Fig<Money>,
    fee_left: Money,
}

/// A holding's long cost as it stood before an event.
#[derive(Clone, Debug)]
struct Before {
    /// Each long lot's value, oldest first.
    lots: Vec<Fig<Money>>,
    /// The average cost of them all.
    cost: Fig<Money>,
}

/// What `take` took off a holding.
#[derive(Clone, Debug)]
struct Taken {
    lots: Vec<Lot>,
    /// The quantity no open lot met.
    left: Dec,
    /// The share by units of the holding's average cost the lots took with them.
    cost: Fig<Money>,
}

/// The whole match.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Matched {
    pub trips: BTreeMap<TripKey, Trip>,
    pub books: BTreeMap<(AccountId, InstrumentId), Book>,
    pub beyond: Vec<Beyond>,
    /// Every amount of a coin that was dust (`crate::dust`): written off, or
    /// taken beyond what was held and not listed.
    pub dust: Vec<Dust>,
    /// Every amount of a coin that arrived with no row of its own.
    pub arrived: Vec<Arrived>,
    /// Each instrument's last price a fill stated above nothing, in its own
    /// currency: what dust is valued at where nothing newer is known.
    pub last_price: BTreeMap<InstrumentId, Dec>,
    pub stock_dividends: Vec<StockDividend>,
    /// Fills whose stated price and cash disagree: the cash stands, and the
    /// record is a problem for the person.
    pub disagreements: Vec<TransactionId>,
    /// Events whose units as the broker states them differ from what the
    /// adjustment's ratio gives: the broker's units stand, and the record is a
    /// problem for the person.
    pub unit_disagreements: Vec<UnitsDisagree>,
    /// Transactions that moved nothing, and what they wait on.
    pub unapplied: Vec<(TransactionId, Gaps)>,
    /// Each holding's units (longs less shorts) at the end of each day they
    /// changed; a count whose lots cannot be summed exactly is that failure.
    pub units: BTreeMap<(AccountId, InstrumentId), BTreeMap<Date, Fig<Dec>>>,
    /// What each fill did to its position: closed part of it, opened one, or both
    /// (a fill that crossed through zero).
    pub roles: BTreeMap<TransactionId, FillRole>,
    /// What waits on the person (`SPEC.md` §2, What you enter), by the transaction
    /// it is about: units that arrived with no cost on the record, and corporate
    /// events nothing says the values of.
    pub waiting: BTreeMap<TransactionId, Waiting>,
    /// Each instrument's round trips (by the instrument each is keyed on): what a
    /// family's figures are rebuilt from without a pass over every round trip.
    pub trips_of: BTreeMap<InstrumentId, BTreeSet<TripKey>>,
    /// The round trips each transaction is a fill of: what a trade anchored on a
    /// fill is found a round trip by, without a pass over every round trip.
    pub trips_by_fill: BTreeMap<TransactionId, BTreeSet<TripKey>>,
    /// The accounts holding a book or a count of units of each instrument.
    pub held_by: BTreeMap<InstrumentId, BTreeSet<AccountId>>,
}

/// A fact only the person can give, and what it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub what: Wanted,
    pub account: AccountId,
    pub instrument: InstrumentId,
    pub day: Date,
    /// The units the transaction moved, as the broker states them.
    pub units: Option<Dec>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wanted {
    /// What the units cost, in total, and the day they were acquired.
    CostOfArrival,
    /// What the event did to cost: each new holding's share, or capital returned.
    Event,
}

/// What a fill did to its position, as the match found it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FillRole {
    pub closed: bool,
    pub opened: bool,
}

impl Matched {
    /// Units of a holding at the end of `day`.
    pub fn units_on(&self, account: AccountId, instrument: InstrumentId, day: Date) -> Fig<Dec> {
        self.units.get(&(account, instrument)).and_then(|t| t.range(..=day).next_back()).map(|(_, q)| q.clone()).unwrap_or(Ok(Dec::ZERO))
    }
}

/// `total × part ÷ whole`, to `SHARE_PLACES`; the whole when the part is all of it.
pub fn share(total: Money, part: Dec, whole: Dec) -> Result<Money, Gaps> {
    Ok(Money::new(share_dec(total.amount, part, whole)?, total.currency))
}

fn share_dec(total: Dec, part: Dec, whole: Dec) -> Result<Dec, Gaps> {
    if part == whole {
        return Ok(total);
    }
    // worked exactly however many digits the product needs (a coin's millions of units
    // at a fraction of a cent), rounded once to the share's places
    Ok(total.mul_div_rounded(part, whole, SHARE_PLACES, Rounding::HalfEven)?)
}

fn fig_share(total: &Fig<Money>, part: Dec, whole: Dec) -> Fig<Money> {
    match total {
        Ok(t) => share(*t, part, whole),
        Err(g) => Err(g.clone()),
    }
}

fn fig_sub(a: &Fig<Money>, b: &Fig<Money>) -> Fig<Money> {
    match (a, b) {
        (Ok(a), Ok(b)) => Ok(a.checked_sub(*b)?),
        (Err(g), _) | (_, Err(g)) => Err(g.clone()),
    }
}

/// What an opening adds to its holding's cost: a long's value and fee, a
/// short's value less its fee.
fn opening_cost(value: &Fig<Money>, fee: Money, direction: Direction) -> Fig<Money> {
    let v = value.clone()?;
    Ok(match direction {
        Direction::Long => v.add_to_fit(fee)?,
        Direction::Short => v.checked_sub(fee)?,
    })
}

fn fig_add(a: &Fig<Money>, b: &Fig<Money>) -> Fig<Money> {
    crate::gap::both(a.clone(), b.clone(), |a, b| Ok(a.add_to_fit(b)?))
}

fn dec_sum(items: impl IntoIterator<Item = Dec>) -> Result<Dec, Gaps> {
    items.into_iter().try_fold(Dec::ZERO, |a, b| a.checked_add(b).map_err(Gaps::from))
}

/// Shares per unit: a contract's stated multiplier, 1 for anything else.
pub fn multiplier(info: Option<&InstrumentInfo>, instrument: InstrumentId) -> Fig<Dec> {
    match info {
        Some(i) if i.instrument.kind == InstrumentKind::OptionContract => {
            i.terms.as_ref().and_then(|t| t.multiplier).ok_or_else(|| Gaps::of(Gap::MultiplierUnstated(instrument)))
        }
        _ => Ok(Dec::ONE),
    }
}

/// The value of a fill of `qty` units: its cash less its fee, since the cash is
/// what moved; the stated price × quantity × multiplier only where no cash is
/// stated (a fill the app booked itself). `acquiring` says which way the cash
/// goes. Where both are stated and disagree, `price_disagrees` says so. A fill
/// that states neither has no value on the record.
pub fn fill_value(t: &Transaction, qty: Dec, acquiring: bool, currency: Currency, mult: &Fig<Dec>) -> Fig<Money> {
    let Some(cash) = t.cash else {
        return match t.price {
            Some(price) if price.currency != currency => Err(Gaps::of(Gap::CurrencyUnstated(t.id.clone()))),
            Some(price) => Ok(Money::new(price.amount.checked_mul(qty)?.checked_mul(mult.clone()?)?, currency)),
            None => Err(Gaps::of(Gap::ValueUnstated(t.id.clone()))),
        };
    };
    let fee = fee_in(t, currency)?.amount;
    let cash = in_currency(t, cash, currency)?;
    let v = if acquiring { cash.neg().checked_sub(fee)? } else { cash.checked_add(fee)? };
    if v.is_negative() {
        return Err(Gaps::of(Gap::Arithmetic(format!("{} moves its cash against its direction", t.id))));
    }
    Ok(Money::new(v, currency))
}

/// A fill's price a unit, in the instrument's currency: as the broker states it,
/// else its value (`fill_value`) over its units, quantity × multiplier, as a
/// trade's entry is. A synced row states no price of its own.
pub fn fill_price(t: &Transaction, info: Option<&InstrumentInfo>) -> Fig<Dec> {
    let Some(i) = info else { return Err(Gaps::of(Gap::ValueUnstated(t.id.clone()))) };
    if let Some(p) = t.price.filter(|p| p.currency == i.instrument.currency) {
        return Ok(p.amount);
    }
    let q = t.quantity.filter(|q| !q.is_zero()).ok_or_else(|| Gaps::of(Gap::QuantityUnstated(t.id.clone())))?;
    let units = q.abs();
    let mult = multiplier(Some(i), i.instrument.id);
    // a row that moves no cash and states no price but states what its units were
    // worth (a coin moved in, a reward): that worth over its units
    if t.cash.is_none() {
        if let Some(v) = t.value.filter(|v| v.currency == i.instrument.currency) {
            return Ok(v.amount.abs().div_rounded(units.checked_mul(mult?)?, crate::trades::PRICE_PLACES, Rounding::HalfEven)?);
        }
    }
    let value = fill_value(t, units, q.is_positive(), i.instrument.currency, &mult)?;
    Ok(value.amount.div_rounded(units.checked_mul(mult?)?, crate::trades::PRICE_PLACES, Rounding::HalfEven)?)
}

/// An amount of a fill in the instrument's currency: as it is, or at the rate
/// the source states it applied (the amount's currency per unit of the
/// instrument's); without a stated rate, the currency is unstated.
fn in_currency(t: &Transaction, amount: Money, currency: Currency) -> Result<Dec, Gaps> {
    if amount.currency == currency {
        return Ok(amount.amount);
    }
    let converts = t.cash.is_some_and(|c| c.currency == amount.currency);
    match t.fx_rate {
        Some(r) if converts && !r.is_zero() => Ok(amount.amount.div_rounded(r, SHARE_PLACES, Rounding::HalfEven)?),
        _ => Err(Gaps::of(Gap::CurrencyUnstated(t.id.clone()))),
    }
}

/// A fill's fee in the instrument's currency (none stated is none charged).
fn fee_in(t: &Transaction, currency: Currency) -> Result<Money, Gaps> {
    match t.fee {
        None => Ok(Money::zero(currency)),
        Some(f) => Ok(Money::new(in_currency(t, f, currency)?, currency)),
    }
}

/// A fill's fee in the instrument's currency, and what its value becomes with
/// it: as it is where the fee is stated in that currency (or converts at the
/// stated rate), unstated where it is not, the fee then standing at zero.
fn charged(t: &Transaction, currency: Currency) -> (Money, impl Fn(Fig<Money>) -> Fig<Money>) {
    let fee = fee_in(t, currency);
    let stands = fee.clone().unwrap_or(Money::zero(currency));
    (stands, move |value: Fig<Money>| match (&fee, value) {
        (Ok(_), v) => v,
        (Err(g), Ok(_)) => Err(g.clone()),
        (Err(g), Err(mut v)) => {
            v.merge(g);
            Err(v)
        }
    })
}

/// Whether a fill states both a price and cash that do not agree: price ×
/// quantity × multiplier against the cash less the fee, at the cash's own
/// places, since the broker rounds the cash to its minor unit. Cash converted
/// at a stated rate disagrees only where no cash, fee and rate that round to the
/// stated ones give the price: the rate is itself rounded, so a difference it
/// explains is not one. Unknown (no multiplier, no value) is not a disagreement.
pub fn price_disagrees(t: &Transaction, qty: Dec, acquiring: bool, currency: Currency, mult: &Fig<Dec>) -> bool {
    let (Some(price), Some(cash)) = (t.price, t.cash) else { return false };
    if price.currency != currency {
        return false;
    }
    let Ok(m) = mult else { return false };
    let Ok(by_price) = price.amount.checked_mul(qty).and_then(|v| v.checked_mul(*m)) else { return true };
    if cash.currency != currency {
        return converted_disagrees(t, qty, acquiring, currency, mult, by_price);
    }
    if t.fee.is_some_and(|f| f.currency != currency) {
        return false;
    }
    let Ok(by_cash) = fill_value(t, qty, acquiring, currency, mult) else { return false };
    let places = currency.minor_units().max(cash.amount.places()).max(t.fee.map(|f| f.amount.places()).unwrap_or(0));
    by_price.round(places, Rounding::HalfEven) != by_cash.amount
}

/// Half a unit in the last place `d` is written to (at least the currency's
/// minor unit for an amount).
/// A value at every place a `Dec` holds has no rounding below it to speak of.
fn half_unit(d: Dec, places: u32) -> Dec {
    Dec::new(5, places.max(d.places()) + 1).unwrap_or(Dec::ZERO)
}

/// A converted fill's value over every cash, fee and rate that round to the
/// stated ones: the price disagrees when it lies outside all of them.
fn converted_disagrees(t: &Transaction, qty: Dec, acquiring: bool, currency: Currency, mult: &Fig<Dec>, by_price: Dec) -> bool {
    let (Some(cash), Some(rate)) = (t.cash, t.fx_rate) else { return false };
    let spread = |d: Dec, places: u32| -> Option<[Dec; 2]> {
        let h = half_unit(d, places);
        Some([d.checked_sub(h).ok()?, d.checked_add(h).ok()?])
    };
    let Some(rates) = spread(rate, 0) else { return false };
    if !rates[0].is_positive() {
        return false;
    }
    let Some(cashes) = spread(cash.amount, cash.currency.minor_units()) else { return false };
    let fees: Vec<Option<Money>> = match t.fee {
        None => vec![None],
        Some(f) => match spread(f.amount, f.currency.minor_units()) {
            Some(fs) => fs.iter().map(|a| Some(Money::new(*a, f.currency))).collect(),
            None => return false,
        },
    };
    let mut values = Vec::new();
    for r in rates {
        for c in cashes {
            for fee in &fees {
                let mut v = t.clone();
                v.fx_rate = Some(r);
                v.cash = Some(Money::new(c, cash.currency));
                v.fee = *fee;
                match fill_value(&v, qty, acquiring, currency, mult) {
                    Ok(value) => values.push(value.amount),
                    Err(_) => return false,
                }
            }
        }
    }
    let (Some(lo), Some(hi)) = (values.iter().min(), values.iter().max()) else { return false };
    by_price < *lo || by_price > *hi
}

/// What one transaction does to a holding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Move {
    /// A buy or a sale, as the record states its effect (options) or not.
    Trade(Option<Effect>),
    Expire,
    /// An assignment or an exercise: the contract closes at zero and the
    /// underlying moves.
    Deliver { exercise: bool },
    /// An asset deposited (quantity positive) or withdrawn (negative).
    Transfer,
    Reward,
    /// An event contract settling at the cash stated.
    Resolve,
    Event,
    Nothing,
}

fn read_move(t: &Transaction) -> Move {
    match t.kind {
        Kind::Buy | Kind::Sell => Move::Trade(t.effect),
        Kind::OptionExpiry => Move::Expire,
        Kind::OptionAssignment => Move::Deliver { exercise: false },
        Kind::OptionExercise => Move::Deliver { exercise: true },
        Kind::TransferIn | Kind::TransferOut => Move::Transfer,
        Kind::StakingReward => Move::Reward,
        Kind::Resolution => Move::Resolve,
        Kind::CorporateEvent => Move::Event,
        _ => Move::Nothing,
    }
}

/// Whether a transaction brings units into the account (`true`) or takes them
/// out; `None` when it states no quantity.
fn acquires(t: &Transaction) -> Option<bool> {
    match t.kind {
        Kind::Buy => Some(true),
        Kind::Sell => Some(false),
        _ => t.quantity.filter(|q| !q.is_zero()).map(|q| q.is_positive()),
    }
}

/// The order transactions are applied in (`docs/plans/stage-2-engine.md`, "The
/// order of transactions").
///
/// - the day the broker files it under;
/// - a transaction stating only its day that brings units in comes before the
///   day's timed ones, and one that takes units out after them;
/// - then the instant;
/// - at one instant, records by what they do: one that states it opens a
///   position, then one that brings units in, then one that takes units out,
///   then one that states it closes (a position cannot close before it opens);
///   except that a stated close meeting a position goes before a stated open of
///   the same contract at that instant (`match_lots`), which would otherwise meet
///   the position the close takes off;
/// - then the record's source key, never when it was stored;
/// - within a record, its closing legs before its opening ones (a roll closes
///   the old contract, then opens the new), then the legs' content and name.
fn order_key(t: &Transaction, record_rank: u8, source_key: &str) -> (Date, u8, Option<Timestamp>, u8, String, u8, String) {
    let (_, bucket, _) = instant(t);
    let leg_rank = match t.effect {
        Some(Effect::Close) => 0,
        None => 1,
        Some(Effect::Open) => 2,
    };
    let content = format!(
        "{}|{}|{}|{}|{}",
        t.kind.as_str(),
        t.instrument.map(|i| i.to_string()).unwrap_or_default(),
        t.quantity.map(|q| q.to_text()).unwrap_or_default(),
        t.cash.map(|c| c.amount.to_text()).unwrap_or_default(),
        t.id.leg.as_str()
    );
    (t.trade_date, bucket, t.occurred_at, record_rank, source_key.to_string(), leg_rank, content)
}

/// A row's place in the walk (`OrderKey`), ranked by its record's legs `legs`.
pub fn order_key_of(t: &Transaction, legs: &[&Transaction], source_key: &str) -> OrderKey {
    order_key(t, record_rank(legs), source_key)
}

/// The instant a transaction is applied at: its day, whether it states only its
/// day (bringing units in, or not), and its time.
fn instant(t: &Transaction) -> (Date, u8, Option<Timestamp>) {
    let bucket = match (t.occurred_at, acquires(t)) {
        (None, Some(true)) => 0,
        (Some(_), _) => 1,
        (None, _) => 2,
    };
    (t.trade_date, bucket, t.occurred_at)
}

/// A record's place among records at one instant (`order_key`).
fn record_rank(legs: &[&Transaction]) -> u8 {
    if legs.iter().any(|l| l.effect == Some(Effect::Open)) {
        0
    } else if legs.iter().any(|l| l.effect.is_none() && acquires(l) == Some(true)) {
        1
    } else if legs.iter().any(|l| l.effect.is_none()) {
        2
    } else {
        3
    }
}

struct Matcher<'a, 'm> {
    inputs: &'a Inputs,
    /// What the match writes: a fresh one for a family matched whole, the
    /// engine's own for a family's match continued (`continue_family`).
    out: &'m mut Matched,
    /// The round trips written, and the holdings touched: what a continued
    /// match's figures are worked out for.
    touched_trips: BTreeSet<TripKey>,
    touched_holdings: BTreeSet<(AccountId, InstrumentId)>,
    /// The round trips each transaction closed lots of, in this walk: what a
    /// roll's other leg is found by.
    closed_by: BTreeMap<TransactionId, BTreeSet<TripKey>>,
    /// Contracts with open lots, by expiry.
    expiring: BTreeMap<Date, BTreeSet<(AccountId, InstrumentId)>>,
    /// What every contract on an underlying in an account waits on: a multi-leg
    /// order whose legs are not stated.
    underlying_taint: BTreeMap<(AccountId, InstrumentId), Gaps>,
    /// What a share waits on: a multi-leg order naming it whose legs are not
    /// stated (its contracts wait through `underlying_taint`).
    instrument_taint: BTreeMap<(AccountId, InstrumentId), Gaps>,
    /// What every holding in an account waits on: a multi-leg order naming no
    /// instrument, whose legs may be anything the account holds.
    account_taint: BTreeMap<AccountId, Gaps>,
    /// Transactions already applied as part of another (an event's rows, a
    /// linked transfer's receiving side).
    consumed: BTreeSet<TransactionId>,
    /// Each transfer out's linked receiving side.
    links_out: BTreeMap<TransactionId, TransactionId>,
    links_in: BTreeSet<TransactionId>,
    /// Each event row, the adjustment it is explained by.
    event_groups: BTreeMap<TransactionId, TransactionId>,
    /// A split's fraction of a unit the broker paid in cash, for the event's
    /// cash leg.
    lieu_fraction: BTreeMap<(AccountId, InstrumentId), Dec>,
    /// Holdings touched since their units were last recorded.
    dirty: BTreeSet<(AccountId, InstrumentId)>,
    /// Contracts whose record has an expiry, assignment or exercise row, on any
    /// day: the broker's own row says how they ended.
    ended_on_record: BTreeSet<(AccountId, InstrumentId)>,
    /// Each coin's finest place each source states its units to: a source
    /// rounds a row's units there, so a row's units are exact only to half a
    /// unit of that place.
    places: BTreeMap<(InstrumentId, bagholder_core::SourceName), u32>,
    /// Each coin holding: how far its rows since it was last empty can be from
    /// the units they moved, by their rounding alone.
    rounding_held: BTreeMap<(AccountId, InstrumentId), Dec>,
    /// The holdings with lots open, as of the last units recorded.
    open: BTreeSet<(AccountId, InstrumentId)>,
    /// The family's flat stretches (`MatchState::stretches`).
    stretches: BTreeMap<Option<OrderKey>, Option<OrderKey>>,
    /// The cuts kept at their starts (`MatchState::cuts`).
    cuts: BTreeMap<OrderKey, Cut>,
    /// The instruments of the family walked, where `out` holds others' too (a
    /// continued match writes the engine's own): what a cut keeps of `out`.
    scope: Option<BTreeSet<InstrumentId>>,
}

impl<'a, 'm> Matcher<'a, 'm> {
    fn info(&self, i: InstrumentId) -> Option<&'a InstrumentInfo> {
        self.inputs.ledger.instruments.get(&i)
    }

    fn currency(&self, i: InstrumentId) -> Currency {
        self.info(i).map(|x| x.instrument.currency).unwrap_or(Currency::CAD)
    }

    fn underlying(&self, i: InstrumentId) -> Option<InstrumentId> {
        self.info(i).and_then(|x| x.terms.as_ref()).map(|t| t.underlying)
    }

    fn is_option(&self, i: InstrumentId) -> bool {
        self.info(i).is_some_and(|x| x.instrument.kind == InstrumentKind::OptionContract)
    }

    fn is_coin(&self, i: InstrumentId) -> bool {
        self.info(i).is_some_and(|x| x.instrument.kind == InstrumentKind::Crypto)
    }

    /// The long units the book holds.
    fn long_held(&mut self, account: AccountId, instrument: InstrumentId) -> Option<Dec> {
        self.book(account, instrument).lots.iter().filter(|l| l.direction == Direction::Long).try_fold(Dec::ZERO, |a, l| a.checked_add(l.qty).ok())
    }

    /// How far the book's units of a coin can be from what the broker holds by
    /// its rows' rounding alone: half a unit of the finest place its rows state,
    /// for each row since the holding was last empty.
    fn rounding(&self, t: &Transaction, instrument: InstrumentId) -> Option<Dec> {
        let held = self.rounding_held.get(&(t.account, instrument)).copied().unwrap_or(Dec::ZERO);
        held.checked_add(self.half_unit(t, instrument)?).ok()
    }

    /// Half a unit of the place the row's source states the coin's units to.
    fn half_unit(&self, t: &Transaction, instrument: InstrumentId) -> Option<Dec> {
        let places = self.places.get(&(instrument, t.mapping.source.clone()))?;
        Dec::new(5, places.checked_add(1)?).ok()
    }

    /// Units of a coin a row states the account holds (a move of the whole
    /// holding into or out of staking) or takes out (a sale), beyond what the
    /// book holds: they arrived with no row, and arrive now, at no cost. Within
    /// the rows' rounding they are what the rounding left out; beyond it, a
    /// row stating units in staking brings the last reward its rows left out
    /// (a recorded Polkadot account, 4 November 2024: unstaked a day's reward more than
    /// every reward row states). Anything else stays beyond what the book held.
    fn arrived_unstated(&mut self, t: &Transaction, instrument: InstrumentId, stated: Dec, staking: bool) {
        let account = t.account;
        let Some(held) = self.long_held(account, instrument) else { return };
        let Ok(excess) = stated.checked_sub(held) else { return };
        if !excess.is_positive() {
            return;
        }
        let within = self.rounding(t, instrument).is_some_and(|r| excess <= r);
        let flags = match (within, staking) {
            (true, _) => BTreeSet::new(),
            (false, true) => BTreeSet::from([Flag::Reward]),
            (false, false) => return,
        };
        let zero = Money::zero(self.currency(instrument));
        let reward = flags.contains(&Flag::Reward);
        // what the rounding left out belongs to the units it was left out of: the round
        // trip of the last lot held, a deposited one's too, never a round trip of its own
        let mut flags = flags;
        let joins = if within {
            let last = self.book(account, instrument).lots.iter().rev().find(|l| l.direction == Direction::Long).map(|l| (l.trip.clone(), l.flags.contains(&Flag::Deposited)));
            last.map(|(trip, deposited)| {
                if deposited {
                    flags.insert(Flag::Deposited);
                }
                trip
            })
        } else {
            None
        };
        self.open_lot(account, instrument, &t.id, t.trade_date, t.occurred_at, Direction::Long, excess, Ok(zero), zero, flags, joins);
        self.out.arrived.push(Arrived { transaction: t.id.clone(), account, instrument, qty: excess, reward });
    }

    /// A row that moved a coin holding: half a unit of the place its source
    /// states units to, added to the holding's rounding, which begins again
    /// once the holding is empty.
    fn count_row(&mut self, t: &Transaction, instrument: InstrumentId) {
        let account = t.account;
        if !self.is_coin(instrument) {
            return;
        }
        if self.long_held(account, instrument).is_some_and(|h| h.is_zero()) {
            self.rounding_held.remove(&(account, instrument));
            return;
        }
        let Some(half) = self.half_unit(t, instrument) else { return };
        let e = self.rounding_held.entry((account, instrument)).or_insert(Dec::ZERO);
        if let Ok(v) = e.checked_add(half) {
            *e = v;
        }
    }

    fn book(&mut self, account: AccountId, instrument: InstrumentId) -> &mut Book {
        let under = self.underlying(instrument).and_then(|u| self.underlying_taint.get(&(account, u)).cloned());
        let own = self.instrument_taint.get(&(account, instrument)).cloned();
        let whole = self.account_taint.get(&account).cloned();
        self.dirty.insert((account, instrument));
        self.touched_holdings.insert((account, instrument));
        let book = self.out.books.entry((account, instrument)).or_default();
        for g in [under, own, whole].into_iter().flatten() {
            book.taint.merge(&g);
        }
        book
    }

    fn trip_mut(&mut self, key: &TripKey) -> &mut Trip {
        self.touched_trips.insert(key.clone());
        self.out.trips.get_mut(key).expect("a lot's round trip exists")
    }

    /// The round trip a new lot joins: the one given (a roll), else the one open
    /// in this holding, else a new one keyed by this opening.
    fn trip_for(&mut self, account: AccountId, instrument: InstrumentId, direction: Direction, deposited: bool, opening: &TransactionId, joins: Option<TripKey>) -> TripKey {
        let book = self.book(account, instrument);
        let current = if deposited { book.deposit_trip.clone() } else { book.trip.clone() };
        // a round trip is one direction: an opening against the other (a
        // conflict on the record) is a round trip of its own
        let current = current.filter(|k| self.out.trips.get(k).is_none_or(|t| t.direction == direction));
        let this = TripKey { opening: opening.clone(), instrument };
        let key = joins.or(current).unwrap_or_else(|| this.clone());
        // a round trip given by the caller (a roll's continuation, or a round
        // trip of its own) becomes the holding's current one only when it has none:
        // what is bought later joins the round trip already open
        let book = self.book(account, instrument);
        let slot = if deposited { &mut book.deposit_trip } else { &mut book.trip };
        if slot.is_none() {
            *slot = Some(key.clone());
        }
        self.touched_trips.insert(key.clone());
        let trip = self.out.trips.entry(key.clone()).or_insert_with(|| Trip {
            key: key.clone(),
            account,
            direction,
            instruments: Vec::new(),
            slices: Vec::new(),
            fills: BTreeSet::new(),
            opened: BTreeSet::new(),
            closed: BTreeSet::new(),
            open_lots: 0,
            moved_out: Ok(Dec::ZERO),
            openings: Vec::new(),
        });
        trip.account = account;
        if !trip.instruments.contains(&instrument) {
            trip.instruments.push(instrument);
        }
        if !trip.openings.contains(&this) {
            trip.openings.push(this);
        }
        key
    }

    #[allow(clippy::too_many_arguments)]
    /// Whether what explains `t` is the person's own entry.
    fn entered(&self, t: &TransactionId) -> bool {
        self.inputs.facts.adjustments.get(t).is_some_and(|a| a.source == bagholder_core::SourceName::person())
    }

    /// A lot opened, its value and fee added to the holding's cost.
    #[allow(clippy::too_many_arguments)]
    fn open_lot(&mut self, account: AccountId, instrument: InstrumentId, opened_by: &TransactionId, day: Date, at: Option<Timestamp>, direction: Direction, qty: Dec, value: Fig<Money>, fee: Money, flags: BTreeSet<Flag>, joins: Option<TripKey>) {
        let cost = opening_cost(&value, fee, direction);
        self.push_lot(account, instrument, opened_by, day, at, direction, qty, value, fee, flags, joins);
        self.basis_add(account, instrument, direction, &cost);
    }

    /// A lot opened whose cost the caller adds to the holding's as a whole:
    /// units moved in with the average cost they left with, a spin-off's
    /// share of its parent's.
    #[allow(clippy::too_many_arguments)]
    fn push_lot(&mut self, account: AccountId, instrument: InstrumentId, opened_by: &TransactionId, day: Date, at: Option<Timestamp>, direction: Direction, qty: Dec, value: Fig<Money>, fee: Money, mut flags: BTreeSet<Flag>, joins: Option<TripKey>) {
        if self.entered(opened_by) {
            flags.insert(Flag::Entered);
        }
        let deposited = flags.contains(&Flag::Deposited);
        let trip = self.trip_for(account, instrument, direction, deposited, opened_by, joins);
        let lot = Lot { trip: trip.clone(), opened_by: opened_by.clone(), day, at, direction, qty, value, fee, flags };
        let book = self.book(account, instrument);
        // lots keep first in first out by the day they were bought: one moved in
        // from another account, or a spin-off's child, can be older than the last
        let older = book.lots.back().is_some_and(|last| (lot.day, lot.at) < (last.day, last.at));
        book.lots.push_back(lot);
        if older {
            book.lots.make_contiguous().sort_by(|a, b| (a.day, a.at).cmp(&(b.day, b.at)));
        }
        let tr = self.trip_mut(&trip);
        tr.open_lots += 1;
        tr.fills.insert(opened_by.clone());
        tr.opened.insert(opened_by.clone());
        self.watch_expiry(account, instrument);
    }

    fn watch_expiry(&mut self, account: AccountId, instrument: InstrumentId) {
        if let Some(expiry) = self.info(instrument).and_then(|i| i.terms.as_ref()).map(|t| t.expiry) {
            self.expiring.entry(expiry).or_default().insert((account, instrument));
        }
    }

    /// The units a holding holds one way.
    fn held(&mut self, account: AccountId, instrument: InstrumentId, direction: Direction) -> Result<Dec, Gaps> {
        dec_sum(self.book(account, instrument).lots.iter().filter(|l| l.direction == direction).map(|l| l.qty))
    }

    fn basis_mut(&mut self, account: AccountId, instrument: InstrumentId, direction: Direction) -> &mut Basis {
        let zero = Money::zero(self.currency(instrument));
        self.book(account, instrument).basis.entry(direction).or_insert(Basis { cost: Ok(zero), realized: Ok(zero) })
    }

    /// Cost added to a holding's.
    fn basis_add(&mut self, account: AccountId, instrument: InstrumentId, direction: Direction, cost: &Fig<Money>) {
        let b = self.basis_mut(account, instrument, direction);
        b.cost = fig_add(&b.cost, cost);
    }

    /// The share by units of a holding's cost that `n` of its `held` units
    /// take with them, taken off it: all of it when they are all it holds.
    fn basis_take(&mut self, account: AccountId, instrument: InstrumentId, direction: Direction, n: Dec, held: Dec) -> Fig<Money> {
        let b = self.basis_mut(account, instrument, direction);
        if !n.is_positive() || !held.is_positive() {
            return Ok(Money::zero(self.currency(instrument)));
        }
        let taken = fig_share(&b.cost, n, held);
        b.cost = fig_sub(&b.cost, &taken);
        taken
    }

    /// A gain realized at average cost.
    fn basis_realize(&mut self, account: AccountId, instrument: InstrumentId, direction: Direction, gain: &Fig<Money>) {
        let b = self.basis_mut(account, instrument, direction);
        b.realized = fig_add(&b.realized, gain);
    }

    /// The holding's current trips cleared once none of their lots remain, and
    /// the cost of a side it no longer holds back to nothing.
    fn settle(&mut self, account: AccountId, instrument: InstrumentId) {
        let zero = Money::zero(self.currency(instrument));
        let book = self.book(account, instrument);
        for (d, b) in book.basis.iter_mut() {
            if !book.lots.iter().any(|l| l.direction == *d) {
                b.cost = Ok(zero);
            }
        }
        if book.lots.iter().all(|l| l.flags.contains(&Flag::Deposited)) {
            book.trip = None;
        }
        if !book.lots.iter().any(|l| l.flags.contains(&Flag::Deposited)) {
            book.deposit_trip = None;
        }
    }

    /// Close up to `qty` of `direction` from the front of a holding, at the
    /// closing value `value` and fee `fee` for the whole `qty`; returns what was
    /// left unclosed with the part of the value and fee that is left to it.
    #[allow(clippy::too_many_arguments)]
    fn close(&mut self, account: AccountId, instrument: InstrumentId, direction: Direction, qty: Dec, value: Fig<Money>, fee: Money, closer: &Closer, day: Date, at: Option<Timestamp>, extra: &BTreeSet<Flag>) -> Result<Closed, Gaps> {
        let held = self.held(account, instrument, direction)?;
        let value_in = value.clone();
        let mut left = qty;
        let mut value_left = value;
        let mut fee_left = fee;
        loop {
            let book = self.book(account, instrument);
            let taint = book.taint.clone();
            let Some(front) = book.lots.front_mut() else { break };
            if !left.is_positive() || front.direction != direction {
                break;
            }
            let matched = if front.qty <= left { front.qty } else { left };
            let entry = fig_share(&front.value, matched, front.qty);
            let entry_fee = share(front.fee, matched, front.qty)?;
            let exit = fig_share(&value_left, matched, left);
            let exit_fee = share(fee_left, matched, left)?;
            let mut flags = front.flags.clone();
            flags.extend(extra.iter().copied());
            let slice = Slice {
                trip: front.trip.clone(),
                account,
                instrument,
                direction,
                qty: matched,
                opened_by: front.opened_by.clone(),
                opened_on: front.day,
                opened_at: front.at,
                closed_by: closer.clone(),
                closed_on: day,
                closed_at: at,
                entry: entry.clone(),
                exit: exit.clone(),
                entry_fee,
                exit_fee,
                flags,
                taint,
            };
            front.value = fig_sub(&front.value, &entry);
            front.fee = front.fee.checked_sub(entry_fee)?;
            front.qty = front.qty.checked_sub(matched)?;
            let emptied = front.qty.is_zero();
            let trip_key = front.trip.clone();
            if emptied {
                book.lots.pop_front();
            }
            value_left = fig_sub(&value_left, &exit);
            fee_left = fee_left.checked_sub(exit_fee)?;
            left = left.checked_sub(matched)?;
            if let Closer::Transaction(id) = closer {
                self.closed_by.entry(id.clone()).or_default().insert(trip_key.clone());
            }
            let tr = self.trip_mut(&trip_key);
            tr.slices.push(slice);
            if let Closer::Transaction(id) = closer {
                tr.fills.insert(id.clone());
                tr.closed.insert(id.clone());
            }
            if emptied {
                tr.open_lots -= 1;
            }
        }
        // at average cost: the closed units' share of the cost against what
        // closing them brought in (a long) or cost (a short), fees in
        let closed = qty.checked_sub(left)?;
        if closed.is_positive() {
            let cost = self.basis_take(account, instrument, direction, closed, held);
            let paid = fee.checked_sub(fee_left)?;
            let proceeds = fig_sub(&value_in, &value_left);
            let gain = crate::gap::both(proceeds, cost, |p, c| {
                Ok(match direction {
                    Direction::Long => p.checked_sub(paid)?.checked_sub(c)?,
                    Direction::Short => c.checked_sub(p)?.checked_sub(paid)?,
                })
            });
            self.basis_realize(account, instrument, direction, &gain);
        }
        self.settle(account, instrument);
        Ok(Closed { left, value_left, fee_left })
    }

    /// Lots that left their round trip with no sale: counted as units it opened.
    fn moved_out(&mut self, lots: &[Lot]) {
        for l in lots {
            let t = self.trip_mut(&l.trip);
            if let Ok(m) = t.moved_out {
                t.moved_out = m.checked_add(l.qty).map_err(Gaps::from);
            }
        }
    }

    /// Take `qty` of longs off the front of a holding with their cost and no
    /// P&L; returns the lots taken (for a linked transfer) and what could not be
    /// taken.
    fn take(&mut self, account: AccountId, instrument: InstrumentId, qty: Dec) -> Result<Taken, Gaps> {
        self.take_side(account, instrument, Direction::Long, qty)
    }

    /// Take `qty` of `direction`'s lots off the front of a holding, as `take`.
    fn take_side(&mut self, account: AccountId, instrument: InstrumentId, direction: Direction, qty: Dec) -> Result<Taken, Gaps> {
        let held = self.held(account, instrument, direction)?;
        let mut left = qty;
        let mut taken = Vec::new();
        let mut emptied = Vec::new();
        let book = self.book(account, instrument);
        while left.is_positive() {
            let Some(front) = book.lots.front_mut() else { break };
            if front.direction != direction {
                break;
            }
            let n = if front.qty <= left { front.qty } else { left };
            let value = fig_share(&front.value, n, front.qty);
            let fee = share(front.fee, n, front.qty)?;
            taken.push(Lot { qty: n, value: value.clone(), fee, ..front.clone() });
            front.value = fig_sub(&front.value, &value);
            front.fee = front.fee.checked_sub(fee)?;
            front.qty = front.qty.checked_sub(n)?;
            left = left.checked_sub(n)?;
            if front.qty.is_zero() {
                emptied.push(front.trip.clone());
                book.lots.pop_front();
            }
        }
        let cost = self.basis_take(account, instrument, direction, qty.checked_sub(left)?, held);
        self.settle(account, instrument);
        for trip in emptied {
            self.trip_mut(&trip).open_lots -= 1;
        }
        Ok(Taken { lots: taken, left, cost })
    }

    /// Lots placed into a holding in date order, keeping their round trips.
    fn place(&mut self, account: AccountId, instrument: InstrumentId, lots: Vec<Lot>) {
        for lot in &lots {
            let tr = self.trip_mut(&lot.trip);
            tr.open_lots += 1;
            tr.account = account;
            if !tr.instruments.contains(&instrument) {
                tr.instruments.push(instrument);
            }
        }
        let book = self.book(account, instrument);
        for lot in &lots {
            if lot.flags.contains(&Flag::Deposited) {
                book.deposit_trip.get_or_insert(lot.trip.clone());
            } else {
                book.trip.get_or_insert(lot.trip.clone());
            }
        }
        let mut all: Vec<Lot> = book.lots.drain(..).chain(lots).collect();
        all.sort_by(|a, b| (a.day, a.at, a.opened_by.to_string()).cmp(&(b.day, b.at, b.opened_by.to_string())));
        book.lots = all.into();
        self.watch_expiry(account, instrument);
    }

    fn taint(&mut self, account: AccountId, instrument: InstrumentId, gaps: &Gaps) {
        self.book(account, instrument).taint.merge(gaps);
    }

    fn unapplied(&mut self, t: &Transaction, gaps: Gaps) {
        self.out.unapplied.push((t.id.clone(), gaps));
    }

    /// Whether the account holds lots of `direction` in the instrument.
    fn holds(&self, account: AccountId, instrument: InstrumentId, direction: Direction) -> bool {
        self.out.books.get(&(account, instrument)).is_some_and(|b| b.lots.iter().any(|l| l.direction == direction))
    }

    /// A transaction whose stated effect the position before it contradicts is
    /// not applied: the holding, and whatever else it would have moved, waits
    /// on it, as for a record that states no quantity.
    fn contradicts(&mut self, t: &Transaction, holdings: &[(AccountId, InstrumentId)]) {
        let gaps = Gaps::of(Gap::EffectConflict(t.id.clone()));
        for (account, instrument) in holdings {
            self.taint(*account, *instrument, &gaps);
        }
        self.unapplied(t, gaps);
    }

    /// A transaction took out more than was held: what it did close is waiting on
    /// what the rest was. The holding it emptied is flat afterwards, so nothing
    /// later waits on it.
    /// Units a sale or a move took beyond what the book held: listed for the broker
    /// check. The lots it did take keep their own cost and P&L; the units beyond
    /// have no cost on record, so they add nothing to P&L and none is made up for
    /// them (a sale's dust past the rows' rounding is cents, never a reason to lose
    /// the P&L of every unit that was held).
    /// The price a coin moved at in a transaction: its own, where it states one
    /// above nothing; else the last one a fill of the coin stated before it (a
    /// withdrawal states none; a write-off states nothing). Each price stated is
    /// kept as the coin's last.
    fn price_of(&mut self, t: &Transaction, instrument: InstrumentId) -> Option<Dec> {
        match fill_price(t, self.info(instrument)).ok().filter(|p| p.is_positive()) {
            Some(p) => {
                self.out.last_price.insert(instrument, p);
                Some(p)
            }
            None => self.out.last_price.get(&instrument).copied(),
        }
    }

    /// Units beyond worth less than the broker's smallest coin order at the
    /// transaction's own price are dust (`crate::dust`), and not listed.
    fn beyond(&mut self, t: &Transaction, account: AccountId, instrument: InstrumentId, qty: Dec) {
        if self.price_of(t, instrument).is_some_and(|p| crate::dust::is_dust(&crate::stated::Sources::of(self.inputs), account, instrument, qty, p, t.trade_date)) {
            return self.out.dust.push(Dust { transaction: t.id.clone(), account, instrument, qty, beyond: true });
        }
        self.out.beyond.push(Beyond { transaction: t.id.clone(), account, instrument, qty, read: StatedRead::Unstated });
    }

    /// What `qty` units arriving on `day` cost, as Wealthsimple's positions read
    /// states the holding (brief 79, amended 2026-10-28, point 5): its book value
    /// less the cost of the lots held, never below nothing, from the read of the day
    /// they arrived, or the day after where that day's does not state them yet (a
    /// row dated at a transfer's start). A day not read yet leaves the cost waiting
    /// on its read; an account no holdings are read of is none.
    fn cost_stated(&self, account: AccountId, instrument: InstrumentId, qty: Dec, day: Date) -> Option<(Fig<Money>, StatedRead)> {
        let stated = self.inputs.market.brokers.get(&account).filter(|b| !b.days_held.is_empty())?;
        let currency = self.currency(instrument);
        let lots: Vec<&Lot> = self.out.books.get(&(account, instrument)).map(|b| b.lots.iter().filter(|l| l.direction == Direction::Long).collect()).unwrap_or_default();
        let held = lots.iter().try_fold(Dec::ZERO, |a, l| a.checked_add(l.qty)).ok()?;
        let cost = lots.iter().try_fold(Money::zero(currency), |a, l| l.value.clone().and_then(|v| Ok(a.checked_add(v)?)));
        // a day read before its book value in the listing's currency was asked is not read yet
        let read_on = |d: Date| stated.days_held.get(&d).filter(|r| !r.book_unasked).map(|r| (d, r));
        let holds = |r: &&crate::input::HeldDay| r.held.get(&instrument).is_some_and(|u| held.checked_add(qty).is_ok_and(|w| *u >= w));
        let after = day.tomorrow().ok();
        // the arrival's own day where its read states them; else the day after, read;
        // else the first of the two not read yet (never a day read already)
        let Some((day, read)) = read_on(day).filter(|(_, r)| holds(r)).or_else(|| after.and_then(read_on)) else {
            let wait = if read_on(day).is_some() { after.unwrap_or(day) } else { day };
            return Some((Err(Gaps::of(Gap::HoldingsUnread(wait))), StatedRead::Unread(wait)));
        };
        let value: Fig<Money> = match (read.book.get(&instrument), cost) {
            (Some(book), Ok(cost)) => crate::fx::convert(&self.inputs.facts.rates, &self.inputs.clock, Money::new(book.amount.abs(), book.currency), currency, day).and_then(|b| {
                let left = b.checked_sub(cost.amount)?;
                Ok(Money::new(if left.is_negative() { Dec::ZERO } else { left }, currency))
            }),
            (None, _) => Err(Gaps::of(Gap::BookUnstated(instrument))),
            (_, Err(g)) => Err(g),
        };
        Some((value, StatedRead::Opened(day)))
    }

    /// Units `tx` takes out of a holding beyond the journal's own lots of it, where
    /// Wealthsimple's positions read of the day before states them held (brief 79,
    /// amended 2026-10-28, point 5): every unit the read states that the lots do not
    /// hold opens a lot, marked `Flag::Stated`, before the transaction takes them.
    /// Its cost is the read's book value for the holding less the cost of the lots
    /// held, never below nothing: a split the feed leaves out brings units at no
    /// cost, a transfer in the whole book value, a coin's rounding almost nothing.
    /// A day not read yet leaves the holding waiting on its read; an account no
    /// holdings are read of leaves the units beyond what was held, as before.
    #[allow(clippy::too_many_arguments)]
    fn stated_before(&mut self, tx: &TransactionId, day: Date, at: Option<Timestamp>, account: AccountId, instrument: InstrumentId, direction: Direction, qty: Dec) {
        let (held, cost, trip) = {
            let lots: Vec<&Lot> = self.out.books.get(&(account, instrument)).map(|b| b.lots.iter().filter(|l| l.direction == direction).collect()).unwrap_or_default();
            let held = lots.iter().try_fold(Dec::ZERO, |a, l| a.checked_add(l.qty));
            let cost = lots.iter().try_fold(Money::zero(self.currency(instrument)), |a, l| l.value.clone().and_then(|v| Ok(a.checked_add(v)?)));
            (held, cost, lots.first().map(|l| l.trip.clone()))
        };
        let Ok(held) = held else { return };
        if qty <= held {
            return;
        }
        let Some(stated) = self.inputs.market.brokers.get(&account).filter(|b| !b.days_held.is_empty()) else { return };
        let Ok(before) = day.yesterday() else { return };
        // a day read before its book value in the listing's currency was asked is not read yet
        let Some(read) = stated.days_held.get(&before).filter(|r| !r.book_unasked) else {
            // the units the transaction takes beyond the lots, their cost waiting on
            // that day's read: the round trip shows as a figure not read yet
            let Ok(missing) = qty.checked_sub(held) else { return };
            let currency = self.currency(instrument);
            self.out.beyond.push(Beyond { transaction: tx.clone(), account, instrument, qty: missing, read: StatedRead::Unread(before) });
            return self.open_lot(account, instrument, tx, before, at, direction, missing, Err(Gaps::of(Gap::HoldingsUnread(before))), Money::zero(currency), BTreeSet::from([Flag::Stated]), trip);
        };
        let units = read.held.get(&instrument).copied().unwrap_or(Dec::ZERO);
        let units = if direction == Direction::Short { units.neg() } else { units };
        let Ok(missing) = units.checked_sub(held) else { return };
        if !missing.is_positive() {
            return;
        }
        let currency = self.currency(instrument);
        let value: Fig<Money> = match (read.book.get(&instrument), cost) {
            (Some(book), Ok(cost)) => crate::fx::convert(&self.inputs.facts.rates, &self.inputs.clock, Money::new(book.amount.abs(), book.currency), currency, before).and_then(|b| {
                let left = b.checked_sub(cost.amount)?;
                Ok(Money::new(if left.is_negative() { Dec::ZERO } else { left }, currency))
            }),
            (None, _) => Err(Gaps::of(Gap::BookUnstated(instrument))),
            (_, Err(g)) => Err(g),
        };
        self.out.beyond.push(Beyond { transaction: tx.clone(), account, instrument, qty: missing, read: StatedRead::Opened(before) });
        self.open_lot(account, instrument, tx, before, at, direction, missing, value, Money::zero(currency), BTreeSet::from([Flag::Stated]), trip);
    }

    /// Every contract past its expiry before `day` with lots still open: closed
    /// at zero on its expiry date when its underlying's close that day shows it
    /// expired out of the money by its terms, otherwise left waiting for the
    /// broker's row.
    fn expire_before(&mut self, day: Date) -> Expired {
        let mut out = Expired::default();
        while let Some((&expiry, _)) = self.expiring.iter().next() {
            if expiry >= day {
                break;
            }
            let books = self.expiring.remove(&expiry).unwrap_or_default();
            out.popped.insert(expiry, books.clone());
            for (account, instrument) in books {
                let lots: Vec<(Direction, Dec)> = self.book(account, instrument).lots.iter().map(|l| (l.direction, l.qty)).collect();
                if lots.is_empty() {
                    continue;
                }
                // the record says how it ended, whenever the broker dated its row:
                // that row closes it, and nothing here stands in for it
                if self.ended_on_record.contains(&(account, instrument)) {
                    continue;
                }
                out.acted = true;
                if !self.expired_worthless(instrument, expiry) {
                    self.taint(account, instrument, &Gaps::of(Gap::NoExpiryRecord(instrument)));
                    continue;
                }
                let currency = self.currency(instrument);
                let flags = BTreeSet::from([Flag::NoExpiryRecord]);
                for (direction, qty) in lots {
                    // nothing is shared out of a zero value or fee, but the lot's own fee
                    // still is, and a share that does not fit is a gap
                    if let Err(g) = self.close(account, instrument, direction, qty, Ok(Money::zero(currency)), Money::zero(currency), &Closer::Expiry, expiry, None, &flags) {
                        self.taint(account, instrument, &g);
                    }
                }
            }
            self.record_units(expiry);
        }
        out
    }

    /// The units of every holding touched since the last record, at the end of `day`.
    fn record_units(&mut self, day: Date) {
        for key in std::mem::take(&mut self.dirty) {
            let net: Fig<Dec> = self.out.books.get(&key).map_or(Ok(Dec::ZERO), |b| {
                b.lots
                    .iter()
                    .try_fold(Dec::ZERO, |a, l| match l.direction {
                        Direction::Long => a.checked_add(l.qty),
                        Direction::Short => a.checked_sub(l.qty),
                    })
                    .map_err(Gaps::from)
            });
            if self.out.books.get(&key).is_some_and(|b| !b.lots.is_empty()) {
                self.open.insert(key);
            } else {
                self.open.remove(&key);
            }
            let timeline = self.out.units.entry(key).or_default();
            if timeline.values().next_back() != Some(&net) {
                timeline.insert(day, net);
            }
        }
    }

    /// The family's open stretch ended by a row at `at` that moves a lot.
    fn stretch_ends(&mut self, at: &OrderKey) {
        if let Some(mut last) = self.stretches.last_entry() {
            if last.get().is_none() {
                *last.get_mut() = Some(at.clone());
            }
        }
    }

    /// A stretch started after the row at `at`, where it left the family flat;
    /// `next` is the day of the next row walked. Where the start is a clean break
    /// a cut is kept there: no row of the family follows on its day, so its units
    /// entry is the day's and no event of that day is half applied. (Nothing
    /// walked later was applied before it: a linked move's units arrive at its
    /// send, so a holding would be open across it; and what an event leaves in
    /// the matcher's maps the cut keeps.)
    fn stretch_starts(&mut self, at: &OrderKey, next: Option<Date>) {
        let open_ended = self.stretches.last_key_value().is_some_and(|(_, end)| end.is_none());
        if !self.open.is_empty() || open_ended {
            return;
        }
        self.stretches.insert(Some(at.clone()), None);
        let day = at.0;
        if next != Some(day) {
            let cut = self.cut(day);
            self.cuts.insert(at.clone(), cut);
        }
    }

    /// What the match holds now, as a cut on `day`.
    fn cut(&self, day: Date) -> Cut {
        let mine = |i: &InstrumentId| self.scope.as_ref().is_none_or(|s| s.contains(i));
        let maps = MatchState {
            expiring: self.expiring.clone(),
            underlying_taint: self.underlying_taint.clone(),
            instrument_taint: self.instrument_taint.clone(),
            account_taint: self.account_taint.clone(),
            consumed: self.consumed.clone(),
            links_out: self.links_out.clone(),
            links_in: self.links_in.clone(),
            event_groups: self.event_groups.clone(),
            lieu_fraction: self.lieu_fraction.clone(),
            ended_on_record: self.ended_on_record.clone(),
            places: self.places.clone(),
            rounding_held: self.rounding_held.clone(),
            ..MatchState::default()
        };
        Cut {
            day,
            maps,
            books: self.out.books.iter().filter(|((_, i), _)| mine(i)).map(|(k, b)| (*k, (b.taint.clone(), b.basis.clone()))).collect(),
            units: self.out.units.iter().filter(|((_, i), _)| mine(i)).filter_map(|(k, u)| u.iter().next_back().map(|(d, v)| (*k, (*d, v.clone())))).collect(),
        }
    }

    /// Whether a contract's underlying closed on its expiry day where the
    /// contract is worth nothing: a call's underlying under its strike, a put's over.
    fn expired_worthless(&self, instrument: InstrumentId, expiry: Date) -> bool {
        let Some(terms) = self.info(instrument).and_then(|i| i.terms.as_ref()) else { return false };
        let Some(close) = self.inputs.market.closes.get(&terms.underlying).and_then(|c| c.get(&expiry)) else { return false };
        // the strike is in the contract's currency; a close in another says nothing here
        let Some(currency) = self.info(instrument).map(|i| i.instrument.currency) else { return false };
        if close.currency != currency {
            return false;
        }
        let close = &close.amount;
        // strictly out of the money: at the strike, whether it was exercised is
        // the broker's to say
        match terms.right {
            OptionRight::Call => *close < terms.strike,
            OptionRight::Put => *close > terms.strike,
        }
    }

    fn apply(&mut self, t: &Transaction, record_legs: &[&Transaction]) {
        if self.consumed.contains(&t.id) {
            return;
        }
        let account = t.account;
        let mv = read_move(t);
        let multi_leg = self.inputs.ledger.records.get(&t.id.record).is_some_and(|r| r.problems.iter().any(|p| p.code == "leg-unstated"));
        if multi_leg && mv != Move::Event {
            // a multi-leg order whose legs are not stated: anything it could
            // have moved waits, from its day on
            let gaps = Gaps::of(Gap::LegUnstated(t.id.clone()));
            let affected: Vec<InstrumentId> = match t.instrument {
                // any contract on the named contract's underlying, or on the
                // named share (and the share itself)
                Some(named) => {
                    let under = self.underlying(named);
                    let key = under.unwrap_or(named);
                    self.underlying_taint.entry((account, key)).or_default().merge(&gaps);
                    if under.is_none() {
                        self.instrument_taint.entry((account, named)).or_default().merge(&gaps);
                    }
                    let mut v: Vec<InstrumentId> = self.out.books.keys().filter(|(a, i)| *a == account && (self.underlying(*i) == Some(key) || (under.is_none() && *i == named))).map(|(_, i)| *i).collect();
                    v.push(named);
                    v
                }
                // anything the account holds
                None => {
                    self.account_taint.entry(account).or_default().merge(&gaps);
                    self.out.books.keys().filter(|(a, _)| *a == account).map(|(_, i)| *i).collect()
                }
            };
            for i in affected {
                self.taint(account, i, &gaps);
            }
            return self.unapplied(t, gaps);
        }
        let Some(instrument) = t.instrument else { return };
        if t.kind == Kind::Dividend {
            return self.apply_distribution(t);
        }
        // a coin's move into or out of staking moves no units, and states the units it moved
        if t.kind == Kind::StakingMove && self.is_coin(instrument) {
            if let Some(stated) = t.paid_on.filter(|q| q.is_positive()) {
                self.arrived_unstated(t, instrument, stated, true);
            }
            return;
        }
        match mv {
            Move::Nothing => return,
            Move::Event => return self.apply_event(t),
            _ => {}
        }
        let Some(q) = t.quantity else {
            if mv == Move::Expire {
                return self.expire_all(t, instrument);
            }
            let gaps = Gaps::of(Gap::QuantityUnstated(t.id.clone()));
            self.taint(account, instrument, &gaps);
            // a delivery moves the underlying too, by what it does not state
            if let (Move::Deliver { .. }, Some(under)) = (mv, self.underlying(instrument)) {
                self.taint(account, under, &gaps);
            }
            if let Some((_, dest, dest_instr)) = self.link_of(t, instrument) {
                // what arrives from it waits as well
                self.taint(dest, dest_instr, &gaps);
            }
            return self.unapplied(t, gaps);
        };
        let qty = q.abs();
        if qty.is_zero() {
            if mv == Move::Expire {
                return self.expire_all(t, instrument);
            }
            return;
        }
        let acquiring = acquires(t).unwrap_or(q.is_positive());
        let currency = self.currency(instrument);
        // a fee not stated in the instrument's currency leaves the fill's value
        // unstated; a delivery's fee is the underlying's, handled there
        let (fee, charge) = charged(t, currency);
        let closer = Closer::Transaction(t.id.clone());
        let none = BTreeSet::new();
        if !acquiring && self.is_coin(instrument) && matches!(mv, Move::Trade(_) | Move::Transfer | Move::Reward | Move::Resolve) {
            self.arrived_unstated(t, instrument, qty, false);
        }
        match mv {
            Move::Trade(effect) => {
                let mult = multiplier(self.info(instrument), instrument);
                let value = charge(fill_value(t, qty, acquiring, currency, &mult));
                if price_disagrees(t, qty, acquiring, currency, &mult) {
                    self.out.disagreements.push(t.id.clone());
                }
                self.apply_trade(t, instrument, qty, acquiring, effect, value, fee, record_legs);
            }
            Move::Expire => {
                let d = if acquiring { Direction::Short } else { Direction::Long };
                // a row whose sign closes the other side than the one held
                if !self.holds(account, instrument, d) && self.holds(account, instrument, d.opposite()) {
                    return self.contradicts(t, &[(account, instrument)]);
                }
                self.stated_before(&t.id, t.trade_date, t.occurred_at, account, instrument, d, qty);
                match self.close(account, instrument, d, qty, charge(Ok(Money::zero(currency))), fee, &closer, t.trade_date, t.occurred_at, &none) {
                    Ok(c) if c.left.is_positive() => self.beyond(t, account, instrument, c.left),
                    Ok(_) => {}
                    Err(g) => self.taint(account, instrument, &g),
                }
            }
            Move::Deliver { exercise } => self.apply_delivery(t, instrument, qty, acquiring, exercise),
            Move::Transfer => self.apply_transfer(t, instrument, qty, acquiring),
            Move::Reward => {
                if acquiring {
                    let flags = BTreeSet::from([Flag::Reward]);
                    self.open_lot(account, instrument, &t.id, t.trade_date, t.occurred_at, Direction::Long, qty, Ok(Money::zero(currency)), Money::zero(currency), flags, None);
                } else {
                    // a reward taken back: the units leave for nothing
                    self.stated_before(&t.id, t.trade_date, t.occurred_at, account, instrument, Direction::Long, qty);
                    match self.close(account, instrument, Direction::Long, qty, Ok(Money::zero(currency)), Money::zero(currency), &closer, t.trade_date, t.occurred_at, &none) {
                        Ok(c) if c.left.is_positive() => self.beyond(t, account, instrument, c.left),
                        Ok(_) => {}
                        Err(g) => self.taint(account, instrument, &g),
                    }
                }
            }
            Move::Resolve => {
                let value = charge(fill_value(t, qty, false, currency, &Ok(Dec::ONE)));
                self.stated_before(&t.id, t.trade_date, t.occurred_at, account, instrument, Direction::Long, qty);
                match self.close(account, instrument, Direction::Long, qty, value, fee, &closer, t.trade_date, t.occurred_at, &none) {
                    Ok(c) if c.left.is_positive() => self.beyond(t, account, instrument, c.left),
                    Ok(_) => {}
                    Err(g) => self.taint(account, instrument, &g),
                }
            }
            Move::Event | Move::Nothing => {}
        }
        self.write_off_dust(t, instrument);
        self.count_row(t, instrument);
    }

    /// What a coin transaction leaves of the holding, worth less than the
    /// smallest coin order the broker takes at the price it was moved at
    /// (`price_of`), is dust (`crate::dust`): it cannot be sold, so it is written
    /// off with the transaction at nothing and the trade ends there.
    fn write_off_dust(&mut self, t: &Transaction, instrument: InstrumentId) {
        let account = t.account;
        let Some(price) = self.price_of(t, instrument) else { return };
        let held = self.book(account, instrument).lots.iter().filter(|l| l.direction == Direction::Long).try_fold(Dec::ZERO, |a, l| a.checked_add(l.qty));
        let Ok(held) = held else { return };
        if !held.is_positive() || !crate::dust::is_dust(&crate::stated::Sources::of(self.inputs), account, instrument, held, price, t.trade_date) {
            return;
        }
        let zero = Money::zero(self.currency(instrument));
        let flags = BTreeSet::from([Flag::Dust]);
        match self.close(account, instrument, Direction::Long, held, Ok(zero), zero, &Closer::Transaction(t.id.clone()), t.trade_date, t.occurred_at, &flags) {
            Ok(_) => self.out.dust.push(Dust { transaction: t.id.clone(), account, instrument, qty: held, beyond: false }),
            Err(g) => self.taint(account, instrument, &g),
        }
    }

    /// An expiry that states no quantity closes the whole holding of the contract.
    fn expire_all(&mut self, t: &Transaction, instrument: InstrumentId) {
        let account = t.account;
        let currency = self.currency(instrument);
        let lots: Vec<(Direction, Dec)> = self.book(account, instrument).lots.iter().map(|l| (l.direction, l.qty)).collect();
        let closer = Closer::Transaction(t.id.clone());
        for (d, q) in lots {
            // a lot's own fee is still shared out, and a share that does not fit is a gap
            if let Err(g) = self.close(account, instrument, d, q, Ok(Money::zero(currency)), Money::zero(currency), &closer, t.trade_date, t.occurred_at, &BTreeSet::new()) {
                self.taint(account, instrument, &g);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_trade(&mut self, t: &Transaction, instrument: InstrumentId, qty: Dec, acquiring: bool, effect: Option<Effect>, value: Fig<Money>, fee: Money, record_legs: &[&Transaction]) {
        let account = t.account;
        let option = self.is_option(instrument);
        // a buy closes shorts; a sale closes longs
        let closes = if acquiring { Direction::Short } else { Direction::Long };
        let opens = if acquiring { Direction::Long } else { Direction::Short };
        let closer = Closer::Transaction(t.id.clone());
        let none = BTreeSet::new();
        // a stated effect the position contradicts: an open meeting the opposite
        // position, a close meeting only the opposite one (a close meeting
        // nothing is beyond what is held)
        let contradicted = match effect {
            Some(Effect::Open) => self.holds(account, instrument, closes),
            Some(Effect::Close) => !self.holds(account, instrument, closes) && self.holds(account, instrument, opens),
            None => false,
        };
        if contradicted {
            return self.contradicts(t, &[(account, instrument)]);
        }
        let may_close = effect != Some(Effect::Open);
        // what is left once the lots run out opens a position only where it may:
        // an option's net position either way, a share, coin or event contract only
        // long, unless the record says it opens; where it may not, units
        // Wealthsimple states held come first
        let may_open_left = match effect {
            Some(Effect::Close) => false,
            Some(Effect::Open) => true,
            None => option || acquiring,
        };
        if may_close && !may_open_left {
            self.stated_before(&t.id, t.trade_date, t.occurred_at, account, instrument, closes, qty);
        }
        let Closed { left, value_left, fee_left } = if may_close {
            match self.close(account, instrument, closes, qty, value, fee, &closer, t.trade_date, t.occurred_at, &none) {
                Ok(r) => r,
                Err(g) => return self.taint(account, instrument, &g),
            }
        } else {
            Closed { left: qty, value_left: value, fee_left: fee }
        };
        let closed = left < qty;
        if closed {
            self.out.roles.insert(t.id.clone(), FillRole { closed, opened: false });
        }
        if !left.is_positive() {
            return;
        }
        // what is left opens a position: an option's net position either way; a
        // share, coin or event contract only long, unless the record says it opens
        if !may_open_left {
            return self.beyond(t, account, instrument, left);
        }
        let joins = if option { self.rolled_from(t, instrument, opens, record_legs) } else { None };
        let mut flags = BTreeSet::new();
        if joins.is_some() {
            flags.insert(Flag::Rolled);
        }
        self.open_lot(account, instrument, &t.id, t.trade_date, t.occurred_at, opens, left, value_left, fee_left, flags, joins);
        self.out.roles.insert(t.id.clone(), FillRole { closed, opened: true });
    }

    /// The round trip a roll's opening leg continues: the one closed by another
    /// leg of the same record, on a contract of the same underlying, the same way.
    /// Note an event whose broker-stated units differ from its ratio's; with
    /// cash paid in lieu of fractions, a whole-unit shortfall under one is the
    /// fraction paid out, and the answer is whether it is that.
    fn check_units(&mut self, anchor: &TransactionId, account: AccountId, instrument: InstrumentId, stated: Dec, by_ratio: Dec, lieu: bool) -> bool {
        if stated == by_ratio {
            return false;
        }
        let fraction = by_ratio.checked_sub(stated).is_ok_and(|d| d.is_positive() && d < Dec::ONE);
        if lieu && fraction {
            return true;
        }
        self.out.unit_disagreements.push(UnitsDisagree { transaction: anchor.clone(), account, instrument, stated, by_ratio });
        false
    }

    /// Whether a fill would close part of a position the account holds.
    fn meets_opposite(&self, t: &Transaction) -> bool {
        let (Some(instrument), Some(q)) = (t.instrument, t.quantity) else { return false };
        if !matches!(read_move(t), Move::Trade(None | Some(Effect::Close))) || q.is_zero() {
            return false;
        }
        let closes = if acquires(t).unwrap_or(q.is_positive()) { Direction::Short } else { Direction::Long };
        self.out.books.get(&(t.account, instrument)).is_some_and(|b| b.lots.iter().any(|l| l.direction == closes))
    }

    fn rolled_from(&self, t: &Transaction, instrument: InstrumentId, direction: Direction, legs: &[&Transaction]) -> Option<TripKey> {
        let under = self.underlying(instrument)?;
        for leg in legs {
            if leg.id == t.id || leg.account != t.account {
                continue;
            }
            let Some(other) = leg.instrument else { continue };
            if other == instrument || self.underlying(other) != Some(under) {
                continue;
            }
            // the other leg closed a position of the same direction
            let closed_dir = if acquires(leg) == Some(true) { Direction::Short } else { Direction::Long };
            if closed_dir != direction {
                continue;
            }
            // the round trips the leg closed, found by the leg, never by a pass
            // over every round trip (the first by key, as such a pass found it)
            let closed = self.closed_by.get(&leg.id).into_iter().flatten();
            if let Some(k) = closed.filter(|k| self.out.trips.get(k).is_some_and(|tr| tr.instruments.contains(&other))).min() {
                return Some(k.clone());
            }
        }
        None
    }

    fn apply_transfer(&mut self, t: &Transaction, instrument: InstrumentId, qty: Dec, acquiring: bool) {
        let account = t.account;
        let currency = self.currency(instrument);
        if acquiring {
            if self.links_in.contains(&t.id) {
                // its lots arrive with the linked transfer out
                return;
            }
            // what the asset cost: the person's stated cost, else what the source
            // states it was worth as it arrived, else not on the record
            let stated = self.inputs.facts.adjustments.get(&t.id).and_then(|a| a.legs.iter().find(|l| l.from.is_none() && l.to == Some(instrument) && l.cost.is_some()).cloned());
            let mut flags = BTreeSet::from([Flag::Deposited]);
            let (value, day) = match stated {
                Some(leg) => (Ok(leg.cost.expect("filtered")), leg.acquired.unwrap_or(t.trade_date)),
                None if self.inputs.facts.adjustments.in_conflict(&t.id) => (Err(Gaps::of(Gap::AdjustmentConflict(t.id.clone()))), t.trade_date),
                None if t.value.is_some() => (Ok(t.value.expect("tested")), t.trade_date),
                // units Wealthsimple's positions read states held: their cost is the
                // read's book value for the holding less the lots' cost, never below
                // nothing, and nothing is asked of the person (brief 79, amended
                // 2026-10-28, point 5); the form only for an account no holdings are
                // read of
                None => match self.cost_stated(account, instrument, qty, t.trade_date) {
                    Some((value, read)) => {
                        self.out.beyond.push(Beyond { transaction: t.id.clone(), account, instrument, qty, read });
                        flags.insert(Flag::Stated);
                        (value, t.trade_date)
                    }
                    None => {
                        self.out.waiting.insert(t.id.clone(), Waiting { what: Wanted::CostOfArrival, account, instrument, day: t.trade_date, units: Some(qty) });
                        (Err(Gaps::of(Gap::BasisUnknown(t.id.clone()))), t.trade_date)
                    }
                },
            };
            let value = match value {
                Ok(v) if v.currency != currency => Err(Gaps::of(Gap::CurrencyUnstated(t.id.clone()))),
                v => v,
            };
            self.open_lot(account, instrument, &t.id, day, t.occurred_at, Direction::Long, qty, value, Money::zero(currency), flags, None);
            return;
        }
        let link = self.link_of(t, instrument);
        // what the holding waits on goes with what leaves it
        let waiting = self.book(account, instrument).taint.clone();
        self.stated_before(&t.id, t.trade_date, t.occurred_at, account, instrument, Direction::Long, qty);
        match self.take(account, instrument, qty) {
            Ok(Taken { lots, left, cost }) => {
                if left.is_positive() {
                    self.beyond(t, account, instrument, left);
                }
                // the person's own accounts: the lots move with their cost and dates,
                // the same round trip; the holding's average cost in the receiving
                // account is what the broker gave the units there where it states
                // it (a cost reset to their value as they moved), else theirs
                let Some((to, dest, dest_instr)) = link else { return self.moved_out(&lots) };
                self.consumed.insert(to.clone());
                let cost = self.cost_on_arrival(&to, dest_instr).unwrap_or(cost);
                if !waiting.is_empty() {
                    self.taint(dest, dest_instr, &waiting);
                }
                if left.is_positive() {
                    // units beyond what was held arrive all the same, their cost
                    // not on the record
                    let own = TripKey { opening: to.clone(), instrument: dest_instr };
                    let flags = BTreeSet::from([Flag::Transferred]);
                    let currency = self.currency(dest_instr);
                    self.open_lot(dest, dest_instr, &to, t.trade_date, t.occurred_at, Direction::Long, left, Err(Gaps::of(Gap::BeyondHeld(t.id.clone()))), Money::zero(currency), flags, Some(own));
                }
                let whole = self.book(account, instrument).lots.is_empty();
                if whole {
                    // the whole holding moved: it is the same round trip, held elsewhere
                    let lots = lots
                        .into_iter()
                        .map(|mut l| {
                            l.flags.insert(Flag::Transferred);
                            l
                        })
                        .collect();
                    self.place(dest, dest_instr, lots);
                    self.basis_add(dest, dest_instr, Direction::Long, &cost);
                } else {
                    // part of it moved: a round trip of its own in the receiving
                    // account, each lot keeping its cost and the day it was bought, the
                    // units bringing their share of the average cost
                    self.moved_out(&lots);
                    for mut l in lots {
                        l.flags.insert(Flag::Transferred);
                        let own = TripKey { opening: to.clone(), instrument: dest_instr };
                        self.push_lot(dest, dest_instr, &to, l.day, l.at, l.direction, l.qty, l.value, l.fee, l.flags, Some(own));
                    }
                    self.basis_add(dest, dest_instr, Direction::Long, &cost);
                }
            }
            Err(g) => {
                self.taint(account, instrument, &g);
                if let Some((to, dest, dest_instr)) = link {
                    // the units arrive, what they cost waiting on what failed here
                    self.consumed.insert(to.clone());
                    let own = TripKey { opening: to.clone(), instrument: dest_instr };
                    let flags = BTreeSet::from([Flag::Transferred]);
                    let currency = self.currency(dest_instr);
                    self.open_lot(dest, dest_instr, &to, t.trade_date, t.occurred_at, Direction::Long, qty, Err(g.clone()), Money::zero(currency), flags, Some(own));
                    self.taint(dest, dest_instr, &g);
                }
            }
        }
    }

    /// The cost the broker states units moved in from another of the person's
    /// accounts carry there (`SPEC.md` §2, Moves between your accounts): the
    /// receiving row's stated value, in the instrument's currency. None where it
    /// states none, or states it only in another currency (no rate the broker
    /// used is stated): the units keep their own.
    fn cost_on_arrival(&self, to: &TransactionId, instrument: InstrumentId) -> Option<Fig<Money>> {
        let recv = self.inputs.ledger.transactions.get(to)?;
        recv.value.filter(|v| v.currency == self.currency(instrument)).map(Ok)
    }

    /// A transfer out's linked receiving side: its id, account and instrument.
    fn link_of(&self, t: &Transaction, instrument: InstrumentId) -> Option<(TransactionId, AccountId, InstrumentId)> {
        let to = self.links_out.get(&t.id)?;
        let recv = self.inputs.ledger.transactions.get(to)?;
        Some((to.clone(), recv.account, recv.instrument.unwrap_or(instrument)))
    }

    /// An assignment or exercise: the contract closes at zero, keeping its
    /// premium, and the underlying moves by contracts × multiplier at the strike.
    fn apply_delivery(&mut self, t: &Transaction, instrument: InstrumentId, qty: Dec, acquiring: bool, exercise: bool) {
        let account = t.account;
        let currency = self.currency(instrument);
        let flags = BTreeSet::from([Flag::Assignment]);
        let closer = Closer::Transaction(t.id.clone());
        // an assignment brings a short's contracts back in; an exercise sends a long's out
        let closing = if exercise { Direction::Long } else { Direction::Short };
        let _ = acquiring;
        // only a short is assigned and only a long exercised: against the other
        // side, or nothing, the record contradicts the book, and neither the
        // contract nor its underlying moves
        if !self.holds(account, instrument, closing) {
            let mut holdings = vec![(account, instrument)];
            if let Some(under) = self.underlying(instrument) {
                holdings.push((account, under));
            }
            return self.contradicts(t, &holdings);
        }
        self.stated_before(&t.id, t.trade_date, t.occurred_at, account, instrument, closing, qty);
        match self.close(account, instrument, closing, qty, Ok(Money::zero(currency)), Money::zero(currency), &closer, t.trade_date, t.occurred_at, &flags) {
            Ok(c) if c.left.is_positive() => self.beyond(t, account, instrument, c.left),
            Ok(_) => {}
            Err(g) => self.taint(account, instrument, &g),
        }
        let Some(terms) = self.info(instrument).and_then(|i| i.terms.clone()) else {
            return self.unapplied(t, Gaps::of(Gap::Arithmetic(format!("{instrument} has no contract terms"))));
        };
        let under = terms.underlying;
        let Some(mult) = terms.multiplier else {
            let gaps = Gaps::of(Gap::MultiplierUnstated(instrument));
            self.taint(account, under, &gaps);
            return self.unapplied(t, gaps);
        };
        let shares = match qty.checked_mul(mult) {
            Ok(s) => s,
            Err(e) => return self.taint(account, under, &Gaps::from(e)),
        };
        // who receives the shares: a call's holder, a put's writer
        let receives = matches!((terms.right, exercise), (OptionRight::Call, true) | (OptionRight::Put, false));
        let under_currency = self.currency(under);
        let value = match t.cash {
            Some(_) => fill_value(t, shares, receives, under_currency, &Ok(Dec::ONE)),
            None => terms.strike.checked_mul(shares).map(|v| Money::new(v, under_currency)).map_err(Gaps::from),
        };
        let (fee, charge) = charged(t, under_currency);
        let value = charge(value);
        let (closes, opens) = if receives { (Direction::Short, Direction::Long) } else { (Direction::Long, Direction::Short) };
        let Closed { left, value_left, fee_left } = match self.close(account, under, closes, shares, value, fee, &closer, t.trade_date, t.occurred_at, &flags) {
            Ok(r) => r,
            Err(g) => return self.taint(account, under, &g),
        };
        if left.is_positive() {
            // delivering shares not held opens a short: the contract obliges it
            let own = TripKey { opening: t.id.clone(), instrument: under };
            self.open_lot(account, under, &t.id, t.trade_date, t.occurred_at, opens, left, value_left, fee_left, flags, Some(own));
        }
    }

    /// A corporate event: the broker's own units are the statement of what moved;
    /// the adjustment says what the event was.
    fn apply_event(&mut self, t: &Transaction) {
        let anchor = self.event_groups.get(&t.id).cloned();
        let account = t.account;
        let Some(anchor) = anchor else {
            // nothing says what this event was: the units the broker states move
            // at an unknown cost, and the holding waits on it
            let Some(instrument) = t.instrument else { return };
            let conflict = self.inputs.facts.adjustments.in_conflict(&t.id);
            let unknown = if conflict { Gaps::of(Gap::AdjustmentConflict(t.id.clone())) } else { Gaps::of(Gap::EventUnknown(t.id.clone())) };
            if !conflict {
                self.out.waiting.insert(t.id.clone(), Waiting { what: Wanted::Event, account, instrument, day: t.trade_date, units: t.quantity });
            }
            self.taint(account, instrument, &unknown);
            if let Some(q) = t.quantity.filter(|q| !q.is_zero()) {
                if q.is_positive() {
                    let currency = self.currency(instrument);
                    let flags = BTreeSet::from([Flag::FromEvent]);
                    self.open_lot(account, instrument, &t.id, t.trade_date, t.occurred_at, Direction::Long, q, Err(unknown.clone()), Money::zero(currency), flags, None);
                } else if let Ok(Taken { lots, left, .. }) = {
                    self.stated_before(&t.id, t.trade_date, t.occurred_at, account, instrument, Direction::Long, q.abs());
                    self.take(account, instrument, q.abs())
                } {
                    self.moved_out(&lots);
                    if left.is_positive() {
                        self.beyond(t, account, instrument, left);
                    }
                }
            }
            return self.unapplied(t, unknown);
        };
        let adjustment = self.inputs.facts.adjustments.get(&anchor).cloned().expect("a group is formed from an adjustment");
        // the event's rows, all applied here
        let rows: Vec<Transaction> = self.inputs.ledger.transactions.iter().filter(|x| self.event_groups.get(&x.id) == Some(&anchor)).cloned().collect();
        for r in &rows {
            self.consumed.insert(r.id.clone());
        }
        let stated: BTreeMap<InstrumentId, Dec> = {
            let mut m: BTreeMap<InstrumentId, Dec> = BTreeMap::new();
            for r in &rows {
                if let (Some(i), Some(q)) = (r.instrument, r.quantity) {
                    let e = m.entry(i).or_insert(Dec::ZERO);
                    match e.checked_add(q) {
                        Ok(v) => *e = v,
                        Err(err) => {
                            let g = Gaps::from(err);
                            self.taint(account, i, &g);
                            return self.unapplied(t, g);
                        }
                    }
                }
            }
            m
        };
        // holdings whose fractions the event pays in cash: the broker's whole
        // units may fall short of the ratio by less than one
        let lieu: BTreeSet<InstrumentId> = adjustment.legs.iter().filter(|l| l.to.is_none() && l.cash_per_unit.is_some()).filter_map(|l| l.from).collect();
        let day = t.trade_date;
        let at = t.occurred_at;
        // each holding's lots' cost and average cost before the event: every leg's
        // share of a cost is a share of the cost as it stood, whatever the legs
        // before it moved
        let mut before: BTreeMap<InstrumentId, Before> = BTreeMap::new();
        for i in adjustment.legs.iter().filter_map(|l| l.from) {
            let lots = self.book(account, i).lots.iter().filter(|l| l.direction == Direction::Long).map(|l| l.value.clone()).collect();
            let cost = self.basis_mut(account, i, Direction::Long).cost.clone();
            before.insert(i, Before { lots, cost });
        }
        for leg in &adjustment.legs {
            if let Err(g) = self.apply_leg(account, &anchor, day, at, leg, &stated, &before, &lieu) {
                for i in [leg.from, leg.to].into_iter().flatten() {
                    self.taint(account, i, &g);
                }
                self.unapplied(t, g);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_leg(&mut self, account: AccountId, anchor: &TransactionId, day: Date, at: Option<Timestamp>, leg: &AdjustmentLeg, stated: &BTreeMap<InstrumentId, Dec>, before: &BTreeMap<InstrumentId, Before>, lieu: &BTreeSet<InstrumentId>) -> Result<(), Gaps> {
        let unknown = || Gaps::of(Gap::EventUnknown(anchor.clone()));
        let Some(from) = leg.from else {
            // a stated cost belongs to a deposit, applied where the deposit is
            return Ok(());
        };
        let short: Dec = dec_sum(self.book(account, from).lots.iter().filter(|l| l.direction == Direction::Short).map(|l| l.qty))?;
        if short.is_positive() {
            return self.event_on_short(account, anchor, leg, from, short);
        }
        let held: Dec = dec_sum(self.book(account, from).lots.iter().filter(|l| l.direction == Direction::Long).map(|l| l.qty))?;
        match (leg.to, leg.cash_per_unit) {
            // a split or consolidation, a stock dividend, a return of capital
            (Some(to), None) if to == from => {
                if let Some(cost) = leg.cost {
                    // a stock dividend: new units at the stated value, which is income
                    let units = match stated.get(&from) {
                        Some(q) if q.is_positive() => {
                            if let Some(r) = leg.units_per_unit {
                                let _ = self.check_units(anchor, account, from, *q, held.checked_mul(r)?, lieu.contains(&from));
                            }
                            *q
                        }
                        _ => held.checked_mul(leg.units_per_unit.ok_or_else(unknown)?)?,
                    };
                    let currency = self.currency(from);
                    if cost.currency != currency {
                        return Err(Gaps::of(Gap::CurrencyUnstated(anchor.clone())));
                    }
                    self.out.stock_dividends.push(StockDividend { transaction: anchor.clone(), account, instrument: from, day, units, value: cost });
                    let flags = BTreeSet::from([Flag::FromEvent]);
                    self.open_lot(account, from, anchor, day, at, Direction::Long, units, Ok(cost), Money::zero(currency), flags, None);
                    return Ok(());
                }
                // the broker's units stand where it states them; the ratio where it does not
                let new_total = match stated.get(&from) {
                    Some(q) if !q.is_zero() => {
                        let total = held.checked_add(*q)?;
                        match leg.units_per_unit {
                            Some(r) => {
                                let by_ratio = held.checked_mul(r)?;
                                if self.check_units(anchor, account, from, total, by_ratio, lieu.contains(&from)) {
                                    // the ratio's units, the fraction then paid out in cash
                                    self.lieu_fraction.insert((account, from), by_ratio.checked_sub(total)?);
                                    by_ratio
                                } else {
                                    total
                                }
                            }
                            None => total,
                        }
                    }
                    _ => held.checked_mul(leg.units_per_unit.ok_or_else(unknown)?)?,
                };
                self.rescale(account, from, held, new_total)?;
                Ok(())
            }
            // a return of capital: cash per unit taken off the cost
            (Some(to), Some(cash)) if to == from => self.return_capital(account, from, cash, anchor, day, at),
            // a merger for shares and cash: the cash per unit comes off the cost,
            // and the holding continues as the new shares
            (Some(_), Some(cash)) => {
                self.return_capital(account, from, cash, anchor, day, at)?;
                let shares = AdjustmentLeg { cash_per_unit: None, ..leg.clone() };
                self.apply_leg(account, anchor, day, at, &shares, stated, before, lieu)
            }
            // the holding continues as another instrument, or a spin-off's child
            (Some(to), None) => {
                let units = match stated.get(&to) {
                    Some(q) if q.is_positive() => {
                        if let Some(r) = leg.units_per_unit {
                            let _ = self.check_units(anchor, account, to, *q, held.checked_mul(r)?, lieu.contains(&from));
                        }
                        *q
                    }
                    _ => held.checked_mul(leg.units_per_unit.ok_or_else(unknown)?)?,
                };
                let cost_share = leg.cost_share.unwrap_or(Dec::ONE);
                if cost_share == Dec::ONE {
                    // the same holding under a new instrument: its lots move whole
                    let Taken { lots, cost, .. } = self.take(account, from, held)?;
                    let taint = self.book(account, from).taint.clone();
                    let scaled = scale_lots(lots, held, units)?;
                    let moved = scaled
                        .into_iter()
                        .map(|mut l| {
                            l.flags.insert(Flag::Continued);
                            l
                        })
                        .collect();
                    self.book(account, to).taint.merge(&taint);
                    self.place(account, to, moved);
                    self.basis_add(account, to, Direction::Long, &cost);
                    return Ok(());
                }
                // a spin-off: the child takes part of each lot's cost and keeps its dates
                let currency = self.currency(to);
                let costs_before = before.get(&from).map(|b| b.lots.clone()).unwrap_or_default();
                let parents: Vec<(Dec, Fig<Money>, Date, Option<Timestamp>)> = self
                    .book(account, from)
                    .lots
                    .iter()
                    .filter(|l| l.direction == Direction::Long)
                    .enumerate()
                    .map(|(k, l)| (l.qty, costs_before.get(k).cloned().unwrap_or_else(|| l.value.clone()), l.day, l.at))
                    .collect();
                let mut child_left = units;
                let mut held_left = held;
                let mut moved_values = Vec::new();
                for (pq, pv, pday, pat) in parents {
                    let child_qty = share_dec(child_left, pq, held_left)?;
                    let moved = match &pv {
                        Ok(v) => Ok(v.times(cost_share)?),
                        Err(g) => Err(g.clone()),
                    };
                    moved_values.push(moved.clone());
                    let value = match moved {
                        Ok(m) if m.currency == currency => Ok(m),
                        Ok(_) => Err(Gaps::of(Gap::CurrencyUnstated(anchor.clone()))),
                        Err(g) => Err(g),
                    };
                    let flags = BTreeSet::from([Flag::FromEvent]);
                    let own = TripKey { opening: anchor.clone(), instrument: to };
                    self.push_lot(account, to, anchor, pday, pat, Direction::Long, child_qty, value, Money::zero(currency), flags, Some(own));
                    child_left = child_left.checked_sub(child_qty)?;
                    held_left = held_left.checked_sub(pq)?;
                }
                let book = self.book(account, from);
                for (lot, moved) in book.lots.iter_mut().filter(|l| l.direction == Direction::Long).zip(moved_values) {
                    lot.value = fig_sub(&lot.value, &moved);
                }
                // the child takes the same share of the parent's average cost
                let parent = before.get(&from).map(|b| b.cost.clone()).unwrap_or(Ok(Money::zero(self.currency(from))));
                let moved: Fig<Money> = parent.and_then(|c| Ok(c.times(cost_share)?));
                let child = match &moved {
                    Ok(m) if m.currency != currency => Err(Gaps::of(Gap::CurrencyUnstated(anchor.clone()))),
                    m => m.clone(),
                };
                self.basis_add(account, to, Direction::Long, &child);
                let b = self.basis_mut(account, from, Direction::Long);
                b.cost = fig_sub(&b.cost, &moved);
                Ok(())
            }
            // a merger for cash, cash in lieu: units out at the cash per unit; how
            // many is the leg's share of the holding as it stands, else the units
            // the broker states went out, never the whole holding by default
            (None, Some(cash)) => {
                let out = match (self.lieu_fraction.remove(&(account, from)), leg.units_per_unit, stated.get(&from)) {
                    // the fraction the broker's whole units fell short of the ratio by
                    (Some(fraction), _, _) => fraction,
                    (None, Some(share), _) => held.checked_mul(share)?,
                    (None, None, Some(q)) if q.is_negative() => q.abs(),
                    _ => return Err(unknown()),
                };
                let currency = self.currency(from);
                if cash.currency != currency {
                    return Err(Gaps::of(Gap::CurrencyUnstated(anchor.clone())));
                }
                let value = cash.times(out)?;
                let closer = Closer::Transaction(anchor.clone());
                self.stated_before(anchor, day, at, account, from, Direction::Long, out);
                let left = self.close(account, from, Direction::Long, out, Ok(value), Money::zero(currency), &closer, day, at, &BTreeSet::new())?.left;
                if left.is_positive() {
                    self.out.beyond.push(Beyond { transaction: anchor.clone(), account, instrument: from, qty: left, read: StatedRead::Unstated });
                }
                Ok(())
            }
            (None, None) => Err(unknown()),
        }
    }

    /// Cash per unit paid back on a holding: taken off each lot's cost. A cost
    /// that would go below nothing is a gain the record does not describe: a gap.
    /// A return of capital: `cash` per unit held taken off each long lot's cost
    /// (`SPEC.md` §2, "What you enter"). Capital returned beyond what is left
    /// of a lot's cost is realized that day, a part of the lot's trade, and the
    /// lot's cost is then zero.
    fn return_capital(&mut self, account: AccountId, instrument: InstrumentId, cash: Money, anchor: &TransactionId, day: Date, at: Option<Timestamp>) -> Result<(), Gaps> {
        let mut realized: Vec<Slice> = Vec::new();
        let entered = self.entered(anchor);
        let book = self.book(account, instrument);
        for lot in book.lots.iter_mut().filter(|l| l.direction == Direction::Long) {
            if entered {
                lot.flags.insert(Flag::Entered);
            }
            let back = cash.times(lot.qty)?;
            lot.value = match &lot.value {
                Ok(v) if v.currency != back.currency => Err(Gaps::of(Gap::CurrencyUnstated(anchor.clone()))),
                Ok(v) => match v.checked_sub(back)? {
                    left if left.amount.is_negative() => {
                        // the excess is realized: a part of the trade with no units
                        realized.push(Slice {
                            trip: lot.trip.clone(),
                            account,
                            instrument,
                            direction: Direction::Long,
                            qty: Dec::ZERO,
                            opened_by: lot.opened_by.clone(),
                            opened_on: lot.day,
                            opened_at: lot.at,
                            closed_by: Closer::Transaction(anchor.clone()),
                            closed_on: day,
                            closed_at: at,
                            entry: Ok(Money::zero(v.currency)),
                            exit: Ok(left.neg()),
                            entry_fee: Money::zero(v.currency),
                            exit_fee: Money::zero(v.currency),
                            flags: lot.flags.clone(),
                            taint: Gaps::none(),
                        });
                        Ok(Money::zero(v.currency))
                    }
                    left => Ok(left),
                },
                Err(g) => Err(g.clone()),
            };
        }
        for slice in realized {
            let key = slice.trip.clone();
            let tr = self.trip_mut(&key);
            tr.slices.push(slice);
            tr.fills.insert(anchor.clone());
        }
        // at average cost: the capital returned on every unit held comes off the
        // holding's cost, and what it returns beyond that cost is realized
        let held = self.held(account, instrument, Direction::Long)?;
        if held.is_positive() {
            let back = cash.times(held)?;
            let b = self.basis_mut(account, instrument, Direction::Long);
            let (cost, gain) = match &b.cost {
                Ok(c) if c.currency != back.currency => (Err(Gaps::of(Gap::CurrencyUnstated(anchor.clone()))), Err(Gaps::of(Gap::CurrencyUnstated(anchor.clone())))),
                Ok(c) => match c.checked_sub(back)? {
                    left if left.amount.is_negative() => (Ok(Money::zero(c.currency)), Ok(left.neg())),
                    left => (Ok(left), Ok(Money::zero(c.currency))),
                },
                Err(g) => (Err(g.clone()), Err(g.clone())),
            };
            b.cost = cost;
            self.basis_realize(account, instrument, Direction::Long, &gain);
        }
        Ok(())
    }

    /// A distribution the person or a source says returned capital: its legs'
    /// cash per unit off the holding's cost, on the day it is paid.
    fn apply_distribution(&mut self, t: &Transaction) {
        let Some(adj) = self.inputs.facts.adjustments.get(&t.id).cloned() else { return };
        for leg in &adj.legs {
            if let (Some(from), Some(to), Some(cash)) = (leg.from, leg.to, leg.cash_per_unit) {
                if from == to {
                    if let Err(g) = self.return_capital(t.account, from, cash, &t.id, t.trade_date, t.occurred_at) {
                        self.taint(t.account, from, &g);
                    }
                }
            }
        }
    }

    /// A holding's lots rescaled from `held` units to `new_total`, each in
    /// proportion, the last taking what is left; the cost stays.
    /// An event on a short holding: a split or consolidation scales the short
    /// by its ratio, and a continuation as another instrument moves it whole by
    /// its ratio, as they do a long. Anything else (a stock dividend, cash, a
    /// spin-off), or an event stating no ratio, leaves what the short owes
    /// unworked, and the holding waits.
    fn event_on_short(&mut self, account: AccountId, anchor: &TransactionId, leg: &AdjustmentLeg, from: InstrumentId, held: Dec) -> Result<(), Gaps> {
        let waits = || Gaps::of(Gap::EventOnShort(anchor.clone()));
        let (Some(to), Some(ratio), None, None) = (leg.to, leg.units_per_unit, leg.cash_per_unit, leg.cost) else { return Err(waits()) };
        if leg.cost_share.is_some_and(|c| c != Dec::ONE) {
            return Err(waits());
        }
        let total = held.checked_mul(ratio)?;
        let Taken { lots, cost, .. } = self.take_side(account, from, Direction::Short, held)?;
        let flag = if to == from { Flag::Split } else { Flag::Continued };
        let moved: Vec<Lot> = scale_side(lots, Direction::Short, held, total)?
            .into_iter()
            .map(|mut l| {
                l.flags.insert(flag);
                l
            })
            .collect();
        if to != from {
            let taint = self.book(account, from).taint.clone();
            self.book(account, to).taint.merge(&taint);
        }
        self.place(account, to, moved);
        self.basis_add(account, to, Direction::Short, &cost);
        Ok(())
    }

    fn rescale(&mut self, account: AccountId, instrument: InstrumentId, held: Dec, new_total: Dec) -> Result<(), Gaps> {
        if held.is_zero() {
            return Ok(());
        }
        let lots: Vec<Lot> = self.book(account, instrument).lots.drain(..).collect();
        let scaled = scale_lots(lots, held, new_total)?;
        let book = self.book(account, instrument);
        book.lots = scaled
            .into_iter()
            .map(|mut l| {
                l.flags.insert(Flag::Split);
                l
            })
            .collect();
        Ok(())
    }
}

/// Long lots holding `held` units in all rescaled to `total`, each in
/// proportion, the last long lot taking what is left.
fn scale_lots(lots: Vec<Lot>, held: Dec, total: Dec) -> Result<Vec<Lot>, Gaps> {
    scale_side(lots, Direction::Long, held, total)
}

/// `direction`'s lots holding `held` units in all rescaled to `total`, as `scale_lots`.
fn scale_side(lots: Vec<Lot>, direction: Direction, held: Dec, total: Dec) -> Result<Vec<Lot>, Gaps> {
    let mut left = total;
    let mut held_left = held;
    let n = lots.len();
    let mut out = Vec::with_capacity(n);
    for (i, mut l) in lots.into_iter().enumerate() {
        if l.direction == direction {
            let q = if i + 1 == n || held_left == l.qty { left } else { share_dec(left, l.qty, held_left)? };
            left = left.checked_sub(q)?;
            held_left = held_left.checked_sub(l.qty)?;
            l.qty = q;
        }
        out.push(l);
    }
    Ok(out)
}

/// The match over every live transaction.
pub fn match_lots(inputs: &Inputs) -> Matched {
    match_over(inputs, inputs.ledger.transactions.iter().collect())
}

/// A transaction's place in the match: the order it is applied in, then its id.
/// A record is ranked and ordered by all its legs, whichever of them are matched.
pub fn place_in_match(ledger: &crate::input::Ledger, t: &Transaction) -> (Date, u8, Option<Timestamp>, u8, String, u8, String, TransactionId) {
    let legs: Vec<&Transaction> = ledger.transactions.of_record(t.id.record).collect();
    let key = ledger.records.get(&t.id.record).map(|r| r.source_key.clone()).unwrap_or_default();
    let (a, b, c, d, e, f, g) = order_key(t, record_rank(&legs), &key);
    (a, b, c, d, e, f, g, t.id.clone())
}

/// The match of `txs` alone: a set of transactions no other transaction's
/// matching reaches (a family, `crate::families`), or every transaction. Each
/// record is ranked, ordered and applied by all its legs, as the whole match
/// applies it (`docs/plans/stage-3-the-record-by-change.md`, B): a record whose
/// legs share a day is applied where its first leg falls, whichever family that
/// leg is in, each of `txs`'s legs as the whole match would. The lists of what
/// the record waits on are in the match's order (`place_in_match`), so the
/// matches of several families put together are the whole match.
pub fn match_over(inputs: &Inputs, txs: Vec<&Transaction>) -> Matched {
    match_family(inputs, txs).0
}

/// A transaction's place in the walk, without its id: what "after everything a
/// family matched" is decided by (`MatchState::last`). The walk breaks ties by
/// the ledger's place.
pub type OrderKey = (Date, u8, Option<Timestamp>, u8, String, u8, String);

/// What a family's match keeps, to continue from for rows placed after
/// everything it matched (`docs/plans/stage-3-a-change-on-a-deep-history.md`, B):
/// every part of the matcher but its inputs and its output, as it stood before
/// the last walk's expiry up to today, with what that expiry took off the
/// calendar to be put back.
#[derive(Clone, Debug, Default)]
pub struct MatchState {
    expiring: BTreeMap<Date, BTreeSet<(AccountId, InstrumentId)>>,
    underlying_taint: BTreeMap<(AccountId, InstrumentId), Gaps>,
    instrument_taint: BTreeMap<(AccountId, InstrumentId), Gaps>,
    account_taint: BTreeMap<AccountId, Gaps>,
    consumed: BTreeSet<TransactionId>,
    links_out: BTreeMap<TransactionId, TransactionId>,
    links_in: BTreeSet<TransactionId>,
    event_groups: BTreeMap<TransactionId, TransactionId>,
    lieu_fraction: BTreeMap<(AccountId, InstrumentId), Dec>,
    ended_on_record: BTreeSet<(AccountId, InstrumentId)>,
    pub places: BTreeMap<(InstrumentId, bagholder_core::SourceName), u32>,
    rounding_held: BTreeMap<(AccountId, InstrumentId), Dec>,
    /// The holdings with lots open.
    open: BTreeSet<(AccountId, InstrumentId)>,
    /// Each stretch in which every holding of the family is flat, from the place
    /// of the row it went flat after (none: before its first row) to the place of
    /// the next row that moves a lot (none: still flat at the end); a row the walk
    /// passes over neither ends nor starts one. A round trip added inside one is
    /// placed there (`place_in_stretch`; `docs/plans/matching-from-a-flat-point.md`).
    pub stretches: BTreeMap<Option<OrderKey>, Option<OrderKey>>,
    /// What the match held at each stretch start that is a clean break, to walk
    /// on from there (`match_from_cut`; `docs/plans/continuing-from-a-flat-stretch.md`).
    /// A cache, left out of comparing two states: a cut is held to a whole match
    /// where it is used, and dropped wherever a change could leave it stale.
    pub cuts: BTreeMap<OrderKey, Cut>,
    /// The largest place walked.
    pub last: Option<OrderKey>,
    /// The holdings of the stated-Open rows walked at `last`'s instant, over every
    /// walk at that instant: what the look-ahead of a close before an open reads.
    pub opens_at_last: BTreeSet<(AccountId, InstrumentId)>,
    /// What the expiry up to today took off the calendar, put back before the
    /// next walk.
    expired_at_end: BTreeMap<Date, BTreeSet<(AccountId, InstrumentId)>>,
    /// Whether that expiry closed or marked any contract: a match it changed
    /// cannot be continued, since rows placed before today would meet it.
    pub ended_quiet: bool,
}

impl PartialEq for MatchState {
    fn eq(&self, o: &Self) -> bool {
        // every part but the cuts
        let MatchState { expiring, underlying_taint, instrument_taint, account_taint, consumed, links_out, links_in, event_groups, lieu_fraction, ended_on_record, places, rounding_held, open, stretches, cuts: _, last, opens_at_last, expired_at_end, ended_quiet } = self;
        (expiring, underlying_taint, instrument_taint, account_taint, consumed, links_out, links_in, event_groups)
            == (&o.expiring, &o.underlying_taint, &o.instrument_taint, &o.account_taint, &o.consumed, &o.links_out, &o.links_in, &o.event_groups)
            && (lieu_fraction, ended_on_record, places, rounding_held, open, stretches, last, opens_at_last, expired_at_end, ended_quiet)
                == (&o.lieu_fraction, &o.ended_on_record, &o.places, &o.rounding_held, &o.open, &o.stretches, &o.last, &o.opens_at_last, &o.expired_at_end, &o.ended_quiet)
    }
}

/// What a family's match held after the row a flat stretch starts at: the
/// matcher's maps, and of each holding its taint and average cost (its lots
/// are empty, and so is its round trip), and its units entry on or before the
/// day. (An instrument's last stated price is not kept: a change walked on from
/// a cut only adds rows, so every price stated after it is stated again.)
#[derive(Clone, Debug)]
pub struct Cut {
    pub day: Date,
    maps: MatchState,
    books: BTreeMap<(AccountId, InstrumentId), (Gaps, BTreeMap<Direction, Basis>)>,
    units: BTreeMap<(AccountId, InstrumentId), (Date, Fig<Dec>)>,
}

impl Cut {
    /// The bytes its entries take, by the sizes of their keys and values (the
    /// maps' own nodes and a taint's gaps beyond it).
    pub fn bytes(&self) -> usize {
        use std::mem::size_of;
        fn of<K, V>(m: &BTreeMap<K, V>) -> usize {
            m.len() * (size_of::<K>() + size_of::<V>())
        }
        fn set<K>(m: &BTreeSet<K>) -> usize {
            m.len() * size_of::<K>()
        }
        let m = &self.maps;
        of(&m.expiring)
            + m.expiring.values().map(set).sum::<usize>()
            + of(&m.underlying_taint)
            + of(&m.instrument_taint)
            + of(&m.account_taint)
            + set(&m.consumed)
            + of(&m.links_out)
            + set(&m.links_in)
            + of(&m.event_groups)
            + of(&m.lieu_fraction)
            + set(&m.ended_on_record)
            + of(&m.places)
            + of(&m.rounding_held)
            + of(&self.books)
            + self.books.values().map(|(_, b)| of(b)).sum::<usize>()
            + of(&self.units)
    }

    /// How many entries it keeps.
    pub fn entries(&self) -> usize {
        let m = &self.maps;
        m.expiring.values().map(BTreeSet::len).sum::<usize>()
            + m.underlying_taint.len()
            + m.instrument_taint.len()
            + m.account_taint.len()
            + m.consumed.len()
            + m.links_out.len()
            + m.links_in.len()
            + m.event_groups.len()
            + m.lieu_fraction.len()
            + m.ended_on_record.len()
            + m.places.len()
            + m.rounding_held.len()
            + self.books.len()
            + self.units.len()
    }
}

impl MatchState {
    /// Whether nothing of the family waits on a multi-leg order whose legs are
    /// not stated.
    pub fn untainted(&self) -> bool {
        self.underlying_taint.is_empty() && self.instrument_taint.is_empty() && self.account_taint.is_empty()
    }
}

/// What a continued match wrote.
#[derive(Clone, Debug, Default)]
pub struct Continued {
    pub trips: BTreeSet<TripKey>,
    pub holdings: BTreeSet<(AccountId, InstrumentId)>,
    /// Rows whose cashflow row reads what the walk wrote: the stock dividends it
    /// took out and wrote again, after a cut.
    pub rows: BTreeSet<TransactionId>,
}

/// What an expiry pass took off the calendar, and whether it closed or marked a
/// contract.
#[derive(Debug, Default)]
struct Expired {
    popped: BTreeMap<Date, BTreeSet<(AccountId, InstrumentId)>>,
    acted: bool,
}

/// What `txs` set up before they are walked: the transfer links each sends and
/// receives, the contracts whose record says how they ended, each coin's finest
/// place per source, and each event's rows.
fn set_up(inputs: &Inputs, txs: &[&Transaction], s: &mut MatchState) {
    let ledger = &inputs.ledger;
    for t in txs {
        if let Some(i) = ledger.transfer_links.received_by(&t.id) {
            s.links_out.insert(t.id.clone(), i.clone());
        }
        if ledger.transfer_links.sent_by(&t.id).is_some() {
            s.links_in.insert(t.id.clone());
        }
        if matches!(t.kind, Kind::OptionExpiry | Kind::OptionAssignment | Kind::OptionExercise) {
            if let Some(i) = t.instrument {
                s.ended_on_record.insert((t.account, i));
            }
        }
        if let (Some(i), Some(q)) = (t.instrument, t.quantity) {
            if ledger.instruments.get(&i).is_some_and(|x| x.instrument.kind == InstrumentKind::Crypto) {
                let e = s.places.entry((i, t.mapping.source.clone())).or_insert(0);
                *e = (*e).max(q.places());
            }
        }
    }
    // each adjustment's event: its own row and the event rows of that account and
    // day on the instruments its legs name
    let mut anchors: Vec<&Transaction> = txs.iter().filter(|t| inputs.facts.adjustments.get(&t.id).is_some()).copied().collect();
    // by the transaction each applies to: of two events naming one row, the first keeps it
    anchors.sort_by(|a, b| a.id.cmp(&b.id));
    for anchor in anchors {
        let Some(adj) = inputs.facts.adjustments.get(&anchor.id) else { continue };
        if anchor.kind != Kind::CorporateEvent {
            continue;
        }
        let named: BTreeSet<InstrumentId> = adj.legs.iter().flat_map(|l| [l.from, l.to]).flatten().collect();
        s.event_groups.insert(anchor.id.clone(), anchor.id.clone());
        for t in txs {
            if t.kind == Kind::CorporateEvent && t.account == anchor.account && t.trade_date == anchor.trade_date && t.instrument.is_some_and(|i| named.contains(&i)) {
                s.event_groups.entry(t.id.clone()).or_insert_with(|| anchor.id.clone());
            }
        }
    }
}

impl<'a, 'm> Matcher<'a, 'm> {
    fn from_state(inputs: &'a Inputs, out: &'m mut Matched, s: MatchState) -> Matcher<'a, 'm> {
        let MatchState { expiring, underlying_taint, instrument_taint, account_taint, consumed, links_out, links_in, event_groups, lieu_fraction, ended_on_record, places, rounding_held, open, stretches, cuts, .. } = s;
        Matcher {
            inputs,
            out,
            touched_trips: BTreeSet::new(),
            touched_holdings: BTreeSet::new(),
            closed_by: BTreeMap::new(),
            expiring,
            underlying_taint,
            instrument_taint,
            account_taint,
            consumed,
            links_out,
            links_in,
            event_groups,
            dirty: BTreeSet::new(),
            lieu_fraction,
            ended_on_record,
            places,
            rounding_held,
            open,
            stretches,
            cuts,
            scope: None,
        }
    }

    /// The matcher's part of the state, taken.
    fn into_state(self, last: Option<OrderKey>, opens_at_last: BTreeSet<(AccountId, InstrumentId)>) -> (MatchState, BTreeSet<TripKey>, BTreeSet<(AccountId, InstrumentId)>) {
        debug_assert!(self.dirty.is_empty(), "units recorded for every holding touched");
        let Matcher { expiring, underlying_taint, instrument_taint, account_taint, consumed, links_out, links_in, event_groups, lieu_fraction, ended_on_record, places, rounding_held, open, stretches, cuts, touched_trips, touched_holdings, .. } = self;
        (
            MatchState { expiring, underlying_taint, instrument_taint, account_taint, consumed, links_out, links_in, event_groups, lieu_fraction, ended_on_record, places, rounding_held, open, stretches, cuts, last, opens_at_last, expired_at_end: BTreeMap::new(), ended_quiet: true },
            touched_trips,
            touched_holdings,
        )
    }

    /// `txs` walked in the match's order: each record ranked, ordered and applied
    /// by all its legs, as the whole match applies it. A record whose legs share a
    /// day is applied where its first leg falls, whichever family that leg is in.
    /// The largest place walked and the stated-Open holdings at its instant, from
    /// `last` and `opens` on.
    fn walk(&mut self, txs: &[&'a Transaction], last: &mut Option<OrderKey>, opens: &mut BTreeSet<(AccountId, InstrumentId)>) {
        let ledger = &self.inputs.ledger;
        let mine: BTreeSet<&TransactionId> = txs.iter().map(|t| &t.id).collect();
        // every leg of every record of `txs`, each record whole
        let mut by_record: BTreeMap<_, Vec<&'a Transaction>> = BTreeMap::new();
        for t in txs {
            by_record.entry(t.id.record).or_insert_with(|| ledger.transactions.of_record(t.id.record).collect());
        }
        let key_of = |t: &Transaction| ledger.records.get(&t.id.record).map(|r| r.source_key.clone()).unwrap_or_default();
        let place = |t: &Transaction| order_key(t, record_rank(&by_record[&t.id.record]), &key_of(t));
        // a record whose legs share a day is applied at its first leg: where that leg
        // is not one of `txs`, it is walked past to apply the others there
        let one_day = |legs: &[&Transaction]| legs.len() > 1 && legs.iter().all(|l| l.trade_date == legs[0].trade_date);
        let mut walk: Vec<&'a Transaction> = txs.to_vec();
        for legs in by_record.values() {
            if one_day(legs) {
                if let Some(first) = legs.iter().min_by_key(|l| (place(l), l.id.clone())) {
                    if !mine.contains(&first.id) {
                        walk.push(first);
                    }
                }
            }
        }
        // in the ledger's order first, so equal places keep it
        walk.sort_by(|a, b| ledger.transactions.place_of(&a.id).cmp(&ledger.transactions.place_of(&b.id)));
        walk.sort_by_cached_key(|t| place(t));
        // the largest place walked, and the holdings of the stated-Open rows at its instant
        for t in walk.iter().filter(|t| mine.contains(&t.id)) {
            let k = place(t);
            if last.as_ref().is_none_or(|l| k > *l) {
                if last.as_ref().is_none_or(|l| (l.0, l.1, l.2) != (k.0, k.1, k.2)) {
                    opens.clear();
                }
                *last = Some(k);
            }
            if t.effect == Some(Effect::Open) && last.as_ref().is_some_and(|l| instant(t) == (l.0, l.1, l.2)) {
                if let Some(i) = t.instrument {
                    opens.insert((t.account, i));
                }
            }
        }
        // a record whose legs share a day is applied as one order where its first
        // leg falls, each next leg the first that meets an opposite position, so a
        // roll that states no effects closes the old contract before it opens the
        // new one, whichever way its legs sort
        let mut in_order: BTreeMap<_, Vec<&Transaction>> = BTreeMap::new();
        for t in walk.iter().filter(|t| mine.contains(&t.id)) {
            in_order.entry(t.id.record).or_default().push(t);
        }
        let mut done: BTreeSet<&TransactionId> = BTreeSet::new();
        for (at, t) in walk.iter().enumerate() {
            if done.contains(&t.id) {
                continue;
            }
            // at one instant a stated close that meets a position goes before a
            // stated open of the same contract, which would meet that position; an
            // open goes first only where the close needs it (a short opened and
            // covered at one instant)
            if t.effect == Some(Effect::Open) && mine.contains(&t.id) {
                let closes: Vec<&Transaction> = walk[at + 1..]
                    .iter()
                    .take_while(|c| instant(c) == instant(t))
                    .filter(|c| mine.contains(&c.id) && !done.contains(&c.id) && c.effect == Some(Effect::Close) && c.account == t.account && c.instrument == t.instrument && by_record.get(&c.id.record).is_some_and(|l| l.len() == 1))
                    .copied()
                    .collect();
                for c in closes {
                    if self.meets_opposite(c) {
                        done.insert(&c.id);
                        self.stretch_ends(&place(c));
                        self.expire_before(c.trade_date);
                        self.apply(c, by_record.get(&c.id.record).map(|v| v.as_slice()).unwrap_or(&[]));
                    }
                }
            }
            let legs = by_record.get(&t.id.record).map(|v| v.as_slice()).unwrap_or(&[]);
            // a row the walk passes over neither ends a flat stretch nor starts one
            let moves = !legs.iter().all(|l| passes_over(self.inputs, l));
            if moves {
                self.stretch_ends(&place(t));
            }
            if !one_day(legs) {
                self.expire_before(t.trade_date);
                self.apply(t, legs);
                self.record_units(t.trade_date);
                if moves {
                    self.stretch_starts(&place(t), walk.get(at + 1).map(|n| n.trade_date));
                }
                continue;
            }
            let mut left = in_order.get(&t.id.record).cloned().unwrap_or_default();
            self.expire_before(t.trade_date);
            while !left.is_empty() {
                let i = left.iter().position(|l| self.meets_opposite(l)).unwrap_or(0);
                let leg = left.remove(i);
                done.insert(&leg.id);
                self.apply(leg, legs);
            }
            self.record_units(t.trade_date);
            if moves {
                self.stretch_starts(&place(t), walk.get(at + 1).map(|n| n.trade_date));
            }
        }
    }
}

/// Whether the match walks past `t` moving nothing but cash: a row naming no
/// instrument, or a distribution no adjustment names (a return of capital is an
/// adjustment, and moves the lots; `Matcher::apply`). Its place in the walk then
/// decides nothing.
pub fn passes_over(inputs: &Inputs, t: &Transaction) -> bool {
    t.instrument.is_none() || (t.kind == Kind::Dividend && inputs.facts.adjustments.get(&t.id).is_none())
}

/// The match of `txs` alone, and what it keeps to be continued from: a set of
/// transactions no other transaction's matching reaches (a family,
/// `crate::families`), or every transaction. The lists of what the record waits
/// on are in the match's order (`place_in_match`), so the matches of several
/// families put together are the whole match.
pub fn match_family(inputs: &Inputs, txs: Vec<&Transaction>) -> (Matched, MatchState) {
    let mut out = Matched::default();
    // flat before its first row
    let mut state = MatchState { stretches: BTreeMap::from([(None, None)]), ..MatchState::default() };
    set_up(inputs, &txs, &mut state);
    let mut m = Matcher::from_state(inputs, &mut out, state);
    let (mut last, mut opens) = (None, BTreeSet::new());
    m.walk(&txs, &mut last, &mut opens);
    // what the expiry up to today does is put back before a continued walk
    let end = m.expire_before(inputs.clock.today);
    let (mut state, _, _) = m.into_state(last, opens);
    state.expired_at_end = end.popped;
    state.ended_quiet = !end.acted;
    out.in_match_order(&inputs.ledger);
    out.index();
    (out, state)
}

/// A family's match continued in `out` from `state` for `added`, rows placed
/// after everything the family matched, as the whole match of the family with
/// them would leave it (`docs/plans/stage-3-a-change-on-a-deep-history.md`, B;
/// the engine's guards decide when it may). What it wrote.
pub fn continue_family(inputs: &Inputs, out: &mut Matched, state: &mut MatchState, added: Vec<&Transaction>, scope: BTreeSet<InstrumentId>) -> Continued {
    let mut s = std::mem::take(state);
    // the calendar as it stood before the expiry up to today
    for (day, books) in std::mem::take(&mut s.expired_at_end) {
        s.expiring.entry(day).or_default().extend(books);
    }
    let (ended, events) = (s.ended_on_record.len(), s.event_groups.len());
    set_up(inputs, &added, &mut s);
    debug_assert!(s.ended_on_record.len() == ended && s.event_groups.len() == events, "a continued match adds no expiry, assignment, exercise or event");
    // a cut on the day the rows start is no clean break once they follow it
    if let Some(first) = added.iter().map(|t| t.trade_date).min() {
        s.cuts.retain(|k, _| k.0 < first);
    }
    let (last, opens) = (s.last.take(), std::mem::take(&mut s.opens_at_last));
    let (s, trips, holdings) = walk_from(inputs, out, s, &added, last, opens, scope);
    *state = s;
    Continued { trips, holdings, rows: BTreeSet::new() }
}

/// `rows` walked on from `s` into `out`, as the whole match walks them: the
/// expiry up to today after them, each row the walk adds to a list put in its
/// place in the match's order, and the indexes of what was written. The state
/// after, and the round trips and holdings written.
#[allow(clippy::type_complexity)]
fn walk_from(inputs: &Inputs, out: &mut Matched, s: MatchState, rows: &[&Transaction], mut last: Option<OrderKey>, mut opens: BTreeSet<(AccountId, InstrumentId)>, scope: BTreeSet<InstrumentId>) -> (MatchState, BTreeSet<TripKey>, BTreeSet<(AccountId, InstrumentId)>) {
    let ledger = &inputs.ledger;
    // what each list held before, so the rows added are put in their places
    let lens = (out.beyond.len(), out.dust.len(), out.arrived.len(), out.stock_dividends.len(), out.disagreements.len(), out.unit_disagreements.len(), out.unapplied.len());
    let mut m = Matcher::from_state(inputs, out, s);
    m.scope = Some(scope);
    m.walk(rows, &mut last, &mut opens);
    let end = m.expire_before(inputs.clock.today);
    let (mut s, trips, holdings) = m.into_state(last, opens);
    s.expired_at_end = end.popped;
    s.ended_quiet = !end.acted;
    // the rows added to the lists, each put in its place in the match's order
    let at = |id: &TransactionId| ledger.transactions.get(id).map(|t| place_in_match(ledger, t));
    fn place<T>(list: &mut Vec<T>, from: usize, key: impl Fn(&T) -> Option<(Date, u8, Option<Timestamp>, u8, String, u8, String, TransactionId)>) {
        let added: Vec<T> = list.drain(from..).collect();
        for row in added {
            let k = key(&row);
            let i = list.partition_point(|x| key(x) <= k);
            list.insert(i, row);
        }
    }
    place(&mut out.beyond, lens.0, |x| at(&x.transaction));
    place(&mut out.dust, lens.1, |x| at(&x.transaction));
    place(&mut out.arrived, lens.2, |x| at(&x.transaction));
    place(&mut out.stock_dividends, lens.3, |x| at(&x.transaction));
    place(&mut out.disagreements, lens.4, |x| at(x));
    place(&mut out.unit_disagreements, lens.5, |x| at(&x.transaction));
    place(&mut out.unapplied, lens.6, |x| at(&x.0));
    // the indexes, for what was written
    for k in &trips {
        out.trips_of.entry(k.instrument).or_default().insert(k.clone());
        for f in out.trips.get(k).into_iter().flat_map(|t| t.fills.iter()) {
            out.trips_by_fill.entry(f.clone()).or_default().insert(k.clone());
        }
    }
    for (a, i) in &holdings {
        if out.books.contains_key(&(*a, *i)) || out.units.contains_key(&(*a, *i)) {
            out.held_by.entry(*i).or_default().insert(*a);
        }
    }
    (s, trips, holdings)
}

/// A family's match walked on again from the cut at `cut` (`docs/plans/continuing-from-a-flat-stretch.md`,
/// B), for `added`, rows dated after the cut's day among the family's rows
/// `family`: everything the match wrote after the cut taken out of `out`
/// within the family's instruments `scope`, then the family's rows after the
/// cut walked from the cut, as the whole match walks them. What it wrote,
/// with the round trips it took out.
pub fn match_from_cut(inputs: &Inputs, out: &mut Matched, state: &mut MatchState, after: Vec<&Transaction>, added: &[&Transaction], cut: &OrderKey, scope: BTreeSet<InstrumentId>) -> Continued {
    let ledger = &inputs.ledger;
    // the cuts and stretches up to this cut, taken from the state: the walk keeps
    // its own from here on
    let mut cuts = std::mem::take(&mut state.cuts);
    let mut later_cuts = cuts.split_off(cut);
    let c = later_cuts.remove(cut).expect("a cut matched from is kept");
    let day = c.day;
    let later: BTreeSet<&TransactionId> = after.iter().map(|t| &t.id).collect();
    debug_assert!(after.iter().all(|t| t.trade_date > day), "the family's rows after the cut's day: none follows it on its day");
    // what the match wrote after the cut, taken out, within the family's holdings
    let opened_after = |k: &TripKey| ledger.transactions.get(&k.opening).is_none_or(|t| t.trade_date > day);
    let mut gone: BTreeSet<TripKey> = BTreeSet::new();
    for i in &scope {
        if let Some(ks) = out.trips_of.get_mut(i) {
            let off: Vec<TripKey> = ks.iter().filter(|k| opened_after(k)).cloned().collect();
            for k in off {
                ks.remove(&k);
                gone.insert(k);
            }
            if ks.is_empty() {
                out.trips_of.remove(i);
            }
        }
    }
    for k in &gone {
        if let Some(t) = out.trips.remove(k) {
            for f in &t.fills {
                if let Some(ks) = out.trips_by_fill.get_mut(f) {
                    ks.remove(k);
                    if ks.is_empty() {
                        out.trips_by_fill.remove(f);
                    }
                }
            }
        }
    }
    for id in &later {
        out.roles.remove(*id);
        out.waiting.remove(*id);
    }
    let tx_after = |id: &TransactionId| later.contains(id);
    out.beyond.retain(|x| !tx_after(&x.transaction));
    out.dust.retain(|x| !tx_after(&x.transaction));
    out.arrived.retain(|x| !tx_after(&x.transaction));
    let mut rows: BTreeSet<TransactionId> = out.stock_dividends.iter().filter(|x| tx_after(&x.transaction)).map(|x| x.transaction.clone()).collect();
    out.stock_dividends.retain(|x| !tx_after(&x.transaction));
    out.disagreements.retain(|x| !tx_after(x));
    out.unit_disagreements.retain(|x| !tx_after(&x.transaction));
    out.unapplied.retain(|x| !tx_after(&x.0));
    // each holding as the cut has it: flat, its taint and average cost; each
    // timeline up to the cut's day, whose last entry is the cut's
    for i in &scope {
        for a in out.held_by.remove(i).unwrap_or_default() {
            out.books.remove(&(a, *i));
            if let Some(u) = out.units.get_mut(&(a, *i)) {
                u.retain(|d, _| *d <= day);
                if u.is_empty() {
                    out.units.remove(&(a, *i));
                } else {
                    out.held_by.entry(*i).or_default().insert(a);
                }
            }
        }
    }
    for (k, (taint, basis)) in &c.books {
        // a holding with a book has a timeline before the cut too, kept above
        out.books.insert(*k, Book { taint: taint.clone(), basis: basis.clone(), ..Book::default() });
    }
    debug_assert!(c.units.iter().all(|(k, (d, v))| out.units.get(k).and_then(|u| u.iter().next_back()) == Some((d, v))), "the timelines up to the cut are the cut's");
    // the state at the cut, with the stretches up to it, its own open again
    let mut s = c.maps.clone();
    let mut stretches = std::mem::take(&mut state.stretches);
    let _later = stretches.split_off(&Some(cut.clone()));
    stretches.insert(Some(cut.clone()), None);
    s.stretches = stretches;
    // the cut itself still holds: nothing before it changed
    cuts.insert(cut.clone(), c);
    s.cuts = cuts;
    set_up(inputs, added, &mut s);
    let (s, mut trips, holdings) = walk_from(inputs, out, s, &after, None, BTreeSet::new(), scope);
    *state = s;
    trips.extend(gone);
    rows.extend(out.stock_dividends.iter().filter(|x| later.contains(&x.transaction)).map(|x| x.transaction.clone()));
    Continued { trips, holdings, rows }
}

/// A round trip added inside one of a family's flat stretches, placed there
/// without walking the rest of the family (`docs/plans/matching-from-a-flat-point.md`,
/// B): `added` walked alone, from no lots, on a scratch match, into the stretch
/// starting at `start`. Placed, and what it wrote, only where the walk wrote
/// nothing but round trips, their roles and units, and books it left flat and
/// untainted, and left every holding flat; otherwise nothing is written, and the
/// family is for matching whole. The engine's guards decide when it is tried:
/// nothing in the kept state but the stretches is read by such rows.
pub fn place_in_stretch(inputs: &Inputs, out: &mut Matched, state: &mut MatchState, added: Vec<&Transaction>, start: &Option<OrderKey>) -> Option<Continued> {
    let end = state.stretches.get(start)?.clone();
    let first_day = added.iter().map(|t| t.trade_date).min()?;
    // each holding's units as they stood when the stretch began: the entry on or
    // before the rows' first day, what a whole walk compares the first row with
    let mut scratch = Matched::default();
    let mut seeded: BTreeMap<(AccountId, InstrumentId), Date> = BTreeMap::new();
    for t in &added {
        let Some(i) = t.instrument else { continue };
        let key = (t.account, i);
        if let Some((d, v)) = out.units.get(&key).and_then(|u| u.range(..=first_day).next_back()) {
            scratch.units.entry(key).or_default().insert(*d, v.clone());
            seeded.insert(key, *d);
        }
    }
    let mut s = MatchState { stretches: BTreeMap::from([(start.clone(), None)]), ..MatchState::default() };
    set_up(inputs, &added, &mut s);
    let mut m = Matcher::from_state(inputs, &mut scratch, s);
    let (mut last, mut opens) = (None, BTreeSet::new());
    m.walk(&added, &mut last, &mut opens);
    let (walked, trips, holdings) = m.into_state(last, opens);
    // nothing but round trips, roles, units and flat books
    let clean = scratch.beyond.is_empty()
        && scratch.dust.is_empty()
        && scratch.arrived.is_empty()
        && scratch.stock_dividends.is_empty()
        && scratch.disagreements.is_empty()
        && scratch.unit_disagreements.is_empty()
        && scratch.unapplied.is_empty()
        && scratch.waiting.is_empty()
        && scratch.books.values().all(|b| b.lots.is_empty() && b.taint.is_empty())
        && walked.open.is_empty()
        && walked.consumed.is_empty()
        && walked.expiring.is_empty()
        && walked.rounding_held.is_empty()
        && walked.lieu_fraction.is_empty()
        && walked.underlying_taint.is_empty()
        && walked.instrument_taint.is_empty()
        && walked.account_taint.is_empty()
        && walked.stretches.last_key_value().is_some_and(|(_, e)| e.is_none());
    if !clean {
        return None;
    }
    let Matched { trips: written, books, units, roles, last_price, .. } = scratch;
    // the last price an instrument's fills stated, where no fill after the rows states one
    let last_day = added.iter().map(|t| t.trade_date).max().unwrap_or(first_day);
    for (i, p) in last_price {
        let info = inputs.ledger.instruments.get(&i);
        let later = inputs.ledger.transactions.of_instrument(i).any(|x| x.trade_date > last_day && fill_price(x, info).ok().is_some_and(|p| p.is_positive()));
        if !later {
            out.last_price.insert(i, p);
        }
    }
    for (k, b) in books {
        match out.books.get_mut(&k) {
            None => {
                out.books.insert(k, b);
            }
            Some(kept) => {
                for (d, basis) in b.basis {
                    match kept.basis.get_mut(&d) {
                        // what the holding realized over its life gains the stretch's
                        Some(k) => k.realized = fig_add(&k.realized, &basis.realized),
                        None => {
                            kept.basis.insert(d, basis);
                        }
                    }
                }
            }
        }
    }
    for (k, mut timeline) in units {
        if let Some(d) = seeded.get(&k) {
            timeline.remove(d);
        }
        if !timeline.is_empty() {
            out.units.entry(k).or_default().extend(timeline);
        }
    }
    out.roles.extend(roles);
    out.trips.extend(written);
    for k in &trips {
        out.trips_of.entry(k.instrument).or_default().insert(k.clone());
        for f in out.trips.get(k).into_iter().flat_map(|t| t.fills.iter()) {
            out.trips_by_fill.entry(f.clone()).or_default().insert(k.clone());
        }
    }
    for (a, i) in &holdings {
        if out.books.contains_key(&(*a, *i)) || out.units.contains_key(&(*a, *i)) {
            out.held_by.entry(*i).or_default().insert(*a);
        }
    }
    // the stretch split at the rows: what they left flat runs on to its end
    let mut stretches = walked.stretches;
    if let Some(mut tail) = stretches.last_entry() {
        *tail.get_mut() = end;
    }
    state.stretches.remove(start);
    state.stretches.extend(stretches);
    // every cut after the rows held the match without them
    state.cuts.retain(|k, _| start.as_ref().is_some_and(|st| k <= st));
    Some(Continued { trips, holdings, rows: BTreeSet::new() })
}

impl Matched {
    /// What the match of the nodes `region` holds taken out, and `fresh`, their
    /// match again, put in: every figure of the match keyed by an instrument of
    /// the region, or by a transaction of it or one no longer in the ledger.
    /// The cash totals are the caller's (`crate::engine`), by family.
    pub fn replace(&mut self, ledger: &crate::input::Ledger, region: &BTreeSet<crate::families::Node>, fresh: Matched) {
        use crate::families::{node_of, Node};
        let ins = |i: &InstrumentId| region.contains(&Node::Instrument(*i));
        let tx_in = |id: &TransactionId| ledger.transactions.get(id).is_none_or(|t| region.contains(&node_of(t)));
        self.trips.retain(|k, _| !ins(&k.instrument));
        self.trips_of.retain(|i, _| !ins(i));
        self.trips_by_fill.retain(|_, ks| {
            ks.retain(|k| !ins(&k.instrument));
            !ks.is_empty()
        });
        self.held_by.retain(|i, _| !ins(i));
        self.books.retain(|(_, i), _| !ins(i));
        self.units.retain(|(_, i), _| !ins(i));
        self.last_price.retain(|i, _| !ins(i));
        self.beyond.retain(|x| !ins(&x.instrument));
        self.dust.retain(|x| !ins(&x.instrument));
        self.arrived.retain(|x| !ins(&x.instrument));
        self.stock_dividends.retain(|x| !ins(&x.instrument));
        self.unit_disagreements.retain(|x| !ins(&x.instrument));
        self.disagreements.retain(|x| !tx_in(x));
        self.unapplied.retain(|x| !tx_in(&x.0));
        self.roles.retain(|x, _| !tx_in(x));
        self.waiting.retain(|x, _| !tx_in(x));
        // the lists kept in the match's order: each new row put in its place
        let Matched { trips, books, beyond, dust, arrived, last_price, stock_dividends, disagreements, unit_disagreements, unapplied, units, roles, waiting, trips_of, trips_by_fill, held_by } = fresh;
        let at = |id: &TransactionId| ledger.transactions.get(id).map(|t| place_in_match(ledger, t));
        fn put<T>(list: &mut Vec<T>, rows: Vec<T>, key: impl Fn(&T) -> Option<(Date, u8, Option<Timestamp>, u8, String, u8, String, TransactionId)>) {
            for row in rows {
                let k = key(&row);
                // after every row at the same place: a transaction's rows in the order found
                let i = list.partition_point(|x| key(x) <= k);
                list.insert(i, row);
            }
        }
        put(&mut self.beyond, beyond, |x| at(&x.transaction));
        put(&mut self.dust, dust, |x| at(&x.transaction));
        put(&mut self.arrived, arrived, |x| at(&x.transaction));
        put(&mut self.stock_dividends, stock_dividends, |x| at(&x.transaction));
        put(&mut self.disagreements, disagreements, |x| at(x));
        put(&mut self.unit_disagreements, unit_disagreements, |x| at(&x.transaction));
        put(&mut self.unapplied, unapplied, |x| at(&x.0));
        self.absorb(Matched { trips, books, last_price, units, roles, waiting, trips_of, trips_by_fill, held_by, ..Matched::default() });
    }

    /// The indexes worked out from what the match holds.
    fn index(&mut self) {
        self.trips_of.clear();
        self.trips_by_fill.clear();
        self.held_by.clear();
        for (k, t) in &self.trips {
            self.trips_of.entry(k.instrument).or_default().insert(k.clone());
            for f in &t.fills {
                self.trips_by_fill.entry(f.clone()).or_default().insert(k.clone());
            }
        }
        for (a, i) in self.books.keys().chain(self.units.keys()) {
            self.held_by.entry(*i).or_default().insert(*a);
        }
    }

    /// The lots still open of a round trip, wherever they are held, each with the
    /// instrument of its book.
    /// In the books' order (account, then instrument), each book's lots in order.
    pub fn lots_of(&self, trip: &Trip) -> Vec<(InstrumentId, &Lot)> {
        let held: BTreeSet<(AccountId, InstrumentId)> = trip.instruments.iter().flat_map(|i| self.held_by.get(i).into_iter().flatten().map(move |a| (*a, *i))).collect();
        let mut out = Vec::new();
        for (a, i) in held {
            if let Some(b) = self.books.get(&(a, i)) {
                out.extend(b.lots.iter().filter(|l| l.trip == trip.key).map(|l| (i, l)));
            }
        }
        out
    }

    /// Everything `fresh` holds put in beside what this holds, the cash totals
    /// aside (the caller's); the lists left for `in_match_order`.
    pub fn absorb(&mut self, fresh: Matched) {
        let Matched { trips, books, beyond, dust, arrived, last_price, stock_dividends, disagreements, unit_disagreements, unapplied, units, roles, waiting, trips_of, trips_by_fill, held_by } = fresh;
        for (i, k) in trips_of {
            self.trips_of.entry(i).or_default().extend(k);
        }
        for (t, k) in trips_by_fill {
            self.trips_by_fill.entry(t).or_default().extend(k);
        }
        for (i, a) in held_by {
            self.held_by.entry(i).or_default().extend(a);
        }
        self.trips.extend(trips);
        self.books.extend(books);
        self.beyond.extend(beyond);
        self.dust.extend(dust);
        self.arrived.extend(arrived);
        self.last_price.extend(last_price);
        self.stock_dividends.extend(stock_dividends);
        self.disagreements.extend(disagreements);
        self.unit_disagreements.extend(unit_disagreements);
        self.unapplied.extend(unapplied);
        self.units.extend(units);
        self.roles.extend(roles);
        self.waiting.extend(waiting);
    }

    /// The lists of what the record waits on in the match's order: by each
    /// transaction's place (`place_in_match`), a transaction's rows in the order
    /// its matching found them.
    pub fn in_match_order(&mut self, ledger: &crate::input::Ledger) {
        let at = |id: &TransactionId| ledger.transactions.get(id).map(|t| place_in_match(ledger, t));
        self.beyond.sort_by_cached_key(|x| at(&x.transaction));
        self.dust.sort_by_cached_key(|x| at(&x.transaction));
        self.arrived.sort_by_cached_key(|x| at(&x.transaction));
        self.stock_dividends.sort_by_cached_key(|x| at(&x.transaction));
        self.disagreements.sort_by_cached_key(|x| at(x));
        self.unit_disagreements.sort_by_cached_key(|x| at(&x.transaction));
        self.unapplied.sort_by_cached_key(|x| at(&x.0));
    }
}

impl Direction {
    /// The direction a close of this one takes: a long is closed by a sale.
    pub fn closed_by_sale(self) -> bool {
        self == Direction::Long
    }

    pub fn flip(self) -> Direction {
        self.opposite()
    }
}

#[cfg(test)]
mod fill_price_tests {
    use super::*;
    use bagholder_core::instrument::{Instrument, OptionTerms};
    use bagholder_core::{MappingVersion, SourceName};

    const U: &str = "0192a000-0000-7000-8000-00000000000";

    fn info(kind: InstrumentKind, multiplier: Option<&str>) -> InstrumentInfo {
        let id = InstrumentId::parse(&format!("{U}1")).unwrap();
        let terms = (kind == InstrumentKind::OptionContract).then(|| OptionTerms {
            underlying: InstrumentId::parse(&format!("{U}2")).unwrap(),
            expiry: "2026-10-16".parse().unwrap(),
            strike: Dec::parse("260").unwrap(),
            right: OptionRight::Call,
            multiplier: multiplier.map(|m| Dec::parse(m).unwrap()),
            source: SourceName::named("test"),
        });
        InstrumentInfo { instrument: Instrument { id, kind, currency: Currency::USD, issuer: None }, names: Vec::new(), terms }
    }

    fn fill(qty: &str, price: Option<&str>, cash: Option<&str>, fee: Option<&str>) -> Transaction {
        let usd = |v: &str| Money::new(Dec::parse(v).unwrap(), Currency::USD);
        Transaction {
            id: TransactionId::parse(&format!("{U}3/trade")).unwrap(),
            mapping: MappingVersion { source: SourceName::named("test"), version: 1 },
            account: AccountId::parse(&format!("{U}4")).unwrap(),
            occurred_at: None,
            trade_date: "2026-04-20".parse().unwrap(),
            settle_date: None,
            kind: if qty.starts_with('-') { Kind::Sell } else { Kind::Buy },
            effect: None,
            instrument: Some(InstrumentId::parse(&format!("{U}1")).unwrap()),
            quantity: Some(Dec::parse(qty).unwrap()),
            price: price.map(usd),
            cash: cash.map(usd),
            fee: fee.map(usd),
            fx_rate: None,
            paid_on: None,
            value: None,
        }
    }

    #[test]
    fn a_fill_is_priced_as_stated_else_by_its_value_over_its_units() {
        let share = info(InstrumentKind::Security, None);
        let d = |v: &str| Dec::parse(v).unwrap();
        // the broker's price stands
        assert_eq!(fill_price(&fill("80", Some("78.3"), Some("-6264"), None), Some(&share)), Ok(d("78.3")));
        // no price: the cash less the fee a buy paid, over its units
        assert_eq!(fill_price(&fill("80", None, Some("-6269"), Some("5")), Some(&share)).map(|p| p.round(2, Rounding::HalfEven)), Ok(d("78.30")));
        // a sale: the cash and the fee it paid, over its units
        assert_eq!(fill_price(&fill("-80", None, Some("7043"), Some("5")), Some(&share)).map(|p| p.round(2, Rounding::HalfEven)), Ok(d("88.10")));
        // a contract: over its units times what each is of the underlying
        let call = info(InstrumentKind::OptionContract, Some("100"));
        assert_eq!(fill_price(&fill("1", None, Some("-410"), None), Some(&call)).map(|p| p.round(2, Rounding::HalfEven)), Ok(d("4.10")));
        // what cannot be priced says why
        let unstated = info(InstrumentKind::OptionContract, None);
        assert!(fill_price(&fill("1", None, Some("-410"), None), Some(&unstated)).is_err(), "a contract whose size is unstated");
        assert!(fill_price(&fill("80", None, None, None), Some(&share)).is_err(), "neither a price nor cash");
        assert!(fill_price(&fill("0", None, Some("-10"), None), Some(&share)).is_err(), "no units");
        // a row moving no cash that states what its units were worth (a coin moved in): that worth over its units
        let mut moved = fill("0.5", None, None, None);
        moved.value = Some(Money::new(d("30000"), Currency::USD));
        assert_eq!(fill_price(&moved, Some(&share)), Ok(d("60000")));
        // stated in another currency than the instrument's, it prices nothing
        moved.value = Some(Money::new(d("30000"), Currency::CAD));
        assert!(fill_price(&moved, Some(&share)).is_err());
    }
}

#[cfg(test)]
mod share_tests {
    use super::*;

    /// A sale of millions of a coin's units at a fraction of a cent, shared over the
    /// lots it closes: the proceeds times a lot's units need more digits than a `Dec`
    /// holds before the division by the sale's units brings them back. Each share is
    /// worked from the exact value and rounded once.
    #[test]
    fn a_coin_sale_of_millions_of_units_shares_its_proceeds_over_its_lots() {
        let d = |s: &str| Dec::parse(s).unwrap();
        let proceeds = Money::new(d("727.54718203318461"), Currency::CAD);
        let sold = d("58523216.72421281");
        let (first, second) = (d("42540998.84372"), d("15982217.88049281"));
        let a = share(proceeds, first, sold).unwrap();
        let b = share(proceeds, second, sold).unwrap();
        // worked at 200 digits (Python's decimal module), each rounded half to even to 12 places
        assert_eq!(a.amount, d("528.859921960173"));
        assert_eq!(b.amount, d("198.687260073012"));
        assert_eq!(share(proceeds, sold, sold).unwrap(), proceeds);
    }
}
