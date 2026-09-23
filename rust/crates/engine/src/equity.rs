//! Bagholder's own equity series (`docs/architecture.md` §8,
//! `docs/plans/stage-2-engine.md`, "What each figure is").
//!
//! Per account and day: its cash in each currency plus each holding's units ×
//! its close × multiplier, in CAD at the day's rate. A holding's close on a day
//! is the latest within the week before it (so a weekend or a holiday takes the
//! last session's); a holding with none in that week has no close that day. A
//! day the own value cannot be stated is empty with its reason, and the
//! broker's stated net value stands for it where the broker states one, marked
//! as the broker's. Money and assets moved in or out are the flows returns are
//! net of.
//!
//! The broker check compares Bagholder's cash and holdings now with what the
//! broker states, exactly.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::ToSpan;
use bagholder_core::transaction::Kind;
use bagholder_core::{AccountId, Currency, Dec, InstrumentId, Money, TransactionId};

use crate::fx::{live_rate, rate};
use crate::gap::{Fig, Gap, Gaps};
use crate::input::Inputs;
use crate::ledger::{multiplier, Matched};
use crate::positions::mark_of;
use crate::stat::Ratio;

/// How far back a close may be and still be a holding's close on a day.
pub const CLOSE_REACH_DAYS: i64 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueSource {
    /// Bagholder's own, from the record, the closes and the rates.
    Own,
    /// The broker's stated net value, on a day the own one cannot be stated.
    Broker,
}

/// One account's value on one day, and the money moved in (positive) or out
/// that day, both in CAD.
#[derive(Clone, Debug, PartialEq)]
pub struct DayValue {
    pub day: Date,
    pub value: Dec,
    /// None where the flows of that day are not known for the source used.
    pub flow: Option<Dec>,
    pub source: ValueSource,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccountEquity {
    pub account: AccountId,
    /// Every day from the account's first to today: its own value, or what it
    /// waits on.
    pub own: BTreeMap<Date, Fig<Dec>>,
    /// Every day's money and assets moved in (positive) or out, or what they
    /// wait on: apart from the value, so a day whose value waits still carries
    /// its flows into the return across it.
    pub flows: BTreeMap<Date, Fig<Dec>>,
    /// The series shown: its own value where stated, else the broker's.
    pub points: Vec<DayValue>,
    /// Each day's return and the value it is over, formed only between two
    /// days of the same source.
    pub returns: Vec<(Date, Ratio, Dec)>,
}

/// Is `t` money or an asset moved into (positive) or out of the account?
fn is_flow(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Deposit | Kind::EmployerDeposit | Kind::GovernmentDeposit | Kind::Withdrawal | Kind::TransferIn | Kind::TransferOut | Kind::CardPurchase | Kind::CardRefund
    )
}

/// The close of an instrument on a day: the latest within `CLOSE_REACH_DAYS`
/// before it, and on today its live price.
fn close_on(inputs: &Inputs, instrument: InstrumentId, day: Date) -> Fig<Dec> {
    if day == inputs.clock.today {
        if let Some(info) = inputs.ledger.instruments.get(&instrument) {
            return mark_of(inputs, instrument, info.instrument.kind, info.instrument.currency).map(|m| m.price);
        }
    }
    let from = day.checked_sub(CLOSE_REACH_DAYS.days()).unwrap_or(day);
    inputs.market.closes.get(&instrument).and_then(|c| c.range(from..=day).next_back()).map(|(_, v)| *v).ok_or_else(|| Gaps::of(Gap::CloseUnknown { instrument, day }))
}

/// CAD per unit on a day: the day's rate, or the live rate today.
fn cad_rate(inputs: &Inputs, currency: Currency, day: Date) -> Fig<Dec> {
    if day == inputs.clock.today {
        return live_rate(&inputs.facts.rates, &inputs.clock, currency).map(|(r, _)| r);
    }
    rate(&inputs.facts.rates, &inputs.clock, currency, day)
}

