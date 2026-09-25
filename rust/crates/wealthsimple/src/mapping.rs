//! Wealthsimple's records into Bagholder's transactions
//! (`docs/plans/stage-3b-wealthsimple.md`, "The mapping").
//!
//! One table from Wealthsimple's `type` and `subType` to a transaction, each
//! entry reading only what the row states. What the row does not state is read
//! from the replies kept beside it in the record (`crate::record`), never worked
//! out. A row whose status is not executed moves nothing and raises nothing.
//!
//! What the rows were found to state (the owner's history, 2026-09-24):
//! - `amountSign` is not the cash's direction on a trade: a buy and a sale are
//!   both `positive`, and a multi-leg row's sign is the opposite of its legs'
//!   net cash. A trade's cash takes its direction from what the row is (a buy
//!   pays, a sale receives); a row that is only cash (a deposit, interest) from
//!   its sign where its kind allows both ways.
//! - a multi-leg row states its order; the order states each leg.
//! - an option fill does not state whether it opened or closed.
//! - a corporate action states the units given up and received, not the
//!   received security: that is the security the account's positions show rising
//!   by exactly those units across the event, net of the book's own moves.
//! - a move of holdings between accounts states a value, not what moved: each
//!   security moved is one whose units fell in one account by what they rose in
//!   the other. A move that states no amount at all (a "full in kind" move of an
//!   account sold to cash first) states none in its detail either; its cash moves
//!   days after its row, on the one day within the week after it that the two
//!   accounts' net deposits fall and rise by the same amount.
//! - a transfer in from another institution states the value asked for, not what
//!   arrived: the cash arrives days later, less the other institution's fee or
//!   more by its interest, and its detail leaves the value that arrived empty
//!   (every one in the owner's history, 2026-09-24). What arrived is what the
//!   account's positions show rising from the day before the row to the day the
//!   detail says it completed, net of the book's own moves.
//! - a withdrawal from a registered account states its gross amount; the tax
//!   withheld from it is a row of its own sharing its id, and the other account
//!   receives the rest.
//! - a currency conversion states the side received; the side paid is stated
//!   by its detail where Wealthsimple keeps one (an internal transfer's), and is
//!   otherwise a problem, not a guess.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_book::mapping::{AdjustmentDraft, AdjustmentLegDraft, Draft, InstrumentDraft, MapContext, Mapped, Mapping, NameDraft, OptionDraft};
use bagholder_core::account::AccountRef;
use bagholder_core::instrument::{InstrumentKind, OptionRight, RefScheme, Reference};
use bagholder_core::json::{self, Value};
use bagholder_core::record::Problem;
use bagholder_core::transaction::{Effect, Kind};
use bagholder_core::{Broker, Currency, Dec, Leg, Money, RecordId, Rounding, SourceName, TransactionId};
use bagholder_sources::reply::{Mismatch, Node, Read};

/// Wealthsimple files a row under Alberta's day (`docs/architecture.md` §7).
pub const ZONE: &str = "America/Edmonton";

pub fn source() -> SourceName {
    SourceName::named("wealthsimple")
}

pub fn broker() -> Broker {
    Broker::named("wealthsimple")
}

pub struct WealthsimpleMapping;

impl Mapping for WealthsimpleMapping {
    fn source(&self) -> SourceName {
        source()
    }

    /// 2: a distribution keeps the units Wealthsimple states it was paid on.
    fn version(&self) -> u32 {
        2
    }

    fn map(&self, ctx: &MapContext, payload: &str) -> Mapped {
        let v = match json::parse(payload) {
            Ok(v) => v,
            Err(e) => return Mapped::unreadable(format!("a Wealthsimple record that is not JSON: {e}")),
        };
        // a row the adapter could not read: kept, with why
        if let Ok(why) = Node::root(&v).text("unread") {
            return Mapped::unreadable(format!("a Wealthsimple row the adapter could not read: {why}"));
        }
        match map_record(ctx, &v) {
            Ok(m) => m,
            Err(Failed::Reply(m)) => Mapped::unreadable(format!("Wealthsimple's reply does not have the shape read: {m}")),
            Err(Failed::Problem(p)) => Mapped { legs: vec![], problems: vec![p], adjustments: vec![] },
        }
    }
}

enum Failed {
    Reply(Mismatch),
    Problem(Problem),
}

impl From<Mismatch> for Failed {
    fn from(m: Mismatch) -> Failed {
        Failed::Reply(m)
    }
}

impl From<Problem> for Failed {
    fn from(p: Problem) -> Failed {
        Failed::Problem(p)
    }
}

fn leg(s: &str) -> Leg {
    Leg::parse(s).expect("a leg name written in the code")
}

/// Legs a record's transactions are named by.
fn row_leg() -> Leg {
    leg("row")
}

