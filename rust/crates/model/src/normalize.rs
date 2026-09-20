//! Activity normalization: the working copy of each stored row, with crypto and
//! option events expressed as trade fills. The stored rows are never rewritten.

use crate::activity::{Activity, Category, Direction, Flag, Kind, RawActivity, Side};
use crate::symbols::{is_option_symbol, option_right, underlying_symbol};
use crate::value::{compact, norm_account_name, EPS};

/// Whether a type names a crypto event.
fn is_crypto(raw_type_c: &str, type_c: &str) -> bool {
    raw_type_c.starts_with("CRYPTO") || type_c.starts_with("CRYPTO")
}

/// A row that says it opens a position.
pub fn is_intentional_open(a: &Activity) -> bool {
    let (at, sub) = (a.type_c(), a.sub_type_c());
    at.contains("TOOPEN") || sub.contains("TOOPEN") || at == "STO" || at == "BTO" || sub == "STO" || sub == "BTO"
}

/// A row that can only reduce a position: an explicit close, or the
/// expiry/assignment/exercise the broker posts for one.
pub fn is_close_only(a: &Activity) -> bool {
    let (at, sub) = (a.type_c(), a.sub_type_c());
    [&at, &sub].iter().any(|f| f.contains("TOCLOSE") || **f == "BTC" || **f == "STC")
        || [&at, &sub].iter().any(|f| f.contains("EXPIR") || f.contains("ASSIGN") || f.contains("EXERCISE"))
}

/// Which way a fill opens, or `None` when it can only close. A bare share sale
/// is never read as a short unless the row says it opened one, so a sale of
/// something bought before the history starts does not invent a short position.
pub fn opening_direction(a: &Activity, side: Side) -> Option<Direction> {
    match side {
        Side::Buy => (!is_close_only(a)).then_some(Direction::Long),
        Side::Sell if is_option_symbol(&a.symbol) => (!is_close_only(a)).then_some(Direction::Short),
        Side::Sell => is_intentional_open(a).then_some(Direction::Short),
    }
}

/// The working copy of a stored row.
pub fn normalize(raw: &RawActivity) -> Activity {
    let mut a = Activity {
        id: raw.id.clone(),
        occurred_at: raw.occurred_at.clone(),
        transaction_date: raw.transaction_date.clone(),
        account_id: raw.account_id.clone(),
        book_id: raw.book_id.clone(),
        fifo_id: raw.fifo_id.clone(),
        account_name: norm_account_name(&raw.account_type),
        activity_type: raw.activity_type.clone(),
        activity_sub_type: raw.activity_sub_type.clone(),
        description: raw.description.clone(),
        cash_direction: raw.direction.clone(),
        symbol: raw.symbol.clone(),
        name: raw.name.clone(),
        currency: raw.currency.clone(),
        quantity: raw.quantity,
        unit_price: raw.unit_price,
        commission: raw.commission,
        net_cash_amount: raw.net_cash_amount,
        category: Category::parse(&raw.category),
        raw_type: raw.raw_type.clone(),
        aft_type: raw.aft_type.clone(),
        security_id: raw.security_id.clone(),
        kind: Kind::Shares,
        flags: vec![],
    };
    let rt = compact(&raw.raw_type);
    let at = compact(&raw.activity_type);
    let cash = raw.net_cash_amount;
    let qty = raw.quantity.abs();
    let is = |name: &str| rt == name || at == name;

    // ---- crypto: the rows carry their direction in the type, not the sign
    let as_trade = |a: &mut Activity, sub: &str, quantity: f64, cash: f64| {
        a.category = Category::Trade;
        a.activity_type = "Trade".into();
        a.activity_sub_type = sub.into();
        a.kind = Kind::Crypto;
        a.quantity = quantity;
        a.net_cash_amount = cash;
    };
    if is("CRYPTOBUY") {
        as_trade(&mut a, "BUY", qty, -cash.abs());
        return a;
    }
    if is("CRYPTOSELL") {
        as_trade(&mut a, "SELL", -qty, cash.abs());
        return a;
    }
    if is("CRYPTOTRANSFER") {
        a.flags.push(Flag::Transfer);
        if compact(&raw.activity_sub_type).contains("OUT") || cash < 0.0 {
            as_trade(&mut a, "SELL", -qty, cash.abs());
            a.flags.push(Flag::TransferOut);
        } else {
            as_trade(&mut a, "BUY", qty, -cash.abs());
            // a deposited coin has no known entry: a later sale is unscoreable
            a.flags.push(Flag::BasisUnknown);
        }
        return a;
    }
    if is("CRYPTOSTAKINGREWARD") {
        // Units arriving at no cost: they enter the book at zero, so the whole
        // proceeds show as gain when they are sold.
        as_trade(&mut a, "BUY", qty, 0.0);
        a.unit_price = 0.0;
        a.flags.push(Flag::Reward);
        return a;
    }
    if rt.starts_with("CRYPTO") {
        a.category = Category::Other;
        a.kind = Kind::Crypto;
        return a;
    }

    // A distribution posted in units with no cash is a pending notice, not a
    // share delivery: Wealthsimple's balance does not grow by it.
    if at == "STKDIS" && rt == "DIVIDEND" && cash.abs() < EPS {
        a.category = Category::Other;
        a.flags.push(Flag::PendingDistribution);
        a.kind = kind_of(raw, &rt, &a);
        return a;
    }

    let raw_both = format!("{}{}", rt, at);
    if raw_both.contains("MULTILEG") {
        a.category = Category::Trade;
        if cash < 0.0 || compact(&raw.direction) == "DEBIT" {
            a.activity_type = "OPTIONS_BUY".into();
            a.activity_sub_type = "BUYTOCLOSE".into();
        } else {
            a.activity_type = "OPTIONS_SELL".into();
            a.activity_sub_type = "SELLTOOPEN".into();
        }
    } else if raw_both.contains("EXPIR") || raw_both.contains("ASSIGN") || raw_both.contains("EXERCISE") {
        a.category = Category::OptionEvent;
        let (kind, sub) = if raw_both.contains("ASSIGN") {
            ("ASSIGN", "BUYTOCLOSE")
        } else if raw_both.contains("SHORTEXPIR") {
            ("EXPIR", "BUY")
        } else if raw_both.contains("EXPIR") {
            ("EXPIR", "SELL")
        } else {
            ("EXERCISE", "SELL")
        };
        a.activity_type = kind.into();
        a.activity_sub_type = sub.into();
        if raw_both.contains("ASSIGN") || cash.abs() < 1e-12 {
            a.unit_price = 0.0;
        }
        if qty > 0.0 {
            a.quantity = if sub == "SELL" { -qty } else { qty };
        }
    }
    a.kind = kind_of(raw, &rt, &a);
    a
}

