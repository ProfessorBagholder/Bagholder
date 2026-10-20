//! Wealthsimple's statements beside its rows, read strictly (`crate::replay`
//! and the network client read them the same way): its accounts, an account's
//! positions as of a day, and its value and net deposits per day.

use crate::account_types::account_type;
use bagholder_broker::{AccountStated, DayValue, Units};
use bagholder_core::instrument::{RefScheme, Reference};
use bagholder_core::json::Value;
use std::collections::BTreeMap;

use bagholder_core::{Currency, Dec, Money};
use bagholder_sources::reply::{Node, Read};

use crate::mapping::broker;

/// An amount as Wealthsimple states it in whole cents (`cents`, an integer):
/// its `amount` beside it can carry more digits than a decimal holds (a crypto
/// account's value to 37 digits), which are not a statement of anything finer
/// than a cent.
fn cents(n: &Node) -> Read<Money> {
    let c = n.text("currency")?;
    let currency = Currency::parse(&c.to_uppercase()).map_err(|e| n.field("currency").map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))?;
    let cents = n.int("cents")?;
    let cent = Dec::parse("0.01").map_err(|e| n.mismatch(e.to_string()))?;
    let amount = Dec::from_int(cents).checked_mul(cent).map_err(|e| n.mismatch(e.to_string()))?;
    Ok(Money::new(amount, currency))
}

/// The margin account's custodian id that an account's Margin Boost names, where
/// the feature is on: the account backs that margin account as collateral.
fn margin_boost(n: &Node) -> Read<Option<String>> {
    for f in n.list("accountFeatures")? {
        if f.text("name")? != "MARGIN_BOOST" {
            continue;
        }
        let functional = match f.field("functional")?.value() {
            Value::Null => true,
            _ => f.bool("functional")?,
        };
        if !f.bool("enabled")? || !functional {
            return Ok(None);
        }
        let meta = f.field("metadata")?;
        if matches!(meta.value(), Value::Null) {
            return Err(meta.mismatch("a Margin Boost on with no metadata naming the margin account"));
        }
        return Ok(Some(meta.text("targetMarginAccountId")?.to_string()));
    }
    Ok(None)
}

/// The accounts, from `FetchAllAccounts`' nodes.
pub fn accounts(nodes: &[Value]) -> Read<Vec<AccountStated>> {
    // each custodian account's id, and the account it belongs to
    let mut custodian: BTreeMap<String, String> = BTreeMap::new();
    for v in nodes {
        let n = Node::root(v);
        for c in n.list("custodianAccounts")? {
            custodian.insert(c.text("id")?.to_string(), n.text("id")?.to_string());
        }
    }
    let mut out = Vec::new();
    for v in nodes {
        let n = Node::root(v);
        let status = n.text("status")?;
        let open = match status {
            "open" => true,
            "closed" => false,
            other => return Err(n.field("status")?.mismatch(format!("expected open or closed, found {other:?}"))),
        };
        let linked = n.field("linkedAccount")?;
        let linked_to = if matches!(linked.value(), Value::Null) { None } else { Some(linked.text("id")?.to_string()) };
        let backs = match margin_boost(&n)? {
            None => None,
            Some(target) => Some(custodian.get(&target).cloned().ok_or_else(|| n.field("accountFeatures").map(|f| f.mismatch(format!("a Margin Boost naming custodian account {target:?}, which no account stated holds"))).unwrap_or_else(|m| m))?),
        };
        // what it is worth now: its combined financials' net liquidation value
        // (the field is asked for: an answer without it is not the one asked for)
        let net_value = match n.field("financials")?.value() {
            Value::Null => None,
            _ => {
                let f = n.obj("financials")?;
                match f.field("currentCombined")?.value() {
                    Value::Null => None,
                    _ => {
                        let v = f.obj("currentCombined")?.obj("netLiquidationValue")?;
                        let amount = Dec::parse_to_fit(v.text("amount")?).map_err(|e| v.field("amount").map(|x| x.mismatch(e.to_string())).unwrap_or_else(|m| m))?;
                        let currency = Currency::parse(v.text("currency")?).map_err(|e| v.field("currency").map(|x| x.mismatch(e.to_string())).unwrap_or_else(|m| m))?;
                        Some(Money::new(amount, currency))
                    }
                }
            }
        };
        out.push(AccountStated {
            key: n.text("id")?.to_string(),
            net_value,
            account_type: account_type(n.text("unifiedAccountType")?),
            open,
            nickname: n.opt_text("nickname")?.map(str::to_string),
            linked_to,
            backs,
        });
    }
    Ok(out)
}

