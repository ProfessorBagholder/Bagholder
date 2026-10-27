//! What an earlier version booked into the book from Wealthsimple's monthly
//! statements (`docs/plans/statement-gaps.md`), mapped. The statements are no
//! longer read (brief 79, `docs/plans/every-amount-as-wealthsimple-states-it.md`
//! §3.7): the pull withdraws every record of theirs (`pull::withdraw_statement_records`),
//! and this mapping stays only because migration 039 maps the records a book
//! older than it holds. It goes with the statement tables at the next schema
//! change.

use bagholder_book::mapping::{Draft, MapContext, Mapped, Mapping};
use bagholder_broker::codes;
use bagholder_core::account::AccountRef;
use bagholder_core::json::{self, Value};
use bagholder_core::record::Problem;
use bagholder_core::transaction::Kind;
use bagholder_core::{Currency, Leg, Money, SourceName};
use bagholder_sources::reply::{Mismatch, Node};

/// A row as a statement stated it, kept in its record.
struct Row {
    /// The day the statement filed it under (a trade's settlement).
    day: jiff::civil::Date,
    /// The day it states it was executed, where it states one.
    executed: Option<jiff::civil::Date>,
    /// The broker's transaction code (`WD`, `TRFIN`, …).
    code: String,
    /// The signed cash it moved.
    cash: bagholder_core::Dec,
}

fn row(r: &Node) -> Result<Row, Mismatch> {
    let description = r.text("description")?;
    let executed = codes::executed_at(description).map_err(|e| r.field("description").map(|f| f.mismatch(e)).unwrap_or_else(|m| m))?;
    Ok(Row { day: r.day("transactionDate")?, executed, code: r.text("transactionType")?.to_string(), cash: r.dec_text("cashMovement")? })
}

pub fn source() -> SourceName {
    SourceName::named("wealthsimple-statement")
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
            return draft(OPENING, n.day("month")?, Kind::Fee, moved);
        }
        Some(PAID) => return draft("paid", n.day("day")?, Kind::CurrencyConversion, n.dec_text("cashMovement")?),
        Some(FILL) => return fill(&n, currency),
        Some(other) => return Err(n.field("kind")?.mismatch(format!("a record of a kind not kept: {other:?}"))),
    }
    let row = row(&n)?;
    let draft = Draft {
        leg: Leg::parse("trade").expect("a leg name written in the code"),
        account: AccountRef::new(crate::mapping::broker(), n.text("account")?.to_string()),
        occurred_at: None,
        // filed at its execution where it states one, as the feed dates a trade
        trade_date: row.executed.unwrap_or(row.day),
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
