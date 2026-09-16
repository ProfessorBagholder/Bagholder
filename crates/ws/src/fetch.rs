//! Asking Wealthsimple for the book: the accounts, the activity feed, the
//! balances and the margin figures.
//!
//! Every one of these walks the broker's own pagination to the end. A page of
//! rows the store already has can still carry a revised one, and the rows
//! behind it are new, so a page is never taken as a stopping point.

use serde_json::{json, Value};

use crate::session::{CallError, Client};
use bagholder_model::value::{field_s, get, num};

/// `bagholder.fetch_all_accounts`.
pub fn fetch_all_accounts(client: &Client, sess: &Value, identity_id: &str) -> Result<Vec<Value>, CallError> {
    let mut accounts: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let variables = json!({
            "identityId": identity_id,
            "pageSize": 25,
            "startDate": "2015-01-01",
            "cursor": cursor,
        });
        let data = client.graphql(sess, "FetchAllAccountFinancials", &variables, None)?;
        let conn = data.get("identity").and_then(|i| i.get("accounts")).cloned().unwrap_or(Value::Null);
        if let Some(edges) = conn.get("edges").and_then(|v| v.as_array()) {
            for edge in edges {
                if let Some(node) = edge.get("node").filter(|n| !n.is_null()) {
                    accounts.push(node.clone());
                }
            }
        }
        let page = conn.get("pageInfo").cloned().unwrap_or(Value::Null);
        if page.get("hasNextPage").and_then(|v| v.as_bool()) != Some(true) {
            break;
        }
        let next = field_s(&page, "endCursor");
        if next.is_empty() {
            break;
        }
        cursor = Some(next);
    }
    Ok(accounts)
}

/// `bagholder.activity_fetch_condition`: the window one pull asks for. The
/// start date is what turns a daily pull into new rows only.
pub fn activity_fetch_condition(account_id: &str, start_date: Option<&str>, now_unix: i64) -> Value {
    let end_secs = now_unix + 86400;
    let days = end_secs.div_euclid(86400);
    let rem = end_secs.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    let end = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.999Z",
        y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60
    );
    let mut cond = serde_json::Map::new();
    cond.insert("endDate".into(), json!(end));
    cond.insert("accountIds".into(), json!([account_id]));
    if let Some(raw) = start_date {
        let raw = raw.trim();
        if !raw.is_empty() {
            let full = if raw.contains('T') {
                raw.to_string()
            } else {
                format!("{}T00:00:00.000Z", raw.chars().take(10).collect::<String>())
            };
            cond.insert("startDate".into(), json!(full));
        }
    }
    Value::Object(cond)
}

/// `bagholder.fetch_activities_for_account`.
pub fn fetch_activities_for_account(
    client: &Client,
    sess: &Value,
    account_id: &str,
    start_date: Option<&str>,
    now_unix: i64,
) -> Result<Vec<Value>, CallError> {
    let mut items: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut variables = serde_json::Map::new();
        variables.insert("first".into(), json!(100));
        variables.insert("orderBy".into(), json!("OCCURRED_AT_DESC"));
        variables.insert("condition".into(), activity_fetch_condition(account_id, start_date, now_unix));
        if let Some(c) = &cursor {
            variables.insert("cursor".into(), json!(c));
        }
        let data = client.graphql(sess, "FetchActivityFeedItems", &Value::Object(variables), None)?;
        let feed = data.get("activityFeedItems").cloned().unwrap_or(Value::Null);
        if let Some(edges) = feed.get("edges").and_then(|v| v.as_array()) {
            for edge in edges {
                if let Some(node) = edge.get("node").filter(|n| !n.is_null()) {
                    items.push(node.clone());
                }
            }
        }
        let page = feed.get("pageInfo").cloned().unwrap_or(Value::Null);
        // Walk every page the broker returns for the window: a page of known
        // rows can still carry a revised one, and the rows behind it are new.
        if page.get("hasNextPage").and_then(|v| v.as_bool()) != Some(true) {
            break;
        }
        let next = field_s(&page, "endCursor");
        if next.is_empty() {
            break;
        }
        cursor = Some(next);
    }
    Ok(items)
}