/// An account's positions as of a day (`FetchHoldingsExportPositionsAsOfDate`'s
/// nodes): each security's units, a short's as a negative quantity. Cash is
/// stated apart, and each position's value now (`totalValue`), which decides
/// whether a coin's amount is dust by the broker's own figure, and its book
/// value (`bookValue`, the cost Wealthsimple states for it, in the currency it
/// states), which the average cost the book keeps is checked against (brief 20
/// §4). `marketBookValue` beside it is not read: in every recorded reply it is
/// the same amount in the same currency. Both are written to more digits than a
/// decimal holds: each is taken to the places that fit.
///
/// Wealthsimple writes a short's quantity negative and a long's not (every
/// recorded reply): a quantity whose sign contradicts its direction is a
/// mismatch, not a sign to correct.
pub fn units(nodes: &Value) -> Read<Vec<Units>> {
    let mut out = Vec::new();
    for n in Node::root(nodes).as_list()? {
        let id = n.obj("security")?.text("id")?;
        if id.starts_with("sec-c-") {
            continue;
        }
        let quantity = n.dec_text("quantity")?;
        let agrees = match n.text("direction")? {
            "LONG" => !quantity.is_negative(),
            "SHORT" => !quantity.is_positive(),
            other => return Err(n.field("direction")?.mismatch(format!("expected LONG or SHORT, found {other:?}"))),
        };
        if !agrees {
            return Err(n.field("quantity")?.mismatch(format!("{} units held {}", quantity.to_text(), n.text("direction")?)));
        }
        let money = |key: &str| -> Read<Money> {
            let v = n.obj(key)?;
            let amount = Dec::parse_to_fit(v.text("amount")?).map_err(|e| v.field("amount").map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))?;
            let currency = Currency::parse(v.text("currency")?).map_err(|e| v.field("currency").map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))?;
            Ok(Money::new(amount, currency))
        };
        let value = Some(money("totalValue")?);
        let book_value = Some(money("bookValue")?);
        out.push(Units { instrument: Reference::new(RefScheme::BrokerSecurity(broker()), id), quantity, book_value, value });
    }
    Ok(out)
}

/// An account's value and net deposits per day (`historicalDaily`'s nodes).
pub fn history(nodes: &[Value]) -> Read<Vec<DayValue>> {
    let mut out = Vec::new();
    for v in nodes {
        let n = Node::root(v);
        out.push(DayValue { day: n.day("date")?, net_value: cents(&n.obj("netLiquidationValueV2")?)?, net_deposits: cents(&n.obj("netDepositsV2")?)? });
    }
    out.sort_by_key(|d| d.day);
    Ok(out)
}

/// Each account's cash per currency (`FetchAccountsWithBalance`'s accounts):
/// the balances of its custodian accounts held as `sec-c-<currency>`.
pub fn cash(accounts: &[Value]) -> Read<BTreeMap<String, BTreeMap<Currency, Dec>>> {
    let mut out: BTreeMap<String, BTreeMap<Currency, Dec>> = BTreeMap::new();
    for v in accounts {
        let a = Node::root(v);
        let e = out.entry(a.text("id")?.to_string()).or_default();
        for c in a.list("custodianAccounts")? {
            for b in c.obj("financials")?.list("balance")? {
                let sec = b.text("securityId")?;
                let Some(code) = sec.strip_prefix("sec-c-") else { continue };
                let currency = Currency::parse(&code.to_uppercase()).map_err(|x| b.field("securityId").map(|f| f.mismatch(x.to_string())).unwrap_or_else(|m| m))?;
                let q = b.dec_text("quantity")?;
                let v = e.entry(currency).or_insert(Dec::ZERO);
                *v = v.checked_add(q).map_err(|x| b.mismatch(format!("a balance too large to add: {x}")))?;
            }
        }
    }
    Ok(out)
}

/// What a margin account can borrow now, in CAD (`FetchAccountCurrentMarginBuyingPowerV2`'s
/// `account`, of the account asked for): the amount where Wealthsimple states one, or
/// why it cannot (`BuyingPowerMetricUnavailable`'s reason, with how many securities
/// hold it back).
pub fn buying_power(node: &Value, account: &str) -> Read<Result<Dec, String>> {
    let n = Node::root(node);
    let id = n.text("id")?;
    if id != account {
        return Err(n.field("id")?.mismatch(format!("the account {id}, not {account}, which was asked")));
    }
    let bp = n.obj("financials")?.obj("current")?.obj("marginV3")?.obj("trading")?.obj("buyingPower")?;
    match bp.text("__typename")? {
        "BuyingPowerMetricAvailable" => {
            let total = bp.obj("total")?;
            let currency = total.text("currency")?;
            if currency != "CAD" {
                return Err(total.field("currency")?.mismatch(format!("{currency}, where CAD was asked")));
            }
            Ok(Ok(total.dec_text("amount")?))
        }
        "BuyingPowerMetricUnavailable" => {
            let reason = bp.obj("reason")?;
            let why = reason.text("__typename")?;
            Ok(Err(match reason.field("securities").ok().map(|s| s.as_list()).transpose()? {
                Some(held) if !held.is_empty() => format!("{why} ({} securities)", held.len()),
                _ => why.to_string(),
            }))
        }
        other => Err(bp.field("__typename")?.mismatch(format!("{other} is neither available nor unavailable"))),
    }
}