/// The n-th leg of a multi-leg order, or the n-th security of a move.
fn nth_leg(prefix: &str, n: usize) -> Leg {
    const LETTERS: &str = "abcdefghijklmnopqrstuvwxyz";
    // legs are named with letters only: `leg-a`, `leg-b`, … `leg-az`
    let mut name = String::new();
    let mut i = n;
    loop {
        name.insert(0, LETTERS.as_bytes()[i % 26] as char);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    leg(&format!("{prefix}-{name}"))
}

/// How the row's cash is signed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cash {
    /// None moves (or none is stated as cash: an in-kind move's value).
    None,
    /// Paid, whatever the sign says.
    Paid,
    /// Received.
    Received,
    /// As `amountSign` states it: a kind that goes both ways.
    Signed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Units {
    None,
    In,
    Out,
}

struct Rule {
    kind: Kind,
    units: Units,
    cash: Cash,
    /// Whether the row names a security the transaction is on.
    instrument: bool,
}

fn rule(ty: &str, sub: Option<&str>) -> Option<Rule> {
    let r = |kind, units, cash, instrument| Some(Rule { kind, units, cash, instrument });
    use Cash as C;
    use Units as U;
    match (ty, sub) {
        ("DIY_BUY" | "MANAGED_BUY" | "CRYPTO_BUY" | "OPTIONS_BUY" | "PREDICTIONS_BUY", _) => r(Kind::Buy, U::In, C::Paid, true),
        ("DIY_SELL" | "MANAGED_SELL" | "CRYPTO_SELL" | "OPTIONS_SELL", _) => r(Kind::Sell, U::Out, C::Received, true),
        // a long's contracts expire out of the account; a short's back into it
        ("OPTIONS_EXPIRY", _) => r(Kind::OptionExpiry, U::Out, C::None, true),
        ("OPTIONS_SHORT_EXPIRY", _) => r(Kind::OptionExpiry, U::In, C::None, true),
        ("PREDICTIONS_RESOLUTION", _) => r(Kind::Resolution, U::Out, C::Received, true),
        ("DIVIDEND", _) => r(Kind::Dividend, U::None, C::Received, true),
        ("NON_RESIDENT_TAX", _) => r(Kind::WithholdingTax, U::None, C::Paid, true),
        ("WITHHOLDING_TAX", _) => r(Kind::WithholdingTax, U::None, C::Paid, false),
        ("INTEREST", _) => r(Kind::Interest, U::None, C::Signed, false),
        ("INTEREST_CHARGE", _) => r(Kind::InterestCharge, U::None, C::Paid, false),
        ("FEE" | "WRITE_OFF", _) => r(Kind::Fee, U::None, C::Paid, false),
        // Wealthsimple paying back its own charges
        ("REIMBURSEMENT", Some("ETF_REBATE" | "ACCOUNTING_REIMBURSEMENT")) => r(Kind::Fee, U::None, C::Received, false),
        ("REIMBURSEMENT", Some("CASHBACK")) | ("PROMOTION", _) => r(Kind::Cashback, U::None, C::Received, false),
        ("DEPOSIT", _) | ("P2P_PAYMENT", Some("SEND_RECEIVED")) | ("GROUP_CONTRIBUTION", Some("EMPLOYEE_CONTRIBUTION")) => r(Kind::Deposit, U::None, C::Received, false),
        ("GROUP_CONTRIBUTION", Some("EMPLOYER_CONTRIBUTION")) => r(Kind::EmployerDeposit, U::None, C::Received, false),
        ("RESP_GRANT", _) => r(Kind::GovernmentDeposit, U::None, C::Signed, false),
        ("WITHDRAWAL", _) | ("P2P_PAYMENT", Some("SEND")) => r(Kind::Withdrawal, U::None, C::Paid, false),
        ("CRYPTO_TRANSFER", Some("TRANSFER_IN")) => r(Kind::TransferIn, U::In, C::None, true),
        ("CRYPTO_TRANSFER", Some("TRANSFER_OUT")) => r(Kind::TransferOut, U::Out, C::None, true),
        ("CRYPTO_STAKING_REWARD", _) => r(Kind::StakingReward, U::In, C::None, true),
        ("CRYPTO_STAKING_ACTION", _) => r(Kind::StakingMove, U::None, C::None, true),
        ("CREDIT_CARD", Some("PURCHASE")) => r(Kind::CardPurchase, U::None, C::Paid, false),
        ("CREDIT_CARD", Some("REFUND")) => r(Kind::CardRefund, U::None, C::Received, false),
        ("CREDIT_CARD", Some("PAYMENT")) => r(Kind::TransferIn, U::None, C::Received, false),
        ("CREDIT_CARD_PAYMENT", _) => r(Kind::TransferOut, U::None, C::Paid, false),
        _ => None,
    }
}

/// The row's own fields, read strictly.
struct Row<'a> {
    node: Node<'a>,
    ty: &'a str,
    sub: Option<&'a str>,
    status: &'a str,
    account: &'a str,
    occurred_at: jiff::Timestamp,
    amount: Option<Dec>,
    sign: Option<&'a str>,
    currency: Option<Currency>,
    quantity: Option<Dec>,
    security: Option<&'a str>,
}

fn read_row<'a>(n: Node<'a>) -> Read<Row<'a>> {
    let at_text = n.text("occurredAt")?;
    let occurred_at: jiff::Timestamp = at_text.parse().map_err(|e| n.field("occurredAt").map(|f| f.mismatch(format!("not an instant: {e}"))).unwrap_or_else(|m| m))?;
    let currency = match n.opt_text("currency")? {
        None => None,
        Some(c) => Some(Currency::parse(c).map_err(|e| n.field("currency").map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))?),
    };
    let sign = n.opt_text("amountSign")?;
    if let Some(s) = sign {
        if s != "positive" && s != "negative" {
            return Err(n.field("amountSign")?.mismatch(format!("expected positive or negative, found {s:?}")));
        }
    }
    Ok(Row {
        ty: n.text("type")?,
        sub: n.opt_text("subType")?,
        status: n.text("unifiedStatus")?,
        account: n.text("accountId")?,
        occurred_at,
        amount: n.opt_dec_text("amount")?,
        sign,
        currency,
        quantity: n.opt_dec_text("assetQuantity")?,
        security: n.opt_text("securityId")?,
        node: n,
    })
}

fn map_record(ctx: &MapContext, v: &Value) -> Result<Mapped, Failed> {
    let root = Node::root(v);
    let row = read_row(root.obj("activity")?)?;
    let day = ctx.zones.day(row.occurred_at, ZONE).map_err(|e| Problem::new("unreadable", e))?;
    let mut out = Mapped::default();
    // only what Wealthsimple states as executed moves anything; a pending row
    // is read again until it is final (brief 07 §2)
    let executed = row.status == "COMPLETED";
    let base = Base { account: AccountRef::new(broker(), row.account), occurred_at: row.occurred_at, day };
    match row.ty {
        "OPTIONS_MULTILEG" => multi_leg(&root, &row, &base, &mut out)?,
        _ if !executed => {}
        "OPTIONS_ASSIGN" => assignment(&root, &row, &base, &mut out)?,
        "CORPORATE_ACTION" => corporate_action(&root, &row, &base, &ctx.record, &mut out)?,
        "FUNDS_CONVERSION" => conversion(&root, &row, &base, &mut out)?,
        "INTERNAL_TRANSFER" | "ASSET_MOVEMENT" | "LEGACY_INTERNAL_TRANSFER" => transfer(&root, &row, &base, &mut out)?,
        "INSTITUTIONAL_TRANSFER_INTENT" => institutional(&root, &row, &base, &mut out)?,
        ty => match rule(ty, row.sub) {
            Some(r) => single(&root, &row, &base, &r, &mut out)?,
            None => {
                out.problems.push(Problem::new("unclassified", format!("a Wealthsimple row this mapping does not place: {} {}", row.ty, row.sub.unwrap_or("-"))));
                out.legs.push(base.draft(row_leg(), Kind::Unclassified));
            }
        },
    }
    Ok(out)
}