fn money_cad(inputs: &Inputs, m: Money, day: Date) -> Fig<Dec> {
    Ok(m.amount.checked_mul(cad_rate(inputs, m.currency, day)?)?)
}

/// The first day each account has a problem on its record: from then on its own
/// value is not stated.
fn problem_days(inputs: &Inputs, matched: &Matched) -> BTreeMap<AccountId, (Date, Gaps)> {
    let by_id: BTreeMap<&TransactionId, (AccountId, Date)> = inputs.ledger.transactions.iter().map(|t| (&t.id, (t.account, t.trade_date))).collect();
    let mut out: BTreeMap<AccountId, (Date, Gaps)> = BTreeMap::new();
    let mut note = |account: AccountId, day: Date, gaps: Gaps| {
        let e = out.entry(account).or_insert((day, Gaps::none()));
        if day < e.0 {
            e.0 = day;
        }
        e.1.merge(&gaps);
    };
    for t in &inputs.ledger.transactions {
        if let Some(r) = inputs.ledger.records.get(&t.id.record) {
            for p in &r.problems {
                note(t.account, t.trade_date, Gaps::of(Gap::RecordProblem { transaction: t.id.clone(), code: p.code.clone() }));
            }
        }
    }
    for (id, gaps) in &matched.unapplied {
        if let Some((a, d)) = by_id.get(id) {
            note(*a, *d, gaps.clone());
        }
    }
    for b in &matched.beyond {
        if let Some((a, d)) = by_id.get(&b.transaction) {
            note(*a, *d, Gaps::of(Gap::BeyondHeld(b.transaction.clone())));
        }
    }
    out
}