/// What is owed on a credit card now (`creditCardAccount.balance.current`: the
/// posted purchases less payments; `pending` is apart), of the card account
/// asked for.
/// What `ownershipType` says of an outside account, as Wealthsimple's own web
/// app sets it (`individual`, `joint`, `joint_with_spouse`): each is the
/// person's own debt where the account is theirs, held alone or jointly.
pub const OWNERSHIPS: [&str; 3] = ["individual", "joint", "joint_with_spouse"];

/// The person's own accounts held outside Wealthsimple that its net worth feature
/// states, from the reply's `identity` node. One of another identity (a household
/// member's, shown to the household) is not the person's and is left out. Each
/// one's kind (its type, `StudentLoanExternalFinancialEntity`
/// read as `StudentLoan`), name, institution, the last four digits of its number
/// where stated, its balance and when that balance was stated. Read strictly: a
/// field of another shape is a named mismatch, never a quiet zero.
pub fn outside_accounts(node: &Value) -> Read<Vec<bagholder_book::debts::OutsideDebt>> {
    let identity = Node::root(node);
    let me = identity.text("id")?;
    let mut out = Vec::new();
    for e in identity.obj("netWorth")?.list("externalFinancialEntities")? {
        let ownership = e.text("ownershipType")?;
        if !OWNERSHIPS.contains(&ownership) {
            return Err(e.field("ownershipType")?.mismatch(format!("{ownership:?} is not an ownership Wealthsimple states")));
        }
        e.obj("visibility")?.bool("household")?;
        // a household member's account, shown to the household, is not the person's
        if e.text("identityId")? != me {
            continue;
        }
        let typename = e.text("__typename")?;
        let kind = typename.strip_suffix("ExternalFinancialEntity").ok_or_else(|| e.field("__typename").map(|f| f.mismatch(format!("{typename} is not an outside account's type"))).unwrap_or_else(|m| m))?;
        let balance = e.obj("balance")?;
        let currency = balance.text("currency")?;
        let as_of = match e.opt_text("balanceAsOf")? {
            None => None,
            Some(t) => Some(t.parse::<jiff::Timestamp>().map_err(|err| e.field("balanceAsOf").map(|f| f.mismatch(format!("not an instant: {err}"))).unwrap_or_else(|m| m))?),
        };
        out.push(bagholder_book::debts::OutsideDebt {
            entity_id: e.text("id")?.to_string(),
            kind: kind.to_string(),
            name: e.opt_text("displayName")?.unwrap_or("").to_string(),
            institution: e.opt_text("institutionName")?.unwrap_or("").to_string(),
            last4: e.opt_text("accountNumberLast4")?.map(String::from),
            // as stated: a debt's balance owed is positive (the person's own
            // capture, 2026-10-19: every debt positive); one in credit is negative
            balance: balance.dec_text("amount")?,
            currency: Currency::parse(currency).map_err(|err| balance.field("currency").map(|f| f.mismatch(err.to_string())).unwrap_or_else(|m| m))?,
            as_of,
        });
    }
    Ok(out)
}