/// What every transaction of a record shares.
struct Base {
    account: AccountRef,
    occurred_at: jiff::Timestamp,
    day: jiff::civil::Date,
}

impl Base {
    fn draft(&self, leg: Leg, kind: Kind) -> Draft {
        Draft {
            leg,
            account: self.account.clone(),
            occurred_at: Some(self.occurred_at),
            trade_date: self.day,
            settle_date: None,
            kind,
            effect: None,
            instrument: None,
            quantity: None,
            price: None,
            cash: None,
            fee: None,
            fx_rate: None,
            paid_on: None,
        }
    }
}

/// The row's amount as cash, signed as the rule says.
fn row_cash(row: &Row, cash: Cash, kind: Kind) -> Result<Option<Money>, Failed> {
    if cash == Cash::None {
        return Ok(None);
    }
    let (Some(amount), Some(currency)) = (row.amount, row.currency) else {
        return Err(Problem::new("cash-not-stated", format!("a {kind} whose amount or currency Wealthsimple does not state")).into());
    };
    let signed = match cash {
        Cash::Paid => amount.abs().neg(),
        Cash::Received => amount.abs(),
        Cash::Signed => match row.sign {
            Some("negative") => amount.abs().neg(),
            Some(_) => amount.abs(),
            None => return Err(Problem::new("sign-not-stated", format!("a {kind} of {amount} {currency} whose direction Wealthsimple does not state")).into()),
        },
        Cash::None => unreachable!("handled above"),
    };
    Ok(Some(Money::new(signed, currency)))
}

fn single(root: &Node, row: &Row, base: &Base, r: &Rule, out: &mut Mapped) -> Result<(), Failed> {
    let mut d = base.draft(row_leg(), r.kind);
    d.cash = row_cash(row, r.cash, r.kind)?;
    if r.instrument {
        let Some(id) = row.security else {
            return Err(Problem::new("instrument-not-named", format!("a {} that names no security", r.kind)).into());
        };
        d.instrument = Some(instrument(root, id, base.day)?);
    }
    // a distribution's units, where Wealthsimple states them: what it was paid on,
    // never a change to the holding
    if r.kind == Kind::Dividend {
        d.paid_on = row.quantity.filter(|q| q.is_positive());
    }
    match (r.units, row.quantity) {
        (Units::None, _) => {}
        (_, None) => out.problems.push(Problem::new("quantity-not-stated", format!("a {} whose quantity Wealthsimple does not state", r.kind))),
        (Units::In, Some(q)) => d.quantity = Some(q.abs()),
        (Units::Out, Some(q)) => d.quantity = Some(q.abs().neg()),
    }
    out.legs.push(d);
    Ok(())
}

/// The security a record names, from its security record kept beside the row.
fn instrument(root: &Node, id: &str, seen: jiff::civil::Date) -> Result<InstrumentDraft, Failed> {
    let Some(s) = root.obj("securities").ok().and_then(|all| all.obj(id).ok()) else {
        return Err(Problem::new("security-not-read", format!("security {id}, which the record names, was not read with it")).into());
    };
    let kind = match s.text("securityType")? {
        "EQUITY" | "EXCHANGE_TRADED_FUND" => InstrumentKind::Security,
        "OPTION" => InstrumentKind::OptionContract,
        "CRYPTOCURRENCY" => InstrumentKind::Crypto,
        "PREDICTION_MARKET" => InstrumentKind::EventContract,
        other => return Err(Problem::new("unclassified", format!("security {id} is of a type this mapping does not place: {other}")).into()),
    };
    let currency = Currency::parse(s.text("currency")?).map_err(|e| s.field("currency").map(|f| f.mismatch(e.to_string())).unwrap_or_else(|m| m))?;
    let mut refs = vec![Reference::new(RefScheme::BrokerSecurity(broker()), id)];
    let stock = s.field("stock")?;
    let name = if matches!(stock.value(), Value::Null) {
        None
    } else {
        Some(NameDraft {
            symbol: stock.text("symbol")?.to_string(),
            venue_mic: stock.opt_text("primaryMic")?.map(str::to_string),
            venue_name: stock.opt_text("primaryExchange")?.map(str::to_string),
            name: stock.opt_text("name")?.map(str::to_string),
            seen,
        })
    };
    let option = if kind == InstrumentKind::OptionContract {
        let o = s.obj("optionDetails")?;
        let osi = o.text("osiSymbol")?;
        refs.push(Reference::new(RefScheme::Occ, osi.split_whitespace().collect::<Vec<_>>().join(" ")));
        let right = match o.text("optionType")? {
            "CALL" | "call" => OptionRight::Call,
            "PUT" | "put" => OptionRight::Put,
            other => return Err(o.field("optionType")?.mismatch(format!("expected CALL or PUT, found {other:?}")).into()),
        };
        let underlying_id = o.obj("underlyingSecurity")?.text("id")?;
        let underlying = instrument(root, underlying_id, seen)?;
        let multiplier = o.dec("multiplier").or_else(|_| o.dec_text("multiplier"))?;
        // the units one contract is on: fills `option_terms.multiplier`, which
        // every figure of the contract is scaled by
        if !multiplier.is_positive() {
            return Err(o.field("multiplier")?.mismatch(format!("a contract on {} units", multiplier.to_text())).into());
        }
        Some(OptionDraft { underlying: Box::new(underlying), expiry: o.day("expiryDate")?, strike: o.dec_text("strikePrice")?, right, multiplier: Some(multiplier) })
    } else {
        None
    };
    Ok(InstrumentDraft { refs, kind, currency, name, option })
}

