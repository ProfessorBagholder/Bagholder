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

/// A fill as the statement states it (`statements::Fill`): the statement's row,
/// the units it states, and what the feed's row states besides, which it keeps.
pub fn fill_payload(f: &bagholder_broker::statements::Fill) -> Value {
    let text = |s: &str| Value::String(s.to_string());
    let opt = |v: Option<String>| v.map(|s| Value::String(s)).unwrap_or(Value::Null);
    let money = |m: Option<Money>| (opt(m.map(|m| m.amount.to_text())), opt(m.map(|m| m.currency.as_str().to_string())));
    let t = &f.feed;
    let (price, price_currency) = money(t.price);
    let (cash, cash_currency) = money(t.cash);
    let (fee, fee_currency) = money(t.fee);
    let Value::Object(mut row) = payload(&f.account, f.month, f.position, &f.row) else { unreachable!("a row is an object") };
    for (k, v) in [
        ("kind", text(FILL)),
        ("security", text(&f.security.value)),
        ("instrumentKind", text(f.kind.as_str())),
        ("instrumentCurrency", text(f.currency.as_str())),
        ("quantity", text(&f.quantity.to_text())),
        ("side", text(t.kind.as_str())),
        ("effect", opt(t.effect.map(|e| e.as_str().to_string()))),
        ("occurredAt", opt(t.occurred_at.map(|a| a.to_string()))),
        ("tradeDate", text(&t.trade_date.to_string())),
        ("settleDate", opt(t.settle_date.map(|d| d.to_string()))),
        ("price", price),
        ("priceCurrency", price_currency),
        ("cash", cash),
        ("cashCurrency", cash_currency),
        ("fee", fee),
        ("feeCurrency", fee_currency),
        ("fxRate", opt(t.fx_rate.map(|r| r.to_text()))),
    ] {
        row.insert(k.to_string(), v);
    }
    Value::Object(row)
}

/// One per row of one account's month, as a row booked is.
pub fn fill_key(f: &bagholder_broker::statements::Fill) -> String {
    format!("{}|fill", key(&f.account, f.month, f.position))
}

const FILL: &str = "fill";
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

/// A fill as the statement states it: the feed's transaction, with the units
/// the statement states.
fn fill(n: &Node, _row_currency: Currency) -> Result<Mapped, Mismatch> {
    let parsed = |field: &str, what: &str| -> Result<Option<String>, Mismatch> {
        let f = n.field(field)?;
        match f.value() {
            Value::Null => Ok(None),
            _ => f.as_text().map(|t| Some(t.to_string())).map_err(|_| f.mismatch(format!("{what} that is not text"))),
        }
    };
    let currency_of = |field: &str| -> Result<Option<Currency>, Mismatch> {
        parsed(field, "a currency")?.map(|c| Currency::parse(&c).map_err(|e| n.field(field).map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))).transpose()
    };
    let dec_of = |field: &str| -> Result<Option<bagholder_core::Dec>, Mismatch> {
        parsed(field, "a number")?.map(|v| bagholder_core::Dec::parse(&v).map_err(|e| n.field(field).map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))).transpose()
    };
    let money = |amount: &str, currency: &str| -> Result<Option<Money>, Mismatch> { Ok(dec_of(amount)?.zip(currency_of(currency)?).map(|(a, c)| Money::new(a, c))) };
    let side = n.text("side")?;
    let kind = Kind::parse(side).map_err(|e| n.field("side").map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))?;
    let effect = parsed("effect", "an effect")?.map(|e| bagholder_core::transaction::Effect::parse(&e).map_err(|x| n.field("effect").map(|f| f.mismatch(x.to_string())).unwrap_or_else(|m| m))).transpose()?;
    let instrument_kind = bagholder_core::instrument::InstrumentKind::parse(n.text("instrumentKind")?).map_err(|e| n.field("instrumentKind").map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))?;
    let instrument_currency = currency_of("instrumentCurrency")?.ok_or_else(|| n.field("instrumentCurrency").map(|f| f.mismatch("absent".to_string())).unwrap_or_else(|m| m))?;
    let occurred_at = parsed("occurredAt", "an instant")?.map(|a| a.parse::<jiff::Timestamp>().map_err(|e| n.field("occurredAt").map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))).transpose()?;
    let settle_date = parsed("settleDate", "a day")?.map(|d| d.parse::<jiff::civil::Date>().map_err(|e| n.field("settleDate").map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))).transpose()?;
    let security = bagholder_core::instrument::Reference::new(bagholder_core::instrument::RefScheme::BrokerSecurity(crate::mapping::broker()), n.text("security")?.to_string());
    Ok(Mapped {
        legs: vec![Draft {
            leg: Leg::parse("trade").expect("a leg name written in the code"),
            account: AccountRef::new(crate::mapping::broker(), n.text("account")?.to_string()),
            occurred_at,
            trade_date: n.day("tradeDate")?,
            settle_date,
            kind,
            effect,
            instrument: Some(bagholder_book::mapping::InstrumentDraft { refs: vec![security], kind: instrument_kind, currency: instrument_currency, name: None, option: None, standing: None }),
            quantity: Some(n.dec_text("quantity")?),
            price: money("price", "priceCurrency")?,
            cash: money("cash", "cashCurrency")?,
            fee: money("fee", "feeCurrency")?,
            fx_rate: dec_of("fxRate")?,
            paid_on: None,
            value: None,
        }],
        problems: vec![],
        adjustments: vec![], hold: None, orders: vec![]
    })
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
            adjustments: vec![], hold: None, orders: vec![]
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
        Some(FILL) => return fill(&n, currency),
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
        Some(Kind::Buy | Kind::Sell) | None => Mapped { legs: vec![draft], problems: vec![Problem::new("unclassified", format!("a statement row of code {:?}, which is not booked", row.code))], adjustments: vec![], hold: None, orders: vec![] },
        Some(kind) => Mapped { legs: vec![Draft { kind, ..draft }], problems: vec![], adjustments: vec![], hold: None, orders: vec![] },
    })
}
