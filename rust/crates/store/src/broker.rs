//! Rows the Wealthsimple client hands the store: the account list, balances,
//! margin figures, NAV points, and one broker activity mapped to a ledger
//! row. Every wire read of Wealthsimple's own answers happens in
//! `bagholder-ws`; these are the typed shapes it hands on to be written.

use serde::{Deserialize, Serialize};

use bagholder_model::lenient;

/// One Wealthsimple account, as the store keeps it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Account {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub nickname: String,
    #[serde(deserialize_with = "lenient::text")]
    pub unified_account_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::text")]
    pub status: String,
    #[serde(rename = "type", deserialize_with = "lenient::text")]
    pub kind: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub net_liquidation_value: Option<f64>,
    /// The margin account this one backs, when it is Margin Boost collateral.
    #[serde(deserialize_with = "lenient::text")]
    pub margin_account_id: String,
}

/// One security position, as Wealthsimple's balance answer carries it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Balance {
    #[serde(deserialize_with = "lenient::text")]
    pub account_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub custodian_account_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub security_id: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub quantity: Option<f64>,
}

/// One account's margin buying power, or the reason it has none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Margin {
    #[serde(deserialize_with = "lenient::text")]
    pub account_id: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub buying_power: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::text")]
    pub unavailable: String,
    #[serde(deserialize_with = "lenient::text")]
    pub fetched_at: String,
}

/// One day's net liquidation value, identity-wide or for one account.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NavPoint {
    /// Absent for the identity-wide series, which carries no account id.
    #[serde(deserialize_with = "lenient::text", skip_serializing_if = "String::is_empty")]
    pub account_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub date: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub equity: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::maybe_number", skip_serializing_if = "Option::is_none")]
    pub net_deposits: Option<f64>,
}

// A Wealthsimple activity mapped to a ledger row is `crate::activities::ActivityRow` --
// `bagholder-ws`'s `mapping` module builds one directly, so no in-between shape is
// needed here any more.
