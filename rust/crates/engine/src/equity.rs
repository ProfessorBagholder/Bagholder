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
#[derive(Clone, Debug, PartialEq)]
pub enum Difference {
    Cash { currency: Currency, own: Fig<Dec>, broker: Dec },
    Units { instrument: InstrumentId, own: Fig<Dec>, broker: Dec },
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
        let mut differences = Vec::new();
        let currencies: BTreeSet<Currency> = own_cash.keys().chain(b.cash.keys()).copied().collect();
        for c in currencies {
            let (o, br) = (own_cash.get(&c).cloned().unwrap_or(Ok(Dec::ZERO)), b.cash.get(&c).copied().unwrap_or(Dec::ZERO));
            if o != Ok(br) {
                differences.push(Difference::Cash { currency: c, own: o, broker: br });
            }
        }
        let instruments: BTreeSet<InstrumentId> = matched.units.keys().filter(|(a, _)| a == account).map(|(_, i)| *i).chain(b.held.keys().copied()).collect();
        for i in instruments {
            let (o, br) = (matched.units_on(*account, i, today), b.held.get(&i).copied().unwrap_or(Dec::ZERO));
            if o != Ok(br) {
                differences.push(Difference::Units { instrument: i, own: o, broker: br });
            }
        }
        let pending = match (b.as_of, b.activity_read_at) {
            (Some(stated), Some(read)) => stated > read,
            (Some(_), None) => true,
            (None, _) => false,
        };
        out.push(BrokerCheck { account: *account, differences, pending });
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
