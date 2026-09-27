//! Wealthsimple's monthly statements (`docs/plans/statement-gaps.md`): the
//! reply to `FetchMonthlyStatementWithTransactions` read strictly into rows,
//! and the mapping of a row booked into the book because the activity feed left
//! the movement out.

use bagholder_book::mapping::{Draft, MapContext, Mapped, Mapping};
use bagholder_broker::{codes, StatementRow};
use bagholder_core::account::AccountRef;
use bagholder_core::json::{self, Value};
use bagholder_core::record::Problem;
use bagholder_core::transaction::Kind;
use bagholder_core::{Currency, Leg, Money, SourceName};
use bagholder_sources::reply::{Mismatch, Node};

/// A spending account's statement.
pub const CASH: &str = "cash_monthly_statement";
/// Every other account's in the book.
pub const BROKERAGE: &str = "brokerage_monthly_statement";

/// The rows of a `monthlyStatement` node. A brokerage statement states its
/// rows per currency (`activitiesPerCurrency`); a cash statement's rows are in
/// the account's own currency, `currency`.
pub fn rows(node: &Value, currency: Currency) -> Result<Vec<StatementRow>, Mismatch> {
    let n = Node::root(node);
    let data = n.obj("data")?;
    let mut out = Vec::new();
    match data.text("__typename")? {
        "CashMonthlyStatementObject" => {
            for r in data.list("currentTransactions")? {
                out.push(row(&r, currency)?);
            }
        }
        // an account in one currency (a crypto account's) may state no lists per
        // currency: its rows are in its own
        "BrokerageMonthlyStatementObject" if matches!(data.field("activitiesPerCurrency")?.value(), Value::Null) => {
            for r in data.list("currentTransactions")? {
                out.push(row(&r, currency)?);
            }
        }
        "BrokerageMonthlyStatementObject" => {
            for per in data.list("activitiesPerCurrency")? {
                let c = per.field("currency")?;
                let currency = Currency::parse(c.as_text()?).map_err(|e| c.mismatch(e.to_string()))?;
                // a row's cash is in its list's currency; its `unit` names what it
                // moved (`$CAD` for cash, a symbol for units), not its currency
                for r in per.list("currentTransactions")? {
                    out.push(row(&r, currency)?);
                }
            }
        }
        other => return Err(data.field("__typename")?.mismatch(format!("a statement of a kind not read: {other:?}"))),
    }
    Ok(out)
}

fn row(r: &Node, currency: Currency) -> Result<StatementRow, Mismatch> {
    let description = r.text("description")?.to_string();
    let executed = codes::executed_at(&description).map_err(|e| r.field("description").map(|f| f.mismatch(e)).unwrap_or_else(|m| m))?;
    Ok(StatementRow {
        day: r.day("transactionDate")?,
        executed,
        code: r.text("transactionType")?.to_string(),
        description,
        currency,
        cash: r.dec_text("cashMovement")?,
        balance: r.dec_text("balance")?,
    })
}

pub fn source() -> SourceName {
    SourceName::named("wealthsimple-statement")
}

/// The record a booked row is kept as: the row as the statement stated it,
/// with the account, month and currency it was read under and its place among
/// them.
pub fn payload(account: &str, month: jiff::civil::Date, position: usize, row: &StatementRow) -> Value {
    let text = |s: &str| Value::String(s.to_string());
    Value::Object(
        [
            ("account", text(account)),
            ("month", text(&month.to_string())),
            ("position", Value::Number(position.to_string())),
            ("currency", text(row.currency.as_str())),
            ("transactionDate", text(&row.day.to_string())),
            ("transactionType", text(&row.code)),
            ("description", text(&row.description)),
            ("cashMovement", text(&row.cash.to_text())),
            ("balance", text(&row.balance.to_text())),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
    )
}

/// The key a booked row is kept by: one per row of one account's month.
pub fn key(account: &str, month: jiff::civil::Date, position: usize) -> String {
    format!("{account}|{month}|{position}")
}

pub struct StatementMapping;

impl Mapping for StatementMapping {
    fn source(&self) -> SourceName {
        source()
    }
    fn version(&self) -> u32 {
        1
    }
    fn map(&self, _ctx: &MapContext, payload: &str) -> Mapped {
        let v = match json::parse(payload) {
            Ok(v) => v,
            Err(e) => return Mapped::unreadable(format!("a statement row that is not JSON: {e}")),
        };
        match map_row(&v) {
            Ok(m) => m,
            Err(m) => Mapped::unreadable(format!("a statement row not of the shape kept: {m}")),
        }
    }
}

fn map_row(v: &Value) -> Result<Mapped, Mismatch> {
    let n = Node::root(v);
    let c = n.field("currency")?;
    let currency = Currency::parse(c.as_text()?).map_err(|e| c.mismatch(e.to_string()))?;
    let row = row(&n, currency)?;
    let draft = Draft {
        leg: Leg::parse("trade").expect("a leg name written in the code"),
        account: AccountRef::new(crate::mapping::broker(), n.text("account")?.to_string()),
        occurred_at: None,
        trade_date: row.book_day(),
        settle_date: row.executed.map(|_| row.day),
        kind: Kind::Unclassified,
        effect: None,
        instrument: None,
        quantity: None,
        price: None,
        cash: Some(Money::new(row.cash, currency)),
        fee: None,
        fx_rate: None,
        paid_on: None,
        value: None,
    };
    Ok(match codes::kind(&row.code) {
        // a trade is the feed's to state, with its units and price: never booked from a statement
        Some(Kind::Buy | Kind::Sell) | None => Mapped { legs: vec![draft], problems: vec![Problem::new("unclassified", format!("a statement row of code {:?}, which is not booked", row.code))], adjustments: vec![] },
        Some(kind) => Mapped { legs: vec![Draft { kind, ..draft }], problems: vec![], adjustments: vec![] },
    })
}
