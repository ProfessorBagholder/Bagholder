//! The rows Wealthsimple does not post but the book implies: the shares an
//! assignment delivered, and the expiry of a contract that is past its date.
//!
//! These are derived rows, marked `source: "derived"`. The raw rows in the
//! store are never rewritten.

use serde_json::{json, Value};
use std::collections::HashSet;

use crate::dates::option_expiry;
use crate::fifo::Lot;
use crate::normalize::kind_of;
use crate::symbols::{is_option_symbol, underlying_symbol};
use crate::value::{compact, field_num, field_s, fold_spaces_upper, EPS};

/// The strike written into a symbol: ` 10.00 CALL` -> 10.0.
fn strike_from_symbol(symbol: &str) -> f64 {
    let u = fold_spaces_upper(symbol);
    let right = if u.ends_with(" CALL") {
        " CALL"
    } else if u.ends_with(" PUT") {
        " PUT"
    } else {
        return 0.0;
    };
    let head = &u[..u.len() - right.len()];
    let tok = match head.rsplit(' ').next() {
        Some(t) if !t.is_empty() => t,
        _ => return 0.0,
    };
    // `\d+(\.\d+)?` and nothing else
    let mut parts = tok.splitn(2, '.');
    let whole = parts.next().unwrap_or("");
    if whole.is_empty() || !whole.bytes().all(|c| c.is_ascii_digit()) {
        return 0.0;
    }
    if let Some(frac) = parts.next() {
        if frac.is_empty() || !frac.bytes().all(|c| c.is_ascii_digit()) {
            return 0.0;
        }
    }
    tok.parse().unwrap_or(0.0)
}

/// `synthesize_assignment_shares`: an assigned short option delivers
/// shares, but Wealthsimple posts only the option row, with the strike cash on
/// it. A call assignment sells contracts x 100 shares at the strike, a put
/// assignment buys them.
pub fn synthesize_assignment_shares(activities: &[Value], underlying_id_of: &dyn Fn(&str) -> Option<String>) -> Vec<Value> {
    let mut out = Vec::new();
    for a in activities {
        if field_s(a, "category") != "option_event" || compact(&field_s(a, "activityType")) != "ASSIGN" {
            continue;
        }
        let symbol = field_s(a, "symbol");
        if !is_option_symbol(&symbol) {
            continue;
        }
        let contracts = field_num(a, "quantity").abs();
        if contracts <= 0.0 {
            continue;
        }
        let shares = contracts * 100.0;
        let cash = field_num(a, "netCashAmount");
        let mut strike = if cash.abs() > EPS { cash.abs() / shares } else { 0.0 };
        if strike <= 0.0 {
            strike = strike_from_symbol(&symbol);
        }
        if strike <= 0.0 {
            continue;
        }
        let upper = symbol.to_uppercase();
        let trimmed = upper.trim_end();
        let is_call = trimmed.ends_with("CALL") || trimmed.ends_with(" C");
        let sell = if cash.abs() <= EPS { is_call } else { cash > 0.0 };
        let under = underlying_symbol(&symbol);
        let under_id = underlying_id_of(&field_s(a, "securityId"));

        let account_id = field_s(a, "accountId");
        let book_id = { let b = field_s(a, "bookId"); if b.is_empty() { account_id.clone() } else { b } };
        let fifo_id = { let b = field_s(a, "fifoId"); if b.is_empty() { account_id.clone() } else { b } };
        let occurred = {
            let o = field_s(a, "occurredAt");
            if o.is_empty() { format!("{}T21:30:00+00:00", field_s(a, "transactionDate")) } else { o }
        };
        out.push(json!({
            "id": format!("assign-shares:{}", field_s(a, "id")),
            "canonicalId": Value::Null,
            "occurredAt": occurred,
            "transactionDate": field_s(a, "transactionDate"),
            "settlementDate": field_s(a, "transactionDate"),
            "accountId": account_id,
            "bookId": book_id,
            "fifoId": fifo_id,
            "accountType": a.get("accountType").cloned().unwrap_or(Value::Null),
            "activityType": "Trade",
            "activitySubType": if sell { "SELL" } else { "BUY" },
            "description": format!("{}: {} {} @ {}",
                if sell { "Called away" } else { "Put to you" },
                crate::value::num_repr(shares), under, crate::value::num_repr(strike)),
            "direction": if sell { "CREDIT" } else { "DEBIT" },
            "symbol": under,
            "name": under,
            "currency": field_s(a, "currency"),
            "quantity": if sell { -shares } else { shares },
            "unitPrice": strike,
            "commission": 0.0,
            "netCashAmount": if sell { shares * strike } else { -shares * strike },
            "category": "trade",
            "balance": Value::Null,
            "source": "derived",
            "rawType": "OPTIONS_ASSIGN_SHARES",
            "aftType": "",
            "counterSymbol": "",
            "securityId": under_id.map(Value::String).unwrap_or(Value::Null),
            "kind": "Shares",
            "flags": ["assignment"],
        }));
    }
    out
}

/// `synthesize_expiries`: Wealthsimple does not always post an expiry
/// row, so an option lot still open after its expiry date is closed at $0 on
/// that date.
pub fn synthesize_expiries(open_lots: &[Lot], today: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    for lot in open_lots {
        let exp = option_expiry(&lot.symbol);
        if exp.is_empty() || exp.as_str() >= today {
            continue;
        }
        let key = (lot.account_type.clone(), lot.symbol.clone(), lot.currency.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key.clone());
        let qty: f64 = open_lots
            .iter()
            .filter(|l| {
                (l.account_type.clone(), l.symbol.clone(), l.currency.clone()) == key && l.direction == lot.direction
            })
            .map(|l| l.qty)
            .fold(0.0, |a, b| a + b);
        if qty <= EPS {
            continue;
        }
        let short = lot.direction == "SHORT";
        out.push(json!({
            "id": format!("expiry:{}|{}|{}", lot.account_type, lot.symbol, lot.currency),
            "canonicalId": Value::Null,
            "occurredAt": format!("{}T21:30:00+00:00", exp),
            "transactionDate": exp,
            "settlementDate": exp,
            "accountId": lot.account_id,
            "bookId": lot.account_id,
            "fifoId": lot.account_id,
            "accountType": lot.account_type,
            "activityType": "EXPIR",
            "activitySubType": if short { "BUY" } else { "SELL" },
            "description": format!("Expired (assumed): {}", lot.symbol),
            "direction": "",
            "symbol": lot.symbol,
            "name": lot.name,
            "currency": lot.currency,
            "quantity": if short { qty } else { -qty },
            "unitPrice": 0.0,
            "commission": 0.0,
            "netCashAmount": 0.0,
            "category": "option_event",
            "balance": Value::Null,
            "source": "derived",
            "rawType": if short { "OPTIONS_SHORT_EXPIRY" } else { "OPTIONS_EXPIRY" },
            "aftType": "",
            "counterSymbol": "",
            "securityId": if lot.security_id.is_empty() { Value::Null } else { Value::String(lot.security_id.clone()) },
            "kind": "Options",
            "flags": ["assumed-expiry"],
        }));
    }
    out
}

/// Kept for callers that hand a row straight to the matcher.
pub fn kind(a: &Value) -> String {
    kind_of(a)
}
