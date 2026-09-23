//! Rows the Wealthsimple client hands the store: the account list, balances,
//! margin figures, NAV points, and one broker activity mapped to a ledger
//! row. Every wire read of Wealthsimple's own answers happens in
//! `bagholder-ws`; these are the typed shapes it hands on to be written.

use serde::{Deserialize, Deserializer, Serialize};

use bagholder_model::lenient;

/// A text field that is `None` rather than empty when absent, null, or blank
/// -- `canonicalId` and `securityId` are never `Some("")`.
fn opt_text<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    let s = bagholder_model::value::s(Some(&v));
    Ok(if s.is_empty() { None } else { Some(s) })
}

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

/// One Wealthsimple activity, mapped to a ledger row. Field order is the
/// order the ledger writer reads them in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MappedActivity {
    #[serde(deserialize_with = "opt_text")]
    pub canonical_id: Option<String>,
    #[serde(deserialize_with = "lenient::text")]
    pub occurred_at: String,
    #[serde(deserialize_with = "lenient::text")]
    pub transaction_date: String,
    #[serde(deserialize_with = "lenient::text")]
    pub settlement_date: String,
    #[serde(deserialize_with = "lenient::text")]
    pub account_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub book_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub fifo_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub account_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub activity_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub activity_sub_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub description: String,
    #[serde(deserialize_with = "lenient::text")]
    pub direction: String,
    #[serde(deserialize_with = "lenient::text")]
    pub symbol: String,
    #[serde(deserialize_with = "lenient::text")]
    pub name: String,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::number")]
    pub quantity: f64,
    #[serde(deserialize_with = "lenient::number")]
    pub unit_price: f64,
    #[serde(deserialize_with = "lenient::number")]
    pub commission: f64,
    #[serde(deserialize_with = "lenient::number")]
    pub net_cash_amount: f64,
    #[serde(deserialize_with = "lenient::text")]
    pub category: String,
    /// Never set here; the running balance is computed downstream.
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub balance: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub source: String,
    #[serde(deserialize_with = "lenient::text")]
    pub raw_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub aft_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub counter_symbol: String,
    #[serde(deserialize_with = "opt_text")]
    pub security_id: Option<String>,
}

impl MappedActivity {
    /// The rows as the seam into `merge::apply_wealthsimple_mapped` still
    /// wants them, until that is typed too (stage 6d7).
    pub fn to_rows(rows: &[Self]) -> Vec<serde_json::Value> {
        rows.iter().map(|r| serde_json::to_value(r).unwrap_or(serde_json::Value::Null)).collect()
    }
}