/// Every account's series; `only` these accounts'.
pub fn build_equity(inputs: &Inputs, matched: &Matched, only: Option<&BTreeSet<AccountId>>) -> BTreeMap<AccountId, AccountEquity> {
    let today = inputs.clock.today;
    let problems = problem_days(inputs, matched);
    let mut by_account: BTreeMap<AccountId, Vec<&bagholder_core::transaction::Transaction>> = BTreeMap::new();
    for t in &inputs.ledger.transactions {
        by_account.entry(t.account).or_default().push(t);
    }
    let mut accounts: BTreeSet<AccountId> = by_account.keys().copied().collect();
    accounts.extend(inputs.market.brokers.iter().filter(|(_, b)| !b.net_value.is_empty()).map(|(a, _)| *a));
    if let Some(only) = only {
        accounts.retain(|a| only.contains(a));
    }
    let mut out = BTreeMap::new();
    for account in accounts {
        let txs = by_account.remove(&account).unwrap_or_default();
        let broker = inputs.market.brokers.get(&account);
        let first = txs.iter().map(|t| t.trade_date).chain(broker.and_then(|b| b.net_value.keys().next().copied())).min();
        let Some(first) = first else { continue };
        let instruments: BTreeSet<InstrumentId> = matched.units.keys().filter(|(a, _)| *a == account).map(|(_, i)| *i).collect();
        // cash per currency, and the flows, by day
        let mut cash_moves: BTreeMap<Date, Vec<Money>> = BTreeMap::new();
        let mut flows: BTreeMap<Date, Vec<&bagholder_core::transaction::Transaction>> = BTreeMap::new();
        for t in &txs {
            if let Some(c) = t.cash {
                cash_moves.entry(t.trade_date).or_default().push(c);
            }
            if is_flow(t.kind) {
                flows.entry(t.trade_date).or_default().push(t);
            }
        }
        let problem = problems.get(&account);
        let mut cash: BTreeMap<Currency, Dec> = BTreeMap::new();
        let mut own = BTreeMap::new();
        let mut day_flows = BTreeMap::new();
        let mut day = first;
        while day <= today {
            for m in cash_moves.get(&day).into_iter().flatten() {
                let e = cash.entry(m.currency).or_insert(Dec::ZERO);
                *e = e.checked_add(m.amount).unwrap_or(*e);
            }
            // a record with a problem leaves the value and the flows unstated from its day
            let problem_now = problem.filter(|(d, _)| *d <= day).map(|(_, g)| g.clone());
            let value = (|| -> Fig<Dec> {
                if let Some(g) = &problem_now {
                    return Err(g.clone());
                }
                let mut gaps = Gaps::none();
                let mut total = Dec::ZERO;
                for (c, amount) in &cash {
                    if amount.is_zero() {
                        continue;
                    }
                    match money_cad(inputs, Money::new(*amount, *c), day) {
                        Ok(v) => total = total.checked_add(v)?,
                        Err(g) => gaps.merge(&g),
                    }
                }
                for i in &instruments {
                    let units = matched.units_on(account, *i, day);
                    if units.is_zero() {
                        continue;
                    }
                    let info = inputs.ledger.instruments.get(i);
                    let currency = info.map(|x| x.instrument.currency).unwrap_or(Currency::CAD);
                    let v = (|| -> Fig<Dec> {
                        let m = multiplier(info, *i)?;
                        let close = close_on(inputs, *i, day)?;
                        money_cad(inputs, Money::new(units.checked_mul(close)?.checked_mul(m)?, currency), day)
                    })();
                    match v {
                        Ok(v) => total = total.checked_add(v)?,
                        Err(g) => gaps.merge(&g),
                    }
                }
                gaps.or(total)
            })();
            let flow = (|| -> Fig<Dec> {
                if let Some(g) = &problem_now {
                    return Err(g.clone());
                }
                let mut gaps = Gaps::none();
                let mut flow = Dec::ZERO;
                for t in flows.get(&day).into_iter().flatten() {
                    let f = (|| -> Fig<Dec> {
                        let mut f = match t.cash {
                            Some(c) => money_cad(inputs, c, day)?,
                            None => Dec::ZERO,
                        };
                        // an asset moved in or out, at its value that day
                        if let (Some(i), Some(q)) = (t.instrument, t.quantity) {
                            let info = inputs.ledger.instruments.get(&i);
                            let currency = info.map(|x| x.instrument.currency).unwrap_or(Currency::CAD);
                            let m = multiplier(info, i)?;
                            let close = close_on(inputs, i, day)?;
                            f = f.checked_add(money_cad(inputs, Money::new(q.checked_mul(close)?.checked_mul(m)?, currency), day)?)?;
                        }
                        Ok(f)
                    })();
                    match f {
                        Ok(v) => flow = flow.checked_add(v)?,
                        Err(g) => gaps.merge(&g),
                    }
                }
                gaps.or(flow)
            })();
            own.insert(day, value);
            day_flows.insert(day, flow);
            day = match day.tomorrow() {
                Ok(d) => d,
                Err(_) => break,
            };
        }
        // the series: its own value where stated, else the broker's
        let mut points = Vec::new();
        let mut last_deposits: Option<Dec> = None;
        for (d, v) in &own {
            let broker_value = broker.and_then(|b| b.net_value.get(d)).copied();
            let deposits = broker.and_then(|b| b.net_deposits.get(d)).copied();
            let broker_flow = match (deposits, last_deposits) {
                (Some(now), Some(before)) => now.checked_sub(before).ok(),
                _ => None,
            };
            if deposits.is_some() {
                last_deposits = deposits;
            }
            match (v, broker_value) {
                (Ok(value), _) => points.push(DayValue { day: *d, value: *value, flow: day_flows.get(d).and_then(|f| f.as_ref().ok()).copied(), source: ValueSource::Own }),
                (Err(_), Some(b)) => points.push(DayValue { day: *d, value: b, flow: broker_flow, source: ValueSource::Broker }),
                (Err(_), None) => {}
            }
        }
        let returns = daily_returns(&own, &day_flows, broker, &points);
        out.insert(account, AccountEquity { account, own, flows: day_flows, points, returns });
    }
    out
}