pub fn card_balance(node: &Value, account: &str) -> Read<Dec> {
    let n = Node::root(node);
    let id = n.text("id")?;
    if id != account {
        return Err(n.field("id")?.mismatch(format!("the card account {id}, not {account}, which was asked")));
    }
    n.obj("balance")?.dec_text("current")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(text: &str) -> Value {
        bagholder_core::json::parse(text).unwrap()
    }

    /// Each outside account as the net worth feature states it: its kind from its
    /// type, a debt's balance owed whatever sign it carries, and when it was stated.
    #[test]
    fn the_outside_accounts_are_read_as_stated() {
        let got = outside_accounts(&node(
            r#"{"id": "me", "netWorth": {"externalFinancialEntities": [
                {"__typename": "StudentLoanExternalFinancialEntity", "id": "efe-1", "identityId": "me", "ownershipType": "individual", "visibility": {"household": false}, "displayName": "Student loan", "institutionName": "Omicron Trust", "accountNumberLast4": null, "balance": {"amount": "-12500.40", "currency": "CAD"}, "balanceAsOf": "2026-10-01T12:00:00Z"},
                {"__typename": "CreditCardExternalFinancialEntity", "id": "efe-2", "identityId": "me", "ownershipType": "joint_with_spouse", "visibility": {"household": true}, "displayName": "Visa", "institutionName": "Zeta Bank", "accountNumberLast4": "4821", "balance": {"amount": "830.00", "currency": "CAD"}, "balanceAsOf": null},
                {"__typename": "ChequingExternalFinancialEntity", "id": "efe-3", "identityId": "me", "ownershipType": "joint", "visibility": {"household": false}, "displayName": "Chequing", "institutionName": null, "accountNumberLast4": null, "balance": {"amount": "200", "currency": "CAD"}, "balanceAsOf": null},
                {"__typename": "MortgageExternalFinancialEntity", "id": "efe-4", "identityId": "spouse", "ownershipType": "individual", "visibility": {"household": true}, "displayName": "Their mortgage", "institutionName": "Omicron Trust", "accountNumberLast4": null, "balance": {"amount": "400000", "currency": "CAD"}, "balanceAsOf": null}
            ]}}"#,
        ))
        .unwrap();
        assert_eq!(got.iter().map(|d| (d.kind.as_str(), d.balance.to_string())).collect::<Vec<_>>(), [("StudentLoan", "-12500.4".to_string()), ("CreditCard", "830".to_string()), ("Chequing", "200".to_string())], "each as stated");
        assert_eq!(got[1].last4.as_deref(), Some("4821"));
        assert!(got[0].as_of.is_some() && got[1].as_of.is_none());
        assert_eq!(got[2].institution, "", "an account entered by hand may name no institution");
        assert!(got.iter().all(|d| d.entity_id != "efe-4"), "a household member's mortgage is not the person's");
        assert_eq!(got[1].entity_id, "efe-2", "a joint debt is the person's");
    }

    /// The person's own reply (2026-10-19, anonymised): every outside account is
    /// read, each debt's balance owed stated positive.
    #[test]
    fn the_persons_own_reply_reads_with_every_debt_owed_positive() {
        let reply = node(include_str!("../tests/replies/wealthsimple-networth/FetchIdentityNetWorthAccounts-1.json"));
        let identity = Node::root(&reply).obj("data").and_then(|d| d.obj("identity")).unwrap().value().clone();
        let got = outside_accounts(&identity).unwrap();
        assert_eq!(got.len(), 9, "every account is the person's own");
        let debts: Vec<_> = got.iter().filter(|d| crate::repayments::LIABILITY_TYPES.contains(&d.kind.as_str())).collect();
        assert_eq!(debts.len(), 6);
        assert!(debts.iter().all(|d| d.balance.is_positive()), "a debt owed is stated positive");
        assert!(got.iter().all(|d| d.as_of.is_some()));
    }

    /// A field of another shape is a named mismatch, never a quiet zero.
    #[test]
    fn a_reply_of_another_shape_is_a_named_mismatch() {
        let mine = r#""identityId": "me", "ownershipType": "individual", "visibility": {"household": false}"#;
        for (bad, why) in [
            (format!(r#"{{"__typename": "Mortgage", "id": "x", {mine}, "balance": {{"amount": "1", "currency": "CAD"}}}}"#), "a type that is not an outside account's"),
            (format!(r#"{{"__typename": "MortgageExternalFinancialEntity", "id": "x", {mine}, "balance": {{"amount": 1, "currency": "CAD"}}}}"#), "an amount as a number"),
            (format!(r#"{{"__typename": "MortgageExternalFinancialEntity", "id": "x", {mine}, "balance": {{"amount": "1", "currency": "dollars"}}}}"#), "a currency that is not one"),
            (format!(r#"{{"__typename": "MortgageExternalFinancialEntity", "id": "x", {mine}, "balance": {{"amount": "1", "currency": "CAD"}}, "balanceAsOf": "yesterday"}}"#), "an instant that is not one"),
            (r#"{"__typename": "MortgageExternalFinancialEntity", "id": "x", "identityId": "me", "ownershipType": "shared", "visibility": {"household": false}, "balance": {"amount": "1", "currency": "CAD"}}"#.to_string(), "an ownership Wealthsimple does not state"),
            (r#"{"__typename": "MortgageExternalFinancialEntity", "id": "x", "identityId": "me", "ownershipType": "individual", "balance": {"amount": "1", "currency": "CAD"}}"#.to_string(), "no visibility"),
        ] {
            assert!(outside_accounts(&node(&format!(r#"{{"id": "me", "netWorth": {{"externalFinancialEntities": [{bad}]}}}}"#))).is_err(), "{why}");
        }
    }
}
