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
    /// Only a JSON `true` is true; any other value, of any type, is the last
    /// page -- and never fails the page it came on.
    #[serde(deserialize_with = "only_true")]
    pub has_next_page: bool,
    #[serde(deserialize_with = "lenient::text")]
    pub end_cursor: String,
}

fn only_true<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(serde_json::Value::deserialize(d)? == serde_json::Value::Bool(true))
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

/// `FetchSecurityMarketData`: the ticket's order types and margin rate for
/// one security.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SecurityMarketData {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub security: Option<MarketDataSecurity>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarketDataSecurity {
    #[serde(deserialize_with = "lenient::texts")]
    pub allowed_order_subtypes: Vec<String>,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub margin_rates: Option<MarginRates>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarginRates {
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub client_margin_rate: Option<f64>,
}

/// `FetchTradingBalanceBuyingPower`: the ticket's buying power and cash on
/// one account, in the ticket's currency.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TradingBalanceBuyingPower {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub account: Option<TradingBalanceAccount>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TradingBalanceAccount {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub financials: Option<TradingBalanceFinancials>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TradingBalanceFinancials {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub current: Option<TradingBalanceCurrent>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TradingBalanceCurrent {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub trading_balance_view_v2: Option<TradingBalanceView>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TradingBalanceView {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub buying_power: Option<BalanceAmount>,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub cash: Option<BalanceAmount>,
}

/// Wealthsimple's trading-balance figure: `quantity` and `currency`, not
/// `Money`'s `amount` -- a different shape for the same idea.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BalanceAmount {
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub quantity: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
}

// --- orders ---

/// `FetchSoOrdersExtendedOrder`: where one order stands.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExtendedOrderAnswer {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub so_orders_extended_order: Option<ExtendedOrder>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExtendedOrder {
    #[serde(deserialize_with = "lenient::text")]
    pub status: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub filled_quantity: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub average_filled_price: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub submitted_at_utc: String,
    #[serde(deserialize_with = "lenient::text")]
    pub expired_at_utc: String,
    #[serde(deserialize_with = "lenient::text")]
    pub first_filled_at_utc: String,
    #[serde(deserialize_with = "lenient::text")]
    pub last_filled_at_utc: String,
    #[serde(deserialize_with = "lenient::text")]
    pub rejection_cause: String,
    #[serde(deserialize_with = "lenient::text")]
    pub rejection_code: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub submitted_quantity: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub limit_price: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub stop_price: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub time_in_force: String,
    #[serde(deserialize_with = "lenient::text")]
    pub security_currency: String,
    #[serde(deserialize_with = "lenient::text")]
    pub canonical_account_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub account_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub security_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub order_type: String,
}

/// `OrderServiceExtendedOrderFeed`: the orders still open at Wealthsimple.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OrderFeedAnswer {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub identity: Option<OrderFeedIdentity>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OrderFeedIdentity {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub order_service_extended_order_feed: Option<Connection<FeedOrder>>,
}

/// One open order in the feed. Its fill price is `averageFillPrice` here and
/// `averageFilledPrice` in the extended order: two names for one figure.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FeedOrder {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub order_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub canonical_account_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub created_at_utc: String,
    #[serde(deserialize_with = "lenient::text")]
    pub status: String,
    #[serde(deserialize_with = "lenient::text")]
    pub side: String,
    #[serde(deserialize_with = "lenient::text")]
    pub execution_type: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub submitted_quantity: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub limit_price: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub stop_price: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub average_fill_price: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub security_currency: String,
    #[serde(deserialize_with = "lenient::text")]
    pub security_id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub symbol: String,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub security: Option<FeedSecurity>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FeedSecurity {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub stock: Option<Stock>,
}

/// `FetchSecuritiesSummary`: the ticket's quote for each security asked.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SecuritiesSummary {
    #[serde(deserialize_with = "lenient::list")]
    pub securities: Vec<SummarySecurity>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SummarySecurity {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub security_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::text")]
    pub status: String,
    #[serde(deserialize_with = "lenient::truthy")]
    pub buyable: bool,
    #[serde(deserialize_with = "lenient::truthy")]
    pub sellable: bool,
    #[serde(deserialize_with = "lenient::truthy")]
    pub ws_trade_eligible: bool,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub stock: Option<Stock>,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub quote_v2: Option<QuoteV2>,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub option_details: Option<QuoteOptionDetails>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QuoteV2 {
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub price: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub last: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub previous_baseline: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub reference_close: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub bid: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub ask: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub bid_size: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub ask_size: Option<f64>,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub mid: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::text")]
    pub market_status: String,
    #[serde(deserialize_with = "lenient::text")]
    pub quoted_as_of: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QuoteOptionDetails {
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub multiplier: Option<f64>,
}

/// `FetchSecuritySearchResult`: the listings a symbol search finds.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SecuritySearchAnswer {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub security_search: Option<SecuritySearch>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SecuritySearch {
    #[serde(deserialize_with = "lenient::list")]
    pub results: Vec<SearchResult>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "lenient::text")]
    pub security_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub stock: Option<Stock>,
}

/// `SoOrdersOrderCreate`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CreateOrderAnswer {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub so_orders_create_order: Option<CreateOrderResult>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CreateOrderResult {
    pub errors: Refusal,
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub order: Option<CreatedOrder>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CreatedOrder {
    #[serde(deserialize_with = "lenient::text")]
    pub order_id: String,
}

/// `SoOrdersOrderCancel`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CancelOrderAnswer {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub order_service_cancel_order: Option<MutationResult>,
}

/// `SoOrdersOrderModify`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModifyOrderAnswer {
    #[serde(deserialize_with = "lenient::maybe_object")]
    pub so_orders_modify_order: Option<MutationResult>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MutationResult {
    pub errors: Refusal,
}

/// A mutation's `errors`: `None` when Wealthsimple said nothing against it
/// (null, false, an empty list, object or text), else its reason -- the first
/// error's `message`, or its `code`, or the text itself. Any other shape that
/// says something is a refusal too, never read as an acceptance: an order is
/// not taken as placed, cancelled or changed on an answer that objects to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Refusal(pub Option<String>);

impl<'de> Deserialize<'de> for Refusal {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde_json::Value;
        fn reason(v: &Value) -> String {
            match v {
                Value::Object(m) => {
                    let text = |k: &str| bagholder_model::value::s(m.get(k));
                    let msg = text("message");
                    if msg.is_empty() { text("code") } else { msg }
                }
                other => bagholder_model::value::s(Some(other)),
            }
        }
        let v = Value::deserialize(d)?;
        Ok(Refusal(match &v {
            Value::Null | Value::Bool(false) => None,
            Value::Array(a) => a.first().map(reason),
            Value::Object(m) if m.is_empty() => None,
            Value::String(t) if t.is_empty() => None,
            other => Some(reason(other)),
        }))
    }
}