/// Each day's return, formed only between two stated days of the same source:
/// its own value on both, else the broker's on both; across a gap, from the last
/// stated day to the next, net of the flows between.
fn daily_returns(own: &BTreeMap<Date, Fig<Dec>>, flows: &BTreeMap<Date, Fig<Dec>>, broker: Option<&crate::input::BrokerAccount>, points: &[DayValue]) -> Vec<(Date, Ratio, Dec)> {
    let mut out = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let own_pair = match (own.get(&a.day), own.get(&b.day)) {
            (Some(Ok(va)), Some(Ok(vb))) => {
                // the flows of every day after `a` up to and including `b`,
                // days whose value waits included
                let flow = flows.range(a.day..=b.day).skip(1).try_fold(Dec::ZERO, |acc, (_, f)| match f {
                    Ok(f) => acc.checked_add(*f).ok(),
                    Err(_) => None,
                });
                flow.map(|f| (*va, *vb, f))
            }
            _ => None,
        };
        let pair = own_pair.or_else(|| {
            let b0 = broker?.net_value.get(&a.day)?;
            let b1 = broker?.net_value.get(&b.day)?;
            let d0 = broker?.net_deposits.get(&a.day)?;
            let d1 = broker?.net_deposits.get(&b.day)?;
            Some((*b0, *b1, d1.checked_sub(*d0).ok()?))
        });
        if let Some((v0, v1, flow)) = pair {
            if let Some(r) = crate::stat::day_return(v0, v1, flow) {
                out.push((b.day, r, v0));
            }
        }
    }
    out
}

/// One difference between what Bagholder holds and what the broker states.
#[derive(Clone, Debug, PartialEq)]
pub enum Difference {
    Cash { currency: Currency, own: Dec, broker: Dec },
    Units { instrument: InstrumentId, own: Dec, broker: Dec },
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
    /// Bagholder's own value now less the broker's stated net value now, in
    /// CAD; none when the broker states no net value.
    pub value_difference: Option<Fig<Dec>>,
}

pub fn broker_checks(inputs: &Inputs, matched: &Matched, equity: &BTreeMap<AccountId, AccountEquity>) -> Vec<BrokerCheck> {
    let today = inputs.clock.today;
    let mut out = Vec::new();
    for (account, b) in &inputs.market.brokers {
        let mut own_cash: BTreeMap<Currency, Dec> = BTreeMap::new();
        for t in inputs.ledger.transactions.iter().filter(|t| t.account == *account) {
            if let Some(c) = t.cash {
                let e = own_cash.entry(c.currency).or_insert(Dec::ZERO);
                *e = e.checked_add(c.amount).unwrap_or(*e);
            }
        }
        let mut differences = Vec::new();
        let currencies: BTreeSet<Currency> = own_cash.keys().chain(b.cash.keys()).copied().collect();
        for c in currencies {
            let (o, br) = (own_cash.get(&c).copied().unwrap_or(Dec::ZERO), b.cash.get(&c).copied().unwrap_or(Dec::ZERO));
            if o != br {
                differences.push(Difference::Cash { currency: c, own: o, broker: br });
            }
        }
        let instruments: BTreeSet<InstrumentId> = matched.units.keys().filter(|(a, _)| a == account).map(|(_, i)| *i).chain(b.held.keys().copied()).collect();
        for i in instruments {
            let (o, br) = (matched.units_on(*account, i, today), b.held.get(&i).copied().unwrap_or(Dec::ZERO));
            if o != br {
                differences.push(Difference::Units { instrument: i, own: o, broker: br });
            }
        }
        let pending = match (b.as_of, b.activity_read_at) {
            (Some(stated), Some(read)) => stated > read,
            (Some(_), None) => true,
            (None, _) => false,
        };
        let own_now = equity.get(account).and_then(|e| e.own.get(&today)).cloned();
        let value_difference = b.net_value_now.map(|n| match own_now {
            Some(Ok(v)) => v.checked_sub(n).map_err(Gaps::from),
            Some(Err(g)) => Err(g),
            // an account with nothing on the record holds nothing
            None => Ok(n.neg()),
        });
        out.push(BrokerCheck { account: *account, differences, pending, value_difference });
    }
    out
}
