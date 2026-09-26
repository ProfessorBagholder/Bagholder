//! What the person enters (`SPEC.md` §2, "What you enter"; `docs/plans/stage-3c-switch.md`,
//! §5): a trade by hand (Add trade), the cost of units that arrived without one
//! (an opening balance), a spin-off's allocation and a distribution's return of
//! capital. Each is read strictly from the page, named by the ids the figures
//! carry, kept in the book as the person's record (`Book::enter`) and applied to
//! the figures (`Figures::record_changed`).

use serde::Deserialize;
use ts_rs::TS;

use bagholder_book::person::{Contract, Entry, Side, Traded, Underlying};
use bagholder_core::account::{AccountKind, AccountRef, AccountStatus, AccountType, Registration};
use bagholder_core::instrument::{InstrumentKind, OptionRight};
use bagholder_core::{AccountId, Broker, Currency, Dec, InstrumentId, Money, TransactionId};
use bagholder_engine::Engine;

use crate::figures::Figures;

/// One entry, as the page sends it. Amounts are decimal text in the instrument's
/// currency; ids are the figures'.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(tag = "entry", rename_all = "kebab-case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum EntryRequest {
    /// Add trade. `account` empty is the Manual account; `instrument` names one the
    /// book holds, else the book is asked for `symbol` in `currency`.
    Trade { account: String, instrument: Option<String>, symbol: String, currency: String, day: String, side: String, quantity: String, price: String, fee: String },
    /// An opening balance: what the units `arrival` brought in cost, in total.
    CostOfArrival { arrival: String, cost: String, acquired: String },
    /// Each new holding's share of the parent's cost, as the issuer published it:
    /// `event` the transaction that brought a new holding in, `parent` the holding
    /// it came out of.
    SpinOff { event: String, parent: String, children: Vec<ChildShare> },
    /// The capital `distribution` returned a unit, as the issuer published it.
    ReturnOfCapital { distribution: String, per_unit: String },
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChildShare {
    pub instrument: String,
    pub cost_share: String,
}

/// An empty body: an entry of nothing, refused by name.
impl Default for EntryRequest {
    fn default() -> Self {
        EntryRequest::ReturnOfCapital { distribution: String::new(), per_unit: String::new() }
    }
}

/// Why an entry was not kept.
#[derive(Debug)]
pub enum Refused {
    /// The entry does not read, or the book cannot stand behind it: the page shows why.
    Entry(String),
    Failed(String),
}

fn bad(why: impl Into<String>) -> Refused {
    Refused::Entry(why.into())
}

fn dec(what: &str, v: &str) -> Result<Dec, Refused> {
    let v = v.trim().replace(',', "");
    Dec::parse(&v).map_err(|_| bad(format!("{what} {v:?} is not a number")))
}

fn day(what: &str, v: &str) -> Result<bagholder_core::jiff::civil::Date, Refused> {
    v.trim().parse().map_err(|_| bad(format!("{what} {v:?} is not a day")))
}

fn tx(what: &str, v: &str) -> Result<TransactionId, Refused> {
    TransactionId::parse(v.trim()).map_err(|_| bad(format!("{what} {v:?} is not a transaction")))
}

fn instrument_id(v: &str) -> Result<InstrumentId, Refused> {
    InstrumentId::parse(v.trim()).map_err(|_| bad(format!("{v:?} is not an instrument")))
}

/// A contract as the app writes one, `LUNR 15JAN27 12.00 CALL`: its underlying's
/// symbol and its terms. `None` for any other symbol.
pub fn contract_of(symbol: &str) -> Option<(String, bagholder_core::jiff::civil::Date, Dec, OptionRight)> {
    let parts: Vec<&str> = symbol.split_whitespace().collect();
    let [under, date, strike, right] = parts.as_slice() else { return None };
    const MONTHS: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];
    let date = date.to_uppercase();
    if date.len() != 7 || !date.is_ascii() {
        return None;
    }
    let (d, m, y) = (&date[..2], &date[2..5], &date[5..]);
    let month = MONTHS.iter().position(|x| *x == m)? as i8 + 1;
    let expiry = bagholder_core::jiff::civil::Date::new(2000 + y.parse::<i16>().ok()?, month, d.parse().ok()?).ok()?;
    let strike = Dec::parse(strike).ok().filter(|s| s.is_positive())?;
    let right = match right.to_uppercase().as_str() {
        "CALL" => OptionRight::Call,
        "PUT" => OptionRight::Put,
        _ => return None,
    };
    Some((under.to_uppercase(), expiry, strike, right))
}

