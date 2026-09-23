//! What Wealthsimple sends back, read once at the edge. Every type here is
//! `Deserialize` only -- nothing downstream ever sees a `serde_json::Value`
//! for one of these answers again.
//!
//! Every reader is lenient (`bagholder_model::lenient`): a field that is
//! absent, null, or of an unexpected shape reads as its harmless default
//! rather than failing the whole answer, exactly as the untyped code did.

use serde::Deserialize;

use bagholder_model::lenient;

/// A GraphQL cursor page. `Serialize` is only for the golden test glue that
/// still reports a page's shape; nothing in the crate itself serializes it.
#[derive(Clone, Debug, Default, Deserialize, serde::Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PageInfo {
    /// Only a JSON `true` is true.
    pub has_next_page: bool,
    #[serde(deserialize_with = "lenient::text")]
    pub end_cursor: String,
}

/// A GraphQL connection: edges of `T`, and where the next page starts.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase", bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub struct Connection<T> {
    #[serde(deserialize_with = "lenient::list")]
    pub edges: Vec<Edge<T>>,
    pub page_info: PageInfo,
}

impl<T> Default for Connection<T> {
    fn default() -> Self {
        Connection { edges: Vec::new(), page_info: PageInfo::default() }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase", bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub struct Edge<T> {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub node: Option<T>,
}

impl<T> Default for Edge<T> {
    fn default() -> Self {
        Edge { node: None }
    }
}

impl<T> Connection<T> {
    pub fn nodes(self) -> impl Iterator<Item = T> {
        self.edges.into_iter().filter_map(|e| e.node)
    }

    /// The next cursor to ask for, or `None` when the broker says there is
    /// no more, or says there is but names no cursor to ask for.
    pub fn next_cursor(&self) -> Option<String> {
        if self.page_info.has_next_page && !self.page_info.end_cursor.is_empty() {
            Some(self.page_info.end_cursor.clone())
        } else {
            None
        }
    }
}

/// A Wealthsimple `Money`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Money {
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub amount: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
}

/// An id, and nothing else -- a `linkedAccount`, a `custodianAccount`, an
/// option's `underlyingSecurity`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IdOnly {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
}

/// `FetchActivityFeedItems` -> `activityFeedItems` node.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ActivityItem {
    #[serde(deserialize_with = "lenient::text")]
    pub canonical_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub occurred_at: String,
    #[serde(deserialize_with = "lenient::text")]
    pub account_id: String,
    #[serde(rename = "type", deserialize_with = "lenient::text")]
    pub kind: String,
    #[serde(deserialize_with = "lenient::text")]
    pub sub_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub status: String,
    #[serde(deserialize_with = "lenient::text")]
    pub aft_transaction_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub aft_transaction_category: String,
    #[serde(deserialize_with = "lenient::text")]
    pub asset_symbol: String,
    #[serde(deserialize_with = "lenient::text")]
    pub counter_asset_symbol: String,
    #[serde(deserialize_with = "lenient::number")]
    pub asset_quantity: f64,
    #[serde(deserialize_with = "lenient::number")]
    pub amount: f64,
    #[serde(deserialize_with = "lenient::text")]
    pub amount_sign: String,
    #[serde(deserialize_with = "lenient::number")]
    pub fees: f64,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::text")]
    pub contract_type: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub strike_price: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub expiry_date: String,
    #[serde(deserialize_with = "lenient::text")]
    pub aft_originator_name: String,
    #[serde(deserialize_with = "lenient::text")]
    pub institution_name: String,
    #[serde(deserialize_with = "lenient::text")]
    pub security_id: String,
}

/// The Margin Boost metadata: the margin account a collateral account backs.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FeatureMetadata {
    #[serde(deserialize_with = "lenient::text")]
    pub target_margin_account_id: String,
}

