//! The facts the person enters (`SPEC.md` §2, "What you enter"): the cost of units
//! that arrived without one, and what a corporate event did to cost, each derived
//! into an adjustment on the transaction it explains, a source's value for the same
//! fact replacing it (`bagholder_engine::input::Adjustments::choose`); and a trade
//! entered by hand (Add trade), derived into the transaction it is, matched as any
//! broker's. Each entry is a record whose source is the person.

use bagholder_core::account::AccountRef;
use bagholder_core::instrument::{InstrumentKind, Reference, Strength};
use bagholder_core::transaction::Kind;
use bagholder_core::{AccountId, Broker, Currency, Dec, InstrumentId, Leg, Money, SourceName, TransactionId};
use serde::{Deserialize, Serialize};

use crate::mapping::{AdjustmentDraft, AdjustmentLegDraft, Draft, InstrumentDraft, MapContext, Mapped, Mapping, NameDraft};
use crate::records::{Incoming, Stored};
use crate::{new_uuid, Book, BookError, Result};

/// One entry, as the person makes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// What units that arrived without a cost cost the person, and when they
    /// acquired them.
    CostOfArrival { arrival: TransactionId, instrument: InstrumentId, cost: Money, acquired: jiff::civil::Date },
    /// The share of the parent's cost each new holding takes, as the issuer published it.
    SpinOff { event: TransactionId, parent: InstrumentId, children: Vec<(InstrumentId, Dec)> },
    /// The capital a distribution returned per unit, as the issuer published it.
    ReturnOfCapital { distribution: TransactionId, instrument: InstrumentId, per_unit: Money },
    /// A trade entered by hand: units bought or sold on a day, in an account the
    /// book holds, at a price a unit and a fee, in the instrument's currency.
    Trade { account: AccountId, instrument: Traded, day: jiff::civil::Date, side: Side, quantity: Dec, price: Money, fee: Option<Money> },
}

/// What a trade entered by hand is of: an instrument the book holds, or one it
/// has not met, by the symbol and the currency the person gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Traded {
    Held(InstrumentId),
    /// `contract`: an option contract, by its terms; its size is left unstated,
    /// so what depends on it waits on it (`multiplier-unstated`).
    Named { symbol: String, currency: Currency, contract: Option<Contract> },
}