/// A multi-leg order: each leg as its order states it. The row's amount is its
/// legs' net cash, which it must equal (the row's sign is not the cash's).
fn multi_leg(root: &Node, row: &Row, base: &Base, out: &mut Mapped) -> Result<(), Failed> {
    let order = root.obj("order")?;
    let legs = order.list("legs")?;
    let mut net = Dec::ZERO;
    let mut drafts = Vec::new();
    for (i, l) in legs.iter().enumerate() {
        let Some(filled) = l.opt_dec_text("filledQuantity")? else { continue };
        if filled.is_zero() {
            continue;
        }
        let side = l.text("side")?;
        let (kind, q_sign) = match side {
            "BUY" => (Kind::Buy, Dec::ONE),
            "SELL" => (Kind::Sell, Dec::ONE.neg()),
            other => return Err(l.field("side")?.mismatch(format!("expected BUY or SELL, found {other:?}")).into()),
        };
        let effect = match l.text("openClose")? {
            "OPEN" => Effect::Open,
            "CLOSE" => Effect::Close,
            other => return Err(l.field("openClose")?.mismatch(format!("expected OPEN or CLOSE, found {other:?}")).into()),
        };
        let price_node = l.obj("averageFillPrice")?;
        // the order service writes its currency codes in lower case (`usd`)
        let currency = Currency::parse(&price_node.text("currency")?.to_uppercase()).map_err(|e| Problem::new("unreadable", e.to_string()))?;
        let price = Money::new(price_node.dec_text("amount")?, currency);
        let value = l.dec_text("filledNetValue")?;
        let cash = if kind == Kind::Buy { value.neg() } else { value };
        net = net.checked_add(cash).map_err(|e| Problem::new("unreadable", e.to_string()))?;
        let mut d = base.draft(nth_leg("leg", i), kind);
        d.effect = Some(effect);
        d.instrument = Some(instrument(root, l.text("securityId")?, base.day)?);
        d.quantity = Some(filled.abs().checked_mul(q_sign).map_err(|e| Problem::new("unreadable", e.to_string()))?);
        d.price = Some(price);
        d.cash = Some(Money::new(cash, currency));
        drafts.push(d);
    }
    if drafts.is_empty() {
        // nothing filled: an expired or cancelled order moved nothing; a row
        // stating it executed for an amount did move it, and its order does not say how
        if row.status == "COMPLETED" {
            if let Some(amount) = row.amount.filter(|a| !a.is_zero()) {
                out.problems.push(Problem::new("legs-disagree", format!("the multi-leg row's amount {amount} is not its legs' net cash: its order states no leg filled")));
            }
        }
        return Ok(());
    }
    // the fee the order states is the account's, on top of the legs' net value
    let fee = order.opt_dec_text("totalFee")?.filter(|f| !f.is_zero());
    let expected = match fee {
        Some(f) => net.checked_sub(f).map_err(|e| Problem::new("unreadable", e.to_string()))?,
        None => net,
    };
    if let Some(amount) = row.amount {
        if amount.abs() != expected.abs() {
            out.problems.push(Problem::new("legs-disagree", format!("the multi-leg row's amount {amount} is not its legs' net cash {expected}")));
            return Ok(());
        }
    }
    if let (Some(f), Some(first)) = (fee, drafts.first_mut()) {
        let currency = first.cash.map(|c| c.currency).unwrap_or(Currency::USD);
        first.fee = Some(Money::new(f.abs(), currency));
    }
    out.legs.extend(drafts);
    Ok(())
}

/// An assignment: a short's contracts come back in, and the shares go at the
/// strike, received for a call written and paid for a put.
fn assignment(root: &Node, row: &Row, base: &Base, out: &mut Mapped) -> Result<(), Failed> {
    let Some(id) = row.security else {
        return Err(Problem::new("instrument-not-named", "an assignment that names no contract").into());
    };
    let inst = instrument(root, id, base.day)?;
    let right = inst.option.as_ref().map(|o| o.right);
    let mut d = base.draft(row_leg(), Kind::OptionAssignment);
    d.cash = match right {
        Some(OptionRight::Call) => row_cash(row, Cash::Received, Kind::OptionAssignment)?,
        Some(OptionRight::Put) => row_cash(row, Cash::Paid, Kind::OptionAssignment)?,
        None => return Err(Problem::new("option-terms-unreadable", format!("an assignment of {id}, which is not an option")).into()),
    };
    d.instrument = Some(inst);
    match row.quantity {
        Some(q) => d.quantity = Some(q.abs()),
        None => out.problems.push(Problem::new("quantity-not-stated", "an assignment whose contracts Wealthsimple does not state")),
    }
    out.legs.push(d);
    Ok(())
}

/// Each security's units in one account on one day, from the positions kept.
fn units(root: &Node, account: &str, day: &str) -> Result<Option<BTreeMap<String, Dec>>, Failed> {
    let Ok(list) = root.list("positions") else { return Ok(None) };
    for p in list {
        if p.text("account")? == account && p.text("day")? == day {
            let mut m = BTreeMap::new();
            for node in p.list("nodes")? {
                let id = node.obj("security")?.text("id")?.to_string();
                let q = node.dec_text("quantity")?;
                let e = m.entry(id).or_insert(Dec::ZERO);
                *e = e.checked_add(q).map_err(|e| Problem::new("unreadable", e.to_string()))?;
            }
            return Ok(Some(m));
        }
    }
    Ok(None)
}

/// What the book's own transactions moved of each security in one account over
/// the given days.
fn booked(root: &Node, account: &str, days: &[String]) -> Result<BTreeMap<String, Dec>, Failed> {
    let mut m = BTreeMap::new();
    let Ok(list) = root.list("book") else { return Ok(m) };
    for b in list {
        if b.text("account")? == account && days.iter().any(|d| d == b.text("day").unwrap_or("")) {
            let e = m.entry(b.text("security")?.to_string()).or_insert(Dec::ZERO);
            *e = e.checked_add(b.dec_text("quantity")?).map_err(|e| Problem::new("unreadable", e.to_string()))?;
        }
    }
    Ok(m)
}

/// Each security's change in one account from the day before `first` to the day
/// after `last` (`sec-c-<currency>` for its cash), net of what the book's own
/// transactions moved on the days between. None where the positions for either
/// end are not kept.
fn changed(root: &Node, account: &str, first: jiff::civil::Date, last: jiff::civil::Date) -> Result<Option<BTreeMap<String, Dec>>, Failed> {
    let bad = |e: jiff::Error| Failed::from(Problem::new("unreadable", e.to_string()));
    changed_between(root, account, first.yesterday().map_err(bad)?, last.tomorrow().map_err(bad)?)
}

