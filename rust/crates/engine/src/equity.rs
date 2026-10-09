//! Each account's equity series (`SPEC.md` §2, Equity and returns; owner,
//! 2026-09-24): the account's value and net deposits per day as its broker
//! states them, in CAD. A day's flow is the change in the stated net deposits,
//! the money moved in (positive) or out, which returns are net of. Kept per
//! account, so an account with no broker statement can be given a value of
//! another kind beside these later.
//!
//! The broker check compares Bagholder's cash and holdings now with what the
//! broker states, exactly.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::jiff::civil::Date;
use bagholder_core::{AccountId, Currency, Dec, InstrumentId};

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

/// One account's series from its broker's statement.
pub fn account_equity(account: AccountId, broker: &BrokerAccount) -> AccountEquity {
    let mut points = Vec::with_capacity(broker.net_value.len());
    let mut returns = Vec::new();
    let mut gaps = Gaps::none();
    let mut before: Option<(Date, Dec)> = None;
    for (day, value) in &broker.net_value {
        let flow = before.and_then(|(b, _)| {
            let now = broker.net_deposits.get(day)?;
            let then = broker.net_deposits.get(&b)?;
            now.add_to_fit(then.neg()).map_err(|e| gaps.merge(&Gaps::from(e))).ok()
        });
        if let (Some((_, v0)), Some(f)) = (before, flow) {
            if let Some(r) = crate::stat::day_return(v0, *value, f) {
                returns.push((*day, r, v0));
            }
        }
        points.push(DayValue { day: *day, value: *value, flow });
        before = Some((*day, *value));
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
        .map(|(a, b)| (*a, account_equity(*a, b)))
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

pub fn broker_checks(inputs: &Inputs, matched: &Matched) -> Vec<BrokerCheck> {
    let today = inputs.clock.today;
    let mut out = Vec::new();
    for (account, b) in &inputs.market.brokers {
        let mut own_cash: BTreeMap<Currency, Fig<Dec>> = BTreeMap::new();
        for t in inputs.ledger.transactions.iter().filter(|t| t.account == *account) {
            if let Some(c) = t.cash {
                let e = own_cash.entry(c.currency).or_insert(Ok(Dec::ZERO));
                if let Ok(total) = e {
                    *e = total.checked_add(c.amount).map_err(Gaps::from);
                }
            }
        }
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
            let instruments: BTreeSet<InstrumentId> = matched.units.keys().filter(|(a, _)| a == account).map(|(_, i)| *i).chain(b.held.keys().copied()).collect();
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
        let mut b = BrokerAccount::default();
        for (n, v) in [(1, "100"), (2, "110"), (3, "120")] {
            b.net_value.insert(day(n), d(v));
        }
        b.net_deposits.insert(day(1), d("-70000000000000000000000000000"));
        b.net_deposits.insert(day(2), d("70000000000000000000000000000"));
        b.net_deposits.insert(day(3), d("70000000000000000000000000000"));
        let e = account_equity(AccountId::parse("00000000-0000-4000-8000-000000000001").unwrap(), &b);
        assert!(e.gaps.has_word("arithmetic"), "{:?}", e.gaps);
        assert_eq!(e.points[1].flow, None);
        // the next day's flow is known and forms its return
        assert_eq!(e.points[2].flow, Some(Dec::ZERO));
        assert_eq!(e.returns.len(), 1);
    }
}
