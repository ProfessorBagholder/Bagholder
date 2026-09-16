//! The GraphQL operations the Wealthsimple client sends.
//!
//! Each one is the exact text `bagholder.QUERIES` holds, extracted from it
//! into `graphql/` and included here rather than retyped, so the two cannot
//! drift. They were recovered from the public web bundle; nothing here is
//! invented.

/// `bagholder.QUERIES`.
pub const QUERIES: [(&str, &str); 17] = [
    ("FetchAccountCurrentMarginBuyingPowerV2", include_str!("../graphql/FetchAccountCurrentMarginBuyingPowerV2.graphql")),
    ("FetchAccountHistoricalFinancials", include_str!("../graphql/FetchAccountHistoricalFinancials.graphql")),
    ("FetchAccountsWithBalance", include_str!("../graphql/FetchAccountsWithBalance.graphql")),
    ("FetchActivityFeedItems", include_str!("../graphql/FetchActivityFeedItems.graphql")),
    ("FetchAllAccountFinancials", include_str!("../graphql/FetchAllAccountFinancials.graphql")),
    ("FetchSecurities", include_str!("../graphql/FetchSecurities.graphql")),
    ("FetchSecuritiesSummary", include_str!("../graphql/FetchSecuritiesSummary.graphql")),
    ("FetchSecurity", include_str!("../graphql/FetchSecurity.graphql")),
    ("FetchSecurityMarketData", include_str!("../graphql/FetchSecurityMarketData.graphql")),
    ("FetchSecuritySearchResult", include_str!("../graphql/FetchSecuritySearchResult.graphql")),
    ("FetchSoOrdersExtendedOrder", include_str!("../graphql/FetchSoOrdersExtendedOrder.graphql")),
    ("FetchTradingBalanceBuyingPower", include_str!("../graphql/FetchTradingBalanceBuyingPower.graphql")),
    ("IdentityHistoricalFinancialsQuery", include_str!("../graphql/IdentityHistoricalFinancialsQuery.graphql")),
    ("OrderServiceExtendedOrderFeed", include_str!("../graphql/OrderServiceExtendedOrderFeed.graphql")),
    ("SoOrdersOrderCancel", include_str!("../graphql/SoOrdersOrderCancel.graphql")),
    ("SoOrdersOrderCreate", include_str!("../graphql/SoOrdersOrderCreate.graphql")),
    ("SoOrdersOrderModify", include_str!("../graphql/SoOrdersOrderModify.graphql")),
];

/// The operation's query text, by name.
pub fn query(operation: &str) -> Option<&'static str> {
    QUERIES.iter().find(|(k, _)| *k == operation).map(|(_, q)| *q)
}
