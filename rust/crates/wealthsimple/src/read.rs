//! Wealthsimple's statements beside its rows, read strictly (`crate::replay`
//! and the network client read them the same way): its accounts, an account's
//! positions as of a day, and its value and net deposits per day.

use bagholder_book::import::wealthsimple_account_type;
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

/// The accounts, from `FetchAllAccounts`' nodes.
pub fn accounts(nodes: &[Value]) -> Read<Vec<AccountStated>> {
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
        out.push(AccountStated {
            key: n.text("id")?.to_string(),
            account_type: wealthsimple_account_type(n.text("unifiedAccountType")?),
            open,
            nickname: n.opt_text("nickname")?.map(str::to_string),
            linked_to,
        });
    }
    Ok(out)
}

/// An account's positions as of a day (`FetchHoldingsExportPositionsAsOfDate`'s
/// nodes): each security's units, a short's as a negative quantity. Cash is
/// stated apart. The book value beside them is not read: no figure uses it (brief
/// 07 §5), and Wealthsimple writes it to more digits than a decimal holds.
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
        out.push(Units { instrument: Reference::new(RefScheme::BrokerSecurity(broker()), id), quantity, book_value: None });
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
pub fn card_balance(node: &Value, account: &str) -> Read<Dec> {
    let n = Node::root(node);
    let id = n.text("id")?;
    if id != account {
        return Err(n.field("id")?.mismatch(format!("the card account {id}, not {account}, which was asked")));
    }
    n.obj("balance")?.dec_text("current")
}
