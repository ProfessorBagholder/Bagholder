//! The import mapping, `bagholder-import` version 2: an earlier database's
//! activity row as a transaction (`docs/plans/stage-1-foundation.md`, the table
//! of row shapes). Version 2 (`docs/plans/stage-2-engine.md`, "The import,
//! version 2") no longer reads an option fill's "to open" or "to close", which
//! the earlier app wrote itself, and books a multi-leg order's one row as its
//! cash alone, with its legs not stated.
//!
//! What each row states depends on where the earlier app got it:
//! - **synced** from Wealthsimple (`wealthsimple`): an instant, filed on
//!   Alberta's day; cash as Wealthsimple signed it; no price (the stored one was
//!   the app's cash over quantity, times a hundred for an option).
//! - a **booked fill** (`bagholder-fill`, `bagholder`): the order's average fill
//!   price as the broker stated it; the cash was the app's arithmetic, so none.
//! - a **file** or a **typed-in row** (`csv`, `statement`, `canonical`, `legacy`,
//!   `manual`): the day, price and cash the file stated; no instant.
//!
//! A row's cash keeps its sign except where the table says the old rows got it
//! wrong (a coin's or an event contract's purchase recorded as a credit: a buy
//! pays). Anything not in the table is unclassified, with a problem naming it.

use bagholder_core::account::AccountRef;
use bagholder_core::instrument::{InstrumentKind, OptionRight, RefScheme, Reference};
use bagholder_core::record::Problem;
use bagholder_core::transaction::{Effect, Kind};
use bagholder_core::{Broker, Currency, Dec, Money, SourceName};

use super::old::{OldActivity, OldDatabase, OldSecurity};
use super::row_leg;
use crate::mapping::{Draft, InstrumentDraft, MapContext, Mapped, Mapping, NameDraft, OptionDraft};

pub const IMPORT_SOURCE: &str = "bagholder-import";

/// The scheme a record's Wealthsimple activity id is kept under, so Wealthsimple's
/// own row can find the imported one it replaces.
pub const WEALTHSIMPLE_RECORD: &str = "broker-record:wealthsimple";

/// The zone Wealthsimple files its rows under (`docs/architecture.md` §7).
const WEALTHSIMPLE_ZONE: &str = "America/Edmonton";

pub fn import_source() -> SourceName {
    SourceName::named(IMPORT_SOURCE)
}

/// An imported record's payload: the row, and the security rows it names, as the
/// earlier database held them.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedRow {
    pub row: OldActivity,
    pub security: Option<OldSecurity>,
    pub underlying: Option<OldSecurity>,
}

impl ImportedRow {
    pub fn of(row: &OldActivity, old: &OldDatabase) -> ImportedRow {
        let security = row.security_id.as_deref().filter(|s| !s.is_empty()).and_then(|id| old.securities.get(id)).cloned();
        let underlying = security.as_ref().and_then(|s| s.underlying_id.as_deref()).filter(|s| !s.is_empty()).and_then(|id| old.securities.get(id)).cloned();
        ImportedRow { row: row.clone(), security, underlying }
    }
}

pub struct ImportMapping;

/// Where a row came from, as far as what it states.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Synced,
    BookedFill,
    File,
}

fn origin(source: &str) -> Option<Origin> {
    match source {
        "wealthsimple" => Some(Origin::Synced),
        "bagholder-fill" | "bagholder" => Some(Origin::BookedFill),
        "csv" | "statement" | "canonical" | "legacy" | "manual" => Some(Origin::File),
        _ => None,
    }
}

/// How the table books a row: Wealthsimple's activity type and sub-type as
/// Bagholder's kind, read the same way wherever a row states them (an earlier
/// database's row, a file Wealthsimple exported).
pub struct Rule {
    pub kind: Kind,
    pub effect: Option<Effect>,
    /// What the instrument is, for a row that concerns one.
    pub instrument: Option<InstrumentKind>,
    pub quantity: Qty,
    pub cash: Cash,
    /// Whether the earlier app signed this row's quantity and cash itself (share
    /// and option orders, expiries): a sign against the kind is then a problem to
    /// see. Coin and event-contract rows kept Wealthsimple's unsigned quantity and
    /// its `amountSign`, which is not the cash's direction, so their signs are
    /// set by the kind alone.
    pub signed: bool,
}

