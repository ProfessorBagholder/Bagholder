//! The rows Wealthsimple does not post but the book implies: the shares an
//! assignment delivered, and the expiry of a contract that is past its date.
//! They are derived rows; the stored rows are never rewritten.

use std::collections::HashSet;

use crate::activity::{Activity, Category, Direction, Flag, Kind};
use crate::dates::option_expiry;
use crate::fifo::Lot;
use crate::symbols::{is_option_symbol, underlying_symbol};
use crate::value::{fold_spaces_upper, num_repr, FSum, EPS};

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

/// An assigned short option delivers shares, but Wealthsimple posts only the
/// option row, with the strike cash on it. A call assignment sells contracts x
/// 100 shares at the strike, a put assignment buys them.
pub fn synthesize_assignment_shares(activities: &[Activity], underlying_id_of: &dyn Fn(&str) -> Option<String>) -> Vec<Activity> {
    let mut out = Vec::new();
    for a in activities {
        if a.category != Category::OptionEvent || a.type_c() != "ASSIGN" || !is_option_symbol(&a.symbol) {
            continue;
        }
        let contracts = a.quantity.abs();
        if contracts <= 0.0 {
            continue;
        }
        let shares = contracts * 100.0;
        let cash = a.net_cash_amount;
        let mut strike = if cash.abs() > EPS { cash.abs() / shares } else { 0.0 };
        if strike <= 0.0 {
            strike = strike_from_symbol(&a.symbol);
        }
        if strike <= 0.0 {
            continue;
        }
        let upper = a.symbol.to_uppercase();
        let trimmed = upper.trim_end();
        let is_call = trimmed.ends_with("CALL") || trimmed.ends_with(" C");
        let sell = if cash.abs() <= EPS { is_call } else { cash > 0.0 };
        let under = underlying_symbol(&a.symbol);
        let or_account = |id: &str| if id.is_empty() { a.account_id.clone() } else { id.to_string() };
        out.push(Activity {
            id: format!("assign-shares:{}", a.id),
            occurred_at: if a.occurred_at.is_empty() { format!("{}T21:30:00+00:00", a.transaction_date) } else { a.occurred_at.clone() },
            transaction_date: a.transaction_date.clone(),
            account_id: a.account_id.clone(),
            book_id: or_account(&a.book_id),
            fifo_id: or_account(&a.fifo_id),
            account_name: a.account_name.clone(),
            activity_type: "Trade".into(),
            activity_sub_type: if sell { "SELL" } else { "BUY" }.into(),
            description: format!("{}: {} {} @ {}", if sell { "Called away" } else { "Put to you" }, num_repr(shares), under, num_repr(strike)),
            cash_direction: if sell { "CREDIT" } else { "DEBIT" }.into(),
            symbol: under.clone(),
            name: under,
            currency: a.currency.clone(),
            quantity: if sell { -shares } else { shares },
            unit_price: strike,
            commission: 0.0,
            net_cash_amount: if sell { shares * strike } else { -shares * strike },
            category: Category::Trade,
            raw_type: "OPTIONS_ASSIGN_SHARES".into(),
            aft_type: String::new(),
            security_id: underlying_id_of(&a.security_id).unwrap_or_default(),
            kind: Kind::Shares,
            flags: vec![Flag::Assignment],
        });
    }
    out
}

/// Wealthsimple does not always post an expiry row, so an option lot still open
/// after its expiry date is closed at $0 on that date.
pub fn synthesize_expiries(open_lots: &[Lot], today: &str) -> Vec<Activity> {
    let mut out = Vec::new();
    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    for lot in open_lots {
        let exp = option_expiry(&lot.symbol);
        if exp.is_empty() || exp.as_str() >= today {
            continue;
        }
        let key = (lot.account_type.clone(), lot.symbol.clone(), lot.currency.clone());
        if !seen.insert(key.clone()) {
            continue;
        }
        let qty: f64 = open_lots
            .iter()
            .filter(|l| (&l.account_type, &l.symbol, &l.currency) == (&key.0, &key.1, &key.2) && l.direction == lot.direction)
            .map(|l| l.qty)
            .fsum();
        if qty <= EPS {
            continue;
        }
        let short = lot.direction == Direction::Short;
        out.push(Activity {
            id: format!("expiry:{}|{}|{}", lot.account_type, lot.symbol, lot.currency),
            occurred_at: format!("{}T21:30:00+00:00", exp),
            transaction_date: exp,
            account_id: lot.account_id.clone(),
            book_id: lot.account_id.clone(),
            fifo_id: lot.account_id.clone(),
            account_name: lot.account_type.clone(),
            activity_type: "EXPIR".into(),
            activity_sub_type: if short { "BUY" } else { "SELL" }.into(),
            description: format!("Expired (assumed): {}", lot.symbol),
            cash_direction: String::new(),
            symbol: lot.symbol.clone(),
            name: lot.name.clone(),
            currency: lot.currency.clone(),
            quantity: if short { qty } else { -qty },
            unit_price: 0.0,
            commission: 0.0,
            net_cash_amount: 0.0,
            category: Category::OptionEvent,
            raw_type: if short { "OPTIONS_SHORT_EXPIRY" } else { "OPTIONS_EXPIRY" }.into(),
            aft_type: String::new(),
            security_id: lot.security_id.clone(),
            kind: Kind::Options,
            flags: vec![Flag::AssumedExpiry],
        });
    }
    out
}