/// An option contract's terms, as the person enters them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    /// What it is an option on: an instrument the book holds, or a symbol it has not met.
    pub underlying: Underlying,
    pub expiry: jiff::civil::Date,
    pub strike: Dec,
    pub right: bagholder_core::instrument::OptionRight,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Underlying {
    Held(InstrumentId),
    Named(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

/// The record an entry is kept as: instruments named by a reference any source
/// can match, never by the book's own ids.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "entry", rename_all = "kebab-case", deny_unknown_fields)]
enum Payload {
    CostOfArrival { applies_to: String, instrument: Vec<(String, String)>, cost: Amount, acquired: String },
    SpinOff { applies_to: String, parent: Vec<(String, String)>, children: Vec<Child> },
    ReturnOfCapital { applies_to: String, instrument: Vec<(String, String)>, per_unit: Amount },
    Trade {
        account: (String, String),
        instrument: Vec<(String, String)>,
        kind: String,
        currency: String,
        symbol: String,
        contract: Option<ContractTerms>,
        day: String,
        side: String,
        quantity: String,
        price: Amount,
        fee: Option<Amount>,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Amount {
    amount: String,
    currency: String,
}

/// An option contract's terms as a record keeps them: its underlying named by
/// references, never the book's id.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTerms {
    pub underlying: Vec<(String, String)>,
    pub underlying_kind: String,
    pub underlying_symbol: String,
    pub expiry: String,
    pub strike: String,
    pub right: String,
}

/// An instrument as a record the book writes names it (an entry, a file's row):
/// by a reference any source can match, its kind, its currency, the symbol it
/// was given and, for a contract, its terms.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Named {
    pub refs: Vec<(String, String)>,
    pub kind: String,
    pub currency: String,
    pub symbol: String,
    pub contract: Option<ContractTerms>,
}

impl Named {
    /// The instrument as a mapping describes it, seen on `day`.
    pub fn draft(&self, day: jiff::civil::Date) -> std::result::Result<InstrumentDraft, String> {
        let currency = Currency::parse(&self.currency).map_err(|e| e.to_string())?;
        let option = match &self.contract {
            None => None,
            Some(c) => Some(crate::mapping::OptionDraft {
                underlying: Box::new(InstrumentDraft {
                    refs: refs(&c.underlying)?,
                    kind: InstrumentKind::parse(&c.underlying_kind).map_err(|e| e.to_string())?,
                    currency,
                    name: Some(NameDraft { symbol: c.underlying_symbol.clone(), venue_mic: None, venue_name: None, name: None, seen: day }),
                    option: None,
                }),
                expiry: c.expiry.parse().map_err(|e: jiff::Error| e.to_string())?,
                strike: Dec::parse(&c.strike).map_err(|e| e.to_string())?,
                right: bagholder_core::instrument::OptionRight::parse(&c.right).map_err(|e| e.to_string())?,
                // no record the book writes states a contract's size: what depends on it waits
                multiplier: None,
            }),
        };
        Ok(InstrumentDraft {
            refs: refs(&self.refs)?,
            kind: InstrumentKind::parse(&self.kind).map_err(|e| e.to_string())?,
            currency,
            name: Some(NameDraft { symbol: self.symbol.clone(), venue_mic: None, venue_name: None, name: None, seen: day }),
            option,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Child {
    instrument: Vec<(String, String)>,
    cost_share: String,
}

pub struct PersonMapping;

fn leg() -> Leg {
    Leg::named("entry")
}

fn refs(r: &[(String, String)]) -> std::result::Result<Vec<Reference>, String> {
    r.iter().map(|(s, v)| Ok(Reference::new(bagholder_core::instrument::RefScheme::parse(s).map_err(|e| e.to_string())?, v.clone()))).collect()
}

fn money(a: &Amount) -> std::result::Result<Money, String> {
    Ok(Money::new(Dec::parse(&a.amount).map_err(|e| e.to_string())?, Currency::parse(&a.currency).map_err(|e| e.to_string())?))
}

impl Mapping for PersonMapping {
    fn source(&self) -> SourceName {
        SourceName::person()
    }

    /// 2: a trade entered by hand.
    fn version(&self) -> u32 {
        2
    }

    fn map(&self, _ctx: &MapContext, payload: &str) -> Mapped {
        let p: Payload = match serde_json::from_str(payload) {
            Ok(p) => p,
            Err(e) => return Mapped::unreadable(format!("an entry that does not read: {e}")),
        };
        if let Payload::Trade { account, instrument, kind, currency, symbol, contract, day, side, quantity, price, fee } = &p {
            let read = || -> std::result::Result<Draft, String> {
                let day: jiff::civil::Date = day.parse().map_err(|e: jiff::Error| e.to_string())?;
                let q = Dec::parse(quantity).map_err(|e| e.to_string())?;
                let named = Named { refs: instrument.clone(), kind: kind.clone(), currency: currency.clone(), symbol: symbol.clone(), contract: contract.clone() };
                let instrument = named.draft(day)?;
                let (kind_tx, q) = match side.as_str() {
                    "buy" => (Kind::Buy, q),
                    "sell" => (Kind::Sell, q.neg()),
                    other => return Err(format!("a side {other:?}, neither buy nor sell")),
                };
                Ok(Draft {
                    leg: Leg::named("trade"),
                    account: AccountRef::new(Broker::parse(&account.0).map_err(|e| e.to_string())?, account.1.clone()),
                    occurred_at: None,
                    trade_date: day,
                    settle_date: None,
                    kind: kind_tx,
                    effect: None,
                    instrument: Some(instrument),
                    quantity: Some(q),
                    price: Some(money(price)?),
                    cash: None,
                    fee: fee.as_ref().map(money).transpose()?,
                    fx_rate: None,
                })
            };
            return match read() {
                Ok(d) => Mapped { legs: vec![d], problems: vec![], adjustments: vec![] },
                Err(why) => Mapped::unreadable(format!("an entry that does not read: {why}")),
            };
        }
        let read = || -> std::result::Result<AdjustmentDraft, String> {
            Ok(match p {
                Payload::CostOfArrival { applies_to, instrument, cost, acquired } => AdjustmentDraft {
                    leg: leg(),
                    applies_to: TransactionId::parse(&applies_to).map_err(|e| e.to_string())?,
                    legs: vec![AdjustmentLegDraft { to: Some(refs(&instrument)?), cost: Some(money(&cost)?), acquired: Some(acquired.parse().map_err(|e: jiff::Error| e.to_string())?), ..AdjustmentLegDraft::default() }],
                },
                Payload::SpinOff { applies_to, parent, children } => {
                    let from = refs(&parent)?;
                    let mut legs = Vec::new();
                    for c in &children {
                        legs.push(AdjustmentLegDraft { from: Some(from.clone()), to: Some(refs(&c.instrument)?), cost_share: Some(Dec::parse(&c.cost_share).map_err(|e| e.to_string())?), ..AdjustmentLegDraft::default() });
                    }
                    AdjustmentDraft { leg: leg(), applies_to: TransactionId::parse(&applies_to).map_err(|e| e.to_string())?, legs }
                }
                Payload::Trade { .. } => unreachable!("read above"),
                Payload::ReturnOfCapital { applies_to, instrument, per_unit } => {
                    let x = refs(&instrument)?;
                    AdjustmentDraft {
                        leg: leg(),
                        applies_to: TransactionId::parse(&applies_to).map_err(|e| e.to_string())?,
                        legs: vec![AdjustmentLegDraft { from: Some(x.clone()), to: Some(x), cash_per_unit: Some(money(&per_unit)?), ..AdjustmentLegDraft::default() }],
                    }
                }
            })
        };
        match read() {
            Ok(a) => Mapped { legs: vec![], problems: vec![], adjustments: vec![a] },
            Err(why) => Mapped::unreadable(format!("an entry that does not read: {why}")),
        }
    }
}

impl Book {
    /// An instrument's strongest reference, which any source that names it matches.
    fn strong_ref(&self, i: InstrumentId) -> Result<(String, String)> {
        let r = self.instrument_refs(i)?.into_iter().find(|r| r.scheme.strength() == Strength::Strong).ok_or_else(|| BookError::Refused(format!("instrument {i} has no reference a source states")))?;
        Ok((r.scheme.to_text(), r.value))
    }

    /// The id the broker states for an account: what a record the book writes
    /// places it by.
    pub fn account_ref(&self, account: AccountId) -> Result<AccountRef> {
        let a = self.account(account)?;
        let broker = self.connections()?.into_iter().find(|c| c.id == a.connection).map(|c| c.broker).ok_or_else(|| BookError::Refused(format!("account {account} has no connection")))?;
        self.account_refs(account)?.into_iter().find(|r| r.broker == broker).ok_or_else(|| BookError::Refused(format!("account {account} has no id its broker states")))
    }

    /// How a record the book writes for `account` names what was traded: an
    /// instrument the book holds by its strongest reference, one it has not met by
    /// its symbol and currency within the account's connection.
    pub fn name(&self, account: AccountId, traded: &Traded) -> Result<Named> {
        let a = self.account(account)?;
        let (refs, kind, currency, symbol, contract) = match traded {
            Traded::Held(i) => {
                let held = self.instrument(*i)?;
                let symbol = self.names(*i)?.last().map(|n| n.symbol.clone()).ok_or_else(|| BookError::Refused(format!("instrument {i} has no name to enter a trade of")))?;
                let strong = self.strong_ref(*i).or_else(|_| {
                    // one a connection names by its symbol: the same reference again
                    self.instrument_refs(*i)?.into_iter().next().map(|r| (r.scheme.to_text(), r.value)).ok_or_else(|| BookError::Refused(format!("instrument {i} has no reference")))
                })?;
                (vec![strong], held.kind, held.currency, symbol, None)
            }
            Traded::Named { symbol, currency, contract } => {
                let symbol = symbol.trim().to_uppercase();
                if symbol.is_empty() {
                    return Err(BookError::Refused("a trade of no symbol".into()));
                }
                let r = Reference::connection_symbol(a.connection, &symbol, *currency);
                let terms = match contract {
                    None => None,
                    Some(c) => {
                        if !c.strike.is_positive() {
                            return Err(BookError::Refused(format!("a strike of {}", c.strike.to_text())));
                        }
                        let (underlying, underlying_kind, underlying_symbol) = match &c.underlying {
                            Underlying::Held(u) => {
                                let held = self.instrument(*u)?;
                                if held.currency != *currency {
                                    return Err(BookError::Refused(format!("a contract in {currency} on an instrument priced in {}", held.currency)));
                                }
                                let name = self.names(*u)?.last().map(|n| n.symbol.clone()).ok_or_else(|| BookError::Refused(format!("instrument {u} has no name")))?;
                                let strong = self.strong_ref(*u).or_else(|_| self.instrument_refs(*u)?.into_iter().next().map(|r| (r.scheme.to_text(), r.value)).ok_or_else(|| BookError::Refused(format!("instrument {u} has no reference"))))?;
                                (vec![strong], held.kind, name)
                            }
                            Underlying::Named(u) => {
                                let u = u.trim().to_uppercase();
                                let r = Reference::connection_symbol(a.connection, &u, *currency);
                                (vec![(r.scheme.to_text(), r.value)], InstrumentKind::Security, u)
                            }
                        };
                        Some(ContractTerms { underlying, underlying_kind: underlying_kind.as_str().to_string(), underlying_symbol, expiry: c.expiry.to_string(), strike: c.strike.to_text(), right: c.right.as_str().to_string() })
                    }
                };
                let kind = if terms.is_some() { InstrumentKind::OptionContract } else { InstrumentKind::Security };
                (vec![(r.scheme.to_text(), r.value)], kind, *currency, symbol, terms)
            }
        };
        Ok(Named { refs, kind: kind.as_str().to_string(), currency: currency.to_string(), symbol, contract })
    }

    /// Keep an entry of the person's, each its own record. An entry the book
    /// cannot stand behind is refused, named: a cost or an amount in another
    /// currency than the instrument's, a share of cost outside (0, 1], a
    /// negative amount, children whose shares add past the whole.
    pub fn enter(&self, entry: &Entry, at: jiff::Timestamp) -> Result<Stored> {
        let refused = |why: String| Err(BookError::Refused(why));
        let currency_of = |i: InstrumentId| -> Result<Currency> { Ok(self.instrument(i)?.currency) };
        let payload = match entry {
            Entry::CostOfArrival { arrival, instrument, cost, acquired } => {
                if cost.currency != currency_of(*instrument)? || cost.amount.is_negative() {
                    return refused(format!("a cost of {} {} for an instrument priced in {}", cost.amount.to_text(), cost.currency, currency_of(*instrument)?));
                }
                Payload::CostOfArrival {
                    applies_to: arrival.to_string(),
                    instrument: vec![self.strong_ref(*instrument)?],
                    cost: Amount { amount: cost.amount.to_text(), currency: cost.currency.to_string() },
                    acquired: acquired.to_string(),
                }
            }
            Entry::SpinOff { event, parent, children } => {
                let mut total = Dec::ZERO;
                let mut out = Vec::new();
                for (child, share) in children {
                    if !share.is_positive() || *share > Dec::ONE {
                        return refused(format!("a share of cost of {} (it is a part of the whole, above 0 and at most 1)", share.to_text()));
                    }
                    total = total.checked_add(*share).map_err(|e| BookError::Refused(e.to_string()))?;
                    out.push(Child { instrument: vec![self.strong_ref(*child)?], cost_share: share.to_text() });
                }
                if total > Dec::ONE {
                    return refused(format!("new holdings taking {} of the parent's cost, more than the whole", total.to_text()));
                }
                Payload::SpinOff { applies_to: event.to_string(), parent: vec![self.strong_ref(*parent)?], children: out }
            }
            Entry::ReturnOfCapital { distribution, instrument, per_unit } => {
                if per_unit.currency != currency_of(*instrument)? || !per_unit.amount.is_positive() {
                    return refused(format!("capital returned of {} {} per unit of an instrument priced in {}", per_unit.amount.to_text(), per_unit.currency, currency_of(*instrument)?));
                }
                Payload::ReturnOfCapital { applies_to: distribution.to_string(), instrument: vec![self.strong_ref(*instrument)?], per_unit: Amount { amount: per_unit.amount.to_text(), currency: per_unit.currency.to_string() } }
            }
            Entry::Trade { account, instrument, day, side, quantity, price, fee } => {
                let (r, named) = (self.account_ref(*account)?, self.name(*account, instrument)?);
                let currency = Currency::parse(&named.currency).map_err(|e| BookError::Refused(e.to_string()))?;
                if !quantity.is_positive() {
                    return refused(format!("a quantity of {} (units bought or sold are more than none)", quantity.to_text()));
                }
                if price.currency != currency || price.amount.is_negative() {
                    return refused(format!("a price of {} {} for an instrument priced in {currency}", price.amount.to_text(), price.currency));
                }
                if let Some(f) = fee {
                    if f.currency != currency || f.amount.is_negative() {
                        return refused(format!("a fee of {} {} on a trade in {currency}", f.amount.to_text(), f.currency));
                    }
                }
                Payload::Trade {
                    account: (r.broker.to_string(), r.value),
                    instrument: named.refs,
                    kind: named.kind,
                    currency: named.currency,
                    symbol: named.symbol,
                    contract: named.contract,
                    day: day.to_string(),
                    side: match side {
                        Side::Buy => "buy",
                        Side::Sell => "sell",
                    }
                    .into(),
                    quantity: quantity.to_text(),
                    price: Amount { amount: price.amount.to_text(), currency: price.currency.to_string() },
                    fee: fee.map(|f| Amount { amount: f.amount.to_text(), currency: f.currency.to_string() }),
                }
            }
        };
        let text = serde_json::to_string(&payload).map_err(|e| BookError::Refused(e.to_string()))?;
        let key = new_uuid(at).to_string();
        self.store(&PersonMapping, &Incoming { connection: None, source_key: &key, payload: &text, refs: vec![] }, at)
    }
}
