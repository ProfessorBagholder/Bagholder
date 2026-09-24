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
    /// Every opening, in order: the candidates for its trade id.
    pub openings: Vec<TripKey>,
}

/// A transaction that took out more than the account held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Beyond {
    pub transaction: TransactionId,
    pub account: AccountId,
    pub instrument: InstrumentId,
    /// What was left once the lots ran out.
    pub qty: Dec,
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

/// The lots of one account in one instrument.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Book {
    pub lots: VecDeque<Lot>,
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

/// The whole match.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Matched {
    pub trips: BTreeMap<TripKey, Trip>,
    pub books: BTreeMap<(AccountId, InstrumentId), Book>,
    pub beyond: Vec<Beyond>,
    pub stock_dividends: Vec<StockDividend>,
    /// Fills whose stated price and cash disagree: the cash stands, and the
    /// record is a problem for the person.
    pub disagreements: Vec<TransactionId>,
    /// Transactions that moved nothing, and what they wait on.
    pub unapplied: Vec<(TransactionId, Gaps)>,
    /// Each holding's units (longs less shorts) at the end of each day they
    /// changed: what the equity series values.
    pub units: BTreeMap<(AccountId, InstrumentId), BTreeMap<Date, Dec>>,
}

impl Matched {
    /// Units of a holding at the end of `day`.
    pub fn units_on(&self, account: AccountId, instrument: InstrumentId, day: Date) -> Dec {
        self.units.get(&(account, instrument)).and_then(|t| t.range(..=day).next_back()).map(|(_, q)| *q).unwrap_or(Dec::ZERO)
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
    Ok(total.checked_mul(part)?.div_rounded(whole, SHARE_PLACES, Rounding::HalfEven)?)
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
/// from another currency is not compared: the stated rate is itself rounded.
/// Unknown (no multiplier) is not a disagreement.
pub fn price_disagrees(t: &Transaction, qty: Dec, acquiring: bool, currency: Currency, mult: &Fig<Dec>) -> bool {
    let (Some(price), Some(cash)) = (t.price, t.cash) else { return false };
    if price.currency != currency || cash.currency != currency || t.fee.is_some_and(|f| f.currency != currency) {
        return false;
    }
    let (Ok(m), Ok(by_cash)) = (mult, fill_value(t, qty, acquiring, currency, mult)) else { return false };
    let places = currency.minor_units().max(cash.amount.places()).max(t.fee.map(|f| f.amount.places()).unwrap_or(0));
    match price.amount.checked_mul(qty).and_then(|v| v.checked_mul(*m)) {
        Ok(by_price) => by_price.round(places, Rounding::HalfEven) != by_cash.amount,
        Err(_) => true,
    }
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
/// - then the record's source key, never when it was stored;
/// - within a record, its closing legs before its opening ones (a roll closes
///   the old contract, then opens the new), then the legs' content and name.
fn order_key(t: &Transaction, record_rank: u8, source_key: &str) -> (Date, u8, Option<Timestamp>, u8, String, u8, String) {
    let bucket = match (t.occurred_at, acquires(t)) {
        (None, Some(true)) => 0,
        (Some(_), _) => 1,
        (None, _) => 2,
    };
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

struct Matcher<'a> {
    inputs: &'a Inputs,
    out: Matched,
    /// Contracts with open lots, by expiry.
    expiring: BTreeMap<Date, BTreeSet<(AccountId, InstrumentId)>>,
    /// What every contract on an underlying in an account waits on: a multi-leg
    /// order whose legs are not stated.
    underlying_taint: BTreeMap<(AccountId, InstrumentId), Gaps>,
    /// Transactions already applied as part of another (an event's rows, a
    /// linked transfer's receiving side).
    consumed: BTreeSet<TransactionId>,
    /// Each transfer out's linked receiving side.
    links_out: BTreeMap<TransactionId, TransactionId>,
    links_in: BTreeSet<TransactionId>,
    /// Each event row, the adjustment it is explained by.
    event_groups: BTreeMap<TransactionId, TransactionId>,
    /// Holdings touched since their units were last recorded.
    dirty: BTreeSet<(AccountId, InstrumentId)>,
    /// Contracts whose record has an expiry, assignment or exercise row, on any
    /// day: the broker's own row says how they ended.
    ended_on_record: BTreeSet<(AccountId, InstrumentId)>,
}

impl<'a> Matcher<'a> {
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

    fn book(&mut self, account: AccountId, instrument: InstrumentId) -> &mut Book {
        let under = self.underlying(instrument).and_then(|u| self.underlying_taint.get(&(account, u)).cloned());
        self.dirty.insert((account, instrument));
        let book = self.out.books.entry((account, instrument)).or_default();
        if let Some(g) = under {
            book.taint.merge(&g);
        }
        book
    }

    fn trip_mut(&mut self, key: &TripKey) -> &mut Trip {
        self.out.trips.get_mut(key).expect("a lot's round trip exists")
    }

    /// The round trip a new lot joins: the one given (a roll), else the one open
    /// in this holding, else a new one keyed by this opening.
    fn trip_for(&mut self, account: AccountId, instrument: InstrumentId, direction: Direction, deposited: bool, opening: &TransactionId, joins: Option<TripKey>) -> TripKey {
        let book = self.book(account, instrument);
        let current = if deposited { book.deposit_trip.clone() } else { book.trip.clone() };
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
    fn open_lot(&mut self, account: AccountId, instrument: InstrumentId, opened_by: &TransactionId, day: Date, at: Option<Timestamp>, direction: Direction, qty: Dec, value: Fig<Money>, fee: Money, flags: BTreeSet<Flag>, joins: Option<TripKey>) {
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

    /// The holding's current trips cleared once none of their lots remain.
    fn settle(&mut self, account: AccountId, instrument: InstrumentId) {
        let book = self.book(account, instrument);
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
        self.settle(account, instrument);
        Ok(Closed { left, value_left, fee_left })
    }

    /// Take `qty` of longs off the front of a holding with their cost and no
    /// P&L; returns the lots taken (for a linked transfer) and what could not be
    /// taken.
    fn take(&mut self, account: AccountId, instrument: InstrumentId, qty: Dec) -> Result<(Vec<Lot>, Dec), Gaps> {
        let mut left = qty;
        let mut taken = Vec::new();
        let mut emptied = Vec::new();
        let book = self.book(account, instrument);
        while left.is_positive() {
            let Some(front) = book.lots.front_mut() else { break };
            if front.direction != Direction::Long {
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
        self.settle(account, instrument);
        for trip in emptied {
            self.trip_mut(&trip).open_lots -= 1;
        }
        Ok((taken, left))
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

    /// A transaction took out more than was held: what it did close is waiting on
    /// what the rest was. The holding it emptied is flat afterwards, so nothing
    /// later waits on it.
    fn beyond(&mut self, t: &Transaction, account: AccountId, instrument: InstrumentId, qty: Dec) {
        self.out.beyond.push(Beyond { transaction: t.id.clone(), account, instrument, qty });
        let gap = Gap::BeyondHeld(t.id.clone());
        let closer = Closer::Transaction(t.id.clone());
        for trip in self.out.trips.values_mut().filter(|tr| tr.closed.contains(&t.id)) {
            for s in trip.slices.iter_mut().filter(|s| s.closed_by == closer && s.instrument == instrument && s.account == account) {
                s.taint.add(gap.clone());
            }
        }
    }

    /// Every contract past its expiry before `day` with lots still open: closed
    /// at zero on its expiry date when its underlying's close that day shows it
    /// expired out of the money by its terms, otherwise left waiting for the
    /// broker's row.
    fn expire_before(&mut self, day: Date) {
        while let Some((&expiry, _)) = self.expiring.iter().next() {
            if expiry >= day {
                break;
            }
            let books = self.expiring.remove(&expiry).unwrap_or_default();
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
                if !self.expired_worthless(instrument, expiry) {
                    self.taint(account, instrument, &Gaps::of(Gap::NoExpiryRecord(instrument)));
                    continue;
                }
                let currency = self.currency(instrument);
                let flags = BTreeSet::from([Flag::NoExpiryRecord]);
                for (direction, qty) in lots {
                    // nothing is shared out of a zero value or fee, so this cannot fail
                    let _ = self.close(account, instrument, direction, qty, Ok(Money::zero(currency)), Money::zero(currency), &Closer::Expiry, expiry, None, &flags);
                }
            }
            self.record_units(expiry);
        }
    }

    /// The units of every holding touched since the last record, at the end of `day`.
    fn record_units(&mut self, day: Date) {
        for key in std::mem::take(&mut self.dirty) {
            let net = self.out.books.get(&key).map(|b| {
                b.lots.iter().fold(Dec::ZERO, |a, l| match l.direction {
                    Direction::Long => a.checked_add(l.qty).unwrap_or(a),
                    Direction::Short => a.checked_sub(l.qty).unwrap_or(a),
                })
            });
            let timeline = self.out.units.entry(key).or_default();
            let net = net.unwrap_or(Dec::ZERO);
            if timeline.values().next_back() != Some(&net) {
                timeline.insert(day, net);
            }
        }
    }

    /// Whether a contract's underlying closed on its expiry day where the
    /// contract is worth nothing: a call's underlying under its strike, a put's over.
    fn expired_worthless(&self, instrument: InstrumentId, expiry: Date) -> bool {
        let Some(terms) = self.info(instrument).and_then(|i| i.terms.as_ref()) else { return false };
        let Some(close) = self.inputs.market.closes.get(&terms.underlying).and_then(|c| c.get(&expiry)) else { return false };
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
        let Some(instrument) = t.instrument else { return };
        let account = t.account;
        let mv = read_move(t);
        match mv {
            Move::Nothing => return,
            Move::Event => return self.apply_event(t),
            _ => {}
        }
        let multi_leg = self.inputs.ledger.records.get(&t.id.record).is_some_and(|r| r.problems.iter().any(|p| p.code == "leg-unstated"));
        if multi_leg && self.is_option(instrument) {
            // any contract on this underlying in this account may be a leg of it
            let gaps = Gaps::of(Gap::LegUnstated(t.id.clone()));
            if let Some(under) = self.underlying(instrument) {
                self.underlying_taint.entry((account, under)).or_default().merge(&gaps);
                let affected: Vec<InstrumentId> = self.out.books.keys().filter(|(a, i)| *a == account && self.underlying(*i) == Some(under)).map(|(_, i)| *i).collect();
                for i in affected {
                    self.taint(account, i, &gaps);
                }
            }
            self.taint(account, instrument, &gaps);
            return self.unapplied(t, gaps);
        }
        let Some(q) = t.quantity else {
            if mv == Move::Expire {
                return self.expire_all(t, instrument);
            }
            let gaps = Gaps::of(Gap::QuantityUnstated(t.id.clone()));
            self.taint(account, instrument, &gaps);
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
                }
            }
            Move::Resolve => {
                let value = charge(fill_value(t, qty, false, currency, &Ok(Dec::ONE)));
                match self.close(account, instrument, Direction::Long, qty, value, fee, &closer, t.trade_date, t.occurred_at, &none) {
                    Ok(c) if c.left.is_positive() => self.beyond(t, account, instrument, c.left),
                    Ok(_) => {}
                    Err(g) => self.taint(account, instrument, &g),
                }
            }
            Move::Event | Move::Nothing => {}
        }
    }

    /// An expiry that states no quantity closes the whole holding of the contract.
    fn expire_all(&mut self, t: &Transaction, instrument: InstrumentId) {
        let account = t.account;
        let currency = self.currency(instrument);
        let lots: Vec<(Direction, Dec)> = self.book(account, instrument).lots.iter().map(|l| (l.direction, l.qty)).collect();
        let closer = Closer::Transaction(t.id.clone());
        for (d, q) in lots {
            let _ = self.close(account, instrument, d, q, Ok(Money::zero(currency)), Money::zero(currency), &closer, t.trade_date, t.occurred_at, &BTreeSet::new());
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
        let may_close = effect != Some(Effect::Open);
        let Closed { left, value_left, fee_left } = if may_close {
            match self.close(account, instrument, closes, qty, value, fee, &closer, t.trade_date, t.occurred_at, &none) {
                Ok(r) => r,
                Err(g) => return self.taint(account, instrument, &g),
            }
        } else {
            Closed { left: qty, value_left: value, fee_left: fee }
        };
        if effect == Some(Effect::Open) && self.book(account, instrument).lots.iter().any(|l| l.direction == closes) {
            // the record says it opens, and it meets an opposite position
            let gaps = Gaps::of(Gap::EffectConflict(t.id.clone()));
            self.taint(account, instrument, &gaps);
        }
        if !left.is_positive() {
            return;
        }
        // what is left opens a position: an option's net position either way; a
        // share, coin or event contract only long, unless the record says it opens
        let may_open = match effect {
            Some(Effect::Close) => false,
            Some(Effect::Open) => true,
            None => option || acquiring,
        };
        if !may_open {
            return self.beyond(t, account, instrument, left);
        }
        let joins = if option { self.rolled_from(t, instrument, opens, record_legs) } else { None };
        let mut flags = BTreeSet::new();
        if joins.is_some() {
            flags.insert(Flag::Rolled);
        }
        self.open_lot(account, instrument, &t.id, t.trade_date, t.occurred_at, opens, left, value_left, fee_left, flags, joins);
    }

    /// The round trip a roll's opening leg continues: the one closed by another
    /// leg of the same record, on a contract of the same underlying, the same way.
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
            if let Some(tr) = self.out.trips.values().find(|tr| tr.closed.contains(&leg.id) && tr.instruments.contains(&other)) {
                return Some(tr.key.clone());
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
            // what the asset cost: the person's stated cost, else not on the record
            let stated = self.inputs.facts.adjustments.get(&t.id).and_then(|a| a.legs.iter().find(|l| l.from.is_none() && l.to == Some(instrument) && l.cost.is_some()).cloned());
            let (value, day) = match stated {
                Some(leg) => (Ok(leg.cost.expect("filtered")), leg.acquired.unwrap_or(t.trade_date)),
                None => (Err(Gaps::of(Gap::BasisUnknown(t.id.clone()))), t.trade_date),
            };
            let value = match value {
                Ok(v) if v.currency != currency => Err(Gaps::of(Gap::CurrencyUnstated(t.id.clone()))),
                v => v,
            };
            let flags = BTreeSet::from([Flag::Deposited]);
            self.open_lot(account, instrument, &t.id, day, t.occurred_at, Direction::Long, qty, value, Money::zero(currency), flags, None);
            return;
        }
        let link = self.link_of(t, instrument);
        // what the holding waits on goes with what leaves it
        let waiting = self.book(account, instrument).taint.clone();
        match self.take(account, instrument, qty) {
            Ok((lots, left)) => {
                if left.is_positive() {
                    self.beyond(t, account, instrument, left);
                }
                // the person's own accounts: the lots move with their cost and dates
                let Some((to, dest, dest_instr)) = link else { return };
                self.consumed.insert(to.clone());
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
                } else {
                    // part of it moved: a round trip of its own in the receiving
                    // account, each lot keeping its cost and the day it was bought
                    for mut l in lots {
                        l.flags.insert(Flag::Transferred);
                        let own = TripKey { opening: to.clone(), instrument: dest_instr };
                        self.open_lot(dest, dest_instr, &to, l.day, l.at, l.direction, l.qty, l.value, l.fee, l.flags, Some(own));
                    }
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

    /// A transfer out's linked receiving side: its id, account and instrument.
    fn link_of(&self, t: &Transaction, instrument: InstrumentId) -> Option<(TransactionId, AccountId, InstrumentId)> {
        let to = self.links_out.get(&t.id)?;
        let recv = self.inputs.ledger.transactions.iter().find(|x| &x.id == to)?;
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
            let unknown = Gaps::of(Gap::EventUnknown(t.id.clone()));
            self.taint(account, instrument, &unknown);
            if let Some(q) = t.quantity.filter(|q| !q.is_zero()) {
                if q.is_positive() {
                    let currency = self.currency(instrument);
                    let flags = BTreeSet::from([Flag::FromEvent]);
                    self.open_lot(account, instrument, &t.id, t.trade_date, t.occurred_at, Direction::Long, q, Err(unknown.clone()), Money::zero(currency), flags, None);
                } else if let Ok((_, left)) = self.take(account, instrument, q.abs()) {
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
                    *e = e.checked_add(q).unwrap_or(*e);
                }
            }
            m
        };
        let day = t.trade_date;
        let at = t.occurred_at;
        // each holding's lots' cost before the event: every leg's share of a cost
        // is a share of the cost as it stood, whatever the legs before it moved
        let before: BTreeMap<InstrumentId, Vec<Fig<Money>>> = adjustment
            .legs
            .iter()
            .filter_map(|l| l.from)
            .map(|i| (i, self.book(account, i).lots.iter().filter(|l| l.direction == Direction::Long).map(|l| l.value.clone()).collect()))
            .collect();
        for leg in &adjustment.legs {
            if let Err(g) = self.apply_leg(account, &anchor, day, at, leg, &stated, &before) {
                for i in [leg.from, leg.to].into_iter().flatten() {
                    self.taint(account, i, &g);
                }
                self.unapplied(t, g);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_leg(&mut self, account: AccountId, anchor: &TransactionId, day: Date, at: Option<Timestamp>, leg: &AdjustmentLeg, stated: &BTreeMap<InstrumentId, Dec>, before: &BTreeMap<InstrumentId, Vec<Fig<Money>>>) -> Result<(), Gaps> {
        let unknown = || Gaps::of(Gap::EventUnknown(anchor.clone()));
        let Some(from) = leg.from else {
            // a stated cost belongs to a deposit, applied where the deposit is
            return Ok(());
        };
        let held: Dec = dec_sum(self.book(account, from).lots.iter().filter(|l| l.direction == Direction::Long).map(|l| l.qty))?;
        match (leg.to, leg.cash_per_unit) {
            // a split or consolidation, a stock dividend, a return of capital
            (Some(to), None) if to == from => {
                if let Some(cost) = leg.cost {
                    // a stock dividend: new units at the stated value, which is income
                    let units = match stated.get(&from) {
                        Some(q) if q.is_positive() => *q,
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
                    Some(q) if !q.is_zero() => held.checked_add(*q)?,
                    _ => held.checked_mul(leg.units_per_unit.ok_or_else(unknown)?)?,
                };
                self.rescale(account, from, held, new_total)?;
                Ok(())
            }
            // a return of capital: cash per unit taken off the cost
            (Some(to), Some(cash)) if to == from => self.return_capital(account, from, cash, anchor),
            // a merger for shares and cash: the cash per unit comes off the cost,
            // and the holding continues as the new shares
            (Some(_), Some(cash)) => {
                self.return_capital(account, from, cash, anchor)?;
                let shares = AdjustmentLeg { cash_per_unit: None, ..leg.clone() };
                self.apply_leg(account, anchor, day, at, &shares, stated, before)
            }
            // the holding continues as another instrument, or a spin-off's child
            (Some(to), None) => {
                let units = match stated.get(&to) {
                    Some(q) if q.is_positive() => *q,
                    _ => held.checked_mul(leg.units_per_unit.ok_or_else(unknown)?)?,
                };
                let cost_share = leg.cost_share.unwrap_or(Dec::ONE);
                if cost_share == Dec::ONE {
                    // the same holding under a new instrument: its lots move whole
                    let (lots, _) = self.take(account, from, held)?;
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
                    return Ok(());
                }
                // a spin-off: the child takes part of each lot's cost and keeps its dates
                let currency = self.currency(to);
                let costs_before = before.get(&from).cloned().unwrap_or_default();
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
                    self.open_lot(account, to, anchor, pday, pat, Direction::Long, child_qty, value, Money::zero(currency), flags, Some(own));
                    child_left = child_left.checked_sub(child_qty)?;
                    held_left = held_left.checked_sub(pq)?;
                }
                let book = self.book(account, from);
                for (lot, moved) in book.lots.iter_mut().filter(|l| l.direction == Direction::Long).zip(moved_values) {
                    lot.value = fig_sub(&lot.value, &moved);
                }
                Ok(())
            }
            // a merger for cash, cash in lieu: units out at the cash per unit; how
            // many is the leg's share of the holding as it stands, else the units
            // the broker states went out, never the whole holding by default
            (None, Some(cash)) => {
                let out = match (leg.units_per_unit, stated.get(&from)) {
                    (Some(share), _) => held.checked_mul(share)?,
                    (None, Some(q)) if q.is_negative() => q.abs(),
                    _ => return Err(unknown()),
                };
                let currency = self.currency(from);
                if cash.currency != currency {
                    return Err(Gaps::of(Gap::CurrencyUnstated(anchor.clone())));
                }
                let value = cash.times(out)?;
                let closer = Closer::Transaction(anchor.clone());
                let left = self.close(account, from, Direction::Long, out, Ok(value), Money::zero(currency), &closer, day, at, &BTreeSet::new())?.left;
                if left.is_positive() {
                    self.out.beyond.push(Beyond { transaction: anchor.clone(), account, instrument: from, qty: left });
                }
                Ok(())
            }
            (None, None) => Err(unknown()),
        }
    }

    /// Cash per unit paid back on a holding: taken off each lot's cost. A cost
    /// that would go below nothing is a gain the record does not describe: a gap.
    fn return_capital(&mut self, account: AccountId, instrument: InstrumentId, cash: Money, anchor: &TransactionId) -> Result<(), Gaps> {
        let book = self.book(account, instrument);
        for lot in book.lots.iter_mut().filter(|l| l.direction == Direction::Long) {
            let back = cash.times(lot.qty)?;
            lot.value = match &lot.value {
                Ok(v) if v.currency != back.currency => Err(Gaps::of(Gap::CurrencyUnstated(anchor.clone()))),
                Ok(v) => match v.checked_sub(back)? {
                    left if left.amount.is_negative() => Err(Gaps::of(Gap::Arithmetic(format!("{anchor} pays back more than the units cost")))),
                    left => Ok(left),
                },
                Err(g) => Err(g.clone()),
            };
        }
        Ok(())
    }

    /// A holding's lots rescaled from `held` units to `new_total`, each in
    /// proportion, the last taking what is left; the cost stays.
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

/// Lots holding `held` units in all rescaled to `total`, each in proportion, the
/// last long lot taking what is left.
fn scale_lots(lots: Vec<Lot>, held: Dec, total: Dec) -> Result<Vec<Lot>, Gaps> {
    let mut left = total;
    let mut held_left = held;
    let n = lots.len();
    let mut out = Vec::with_capacity(n);
    for (i, mut l) in lots.into_iter().enumerate() {
        if l.direction == Direction::Long {
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
    let ledger = &inputs.ledger;
    let mut by_record: BTreeMap<_, Vec<&Transaction>> = BTreeMap::new();
    for t in &ledger.transactions {
        by_record.entry(t.id.record).or_default().push(t);
    }
    let key_of = |t: &Transaction| ledger.records.get(&t.id.record).map(|r| r.source_key.clone()).unwrap_or_default();
    let mut txs: Vec<&Transaction> = ledger.transactions.iter().collect();
    txs.sort_by_cached_key(|t| order_key(t, record_rank(&by_record[&t.id.record]), &key_of(t)));

    let mut m = Matcher {
        inputs,
        out: Matched::default(),
        expiring: BTreeMap::new(),
        underlying_taint: BTreeMap::new(),
        consumed: BTreeSet::new(),
        links_out: ledger.transfer_links.iter().cloned().collect(),
        links_in: ledger.transfer_links.iter().map(|(_, i)| i.clone()).collect(),
        event_groups: BTreeMap::new(),
        dirty: BTreeSet::new(),
        ended_on_record: ledger
            .transactions
            .iter()
            .filter(|t| matches!(t.kind, Kind::OptionExpiry | Kind::OptionAssignment | Kind::OptionExercise))
            .filter_map(|t| t.instrument.map(|i| (t.account, i)))
            .collect(),
    };
    // each adjustment's event: its own row and the event rows of that account and
    // day on the instruments its legs name
    for (applies_to, adj) in &inputs.facts.adjustments {
        let Some(anchor) = ledger.transactions.iter().find(|t| &t.id == applies_to) else { continue };
        if anchor.kind != Kind::CorporateEvent {
            continue;
        }
        let named: BTreeSet<InstrumentId> = adj.legs.iter().flat_map(|l| [l.from, l.to]).flatten().collect();
        m.event_groups.insert(anchor.id.clone(), anchor.id.clone());
        for t in &ledger.transactions {
            if t.kind == Kind::CorporateEvent && t.account == anchor.account && t.trade_date == anchor.trade_date && t.instrument.is_some_and(|i| named.contains(&i)) {
                m.event_groups.entry(t.id.clone()).or_insert_with(|| anchor.id.clone());
            }
        }
    }
    // a record whose legs share a day is applied as one order where its first
    // leg falls, each next leg the first that meets an opposite position, so a
    // roll that states no effects closes the old contract before it opens the
    // new one, whichever way its legs sort
    let mut in_order: BTreeMap<_, Vec<&Transaction>> = BTreeMap::new();
    for t in &txs {
        in_order.entry(t.id.record).or_default().push(t);
    }
    let mut done: BTreeSet<&TransactionId> = BTreeSet::new();
    for t in &txs {
        if done.contains(&t.id) {
            continue;
        }
        let legs = by_record.get(&t.id.record).map(|v| v.as_slice()).unwrap_or(&[]);
        let one_day = legs.len() > 1 && legs.iter().all(|l| l.trade_date == t.trade_date);
        if !one_day {
            m.expire_before(t.trade_date);
            m.apply(t, legs);
            m.record_units(t.trade_date);
            continue;
        }
        let mut left = in_order[&t.id.record].clone();
        m.expire_before(t.trade_date);
        while !left.is_empty() {
            let i = left.iter().position(|l| m.meets_opposite(l)).unwrap_or(0);
            let leg = left.remove(i);
            done.insert(&leg.id);
            m.apply(leg, legs);
        }
        m.record_units(t.trade_date);
    }
    m.expire_before(inputs.clock.today);
    m.out
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