/// The instrument the book holds by `symbol` in `currency`, if exactly one.
pub(crate) fn held_by_symbol(e: &Engine, symbol: &str, currency: Currency) -> Option<InstrumentId> {
    let mut found = e
        .inputs()
        .ledger
        .instruments
        .iter()
        .filter(|(_, i)| i.instrument.currency == currency && i.current_name().is_some_and(|n| n.symbol.eq_ignore_ascii_case(symbol)))
        .map(|(id, _)| *id);
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// The account a trade entered without one goes to: made the first time.
pub(crate) fn manual_account(f: &Figures, now: bagholder_core::jiff::Timestamp) -> Result<AccountId, Refused> {
    let book = f.book().map_err(Refused::Failed)?;
    let manual = Broker::named("manual");
    let r = AccountRef::new(manual.clone(), "manual");
    if let Some(a) = book.account_by_ref(&r).map_err(|e| Refused::Failed(e.to_string()))? {
        return Ok(a);
    }
    let fail = |e: bagholder_book::BookError| Refused::Failed(e.to_string());
    let connection = match book.connections().map_err(fail)?.into_iter().find(|c| c.broker == manual) {
        Some(c) => c.id,
        None => book.add_connection(&manual, "Manual", now).map_err(fail)?,
    };
    let kind = AccountType::Known { kind: AccountKind::Cash, registration: Registration::Unregistered, managed: false, joint: false };
    book.add_account(connection, &[r], &kind, AccountStatus::Open, Some("Manual"), now).map_err(fail)
}

/// An entry read, ready to keep; a trade for the Manual account waits on that
/// account being made, which is a write to the book the engine's lock is not held
/// over.
enum Prepared {
    Ready(Entry),
    Manual { traded: Traded, day: bagholder_core::jiff::civil::Date, side: Side, quantity: Dec, price: Money, fee: Option<Money> },
}

/// Read the entry, keep it, and apply it.
pub fn enter(f: &Figures, req: &EntryRequest, now: bagholder_core::jiff::Timestamp) -> Result<(), Refused> {
    let entry = f
        .read(|e| -> Result<Prepared, Refused> {
            let i = e.inputs();
            let transaction = |id: &TransactionId| i.ledger.transactions.iter().find(|t| &t.id == id).ok_or_else(|| bad(format!("no transaction {id} in the book")));
            let waits = |id: &TransactionId, what: bagholder_engine::ledger::Wanted| e.figures().matched.waiting.get(id).is_some_and(|w| w.what == what);
            let currency_of = |id: InstrumentId| i.ledger.instruments.get(&id).map(|x| x.instrument.currency).ok_or_else(|| bad(format!("no instrument {id} in the book")));
            Ok(Prepared::Ready(match req {
                EntryRequest::Trade { account, instrument, symbol, currency, day: d, side, quantity, price, fee } => {
                    let currency = Currency::parse(currency.trim()).map_err(|_| bad(format!("{currency:?} is not a currency")))?;
                    let symbol = symbol.trim().to_uppercase();
                    let traded = match instrument.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                        Some(id) => Traded::Held(instrument_id(id)?),
                        None if symbol.is_empty() => return Err(bad("a symbol is required")),
                        None => match held_by_symbol(e, &symbol, currency) {
                            Some(id) => Traded::Held(id),
                            None => {
                                let contract = contract_of(&symbol).map(|(under, expiry, strike, right)| Contract {
                                    underlying: match held_by_symbol(e, &under, currency).filter(|u| i.ledger.instruments[u].instrument.kind != InstrumentKind::OptionContract) {
                                        Some(u) => Underlying::Held(u),
                                        None => Underlying::Named(under),
                                    },
                                    expiry,
                                    strike,
                                    right,
                                });
                                Traded::Named { symbol: symbol.clone(), currency, contract }
                            }
                        },
                    };
                    if let Traded::Held(id) = &traded {
                        let priced = currency_of(*id)?;
                        if priced != currency {
                            return Err(bad(format!("{symbol} is priced in {priced}, not {currency}")));
                        }
                    }
                    let side = match side.trim().to_uppercase().as_str() {
                        "BUY" => Side::Buy,
                        "SELL" => Side::Sell,
                        other => return Err(bad(format!("a side {other:?}, neither BUY nor SELL"))),
                    };
                    let fee = match fee.trim() {
                        "" => None,
                        v => Some(Money::new(dec("the fee", v)?, currency)),
                    };
                    let account = match account.trim() {
                        "" => None,
                        a => Some(AccountId::parse(a).map_err(|_| bad(format!("{a:?} is not an account")))?),
                    };
                    if let Some(a) = account {
                        if !i.ledger.accounts.contains_key(&a) {
                            return Err(bad(format!("no account {a} in the book")));
                        }
                    }
                    let (day, quantity, price) = (day("the day", d)?, dec("the quantity", quantity)?, Money::new(dec("the price", price)?, currency));
                    return Ok(match account {
                        Some(account) => Prepared::Ready(Entry::Trade { account, instrument: traded, day, side, quantity, price, fee }),
                        None => Prepared::Manual { traded, day, side, quantity, price, fee },
                    });
                }
                EntryRequest::CostOfArrival { arrival, cost, acquired } => {
                    let t = transaction(&tx("the arrival", arrival)?)?;
                    if !waits(&t.id, bagholder_engine::ledger::Wanted::CostOfArrival) {
                        return Err(bad(format!("transaction {} is not units that arrived without a cost", t.id)));
                    }
                    let instrument = t.instrument.ok_or_else(|| bad(format!("transaction {} brought no units in", t.id)))?;
                    Entry::CostOfArrival { arrival: t.id.clone(), instrument, cost: Money::new(dec("the cost", cost)?, currency_of(instrument)?), acquired: day("the day acquired", acquired)? }
                }
                EntryRequest::SpinOff { event, parent, children } => {
                    let t = transaction(&tx("the event", event)?)?;
                    if !waits(&t.id, bagholder_engine::ledger::Wanted::Event) {
                        return Err(bad(format!("transaction {} is not an event waiting on what it did", t.id)));
                    }
                    let parent = instrument_id(parent)?;
                    currency_of(parent)?;
                    let children = children.iter().map(|c| Ok((instrument_id(&c.instrument)?, dec("a share of cost", &c.cost_share)?))).collect::<Result<Vec<_>, Refused>>()?;
                    if children.is_empty() {
                        return Err(bad("a spin-off with no new holding"));
                    }
                    Entry::SpinOff { event: t.id.clone(), parent, children }
                }
                EntryRequest::ReturnOfCapital { distribution, per_unit } => {
                    let t = transaction(&tx("the distribution", distribution)?)?;
                    if t.kind != bagholder_core::transaction::Kind::Dividend && !waits(&t.id, bagholder_engine::ledger::Wanted::Event) {
                        return Err(bad(format!("transaction {} is neither a distribution nor an event waiting on what it did", t.id)));
                    }
                    let instrument = t.instrument.ok_or_else(|| bad(format!("transaction {} names no holding", t.id)))?;
                    Entry::ReturnOfCapital { distribution: t.id.clone(), instrument, per_unit: Money::new(dec("the capital returned a unit", per_unit)?, currency_of(instrument)?) }
                }
            }))
        })
        .ok_or_else(|| Refused::Failed("the figures are not built yet".into()))?;
    let entry = match entry? {
        Prepared::Ready(e) => e,
        // a trade for the Manual account: its account is made first
        Prepared::Manual { traded, day, side, quantity, price, fee } => Entry::Trade { account: manual_account(f, now)?, instrument: traded, day, side, quantity, price, fee },
    };
    let book = f.book().map_err(Refused::Failed)?;
    book.enter(&entry, now).map_err(|e| match e {
        bagholder_book::BookError::Refused(why) => Refused::Entry(why),
        other => Refused::Failed(other.to_string()),
    })?;
    f.record_changed(now).map_err(Refused::Failed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn figures() -> (tempfile::TempDir, Figures) {
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let now: bagholder_core::jiff::Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        (home, f)
    }

    fn now() -> bagholder_core::jiff::Timestamp {
        "2025-11-19T21:30:00Z".parse().unwrap()
    }

    fn trade(account: &str, symbol: &str, currency: &str, side: &str, quantity: &str, price: &str, fee: &str) -> EntryRequest {
        EntryRequest::Trade { account: account.into(), instrument: None, symbol: symbol.into(), currency: currency.into(), day: "2025-11-19".into(), side: side.into(), quantity: quantity.into(), price: price.into(), fee: fee.into() }
    }

    /// A share the recorded month holds: its symbol, currency and account.
    fn held(f: &Figures) -> (String, String, String, InstrumentId) {
        f.read(|e| {
            let p = e.figures().positions.iter().find(|p| p.kind == InstrumentKind::Security).unwrap();
            let info = &e.inputs().ledger.instruments[&p.instrument];
            (info.current_name().unwrap().symbol.clone(), info.instrument.currency.as_str().to_string(), p.account.to_string(), p.instrument)
        })
        .unwrap()
    }

    fn t0_account(f: &Figures) -> AccountId {
        f.read(|e| e.figures().positions.iter().find(|p| p.kind == InstrumentKind::Security).unwrap().account).unwrap()
    }

    fn person_trades(f: &Figures) -> Vec<bagholder_core::transaction::Transaction> {
        let book = f.book().unwrap();
        let person: std::collections::BTreeSet<_> = book.live_records(&bagholder_core::SourceName::person()).unwrap().into_iter().collect();
        f.read(|e| e.inputs().ledger.transactions.iter().filter(|t| person.contains(&t.id.record)).cloned().collect()).unwrap()
    }

    #[test]
    fn a_trade_by_hand_is_of_the_instrument_the_book_holds_by_that_symbol_and_reaches_the_figures() {
        let (_h, f) = figures();
        let (symbol, currency, account, instrument) = held(&f);
        let before = f.version();
        enter(&f, &trade(&account, &symbol.to_lowercase(), &currency, "sell", "1", "2.50", "0"), now()).unwrap();
        assert!(f.version() > before, "the figures moved");
        let t = person_trades(&f);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].instrument, t[0].quantity, t[0].price.map(|p| p.amount)), (Some(instrument), Some(Dec::parse("-1").unwrap()), Some(Dec::parse("2.50").unwrap())));
        assert_eq!(t[0].fee.map(|f| f.amount), Some(Dec::ZERO), "a fee of 0 as entered");
    }

    #[test]
    fn a_trade_without_an_account_goes_to_the_manual_account_and_a_new_symbol_to_a_new_instrument() {
        let (_h, f) = figures();
        enter(&f, &trade("", "zzqq", "USD", "BUY", "10", "1.75", ""), now()).unwrap();
        enter(&f, &trade("", "ZZQQ 15JAN27 12.00 CALL", "USD", "BUY", "2", "0.40", ""), now()).unwrap();
        let book = f.book().unwrap();
        let manual = book.account_by_ref(&AccountRef::new(Broker::named("manual"), "manual")).unwrap().expect("the Manual account");
        let t = person_trades(&f);
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|t| t.account == manual));
        let kind = |t: &bagholder_core::transaction::Transaction| book.instrument(t.instrument.unwrap()).unwrap().kind;
        let share = t.iter().find(|t| kind(t) == InstrumentKind::Security).expect("the share").instrument.unwrap();
        let contract = t.iter().find(|t| kind(t) == InstrumentKind::OptionContract).expect("the contract").instrument.unwrap();
        let terms = book.option_terms(contract).unwrap().unwrap();
        assert_eq!((terms.underlying, terms.multiplier), (share, None), "on the share entered first, its size not assumed");
        assert!(t.iter().all(|t| t.fee.is_none()), "no fee entered is none");
        // no order can be placed in it: the ticket does not offer it
        let names = f.names().unwrap();
        let accounts = f.read(|e| crate::wire::build::accounts(e.inputs(), &names)).unwrap();
        let listed = |id: AccountId| accounts.iter().find(|a| a.id == id.to_string()).unwrap().tradable;
        assert!(!listed(manual), "the Manual account is not tradable");
        assert!(listed(t0_account(&f)), "a Wealthsimple cash or margin account is");
    }

    #[test]
    fn an_entry_that_does_not_read_or_that_the_book_refuses_is_said() {
        let (_h, f) = figures();
        let (symbol, currency, account, instrument) = held(&f);
        let other = if currency == "CAD" { "USD" } else { "CAD" };
        let said = |r: &EntryRequest| match enter(&f, r, now()) {
            Err(Refused::Entry(why)) => why,
            other => panic!("{other:?}"),
        };
        assert!(said(&trade(&account, &symbol, &currency, "buy", "ten", "1", "")).contains("\"ten\""));
        // a holding named by its id, in another currency than it is priced in
        let by_id = EntryRequest::Trade { account: account.clone(), instrument: Some(instrument.to_string()), symbol: symbol.clone(), currency: other.into(), day: "2025-11-19".into(), side: "buy".into(), quantity: "1".into(), price: "1".into(), fee: "".into() };
        assert!(said(&by_id).contains(&format!("priced in {currency}")));
        assert!(said(&trade(&account, &symbol, &currency, "hold", "1", "1", "")).contains("HOLD"));
        assert!(said(&trade(&account, "", &currency, "buy", "1", "1", "")).contains("symbol"));
        assert!(said(&trade(&account, &symbol, &currency, "buy", "0", "1", "")).contains("quantity"), "the book's refusal, named");
        assert!(said(&EntryRequest::default()).contains("distribution"));
        assert!(person_trades(&f).is_empty(), "nothing kept");
    }

    #[test]
    fn a_contract_is_read_from_the_symbol_the_app_writes_for_one() {
        let (u, e, s, r) = contract_of("lunr 15jan27 12.00 call").unwrap();
        assert_eq!((u.as_str(), e.to_string(), s, r), ("LUNR", "2027-01-15".to_string(), Dec::parse("12").unwrap(), OptionRight::Call));
        assert_eq!(contract_of("SPY 20JUN25 550 PUT").map(|c| c.3), Some(OptionRight::Put));
        for other in ["LUNR", "LUNR 15JAN27 12.00", "LUNR 32JAN27 12 CALL", "LUNR 15XYZ27 12 CALL", "LUNR 15JAN27 0 CALL", "LUNR 15JAN27 12 STRADDLE"] {
            assert!(contract_of(other).is_none(), "{other}");
        }
    }

    #[test]
    fn an_entry_against_a_transaction_that_does_not_wait_on_it_is_refused() {
        let (_h, f) = figures();
        let buy = f.read(|e| e.inputs().ledger.transactions.iter().find(|t| t.kind == bagholder_core::transaction::Kind::Buy).unwrap().id.to_string()).unwrap();
        let said = |r: &EntryRequest| match enter(&f, r, now()) {
            Err(Refused::Entry(why)) => why,
            other => panic!("{other:?}"),
        };
        assert!(said(&EntryRequest::CostOfArrival { arrival: buy.clone(), cost: "100".into(), acquired: "2020-01-02".into() }).contains("arrived without a cost"), "a purchase has its cost");
        assert!(said(&EntryRequest::SpinOff { event: buy.clone(), parent: String::new(), children: vec![] }).contains("waiting"));
        assert!(said(&EntryRequest::ReturnOfCapital { distribution: buy, per_unit: "0.1".into() }).contains("neither a distribution"));
        assert!(f.book().unwrap().adjustments().unwrap().is_empty(), "nothing kept");
    }
}
