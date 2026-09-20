//! What the book paid out: `build_cashflow`.
//!
//! Dividends, interest, withholding tax and interest charges, each in its own
//! currency with the CAD value converted on the day it was paid.

use crate::activity::{Activity, Category};
use crate::clock::when_parts;
use crate::fx::{to_cad, Fx};
use crate::securities::Securities;
use crate::value::EPS;
use crate::wire::{CashflowRow, Payment};

fn payment(a: &Activity) -> Option<Payment> {
    let is = |name: &str| a.raw_type_c() == name || a.type_c() == name;
    match a.category {
        Category::Dividend => Some(Payment::Dividend),
        Category::Interest => Some(Payment::Interest),
        _ if is("WITHHOLDINGTAX") => Some(Payment::WithholdingTax),
        _ if is("INTERESTCHARGE") => Some(Payment::InterestCharge),
        _ => None,
    }
}

pub fn build_cashflow(activities: &[Activity], securities: &Securities, fx: &Fx) -> Vec<CashflowRow> {
    let mut rows: Vec<CashflowRow> = Vec::new();
    for a in activities {
        let Some(kind) = payment(a) else { continue };
        let cash = a.net_cash_amount;
        if cash.abs() < EPS {
            continue;
        }
        let (day, clock) = when_parts(a.when());
        let ticker = a.symbol.trim();
        let symbol = if !ticker.is_empty() {
            ticker
        } else if matches!(kind, Payment::Interest | Payment::InterestCharge) {
            "Cash"
        } else {
            ""
        };
        rows.push(CashflowRow {
            id: a.id.clone(),
            date: if a.transaction_date.is_empty() { day } else { a.transaction_date.clone() },
            time: clock,
            symbol: if symbol.is_empty() { "—" } else { symbol }.to_string(),
            name: securities.name(&a.security_id, if a.name != symbol { &a.name } else { "" }),
            kind,
            account: if a.account_name.is_empty() { a.account_id.clone() } else { a.account_name.clone() },
            account_id: a.account_id.clone(),
            // a zero reads as nothing to show
            qty: (a.quantity != 0.0).then_some(a.quantity),
            per: (a.unit_price != 0.0).then_some(a.unit_price),
            amount: cash,
            currency: if a.currency.is_empty() { "CAD".to_string() } else { a.currency.clone() },
            amount_cad: to_cad(fx, cash, &a.currency, &a.transaction_date),
        });
    }
    rows.sort_by(|a, b| (&b.date, &b.id).cmp(&(&a.date, &a.id)));
    rows
}