/// One entry of `accountFeatures`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Feature {
    #[serde(deserialize_with = "lenient::text")]
    pub name: String,
    #[serde(deserialize_with = "lenient::truthy")]
    pub enabled: bool,
    /// Only a JSON `false` turns a feature off.
    pub functional: Option<bool>,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub metadata: Option<FeatureMetadata>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CurrentCombined {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub net_liquidation_value: Option<Money>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountFinancials {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub current_combined: Option<CurrentCombined>,
}

/// `FetchAllAccountFinancials` -> `identity.accounts` node.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountNode {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub nickname: String,
    #[serde(alias = "unified_account_type", deserialize_with = "lenient::text")]
    pub unified_account_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::text")]
    pub status: String,
    #[serde(rename = "type", deserialize_with = "lenient::text")]
    pub kind: String,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub linked_account: Option<IdOnly>,
    #[serde(deserialize_with = "lenient::list")]
    pub custodian_accounts: Vec<IdOnly>,
    #[serde(deserialize_with = "lenient::list")]
    pub account_features: Vec<Feature>,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub financials: Option<AccountFinancials>,
}

/// `FetchAllAccountFinancials` -> `identity.accounts`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IdentityAccounts {
    pub accounts: Option<Connection<AccountNode>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountsAnswer {
    pub identity: Option<IdentityAccounts>,
}

/// `FetchActivityFeedItems`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ActivityAnswer {
    pub activity_feed_items: Option<Connection<ActivityItem>>,
}

/// `FetchAccountsWithBalance`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountsWithBalance {
    #[serde(deserialize_with = "lenient::list")]
    pub accounts: Vec<BalanceAccount>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BalanceAccount {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::list")]
    pub custodian_accounts: Vec<CustodianAccount>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CustodianAccount {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub financials: Option<CustodianFinancials>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CustodianFinancials {
    /// An object is one balance, an array many, anything else none.
    #[serde(deserialize_with = "lenient::one_or_many")]
    pub balance: Vec<BalanceNode>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BalanceNode {
    #[serde(deserialize_with = "lenient::text")]
    pub security_id: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub quantity: Option<f64>,
}

/// `FetchAccountCurrentMarginBuyingPowerV2`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarginAnswer {
    pub account: Option<MarginAccountNode>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarginAccountNode {
    pub financials: Option<MarginFinancials>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarginFinancials {
    pub current: Option<MarginCurrent>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarginCurrent {
    pub margin_v3: Option<MarginV3>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarginV3 {
    pub trading: Option<Trading>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Trading {
    pub buying_power: Option<BuyingPower>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BuyingPower {
    #[serde(rename = "__typename", deserialize_with = "lenient::text")]
    pub typename: String,
    pub total: Option<Money>,
    pub reason: Option<Reason>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Reason {
    #[serde(rename = "__typename", deserialize_with = "lenient::text")]
    pub typename: String,
    /// Only its length is ever read.
    #[serde(deserialize_with = "lenient::list")]
    pub securities: Vec<serde::de::IgnoredAny>,
}

/// `IdentityHistoricalFinancialsQuery` / `FetchAccountHistoricalFinancials`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NavAnswer {
    pub identity: Option<Holder>,
    pub account: Option<Holder>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Holder {
    pub financials: Option<HistoryFinancials>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HistoryFinancials {
    pub historical_daily: Option<Connection<NavNode>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NavNode {
    #[serde(deserialize_with = "lenient::text")]
    pub date: String,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub net_liquidation_value: Option<Money>,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub net_liquidation_value_v2: Option<Money>,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub net_deposits: Option<Money>,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub net_deposits_v2: Option<Money>,
}

/// `FetchSecurity` / `FetchSecurities`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SecurityNode {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    pub stock: Option<Stock>,
    pub option_details: Option<OptionDetails>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Stock {
    #[serde(deserialize_with = "lenient::text")]
    pub symbol: String,
    #[serde(deserialize_with = "lenient::text")]
    pub name: String,
    #[serde(deserialize_with = "lenient::text")]
    pub primary_exchange: String,
    #[serde(deserialize_with = "lenient::text")]
    pub primary_mic: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OptionDetails {
    pub underlying_security: Option<IdOnly>,
}

/// `security` comes back as an empty object when Wealthsimple has no record
/// -- which is not the same as no answer at all -- so it is read as a raw
/// `Value` first and only turned into a `SecurityNode` once known non-empty.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SecurityAnswer {
    pub security: Option<serde_json::Value>,
}

/// `securities`: `None` when the key itself is missing or null, which falls
/// back to one `FetchSecurity` call per id -- distinct from an empty array.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SecuritiesAnswer {
    pub securities: Option<Vec<serde_json::Value>>,
}
