//! Broker connections and accounts (`docs/architecture.md` §5), in Bagholder's
//! vocabulary. A broker's own account ids are references to an account, so the
//! separate ids a broker gives one account's currencies are one account here.

use crate::ids::{AccountId, ConnectionId, IdError};
use crate::names::Broker;

/// One login at one broker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    pub id: ConnectionId,
    pub broker: Broker,
    /// What the person calls it (`Wealthsimple`), for the screens.
    pub label: String,
}

text_enum! {
    /// What an account does.
    AccountKind "kind of account" {
        /// Securities, paid in full.
        Cash = "cash",
        /// Securities, with borrowing.
        Margin = "margin",
        Crypto = "crypto",
        /// A prediction market's contracts.
        EventContracts = "event-contracts",
        /// Everyday money: a chequing or savings account.
        Spending = "spending",
        CreditCard = "credit-card",
        LineOfCredit = "line-of-credit",
    }
}

text_enum! {
    /// A registered plan, or none.
    Registration "registration" {
        Unregistered = "none",
        Tfsa = "tfsa",
        Fhsa = "fhsa",
        Rrsp = "rrsp",
        Rrif = "rrif",
        Resp = "resp",
        Lira = "lira",
        GroupRrsp = "group-rrsp",
    }
}

text_enum! {
    AccountStatus "account status" {
        Open = "open",
        Closed = "closed",
    }
}

/// What a broker says an account is, once mapped: a type Bagholder knows, or the
/// broker's own words for one it does not, kept and shown as a problem rather
/// than guessed into another kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountType {
    Known {
        kind: AccountKind,
        registration: Registration,
        /// Managed by the broker, not self-directed.
        managed: bool,
        /// Held jointly.
        joint: bool,
    },
    Unrecognised(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub connection: ConnectionId,
    pub account_type: AccountType,
    pub status: AccountStatus,
    /// The person's name for it, where they gave one.
    pub nickname: Option<String>,
}

/// A broker's own id for an account: the scheme is the broker.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountRef {
    pub broker: Broker,
    pub value: String,
}

impl AccountRef {
    pub fn new(broker: Broker, value: impl Into<String>) -> AccountRef {
        AccountRef { broker, value: value.into() }
    }

    /// The stored scheme: `broker-account:wealthsimple`.
    pub fn scheme_text(&self) -> String {
        format!("broker-account:{}", self.broker)
    }

    pub fn parse_scheme(s: &str) -> Result<Broker, IdError> {
        let b = s.strip_prefix("broker-account:").ok_or_else(|| IdError(s.to_string()))?;
        Broker::parse(b)
    }
}
