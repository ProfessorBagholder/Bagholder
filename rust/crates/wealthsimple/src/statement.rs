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

/// A gap booked (`statements::Gap`): the account, the currency, the month
/// before and its closing, the month and its opening, as the statements state them.
pub fn gap_payload(g: &bagholder_broker::statements::Gap) -> Value {
    let text = |s: &str| Value::String(s.to_string());
    Value::Object(
        [
            ("kind", text(OPENING)),
            ("account", text(&g.account)),
            ("currency", text(g.currency.as_str())),
            ("from", text(&g.from.to_string())),
            ("closing", text(&g.closing.to_text())),
            ("month", text(&g.month.to_string())),
            ("opening", text(&g.opening.to_text())),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
    )
}

/// One per account, currency and month.
pub fn gap_key(g: &bagholder_broker::statements::Gap) -> String {
    format!("{}|{}|opening|{}", g.account, g.month, g.currency.as_str())
}

/// A conversion's side paid read from the stated cash (`statements::Paid`):
/// the conversion, its day, the side paid, and the stated cash it is read from.
pub fn paid_payload(p: &bagholder_broker::statements::Paid) -> Value {
    let text = |s: &str| Value::String(s.to_string());
    Value::Object(
        [
            ("kind", text(PAID)),
            ("account", text(&p.account)),
            ("conversion", text(&p.conversion)),
            ("day", text(&p.day.to_string())),
            ("currency", text(p.currency.as_str())),
            ("cashMovement", text(&p.cash.to_text())),
            ("stated", text(&p.stated.to_text())),
            ("statedAt", text(&p.stated_at.to_string())),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
    )
}

/// One per conversion: a later reading of the stated cash revises it.
pub fn paid_key(p: &bagholder_broker::statements::Paid) -> String {
    format!("{}|paid|{}", p.account, p.conversion)
}

const OPENING: &str = "opening";
const PAID: &str = "conversion-paid";

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
    let draft = |leg: &str, day: jiff::civil::Date, kind: Kind, cash: bagholder_core::Dec| -> Result<Mapped, Mismatch> {
        Ok(Mapped {
            legs: vec![Draft {
                leg: Leg::parse(leg).expect("a leg name written in the code"),
                account: AccountRef::new(crate::mapping::broker(), n.text("account")?.to_string()),
                occurred_at: None,
                trade_date: day,
                settle_date: None,
                kind,
                effect: None,
                instrument: None,
                quantity: None,
                price: None,
                cash: Some(Money::new(cash, currency)),
                fee: None,
                fx_rate: None,
                paid_on: None,
                value: None,
            }],
            problems: vec![],
            adjustments: vec![],
        })
    };
    // a row as the statement stated it has no kind; a record kept beside the rows names its own
    let kind = match n.field("kind") {
        Ok(k) => Some(k.as_text()?),
        Err(_) => None,
    };
    match kind {
        None => {}
        // the balance moved as the broker's own cash correction does (`CORRECTION`), on the month's first day
        Some(OPENING) => {
            let (closing, opening) = (n.dec_text("closing")?, n.dec_text("opening")?);
            let moved = opening.checked_sub(closing).map_err(|e| n.field("opening").map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))?;
            return draft(bagholder_broker::statements::gap_leg().as_str(), n.day("month")?, Kind::Fee, moved);
        }
        Some(PAID) => return draft(bagholder_broker::statements::paid_leg().as_str(), n.day("day")?, Kind::CurrencyConversion, n.dec_text("cashMovement")?),
        Some(other) => return Err(n.field("kind")?.mismatch(format!("a record of a kind not kept: {other:?}"))),
    }
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