/// What is traded: a kind the row states wins, then crypto, then the symbol.
fn kind_of(raw: &RawActivity, raw_type_c: &str, a: &Activity) -> Kind {
    Kind::parse(&raw.kind).unwrap_or_else(|| {
        if is_crypto(raw_type_c, &a.type_c()) {
            Kind::Crypto
        } else if is_option_symbol(&a.symbol) {
            Kind::Options
        } else {
            Kind::Shares
        }
    })
}

pub fn normalize_all(rows: &[RawActivity]) -> Vec<Activity> {
    rows.iter().map(normalize).collect()
}

/// The account a row's book belongs to: the nickname when there is one, so two
/// accounts with the same symbol keep separate books; the ids only when there
/// is not.
pub fn fifo_account(a: &Activity) -> String {
    [&a.account_name, &a.fifo_id, &a.account_id].into_iter().find(|v| !v.is_empty()).cloned().unwrap_or_default()
}

/// One book: an account's lots in one symbol and currency.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BookKey {
    pub account: String,
    pub symbol: String,
    pub currency: String,
}

pub fn book_key(a: &Activity) -> BookKey {
    BookKey { account: fifo_account(a), symbol: a.symbol.clone(), currency: a.currency.clone() }
}

/// What an option roll is folded within: one account, one underlying, one right.
pub type RollKey = (String, String, &'static str);

pub fn roll_key(a: &Activity) -> RollKey {
    (fifo_account(a), underlying_symbol(&a.symbol), option_right(&a.symbol))
}

pub fn is_multileg(a: &Activity) -> bool {
    a.raw_type_c().contains("MULTILEG")
}

/// Net the +N/-N name-change rows posted on one day, and open whatever is left
/// over at $0. Each surviving row keeps its index in the caller's list; the
/// netted replacement has none, since it is not one of them.
pub fn fold_stkdis(activities: Vec<(Option<usize>, Activity)>) -> Vec<(Option<usize>, Activity)> {
    struct Group {
        pos: f64,
        neg: f64,
        sample: Activity,
    }
    let mut rest: Vec<(Option<usize>, Activity)> = Vec::new();
    let mut groups: Vec<((String, String, String), Group)> = Vec::new();
    for (src, a) in activities {
        if a.type_c() != "STKDIS" {
            rest.push((src, a));
            continue;
        }
        let key = (a.symbol.clone(), a.transaction_date.clone(), a.currency.clone());
        let at = match groups.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                groups.push((key, Group { pos: 0.0, neg: 0.0, sample: a.clone() }));
                groups.len() - 1
            }
        };
        if a.activity_sub_type == "SELL" || a.quantity < 0.0 {
            groups[at].1.neg += a.quantity.abs();
        } else {
            groups[at].1.pos += a.quantity.abs();
        }
    }
    for (_, g) in groups {
        let net = g.pos - g.neg;
        if net > EPS {
            let mut a = g.sample;
            a.quantity = net;
            a.activity_sub_type = "BUY".into();
            a.unit_price = 0.0;
            a.net_cash_amount = 0.0;
            a.category = Category::Trade;
            rest.push((None, a));
        }
    }
    rest
}