/// Each security's change in one account from the positions of `before_day` to
/// those of `after_day`, net of what the book's own transactions moved on the
/// days after the first up to the last. None where either end is not kept.
fn changed_between(root: &Node, account: &str, before_day: jiff::civil::Date, after_day: jiff::civil::Date) -> Result<Option<BTreeMap<String, Dec>>, Failed> {
    let bad = |e: jiff::Error| Failed::from(Problem::new("unreadable", e.to_string()));
    let (Some(before), Some(after)) = (units(root, account, &before_day.to_string())?, units(root, account, &after_day.to_string())?) else { return Ok(None) };
    let mut days = Vec::new();
    let mut d = before_day.tomorrow().map_err(bad)?;
    while d <= after_day {
        days.push(d.to_string());
        d = d.tomorrow().map_err(bad)?;
    }
    let own = booked(root, account, &days)?;
    let ids: BTreeSet<&String> = before.keys().chain(after.keys()).chain(own.keys()).collect();
    let mut m = BTreeMap::new();
    for id in ids {
        let d = after.get(id).copied().unwrap_or(Dec::ZERO).checked_sub(before.get(id).copied().unwrap_or(Dec::ZERO)).and_then(|d| d.checked_sub(own.get(id).copied().unwrap_or(Dec::ZERO))).map_err(|e| Problem::new("unreadable", e.to_string()))?;
        if !d.is_zero() {
            m.insert(id.clone(), d);
        }
    }
    Ok(Some(m))
}

/// A corporate action: the units given up and received, as its entitlements
/// state them; the security received is the one the account's positions show
/// rising by exactly those units. Where the kind of event says the whole cost
/// continues (a consolidation, a split, a change of code or name), the
/// adjustment says so; otherwise what it did to cost waits for the person or a
/// source.
fn corporate_action(root: &Node, row: &Row, base: &Base, record: &RecordId, out: &mut Mapped) -> Result<(), Failed> {
    let Some(old) = row.security else {
        return Err(Problem::new("instrument-not-named", "a corporate action that names no security").into());
    };
    let items = root.obj("entitlements")?.list("nodes")?;
    let mut given = Dec::ZERO;
    let mut received: Vec<Dec> = Vec::new();
    let mut cash: Option<Money> = None;
    for e in &items {
        let kind = e.text("entitlementType")?;
        let asset = e.text("assetType")?;
        match (kind, asset) {
            ("SUBMIT", "EQUITY") => given = given.checked_add(e.dec_text("quantity")?).map_err(|x| Problem::new("unreadable", x.to_string()))?,
            ("RECEIVE", "EQUITY") => received.push(e.dec_text("quantity")?),
            ("RECEIVE", "CASH") => {
                let c = Currency::parse(e.text("currency")?).map_err(|x| Problem::new("unreadable", x.to_string()))?;
                cash = Some(Money::new(e.dec_text("quantity")?, c));
            }
            ("HOLD", _) => {}
            (k, a) => return Err(Problem::new("unclassified", format!("a corporate action entitlement this mapping does not place: {k} {a}")).into()),
        }
    }
    if given.is_zero() && received.is_empty() && cash.is_none() {
        // an event stated as done whose entitlements state nothing given up or
        // received: what it moved is not stated
        return Err(Problem::new("event-unstated", format!("a corporate action on {old} whose entitlements state no units given up or received")).into());
    }
    let event = base.draft(row_leg(), Kind::CorporateEvent);
    let mut legs = Vec::new();
    // the units given up
    if !given.is_zero() {
        let mut d = event.clone();
        d.instrument = Some(instrument(root, old, base.day)?);
        d.quantity = Some(given.abs().neg());
        legs.push(d);
    }
    // the units received, each on the security the positions show them in
    let moved = changed(root, row.account, base.day, base.day)?;
    let mut to_ids = Vec::new();
    for (i, q) in received.iter().enumerate() {
        let found: Vec<&String> = moved.as_ref().map(|m| m.iter().filter(|(id, d)| **d == *q && *id != old).map(|(id, _)| id).collect()).unwrap_or_default();
        match found.as_slice() {
            [id] => {
                let mut d = event.clone();
                d.leg = nth_leg("receive", i);
                d.instrument = Some(instrument(root, id, base.day)?);
                d.quantity = Some(q.abs());
                to_ids.push((*id).clone());
                legs.push(d);
            }
            _ => out.problems.push(Problem::new("received-security-unstated", format!("{q} units received in a corporate action, which no single security in the account's positions shows arriving"))),
        }
    }
    if let Some(c) = cash {
        let mut d = event.clone();
        d.leg = leg("cash");
        d.cash = Some(c);
        legs.push(d);
    }
    let continues = matches!(row.sub, Some("CONSOLIDATION" | "SPLIT" | "STOCK_SPLIT" | "REVERSE_SPLIT" | "INTERNATIONAL_CODE_CHANGE" | "CODE_CHANGE" | "NAME_CHANGE" | "SYMBOL_CHANGE" | "CUSIP_CHANGE"));
    if continues && to_ids.len() == 1 && received.len() == 1 && !given.is_zero() {
        let q = received[0];
        // the ratio where the units state it exactly; else the units alone stand
        let ratio = q.div_rounded(given, 28, Rounding::HalfEven).ok().filter(|r| r.checked_mul(given).ok() == Some(q));
        let applies_to = TransactionId::new(record.clone(), row_leg());
        out.adjustments.push(AdjustmentDraft {
            leg: row_leg(),
            applies_to,
            legs: vec![AdjustmentLegDraft {
                from: Some(vec![Reference::new(RefScheme::BrokerSecurity(broker()), old)]),
                to: Some(vec![Reference::new(RefScheme::BrokerSecurity(broker()), to_ids[0].clone())]),
                units_per_unit: ratio,
                cost_share: Some(Dec::ONE),
                ..AdjustmentLegDraft::default()
            }],
        });
    }
    out.legs.extend(legs);
    Ok(())
}

