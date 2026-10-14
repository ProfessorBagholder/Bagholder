//! Each account's equity series (`SPEC.md` §2, Equity and returns; owner,
//! 2026-09-24): the account's value and net deposits per day as its broker
//! states them, each day in CAD at that day's rate (an account held in another
//! currency states them in it). A day's flow is the change in the stated net
//! deposits, the money moved in (positive) or out, which returns are net of. Kept per
//! account, so an account with no broker statement can be given a value of
//! another kind beside these later.
//!
//! The broker check compares Bagholder's cash and holdings now with what the
//! broker states, exactly.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::jiff::civil::Date;
use bagholder_core::{AccountId, Currency, Dec, InstrumentId, Money};

use crate::gap::{Fig, Gaps};
use crate::input::{BrokerAccount, Inputs};
use crate::ledger::Matched;
use crate::stat::Ratio;

/// One account's value on one day, and the money moved in (positive) or out
/// since its stated day before, both in CAD.
#[derive(Clone, Debug, PartialEq)]
pub struct DayValue {
    pub day: Date,
    pub value: Dec,
    /// None on the first stated day, and where the broker states no net
    /// deposits for this day or the one before.
    pub flow: Option<Dec>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccountEquity {
    pub account: AccountId,
    /// Every day the broker states a value, oldest first.
    pub points: Vec<DayValue>,
    /// Each day's return and the value it is over, formed between two
    /// consecutive stated days whose flow is known.
    pub returns: Vec<(Date, Ratio, Dec)>,
    /// What the series waits on: a day whose flow could not be worked out
    /// from the broker's stated deposits forms no return, and says so here.
    pub gaps: Gaps,
}

/// One account's series from its broker's statement, each day in CAD at that
/// day's rate: the value as stated, and the flow as the change in net deposits
/// in the currency stated, converted on the day it moved. A day whose value or
/// flow cannot be put in CAD (its rate not published yet) forms no point, and
/// says so in the gaps.
pub fn account_equity(account: AccountId, broker: &BrokerAccount, rates: &crate::input::Rates, clock: &crate::input::Clock) -> AccountEquity {
    let mut points = Vec::with_capacity(broker.net_value.len());
    let mut returns = Vec::new();
    let mut gaps = Gaps::none();
    let mut before: Option<(Date, Dec)> = None;
    let cad = |m: Money, day: Date, gaps: &mut Gaps| -> Option<Dec> { crate::fx::to_cad(rates, clock, m, day).map(|m| m.amount).map_err(|g| gaps.merge(&g)).ok() };
    for (day, value) in &broker.net_value {
        let Some(value) = cad(*value, *day, &mut gaps) else { continue };
        let flow = before.and_then(|(b, _)| {
            let now = *broker.net_deposits.get(day)?;
            let then = *broker.net_deposits.get(&b)?;
            if now.currency == then.currency {
                let moved = now.amount.add_to_fit(then.amount.neg()).map_err(|e| gaps.merge(&Gaps::from(e))).ok()?;
                cad(Money::new(moved, now.currency), *day, &mut gaps)
            } else {
                let (n, t) = (cad(now, *day, &mut gaps)?, cad(then, b, &mut gaps)?);
                n.add_to_fit(t.neg()).map_err(|e| gaps.merge(&Gaps::from(e))).ok()
            }
        });
        if let (Some((_, v0)), Some(f)) = (before, flow) {
            if let Some(r) = crate::stat::day_return(v0, value, f) {
                returns.push((*day, r, v0));
            }
        }
        points.push(DayValue { day: *day, value, flow });
        before = Some((*day, value));
    }
    AccountEquity { account, points, returns, gaps }
}

/// Every account's series; `only` these accounts'. An account whose broker
/// states no value has none.
pub fn build_equity(inputs: &Inputs, only: Option<&BTreeSet<AccountId>>) -> BTreeMap<AccountId, AccountEquity> {
    inputs
        .market
        .brokers
        .iter()
        .filter(|(a, b)| !b.net_value.is_empty() && only.is_none_or(|o| o.contains(a)))
        .map(|(a, b)| (*a, account_equity(*a, b, &inputs.facts.rates, &inputs.clock)))
        .collect()
}

/// One difference between what Bagholder holds and what the broker states.
/// Bagholder's side is the failure when its record cannot be summed exactly.
/// `pending` when the statement is newer than the last full read of the
/// activity: the difference may be a fill the record does not hold yet.
#[derive(Clone, Debug, PartialEq)]
pub enum Difference {
    Cash { currency: Currency, own: Fig<Dec>, broker: Dec, pending: bool },
    Units { instrument: InstrumentId, own: Fig<Dec>, broker: Dec, pending: bool },
}

impl Difference {
    pub fn pending(&self) -> bool {
        match self {
            Difference::Cash { pending, .. } | Difference::Units { pending, .. } => *pending,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrokerCheck {
    pub account: AccountId,
    pub differences: Vec<Difference>,
    /// The broker's statement is newer than the last full read of the
    /// account's activity (or no read is known): a difference may be a fill
    /// the record does not hold yet, so it is pending until a read of the
    /// activity made after the statement confirms or clears it.
    pub pending: bool,
}

/// Each account's check against what its broker states, or `only` that one's:
/// its work is its own account's cash totals, holds and holdings, read by key, so
/// one account's check does not grow with the rest of the book.
pub fn broker_checks(inputs: &Inputs, matched: &Matched, only: Option<AccountId>) -> Vec<BrokerCheck> {
    let today = inputs.clock.today;
    let mut out = Vec::new();
    let brokers: Vec<(&AccountId, &crate::input::BrokerAccount)> = match only {
        Some(a) => inputs.market.brokers.get_key_value(&a).into_iter().collect(),
        None => inputs.market.brokers.iter().collect(),
    };
    for (account, b) in brokers {
        let no_cash = BTreeMap::new();
        let own_cash: &BTreeMap<Currency, Fig<Dec>> = matched.cash.get(account).unwrap_or(&no_cash);
        let pending = match (b.as_of, b.activity_read_at) {
            (Some(stated), Some(read)) => stated > read,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let mut differences = Vec::new();
        // the cash stated at the last full read, which every fill it reflects is
        // on the record for; else the newest, pending while it is newer than the read
        let (stated_cash, cash_pending) = match &b.cash_read {
            Some(c) => (c, false),
            None => (&b.cash, pending),
        };
        // the stated cash is net of what the broker held against the account when
        // it stated it: the book's cash less those holds is compared with it, and
        // while one in a currency is of unknown size that currency's check waits
        let holds = if b.cash_read.is_some() { held(inputs, matched, *account, &b.cash_read_holds, today) } else { Held::default() };
        let currencies: BTreeSet<Currency> = own_cash.keys().chain(stated_cash.keys()).chain(holds.cash.keys()).copied().collect();
        for c in currencies {
            let (o, br) = (own_cash.get(&c).cloned().unwrap_or(Ok(Dec::ZERO)), stated_cash.get(&c).copied().unwrap_or(Dec::ZERO));
            let unknown = holds.every_currency_unknown || holds.unknown.contains(&c);
            // a difference is told only once confirmed: a statement of the broker's
            // made after the read states the same cash again, so nothing the broker
            // moved since (and the feed may not list yet) can be behind it, and a
            // difference never shows for a moment and goes (`SPEC.md` §4, the header)
            let confirmed = b.cash_read.is_some()
                && matches!((b.as_of, b.activity_read_at), (Some(stated), Some(read)) if stated > read)
                && b.cash.get(&c).copied().unwrap_or(Dec::ZERO) == br;
            let o = match holds.cash.get(&c) {
                Some(h) if !unknown => o.and_then(|o| o.checked_sub(*h).map_err(Gaps::from)),
                _ => o,
            };
            if o != Ok(br) {
                differences.push(Difference::Cash { currency: c, own: o, broker: br, pending: cash_pending || unknown || !confirmed });
            }
        }
        // units only where the broker stated them: an account whose holdings it
        // never stated (no units and no day they are stated as of) is not stated to
        // hold nothing. Units stated as of a day whose activity is read in full are
        // never pending; units stated as of today are, while the statement is newer.
        if b.held_as_of.is_some() || !b.held.is_empty() {
            let units_pending = b.held_as_of.is_none() && pending;
            let as_of = b.held_as_of.unwrap_or(today);
            let instruments: BTreeSet<InstrumentId> = matched.units.range((*account, InstrumentId::MIN)..).take_while(|((a, _), _)| a == account).map(|((_, i), _)| *i).chain(b.held.keys().copied()).collect();
            for i in instruments {
                let (o, br) = (matched.units_on(*account, i, as_of), b.held.get(&i).copied().unwrap_or(Dec::ZERO));
                // a coin's difference worth less than the broker's smallest order
                // is dust, never a disagreement: the broker's units valued as the
                // broker states them; the book's beyond them at the coin's price
                // now (else the last a fill of it stated)
                if let Ok(own) = &o {
                    let dust = match own.checked_sub(br) {
                        Ok(diff) if !diff.is_positive() => b.held_value.get(&i).is_some_and(|v| {
                            // what the broker holds and the book does not: its share of the stated worth
                            let worth = if own.is_zero() { Some(*v) } else { diff.abs().div_rounded(br.abs(), 28, bagholder_core::Rounding::HalfEven).ok().and_then(|s| v.amount.mul_to_fit(s).ok()).map(|a| bagholder_core::Money::new(a, v.currency)) };
                            worth.is_some_and(|w| crate::dust::is_dust_worth(inputs, *account, i, w, today))
                        }),
                        Ok(diff) => {
                            let price = crate::positions::current_price(inputs, i).or_else(|| matched.last_price.get(&i).copied());
                            price.is_some_and(|p| crate::dust::is_dust(inputs, *account, i, diff, p, today))
                        }
                        Err(_) => false,
                    };
                    if dust {
                        continue;
                    }
                }
                if o != Ok(br) {
                    differences.push(Difference::Units { instrument: i, own: o, broker: br, pending: units_pending });
                }
            }
        }
        out.push(BrokerCheck { account: *account, differences, pending });
    }
    out
}

/// What the broker held against an account's cash at a statement, per currency.
#[derive(Debug, Default)]
struct Held {
    cash: BTreeMap<Currency, Dec>,
    /// Currencies a hold of unknown size is in.
    unknown: BTreeSet<Currency>,
    /// A hold of unknown size whose currency is not stated either.
    every_currency_unknown: bool,
}

/// Each hold's cash (`bagholder_core::hold`): a buy's stated amount; a put sale's
/// collateral less its premium for the contracts it opens beyond the long ones
/// the account holds, in an account without margin only (with margin it is
/// secured by buying power, not cash); anything else of unknown size.
fn held(inputs: &Inputs, matched: &Matched, account: AccountId, holds: &[bagholder_core::hold::Hold], today: Date) -> Held {
    use bagholder_core::hold::HoldKind;
    let margin = inputs.ledger.accounts.get(&account).is_some_and(crate::scope::is_margin);
    let mut out = Held::default();
    for h in holds {
        let amount: Option<Dec> = match h.kind {
            HoldKind::Buy => h.amount,
            HoldKind::Withdrawal => None,
            HoldKind::PutSale if margin => continue,
            HoldKind::PutSale => (|| {
                let (collateral, contracts, premium) = (h.amount?, h.quantity?, h.premium?);
                let long = match h.instrument {
                    Some(i) => matched.units_on(account, i, today).ok()?.max(Dec::ZERO),
                    None => Dec::ZERO,
                };
                let opening = contracts.checked_sub(long).ok()?.max(Dec::ZERO);
                if opening.is_zero() || !contracts.is_positive() {
                    return Some(Dec::ZERO);
                }
                collateral.checked_sub(premium).ok()?.mul_div_rounded(opening, contracts, 10, bagholder_core::Rounding::HalfEven).ok()
            })(),
        };
        match (h.currency, amount) {
            (Some(c), Some(a)) => {
                let e = out.cash.entry(c).or_insert(Dec::ZERO);
                match e.checked_add(a) {
                    Ok(t) => *e = t,
                    Err(_) => {
                        out.unknown.insert(c);
                    }
                }
            }
            (Some(c), None) => {
                out.unknown.insert(c);
            }
            (None, _) => out.every_currency_unknown = true,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }

    #[test]
    fn a_flow_that_cannot_be_worked_out_forms_no_return_and_says_so() {
        let day = |n: i8| Date::new(2026, 9, n).unwrap();
        let cad = |s: &str| Money::new(d(s), Currency::CAD);
        let mut b = BrokerAccount::default();
        for (n, v) in [(1, "100"), (2, "110"), (3, "120")] {
            b.net_value.insert(day(n), cad(v));
        }
        b.net_deposits.insert(day(1), cad("-70000000000000000000000000000"));
        b.net_deposits.insert(day(2), cad("70000000000000000000000000000"));
        b.net_deposits.insert(day(3), cad("70000000000000000000000000000"));
        let e = account_equity(AccountId::parse("00000000-0000-4000-8000-000000000001").unwrap(), &b, &crate::input::Rates::default(), &clock());
        assert!(e.gaps.has_word("arithmetic"), "{:?}", e.gaps);
        assert_eq!(e.points[1].flow, None);
        // the next day's flow is known and forms its return
        assert_eq!(e.points[2].flow, Some(Dec::ZERO));
        assert_eq!(e.returns.len(), 1);
    }

    fn clock() -> crate::input::Clock {
        let zone = bagholder_core::jiff::tz::TimeZone::get("America/Toronto").unwrap();
        crate::input::Clock { today: Date::new(2026, 9, 30).unwrap(), now: "2026-09-30T20:00:00Z".parse().unwrap(), home: zone.clone(), bank: zone }
    }

    /// An account held in US dollars states its value and deposits in them: each
    /// day is put in CAD at that day's rate, the flow at the rate of the day it
    /// moved, so the return is the account's in CAD, the rate's move in it.
    #[test]
    fn an_account_stated_in_another_currency_is_in_cad_at_each_days_rate() {
        let day = |n: i8| Date::new(2026, 9, n).unwrap();
        let usd = |s: &str| Money::new(d(s), Currency::USD);
        let mut rates = crate::input::Rates::default();
        rates.by_currency.entry(Currency::USD).or_default().insert(day(1), d("1.30"));
        rates.by_currency.entry(Currency::USD).or_default().insert(day(2), d("1.40"));
        rates.covered.entry(Currency::USD).or_default().push(crate::input::Read { first: day(1), last: day(2), at: "2026-09-02T22:00:00Z".parse().unwrap() });
        let mut b = BrokerAccount::default();
        b.net_value.insert(day(1), usd("100"));
        b.net_value.insert(day(2), usd("110"));
        b.net_deposits.insert(day(1), usd("100"));
        b.net_deposits.insert(day(2), usd("110"));
        // a day whose rate is not published forms no point and says so
        b.net_value.insert(day(3), usd("120"));
        let e = account_equity(AccountId::parse("00000000-0000-4000-8000-000000000001").unwrap(), &b, &rates, &clock());
        assert_eq!(e.points.iter().map(|p| (p.value, p.flow)).collect::<Vec<_>>(), vec![(d("130.00"), None), (d("154.00"), Some(d("14.00")))]);
        // 130 CAD, 14 CAD put in, 154 CAD: a tenth, the rate's rise in it
        assert_eq!(e.returns.len(), 1);
        assert!(!e.gaps.is_empty(), "the day without its rate is said");
    }
}