#[derive(Clone, Copy)]
pub enum Qty {
    None,
    /// The row's quantity made positive.
    In,
    /// The row's quantity made negative.
    Out,
    /// The row's quantity with its own sign.
    AsSigned,
}

#[derive(Clone, Copy)]
pub enum Cash {
    None,
    /// The row's cash with its own sign.
    AsSigned,
    /// Paid: made negative.
    Paid,
    /// Received: made positive.
    Received,
}

pub fn rule(ty: &str, sub: &str, cash: Option<Dec>, direction: &str) -> Option<Rule> {
    use InstrumentKind as I;
    let signed = matches!(ty, "Trade" | "OPTIONS_BUY" | "OPTIONS_SELL" | "EXPIR");
    let r = |kind, effect, instrument, quantity, cash| Some(Rule { kind, effect, instrument, quantity, cash, signed });
    // a transfer's way: its cash's sign, or the row's direction where it moved
    // none; with neither, the way is not stated and the row is not placed
    let transfer = || match cash {
        Some(c) if c.is_positive() => Some(Kind::TransferIn),
        Some(c) if c.is_negative() => Some(Kind::TransferOut),
        _ => match direction {
            "CREDIT" => Some(Kind::TransferIn),
            "DEBIT" => Some(Kind::TransferOut),
            _ => None,
        },
    };
    match (ty, sub) {
        ("Trade", "BUY") => r(Kind::Buy, None, Some(I::Security), Qty::In, Cash::Paid),
        ("Trade", "SELL") => r(Kind::Sell, None, Some(I::Security), Qty::Out, Cash::Received),
        // Whether an option fill opened or closed is not stated: the earlier app
        // relabelled every option buy and sale "to open" after each pull, and
        // every multi-leg order by its cash's sign (`store/src/relabel.rs`)
        ("OPTIONS_BUY", "BUYTOOPEN" | "BUYTOCLOSE") => r(Kind::Buy, None, Some(I::OptionContract), Qty::In, Cash::Paid),
        ("OPTIONS_SELL", "SELLTOOPEN" | "SELLTOCLOSE") => r(Kind::Sell, None, Some(I::OptionContract), Qty::Out, Cash::Received),
        ("EXPIR", "BUY") => r(Kind::OptionExpiry, None, Some(I::OptionContract), Qty::In, Cash::None),
        ("EXPIR", "SELL") => r(Kind::OptionExpiry, None, Some(I::OptionContract), Qty::Out, Cash::None),
        ("ASSIGN", _) => r(Kind::OptionAssignment, None, Some(I::OptionContract), Qty::AsSigned, Cash::AsSigned),
        ("EXERCISE", _) => r(Kind::OptionExercise, None, Some(I::OptionContract), Qty::AsSigned, Cash::AsSigned),
        ("STKDIS", "BUY" | "SELL") => r(Kind::CorporateEvent, None, Some(I::Security), Qty::AsSigned, Cash::None),
        ("Dividend", _) => r(Kind::Dividend, None, Some(I::Security), Qty::None, Cash::AsSigned),
        ("Interest", _) => r(Kind::Interest, None, None, Qty::None, Cash::AsSigned),
        ("INTEREST_CHARGE", _) => r(Kind::InterestCharge, None, None, Qty::None, Cash::AsSigned),
        ("WITHHOLDING_TAX", _) => r(Kind::WithholdingTax, None, None, Qty::None, Cash::AsSigned),
        ("FxExchange", _) => r(Kind::CurrencyConversion, None, None, Qty::None, Cash::AsSigned),
        ("Deposit", _) => r(Kind::Deposit, None, None, Qty::None, Cash::AsSigned),
        ("GROUP_CONTRIBUTION", "EMPLOYER_CONTRIBUTION") => r(Kind::EmployerDeposit, None, None, Qty::None, Cash::AsSigned),
        ("GROUP_CONTRIBUTION", "EMPLOYEE_CONTRIBUTION") => r(Kind::Deposit, None, None, Qty::None, Cash::AsSigned),
        ("RESP_GRANT", _) => r(Kind::GovernmentDeposit, None, None, Qty::None, Cash::AsSigned),
        ("Withdrawal", _) => r(Kind::Withdrawal, None, None, Qty::None, Cash::AsSigned),
        ("Transfer", _) | ("ASSET_MOVEMENT", _) => r(transfer()?, None, None, Qty::None, Cash::AsSigned),
        ("CRYPTO_BUY", _) => r(Kind::Buy, None, Some(I::Crypto), Qty::In, Cash::Paid),
        ("CRYPTO_SELL", _) => r(Kind::Sell, None, Some(I::Crypto), Qty::Out, Cash::Received),
        ("CRYPTO_TRANSFER", "TRANSFER_IN") => r(Kind::TransferIn, None, Some(I::Crypto), Qty::In, Cash::None),
        ("CRYPTO_TRANSFER", "TRANSFER_OUT") => r(Kind::TransferOut, None, Some(I::Crypto), Qty::Out, Cash::None),
        ("CRYPTO_STAKING_REWARD", _) => r(Kind::StakingReward, None, Some(I::Crypto), Qty::In, Cash::None),
        ("CRYPTO_STAKING_ACTION", _) => r(Kind::StakingMove, None, Some(I::Crypto), Qty::None, Cash::None),
        ("PREDICTIONS_BUY", _) => r(Kind::Buy, None, Some(I::EventContract), Qty::In, Cash::Paid),
        ("PREDICTIONS_RESOLUTION", _) => r(Kind::Resolution, None, Some(I::EventContract), Qty::Out, Cash::AsSigned),
        ("CREDIT_CARD", "PURCHASE") => r(Kind::CardPurchase, None, None, Qty::None, Cash::AsSigned),
        ("CREDIT_CARD", "REFUND") => r(Kind::CardRefund, None, None, Qty::None, Cash::AsSigned),
        ("CREDIT_CARD", "PAYMENT") => r(Kind::TransferIn, None, None, Qty::None, Cash::AsSigned),
        ("CREDIT_CARD_PAYMENT", _) => r(Kind::TransferOut, None, None, Qty::None, Cash::AsSigned),
        ("REIMBURSEMENT", _) => r(Kind::Cashback, None, None, Qty::None, Cash::AsSigned),
        _ => None,
    }
}

