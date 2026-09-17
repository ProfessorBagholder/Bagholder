// The GraphQL documents the Wealthsimple web app sends, as crates/ws/src/queries.rs and
// the iOS app carry them. Generated from WSPull.swift; keep the three the same.
package com.bagholder.app

object Queries {
    val FETCH_ALL_ACCOUNT_FINANCIALS = """
query FetchAllAccountFinancials(${'$'}identityId: ID!, ${'$'}startDate: Date, ${'$'}pageSize: Int = 25, ${'$'}cursor: String) {
  identity(id: ${'$'}identityId) {
    id
    ...AllAccountFinancials
    __typename
  }
}

fragment AllAccountFinancials on Identity {
  accounts(filter: {}, first: ${'$'}pageSize, after: ${'$'}cursor) {
    pageInfo {
      hasNextPage
      endCursor
      __typename
    }
    edges {
      cursor
      node {
        ...AccountWithFinancials
        __typename
      }
      __typename
    }
    __typename
  }
  __typename
}

fragment AccountWithFinancials on Account {
  ...AccountWithLink
  ...AccountFinancials
  __typename
}

fragment AccountWithLink on Account {
  ...Account
  linkedAccount {
    ...Account
    __typename
  }
  __typename
}

fragment Account on Account {
  ...AccountCore
  custodianAccounts {
    ...CustodianAccount
    __typename
  }
  __typename
}

fragment AccountCore on Account {
  id
  archivedAt
  branch
  closedAt
  createdAt
  cacheExpiredAt
  currency
  requiredIdentityVerification
  unifiedAccountType
  supportedCurrencies
  nickname
  status
  accountOwnerConfiguration
  accountFeatures {
    ...AccountFeature
    __typename
  }
  accountOwners {
    ...AccountOwner
    __typename
  }
  type
  __typename
}

fragment AccountFeature on AccountFeature {
  name
  enabled
  __typename
}

fragment AccountOwner on AccountOwner {
  accountId
  identityId
  accountNickname
  clientCanonicalId
  accountOpeningAgreementsSigned
  name
  email
  ownershipType
  activeInvitation {
    ...AccountOwnerInvitation
    __typename
  }
  sentInvitations {
    ...AccountOwnerInvitation
    __typename
  }
  __typename
}

fragment AccountOwnerInvitation on AccountOwnerInvitation {
  id
  createdAt
  inviteeName
  inviteeEmail
  inviterName
  inviterEmail
  updatedAt
  sentAt
  status
  __typename
}

fragment CustodianAccount on CustodianAccount {
  id
  branch
  custodian
  status
  updatedAt
  __typename
}

fragment AccountFinancials on Account {
  id
  custodianAccounts {
    id
    branch
    financials {
      current {
        ...CustodianAccountCurrentFinancialValues
        __typename
      }
      __typename
    }
    __typename
  }
  financials {
    currentCombined {
      id
      ...AccountCurrentFinancials
      __typename
    }
    __typename
  }
  __typename
}

fragment CustodianAccountCurrentFinancialValues on CustodianAccountCurrentFinancialValues {
  deposits { ...Money __typename }
  earnings { ...Money __typename }
  netDeposits { ...Money __typename }
  netLiquidationValue { ...Money __typename }
  withdrawals { ...Money __typename }
  __typename
}

fragment Money on Money {
  amount
  cents
  currency
  __typename
}

fragment AccountCurrentFinancials on AccountCurrentFinancials {
  id
  netLiquidationValue { ...Money __typename }
  netDeposits { ...Money __typename }
  simpleReturns(referenceDate: ${'$'}startDate) { ...SimpleReturns __typename }
  totalDeposits { ...Money __typename }
  totalWithdrawals { ...Money __typename }
  __typename
}

fragment SimpleReturns on SimpleReturns {
  amount { ...Money __typename }
  asOf
  rate
  referenceDate
  __typename
}
""".trimIndent()

    val FETCH_ACCOUNTS_WITH_BALANCE = """
query FetchAccountsWithBalance(${'$'}ids: [String!]!, ${'$'}type: BalanceType!) {
  accounts(ids: ${'$'}ids) {
    ...AccountWithBalance
    __typename
  }
}

fragment AccountWithBalance on Account {
  id
  custodianAccounts {
    id
    financials {
      ... on CustodianAccountFinancialsSo {
        balance(type: ${'$'}type) {
          ...Balance
          __typename
        }
        __typename
      }
      __typename
    }
    __typename
  }
  __typename
}

fragment Balance on Balance {
  quantity
  securityId
  __typename
}
""".trimIndent()

