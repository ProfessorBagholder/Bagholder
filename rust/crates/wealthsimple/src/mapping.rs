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
//! - a move of holdings between accounts, or in from another institution,
//!   states a value, not what moved: each security moved is one whose units fell
//!   in one account by what they rose in the other.
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

    fn version(&self) -> u32 {
        1
    }

    fn map(&self, ctx: &MapContext, payload: &str) -> Mapped {
        let v = match json::parse(payload) {
            Ok(v) => v,
            Err(e) => return Mapped::unreadable(format!("a Wealthsimple record that is not JSON: {e}")),
        };
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
        "INTERNAL_TRANSFER" | "ASSET_MOVEMENT" | "LEGACY_INTERNAL_TRANSFER" | "INSTITUTIONAL_TRANSFER_INTENT" => transfer(&root, &row, &base, &mut out)?,
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
        Some(OptionDraft {
            underlying: Box::new(underlying),
            expiry: o.day("expiryDate")?,
            strike: o.dec_text("strikePrice")?,
            right,
            multiplier: Some(o.dec("multiplier").or_else(|_| o.dec_text("multiplier"))?),
        })
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
        // nothing filled: an expired or cancelled order moved nothing
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
    let (before_day, after_day) = (first.yesterday().map_err(bad)?, last.tomorrow().map_err(bad)?);
    let (Some(before), Some(after)) = (units(root, account, &before_day.to_string())?, units(root, account, &after_day.to_string())?) else { return Ok(None) };
    let mut days = Vec::new();
    let mut d = first;
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
    let detail = root.obj("conversion")?;
    // an internal transfer's detail states the amount paid and its currency;
    // a funding intent's states only the side received
    if detail.field("fxAdjustedAmount").is_ok() && detail.field("amount").is_ok() {
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
        // the row states no amount: the move's detail does, in the source
        // account's currency, and on the destination's side in the same
        // currency where its rate is one
        if let Ok(detail) = root.obj("conversion") {
            let amount = detail.dec_text("amount")?;
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