/// A currency conversion: the side received as the row states it, the side paid
/// as the conversion's detail states it.
fn conversion(root: &Node, row: &Row, base: &Base, out: &mut Mapped) -> Result<(), Failed> {
    let received = row_cash(row, Cash::Received, Kind::CurrencyConversion)?;
    let mut d = base.draft(leg("received"), Kind::CurrencyConversion);
    d.cash = received;
    out.legs.push(d);
    let Ok(detail) = root.obj("conversion") else {
        out.problems.push(Problem::new("conversion-side-unstated", "Wealthsimple did not answer this conversion's detail, which states the amount paid"));
        return Ok(());
    };
    // an internal transfer's detail states the amount paid and its currency;
    // a funding intent's states only the side received, which must be the row's
    if let Ok(kind) = detail.field("fundableType") {
        if kind.as_text()? != "CurrencyConversion" {
            return Err(Problem::new("conversion-disagrees", format!("the conversion's detail is a funding intent of another kind: {}", kind.as_text()?)).into());
        }
        let f = detail.obj("fundableDetails")?;
        let amount = f.dec_text("fxAdjustedAmount")?;
        let currency = Currency::parse(f.text("targetCurrency")?).map_err(|e| f.field("targetCurrency").map(|t| t.mismatch(e.to_string())).unwrap_or_else(|m| m))?;
        if Some(amount) != row.amount || Some(currency) != row.currency {
            out.problems.push(Problem::new("conversion-disagrees", format!("the conversion's detail states {amount} {currency} received, not its row's")));
        }
        out.problems.push(Problem::new("conversion-side-unstated", "Wealthsimple states the side received of this conversion, not the amount paid"));
    } else if detail.field("fxAdjustedAmount").is_ok() && detail.field("amount").is_ok() {
        let paid = detail.dec_text("amount")?;
        let currency = Currency::parse(detail.text("currency")?).map_err(|e| Problem::new("unreadable", e.to_string()))?;
        if Some(detail.dec_text("fxAdjustedAmount")?) != row.amount {
            out.problems.push(Problem::new("conversion-disagrees", "the conversion's detail states another amount received than its row"));
        }
        let mut p = base.draft(leg("paid"), Kind::CurrencyConversion);
        p.cash = Some(Money::new(paid.abs().neg(), currency));
        p.fx_rate = detail.opt_dec_text("fxRate")?;
        out.legs.push(p);
    } else {
        out.problems.push(Problem::new("conversion-side-unstated", "Wealthsimple states the side received of this conversion, not the amount paid"));
    }
    Ok(())
}

/// A move between the person's accounts or in from another institution. Cash
/// moves as the row states it. Holdings move as the positions show them.
///
/// Moves of holdings touching the same accounts on neighbouring days are read
/// together, as one group (the record keeps the others as its siblings):
/// Wealthsimple marks a move of cash "in kind" when it follows a sale in the
/// source, and an institutional transfer's intent moves nothing itself (the
/// money arrives as its own row). Over the days the group spans, each account's
/// positions change, net of the book's own transactions, is:
/// - no holdings leaving one account for another: the row moved cash, its own
///   stated amount (an institutional transfer's intent included: no other row
///   brings its money in), or where it states none, the cash that fell in one
///   account of a lone pair and rose by as much in the other. What else the
///   accounts' cash did (a sale's proceeds, a conversion's side not stated) is
///   the broker check's to show. That the positions show nothing moved is not
///   read as the row moving nothing: a transfer can settle after its row's day;
/// - holdings whose fall in one account is their rise in another (from another
///   institution, their rise): the holdings, where the group is one move, since
///   which row moved which is not stated otherwise.
/// Anything else is not stated, and named.
fn transfer(root: &Node, row: &Row, base: &Base, out: &mut Mapped) -> Result<(), Failed> {
    let incoming = matches!(row.sub, Some("DESTINATION" | "TRANSFER_IN"));
    let kind = if incoming { Kind::TransferIn } else { Kind::TransferOut };
    let transfer_type = row.node.opt_text("transferType")?.unwrap_or("");
    let in_cash = row.ty == "LEGACY_INTERNAL_TRANSFER" || transfer_type.contains("in_cash");
    let cash_leg = |out: &mut Mapped| -> Result<(), Failed> {
        let mut d = base.draft(row_leg(), kind);
        d.cash = row_cash(row, if incoming { Cash::Received } else { Cash::Paid }, kind)?;
        // a withdrawal's gross amount holds the tax withheld from it, which its
        // own row books: what left for the other account is the rest
        if let (Some(cash), Ok(list)) = (d.cash.as_mut(), root.list("withheld")) {
            for w in list {
                let w = read_row(w)?;
                if w.status != "COMPLETED" {
                    continue;
                }
                let (Some(tax), Some(c)) = (w.amount, w.currency) else {
                    return Err(Problem::new("cash-not-stated", "tax withheld from a withdrawal whose amount Wealthsimple does not state").into());
                };
                if c != cash.currency {
                    return Err(Problem::new("withheld-currency-differs", format!("tax withheld in {c} from a withdrawal in {}", cash.currency)).into());
                }
                cash.amount = cash.amount.checked_add(tax.abs()).map_err(|e| Problem::new("unreadable", e.to_string()))?;
            }
        }
        out.legs.push(d);
        Ok(())
    };
    if in_cash {
        return cash_leg(out);
    }
    let unstated = |out: &mut Mapped, why: String| {
        out.problems.push(Problem::new("moved-holdings-unstated", why));
        out.legs.push(base.draft(row_leg(), kind));
    };
    // the group: this row and its siblings
    let mut group: Vec<Row> = vec![];
    if let Ok(list) = root.list("siblings") {
        for n in list {
            group.push(read_row(n)?);
        }
    }
    let me = row.node.text("canonicalId")?;
    if !group.iter().any(|g| g.node.text("canonicalId").ok() == Some(me)) {
        group.push(read_row(row.node.clone())?);
    }
    let zones = bagholder_book::zones::Zones::default();
    let days: Vec<jiff::civil::Date> = group.iter().filter_map(|g| zones.day(g.occurred_at, ZONE).ok()).collect();
    let (first, last) = (days.iter().min().copied().unwrap_or(base.day), days.iter().max().copied().unwrap_or(base.day));
    // each account's change across the group's days, and what the group's rows
    // say moved in and out of it in cash
    let mut accounts: BTreeSet<&str> = BTreeSet::new();
    for g in &group {
        accounts.insert(g.account);
        if let Some(o) = g.node.opt_text("opposingAccountId")? {
            accounts.insert(o);
        }
    }
    let mut change: BTreeMap<&str, BTreeMap<String, Dec>> = BTreeMap::new();
    for a in &accounts {
        match changed(root, a, first, last)? {
            Some(c) => {
                change.insert(a, c);
            }
            None => {
                unstated(out, format!("a {kind} whose holdings Wealthsimple states only as a value, with no positions kept to show what moved"));
                return Ok(());
            }
        }
    }
    // holdings that left one account of the group and arrived in another: a
    // change the other side does not mirror moved nothing between them
    let mut matched: BTreeMap<(&str, String), Dec> = BTreeMap::new();
    for (a, c) in &change {
        for (id, q) in c {
            if id.starts_with("sec-c-") {
                continue;
            }
            let mirrored = change.iter().any(|(b, d)| b != a && d.get(id).is_some_and(|x| x.checked_add(*q).ok() == Some(Dec::ZERO)));
            let from_outside = group.len() == 1 && row.node.opt_text("opposingAccountId")?.is_none() && q.is_positive();
            if mirrored || from_outside {
                matched.insert((a, id.clone()), *q);
            }
        }
    }
    if matched.is_empty() {
        // no holdings left one account for another: the move was cash. Its
        // amount is the row's where it states one; else the cash that fell in
        // one account of a lone pair and rose by as much in the other
        if row.amount.is_some() {
            return cash_leg(out);
        }
        // the row states no amount: its detail states it where Wealthsimple
        // kept one, in the source account's currency, and on the destination's
        // side in the same currency where its rate is one; else the accounts'
        // net deposits show it moving (below)
        if let Some((detail, amount)) = root.obj("conversion").ok().and_then(|d| d.opt_dec_text("amount").ok().flatten().map(|a| (d, a))) {
            let currency = Currency::parse(&detail.text("currency")?.to_uppercase()).map_err(|e| Problem::new("unreadable", e.to_string()))?;
            let same = detail.opt_dec_text("fxRate")?.is_none_or(|r| r == Dec::ONE) && detail.opt_dec_text("fxAdjustedAmount")?.is_none_or(|a| a == amount);
            if incoming && !same {
                unstated(out, "a move between accounts in two currencies whose detail states the amount sent, not the currency received".to_string());
                return Ok(());
            }
            let mut d = base.draft(row_leg(), kind);
            d.cash = Some(Money::new(if incoming { amount.abs() } else { amount.abs().neg() }, currency));
            out.legs.push(d);
            return Ok(());
        }
        if let Some((day, m)) = moved_by_deposits(root, row, base.day, incoming)? {
            // on the day it moved, whose instant is not stated
            let mut d = base.draft(row_leg(), kind);
            d.occurred_at = None;
            d.trade_date = day;
            d.cash = Some(m);
            out.legs.push(d);
            return Ok(());
        }
        let other = row.node.opt_text("opposingAccountId")?;
        let mirrored: Vec<(String, Dec)> = match (other, change.get(row.account)) {
            (Some(o), Some(mine)) if group.len() == 2 => mine
                .iter()
                .filter(|(id, q)| id.starts_with("sec-c-") && change.get(o).is_some_and(|t| t.get(*id).is_some_and(|x| x.checked_add(**q).ok() == Some(Dec::ZERO))))
                .map(|(id, q)| (id.clone(), *q))
                .collect(),
            _ => vec![],
        };
        if let [(id, q)] = mirrored.as_slice() {
            let cur = Currency::parse(&id["sec-c-".len()..].to_uppercase()).map_err(|e| Problem::new("unreadable", e.to_string()))?;
            let mut d = base.draft(row_leg(), kind);
            d.cash = Some(Money::new(*q, cur));
            out.legs.push(d);
            return Ok(());
        }
        unstated(out, format!("a {kind} that states no amount, and whose cash the positions do not show moving between the two accounts"));
        return Ok(());
    }
    if group.len() > 2 {
        unstated(out, format!("{} moves of holdings on neighbouring days: which moved which is not stated", group.len()));
        return Ok(());
    }
    let this_way = |q: &Dec| if incoming { q.is_positive() } else { q.is_negative() };
    let mut n = 0;
    for ((a, id), q) in &matched {
        if *a != row.account || !this_way(q) {
            continue;
        }
        let mut d = base.draft(nth_leg("move", n), kind);
        n += 1;
        d.instrument = Some(instrument(root, id, base.day)?);
        d.quantity = Some(*q);
        out.legs.push(d);
    }
    // cash that moved with the holdings, mirrored in the other account
    if let Some(c) = change.get(row.account) {
        for (id, q) in c.iter().filter(|(id, q)| id.starts_with("sec-c-") && this_way(q)) {
            let mirrored = change.iter().any(|(b, d)| *b != row.account && d.get(id).is_some_and(|x| x.checked_add(*q).ok() == Some(Dec::ZERO)));
            if mirrored {
                let code = &id["sec-c-".len()..];
                let cur = Currency::parse(&code.to_uppercase()).map_err(|e| Problem::new("unreadable", e.to_string()))?;
                let mut d = base.draft(nth_leg("move", n), kind);
                n += 1;
                d.cash = Some(Money::new(*q, cur));
                out.legs.push(d);
            }
        }
    }
    if n == 0 {
        unstated(out, format!("a {kind} the positions do not show moving anything its way"));
    }
    Ok(())
}

