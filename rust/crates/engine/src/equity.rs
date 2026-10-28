//! Each account's equity series (`SPEC.md` §2, Equity and returns; owner,
//! 2026-09-24): the account's value and net deposits per day as its broker
//! states them, each day in CAD at that day's rate (an account held in another
//! currency states them in it). A day's flow is the change in the stated net
//! deposits, the money moved in (positive) or out, which returns are net of. Kept per
//! account, so an account with no broker statement can be given a value of
//! another kind beside these later.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::jiff::civil::Date;
use bagholder_core::{AccountId, Dec, Money};

use crate::gap::Gaps;
use crate::input::{BrokerAccount, Inputs};
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
/// says so in the gaps; a day whose deposits are stated in another currency
/// than the day before forms no flow, and says so.
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
            if now.currency != then.currency {
                // a total restated in another currency is not a deposit: converting
                // the two at their own days' rates would count the rate's move on
                // the whole deposit base as money moved, and at one rate the
                // restatement itself; the day forms no flow, as a day with no stated
                // deposits forms none, and says why
                gaps.merge(&Gaps::from(crate::gap::Gap::DepositsRestated { account, day: *day }));
                return None;
            }
            let moved = now.amount.add_to_fit(then.amount.neg()).map_err(|e| gaps.merge(&Gaps::from(e))).ok()?;
            cad(Money::new(moved, now.currency), *day, &mut gaps)
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

#[cfg(test)]
mod tests {
    use super::*;
    use bagholder_core::Currency;

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
        // a day whose rate is due and not read takes the Bank's last published
        // rate, with no sign (the owner, 2026-10-28: "Show nothing")
        b.net_value.insert(day(3), usd("120"));
        let e = account_equity(AccountId::parse("00000000-0000-4000-8000-000000000001").unwrap(), &b, &rates, &clock());
        assert_eq!(e.points.iter().map(|p| (p.value, p.flow)).collect::<Vec<_>>(), vec![(d("130.00"), None), (d("154.00"), Some(d("14.00"))), (d("168.00"), None)]);
        // 130 CAD, 14 CAD put in, 154 CAD: a tenth, the rate's rise in it
        assert_eq!(e.returns.len(), 1, "a day stating no deposits forms no return");
        assert!(e.gaps.is_empty(), "nothing waits on a rate the Bank published before");
    }
}