fn opt(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

fn number(what: &str, v: &Option<String>) -> Result<Option<Dec>, Problem> {
    match opt(v) {
        None => Ok(None),
        Some(s) => Dec::parse(s).map(Some).map_err(|e| Problem::new("unreadable-number", format!("the row's {what} {s:?} is not a number: {e}"))),
    }
}

impl Mapping for ImportMapping {
    fn source(&self) -> SourceName {
        import_source()
    }

    /// 3: a dividend keeps the units the row states it was paid on.
    fn version(&self) -> u32 {
        3
    }

    fn map(&self, ctx: &MapContext, payload: &str) -> Mapped {
        let p: ImportedRow = match serde_json::from_str(payload) {
            Ok(p) => p,
            Err(e) => return Mapped::unreadable(format!("an imported row the import cannot read: {e}")),
        };
        match map_row(ctx, &p) {
            Ok(m) => m,
            Err(problem) => Mapped { legs: vec![], problems: vec![problem], ..Mapped::default() },
        }
    }
}

fn map_row(ctx: &MapContext, p: &ImportedRow) -> Result<Mapped, Problem> {
    let row = &p.row;
    let mut problems = Vec::new();
    let unreadable = |why: String| Problem::new("unreadable", why);

    let source = opt(&row.source).unwrap_or("");
    let origin = origin(source).ok_or_else(|| Problem::new("unclassified", format!("a row from a source the import does not know: {source:?}")))?;
    let account_id = opt(&row.account_id).ok_or_else(|| unreadable("a row with no account".into()))?;
    let account = AccountRef::new(Broker::named("wealthsimple"), account_id);
    let currency_text = opt(&row.currency).ok_or_else(|| unreadable("a row with no currency".into()))?;
    let currency = Currency::parse(currency_text).map_err(|e| unreadable(e.to_string()))?;

    // when it happened, and the day it is filed under
    let stated_day = || -> Result<jiff::civil::Date, Problem> {
        let d = opt(&row.transaction_date).ok_or_else(|| unreadable("a row with no date".into()))?;
        d.parse().map_err(|e| unreadable(format!("the row's date {d:?} is not a day: {e}")))
    };
    let (occurred_at, trade_date) = match origin {
        Origin::File => (None, stated_day()?),
        Origin::Synced | Origin::BookedFill => match opt(&row.occurred_at) {
            Some(s) if s.len() > 10 => {
                let at: jiff::Timestamp = s.parse().map_err(|e| unreadable(format!("the row's time {s:?} is not an instant: {e}")))?;
                let day = ctx.zones.day(at, WEALTHSIMPLE_ZONE).map_err(unreadable)?;
                (Some(at), day)
            }
            // only a day was kept: the day as it is
            _ => (None, stated_day()?),
        },
    };

    let ty = opt(&row.activity_type).unwrap_or("");
    // Wealthsimple's notice of a distribution to come, listing the units held on
    // the record date with no cash, which the earlier app stored as a share
    // movement: it is a dividend of no cash (a notice, not a payment), and it
    // moves no position
    let ty = if ty == "STKDIS" && opt(&row.raw_type) == Some("DIVIDEND") { "Dividend" } else { ty };
    let sub = opt(&row.activity_sub_type).unwrap_or("");
    let quantity = number("quantity", &row.quantity)?;
    // the row's own units, whatever the kind makes of them
    let row_quantity = quantity.map(|q| q.abs());
    let cash_stated = number("cash", &row.net_cash_amount)?;
    let unit_price = number("price", &row.unit_price)?;
    let commission = number("commission", &row.commission)?;
    let direction = opt(&row.direction).unwrap_or("");

    let Some(rule) = rule(ty, sub, cash_stated, direction) else {
        let raw = opt(&row.raw_type).unwrap_or("");
        problems.push(Problem::new("unclassified", format!(
            "a row the import does not place: {ty} {sub} {raw}{}",
            if ty == "INSTITUTIONAL_TRANSFER_INTENT" { " (the earlier app dropped the status that says whether the transfer happened; Wealthsimple's own row will say)" } else { "" }
        )));
        let leg = Draft {
            leg: row_leg(),
            account,
            occurred_at,
            trade_date,
            settle_date: None,
            kind: Kind::Unclassified,
            effect: None,
            instrument: None,
            quantity: None,
            price: None,
            cash: None,
            fee: None,
            fx_rate: None,
            paid_on: None,
        };
        return Ok(Mapped { legs: vec![leg], problems, ..Mapped::default() });
    };

    // a sign the earlier app set against the kind: booked as the kind says, and shown
    if rule.signed {
        let against = |v: Option<Dec>, want_positive: bool| v.is_some_and(|v| !v.is_zero() && v.is_positive() != want_positive);
        let q_against = match rule.quantity {
            Qty::In => against(quantity, true),
            Qty::Out => against(quantity, false),
            _ => false,
        };
        let c_against = match rule.cash {
            Cash::Paid => against(cash_stated, false),
            Cash::Received => against(cash_stated, true),
            _ => false,
        };
        if q_against || c_against {
            problems.push(Problem::new("sign-against-kind", format!("a {} whose {} the earlier app signed the other way; booked as a {}", rule.kind, if q_against { "quantity" } else { "cash" }, rule.kind)));
        }
    }
    let quantity = match (rule.quantity, quantity) {
        (Qty::None, _) => None,
        // a quantity of zero on a row that moves a position is a quantity not stated
        (_, None) => {
            problems.push(Problem::new("quantity-not-stated", format!("a {} with no quantity (the earlier app kept none for it)", rule.kind)));
            None
        }
        (_, Some(q)) if q.is_zero() && !matches!(rule.kind, Kind::CorporateEvent) => {
            problems.push(Problem::new("quantity-not-stated", format!("a {} with no quantity (the earlier app kept none for it)", rule.kind)));
            None
        }
        (Qty::In, Some(q)) => Some(q.abs()),
        (Qty::Out, Some(q)) => Some(q.abs().neg()),
        (Qty::AsSigned, Some(q)) => Some(q),
    };
    // a corporate event of no quantity (the marker a split left) moves no position
    let quantity = quantity.filter(|q| !q.is_zero());

    let cash = match (origin, rule.cash, cash_stated) {
        // a booked fill's cash was the earlier app's arithmetic, not a statement
        (Origin::BookedFill, _, _) => None,
        (_, Cash::None, _) | (_, _, None) => None,
        (_, Cash::AsSigned, Some(c)) => Some(c),
        (_, Cash::Paid, Some(c)) => Some(c.abs().neg()),
        (_, Cash::Received, Some(c)) => Some(c.abs()),
    };
    // the earlier app stored no cash as zero: on a trade or an option's assignment
    // or exercise a zero is nothing stated; where the cash is the whole of the row
    // (a dividend, a deposit, an event contract's payout) it is the amount
    let zero_is_unstated = matches!(rule.cash, Cash::Paid | Cash::Received) || matches!(rule.kind, Kind::OptionAssignment | Kind::OptionExercise);
    let cash = cash.filter(|c| !(c.is_zero() && zero_is_unstated)).map(|c| Money::new(c, currency));

    let price = match origin {
        Origin::Synced => None,
        Origin::BookedFill | Origin::File => unit_price.filter(|p| !p.is_zero() && quantity.is_some()).map(|p| Money::new(p, currency)),
    };
    let fee = commission.filter(|c| !c.is_zero()).map(|c| Money::new(c.abs(), currency));

    let instrument = match rule.instrument {
        None => None,
        Some(kind) => {
            let before = problems.len();
            let found = instrument(ctx, p, kind, currency, trade_date, &mut problems);
            if found.is_none() && problems.len() == before {
                problems.push(Problem::new("instrument-not-named", format!("a {} that names no instrument", rule.kind)));
            }
            found
        }
    };
    let quantity = if instrument.is_some() { quantity } else { None };
    // One row of a multi-leg order: the earlier app kept one row for the whole
    // order and none of its legs, so what it moved is not stated. Its cash moved.
    let multi_leg = opt(&row.raw_type).is_some_and(|raw| raw.to_uppercase().contains("MULTILEG"));
    let quantity = if multi_leg {
        problems.retain(|p| p.code != "quantity-not-stated");
        problems.push(Problem::new("leg-unstated", "one row for a multi-leg order: the earlier app kept none of its legs, so the contracts and quantities it moved are not stated"));
        None
    } else {
        quantity
    };
    let price = if quantity.is_some() { price } else { None };

    let paid_on = if rule.kind == Kind::Dividend { row_quantity.filter(|q| q.is_positive()) } else { None };
    let leg = Draft {
        leg: row_leg(),
        account,
        occurred_at,
        trade_date,
        settle_date: None,
        kind: rule.kind,
        effect: rule.effect,
        instrument,
        quantity,
        price,
        cash,
        fee,
        fx_rate: None,
        paid_on,
    };
    Ok(Mapped { legs: vec![leg], problems, ..Mapped::default() })
}

/// The instrument a row names: by Wealthsimple's security id where it has one,
/// priced in the currency the security's row states; else by its symbol and the
/// currency the row states, within the connection (a file's row).
fn instrument(ctx: &MapContext, p: &ImportedRow, kind: InstrumentKind, row_currency: Currency, seen: jiff::civil::Date, problems: &mut Vec<Problem>) -> Option<InstrumentDraft> {
    let row = &p.row;
    let ws = Broker::named("wealthsimple");
    let symbol = opt(&row.symbol).map(str::to_string);
    let security = p.security.as_ref();
    let (refs, currency) = match opt(&row.security_id) {
        Some(id) => {
            let Some(c) = security.and_then(|s| opt(&s.currency)) else {
                problems.push(Problem::new("instrument-currency-unknown", format!("the earlier database does not say what currency security {id} is priced in")));
                return None;
            };
            match Currency::parse(c) {
                Ok(c) => (vec![Reference::new(RefScheme::BrokerSecurity(ws.clone()), id)], c),
                Err(e) => {
                    problems.push(Problem::new("unreadable", format!("the security's currency: {e}")));
                    return None;
                }
            }
        }
        None => (vec![Reference::connection_symbol(ctx.connection?, symbol.as_deref()?, row_currency)], row_currency),
    };
    let name = symbol.clone().map(|symbol| NameDraft {
        symbol: symbol.clone(),
        venue_mic: security.and_then(|s| opt(&s.primary_mic)).map(str::to_string),
        venue_name: security.and_then(|s| opt(&s.primary_exchange)).map(str::to_string),
        name: security.and_then(|s| opt(&s.name)).map(str::to_string).or_else(|| opt(&row.name).filter(|n| *n != symbol).map(str::to_string)),
        seen,
    });
    let option = match (kind, symbol.as_deref()) {
        (InstrumentKind::OptionContract, Some(symbol)) => option_draft(p, symbol, currency, seen, problems),
        (InstrumentKind::OptionContract, None) => {
            problems.push(Problem::new("option-terms-unreadable", "an option row with no name to read its terms from"));
            None
        }
        _ => None,
    };
    Some(InstrumentDraft { refs, kind, currency, name, option })
}

/// An option's terms, from the name the earlier app printed from Wealthsimple's
/// contract fields (`QNC 19FEB27 3.00 CALL`) and its underlying's security row.
/// The multiplier is left for a source that states it.
fn option_draft(p: &ImportedRow, symbol: &str, currency: Currency, seen: jiff::civil::Date, problems: &mut Vec<Problem>) -> Option<OptionDraft> {
    let terms = match parse_option_name(symbol) {
        Ok(t) => t,
        Err(why) => {
            problems.push(Problem::new("option-terms-unreadable", format!("the option's name {symbol:?} {why}")));
            return None;
        }
    };
    let underlying_id = p.security.as_ref().and_then(|s| opt(&s.underlying_id));
    let Some(underlying_id) = underlying_id else {
        problems.push(Problem::new("underlying-unknown", format!("the earlier database does not say which security option {symbol} is on")));
        return None;
    };
    let u = p.underlying.as_ref();
    let u_currency = match u.and_then(|s| opt(&s.currency)) {
        None => currency,
        Some(c) => match Currency::parse(c) {
            Ok(c) => c,
            Err(e) => {
                problems.push(Problem::new("unreadable", format!("the underlying security's currency: {e}")));
                return None;
            }
        },
    };
    let u_symbol = u.and_then(|s| opt(&s.symbol)).unwrap_or(&terms.0);
    let underlying = InstrumentDraft {
        refs: vec![Reference::new(RefScheme::BrokerSecurity(Broker::named("wealthsimple")), underlying_id)],
        kind: InstrumentKind::Security,
        currency: u_currency,
        name: Some(NameDraft {
            symbol: u_symbol.to_string(),
            venue_mic: u.and_then(|s| opt(&s.primary_mic)).map(str::to_string),
            venue_name: u.and_then(|s| opt(&s.primary_exchange)).map(str::to_string),
            name: u.and_then(|s| opt(&s.name)).map(str::to_string),
            seen,
        }),
        option: None,
    };
    let (_, expiry, strike, right) = terms;
    Some(OptionDraft { underlying: Box::new(underlying), expiry, strike, right, multiplier: None })
}

const MONTHS: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

/// `UNDERLYING DDMONYY STRIKE CALL|PUT`, as the earlier app printed it, read strictly.
pub fn parse_option_name(s: &str) -> Result<(String, jiff::civil::Date, Dec, OptionRight), String> {
    let parts: Vec<&str> = s.split(' ').collect();
    let [under, date, strike, right] = parts.as_slice() else { return Err("is not four words".into()) };
    if under.is_empty() {
        return Err("names no underlying".into());
    }
    let d = date.as_bytes();
    if d.len() != 7 {
        return Err(format!("has a date {date:?} not written DDMONYY"));
    }
    let day: i8 = date[0..2].parse().map_err(|_| format!("has a date {date:?} not written DDMONYY"))?;
    let month = MONTHS.iter().position(|m| *m == &date[2..5]).ok_or_else(|| format!("has a date {date:?} not written DDMONYY"))? as i8 + 1;
    let year: i16 = date[5..7].parse::<i16>().map_err(|_| format!("has a date {date:?} not written DDMONYY"))? + 2000;
    let expiry = jiff::civil::Date::new(year, month, day).map_err(|e| format!("has a date {date:?} that is no day: {e}"))?;
    let strike = Dec::parse(strike).map_err(|e| format!("has a strike {strike:?} that is not a number: {e}"))?;
    let right = match *right {
        "CALL" => OptionRight::Call,
        "PUT" => OptionRight::Put,
        other => return Err(format!("has a right {other:?} that is neither CALL nor PUT")),
    };
    Ok((under.to_string(), expiry, strike, right))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_names_read_strictly() {
        let (u, e, k, r) = parse_option_name("QNC 19FEB27 3.00 CALL").unwrap();
        assert_eq!((u.as_str(), e, k, r), ("QNC", jiff::civil::date(2027, 2, 19), Dec::parse("3").unwrap(), OptionRight::Call));
        assert_eq!(parse_option_name("SPY 17JUL25 624.00 PUT").unwrap().2, Dec::parse("624").unwrap());
        for bad in ["QNC", "QNC 19FEB27 3.00", "QNC 19FEB27 3.00 CALL X", "QNC 31FEB27 3.00 CALL", "QNC 19FEX27 3.00 CALL", "QNC 19FEB27 3,00 CALL", "QNC 19FEB27 3.00 call", " 19FEB27 3.00 CALL"] {
            assert!(parse_option_name(bad).is_err(), "{bad}");
        }
    }
}