/// The days after a move's own within which its cash moves: the sales that
/// fund a move of an account sold to cash settle in one or two business days
/// (T+2 in Canada until 2024-05-27, T+1 since), and the cash moves on a
/// business day after, which weekends and a holiday keep within a week.
pub const SETTLES_WITHIN: i64 = 7;

/// A move between two accounts that states no amount: the cash its two
/// accounts' net deposits show moving, and the day: the one day, from the
/// row's own to `SETTLES_WITHIN` days after, on which one account's fell by
/// exactly what the other's rose (their other deposits and withdrawals change
/// one account alone), apart from the days the other moves between them that
/// state their amount account for. None where the days are not kept, or no day or more
/// than one shows it: which is this move's is then not stated.
fn moved_by_deposits(root: &Node, row: &Row, day: jiff::civil::Date, incoming: bool) -> Result<Option<(jiff::civil::Date, Money)>, Failed> {
    let Some(other) = row.node.opt_text("opposingAccountId")? else { return Ok(None) };
    let Ok(list) = root.list("deposits") else { return Ok(None) };
    let mut days: BTreeMap<&str, Vec<bagholder_broker::DayValue>> = BTreeMap::new();
    for d in list {
        let nodes: Vec<Value> = d.list("nodes")?.into_iter().map(|n| n.value().clone()).collect();
        days.insert(d.text("account")?, crate::read::history(&nodes)?);
    }
    let (Some(mine), Some(theirs)) = (days.get(row.account), days.get(other)) else { return Ok(None) };
    let last = day.checked_add(jiff::Span::new().days(SETTLES_WITHIN)).map_err(|e| Problem::new("unreadable", e.to_string()))?;
    let mut found = mirrored_days(mine, theirs, last);
    // a day another move between the two states its amount for is that move's
    if let Ok(list) = root.list("stated_moves") {
        for m in list {
            let amount = m.dec_text("amount")?.abs();
            if let Some(i) = found.iter().position(|(_, c)| c.amount.abs() == amount) {
                found.remove(i);
            }
        }
    }
    let [(on, moved)] = found.as_slice() else { return Ok(None) };
    let this_way = if incoming { moved.amount.is_positive() } else { moved.amount.is_negative() };
    Ok(this_way.then_some((*on, *moved)))
}

