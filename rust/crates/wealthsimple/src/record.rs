//! What one Wealthsimple record holds (`docs/plans/stage-3b-wealthsimple.md`,
//! "The pull"): an activity row, and beside it every reply read once for that
//! row, each kept as Wealthsimple sent it, so the mapping derives from one record
//! everything Wealthsimple stated about the row and `rederive` never needs the
//! network.
//!
//! ```text
//! {
//!   "activity":     the row (FetchActivityFeedItems' node)
//!   "securities":   { security id: the security's record }, for the row's
//!                   security, each leg's, and each option's underlying
//!   "order":        a multi-leg row's order (FetchSoOrdersMultilegOrder)
//!   "entitlements": a corporate action's children (FetchCorporateActionChildActivities)
//!   "conversion":   a currency conversion's detail (FetchFundingIntent's node or
//!                   FetchInternalTransfer's), or a move's that states no amount
//!   "transfer":     a transfer in from another institution's detail
//!                   (FetchInstitutionalTransfer)
//!   "deposits":     [{ "account", "nodes": each day's net deposits }]: for a move
//!                   between accounts that states no amount, both accounts' days
//!                   from the day before its own to the first on which either's
//!                   net deposits changed (FetchAccountHistoricalFinancials)
//!   "withheld":     a withdrawal's tax withheld rows (sharing its id): its
//!                   gross amount is what reached the other account and the tax
//!   "siblings":     moves between the same two accounts on neighbouring days; for
//!                   a transfer from another institution, the account's other rows
//!                   read against positions while it was under way
//!   "positions":    [{ "account", "day", "nodes": positions as of that day }],
//!                   around an event or a move of holdings
//!   "book":         [{ "account", "security", "day", "quantity" }]: what the
//!                   book's own transactions moved in those accounts on those
//!                   days, as the pull saw it, so the positions' change is read
//!                   net of it (brief 07 §4). The one part not from Wealthsimple.
//! }
//! ```
//!
//! A part that does not apply to the row is absent.

use std::collections::BTreeMap;

use bagholder_core::json::Value;

/// A record's payload, assembled by the pull.
#[derive(Clone, Debug)]
pub struct Record {
    pub activity: Value,
    pub securities: BTreeMap<String, Value>,
    pub order: Option<Value>,
    pub entitlements: Option<Value>,
    pub conversion: Option<Value>,
    pub transfer: Option<Value>,
    pub deposits: Vec<Deposits>,
    pub withheld: Vec<Value>,
    /// Moves between the same two accounts on neighbouring days, read with it.
    pub siblings: Vec<Value>,
    pub positions: Vec<Positions>,
    pub book: Vec<BookMove>,
}

/// Positions as of one day in one account, as Wealthsimple stated them.
#[derive(Clone, Debug)]
pub struct Positions {
    pub account: String,
    pub day: String,
    pub nodes: Value,
}

/// An account's net deposits day by day, as Wealthsimple stated them.
#[derive(Clone, Debug)]
pub struct Deposits {
    pub account: String,
    pub nodes: Value,
}

/// What the book's own transactions moved of one security in one account on
/// one day.
#[derive(Clone, Debug)]
pub struct BookMove {
    pub account: String,
    pub security: String,
    pub day: String,
    /// Decimal text, signed: into the account is positive.
    pub quantity: String,
}

impl Record {
    /// A row with nothing read beside it yet.
    pub fn of(activity: Value) -> Record {
        Record { activity, securities: BTreeMap::new(), order: None, entitlements: None, conversion: None, transfer: None, deposits: vec![], withheld: vec![], siblings: vec![], positions: vec![], book: vec![] }
    }

    /// The payload the book stores.
    pub fn to_value(&self) -> Value {
        let mut m = BTreeMap::new();
        m.insert("activity".to_string(), self.activity.clone());
        if !self.securities.is_empty() {
            m.insert("securities".to_string(), Value::Object(self.securities.clone()));
        }
        for (k, v) in [("order", &self.order), ("entitlements", &self.entitlements), ("conversion", &self.conversion), ("transfer", &self.transfer)] {
            if let Some(v) = v {
                m.insert(k.to_string(), v.clone());
            }
        }
        if !self.deposits.is_empty() {
            let items = self
                .deposits
                .iter()
                .map(|d| {
                    let mut o = BTreeMap::new();
                    o.insert("account".to_string(), Value::String(d.account.clone()));
                    o.insert("nodes".to_string(), d.nodes.clone());
                    Value::Object(o)
                })
                .collect();
            m.insert("deposits".to_string(), Value::Array(items));
        }
        if !self.withheld.is_empty() {
            m.insert("withheld".to_string(), Value::Array(self.withheld.clone()));
        }
        if !self.siblings.is_empty() {
            m.insert("siblings".to_string(), Value::Array(self.siblings.clone()));
        }
        if !self.positions.is_empty() {
            let items = self
                .positions
                .iter()
                .map(|p| {
                    let mut o = BTreeMap::new();
                    o.insert("account".to_string(), Value::String(p.account.clone()));
                    o.insert("day".to_string(), Value::String(p.day.clone()));
                    o.insert("nodes".to_string(), p.nodes.clone());
                    Value::Object(o)
                })
                .collect();
            m.insert("positions".to_string(), Value::Array(items));
        }
        if !self.book.is_empty() {
            let items = self
                .book
                .iter()
                .map(|b| {
                    let mut o = BTreeMap::new();
                    o.insert("account".to_string(), Value::String(b.account.clone()));
                    o.insert("security".to_string(), Value::String(b.security.clone()));
                    o.insert("day".to_string(), Value::String(b.day.clone()));
                    o.insert("quantity".to_string(), Value::String(b.quantity.clone()));
                    Value::Object(o)
                })
                .collect();
            m.insert("book".to_string(), Value::Array(items));
        }
        Value::Object(m)
    }
}