    val FETCH_ACCOUNT_MARGIN_BUYING_POWER = """
query FetchAccountCurrentMarginBuyingPowerV2(${'$'}accountId: ID!, ${'$'}currency: Currency = CAD) {
  account(id: ${'$'}accountId) {
    id
    financials {
      current {
        id
        marginV3 {
          trading {
            buyingPower(asCurrency: ${'$'}currency) {
              __typename
              ... on BuyingPowerMetricAvailable {
                total { amount currency __typename }
                __typename
              }
              ... on BuyingPowerMetricUnavailable {
                reason {
                  __typename
                  ... on UnavailableSecurities {
                    securities { securityId status __typename }
                    __typename
                  }
                }
                __typename
              }
            }
            __typename
          }
          __typename
        }
        __typename
      }
      __typename
    }
    __typename
  }
}
""".trimIndent()

    val FETCH_ACTIVITY_FEED_ITEMS = """
query FetchActivityFeedItems(${'$'}first: Int, ${'$'}cursor: Cursor, ${'$'}condition: ActivityCondition, ${'$'}orderBy: [ActivitiesOrderBy!] = OCCURRED_AT_DESC) {
  activityFeedItems(
    first: ${'$'}first
    after: ${'$'}cursor
    condition: ${'$'}condition
    orderBy: ${'$'}orderBy
  ) {
    edges {
      node {
        ...Activity
        __typename
      }
      __typename
    }
    pageInfo {
      hasNextPage
      endCursor
      __typename
    }
    __typename
  }
}

fragment Activity on ActivityFeedItem {
  accountId
  aftOriginatorName
  aftTransactionCategory
  aftTransactionType
  amount
  amountSign
  assetQuantity
  assetSymbol
  canonicalId
  currency
  eTransferEmail
  eTransferName
  externalCanonicalId
  identityId
  institutionName
  occurredAt
  p2pHandle
  p2pMessage
  spendMerchant
  securityId
  billPayCompanyName
  billPayPayeeNickname
  redactedExternalAccountNumber
  opposingAccountId
  status
  subType
  type
  strikePrice
  contractType
  expiryDate
  chequeNumber
  provisionalCreditAmount
  primaryBlocker
  interestRate
  frequency
  counterAssetSymbol
  rewardProgram
  counterPartyCurrency
  counterPartyCurrencyAmount
  counterPartyName
  fxRate
  fees
  reference
  __typename
}
""".trimIndent()

    val FETCH_SECURITY = """
query FetchSecurity(${'$'}securityId: ID!) {
  security(id: ${'$'}securityId) {
    id
    currency
    stock { name primaryExchange primaryMic symbol }
    optionDetails { underlyingSecurity { id currency } }
    __typename
  }
}
""".trimIndent()

    val IDENTITY_HISTORICAL_FINANCIALS = """
query IdentityHistoricalFinancialsQuery(
  ${'$'}identityId: ID!
  ${'$'}currency: Currency!
  ${'$'}startDate: Date!
  ${'$'}endDate: Date
  ${'$'}limit: Int
  ${'$'}cursor: String
  ${'$'}includeNetDeposits: Boolean = true
) {
  identity(id: ${'$'}identityId) {
    id
    financials(filter: { archived: false }) {
      historicalDaily(
        currency: ${'$'}currency
        startDate: ${'$'}startDate
        endDate: ${'$'}endDate
        first: ${'$'}limit
        after: ${'$'}cursor
      ) {
        edges {
          cursor
          node {
            date
            netLiquidationValue { amount currency __typename }
            netDeposits @include(if: ${'$'}includeNetDeposits) { amount currency __typename }
            __typename
          }
          __typename
        }
        pageInfo {
          endCursor
          hasNextPage
          __typename
        }
        __typename
      }
      __typename
    }
    __typename
  }
}
""".trimIndent()

    val FETCH_ACCOUNT_HISTORICAL_FINANCIALS = """
query FetchAccountHistoricalFinancials(
  ${'$'}id: ID!
  ${'$'}currency: Currency!
  ${'$'}startDate: Date
  ${'$'}resolution: DateResolution!
  ${'$'}endDate: Date
  ${'$'}first: Int
  ${'$'}cursor: String
) {
  account(id: ${'$'}id) {
    id
    financials {
      historicalDaily(
        currency: ${'$'}currency
        startDate: ${'$'}startDate
        resolution: ${'$'}resolution
        endDate: ${'$'}endDate
        first: ${'$'}first
        after: ${'$'}cursor
      ) {
        edges {
          node {
            date
            netLiquidationValueV2 { amount currency __typename }
            netDepositsV2 { amount currency __typename }
            __typename
          }
          __typename
        }
        pageInfo {
          hasNextPage
          endCursor
          __typename
        }
        __typename
      }
      __typename
    }
    __typename
  }
}
""".trimIndent()

}