/// Each day up to `last` on which two accounts' net deposits changed by
/// exactly opposite amounts, and the first account's change: each list is one
/// account's days in order from the day before a move.
pub fn mirrored_days(a: &[bagholder_broker::DayValue], b: &[bagholder_broker::DayValue], last: jiff::civil::Date) -> Vec<(jiff::civil::Date, Money)> {
    let changes = |days: &[bagholder_broker::DayValue]| -> BTreeMap<jiff::civil::Date, Money> {
        days.windows(2)
            .filter(|w| w[1].day <= last)
            .filter_map(|w| {
                let d = w[1].net_deposits.amount.checked_sub(w[0].net_deposits.amount).ok()?;
                (!d.is_zero() && w[1].net_deposits.currency == w[0].net_deposits.currency).then(|| (w[1].day, Money::new(d, w[1].net_deposits.currency)))
            })
            .collect()
    };
    let (ca, cb) = (changes(a), changes(b));
    ca.into_iter().filter(|(day, m)| cb.get(day).is_some_and(|o| o.currency == m.currency && o.amount.checked_add(m.amount).ok() == Some(Dec::ZERO))).collect()
}

/// The instant a transfer from another institution completed: its detail's
/// `completed` event, where its state is completed.
pub fn completed_at(detail: &Node) -> Read<Option<jiff::Timestamp>> {
    if detail.text("state")? != "completed" {
        return Ok(None);
    }
    for h in detail.list("stateHistories")? {
        if h.text("event")? == "completed" {
            let t = h.text("transitionedAt")?;
            return t.parse().map(Some).map_err(|e| h.field("transitionedAt").map(|f| f.mismatch(format!("not an instant: {e}"))).unwrap_or_else(|m| m));
        }
    }
    Ok(None)
}

/// The day a transfer from another institution completed, as Wealthsimple
/// files a row.
pub fn completed_on(detail: &Node) -> Read<Option<jiff::civil::Date>> {
    Ok(completed_at(detail)?.and_then(|at| bagholder_book::zones::Zones::default().day(at, ZONE).ok()))
}

/// An instrument as Wealthsimple describes it: `securities` holds its security's
/// record (and an option's underlying's) by id.
pub fn draft_of(securities: &Value, id: &str, seen: jiff::civil::Date) -> Result<InstrumentDraft, String> {
    let v = Value::Object(BTreeMap::from([("securities".to_string(), securities.clone())]));
    instrument(&Node::root(&v), id, seen).map_err(|f| match f {
        Failed::Reply(m) => m.to_string(),
        Failed::Problem(p) => p.detail,
    })
}

/// A transfer in from another institution: what arrived, on the day it
/// completed. The detail's value that arrived per currency where it states one;
/// else what the account's positions show rising from the day before the row to
/// the day the detail says it completed, net of the book's own moves over those
/// days: its cash, and for a transfer in kind or mixed, its holdings. Another
/// row of the account read against positions over those days leaves which moved
/// what unstated, as does anything that fell.
fn institutional(root: &Node, row: &Row, base: &Base, out: &mut Mapped) -> Result<(), Failed> {
    let kind = Kind::TransferIn;
    let unstated = |out: &mut Mapped, why: &str| {
        out.problems.push(Problem::new("transfer-arrival-unstated", format!("a transfer from another institution {why}")));
        out.legs.push(base.draft(row_leg(), kind));
    };
    let Ok(detail) = root.obj("transfer") else {
        unstated(out, "whose detail was not read");
        return Ok(());
    };
    let state = detail.text("state")?;
    if state != "completed" {
        return Err(Problem::new("transfer-disagrees", format!("a transfer from another institution its row states completed and its detail {state}")).into());
    }
    let Some(at) = completed_at(&detail)? else {
        unstated(out, "whose detail states no day it completed");
        return Ok(());
    };
    let done = bagholder_book::zones::Zones::default().day(at, ZONE).map_err(|e| Problem::new("unreadable", e))?;
    let arrived = |out: &mut Mapped, n: usize| {
        let mut d = base.draft(nth_leg("arrived", n), kind);
        d.occurred_at = Some(at);
        d.trade_date = done;
        out.legs.push(d);
    };
    // the value that arrived, where the detail states it
    let mut n = 0;
    for (field, code) in [("actualValueLegCad", "CAD"), ("actualValueLegUsd", "USD")] {
        if let Some(a) = detail.opt_dec_text(field)? {
            arrived(out, n);
            out.legs.last_mut().expect("just pushed").cash = Some(Money::new(a, Currency::parse(code).map_err(|e| Problem::new("unreadable", e.to_string()))?));
            n += 1;
        }
    }
    if n > 0 {
        return Ok(());
    }
    let holdings = match detail.text("transferType")? {
        "IN_CASH" | "PARTIAL_IN_CASH" => false,
        "IN_KIND" | "PARTIAL_IN_KIND" | "PARTIAL_MIXED" | "FULL_MIXED" => true,
        other => return Err(detail.field("transferType")?.mismatch(format!("a transfer type this mapping does not place: {other:?}")).into()),
    };
    if root.list("siblings").is_ok_and(|l| !l.is_empty()) {
        unstated(out, "under way while the account's other moves read against positions were: which moved what is not stated");
        return Ok(());
    }
    let before = base.day.yesterday().map_err(|e| Problem::new("unreadable", e.to_string()))?;
    let Some(change) = changed_between(root, row.account, before, done)? else {
        unstated(out, "whose account's positions before it and on the day it completed are not kept");
        return Ok(());
    };
    let moved: Vec<(&String, &Dec)> = change.iter().filter(|(id, _)| holdings || id.starts_with("sec-c-")).collect();
    if moved.is_empty() || moved.iter().any(|(_, q)| !q.is_positive()) {
        unstated(out, "whose account's positions do not show only what arrived");
        return Ok(());
    }
    for (i, (id, q)) in moved.into_iter().enumerate() {
        arrived(out, i);
        let d = out.legs.last_mut().expect("just pushed");
        match id.strip_prefix("sec-c-") {
            Some(code) => d.cash = Some(Money::new(*q, Currency::parse(&code.to_uppercase()).map_err(|e| Problem::new("unreadable", e.to_string()))?)),
            None => {
                d.instrument = Some(instrument(root, id, done)?);
                d.quantity = Some(*q);
            }
        }
    }
    Ok(())
}