/// `bagholder.fetch_balances`: twenty accounts to a request.
pub fn fetch_balances(client: &Client, sess: &Value, account_ids: &[String]) -> Result<Vec<Value>, CallError> {
    let ids: Vec<&String> = account_ids.iter().filter(|i| !i.is_empty()).collect();
    let mut balances = Vec::new();
    for chunk in ids.chunks(20) {
        let list: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();
        let data = client.graphql(sess, "FetchAccountsWithBalance", &json!({"ids": list, "type": "TRADING"}), None)?;
        let accounts = data.get("accounts").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        for acc in accounts {
            let aid = acc.get("id").cloned().unwrap_or(Value::Null);
            let custodians = acc.get("custodianAccounts").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            for ca in custodians {
                let fin = ca.get("financials").cloned().unwrap_or(Value::Null);
                let bals = match fin.get("balance") {
                    Some(Value::Array(a)) => a.clone(),
                    Some(o) if o.is_object() => vec![o.clone()],
                    _ => vec![],
                };
                for b in bals {
                    balances.push(json!({
                        "accountId": aid,
                        "custodianAccountId": ca.get("id").cloned().unwrap_or(Value::Null),
                        "securityId": b.get("securityId").cloned().unwrap_or(Value::Null),
                        "quantity": b.get("quantity").cloned().unwrap_or(Value::Null),
                    }));
                }
            }
        }
    }
    Ok(balances)
}

/// `bagholder.parse_margin`: the buying power as Wealthsimple answers it -- a
/// Money when it is available, the reason when it is not, and nothing at all
/// when the account has no margin figures.
pub fn parse_margin(data: &Value) -> Option<Value> {
    let trading = data
        .get("account")?
        .get("financials")?
        .get("current")?
        .get("marginV3")?
        .get("trading")
        .cloned()
        .unwrap_or(Value::Null);
    let bp = trading.get("buyingPower").filter(|b| b.is_object())?;

    if field_s(bp, "__typename") == "BuyingPowerMetricAvailable" {
        let total = bp.get("total").cloned().unwrap_or(Value::Null);
        let amount = match get(&total, "amount") {
            Some(v) => {
                let f = num(Some(v), f64::NAN);
                if f.is_nan() { return None } else { f }
            }
            None => return None,
        };
        let currency = { let c = field_s(&total, "currency"); if c.is_empty() { "CAD".to_string() } else { c } };
        return Some(json!({"buyingPower": amount, "currency": currency, "unavailable": ""}));
    }
    let reason = bp.get("reason").cloned().unwrap_or(Value::Null);
    let mut why = field_s(&reason, "__typename");
    if why.is_empty() {
        why = field_s(bp, "__typename");
    }
    if why.is_empty() {
        why = "unavailable".into();
    }
    let n = reason.get("securities").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    if n > 0 {
        why = format!("{} ({} securities)", why, n);
    }
    Some(json!({"buyingPower": Value::Null, "currency": "CAD", "unavailable": why}))
}

/// `bagholder.margin_account_ids`: the open margin accounts, which are the
/// only ones whose buying power is margin available.
///
/// Wealthsimple answers the buying-power query for every self-directed account
/// with the cash it could buy with, and with an error for cash, card and
/// crypto accounts. Neither is margin.
pub fn margin_account_ids(accounts: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    for a in accounts {
        let typ = {
            let t = field_s(a, "unifiedAccountType");
            if t.is_empty() { field_s(a, "unified_account_type") } else { t }
        }
        .to_uppercase();
        let status = field_s(a, "status").to_lowercase();
        let id = field_s(a, "id");
        if !id.is_empty() && typ.contains("MARGIN") && status != "closed" {
            out.push(id);
        }
    }
    out
}

/// `bagholder._money_amount`: the first Money.amount present among the named
/// keys, with its currency.
pub fn money_amount(node: &Value, keys: &[&str]) -> (Option<f64>, Option<String>) {
    if !node.is_object() {
        return (None, None);
    }
    for key in keys {
        let money = match node.get(*key) { Some(m) if m.is_object() => m, _ => continue };
        let amount = match get(money, "amount") { Some(a) => a, None => continue };
        let f = num(Some(amount), f64::NAN);
        if f.is_nan() {
            continue;
        }
        let ccy = { let c = field_s(money, "currency"); if c.is_empty() { "CAD".to_string() } else { c } };
        return (Some(f), Some(ccy));
    }
    (None, None)
}
