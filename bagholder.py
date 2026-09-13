#!/usr/bin/env python3
"""
Bagholder — unofficial local Wealthsimple session reuse.
Passkey login happens on Wealthsimple's own site (we just open that URL).
After capture of the first-party session, later syncs are silent on this computer.
Nothing is hosted publicly. Tokens never leave this machine.

Run:  python3 bagholder.py
Open the printed http://127.0.0.1 URL (the dashboard), not Wealthsimple.
"""

from __future__ import annotations

import base64
import gzip
import json
import os
import re
import shutil
import signal
import socket
import ssl
import struct
import subprocess
import sys
import threading
import time
import traceback
import uuid
import webbrowser
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, quote, unquote, urlparse
from urllib.request import Request, urlopen

import csvimport
import exposure
import instruments
import market
import news
import universes
import model
import store

# --- constants (tradesimple WealthsimpleAPIBase) ---
OAUTH = "https://api.production.wealthsimple.com/v1/oauth/v2"
GRAPHQL = "https://my.wealthsimple.com/graphql"
GRAPHQL_VERSION = "12"
WS_CLIENT = "@wealthsimple/wealthsimple"
LOGIN_URL = "https://my.wealthsimple.com/app/login"


def _data_home():
    env = (os.environ.get("BAGHOLDER_HOME") or "").strip()
    if env:
        return Path(env)
    return Path.home() / ".bagholder"


HOME = _data_home()
SESSION_PATH = HOME / "session.json"
CLIENT_ID_PATH = HOME / "client_id"
UA_PATH = HOME / "user_agent"
TOKEN_CHECK_SEC = 30
TOKEN_REFRESH_MARGIN_SEC = 300
ACTIVITY_PULL_SEC = 24 * 60 * 60
PORTS = (8765, 8766, 8767)
DEBUG_PORTS = (18765, 18766, 18767)
CAPTURE_WAIT_SEC = 180
OAUTH_COOKIE = "_oauth2_access_v2"
DEVICE_COOKIE = "wssdi"

MONTHS = (
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN",
    "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
)

Q_FETCH_ALL_ACCOUNT_FINANCIALS = """
query FetchAllAccountFinancials($identityId: ID!, $startDate: Date, $pageSize: Int = 25, $cursor: String) {
  identity(id: $identityId) {
    id
    ...AllAccountFinancials
    __typename
  }
}

fragment AllAccountFinancials on Identity {
  accounts(filter: {}, first: $pageSize, after: $cursor) {
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
  functional
  metadata {
    __typename
    ... on MarginBoostFeatureMetadata {
      targetMarginAccountId
      __typename
    }
  }
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
  simpleReturns(referenceDate: $startDate) { ...SimpleReturns __typename }
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
""".strip()

Q_FETCH_ACTIVITY_FEED_ITEMS = """
query FetchActivityFeedItems($first: Int, $cursor: Cursor, $condition: ActivityCondition, $orderBy: [ActivitiesOrderBy!] = OCCURRED_AT_DESC) {
  activityFeedItems(
    first: $first
    after: $cursor
    condition: $condition
    orderBy: $orderBy
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
""".strip()

Q_FETCH_ACCOUNTS_WITH_BALANCE = """
query FetchAccountsWithBalance($ids: [String!]!, $type: BalanceType!) {
  accounts(ids: $ids) {
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
        balance(type: $type) {
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
""".strip()

Q_FETCH_SECURITY_SEARCH_RESULT = """
query FetchSecuritySearchResult($query: String!) {
  securitySearch(input: {query: $query}) {
    results {
      ...SecuritySearchResult
      __typename
    }
    __typename
  }
}

fragment SecuritySearchResult on Security {
  id
  buyable
  status
  currency
  securityType
  wsTradeEligible
  stock {
    symbol
    name
    primaryExchange
    primaryMic
    __typename
  }
  securityGroups {
    id
    name
    __typename
  }
  quoteV2 {
    ... on EquityQuote {
      marketStatus
      __typename
    }
    __typename
  }
  __typename
}
""".strip()


Q_IDENTITY_HISTORICAL_FINANCIALS = """
query IdentityHistoricalFinancialsQuery(
  $identityId: ID!
  $currency: Currency!
  $startDate: Date!
  $endDate: Date
  $limit: Int
  $cursor: String
  $includeNetDeposits: Boolean = true
) {
  identity(id: $identityId) {
    id
    financials(filter: { archived: false }) {
      historicalDaily(
        currency: $currency
        startDate: $startDate
        endDate: $endDate
        first: $limit
        after: $cursor
      ) {
        edges {
          cursor
          node {
            date
            netLiquidationValue { amount currency __typename }
            netDeposits @include(if: $includeNetDeposits) { amount currency __typename }
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
""".strip()

Q_FETCH_ACCOUNT_HISTORICAL_FINANCIALS = """
query FetchAccountHistoricalFinancials(
  $id: ID!
  $currency: Currency!
  $startDate: Date
  $resolution: DateResolution!
  $endDate: Date
  $first: Int
  $cursor: String
) {
  account(id: $id) {
    id
    financials {
      historicalDaily(
        currency: $currency
        startDate: $startDate
        resolution: $resolution
        endDate: $endDate
        first: $first
        after: $cursor
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
""".strip()

Q_FETCH_SECURITY = """
query FetchSecurity($securityId: ID!) {
  security(id: $securityId) {
    id
    currency
    stock { name primaryExchange primaryMic symbol }
    optionDetails { underlyingSecurity { id currency } }
    __typename
  }
}
""".strip()


Q_FETCH_SECURITIES = """
query FetchSecurities($ids: [ID!]!) {
  securities(ids: $ids) {
    id
    currency
    stock { name primaryExchange primaryMic symbol }
    optionDetails { underlyingSecurity { id currency } }
    __typename
  }
}
""".strip()

SECURITY_BATCH = 50

# --- the order ticket: Wealthsimple's own order operations, as its web app sends them ---

Q_FETCH_SECURITIES_SUMMARY = """
query FetchSecuritiesSummary($ids: [ID!]!) {
  securities(ids: $ids) {
    id
    buyable
    sellable
    wsTradeEligible
    securityType
    currency
    status
    stock { name symbol primaryExchange primaryMic }
    optionDetails { multiplier optionType strikePrice expiryDate underlyingSecurity { id } }
    quoteV2(currency: null) {
      __typename
      securityId
      ask
      bid
      currency
      price
      sessionPrice
      quotedAsOf
      previousBaseline
      ... on EquityQuote { marketStatus askSize bidSize close high last lastSize low open mid referenceClose }
      ... on OptionQuote { marketStatus askSize bidSize close high last lastSize low open mid underlyingSpot }
    }
  }
}
""".strip()

Q_FETCH_SECURITY_MARKET_DATA = """
query FetchSecurityMarketData($id: ID!) {
  security(id: $id) {
    id
    allowedOrderSubtypes
    marginRates { clientMarginRate }
  }
}
""".strip()

Q_FETCH_TRADING_BALANCE_BUYING_POWER = """
query FetchTradingBalanceBuyingPower($accountCanonicalId: ID!, $currency: Currency!, $securityId: ID) {
  account(id: $accountCanonicalId) {
    id
    financials {
      current {
        id
        tradingBalanceViewV2 {
          id
          buyingPower(securityId: $securityId, currency: $currency) { id quantity currency }
          cash(currency: $currency) { id quantity currency }
        }
      }
    }
  }
}
""".strip()

Q_FETCH_SO_ORDERS_EXTENDED_ORDER = """
query FetchSoOrdersExtendedOrder($branchId: String!, $externalId: String!) {
  soOrdersExtendedOrder(branchId: $branchId, externalId: $externalId) {
    averageFilledPrice
    filledQuantity
    firstFilledAtUtc
    lastFilledAtUtc
    limitPrice
    orderType
    rejectionCause
    rejectionCode
    securityCurrency
    status
    stopPrice
    submittedAtUtc
    submittedQuantity
    timeInForce
    accountId
    canonicalAccountId
    cancellationCutoff
    expiredAtUtc
    securityId
  }
}
""".strip()

Q_ORDER_SERVICE_EXTENDED_ORDER_FEED = """
query OrderServiceExtendedOrderFeed($identityId: ID!, $statuses: [OrderServiceOrderStatus!]!, $first: Int = 25, $cursor: String) {
  identity(id: $identityId) {
    id
    orderServiceExtendedOrderFeed(statuses: $statuses, first: $first, after: $cursor) {
      edges {
        cursor
        node {
          id
          orderId
          canonicalAccountId
          createdAtUtc
          status
          side
          executionType
          submittedQuantity
          limitPrice
          stopPrice
          averageFillPrice
          securityCurrency
          securityId
          symbol
          security { id stock { symbol name } }
        }
      }
      pageInfo { endCursor hasNextPage }
    }
  }
}
""".strip()

Q_SO_ORDERS_ORDER_CANCEL = """
mutation SoOrdersOrderCancel($cancelOrderRequest: CancelOrderRequest!) {
  orderServiceCancelOrder(cancelOrderRequest: $cancelOrderRequest) {
    externalId
    errors { code message }
  }
}
""".strip()

Q_SO_ORDERS_ORDER_MODIFY = """
mutation SoOrdersOrderModify($input: SoOrders_ModifyOrderInput!) {
  soOrdersModifyOrder(input: $input) {
    errors { code message }
  }
}
""".strip()

Q_SO_ORDERS_ORDER_CREATE = """
mutation SoOrdersOrderCreate($input: SoOrders_CreateOrderInput!) {
  soOrdersCreateOrder(input: $input) {
    errors { code message }
    order { orderId createdAt }
  }
}
""".strip()


# Versions are GitHub releases tagged vMAJOR.MINOR.PATCH. APP_VERSION is bumped in
# the commit that a release is cut from; once a day the app asks GitHub for the
# latest release and shows an update link when that tag is newer than this copy.
# Commits without a release never trigger it.
APP_VERSION = "1.24.0"
REPO = "ProfessorBagholder/Bagholder"
REPO_URL = "https://github.com/" + REPO
RELEASE_URL = "https://api.github.com/repos/" + REPO + "/releases/latest"
RELEASE_TAG_URL = "https://api.github.com/repos/" + REPO + "/releases/tags/%s"
# In-app update: the running server is a child of a small supervisor (see
# supervise). An update swaps the files, then the child exits with RESTART_CODE
# and the supervisor starts it again in the same console, on the same port.
RESTART_CODE = 3
APP_DIR = Path(__file__).resolve().parent
UPDATE_HEALTHY_SEC = 20          # a restarted server alive this long is a good update
UPDATE_MAX_BYTES = 50 * 1024 * 1024
UPDATE_CHECK_HOURS = 1   # a release is a click away now, so the check is hourly and at every start
# A copy in a container (the Dockerfile): bound to every interface of the container while
# compose publishes it on the host's loopback only, and never installing a release into
# itself, since a new release is a new image; it still checks for one and says so in the
# header, where the pull command stands in for the Update button. Both empty on a desktop.
BIND_HOST = (os.environ.get("BAGHOLDER_BIND") or "").strip() or "127.0.0.1"
UPDATES_OFF = bool((os.environ.get("BAGHOLDER_NO_UPDATE") or "").strip())
UPDATES_OFF_MESSAGE = "This copy is updated with docker compose pull; a new release is a new image."
IMAGE_PAGE = REPO_URL + "/pkgs/container/bagholder"   # where a container copy's header sends the user for a new release
# The sign-in window inside the page (a container, where the user sees no window): Chromium
# runs on the container's virtual display, the page shows its frames and sends it clicks and keys.
LOGIN_VIEW = bool((os.environ.get("BAGHOLDER_LOGIN_VIEW") or "").strip())
# Orders reach Wealthsimple. BAGHOLDER_DRY_ORDERS=1 is for development: a submitted
# ticket is then recorded and printed with the exact request that would have been sent.
ORDERS_LIVE = (os.environ.get("BAGHOLDER_DRY_ORDERS") or "").strip() != "1"
LOGIN_VIEW_SIZE = (960, 1000)

# Bumped whenever the page and the server change together. The page compares it
# with what /api/status reports and tells the user to restart when they differ.
PROTOCOL = "2026-09-11.9"
STARTED_AT = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

Q_FETCH_ACCOUNT_MARGIN_BUYING_POWER = """
query FetchAccountCurrentMarginBuyingPowerV2($accountId: ID!, $currency: Currency = CAD) {
  account(id: $accountId) {
    id
    financials {
      current {
        id
        marginV3 {
          trading {
            buyingPower(asCurrency: $currency) {
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
""".strip()

QUERIES = {
    "FetchAccountCurrentMarginBuyingPowerV2": Q_FETCH_ACCOUNT_MARGIN_BUYING_POWER,
    "FetchSecurities": Q_FETCH_SECURITIES,
    "IdentityHistoricalFinancialsQuery": Q_IDENTITY_HISTORICAL_FINANCIALS,
    "FetchAccountHistoricalFinancials": Q_FETCH_ACCOUNT_HISTORICAL_FINANCIALS,
    "FetchAllAccountFinancials": Q_FETCH_ALL_ACCOUNT_FINANCIALS,
    "FetchActivityFeedItems": Q_FETCH_ACTIVITY_FEED_ITEMS,
    "FetchAccountsWithBalance": Q_FETCH_ACCOUNTS_WITH_BALANCE,
    "FetchSecuritySearchResult": Q_FETCH_SECURITY_SEARCH_RESULT,
    "FetchSecurity": Q_FETCH_SECURITY,
    "FetchSecuritiesSummary": Q_FETCH_SECURITIES_SUMMARY,
    "FetchSecurityMarketData": Q_FETCH_SECURITY_MARKET_DATA,
    "FetchTradingBalanceBuyingPower": Q_FETCH_TRADING_BALANCE_BUYING_POWER,
    "SoOrdersOrderCreate": Q_SO_ORDERS_ORDER_CREATE,
    "FetchSoOrdersExtendedOrder": Q_FETCH_SO_ORDERS_EXTENDED_ORDER,
    "OrderServiceExtendedOrderFeed": Q_ORDER_SERVICE_EXTENDED_ORDER_FEED,
    "SoOrdersOrderCancel": Q_SO_ORDERS_ORDER_CANCEL,
    "SoOrdersOrderModify": Q_SO_ORDERS_ORDER_MODIFY,
}

SKIP_TYPE_MARKERS = (
    "SHARE_LENDING",
    "SHARELENDING",
    "STOCK_LENDING",
    "STOCKLENDING",
)


def _num(v, default=0.0):
    if v is None or v == "":
        return default
    try:
        return float(v)
    except (TypeError, ValueError):
        return default


def _s(v):
    if v is None:
        return ""
    return str(v)


def _upper(v):
    return _s(v).strip().upper()


def _compact(v):
    return re.sub(r"[\s_\-]+", "", _upper(v))


def _date_only(occurred):
    s = _s(occurred).strip()
    if not s:
        return ""
    if "T" in s:
        s = s.split("T", 1)[0]
    return s[:10]


def _asset_symbol(item):
    raw = _s(item.get("assetSymbol")).strip()
    if raw.upper().startswith("EXCHANGE:"):
        raw = raw.split(":", 1)[-1]
    return raw.upper().strip()


# Fills that hit the cash book. Everything else (pending limits, cancelled,
# submitted, working) is not a trade.
_KEEP_STATUS = (
    "POSTED",
    "COMPLETED",
    "SETTLED",
    "COMPLETE",
    "FILLED",
    "EXECUTED",
    "PROCESSED",
    "CONFIRMED",
    "BOOKED",
    "SUCCEEDED",
    "SUCCESS",
)

_CORP_BLOBS = (
    "STKDIS",
    "STOCKDISTRIBUTION",
    "STOCKDIV",
    "SPINOFF",
    "SPIN",
    "DIVIDENDINKIND",
    "INKIND",
    "CORPORATEACTION",
    "CODECHANGE",
    "SYMBOLCHANGE",
    "TICKERCHANGE",
    "LISTINGSTATUS",
    "SECURITYSWAP",
    "MANDATORYEXCHANGE",
    "NAMECHANGE",
)


def _type_blob(item):
    typ = _upper(item.get("type")).replace("-", "_")
    sub = _upper(item.get("subType")).replace("-", "_")
    blob = "_".join(
        _compact(x)
        for x in (
            typ,
            sub,
            item.get("aftTransactionType"),
            item.get("aftTransactionCategory"),
        )
        if x
    )
    return typ, sub, blob


def _is_corp_share_move(item):
    typ, sub, blob = _type_blob(item)
    if any(k in blob for k in _CORP_BLOBS):
        return True
    # In-kind / spin-in: a dividend (or distribution) that delivers shares, not cash.
    qty = abs(_num(item.get("assetQuantity")))
    cash = abs(_num(item.get("amount")))
    if qty and _asset_symbol(item) and cash == 0 and (
        "DIVIDEND" in _compact(typ) or "DISTRIBUT" in blob
    ):
        return True
    return False


def _is_code_change(item):
    _, _, blob = _type_blob(item)
    return any(
        k in blob
        for k in (
            "CODECHANGE",
            "SYMBOLCHANGE",
            "TICKERCHANGE",
            "LISTINGSTATUS",
            "SECURITYSWAP",
            "MANDATORYEXCHANGE",
            "NAMECHANGE",
        )
    )


def set_home(path):
    """Point session + SQLite paths at a directory (used by tests)."""
    global HOME, SESSION_PATH, CLIENT_ID_PATH, UA_PATH, _refused_refresh_token
    HOME = Path(path)
    _refused_refresh_token = None
    SESSION_PATH = HOME / "session.json"
    CLIENT_ID_PATH = HOME / "client_id"
    UA_PATH = HOME / "user_agent"
    store.set_home(HOME)


def skip_activity(item):
    """True when this GraphQL row should not be stored."""
    if not item:
        return True
    if not _s(item.get("occurredAt")).strip():
        return True
    status = _compact(item.get("status"))
    typ, sub, blob = _type_blob(item)
    # Corporate actions often land as processed / empty, not posted.
    if _is_corp_share_move(item):
        if any(x in status for x in ("REJECT", "CANCEL", "FAIL", "VOID")):
            return True
    elif typ in ("DIVIDEND", "INTEREST_CHARGE") and not status:
        # Cash dividends and margin interest charges often arrive with no
        # status at all; both have already hit the cash balance.
        pass
    elif not status or status not in _KEEP_STATUS:
        return True
    # INTEREST / FPL_INTEREST must not be treated as a loan skip.
    if typ in ("LOAN", "RECALL") or sub in ("LOAN", "RECALL"):
        return True
    if typ.endswith("_LOAN") or sub.endswith("_LOAN"):
        return True
    if typ.endswith("_RECALL") or sub.endswith("_RECALL"):
        return True
    for marker in SKIP_TYPE_MARKERS:
        if marker in blob:
            return True
    if "SHARE_LENDING" in blob or "SHARELENDING" in blob:
        return True
    return False


def option_symbol(item):
    """OCC-ish display so an option rolls into the underlying ticker."""
    under = _asset_symbol(item)
    contract = item.get("contractType")
    strike = item.get("strikePrice")
    expiry = item.get("expiryDate")
    if not (contract and strike is not None and expiry and under):
        return under
    ds = _s(expiry).strip()
    if "T" in ds:
        ds = ds.split("T", 1)[0]
    parts = ds.replace("/", "-")[:10].split("-")
    if len(parts) != 3:
        return under
    try:
        year, month, day = int(parts[0]), int(parts[1]), int(parts[2])
        mon = MONTHS[month - 1]
    except (ValueError, IndexError):
        return under
    yy = f"{year % 100:02d}"
    dd = f"{day:02d}"
    try:
        strike_f = float(strike)
    except (TypeError, ValueError):
        return under
    strike_s = f"{strike_f:.2f}"
    cp = _upper(contract)
    if cp in ("C", "CALL"):
        cp = "CALL"
    elif cp in ("P", "PUT"):
        cp = "PUT"
    return f"{under} {dd}{mon}{yy} {strike_s} {cp}"


def signed_cash(item):
    """Ledger cash: buys/withdrawals/source transfers negative; sells/deposits/income positive."""
    amount = abs(_num(item.get("amount")))
    typ = _upper(item.get("type")).replace("-", "_")
    sub = _upper(item.get("subType")).replace("-", "_")
    if typ in ("DIY_BUY", "OPTIONS_BUY", "WITHDRAWAL") or (
        typ == "INTERNAL_TRANSFER" and "SOURCE" in sub
    ):
        return -amount
    if typ in (
        "DIY_SELL",
        "OPTIONS_SELL",
        "DEPOSIT",
        "CONTRIBUTION",
        "DIVIDEND",
        "INTEREST",
    ) or (typ == "INTERNAL_TRANSFER" and "DESTINATION" in sub):
        return amount
    sign = _s(item.get("amountSign")).strip().lower()
    if sign in ("negative", "debit", "-", "neg"):
        return -amount
    if sign in ("positive", "credit", "+", "pos"):
        return amount
    raw = item.get("amount")
    if raw is None or raw == "":
        return 0.0
    return _num(raw)


def _account_type(account_id, accounts):
    if not accounts:
        return ""
    rec = None
    if isinstance(accounts, dict):
        rec = accounts.get(account_id) or accounts.get(_s(account_id))
    else:
        for a in accounts:
            if isinstance(a, dict) and _s(a.get("id")) == _s(account_id):
                rec = a
                break
    if not rec or not isinstance(rec, dict):
        return ""
    return _s(rec.get("nickname") or rec.get("unifiedAccountType") or rec.get("type"))


def nav_account_groups(accounts):
    """Filter nickname -> Wealthsimple account ids. CAD+USD with the same nickname share a group."""
    if isinstance(accounts, dict):
        recs = [v for v in accounts.values() if isinstance(v, dict)]
    else:
        recs = [a for a in (accounts or []) if isinstance(a, dict)]
    groups = {}
    for acc in recs:
        aid = _s(acc.get("id")).strip()
        if not aid:
            continue
        nick = _s(acc.get("nickname") or acc.get("unifiedAccountType") or acc.get("type")).strip()
        if not nick:
            continue
        bucket = groups.setdefault(nick, [])
        if aid not in bucket:
            bucket.append(aid)
    return groups


def fifo_pool_ids(accounts):
    """CAD + USD sides of the same Wealthsimple account share one FIFO book.

    Linked pairs (linkedAccount) and matching nicknames with CAD/USD collapse
    to one root id. Distinct nicknames stay separate.
    """
    recs = []
    if isinstance(accounts, dict):
        recs = [v for v in accounts.values() if isinstance(v, dict)]
    else:
        recs = [a for a in (accounts or []) if isinstance(a, dict)]
    parent = {}

    def find(x):
        x = str(x)
        parent.setdefault(x, x)
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    def union(a, b):
        a, b = _s(a), _s(b)
        if not a or not b:
            return
        ra, rb = find(a), find(b)
        if ra != rb:
            parent[max(ra, rb)] = min(ra, rb)

    by_nick = {}
    for a in recs:
        aid = _s(a.get("id"))
        if not aid:
            continue
        find(aid)
        linked = a.get("linkedAccount") if isinstance(a.get("linkedAccount"), dict) else {}
        lid = _s(linked.get("id"))
        if lid:
            union(aid, lid)
        nick = _s(a.get("nickname")).strip()
        if nick:
            by_nick.setdefault(nick, []).append(aid)
    for ids in by_nick.values():
        root = ids[0]
        for other in ids[1:]:
            union(root, other)
    return {aid: find(aid) for aid in list(parent)}


def _is_option(item):
    return bool(item.get("contractType"))


def _is_to_close(sub):
    c = _compact(sub)
    return "TOCLOSE" in c or c in ("BTC", "STC", "BUYTOCLOSE", "SELLTOCLOSE")


def _human_desc(item, typ, sub, symbol, qty, px, cash):
    t = _upper(typ).replace("-", "_")
    s = _upper(sub).replace("-", "_")
    if t == "DIY_BUY" or (t == "TRADE" and _compact(sub) in ("BUY", "BUYTOOPEN", "BUYTOCLOSE")):
        verb = "Buy to close" if "CLOSE" in _compact(sub) else ("Buy to open" if "OPEN" in _compact(sub) else "Buy")
        if qty and px:
            return f"{verb} {qty:g} {symbol} @ {px:g}"
        return f"{verb} {symbol}".strip()
    if t == "DIY_SELL" or (t == "TRADE" and "SELL" in _compact(sub)):
        verb = "Sell to close" if "CLOSE" in _compact(sub) else ("Sell to open" if "OPEN" in _compact(sub) else "Sell")
        if qty and px:
            return f"{verb} {abs(qty):g} {symbol} @ {px:g}"
        return f"{verb} {symbol}".strip()
    if t in ("DEPOSIT", "CONTRIBUTION"):
        return "Deposit"
    if t == "WITHDRAWAL":
        return "Withdrawal"
    if t == "INTERNAL_TRANSFER":
        return "Transfer out" if "SOURCE" in s else "Transfer in"
    if t == "DIVIDEND":
        return f"Dividend: {symbol}" if symbol else "Dividend"
    if t == "INTEREST":
        if "FPL" in s:
            return "Stock Lending Earnings"
        return "Interest"
    if t == "FUNDS_CONVERSION":
        return "Funds conversion"
    if t in ("FEE", "REFUND"):
        return "Fee refund" if t == "REFUND" else "Fee"
    if t in ("STOCK_DISTRIBUTION", "STKDIS", "SPIN", "SPINOFF"):
        return f"Stock distribution: {symbol}" if symbol else "Stock distribution"
    if (
        t in ("EXPIR", "EXPIRY", "EXPIRE", "ASSIGN", "ASSIGNMENT", "EXERCISE")
        or "EXPIR" in t
        or "ASSIGN" in t
        or "EXERCISE" in t
    ):
        label = "Assign" if "ASSIGN" in t else ("Exercise" if "EXERCISE" in t else "Expir")
        return f"{label} {symbol}".strip()
    if symbol:
        return f"{t}: {symbol}"
    return t.replace("_", " ").title() or "Activity"


def _counter_symbol(item):
    raw = _s(item.get("counterAssetSymbol")).strip()
    if raw.upper().startswith("EXCHANGE:"):
        raw = raw.split(":", 1)[-1]
    return raw.upper().strip()


def map_activity_rows(item, accounts=None):
    """One GraphQL row may become two when a code change names a different ticker."""
    if not item:
        return []
    src = _asset_symbol(item)
    dst = _counter_symbol(item)
    qty = abs(_num(item.get("assetQuantity")))
    if src and dst and src != dst and qty and _is_corp_share_move(item):
        cid = _s(item.get("canonicalId")).strip() or "swap"
        outgoing = dict(item)
        outgoing["assetSymbol"] = src
        outgoing["counterAssetSymbol"] = ""
        outgoing["type"] = "STKDIS"
        outgoing["subType"] = "STKDIS"
        outgoing["assetQuantity"] = -qty
        outgoing["amount"] = 0
        outgoing["amountSign"] = "negative"
        outgoing["canonicalId"] = cid + ":out"
        incoming = dict(item)
        incoming["assetSymbol"] = dst
        incoming["counterAssetSymbol"] = ""
        incoming["type"] = "STKDIS"
        incoming["subType"] = "STKDIS"
        incoming["assetQuantity"] = qty
        incoming["amount"] = 0
        incoming["amountSign"] = "positive"
        incoming["canonicalId"] = cid + ":in"
        rows = [map_activity(outgoing, accounts), map_activity(incoming, accounts)]
        return [r for r in rows if r]
    row = map_activity(item, accounts)
    return [row] if row else []


def map_activity(item, accounts=None):
    """GraphQL ActivityFeedItem -> ledger activity object. None if skipped."""
    if skip_activity(item):
        return None
    occurred = _s(item.get("occurredAt")).strip()
    transaction_date = _date_only(occurred)
    if not transaction_date:
        return None
    # Keep Wealthsimple date and time. transactionDate stays the calendar day
    # for FIFO / filters that compare YYYY-MM-DD.
    account_id = _s(item.get("accountId"))
    typ = _upper(item.get("type")).replace("-", "_")
    sub = _upper(item.get("subType")).replace("-", "_")
    qty_raw = _num(item.get("assetQuantity"))
    qty_abs = abs(qty_raw)
    cash = signed_cash(item)
    amount_abs = abs(_num(item.get("amount")))
    fees = abs(_num(item.get("fees")))
    is_opt = _is_option(item)
    symbol = option_symbol(item) if is_opt else _asset_symbol(item)

    cur = _upper(item.get("currency"))
    if cur not in ("CAD", "USD"):
        cur = "USD" if is_opt else "CAD"

    unit_price = 0.0
    if qty_abs:
        # WS option `amount` is full contract cash (per-share × 100 × contracts).
        if is_opt:
            unit_price = amount_abs / (qty_abs * 100.0)
        else:
            unit_price = amount_abs / qty_abs

    activity_type = "Other"
    activity_sub = sub or typ
    category = "other"
    quantity = qty_abs

    if typ == "DIY_BUY":
        category = "trade"
        if is_opt:
            if _is_to_close(sub):
                activity_type, activity_sub = "Trade", "BUYTOCLOSE"
            else:
                activity_type, activity_sub = "Trade", "BUYTOOPEN"
        else:
            activity_type, activity_sub = "Trade", "BUY"
        quantity = abs(qty_abs)
    elif typ == "DIY_SELL":
        category = "trade"
        if is_opt:
            if _is_to_close(sub):
                activity_type, activity_sub = "Trade", "SELLTOCLOSE"
            else:
                activity_type, activity_sub = "Trade", "SELLTOOPEN"
        else:
            activity_type, activity_sub = "Trade", "SELL"
        quantity = -abs(qty_abs)
    elif typ == "OPTIONS_BUY":
        category = "trade"
        if _is_to_close(sub):
            activity_type, activity_sub = "OPTIONS_BUY", "BUYTOCLOSE"
        else:
            activity_type, activity_sub = "OPTIONS_BUY", "BUYTOOPEN"
        quantity = abs(qty_abs)
    elif typ == "OPTIONS_SELL":
        category = "trade"
        if _is_to_close(sub):
            activity_type, activity_sub = "OPTIONS_SELL", "SELLTOCLOSE"
        else:
            activity_type, activity_sub = "OPTIONS_SELL", "SELLTOOPEN"
        quantity = -abs(qty_abs)
    elif "MULTILEG" in typ:
        # WS filled combo / roll legs often have null assetQuantity.
        # Credit is covered-call premium: sell-to-open a short, not close a long.
        # Debit prefers BUYTOCLOSE; FIFO opens LONG if no short exists.
        category = "trade"
        if cash < 0:
            activity_type, activity_sub = "OPTIONS_BUY", "BUYTOCLOSE"
            quantity = abs(qty_abs)
        else:
            activity_type, activity_sub = "OPTIONS_SELL", "SELLTOOPEN"
            quantity = -abs(qty_abs) if qty_abs else 0.0
    elif (
        typ in ("EXPIR", "EXPIRY", "EXPIRE", "ASSIGN", "ASSIGNMENT", "EXERCISE")
        or "EXPIR" in typ
        or "ASSIGN" in typ
        or "EXERCISE" in typ
    ):
        category = "option_event"
        keep = "ASSIGN" if "ASSIGN" in typ else ("EXERCISE" if "EXERCISE" in typ else "EXPIR")
        activity_type = keep
        short_expir = "SHORT_EXPIR" in typ or ("SHORT" in typ and "EXPIR" in typ)
        if "ASSIGN" in typ:
            activity_sub = "BUYTOCLOSE"
        elif short_expir:
            activity_sub = "BUY"
        elif "EXPIR" in typ:
            activity_sub = "SELL"
        elif "COVER" in _compact(sub) or _is_to_close(sub):
            activity_sub = "BUY"
        else:
            activity_sub = "SELL"
        quantity = -abs(qty_abs) if activity_sub == "SELL" else abs(qty_abs)
        # Strike cash on ASSIGN is share delivery, not option buyback.
        if "ASSIGN" in typ or abs(cash) < 1e-12:
            unit_price = 0.0
        if not is_opt:
            symbol = symbol or _asset_symbol(item)
    elif typ in ("DEPOSIT", "CONTRIBUTION"):
        activity_type, activity_sub, category = "Deposit", "deposit", "deposit"
    elif typ == "WITHDRAWAL":
        activity_type, activity_sub, category = "Withdrawal", "withdrawal", "withdrawal"
    elif typ == "INTERNAL_TRANSFER" or _compact(typ) in (
        "TRFIN",
        "TRFOUT",
        "TRANSFERIN",
        "TRANSFEROUT",
        "INTERNALTRANSFER",
    ):
        # Share TRFIN/TRFOUT are custody moves, not sells.
        activity_type, activity_sub, category = "Transfer", "transfer", "transfer"
    elif typ == "DIVIDEND" and not _is_corp_share_move(item):
        activity_type, activity_sub, category = "Dividend", "dividend", "dividend"
    elif typ == "INTEREST" or "FPL_INTEREST" in sub or _compact(typ) == "FPLINTEREST":
        activity_type, activity_sub, category = "Interest", "interest", "interest"
    elif typ == "FUNDS_CONVERSION":
        activity_type, activity_sub, category = "FxExchange", "fx", "fx"
    elif typ in ("FEE", "REFUND"):
        activity_type = "Refund" if typ == "REFUND" else "Fee"
        activity_sub, category = "fee", "fee"
    elif _is_corp_share_move(item) or (
        typ in ("STOCK_DISTRIBUTION", "STKDIS", "SPIN", "SPINOFF", "STK_DIS")
        or "STKDIS" in _compact(typ)
        or "STOCKDISTRIBUTION" in _compact(typ)
        or "STOCKDISTRIBUTION" in _compact(sub)
    ):
        # Name-change is -N then +N of the same ticker.
        # foldStkdis nets SELL against BUY. Unsigned qty would open 2N at $0.
        # Leftover +N opens at $0 so a later sell has lots.
        activity_type, category = "STKDIS", "trade"
        unit_price = 0.0
        sign = _s(item.get("amountSign")).strip().lower()
        # A lone international code change names the old ticker with +qty.
        # Those shares were replaced, not bought.
        outgoing = qty_raw < 0 or sign in ("negative", "debit", "-", "neg")
        if (
            not outgoing
            and _is_code_change(item)
            and not _counter_symbol(item)
            and "STKDIS" not in _compact(item.get("type"))
        ):
            outgoing = True
        if outgoing:
            activity_sub = "SELL"
            quantity = -qty_abs
        else:
            activity_sub = "BUY"
            quantity = qty_abs
    else:
        activity_type = (item.get("type") or "Other")
        activity_sub = item.get("subType") or "other"
        category = "other"

    if activity_sub in ("SELL", "SELLTOOPEN", "SELLTOCLOSE"):
        quantity = -abs(qty_abs) if qty_abs else quantity

    sign = _s(item.get("amountSign")).strip().lower()
    direction = ""
    if sign in ("negative", "debit", "-", "neg") or cash < 0:
        direction = "DEBIT"
    elif sign in ("positive", "credit", "+", "pos") or cash > 0:
        direction = "CREDIT"

    cid = _s(item.get("canonicalId")).strip()
    if store.looks_like_homemade_id(cid):
        cid = ""

    desc = _human_desc(item, typ, sub, symbol, quantity, unit_price, cash)
    name = _s(item.get("aftOriginatorName") or item.get("institutionName") or symbol)

    return {
        "canonicalId": cid or None,
        "occurredAt": occurred,
        "transactionDate": transaction_date,
        "settlementDate": transaction_date,
        "accountId": account_id,
        "bookId": account_id,
        "fifoId": fifo_pool_ids(accounts).get(account_id, account_id) if accounts else account_id,
        "accountType": _account_type(account_id, accounts),
        "activityType": activity_type,
        "activitySubType": activity_sub,
        "description": desc,
        "direction": direction,
        "symbol": symbol,
        "name": name,
        "currency": cur,
        "quantity": quantity,
        "unitPrice": unit_price,
        "commission": fees,
        "netCashAmount": cash,
        "category": category,
        "balance": None,
        "source": "wealthsimple",
        "rawType": _s(item.get("type")),
        "aftType": _s(item.get("aftTransactionType")),
        "counterSymbol": _counter_symbol(item),
        "securityId": _s(item.get("securityId")).strip() or None,
    }


# --- runtime (not executed on import) ---

_lock = threading.RLock()
_state = {
    "connected": False,
    "capturing": False,
    "syncing": False,
    "syncStep": "",
    "email": "",
    "lastSync": "",
    "error": "",
    "chrome_proc": None,
    "login_attempt": 0,
    "updating": "",
    "updateError": "",
    "listingsFilling": False,
}
_stop = threading.Event()
_httpd = None
_exit_code = [0]


def _ensure_home():
    HOME.mkdir(mode=0o700, exist_ok=True)
    try:
        os.chmod(HOME, 0o700)
    except OSError:
        pass


def _atomic_write(path: Path, data: bytes, mode=0o600):
    _ensure_home()
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_bytes(data)
    os.chmod(tmp, mode)
    tmp.replace(path)
    try:
        os.chmod(path, mode)
    except OSError:
        pass


def load_session():
    with _lock:
        if not SESSION_PATH.exists():
            return None
        try:
            return json.loads(SESSION_PATH.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return None


def save_session(sess):
    with _lock:
        _atomic_write(SESSION_PATH, json.dumps(sess, indent=2).encode("utf-8"), 0o600)


def delete_session_and_book():
    """Drop the login session only. Stored activity rows stay in SQLite."""
    global _refused_refresh_token
    _refused_refresh_token = None
    with _lock:
        try:
            SESSION_PATH.unlink()
        except OSError:
            pass
        _state["connected"] = False
        _state["email"] = ""
        _state["lastSync"] = ""
        _state["capturing"] = False
        _state["error"] = ""


def load_book():
    store.ensure()
    return store.snapshot()


def save_accounts_snapshot(book):
    """Write accounts / balances / NAV. Never rebuilds the activity table."""
    store.replace_accounts(book.get("accounts") or [])
    store.replace_balances(book.get("balances") or [])
    if "margin" in book:
        store.replace_margin(book.get("margin") or [])
    store.upsert_nav(book.get("navHistory") or [])
    if book.get("syncedAt"):
        store.set_meta("synced_at", book["syncedAt"])



_SSL_CTX = None

def _ssl_context():
    global _SSL_CTX
    if _SSL_CTX is not None:
        return _SSL_CTX
    ca_files = []
    try:
        import certifi
        where = certifi.where()
        if where:
            ca_files.append(where)
    except Exception:
        pass
    ca_files.extend((
        "/etc/ssl/cert.pem",
        "/etc/ssl/certs/ca-certificates.crt",
        "/opt/homebrew/etc/openssl@3/cert.pem",
        "/usr/local/etc/openssl@3/cert.pem",
        "/opt/homebrew/etc/openssl@1.1/cert.pem",
    ))
    for path in ca_files:
        if path and os.path.isfile(path):
            try:
                _SSL_CTX = ssl.create_default_context(cafile=path)
                return _SSL_CTX
            except Exception:
                continue
    _SSL_CTX = ssl.create_default_context()
    return _SSL_CTX


_GZIP_MAGIC = b"\x1f\x8b"


def _http_body_text(raw, headers=None):
    """Decode HTTP body bytes. Decompress gzip when the payload is still compressed."""
    if not raw:
        return ""
    encoding = ""
    if headers is not None:
        try:
            encoding = (headers.get("Content-Encoding") or "").strip().lower()
        except Exception:
            encoding = ""
    gzip_magic = raw.startswith(_GZIP_MAGIC)
    # urllib may leave gzip bodies compressed. Magic bytes are the reliable check;
    # Content-Encoding alone is not enough, and must not cause a second decompress.
    if gzip_magic or (encoding == "gzip" and gzip_magic):
        try:
            raw = gzip.decompress(raw)
        except OSError:
            pass
    return raw.decode("utf-8", "replace")


def _http_json(method, url, body=None, headers=None, timeout=60):
    hdrs = {
        "Accept": "application/json",
    }
    ua = cached_user_agent()
    if ua:
        hdrs["User-Agent"] = ua
    if headers:
        hdrs.update(headers)
    data = None
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        hdrs["Content-Type"] = "application/json"
    req = Request(url, data=data, headers=hdrs, method=method)
    try:
        with urlopen(req, timeout=timeout, context=_ssl_context()) as resp:
            raw = resp.read()
            text = _http_body_text(raw, getattr(resp, "headers", None))
            if not text:
                return {}
            try:
                return json.loads(text)
            except ValueError:
                return {
                    "error": "invalid_json",
                    "_http_status": getattr(resp, "status", None) or 200,
                }
    except HTTPError as e:
        raw = e.read() if e.fp else b""
        text = _http_body_text(raw, getattr(e, "headers", None))
        parsed = None
        try:
            parsed = json.loads(text) if text else {}
        except ValueError:
            parsed = {"error": "http_%s" % e.code}
        parsed = parsed or {}
        parsed["_http_status"] = e.code
        return parsed


def cached_client_id():
    if CLIENT_ID_PATH.exists():
        try:
            v = CLIENT_ID_PATH.read_text(encoding="utf-8").strip()
            if v:
                return v
        except OSError:
            pass
    return ""


def save_client_id(cid):
    if not cid:
        return
    _ensure_home()
    try:
        CLIENT_ID_PATH.write_text(cid, encoding="utf-8")
        os.chmod(CLIENT_ID_PATH, 0o600)
    except OSError:
        pass


def cached_user_agent():
    sess = load_session() or {}
    v = _s(sess.get("user_agent")).strip()
    if v:
        return v
    if UA_PATH.exists():
        try:
            v = UA_PATH.read_text(encoding="utf-8").strip()
            if v:
                return v
        except OSError:
            pass
    return ""


def save_user_agent(ua):
    if not ua:
        return
    _ensure_home()
    try:
        UA_PATH.write_text(ua, encoding="utf-8")
        os.chmod(UA_PATH, 0o600)
    except OSError:
        pass


def scrape_client_id():
    """Production clientId from Wealthsimple login JS. Empty if it cannot be read."""
    cached = cached_client_id()
    if cached:
        return cached
    try:
        hdrs = {}
        ua = cached_user_agent()
        if ua:
            hdrs["User-Agent"] = ua
        req = Request(LOGIN_URL, headers=hdrs)
        with urlopen(req, timeout=20, context=_ssl_context()) as resp:
            html = _http_body_text(resp.read(), getattr(resp, "headers", None))
        m = re.search(r'<script[^>]+src="([^"]*app-[a-f0-9]+\.js[^"]*)"', html, re.I)
        if not m:
            return ""
        js_url = m.group(1)
        if js_url.startswith("//"):
            js_url = "https:" + js_url
        elif js_url.startswith("/"):
            js_url = "https://my.wealthsimple.com" + js_url
        req2 = Request(js_url, headers=hdrs)
        with urlopen(req2, timeout=20, context=_ssl_context()) as resp:
            js = _http_body_text(resp.read(), getattr(resp, "headers", None))
        m2 = re.search(r'production:.*?clientId:"([a-f0-9]+)"', js, re.S)
        if m2:
            save_client_id(m2.group(1))
            return m2.group(1)
    except Exception:
        pass
    return ""


def client_id_for(sess):
    """Session or cached file only. Do not scrape at refresh time."""
    if sess and sess.get("client_id"):
        save_client_id(sess["client_id"])
        return sess["client_id"]
    return cached_client_id()


def _ws_session_headers(sess, headers):
    headers = dict(headers)
    if sess and sess.get("wssdi"):
        headers["x-ws-device-id"] = sess["wssdi"]
    if sess and sess.get("session_id"):
        headers["x-ws-session-id"] = sess["session_id"]
    return headers


def _set_public_error(msg):
    with _lock:
        _state["error"] = msg or ""


def _oauth_error_code(data):
    """Short OAuth `error` field only. Never tokens, client_id values, or raw bodies."""
    err = (data or {}).get("error")
    if not isinstance(err, str):
        return ""
    err = err.strip()
    if not err:
        return ""
    if re.fullmatch(r"[a-f0-9]{32,}", err, re.I):
        return ""
    if not re.fullmatch(r"[A-Za-z0-9_.-]{1,64}", err):
        return ""
    return _public_sync_error(err)


REFUSED_LOGIN_MESSAGE = "Saved login refused. Connect Wealthsimple again."


def _refresh_failure_message(data):
    status = (data or {}).get("_http_status")
    oauth_err = _oauth_error_code(data)
    if oauth_err == "invalid_grant":
        return REFUSED_LOGIN_MESSAGE
    parts = []
    if status:
        parts.append("Wealthsimple token refresh HTTP %s" % status)
    if oauth_err:
        parts.append(oauth_err)
    return " ".join(parts) if parts else "Wealthsimple token refresh failed"


def _expires_at_as_timestamp(data):
    """Store Wealthsimple expiry as the same timestamp string the cookie uses."""
    raw = (data or {}).get("expires_at")
    if isinstance(raw, str) and "T" in raw.strip():
        return raw.strip()
    unix = None
    if isinstance(raw, (int, float)):
        unix = float(raw)
    elif data and data.get("expires_in") is not None:
        unix = time.time() + int(data["expires_in"])
    if unix is None:
        return None
    dt = datetime.fromtimestamp(unix, timezone.utc)
    return dt.strftime("%Y-%m-%dT%H:%M:%S.000Z")


# Background refreshes: one of a kind at a time.
#
# A page open, a filter change, a poll and a background loop can all ask for the
# same refresh within a second of each other. Each used to start its own thread,
# and every thread built its own copy of the model and then dropped the cached
# one, so the next request rebuilt it and started another thread. On a machine
# where a source answers slowly the threads outlived the requests that made
# them and piled up until nothing was left for the page.
_jobs_lock = threading.Lock()
_jobs = {}


def _job(name):
    return _jobs.setdefault(name, {"running": False, "until": 0.0})


def single_flight(name, busy=0):
    """Run the wrapped function only when no other call to it is in flight.
    A call that finds one running returns `busy` at once instead of queueing."""
    def wrap(fn):
        def inner(*args, **kwargs):
            with _jobs_lock:
                job = _job(name)
                if job["running"]:
                    return busy
                job["running"] = True
            try:
                return fn(*args, **kwargs)
            finally:
                with _jobs_lock:
                    _job(name)["running"] = False
                    _job(name)["until"] = time.time() + _COOLDOWN.get(name, 0)
        inner.__name__ = getattr(fn, "__name__", name)
        inner.__doc__ = fn.__doc__
        return inner
    return wrap


# How long a kind of refresh waits after one finishes before a request may ask
# for it again. The background loops keep their own clocks; this only stops the
# request path from asking a source that is slow, rate-limited or down on every
# page the user opens.
_COOLDOWN = {"quotes": 60.0, "market": 300.0}


def kick(name, fn):
    """Start a background refresh unless one is running or its cooldown holds.
    True when a thread was started."""
    with _jobs_lock:
        job = _job(name)
        if job["running"] or time.time() < job["until"]:
            return False
    threading.Thread(target=fn, name="bagholder-" + name, daemon=True).start()
    return True


_refresh_lock = threading.Lock()
_refused_refresh_token = None  # a token Wealthsimple answered invalid_grant to; never posted again this run


def refresh_session(sess, adopt=True):
    """refresh_token grant. Do not send Authorization.

    Wealthsimple rotates the refresh token on every grant, so two threads
    posting the same token would leave the loser with invalid_grant and the
    login dead. Refreshes run one at a time; under the lock the session on
    disk is read again and, when another thread has rotated it meanwhile,
    that session is adopted and nothing is posted. A caller holding a login
    newer than the file (a capture) passes adopt=False."""
    rt = (sess or {}).get("refresh_token")
    if not rt:
        _set_public_error("missing refresh token")
        return False
    with _refresh_lock:
        if adopt:
            current = load_session() or {}
            if current.get("access_token") and current.get("refresh_token") and current.get("refresh_token") != rt:
                sess.update(current)
                return True
        if rt == _refused_refresh_token:
            _set_public_error(REFUSED_LOGIN_MESSAGE)
            return False
        return _refresh_session_locked(sess, rt)


def _refresh_session_locked(sess, rt):
    cid = client_id_for(sess)
    if not cid:
        _set_public_error("session has no client id")
        return False
    body = {
        "grant_type": "refresh_token",
        "refresh_token": rt,
        "client_id": cid,
    }
    headers = _ws_session_headers(
        sess,
        {
            "x-wealthsimple-client": WS_CLIENT,
            "x-ws-profile": "invest",
        },
    )
    data = _http_json("POST", OAUTH + "/token", body, headers)
    if not data or not data.get("access_token"):
        if _oauth_error_code(data) == "invalid_grant":
            global _refused_refresh_token
            _refused_refresh_token = rt
        _set_public_error(_refresh_failure_message(data))
        return False
    sess["access_token"] = data["access_token"]
    if data.get("refresh_token"):
        sess["refresh_token"] = data["refresh_token"]
    stamped = _expires_at_as_timestamp(data)
    if stamped:
        sess["expires_at"] = stamped
    sess["client_id"] = cid
    save_session(sess)
    return True


def token_info(sess):
    token = (sess or {}).get("access_token")
    if not token:
        return {}
    headers = _ws_session_headers(
        sess,
        {
            "Authorization": "Bearer " + token,
            "x-wealthsimple-client": WS_CLIENT,
        },
    )
    data = _http_json("GET", OAUTH + "/token/info", None, headers)
    if data and data.get("_http_status") in (401, 403):
        return {}
    return data or {}


def client_id_from_token_info(info):
    """OAuth application uid that issued these tokens. Unofficial token/info shape."""
    if not isinstance(info, dict):
        return ""
    uid = info.get("application_uid")
    if uid:
        return str(uid).strip()
    app = info.get("application")
    if isinstance(app, dict) and app.get("uid"):
        return str(app.get("uid")).strip()
    return ""


def apply_token_info_client_id(sess, info=None):
    """Write token/info application uid to the session and CLIENT_ID_PATH. Do not scrape."""
    if not sess or not sess.get("access_token"):
        return ""
    if info is None:
        try:
            info = token_info(sess) or {}
        except Exception:
            info = {}
    cid = client_id_from_token_info(info)
    if not cid:
        return ""
    sess["client_id"] = cid
    save_client_id(cid)
    return cid


IDENTITY_KEYS = (
    "identity_canonical_id",
    "identityCanonicalId",
    "canonical_id",
    "identity_id",
    "resource_owner_id",
    "sub",
)


def _identity_from(obj):
    if not isinstance(obj, dict):
        return ""
    for k in IDENTITY_KEYS:
        v = obj.get(k)
        if v:
            return str(v)
    return ""


def graphql(sess, operation, variables, query=None):
    token = sess.get("access_token") or ""
    headers = {
        "Authorization": "Bearer " + token,
        "x-wealthsimple-client": WS_CLIENT,
        "x-ws-profile": "trade",
        "x-ws-api-version": GRAPHQL_VERSION,
        "x-ws-locale": "en-CA",
        "x-platform-os": "web",
        "Content-Type": "application/json",
        "Origin": "https://my.wealthsimple.com",
        "Referer": "https://my.wealthsimple.com/app/trade",
    }
    if sess.get("wssdi"):
        headers["x-ws-device-id"] = sess["wssdi"]
    if sess.get("session_id"):
        headers["x-ws-session-id"] = sess["session_id"]
    body = {
        "operationName": operation,
        "query": query if query is not None else QUERIES[operation],
        "variables": {k: v for k, v in variables.items() if v is not None},
    }
    data = _http_json("POST", GRAPHQL, body, headers, timeout=90)
    if data and data.get("_http_status") in (401, 403):
        raise PermissionError("not authorized")
    errs = (data or {}).get("errors")
    if errs:
        first = errs[0] if isinstance(errs, list) else errs
        if isinstance(first, dict):
            emsg = first.get("message") or first.get("error") or str(first)
        else:
            emsg = str(first)
        raise RuntimeError(str(operation) + ": " + str(emsg))
    if not data or data.get("data") is None:
        raise RuntimeError("graphql failed: " + operation)
    return data["data"]



def _money_amount(node, *keys):
    """First present Money.amount from netLiquidationValue / V2 (or deposits)."""
    if not isinstance(node, dict):
        return None, None
    for key in keys:
        money = node.get(key)
        if not isinstance(money, dict) or money.get("amount") is None:
            continue
        try:
            return float(money["amount"]), money.get("currency") or "CAD"
        except (TypeError, ValueError):
            continue
    return None, None


def _nav_points_from_payload(data):
    points = []
    blob = data or {}
    ident = blob.get("identity") or {}
    acc = blob.get("account") or {}
    fin = ident.get("financials") if ident.get("financials") is not None else acc.get("financials")
    hist = ((fin or {}).get("historicalDaily") or {})
    for edge in hist.get("edges") or []:
        node = (edge or {}).get("node") or {}
        amt, cur = _money_amount(node, "netLiquidationValue", "netLiquidationValueV2")
        d = (node.get("date") or "")[:10]
        if not d or amt is None:
            continue
        rec = {"date": d, "equity": amt, "currency": cur or "CAD"}
        nd_amt, _nd_cur = _money_amount(node, "netDeposits", "netDepositsV2")
        if nd_amt is not None:
            rec["netDeposits"] = nd_amt
        points.append(rec)
    page = hist.get("pageInfo") or {}
    return points, page


def _paginate_nav_history(sess, operation, extra_variables, query=None, since_date=None):
    today = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    since = _s(since_date)[:10]
    if since and since > today:
        return []
    year0 = int(since[:4]) if since else 2020
    year1 = int(today[:4])
    points = []
    for year in range(year0, year1 + 1):
        start = f"{year}-01-01"
        if since and start < since:
            start = since
        end = today if year == year1 else f"{year}-12-31"
        if start > end:
            continue
        cursor = None
        for _ in range(8):
            variables = dict(extra_variables)
            variables["startDate"] = start
            variables["endDate"] = end
            variables["cursor"] = cursor
            data = graphql(sess, operation, variables, query=query)
            chunk, page = _nav_points_from_payload(data)
            points.extend(chunk)
            if not page.get("hasNextPage"):
                break
            cursor = page.get("endCursor")
            if not cursor:
                break
    by_date = {}
    for rec in points:
        by_date[rec["date"]] = rec
    return [by_date[d] for d in sorted(by_date)]


def fetch_nav_history(sess, identity_id, since_date=None):
    """Identity-wide Wealthsimple net liquidation (All / accountId '')."""
    return _paginate_nav_history(
        sess,
        "IdentityHistoricalFinancialsQuery",
        {
            "identityId": identity_id,
            "currency": "CAD",
            "limit": 400,
            "includeNetDeposits": True,
        },
        query=Q_IDENTITY_HISTORICAL_FINANCIALS,
        since_date=since_date,
    )


def fetch_account_nav_history(sess, account_id, since_date=None):
    """Daily NAV for one Wealthsimple account via FetchAccountHistoricalFinancials."""
    aid = _s(account_id).strip()
    if not aid:
        return []
    return _paginate_nav_history(
        sess,
        "FetchAccountHistoricalFinancials",
        {
            "id": aid,
            "currency": "CAD",
            "resolution": "DAILY",
            "first": 400,
        },
        query=Q_FETCH_ACCOUNT_HISTORICAL_FINANCIALS,
        since_date=since_date,
    )


def merge_nav_points(series_list):
    """Sum equity (and netDeposits when present) by date across account series."""
    by_date = {}
    for series in series_list or []:
        for rec in series or []:
            if not isinstance(rec, dict):
                continue
            d = _s(rec.get("date"))[:10]
            if not d:
                continue
            equity = rec.get("equity")
            if equity is None:
                continue
            try:
                eq = float(equity)
            except (TypeError, ValueError):
                continue
            cur = by_date.get(d)
            if cur is None:
                cur = {
                    "date": d,
                    "equity": 0.0,
                    "currency": _s(rec.get("currency") or "CAD") or "CAD",
                }
                by_date[d] = cur
            cur["equity"] += eq
            if rec.get("currency"):
                cur["currency"] = _s(rec.get("currency")) or cur["currency"]
            nd = rec.get("netDeposits")
            if nd is not None:
                try:
                    cur["netDeposits"] = cur.get("netDeposits", 0.0) + float(nd)
                except (TypeError, ValueError):
                    pass
    return [by_date[d] for d in sorted(by_date)]


def fetch_nickname_nav_history(sess, accounts):
    """Per-filter-nickname daily NAV. Returns (points, public_errors)."""
    points = []
    errors = []
    last_by = store.nav_last_dates()
    for nick, ids in sorted(nav_account_groups(accounts).items()):
        _set_sync_step("Fetching equity history for %s…" % nick)
        try:
            since = last_by.get(nick)
            series = [fetch_account_nav_history(sess, aid, since_date=since) for aid in ids]
            pts = merge_nav_points(series)
        except Exception as e:
            public = _public_sync_error(e)
            errors.append("%s: %s" % (nick, public))
            sys.stderr.write("NAV history failed for %s: %s\n" % (nick, public))
            continue
        for rec in pts:
            tagged = dict(rec)
            tagged["accountId"] = nick
            points.append(tagged)
    return points, errors


def refresh_nav_only(allow_refresh=True):
    """Pull daily NAV only. Does not pull activity."""
    store.ensure()
    sess = load_session()
    if not sess or not sess.get("access_token"):
        return {"ok": False, "error": "not connected"}
    identity = _identity_from(sess)
    if not identity:
        try:
            info = token_info(sess)
        except Exception:
            info = {}
        identity = _identity_from(info or {})
    if not identity:
        return {"ok": False, "error": "no identity"}
    accounts = (load_book() or {}).get("accounts") or []
    if not accounts:
        return {"ok": False, "error": "no accounts stored"}
    try:
        last_by = store.nav_last_dates()
        try:
            nav_history = fetch_nav_history(sess, identity, since_date=last_by.get(""))
        except PermissionError:
            raise
        except Exception:
            nav_history = []
        combined = []
        for rec in nav_history:
            tagged = dict(rec)
            tagged["accountId"] = ""
            combined.append(tagged)
        nickname_pts, nav_errors = fetch_nickname_nav_history(sess, accounts)
        combined.extend(nickname_pts)
        store.upsert_nav(combined)
        nicks = sorted({_s(p.get("accountId")) for p in combined if _s(p.get("accountId"))})
        return {
            "ok": True,
            "allDays": sum(1 for p in combined if not _s(p.get("accountId"))),
            "accounts": len(nicks),
            "errors": nav_errors,
        }
    except PermissionError:
        if allow_refresh and refresh_session(load_session() or {}):
            return refresh_nav_only(allow_refresh=False)
        return {"ok": False, "error": "Session expired. Connect again."}


def fetch_all_accounts(sess, identity_id):
    accounts = []
    cursor = None
    while True:
        variables = {
            "identityId": identity_id,
            "pageSize": 25,
            "startDate": "2015-01-01",
            "cursor": cursor,
        }
        data = graphql(sess, "FetchAllAccountFinancials", variables)
        ident = (data or {}).get("identity") or {}
        conn = ident.get("accounts") or {}
        for edge in conn.get("edges") or []:
            node = (edge or {}).get("node")
            if node:
                accounts.append(node)
        page = conn.get("pageInfo") or {}
        if not page.get("hasNextPage"):
            break
        cursor = page.get("endCursor")
        if not cursor:
            break
    return accounts


def activity_fetch_condition(account_id, start_date=None, now=None):
    """Wealthsimple ActivityCondition. startDate bounds a daily pull to new rows."""
    end = (now or datetime.now(timezone.utc)) + timedelta(days=1)
    cond = {
        "endDate": end.strftime("%Y-%m-%dT%H:%M:%S.999Z"),
        "accountIds": [account_id],
    }
    if start_date:
        raw = _s(start_date).strip()
        if raw:
            if "T" not in raw:
                raw = raw[:10] + "T00:00:00.000Z"
            cond["startDate"] = raw
    return cond


def fetch_activities_for_account(
    sess, account_id, start_date=None, known_canonical_ids=None
):
    items = []
    cursor = None
    while True:
        variables = {
            "first": 100,
            "orderBy": "OCCURRED_AT_DESC",
            "condition": activity_fetch_condition(account_id, start_date=start_date),
        }
        if cursor:
            variables["cursor"] = cursor
        data = graphql(sess, "FetchActivityFeedItems", variables)
        feed = (data or {}).get("activityFeedItems") or {}
        for edge in feed.get("edges") or []:
            node = (edge or {}).get("node")
            if node:
                items.append(node)
        page = feed.get("pageInfo") or {}
        # Walk every page Wealthsimple returns for the window: a page of known
        # rows can still carry a revised one, and the rows behind it are new.
        if not page.get("hasNextPage"):
            break
        cursor = page.get("endCursor")
        if not cursor:
            break
    return items


def fetch_balances(sess, account_ids):
    balances = []
    ids = [i for i in account_ids if i]
    for i in range(0, len(ids), 20):
        chunk = ids[i : i + 20]
        data = graphql(
            sess,
            "FetchAccountsWithBalance",
            {"ids": chunk, "type": "TRADING"},
        )
        for acc in data.get("accounts") or []:
            aid = acc.get("id")
            for ca in acc.get("custodianAccounts") or []:
                fin = ca.get("financials") or {}
                bals = fin.get("balance") or []
                if isinstance(bals, dict):
                    bals = [bals]
                for b in bals:
                    balances.append(
                        {
                            "accountId": aid,
                            "custodianAccountId": ca.get("id"),
                            "securityId": b.get("securityId"),
                            "quantity": b.get("quantity"),
                        }
                    )
    return balances


def parse_margin(data):
    """Wealthsimple's buying power for one account, as it answers: a Money when
    available, the reason when not, None when the account has no margin figures."""
    try:
        trading = (((((data or {}).get("account") or {}).get("financials") or {}).get("current") or {}).get("marginV3") or {}).get("trading") or {}
    except AttributeError:
        return None
    bp = trading.get("buyingPower")
    if not isinstance(bp, dict):
        return None
    if bp.get("__typename") == "BuyingPowerMetricAvailable":
        total = bp.get("total") or {}
        amount = _num(total.get("amount"), None)
        if amount is None:
            return None
        return {"buyingPower": amount, "currency": _s(total.get("currency")) or "CAD", "unavailable": ""}
    reason = (bp.get("reason") or {})
    why = _s(reason.get("__typename")) or _s(bp.get("__typename")) or "unavailable"
    n = len(reason.get("securities") or []) if isinstance(reason.get("securities"), list) else 0
    if n:
        why += " (%d securities)" % n
    return {"buyingPower": None, "currency": "CAD", "unavailable": why}


def margin_account_ids(accounts):
    """The open margin accounts: the only ones whose buying power is margin available.
    Wealthsimple answers the buying-power query for every self-directed account with
    the cash it could buy with, and with an error for cash, card and crypto accounts;
    neither is margin."""
    out = []
    for a in accounts or []:
        typ = _s(a.get("unifiedAccountType") or a.get("unified_account_type")).upper()
        status = _s(a.get("status")).lower()
        if a.get("id") and "MARGIN" in typ and status != "closed":
            out.append(a.get("id"))
    return out


def fetch_margin(sess, account_ids):
    """One buying-power request per margin account (margin_account_ids); only accounts
    that answer are rows. A request that fails is reported once on the terminal, not hidden."""
    rows = []
    now = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    failed = 0
    first_error = ""
    for aid in account_ids or []:
        if not aid:
            continue
        try:
            data = graphql(sess, "FetchAccountCurrentMarginBuyingPowerV2", {"accountId": aid, "currency": "CAD"})
        except Exception as e:
            failed += 1
            if not first_error:
                first_error = e.__class__.__name__ + (": " + str(e) if str(e) else "")
            continue
        parsed = parse_margin(data)
        if parsed is None:
            continue
        parsed["accountId"] = aid
        parsed["fetchedAt"] = now
        rows.append(parsed)
    if failed:
        sys.stderr.write("bagholder portfolio: buying power request failed for %d of %d accounts (%s)\n" % (failed, len([a for a in account_ids or [] if a]), first_error))
    return rows


PORTFOLIO_REFRESH_MINUTES = 5


def refresh_portfolio():
    """Net liquidation values, cash balances and buying power, read again between
    syncs so the Portfolio tiles move with the day. Never raises; says what it
    did on the terminal, since a tile showing a dash must be explainable."""
    with _lock:
        if _state["syncing"]:
            return {"ok": False, "skipped": "sync running"}
        if not _state["connected"]:
            return {"ok": False, "skipped": "not connected"}
    try:
        sess = load_session()
        identity = _identity_from(sess or {})
        if not sess or not sess.get("access_token") or not identity:
            sys.stderr.write("bagholder portfolio: no session to read with\n")
            return {"ok": False, "skipped": "no session"}
        accounts = fetch_all_accounts(sess, identity)
        ids = [a.get("id") for a in accounts if a.get("id")]
        if not ids:
            sys.stderr.write("bagholder portfolio: Wealthsimple returned no accounts\n")
            return {"ok": False, "skipped": "no accounts"}
        balances = fetch_balances(sess, ids)
        margin = fetch_margin(sess, margin_account_ids(accounts))
        store.replace_accounts(slim_accounts(accounts))
        store.replace_balances(balances)
        store.replace_margin(margin)
        store.set_meta("balances_read_at", datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))
        model.invalidate()
        available = sum(1 for m in margin if m.get("buyingPower") is not None)
        sys.stderr.write("bagholder portfolio: %d accounts, %d balances, buying power for %d of %d margin accounts\n" % (len(ids), len(balances), available, len(margin)))
        return {"ok": True, "accounts": len(ids), "balances": len(balances), "margin": len(margin)}
    except Exception as e:
        sys.stderr.write("bagholder portfolio: failed: %s\n" % (e.__class__.__name__ + (": " + str(e) if str(e) else "")))
        return {"ok": False, "skipped": "error"}


EXPOSURE_CHECK_SEC = 30 * 60
EXPOSURE_FIRST_SEC = 20


EXPOSURE_WORKERS = 4


def refresh_exposures():
    """The exposure record of every held security that has none or an old one, read
    from records outside Wealthsimple (exposure.py): shares first, four securities at
    a time (the pause between requests is per site, and the families are independent),
    each record shown as soon as it lands. Never raises."""
    from concurrent.futures import ThreadPoolExecutor, as_completed
    snap = store.snapshot()
    secs = {sec["id"]: sec for sec in (snap.get("securities") or []) if sec.get("id")}
    positions = model.base_model().get("positions") or []
    held = {_s(p.get("securityId")) for p in positions if p.get("kind") == "Shares"}
    held |= {_s(b.get("securityId")) for b in (snap.get("balances") or []) if (_num(b.get("quantity"), 0.0) or 0.0) > 0 and _s(b.get("securityId")).startswith("sec-s-")}
    held = sorted(sid for sid in held if sid and not sid.startswith("sec-c-"))
    todo = [sid for sid in exposure.stale(held) if sid in secs]
    todo.sort(key=lambda sid: 1 if exposure.is_fund(secs[sid].get("name"), secs[sid].get("symbol")) else 0)
    # a contract's exposure is its underlying's: the share is classified under its own key
    unders = sorted({(_s(p.get("underlying")).upper(), _s(p.get("currency"))) for p in positions if p.get("kind") == "Options" and p.get("underlying")})
    unders = [(u, c) for u, c in unders if exposure.stale([exposure.SHARE_KEY + u + ":" + (market.tmx_form("", c) or "")])]
    # a watched listing is classified like an underlying: a share under its own key
    watched = [(w["symbol"], w.get("exchange") or "", w.get("currency") or "") for w in model.base_model().get("watchlist") or [] if not instruments.find(w["symbol"], w.get("exchange")) and _s(w.get("exchange")).upper() != "CRYPTO"]
    watched = [(s, e, c) for s, e, c in watched if exposure.stale([model.watch_exposure_key(s, e, c)])]

    def one(job):
        if _stop.is_set():
            return None
        if job[0] == "sec":
            sec = job[1]
            rec = exposure.refresh_security(sec)
            cov = rec.get("coverage") or 0.0
            return "bagholder exposure: %s %s: %s (%d%% covered)%s" % (sec.get("symbol"), "fund" if exposure.is_fund(sec.get("name"), sec.get("symbol")) else "share",
                                                                      rec.get("source") or "no source", int(round(cov * 100)), (": " + rec["error"]) if rec.get("error") else "")
        if job[0] == "watch":
            sym, ex, ccy = job[1], job[2], job[3]
            try:
                exposure.share_exposure(sym, ex, ccy)
                return "bagholder exposure: %s (watched) classified" % sym
            except Exception as e:
                return "bagholder exposure: %s (watched): %s" % (sym, e)
        under, ccy = job[1], job[2]
        try:
            exposure.share_exposure(under, "", ccy)
            return "bagholder exposure: %s (an option's underlying) classified" % under
        except Exception as e:
            return "bagholder exposure: %s (an option's underlying): %s" % (under, str(e) or e.__class__.__name__)

    jobs = [("sec", secs[sid]) for sid in todo] + [("under", u, c) for u, c in unders] + [("watch", s, e, c) for s, e, c in watched]
    done = 0
    if jobs:
        with ThreadPoolExecutor(max_workers=EXPOSURE_WORKERS, thread_name_prefix="bagholder-exposure") as pool:
            for fut in as_completed([pool.submit(one, j) for j in jobs]):
                line = fut.result()
                if line:
                    done += 1
                    sys.stderr.write(line + "\n")
                    model.invalidate()   # each record shows as soon as it lands: the data version moves with the table
    return {"ok": True, "held": len(held), "refreshed": done}


def exposure_loop():
    """Soon after start and every half hour: the held securities' exposure records."""
    wait = EXPOSURE_FIRST_SEC
    while not _stop.wait(wait):
        wait = EXPOSURE_CHECK_SEC
        try:
            refresh_exposures()
        except Exception as e:
            sys.stderr.write("bagholder exposure: refresh failed: %s\n" % (str(e) or e.__class__.__name__))


def portfolio_loop():
    """The Portfolio figures Wealthsimple states: once at start, then every
    PORTFOLIO_REFRESH_MINUTES while connected. The first read does not wait,
    so the tiles are filled by the time the page is up."""
    refresh_portfolio()
    while not _stop.wait(60 * PORTFOLIO_REFRESH_MINUTES):
        refresh_portfolio()


def fetch_security(sess, security_id):
    sid = _s(security_id).strip()
    if not sid:
        return None
    try:
        data = graphql(sess, "FetchSecurity", {"securityId": sid})
    except Exception:
        return None
    return _security_record((data or {}).get("security"), sid)


def fetch_securities(sess, security_ids):
    """One request per SECURITY_BATCH ids. Unknown ids come back null and are
    dropped; a failed batch falls back to one request per id."""
    ids = []
    seen = set()
    for raw in security_ids or []:
        sid = _s(raw).strip()
        if sid and sid not in seen:
            seen.add(sid)
            ids.append(sid)
    out = []
    for i in range(0, len(ids), SECURITY_BATCH):
        chunk = ids[i : i + SECURITY_BATCH]
        try:
            data = graphql(sess, "FetchSecurities", {"ids": chunk})
            rows = (data or {}).get("securities")
            if not isinstance(rows, list):
                raise RuntimeError("no securities list")
        except Exception:
            for sid in chunk:
                rec = fetch_security(sess, sid)
                if rec:
                    out.append(rec)
            continue
        for sec in rows:
            rec = _security_record(sec, "")
            if rec:
                out.append(rec)
    return out


def _security_record(sec, sid):
    if not isinstance(sec, dict) or not sec:
        return None
    stock = sec.get("stock") or {}
    if not isinstance(stock, dict):
        stock = {}
    option = sec.get("optionDetails") or {}
    if not isinstance(option, dict):
        option = {}
    under = option.get("underlyingSecurity") or {}
    if not isinstance(under, dict):
        under = {}
    under_id = _s(under.get("id")).strip() or None
    return {
        "id": _s(sec.get("id")).strip() or sid,
        "symbol": _s(stock.get("symbol")).strip(),
        "name": _s(stock.get("name")).strip(),
        "primaryExchange": _s(stock.get("primaryExchange")).strip(),
        "primaryMic": _s(stock.get("primaryMic")).strip(),
        "currency": _s(sec.get("currency")).strip(),
        "underlyingId": under_id,
    }


def _collect_security_ids():
    ids = []
    seen = set()
    snap = store.snapshot()
    for a in snap.get("activities") or []:
        sid = _s(a.get("securityId")).strip()
        if sid and sid not in seen:
            seen.add(sid)
            ids.append(sid)
    for b in snap.get("balances") or []:
        sid = _s(b.get("securityId")).strip()
        if sid and sid not in seen:
            seen.add(sid)
            ids.append(sid)
    return ids


def _account_ids_for_backfill():
    snap = store.snapshot()
    ids = []
    seen = set()
    for acc in snap.get("accounts") or []:
        aid = _s(acc.get("id")).strip()
        if aid and aid not in seen:
            seen.add(aid)
            ids.append(aid)
    if ids:
        return ids
    for a in snap.get("activities") or []:
        aid = _s(a.get("accountId")).strip()
        if aid and aid not in seen:
            seen.add(aid)
            ids.append(aid)
    return ids


def fill_listings(sess, from_sync=False):
    """Stamp missing activity security_id values and cache FetchSecurity listings."""
    store.ensure()
    if not sess or not sess.get("access_token"):
        return False
    with _lock:
        if _state.get("listingsFilling"):
            return False
        if _state.get("syncing") and not from_sync:
            return False
        _state["listingsFilling"] = True
        _state["syncStep"] = "Attaching listing ids…"
    try:
        if store.needs_security_id_backfill():
            walk_ok = True
            known = store.canonical_ids()
            mapped = []
            _set_sync_step("Attaching listing ids…")
            for aid in _account_ids_for_backfill():
                try:
                    raw_items = fetch_activities_for_account(
                        sess,
                        aid,
                        start_date=None,
                        known_canonical_ids=known,
                    )
                except Exception:
                    walk_ok = False
                    continue
                acc_by_id = {
                    a.get("id"): a
                    for a in (store.snapshot().get("accounts") or [])
                    if a.get("id")
                }
                for it in raw_items:
                    mapped.extend(map_activity_rows(it, acc_by_id))
            if mapped:
                store.apply_wealthsimple_mapped(mapped)
            if walk_ok:
                store.set_meta("security_id_backfill_done", "1")
        wanted = _collect_security_ids()
        pending = store.missing_security_ids(wanted)
        seen = set()
        to_upsert = []
        # Options point at an underlying security; fetch those in a second round.
        while pending:
            _set_sync_step("Looking up company names, %s left" % len(pending))
            batch = [sid for sid in pending if sid not in seen]
            seen.update(batch)
            pending = []
            if not batch:
                break
            recs = fetch_securities(sess, batch)
            to_upsert.extend(recs)
            under = [_s(r.get("underlyingId")).strip() for r in recs]
            under = [u for u in under if u and u not in seen]
            if under:
                pending = store.missing_security_ids(under)
        if to_upsert:
            store.upsert_securities(to_upsert)
        return True
    except Exception:
        return False
    finally:
        with _lock:
            _state["listingsFilling"] = False
            _state["syncStep"] = ""


def margin_boost_target(acc):
    """The custodian account id a Margin Boost feature points at: an account
    Wealthsimple lets back a margin account as collateral carries the feature
    MARGIN_BOOST, and its metadata names the margin account's custodian account."""
    for f in (acc.get("accountFeatures") or []) if isinstance(acc, dict) else []:
        if not isinstance(f, dict) or _s(f.get("name")).upper() != "MARGIN_BOOST" or not f.get("enabled") or f.get("functional") is False:
            continue
        md = f.get("metadata") if isinstance(f.get("metadata"), dict) else {}
        return _s(md.get("targetMarginAccountId"))
    return ""


def slim_accounts(accounts):
    """The stored shape of every account, each collateral account naming the margin
    account it backs (the custodian id in its feature resolved to the account id)."""
    custodian = {}
    for a in accounts or []:
        if not isinstance(a, dict):
            continue
        for c in a.get("custodianAccounts") or []:
            if isinstance(c, dict) and _s(c.get("id")):
                custodian[_s(c.get("id"))] = _s(a.get("id"))
    out = []
    for a in accounts or []:
        if not isinstance(a, dict):
            continue
        row = slim_account(a)
        target = margin_boost_target(a)
        row["marginAccountId"] = custodian.get(target, "") if target else ""
        out.append(row)
    return out


def slim_account(acc):
    nlv = None
    try:
        nlv = (
            (((acc.get("financials") or {}).get("currentCombined") or {}).get("netLiquidationValue") or {}).get("amount")
        )
    except Exception:
        nlv = None
    return {
        "id": acc.get("id"),
        "nickname": acc.get("nickname") or "",
        "unifiedAccountType": acc.get("unifiedAccountType") or "",
        "currency": acc.get("currency") or "",
        "status": acc.get("status") or "",
        "type": acc.get("type") or "",
        "netLiquidationValue": nlv,
    }


def _public_sync_error(exc):
    msg = str(exc or "").replace("\n", " ").strip()
    if "CERTIFICATE_VERIFY_FAILED" in msg or "unable to get local issuer certificate" in msg:
        return "could not verify HTTPS certificates"
    msg = re.sub(r"(?i)bearer\s+\S+", "[redacted]", msg)
    msg = re.sub(r"(?i)(access_token|refresh_token)\s*[:=]\s*\S+", r"\1=[redacted]", msg)
    if "bearer" in msg.lower() or "access_token" in msg.lower() or "refresh_token" in msg.lower():
        msg = re.sub(r"(?i)(bearer|access_token|refresh_token)", "[redacted]", msg)
    msg = re.sub(r"\s+", " ", msg).strip()
    if len(msg) > 180:
        msg = msg[:177] + "..."
    return msg or "unknown error"


def _set_sync_step(msg):
    with _lock:
        _state["syncStep"] = msg or ""


def _expires_at_unix(sess):
    raw = (sess or {}).get("expires_at")
    if raw is None or raw == "":
        return None
    if isinstance(raw, (int, float)):
        return float(raw)
    s = str(raw).strip()
    try:
        return float(s)
    except ValueError:
        pass
    try:
        if s.endswith("Z"):
            s = s[:-1] + "+00:00"
        dt = datetime.fromisoformat(s)
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return dt.timestamp()
    except ValueError:
        return None


def token_refresh_needed(sess, now=None):
    exp = _expires_at_unix(sess)
    if exp is None:
        return True
    now = time.time() if now is None else float(now)
    return now >= exp - TOKEN_REFRESH_MARGIN_SEC


def ensure_fresh_token(sess=None):
    """Refresh the access token before expires_at. Token POST only.

    Connected means this POST produced a new access token. A failed POST
    leaves session.json on disk and marks connected False.
    """
    sess = sess if sess is not None else load_session()
    if not sess or not sess.get("refresh_token"):
        with _lock:
            _state["connected"] = False
            _state["error"] = "missing refresh token"
        return False
    with _lock:
        connected = bool(_state.get("connected"))
    if connected and not token_refresh_needed(sess):
        return True
    ok = refresh_session(sess)
    with _lock:
        _state["connected"] = bool(ok)
        if ok:
            _state["error"] = ""
        elif not (_state.get("error") or "").strip():
            _state["error"] = "Wealthsimple token refresh failed"
    return ok


def activity_sync_bounds():
    """Full history only when the activity table has no rows yet."""
    if store.activity_count() == 0:
        return {"start_date": None, "full_history": True}
    start = store.incremental_start_date() or None
    return {"start_date": start, "full_history": False}


def run_sync(allow_refresh=True, force_activity=True):
    """GraphQL pull. Inserts new Wealthsimple rows only. Never rebuilds the table."""
    store.ensure()
    with _lock:
        if _state["syncing"]:
            return False
        _state["syncing"] = True
        _state["error"] = ""
        _state["syncStep"] = "Checking session…"
    try:
        sess = load_session()
        if not sess or not (sess.get("access_token") or sess.get("refresh_token")):
            with _lock:
                _state["connected"] = False
            return False
        info = {}
        if not sess or not sess.get("access_token"):
            with _lock:
                _state["connected"] = False
            return False
        identity = _identity_from(sess) or _identity_from(info or {})
        if not identity:
            info = info or token_info(sess)
            identity = _identity_from(info or {})
        if not identity:
            raise RuntimeError("no identity_canonical_id")
        sess["identity_canonical_id"] = identity
        email = (info or {}).get("email") or (info or {}).get("username") or sess.get("email") or ""
        if email:
            sess["email"] = email
        save_session(sess)

        if not force_activity and not store.activity_pull_due(interval_sec=ACTIVITY_PULL_SEC):
            with _lock:
                _state["connected"] = True
                _state["email"] = email
                _state["lastSync"] = store.get_meta("synced_at") or _state["lastSync"]
                _state["capturing"] = False
                _state["error"] = ""
                _state["syncStep"] = ""
            return True

        _set_sync_step("Fetching accounts…")
        accounts = fetch_all_accounts(sess, identity)
        acc_by_id = {a.get("id"): a for a in accounts if a.get("id")}
        bounds = activity_sync_bounds()
        start_date = bounds["start_date"]
        known = store.canonical_ids() if not bounds["full_history"] else set()
        mapped = []
        with_ids = [a for a in accounts if a.get("id")]
        _set_sync_step("Syncing transactions")
        for acc in with_ids:
            aid = acc.get("id")
            raw_items = fetch_activities_for_account(
                sess,
                aid,
                start_date=start_date,
                known_canonical_ids=known,
            )
            for it in raw_items:
                mapped.extend(map_activity_rows(it, acc_by_id))
        pools = fifo_pool_ids(accounts)
        for row in mapped:
            aid = row.get("accountId") or ""
            row["fifoId"] = pools.get(aid, aid)
        _set_sync_step("Fetching balances…")
        balances = fetch_balances(sess, list(acc_by_id.keys()))
        margin = fetch_margin(sess, margin_account_ids(accounts))
        _set_sync_step("Fetching equity history…")
        last_by = store.nav_last_dates()
        try:
            nav_history = fetch_nav_history(sess, identity, since_date=last_by.get(""))
        except Exception:
            nav_history = []
        combined = []
        for rec in nav_history:
            tagged = dict(rec)
            tagged["accountId"] = ""
            combined.append(tagged)
        nickname_pts, nav_errors = fetch_nickname_nav_history(sess, accounts)
        combined.extend(nickname_pts)
        store.apply_wealthsimple_mapped(mapped)
        synced = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        _set_sync_step("Saving…")
        save_accounts_snapshot(
            {
                "accounts": slim_accounts(accounts),
                "balances": balances,
                "margin": margin,
                "navHistory": combined,
                "syncedAt": synced,
            }
        )
        store.mark_activity_pulled(synced)
        fill_listings(sess, from_sync=True)
        nav_err_line = ""
        if nav_errors:
            nav_err_line = "NAV history failed for " + "; ".join(nav_errors)
        with _lock:
            _state["connected"] = True
            _state["email"] = email
            _state["lastSync"] = synced
            _state["capturing"] = False
            _state["error"] = nav_err_line
            _state["syncStep"] = ""
        return True
    except PermissionError:
        if allow_refresh and refresh_session(load_session() or {}):
            with _lock:
                _state["syncing"] = False
            return run_sync(allow_refresh=False, force_activity=force_activity)
        with _lock:
            _state["connected"] = False
            _state["error"] = "Session expired. Connect again."
        return False
    except Exception as e:
        line = "Sync failed: " + _public_sync_error(e)
        sys.stderr.write(line + "\n")
        with _lock:
            _state["error"] = line
        return False
    finally:
        with _lock:
            _state["syncing"] = False
            _state["syncStep"] = ""


def boot_session():
    store.ensure()
    sess = load_session()
    if not sess:
        with _lock:
            _state["connected"] = False
            snap = store.snapshot()
            _state["lastSync"] = snap.get("syncedAt") or ""
        return
    info = {}
    info_ok = False
    if sess.get("access_token"):
        try:
            info = token_info(sess) or {}
        except Exception:
            info = {}
        info_ok = bool(info) and not info.get("error") and not info.get("_http_status")
        if info_ok:
            apply_token_info_client_id(sess, info)
            if info.get("identity_canonical_id") and not sess.get("identity_canonical_id"):
                sess["identity_canonical_id"] = info["identity_canonical_id"]
            if info.get("email"):
                sess["email"] = info["email"]
            save_session(sess)
    ok = bool(info_ok)
    if not ok and sess.get("refresh_token"):
        ok = refresh_session(sess)
        sess = load_session() or sess
    with _lock:
        _state["connected"] = bool(ok)
        if ok:
            _state["error"] = ""
        elif not (_state.get("error") or "").strip():
            if not (sess or {}).get("refresh_token"):
                _state["error"] = "missing refresh token"
            else:
                _state["error"] = "Wealthsimple token refresh failed"
        _state["email"] = (sess or {}).get("email") or ""
        book = load_book()
        _state["lastSync"] = book.get("syncedAt") or ""


def find_chrome():
    """Find a Chromium-family browser capable of the DevTools login flow.

    The name predates support for other compatible browsers; keep it for the
    callers and integrations that already use it.
    """
    explicit = (os.environ.get("BAGHOLDER_CHROME") or "").strip()
    if explicit and os.path.isfile(explicit):
        return explicit
    if sys.platform == "darwin":
        for p in (
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ):
            if os.path.isfile(p):
                return p
    names = [
        "google-chrome",
        "google-chrome-stable",
        "brave-browser",
        "brave-browser-stable",
        "brave",
        "chromium",
        "chromium-browser",
        "microsoft-edge",
        "msedge",
        "chrome",
    ]
    for n in names:
        p = shutil.which(n)
        if p:
            return p
    extras = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ]
    pf = os.environ.get("PROGRAMFILES", r"C:\Program Files")
    pf86 = os.environ.get("PROGRAMFILES(X86)", r"C:\Program Files (x86)")
    local = os.environ.get("LOCALAPPDATA", "")
    extras.extend(
        [
            os.path.join(pf, "Google", "Chrome", "Application", "chrome.exe"),
            os.path.join(pf86, "Google", "Chrome", "Application", "chrome.exe"),
            os.path.join(local, "Google", "Chrome", "Application", "chrome.exe"),
            os.path.join(pf, "BraveSoftware", "Brave-Browser", "Application", "brave.exe"),
            os.path.join(pf86, "BraveSoftware", "Brave-Browser", "Application", "brave.exe"),
            os.path.join(local, "BraveSoftware", "Brave-Browser", "Application", "brave.exe"),
            os.path.join(pf, "Microsoft", "Edge", "Application", "msedge.exe"),
            os.path.join(pf86, "Microsoft", "Edge", "Application", "msedge.exe"),
        ]
    )
    for p in extras:
        if p and os.path.isfile(p):
            return p
    return ""


def _cdp_list(port, timeout=1):
    for path in ("/json/list", "/json"):
        try:
            url = "http://127.0.0.1:%s%s" % (port, path)
            req = Request(url, headers={"Host": "127.0.0.1:%s" % port})
            with urlopen(req, timeout=timeout) as resp:
                raw = resp.read()
            if not raw:
                continue
            data = json.loads(raw.decode("utf-8"))
            if isinstance(data, list):
                return data
        except Exception:
            continue
    return []


def _mask_ws(data, key):
    data = data if isinstance(data, (bytes, bytearray)) else bytes(data)
    key = bytes(key)
    out = bytearray(len(data))
    for i, b in enumerate(data):
        out[i] = b ^ key[i % 4]
    return bytes(out)


class _MiniWS:
    """RFC6455 client: mask outgoing frames, answer ping with pong."""

    def __init__(self, sock):
        self.sock = sock
        self._buf = b""
        self._next_id = 1

    def close(self):
        try:
            self.send_frame(0x8, b"")
        except Exception:
            pass
        try:
            self.sock.close()
        except OSError:
            pass

    def send_frame(self, opcode, payload=b""):
        if isinstance(payload, str):
            payload = payload.encode("utf-8")
        payload = payload or b""
        mask_key = os.urandom(4)
        masked = _mask_ws(payload, mask_key)
        n = len(payload)
        header = bytearray()
        header.append(0x80 | (opcode & 0x0F))
        if n < 126:
            header.append(0x80 | n)
        elif n < 65536:
            header.append(0x80 | 126)
            header.extend(struct.pack("!H", n))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack("!Q", n))
        header.extend(mask_key)
        self.sock.sendall(header + masked)

    def send_text(self, text):
        if not isinstance(text, str):
            text = text.decode("utf-8") if isinstance(text, (bytes, bytearray)) else str(text)
        self.send_frame(0x1, text.encode("utf-8"))

    def _recv_exact(self, n, deadline):
        if n <= 0:
            return b""
        while len(self._buf) < n:
            remain = deadline - time.time()
            if remain <= 0:
                raise TimeoutError("ws read timeout")
            self.sock.settimeout(max(0.05, remain))
            chunk = self.sock.recv(65536)
            if not chunk:
                raise OSError("ws closed")
            self._buf += chunk
        data = self._buf[:n]
        self._buf = self._buf[n:]
        return data

    def _read_frame(self, deadline):
        b1 = self._recv_exact(1, deadline)[0]
        b2 = self._recv_exact(1, deadline)[0]
        fin = (b1 & 0x80) != 0
        opcode = b1 & 0x0F
        masked = (b2 & 0x80) != 0
        length = b2 & 0x7F
        if length == 126:
            length = struct.unpack("!H", self._recv_exact(2, deadline))[0]
        elif length == 127:
            length = struct.unpack("!Q", self._recv_exact(8, deadline))[0]
        mask_key = self._recv_exact(4, deadline) if masked else None
        payload = self._recv_exact(length, deadline) if length else b""
        if mask_key:
            payload = _mask_ws(payload, mask_key)
        return fin, opcode, payload

    def recv_message(self, timeout=8):
        deadline = time.time() + timeout
        fragments = []
        started = None
        while True:
            remain = deadline - time.time()
            if remain <= 0:
                raise TimeoutError("ws read timeout")
            fin, opcode, payload = self._read_frame(deadline)
            if opcode == 0x8:
                raise OSError("ws closed")
            if opcode == 0x9:
                self.send_frame(0xA, payload)
                continue
            if opcode == 0xA:
                continue
            if opcode in (0x1, 0x2):
                started = opcode
                fragments = [payload]
                if fin:
                    return started, b"".join(fragments)
                continue
            if opcode == 0x0:
                fragments.append(payload)
                if fin:
                    return started or 0x1, b"".join(fragments)
                continue


def _ws_connect(ws_url, timeout=5):
    parsed = urlparse(ws_url)
    host = "127.0.0.1"
    port = parsed.port or (443 if parsed.scheme == "wss" else 80)
    path = parsed.path or "/"
    if parsed.query:
        path = path + "?" + parsed.query
    sock = socket.create_connection((host, port), timeout=timeout)
    try:
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        req = (
            "GET %s HTTP/1.1\r\n"
            "Host: %s:%s\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            "Sec-WebSocket-Key: %s\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "Origin: http://127.0.0.1\r\n"
            "\r\n"
        ) % (path, host, port, key)
        sock.sendall(req.encode("ascii"))
        buf = b""
        deadline = time.time() + timeout
        while b"\r\n\r\n" not in buf:
            remain = deadline - time.time()
            if remain <= 0:
                raise TimeoutError("ws handshake timeout")
            sock.settimeout(max(0.05, remain))
            chunk = sock.recv(4096)
            if not chunk:
                raise OSError("ws handshake closed")
            buf += chunk
        header, rest = buf.split(b"\r\n\r\n", 1)
        status_line = header.split(b"\r\n", 1)[0]
        if b"101" not in status_line:
            raise OSError("ws handshake failed: %s" % status_line.decode("latin1", "replace"))
        ws = _MiniWS(sock)
        ws._buf = rest
        sock = None
        return ws
    finally:
        if sock is not None:
            try:
                sock.close()
            except OSError:
                pass


def _cdp_call(ws, method, params=None, timeout=8):
    msg_id = ws._next_id
    ws._next_id = msg_id + 1
    payload = {"id": msg_id, "method": method}
    if params is not None:
        payload["params"] = params
    ws.send_text(json.dumps(payload))
    deadline = time.time() + timeout
    while time.time() < deadline:
        remain = deadline - time.time()
        try:
            opcode, data = ws.recv_message(timeout=max(0.2, remain))
        except Exception:
            continue
        if opcode not in (0x1, 0x2):
            continue
        try:
            msg = json.loads(data.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            continue
        if msg.get("id") == msg_id:
            return msg
    return None


def _json_with_access_token(raw):
    if raw is None:
        return None
    s = raw if isinstance(raw, str) else str(raw)
    if not s:
        return None
    cur = s.strip()
    for _ in range(3):
        if "access_token" in cur:
            try:
                obj = json.loads(cur)
            except ValueError:
                obj = None
            if isinstance(obj, dict) and obj.get("access_token"):
                return obj
        nxt = unquote(cur)
        if nxt == cur:
            break
        cur = nxt
    return None

def _cookies_from_document_cookie(text):
    cookies = []
    if not text:
        return cookies
    for part in text.split(";"):
        part = part.strip()
        if not part or "=" not in part:
            continue
        name, value = part.split("=", 1)
        cookies.append({"name": name.strip(), "value": value})
    return cookies

def _tokens_from_cookie_list(cookies):
    if not cookies:
        return None
    body = {}
    oauth = None
    wssdi = ""
    for c in cookies:
        if not isinstance(c, dict):
            continue
        name = c.get("name") or ""
        value = c.get("value") or ""
        if name == DEVICE_COOKIE and value:
            wssdi = value
        parsed = _json_with_access_token(value)
        if parsed:
            if name == OAUTH_COOKIE or oauth is None:
                oauth = parsed
    if not oauth or not oauth.get("access_token"):
        return None
    for k in ("access_token", "refresh_token", "identity_canonical_id", "client_id", "session_id"):
        if oauth.get(k):
            body[k] = oauth[k]
    ident = _identity_from(oauth)
    if ident:
        body["identity_canonical_id"] = ident
    if oauth.get("expires_at") is not None:
        body["expires_at"] = oauth["expires_at"]
    if wssdi:
        body["wssdi"] = wssdi
    return body

def _cdp_cookie_list(msg):
    if msg and isinstance(msg.get("result"), dict):
        return msg["result"].get("cookies") or []
    return []


CAPTURE_CALL_SEC = 2   # each DevTools call while capturing: short, so a closed window is noticed quickly
WINDOW_CHECK_SEC = 0.5   # how often the watcher looks for the window
CAPTURE_EVERY_SEC = 1.5  # how often it tries to capture the session


def _cdp_cookies_from_target(ws_url):
    ws = None
    try:
        ws = _ws_connect(ws_url, timeout=CAPTURE_CALL_SEC)
        ua = ""
        ver = _cdp_call(ws, "Browser.getVersion", timeout=CAPTURE_CALL_SEC)
        if ver and isinstance(ver.get("result"), dict):
            ua = _s(ver["result"].get("userAgent")).strip()
        if ua:
            save_user_agent(ua)
        _cdp_call(ws, "Network.enable", timeout=CAPTURE_CALL_SEC)
        cookies = _cdp_cookie_list(_cdp_call(ws, "Network.getAllCookies", timeout=CAPTURE_CALL_SEC))
        body = _tokens_from_cookie_list(cookies)
        if body:
            if ua:
                body["user_agent"] = ua
            return body
        extra = _cdp_cookie_list(_cdp_call(ws, "Storage.getCookies", timeout=CAPTURE_CALL_SEC))
        if extra:
            cookies = list(cookies) + list(extra)
        body = _tokens_from_cookie_list(cookies)
        if body:
            if ua:
                body["user_agent"] = ua
            return body
        ev = _cdp_call(
            ws,
            "Runtime.evaluate",
            {"expression": "document.cookie", "returnByValue": True},
            timeout=CAPTURE_CALL_SEC,
        )
        val = ""
        if ev:
            res = (ev.get("result") or {}).get("result") or {}
            val = res.get("value") or ""
        body = _tokens_from_cookie_list(_cookies_from_document_cookie(val))
        if body and ua:
            body["user_agent"] = ua
        return body
    except Exception:
        return None
    finally:
        if ws is not None:
            ws.close()


def _try_capture_from_cdp(port):
    targets = _cdp_list(port)
    pages = []
    others = []
    for t in targets:
        if not isinstance(t, dict) or not t.get("webSocketDebuggerUrl"):
            continue
        if t.get("type") == "page":
            pages.append(t)
        else:
            others.append(t)
    for t in pages + others:
        try:
            body = _cdp_cookies_from_target(t["webSocketDebuggerUrl"])
        except Exception:
            continue
        if body and body.get("access_token"):
            return body
    return None


def _cdp_pages(port):
    """The open windows and tabs of the app's login Chrome. A Chrome left running
    with no window still answers on the debug port with background targets; only
    page targets mean a window is up."""
    return [t for t in _cdp_list(port, timeout=WINDOW_CHECK_SEC) if isinstance(t, dict) and t.get("type") == "page" and t.get("id")]


def _attempt_is(attempt):
    with _lock:
        return attempt is None or _state.get("login_attempt") == attempt


def _capture_loop(proc, debug_port, attempt):
    """Try to capture the session every CAPTURE_EVERY_SEC while the attempt is
    live. Runs beside the window watcher so a capture call stuck on a window
    that just closed never delays noticing the close. A captured refresh token
    Wealthsimple refused is not posted again; the loop waits for the browser
    to hold a different one."""
    refused = None
    while _attempt_is(attempt):
        with _lock:
            if not _state.get("capturing"):
                return
        body = None
        try:
            if _cdp_pages(debug_port):
                body = _try_capture_from_cdp(debug_port)
        except Exception:
            body = None
        if body and body.get("access_token") and _attempt_is(attempt) and body.get("refresh_token") != refused:
            with _lock:
                if not _state.get("capturing"):
                    return
            if capture_tokens(body).get("ok"):
                sys.stderr.write("bagholder captured Wealthsimple session\n")
                _close_login_browser(proc)
                return
            refused = body.get("refresh_token")
            sys.stderr.write("bagholder login: Wealthsimple refused the captured session on refresh; still watching the window\n")
        time.sleep(CAPTURE_EVERY_SEC)


def _poll_chrome_session(proc, debug_port, attempt=None):
    """Watch one login attempt's window: every WINDOW_CHECK_SEC, is it still
    there? The capture runs in its own thread (_capture_loop). A watcher belongs
    to the attempt it was started for; once a later Connect has started
    another, it exits without touching anything."""
    deadline = time.time() + CAPTURE_WAIT_SEC
    start = time.time()
    seen_page = False
    threading.Thread(target=_capture_loop, args=(proc, debug_port, attempt), name="bagholder-cdp-capture", daemon=True).start()
    while time.time() < deadline:
        if not _attempt_is(attempt):
            return
        with _lock:
            still = bool(_state.get("capturing"))
        if not still:
            return
        try:
            pages = _cdp_pages(debug_port) if proc.poll() is None else []
        except Exception:
            pages = []
        seen_page = seen_page or bool(pages)
        gone = proc.poll() is not None or (not pages and (seen_page or time.time() - start > 10))
        if gone:
            # the window is gone (Chrome quit, or the window closed with Chrome
            # lingering without one): the attempt is over, nothing is relaunched
            if not _attempt_is(attempt):
                return
            with _lock:
                if _state.get("capturing"):
                    _state["error"] = (
                        "The Chrome window closed before a session showed up. Choose Connect Wealthsimple to try again."
                    )
                    _state["capturing"] = False
            sys.stderr.write("bagholder login: window closed, waiting stopped\n")
            _close_login_browser(proc)
            return
        time.sleep(WINDOW_CHECK_SEC)
    if not _attempt_is(attempt):
        return
    with _lock:
        if _state.get("capturing"):
            _state["error"] = (
                "No session yet. Finish login in the Chrome window, then wait a few seconds."
            )
            _state["capturing"] = False
    _close_login_browser(proc)


def _login_browser_ws():
    """DevTools browser endpoint of the Chrome the app launched, or None."""
    port = DEBUG_PORTS[0]
    try:
        req = Request("http://127.0.0.1:%s/json/version" % port, headers={"Host": "127.0.0.1:%s" % port})
        with urlopen(req, timeout=2) as resp:
            v = json.loads(resp.read().decode("utf-8"))
        return v.get("webSocketDebuggerUrl") or None
    except Exception:
        return None


def _close_login_browser(only=None):
    """Close the Chrome the app launched for login: gracefully through DevTools,
    then by ending the process if it lingers. Only ever the app's own instance,
    never the user's Chrome. With `only`, a watcher closes just the instance it
    was watching, never a later attempt's window."""
    with _lock:
        proc = _state.get("chrome_proc")
        if only is not None and proc is not only:
            proc = only
            current = False
        else:
            current = True
        if current:
            _state["chrome_proc"] = None
    if proc is None:
        return
    if not current:
        # an older instance: it is no longer on the debug port, just end it if it lingers
        try:
            proc.wait(timeout=0.1)
        except Exception:
            try:
                proc.terminate()
            except Exception:
                pass
        return
    ws_url = _login_browser_ws()
    if ws_url:
        try:
            ws = _ws_connect(ws_url)
            try:
                _cdp_call(ws, "Browser.close")
            finally:
                ws.close()
        except Exception:
            pass
    try:
        proc.wait(timeout=5)
    except Exception:
        try:
            proc.terminate()
        except Exception:
            pass


def _login_browser_alive():
    """True only while the app's login Chrome has a window up."""
    with _lock:
        proc = _state.get("chrome_proc")
    if proc is None or proc.poll() is not None or not _login_browser_ws():
        return False
    try:
        return bool(_cdp_pages(DEBUG_PORTS[0]))
    except Exception:
        return False


_view_lock = threading.Lock()
_view = {"ws": None, "target": ""}


def _login_view_drop():
    with _view_lock:
        ws, _view["ws"], _view["target"] = _view["ws"], None, ""
    if ws is not None:
        try:
            ws.close()
        except Exception:
            pass


def _login_view_ws():
    """A DevTools socket to the login window's page, kept between calls; None without a window."""
    pages = _cdp_pages(DEBUG_PORTS[0])
    if not pages:
        _login_view_drop()
        return None
    page = pages[0]
    with _view_lock:
        if _view["ws"] is not None and _view["target"] == page.get("id"):
            return _view["ws"]
    _login_view_drop()
    ws = _ws_connect(page["webSocketDebuggerUrl"], timeout=CAPTURE_CALL_SEC)
    with _view_lock:
        _view["ws"], _view["target"] = ws, page.get("id")
    return ws


def login_frame():
    """The login window as a JPEG, or None when the app is not waiting for a login or has no window."""
    with _lock:
        capturing = bool(_state.get("capturing"))
    if not capturing:
        return None
    try:
        ws = _login_view_ws()
        if ws is None:
            return None
        r = _cdp_call(ws, "Page.captureScreenshot", {"format": "jpeg", "quality": 60}, timeout=CAPTURE_CALL_SEC)
        data = ((r or {}).get("result") or {}).get("data")
        return base64.b64decode(data) if data else None
    except Exception:
        _login_view_drop()
        return None


_cast = {"frame": None, "seq": 0, "cond": threading.Condition()}


def _screencast_loop(attempt):
    """Chromium pushes the login window's frames as they change (Page.startScreencast)
    on a socket of its own; the latest frame waits for the page's stream. Runs while
    the attempt is live, reconnecting when the window's socket drops."""
    while _attempt_is(attempt):
        with _lock:
            if not _state.get("capturing"):
                return
        pages = _cdp_pages(DEBUG_PORTS[0])
        if not pages:
            time.sleep(0.5)
            continue
        ws = None
        try:
            ws = _ws_connect(pages[0]["webSocketDebuggerUrl"], timeout=CAPTURE_CALL_SEC)
            _cdp_call(ws, "Page.startScreencast", {"format": "jpeg", "quality": 60, "maxWidth": LOGIN_VIEW_SIZE[0], "maxHeight": LOGIN_VIEW_SIZE[1], "everyNthFrame": 1}, timeout=CAPTURE_CALL_SEC)
            while _attempt_is(attempt):
                with _lock:
                    if not _state.get("capturing"):
                        return
                try:
                    opcode, data = ws.recv_message(timeout=2)
                except TimeoutError:
                    continue
                if opcode not in (0x1, 0x2):
                    continue
                msg = json.loads(data.decode("utf-8"))
                if msg.get("method") != "Page.screencastFrame":
                    continue
                p = msg.get("params") or {}
                frame = base64.b64decode(p.get("data") or "")
                if frame:
                    with _cast["cond"]:
                        _cast["frame"] = frame
                        _cast["seq"] += 1
                        _cast["cond"].notify_all()
                # acknowledged without waiting for the answer: waiting would eat the next frames
                ws.send_text(json.dumps({"id": ws._next_id, "method": "Page.screencastFrameAck", "params": {"sessionId": p.get("sessionId")}}))
                ws._next_id += 1
        except Exception:
            time.sleep(0.5)
        finally:
            if ws is not None:
                try:
                    ws.close()
                except Exception:
                    pass


def login_stream(write, alive):
    """The login window as a multipart JPEG stream: each frame as Chromium pushes it,
    until the app stops waiting for a login or the reader goes away."""
    last = -1
    while True:
        with _lock:
            if not _state.get("capturing"):
                return
        with _cast["cond"]:
            if _cast["seq"] == last:
                _cast["cond"].wait(1.0)
            if _cast["seq"] == last or _cast["frame"] is None:
                continue
            frame, last = _cast["frame"], _cast["seq"]
        if not alive():
            return
        write(b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: %d\r\n\r\n" % len(frame) + frame + b"\r\n")


_VIEW_KEYS = {"Enter": 13, "Tab": 9, "Backspace": 8, "Delete": 46, "Escape": 27, "ArrowLeft": 37, "ArrowUp": 38,
              "ArrowRight": 39, "ArrowDown": 40, "Home": 36, "End": 35}


def _view_key_event(ch):
    """The key event for one typed character: its text, key, code and virtual key code."""
    up = ch.upper()
    if ch.isdigit():
        code, vk = "Digit" + ch, ord(ch)
    elif "A" <= up <= "Z" and ch.isascii():
        code, vk = "Key" + up, ord(up)
    elif ch == " ":
        code, vk = "Space", 32
    else:
        code, vk = "", 0
    ev = {"key": ch, "text": ch, "unmodifiedText": ch, "code": code}
    if vk:
        ev["windowsVirtualKeyCode"] = vk
        ev["nativeVirtualKeyCode"] = vk
    return ev


def login_input(ev):
    """One click, typed text, key or scroll from the page, forwarded to the login window."""
    kind = _s((ev or {}).get("kind"))
    try:
        ws = _login_view_ws()
        if ws is None:
            return {"ok": False, "error": "No login window."}
        x, y = float(ev.get("x") or 0), float(ev.get("y") or 0)
        call = lambda method, params: _cdp_call(ws, method, params, timeout=CAPTURE_CALL_SEC)
        if kind == "click":
            call("Input.dispatchMouseEvent", {"type": "mouseMoved", "x": x, "y": y})
            for typ in ("mousePressed", "mouseReleased"):
                call("Input.dispatchMouseEvent", {"type": typ, "x": x, "y": y, "button": "left", "clickCount": 1})
        elif kind == "text":
            text = _s(ev.get("text"))
            if len(text) == 1 or (0 < len(text) <= 8 and text.isalnum()):
                # a keystroke, or a pasted code: real key events, one per character, since a
                # one-time-code field listens for keys and ignores text inserted as a block
                for ch in text:
                    call("Input.dispatchKeyEvent", dict(_view_key_event(ch), type="keyDown"))
                    call("Input.dispatchKeyEvent", dict(_view_key_event(ch), type="keyUp"))
            elif text:
                call("Input.insertText", {"text": text})
        elif kind == "key":
            key = _s(ev.get("key"))
            vk = _VIEW_KEYS.get(key)
            if vk is None:
                return {"ok": False, "error": "unknown key"}
            base = {"key": key, "code": key, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk}
            if key == "Enter":
                base["text"] = "\r"
            call("Input.dispatchKeyEvent", dict(base, type="keyDown"))
            call("Input.dispatchKeyEvent", dict(base, type="keyUp"))
        elif kind == "wheel":
            call("Input.dispatchMouseEvent", {"type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": float(ev.get("deltaY") or 0)})
        else:
            return {"ok": False, "error": "unknown input"}
        return {"ok": True}
    except Exception:
        _login_view_drop()
        return {"ok": False, "error": "The login window did not take that."}


def cancel_login():
    """Stop waiting for a login and close the window the app opened."""
    with _lock:
        was = bool(_state.get("capturing"))
        _state["capturing"] = False
        _state["error"] = ""
    sys.stderr.write("bagholder login: cancelled\n")
    _close_login_browser()
    return {"ok": True, "cancelled": was}


def start_login_browser():
    """Open the login window. Only a press of Connect reaches here, and this is
    the only place the app opens a browser window: a window the app's Chrome
    still has up is brought forward instead; anything else (no window, a
    lingering windowless Chrome) is closed and one fresh window is launched."""
    sys.stderr.write("bagholder login: connect requested\n")
    if _login_browser_alive():
        try:
            ws = _ws_connect(_login_browser_ws())
            try:
                _cdp_call(ws, "Target.activateTarget", {"targetId": _cdp_pages(DEBUG_PORTS[0])[0]["id"]})
            finally:
                ws.close()
        except Exception:
            pass
        with _lock:
            already = bool(_state.get("capturing"))
            _state["capturing"] = True
            _state["error"] = ""
            proc = _state.get("chrome_proc")
            if not already:
                _state["login_attempt"] += 1
            attempt = _state["login_attempt"]
        if not already:
            threading.Thread(target=_poll_chrome_session, args=(proc, DEBUG_PORTS[0], attempt), name="bagholder-cdp-capture", daemon=True).start()
        sys.stderr.write("bagholder login: window already up, brought forward\n")
        return {"ok": True, "reused": True}
    _close_login_browser()   # a windowless leftover of ours, if any
    chrome = find_chrome()
    if not chrome:
        return {
            "ok": False,
            "error": 'Install Chrome, Brave, Edge, or another Chromium browser. Passkey login has to happen on Wealthsimple’s site.',
        }
    profile = HOME / "chrome"
    _ensure_home()
    profile.mkdir(mode=0o700, exist_ok=True)
    debug_port = DEBUG_PORTS[0]
    args = [
        chrome,
        "--user-data-dir=" + str(profile),
        "--remote-debugging-port=%s" % debug_port,
        "--remote-debugging-address=127.0.0.1",
        "--remote-allow-origins=http://127.0.0.1",
        "--no-first-run",
        "--no-default-browser-check",
        "--new-window",
    ]
    if LOGIN_VIEW:
        # a container: a real window on its virtual display (headless Chromium is turned
        # away at Wealthsimple's door), sized for the page, sandbox off since the process is root
        args += ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage", "--window-position=0,0",
                 "--window-size=%d,%d" % LOGIN_VIEW_SIZE]
    args.append(LOGIN_URL)
    try:
        kwargs = {
            "stdin": subprocess.DEVNULL,
            "stdout": subprocess.DEVNULL,
            "stderr": subprocess.DEVNULL,
        }
        if os.name != "nt":
            kwargs["start_new_session"] = True
        proc = subprocess.Popen(args, **kwargs)
        sys.stderr.write("bagholder login: chrome launched (pid %s)\n" % proc.pid)
        with _lock:
            _state["chrome_proc"] = proc
            _state["capturing"] = True
            _state["error"] = ""
            _state["login_attempt"] += 1   # any watcher of an earlier attempt now exits quietly
            attempt = _state["login_attempt"]
        t = threading.Thread(
            target=_poll_chrome_session,
            args=(proc, debug_port, attempt),
            name="bagholder-cdp-capture",
            daemon=True,
        )
        t.start()
        if LOGIN_VIEW:
            with _cast["cond"]:
                _cast["frame"], _cast["seq"] = None, 0
            threading.Thread(target=_screencast_loop, args=(attempt,), name="bagholder-screencast", daemon=True).start()
        return {"ok": True}
    except Exception:
        return {
            "ok": False,
            "error": 'Install Chrome, Brave, Edge, or another Chromium browser. Passkey login has to happen on Wealthsimple’s site.',
        }


def capture_tokens(body):
    """Persist session.json and kick a sync thread. Never print tokens."""
    if not isinstance(body, dict):
        return {"ok": False, "error": "bad body"}
    access = body.get("access_token")
    if not access:
        return {"ok": False, "error": "missing access_token"}
    sess = load_session() or {}
    for k in (
        "access_token",
        "refresh_token",
        "identity_canonical_id",
        "expires_at",
        "wssdi",
        "client_id",
        "session_id",
        "user_agent",
    ):
        if body.get(k):
            sess[k] = body[k]
    ident = _identity_from(body) or _identity_from(sess)
    info = {}
    if sess.get("access_token"):
        try:
            info = token_info(sess) or {}
        except Exception:
            info = {}
    if not ident:
        ident = _identity_from(info)
    if ident:
        sess["identity_canonical_id"] = ident
    if not sess.get("session_id"):
        sess["session_id"] = str(uuid.uuid4())
    apply_token_info_client_id(sess, info)
    if not sess.get("client_id"):
        cid = scrape_client_id()
        if cid:
            sess["client_id"] = cid
    if not sess.get("user_agent"):
        ua = cached_user_agent()
        if ua:
            sess["user_agent"] = ua
    # Take the login over: rotate its refresh token now, so the copy the
    # browser holds goes stale instead of ours, and a capture Wealthsimple
    # will not honour is found out here, not at the next restart.
    if not refresh_session(sess, adopt=False):
        with _lock:
            _state["connected"] = False
            err = _state.get("error") or "Wealthsimple refused the captured login"
        return {"ok": False, "error": err}
    save_session(sess)
    with _lock:
        _state["connected"] = True
        _state["capturing"] = False
        _state["error"] = ""
    t = threading.Thread(target=run_sync, name="bagholder-sync", daemon=True)
    t.start()
    return {"ok": True}


def _manual_from_fields(body):
    side = _upper(body.get("side") or "BUY")
    if side not in ("BUY", "SELL"):
        side = "BUY"
    qty = abs(_num(body.get("qty") if body.get("qty") is not None else body.get("quantity")))
    px = abs(_num(body.get("price") if body.get("price") is not None else body.get("unitPrice")))
    date = _date_only(body.get("date") or body.get("transactionDate") or body.get("occurredAt")) or datetime.now(timezone.utc).strftime("%Y-%m-%d")
    symbol = _upper(body.get("symbol"))
    currency = _upper(body.get("currency") or "CAD")
    if currency not in ("CAD", "USD"):
        currency = "CAD"
    account_id = _s(body.get("accountId") or body.get("account") or "manual") or "manual"
    signed_qty = qty if side == "BUY" else -qty
    cash = -(qty * px) if side == "BUY" else (qty * px)
    return {
        "id": str(uuid.uuid4()),
        "occurredAt": date,
        "transactionDate": date,
        "settlementDate": date,
        "accountId": account_id,
        "bookId": account_id,
        "accountType": _s(body.get("accountType") or ("Manual" if account_id == "manual" else "")),
        "activityType": "Trade",
        "activitySubType": side,
        "description": ("Buy" if side == "BUY" else "Sell") + (f" {qty:g} {symbol} @ {px:g}" if symbol else ""),
        "direction": "DEBIT" if side == "BUY" else "CREDIT",
        "symbol": symbol,
        "name": symbol,
        "currency": currency,
        "quantity": signed_qty,
        "unitPrice": px,
        "commission": abs(_num(body.get("commission"))),
        "netCashAmount": cash,
        "category": "trade",
        "balance": None,
        "source": "manual",
    }


def _normalize_local_row(act):
    act = dict(act or {})
    source = _s(act.get("source")) or "manual"
    if source == "wealthsimple":
        cid = store._canonical_from_row(act, "wealthsimple")
        if cid:
            act["canonicalId"] = cid
            act["source"] = "wealthsimple"
            return act
        source = "manual"
    act["source"] = source
    act.pop("canonicalId", None)
    act.pop("canonical_id", None)
    if not act.get("accountId"):
        act["accountId"] = "manual"
    if not act.get("bookId"):
        act["bookId"] = act["accountId"]
    if not act.get("id") or store.looks_like_homemade_id(act.get("id")):
        act["id"] = str(uuid.uuid4())
    if not act.get("occurredAt"):
        act["occurredAt"] = act.get("transactionDate") or ""
    return act


def append_manual(body):
    body = body or {}
    rows = []
    if isinstance(body.get("activities"), list):
        rows = [r for r in body["activities"] if isinstance(r, dict)]
    elif body.get("activity") and isinstance(body["activity"], dict):
        rows = [body["activity"]]
    else:
        rows = [_manual_from_fields(body)]
    rows = [_normalize_local_row(r) for r in rows]
    result = store.merge_local_rows(rows)
    snap = store.snapshot()
    if not snap.get("syncedAt"):
        store.set_meta("synced_at", datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))
        snap = store.snapshot()
    with _lock:
        _state["lastSync"] = snap.get("syncedAt") or _state["lastSync"]
    saved = result.get("activities") or []
    out = {"ok": True, "added": result.get("added", 0), "duplicates": result.get("duplicates", 0)}
    if len(saved) == 1:
        out["activity"] = saved[0]
    elif saved:
        out["activities"] = saved
    elif len(rows) == 1:
        # duplicate of an already-stored local row
        out["activity"] = rows[0]
    return out


# ---------------------------------------------------------------------------
# order ticket
# ---------------------------------------------------------------------------
ORDER_EXEC_TYPES = ("MARKET", "LIMIT", "STOP", "STOP_LIMIT")
ORDER_TIFS = ("DAY", "UNTIL_CANCEL")
ORDER_TRADABLE_TYPES = ("SELF_DIRECTED",)   # account types the ticket offers, by prefix
ORDER_UNTRADABLE_MARKERS = ("CRYPTO", "PREDICTIONS", "MANAGED")


def _ticket_session():
    """The session for a ticket call: refreshed when its token is near expiry.
    None when there is no login."""
    sess = load_session()
    if not sess or not sess.get("access_token"):
        return None
    try:
        ensure_fresh_token(sess)
    except Exception:
        pass
    return load_session() or sess


def order_accounts(accounts=None):
    """The accounts a ticket can route to: open self-directed accounts that trade
    securities (crypto and predictions accounts have their own order paths). Each
    carries whether it is a margin account, and the account whose available margin
    the review shows: itself for a margin account, the margin account it backs for
    a collateral account, none otherwise."""
    out = []
    for a in (accounts if accounts is not None else store.snapshot().get("accounts") or []):
        typ = _s(a.get("unifiedAccountType") or a.get("unified_account_type")).upper()
        status = _s(a.get("status")).lower()
        if not a.get("id") or status == "closed" or not typ.startswith(ORDER_TRADABLE_TYPES):
            continue
        if any(m in typ for m in ORDER_UNTRADABLE_MARKERS):
            continue
        nick = model.norm_account_name(a.get("nickname") or typ)
        margin = "MARGIN" in typ
        out.append({"id": _s(a.get("id")), "name": nick, "type": typ, "margin": margin, "currency": _s(a.get("currency")),
                    "marginAccountId": _s(a.get("id")) if margin else _s(a.get("marginAccountId"))})
    return out


def resolve_security(symbol="", security_id=""):
    """The security a ticket is for: by id when the page knows it, else the stored
    listing under that exact symbol, a share listing before an option or cash row."""
    rows = store.list_securities()
    sid = _s(security_id).strip()
    if sid:
        for r in rows:
            if r["id"] == sid:
                return r
        # an id the book carries but no listing was fetched for yet: the quote names it
        return {"id": sid, "symbol": _s(symbol).strip().upper(), "name": "", "primaryExchange": "", "primaryMic": "", "currency": "", "underlyingId": None}
    sym = _s(symbol).strip().upper()
    if not sym:
        return None
    same = [r for r in rows if _s(r.get("symbol")).upper() == sym]
    same.sort(key=lambda r: (0 if _s(r["id"]).startswith("sec-s-") else 1, r["id"]))
    return same[0] if same else None


# --------------------------------------------------------------------------
# symbol search: the ⌘K box asks the exchanges' own directories, never
# Wealthsimple, for a text the book has no symbol for; one round per distinct
# text while the app runs (Nasdaq's autocomplete for US listings, TSX's company
# directory for the TSX and the TSX-V), the three asked together
# --------------------------------------------------------------------------

NASDAQ_SEARCH_URL = "https://api.nasdaq.com/api/autocomplete/slookup/10?search=%s"
TSX_SEARCH_URL = "https://www.tsx.com/json/company-directory/search/%s/%s"
SEARCH_HEADERS = {"User-Agent": market.UA, "Accept": "application/json, text/plain, */*", "Accept-Language": "en-CA,en;q=0.9"}
# Nasdaq's exchange code -> the exchange as the book names it
NASDAQ_EXCHANGES = {"NYSE": "NYSE", "AMEX": "NYSE", "PSE": "NYSE", "NASDAQ-GS": "NASDAQ", "NASDAQ-GM": "NASDAQ", "NASDAQ-CM": "NASDAQ", "NASDAQ": "NASDAQ", "BAT": "BATS"}
NASDAQ_ASSETS = ("STOCKS", "ETF")
NASDAQ_DERIVATIVE_SUFFIXES = ("WS", "W", "U", "RT", "R")   # warrants, units and rights listed beside a share
NASDAQ_NAME_TAILS = (" Common Stock", " Common Shares", " Ordinary Shares", " Class A Common Stock", " Class A Ordinary Shares")
SEARCH_MAX = 12
_search_cache = {}


def parse_nasdaq_search(data):
    """Nasdaq's autocomplete answer into US listings: shares and ETFs on the
    exchanges Wealthsimple trades, [{symbol, name, exchange, currency}]."""
    out = []
    for q in ((data or {}).get("data") or []) if isinstance(data, dict) else []:
        if not isinstance(q, dict) or _s(q.get("asset")).upper() not in NASDAQ_ASSETS:
            continue
        ex = NASDAQ_EXCHANGES.get(_s(q.get("exchange")).upper())
        sym = _s(q.get("symbol")).upper()
        if not ex or not sym or sym.rsplit(".", 1)[-1] in NASDAQ_DERIVATIVE_SUFFIXES:
            continue
        name = _s(q.get("name"))
        for tail in NASDAQ_NAME_TAILS:
            if name.endswith(tail):
                name = name[: -len(tail)].rstrip(" ,")
                break
        out.append({"symbol": sym, "name": name, "exchange": ex, "currency": "USD"})
    return out


def parse_tsx_search(data, exchange):
    """TSX's directory answer into that exchange's listings, one per issuer."""
    out = []
    for r in ((data or {}).get("results") or []) if isinstance(data, dict) else []:
        sym = _s(r.get("symbol")).upper() if isinstance(r, dict) else ""
        if sym:
            out.append({"symbol": sym, "name": _s(r.get("name")), "exchange": exchange, "currency": "CAD"})
    return out


def rank_search(text, rows):
    """Exact symbols first, then symbols starting with the text, then the rest,
    each group in the order the sources gave; duplicates dropped; at most SEARCH_MAX."""
    key = _s(text).strip().upper()
    seen, out = set(), []
    for r in rows:
        k = (r["symbol"], r["exchange"])
        if k in seen:
            continue
        seen.add(k)
        out.append(r)
    # an instrument found by an alias (`WTI` for the crude future) ranks as the exact match it is
    out.sort(key=lambda r: r["rank"] if r.get("rank") is not None else 0 if r["symbol"] == key else 1 if r["symbol"].startswith(key) else 2)
    return out[:SEARCH_MAX]


def symbol_search(text):
    """The listings the directories find for the text, remembered for the process.
    A source that fails leaves the others' answer; nothing is remembered when
    none answered."""
    text = _s(text).strip()
    if not text:
        return {"ok": True, "matches": []}
    key = text.upper()
    if key in _search_cache:
        return {"ok": True, "matches": _search_cache[key]}
    # `YES.V` as Yahoo writes it: the directories are asked for YES, and the suffix's venue is kept
    text, venues = market.yahoo_split(text)
    q = quote(text, safe="")
    jobs = [("nasdaq", NASDAQ_SEARCH_URL % q, lambda d: parse_nasdaq_search(d)),
            ("tsx", TSX_SEARCH_URL % ("tsx", q), lambda d: parse_tsx_search(d, "TSX")),
            ("tsxv", TSX_SEARCH_URL % ("tsxv", q), lambda d: parse_tsx_search(d, "TSX-V"))]
    answers, errors = {}, {}

    def run(name, url, parse):
        try:
            answers[name] = parse(json.loads(market._get_text(url, headers=SEARCH_HEADERS)))
        except Exception as e:
            errors[name] = str(e) or e.__class__.__name__
    threads = [threading.Thread(target=run, args=j, daemon=True) for j in jobs]
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=market.TIMEOUT_SEC + 2)
    if not answers:
        return {"ok": False, "error": "Search failed: " + "; ".join(errors.values()), "matches": []}
    rows = rank_search(text, instruments.search(text) + [r for name, _, _ in jobs for r in answers.get(name, [])])
    if venues:
        rows = [r for r in rows if _s(r.get("exchange")).upper() in venues] or rows
    if not errors:
        _search_cache[key] = rows
    return {"ok": True, "matches": rows}


def parse_quote(node):
    """One securities(ids) node into the ticket's quote card. Prices in the
    security's currency; the day's change against Wealthsimple's previous baseline."""
    if not isinstance(node, dict) or not node.get("id"):
        return None
    q = node.get("quoteV2") if isinstance(node.get("quoteV2"), dict) else {}
    stock = node.get("stock") if isinstance(node.get("stock"), dict) else {}
    opt = node.get("optionDetails") if isinstance(node.get("optionDetails"), dict) else {}
    last = _num(q.get("price"), None)
    if last is None:
        last = _num(q.get("last"), None)
    base = _num(q.get("previousBaseline"), None)
    if base is None:
        base = _num(q.get("referenceClose"), None)
    bid, ask = _num(q.get("bid"), None), _num(q.get("ask"), None)
    change = (last - base) if (last is not None and base is not None) else None
    return {
        "securityId": _s(node.get("id")),
        "symbol": _s(stock.get("symbol")),
        "name": _s(stock.get("name")),
        "exchange": _s(stock.get("primaryExchange")),
        "currency": _s(q.get("currency") or node.get("currency")).upper(),
        "securityType": _s(node.get("securityType")),
        "buyable": bool(node.get("buyable")),
        "sellable": bool(node.get("sellable")),
        "tradeEligible": bool(node.get("wsTradeEligible")),
        "status": _s(node.get("status")),
        "last": last,
        "bid": bid,
        "ask": ask,
        "bidSize": _num(q.get("bidSize"), None),
        "askSize": _num(q.get("askSize"), None),
        "mid": _num(q.get("mid"), None) if q.get("mid") is not None else ((bid + ask) / 2 if bid is not None and ask is not None else None),
        "change": change,
        "changePct": (change / base) if (change is not None and base) else None,
        "marketStatus": _s(q.get("marketStatus")),
        "quotedAsOf": _s(q.get("quotedAsOf")),
        "multiplier": _num(opt.get("multiplier"), None) if opt else None,
    }


def parse_market_data(data):
    sec = (data or {}).get("security") if isinstance(data, dict) else None
    sec = sec if isinstance(sec, dict) else {}
    subtypes = [str(s).upper() for s in (sec.get("allowedOrderSubtypes") or []) if s]
    rate = _num(((sec.get("marginRates") or {}) if isinstance(sec.get("marginRates"), dict) else {}).get("clientMarginRate"), None)
    if rate is not None and rate > 1:
        rate = rate / 100.0   # a percentage; the ticket wants a fraction
    return {"orderTypes": [t for t in ORDER_EXEC_TYPES if t in subtypes], "marginRate": rate}


def parse_buying_power(data):
    view = ((((data or {}).get("account") or {}).get("financials") or {}).get("current") or {}).get("tradingBalanceViewV2") or {}
    bp = view.get("buyingPower") if isinstance(view.get("buyingPower"), dict) else {}
    cash = view.get("cash") if isinstance(view.get("cash"), dict) else {}
    return {"buyingPower": _num(bp.get("quantity"), None), "cash": _num(cash.get("quantity"), None), "currency": _s(bp.get("currency") or cash.get("currency"))}


def fetch_quotes(sess, security_ids):
    """The ticket's quote for each id, one request. Missing ids are absent."""
    ids = [s for s in (_s(x).strip() for x in security_ids or []) if s]
    if not ids:
        return {}
    data = graphql(sess, "FetchSecuritiesSummary", {"ids": ids})
    out = {}
    for node in (data or {}).get("securities") or []:
        q = parse_quote(node)
        if q:
            out[q["securityId"]] = q
    return out


LOOKUP_TYPES = ("EQUITY", "EXCHANGE_TRADED_FUND")
CANADIAN_SUFFIXES = (".TO", ".V", ".CN", ".NE")


def _bare_symbol(sym):
    sym = _s(sym).upper()
    for suf in CANADIAN_SUFFIXES:
        if sym.endswith(suf):
            return sym[: -len(suf)]
    return sym


def parse_listing_search(data, symbol, exchange):
    """The one result of a securitySearch answer that is the listing asked for: same
    bare symbol (Wealthsimple writes a Canadian listing as QNC.TO whatever its
    venue), same exchange, a share or an ETF; the stored shape, or None."""
    want_sym, want_ex = _bare_symbol(symbol), _s(exchange).strip().upper()
    for r in (((data or {}).get("securitySearch") or {}).get("results") or []) if isinstance(data, dict) else []:
        if not isinstance(r, dict) or not r.get("id"):
            continue
        stock = r.get("stock") if isinstance(r.get("stock"), dict) else {}
        if _bare_symbol(stock.get("symbol")) != want_sym or _s(stock.get("primaryExchange")).upper() != want_ex:
            continue
        if _s(r.get("securityType")).upper() not in LOOKUP_TYPES:
            continue
        return {"id": _s(r["id"]), "symbol": _s(stock.get("symbol")).upper(), "name": _s(stock.get("name")), "primaryExchange": _s(stock.get("primaryExchange")),
                "primaryMic": _s(stock.get("primaryMic")), "currency": _s(r.get("currency")).upper(), "underlyingId": None}
    return None


def lookup_listing(sess, symbol, exchange):
    """Wealthsimple's listing for a symbol the book has never held: its id, which
    an order is placed against, asked once by symbol and kept with the book's
    listings, so the symbol is never asked for again. None when Wealthsimple
    has no such listing."""
    try:
        data = graphql(sess, "FetchSecuritySearchResult", {"query": _s(symbol).strip()})
    except Exception as e:
        sys.stderr.write("bagholder ticket: listing search for %s failed: %s\n" % (symbol, e))
        return None
    sec = parse_listing_search(data, symbol, exchange)
    if sec:
        store.upsert_securities([sec])
    return sec


def ticket_quote(symbol="", security_id="", account_id="", exchange=""):
    """Everything the ticket shows for one security in one account: the quote card,
    the order types Wealthsimple allows for it, its margin rate, the account's buying
    power and cash for it, the margin account's available margin, today's USD rate.
    Errors are answers, not exceptions: the page prints them in the panel."""
    sec = resolve_security(symbol, security_id)
    if not sec and not _s(exchange):
        return {"ok": False, "error": "No listing stored for " + (_s(symbol) or _s(security_id)) + "."}
    sess = _ticket_session()
    if not sess:
        return {"ok": False, "error": "Not connected."}
    if not sec:
        sec = lookup_listing(sess, _s(symbol).strip().upper(), _s(exchange))
    if not sec:
        return {"ok": False, "error": "No listing stored for " + (_s(symbol) or _s(security_id)) + "."}
    try:
        quotes = fetch_quotes(sess, [sec["id"]])
    except PermissionError:
        return {"ok": False, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}
    except Exception as e:
        return {"ok": False, "error": "Quote failed: " + (str(e) or e.__class__.__name__)}
    quote = quotes.get(sec["id"])
    if not quote:
        return {"ok": False, "error": "Wealthsimple has no quote for " + (sec.get("symbol") or sec["id"]) + "."}
    if not quote["symbol"]:
        quote["symbol"] = sec.get("symbol") or ""
    if not quote["name"]:
        quote["name"] = sec.get("name") or ""
    if not quote["exchange"]:
        quote["exchange"] = sec.get("primaryExchange") or ""
    if not quote["currency"]:
        quote["currency"] = _s(sec.get("currency")).upper()
    md = {"orderTypes": list(ORDER_EXEC_TYPES), "marginRate": None}
    try:
        md = parse_market_data(graphql(sess, "FetchSecurityMarketData", {"id": sec["id"]}))
    except Exception as e:
        sys.stderr.write("bagholder ticket: market data for %s failed: %s\n" % (sec["id"], e))
    accounts = order_accounts()
    acct = next((a for a in accounts if a["id"] == _s(account_id)), None)
    balance = {"buyingPower": None, "cash": None, "currency": ""}
    if acct:
        try:
            balance = parse_buying_power(graphql(sess, "FetchTradingBalanceBuyingPower", {"accountCanonicalId": acct["id"], "currency": quote["currency"] or "CAD", "securityId": sec["id"]}))
        except Exception as e:
            sys.stderr.write("bagholder ticket: buying power for %s failed: %s\n" % (acct["id"], e))
    margin_available = None
    if acct and acct["marginAccountId"]:
        for m in store.snapshot().get("margin") or []:
            if _s(m.get("accountId")) == acct["marginAccountId"] and m.get("buyingPower") is not None:
                margin_available = _num(m.get("buyingPower"), None)
    fx = store.fx_rates()
    return {
        "ok": True,
        "quote": quote,
        "orderTypes": md["orderTypes"] or list(ORDER_EXEC_TYPES),
        "marginRate": md["marginRate"],
        "accounts": accounts,
        "account": acct,
        "buyingPower": balance["buyingPower"],
        "cash": balance["cash"],
        "marginAvailable": margin_available,
        "fxUsdCad": model.rate_on(fx, model.today_local()) if fx else None,
        "live": ORDERS_LIVE,
    }


def order_tick(price):
    """A price as Wealthsimple accepts it: two decimals from $1, four below. A quote
    can carry more, and an order built from one must not."""
    if price is None:
        return None
    return round(float(price), 2 if price >= 1 else 4)


def order_request(body):
    """Validate a ticket and build the request Wealthsimple's web app sends for it.
    Returns (row, request, error); the row is what the store keeps."""
    b = body if isinstance(body, dict) else {}
    err = lambda m: (None, None, m)
    side = _s(b.get("side")).upper()
    if side not in ("BUY", "SELL"):
        return err("Side must be Buy or Sell.")
    exec_type = _s(b.get("type")).upper()
    if exec_type not in ORDER_EXEC_TYPES:
        return err("Order type must be Market, Limit, Stop or Stop limit.")
    tif = _s(b.get("tif") or "DAY").upper()
    if tif not in ORDER_TIFS:
        return err("Time in force must be Day or Good till cancelled.")
    qty = _num(b.get("quantity"), 0.0)
    if not qty or qty <= 0:
        return err("Quantity must be more than zero.")
    limit_price = order_tick(_num(b.get("limitPrice"), None))
    stop_price = order_tick(_num(b.get("stopPrice"), None))
    if exec_type in ("LIMIT", "STOP_LIMIT") and not (limit_price and limit_price > 0):
        return err("A limit price is required.")
    if exec_type in ("STOP", "STOP_LIMIT") and not (stop_price and stop_price > 0):
        return err("A stop price is required.")
    acct = next((a for a in order_accounts() if a["id"] == _s(b.get("accountId"))), None)
    if not acct:
        return err("Choose an account.")
    sec = resolve_security(b.get("symbol"), b.get("securityId"))
    if not sec:
        return err("No listing stored for " + _s(b.get("symbol")) + ".")
    sl = b.get("stopLoss") if isinstance(b.get("stopLoss"), dict) else None
    tp = b.get("takeProfit") if isinstance(b.get("takeProfit"), dict) else None
    if side == "SELL":
        sl, tp = None, None   # nothing to protect: the shares leave
    if sl:
        kind = _s(sl.get("kind") or "stop").lower()
        if kind not in ("stop", "trail"):
            return err("Stop loss type must be Stop or Trailing stop.")
        if kind == "stop" and not (_num(sl.get("price"), 0) > 0):
            return err("A stop loss price is required.")
        if kind == "trail" and not (_num(sl.get("trail"), 0) > 0):
            return err("A trail is required.")
        sl = {"kind": kind, "price": order_tick(_num(sl.get("price"), None)), "trail": _num(sl.get("trail"), None), "trailUnit": "amt" if _s(sl.get("trailUnit")).lower() == "amt" else "pct"}
    if tp:
        if not (_num(tp.get("price"), 0) > 0):
            return err("A take profit price is required.")
        tp = {"price": order_tick(_num(tp.get("price"), None))}
    oid = "order-" + str(uuid.uuid4())
    req = {
        "canonicalAccountId": acct["id"],
        "externalId": oid,
        "executionType": exec_type,
        "orderType": side + "_QUANTITY",
        "quantity": qty,
        "securityId": sec["id"],
        "timeInForce": tif,
    }
    if exec_type in ("LIMIT", "STOP_LIMIT"):
        req["limitPrice"] = limit_price
    if exec_type in ("STOP", "STOP_LIMIT"):
        req["stopPrice"] = stop_price
    row = {
        "id": oid,
        "createdAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "accountId": acct["id"],
        "account": acct["name"],
        "securityId": sec["id"],
        "symbol": _s(sec.get("symbol")),
        "currency": _s(b.get("currency") or sec.get("currency")).upper(),
        "side": side,
        "type": exec_type,
        "quantity": qty,
        "limitPrice": req.get("limitPrice"),
        "stopPrice": req.get("stopPrice"),
        "tif": tif,
        "stopLoss": sl,
        "takeProfit": tp,
        "status": "",
        "wsOrderId": "",
        "error": "",
        "request": req,
    }
    return row, req, ""


def submit_order(row, req):
    """Record the row; send it to Wealthsimple only when ORDERS_LIVE. The row is
    written before the request goes out and updated with the answer, so a crash
    between the two leaves a row that names the external id Wealthsimple knows."""
    if not ORDERS_LIVE:
        row["status"] = "dry"
        store.insert_order(row)
        sys.stderr.write("bagholder order (dry run, not sent): %s\n" % json.dumps(req, sort_keys=True))
        return {"ok": True, "id": row["id"], "status": "dry", "order": row}
    sess = _ticket_session()
    if not sess:
        return {"ok": False, "error": "Not connected."}
    row["status"] = "sending"
    store.insert_order(row)
    try:
        data = graphql(sess, "SoOrdersOrderCreate", {"input": req})
    except PermissionError:
        store.update_order(row["id"], {"status": "failed", "error": "Wealthsimple refused the session."})
        return {"ok": False, "error": "Wealthsimple refused the session. Connect Wealthsimple again.", "id": row["id"]}
    except Exception as e:
        msg = str(e) or e.__class__.__name__
        store.update_order(row["id"], {"status": "failed", "error": msg})
        sys.stderr.write("bagholder order: %s failed: %s\n" % (row["id"], msg))
        return {"ok": False, "error": "Order failed: " + msg, "id": row["id"]}
    result = (data or {}).get("soOrdersCreateOrder") or {}
    errs = result.get("errors") or []
    if errs:
        first = errs[0] if isinstance(errs[0], dict) else {"message": str(errs[0])}
        msg = _s(first.get("message") or first.get("code"))
        store.update_order(row["id"], {"status": "rejected", "error": msg})
        sys.stderr.write("bagholder order: %s rejected: %s\n" % (row["id"], msg))
        return {"ok": False, "error": "Wealthsimple rejected the order: " + msg, "id": row["id"]}
    order = result.get("order") or {}
    store.update_order(row["id"], {"status": "sent", "wsOrderId": _s(order.get("orderId"))})
    sys.stderr.write("bagholder order: %s sent, Wealthsimple order %s\n" % (row["id"], _s(order.get("orderId"))))
    threading.Thread(target=refresh_orders, args=(row["id"],), name="bagholder-order-refresh", daemon=True).start()
    return {"ok": True, "id": row["id"], "status": "sent", "wsOrderId": _s(order.get("orderId"))}


def place_order(body):
    """A ticket: validated, recorded, sent when ORDERS_LIVE; its brackets, when it
    has any, become a bracket row that waits for the fill."""
    row, req, error = order_request(body)
    if error:
        return {"ok": False, "error": error}
    if row.get("side") == "SELL":
        # A position's shares are all held by the one order resting on them. A bracket's
        # exit on these shares is cancelled, and confirmed gone, before the sell goes out.
        # Selling part of them: the bracket keeps the rest and places its stop on them again.
        left = _num(row.get("quantity"), 0.0) or 0.0
        for b in store.list_brackets(BRACKET_LIVE):
            if b["accountId"] != row.get("accountId") or b["securityId"] != row.get("securityId") or b["status"] in ("waiting", "closing"):
                continue
            held = _num(b.get("quantity"), 0.0) or 0.0
            if left >= held:
                _end_bracket(b, "sold from the ticket")
                _await_cancels(b)
                left -= held
            elif left > 0:
                _release_shares(b, left)
                _await_cancels(b)
                left = 0.0
    r = submit_order(row, req)
    if r.get("ok") and (row.get("stopLoss") or row.get("takeProfit")):
        b = create_bracket(row)
        r["bracketId"] = b["id"]
    return r


# --- reading orders back: Wealthsimple's state of every order, from anywhere ---
ORDER_BRANCH = "TR"   # the branch id Wealthsimple's web app passes with every order lookup
# Wealthsimple's order statuses, grouped as the page shows them
WS_PENDING = ("NEW", "PENDING_SUBMISSION", "PENDING_REVIEW", "PENDING_FUND_TRANSFER", "SUBMITTED", "PLACED", "PARTIALLY_FILLED", "CONTINGENT")
WS_CANCELLING = ("CANCEL_PENDING",)
WS_STATUS_MAP = {"FILLED": "filled", "POSTED": "filled", "CANCELLED": "cancelled", "DELETED": "cancelled", "EXPIRED": "expired", "REJECTED": "rejected"}
LIVE_STATUSES = ("sent", "pending", "cancelling")   # rows still worth asking Wealthsimple about
ORDERS_REFRESH_SEC = 30


def app_status(ws_status):
    s = _s(ws_status).upper()
    if s in WS_PENDING:
        return "pending"
    if s in WS_CANCELLING:
        return "cancelling"
    return WS_STATUS_MAP.get(s, "pending" if s else "")


def parse_extended_order(data):
    """One soOrdersExtendedOrder answer into the fields the row keeps."""
    o = (data or {}).get("soOrdersExtendedOrder") if isinstance(data, dict) else None
    if not isinstance(o, dict) or not o.get("status"):
        return None
    return {
        "wsStatus": _s(o.get("status")).upper(),
        "status": app_status(o.get("status")),
        "filledQty": _num(o.get("filledQuantity"), None),
        "avgFill": _num(o.get("averageFilledPrice"), None),
        "submittedAt": _s(o.get("submittedAtUtc")),
        "expiresAt": _s(o.get("expiredAtUtc")),
        "error": _s(o.get("rejectionCause") or o.get("rejectionCode")),
        "quantity": _num(o.get("submittedQuantity"), None),
        "limitPrice": _num(o.get("limitPrice"), None),
        "stopPrice": _num(o.get("stopPrice"), None),
        "tif": _s(o.get("timeInForce")).upper(),
        "currency": _s(o.get("securityCurrency")).upper(),
        "accountId": _s(o.get("canonicalAccountId") or o.get("accountId")),
        "securityId": _s(o.get("securityId")),
        "type": _s(o.get("orderType")).upper(),
    }


def fetch_extended_order(sess, external_id):
    return parse_extended_order(graphql(sess, "FetchSoOrdersExtendedOrder", {"branchId": ORDER_BRANCH, "externalId": _s(external_id)}))


def fetch_order_feed(sess, identity, statuses=WS_PENDING):
    """Every order of the identity in the given statuses, placed from anywhere."""
    out = []
    cursor = None
    while True:
        data = graphql(sess, "OrderServiceExtendedOrderFeed", {"identityId": identity, "statuses": list(statuses), "first": 25, "cursor": cursor})
        feed = (((data or {}).get("identity") or {}).get("orderServiceExtendedOrderFeed") or {})
        for edge in feed.get("edges") or []:
            node = (edge or {}).get("node")
            if isinstance(node, dict) and node.get("id"):
                out.append(node)
        page = feed.get("pageInfo") or {}
        cursor = page.get("endCursor")
        if not page.get("hasNextPage") or not cursor:
            break
    return out


def feed_order_row(node):
    """A feed node that Bagholder did not place, as a row of its own."""
    sec = node.get("security") if isinstance(node.get("security"), dict) else {}
    stock = sec.get("stock") if isinstance(sec.get("stock"), dict) else {}
    acct = next((a for a in order_accounts() if a["id"] == _s(node.get("canonicalAccountId"))), None)
    side = _s(node.get("side")).upper()
    sec_id = _s(node.get("securityId") or sec.get("id"))
    # the feed names an option by its underlying; the book knows the contract
    symbol = store.symbol_for_security(sec_id) or _s(node.get("symbol") or stock.get("symbol"))
    return {
        "id": _s(node.get("id")),
        "createdAt": _s(node.get("createdAtUtc")),
        "accountId": _s(node.get("canonicalAccountId")),
        "account": acct["name"] if acct else "",
        "securityId": sec_id,
        "symbol": symbol,
        "currency": _s(node.get("securityCurrency")).upper(),
        "side": "SELL" if side.startswith("SELL") else "BUY",
        "type": _s(node.get("executionType")).upper() or "LIMIT",
        "quantity": _num(node.get("submittedQuantity"), 0.0),
        "limitPrice": _num(node.get("limitPrice"), None),
        "stopPrice": _num(node.get("stopPrice"), None),
        "tif": "",
        "stopLoss": None,
        "takeProfit": None,
        "status": app_status(node.get("status")),
        "wsStatus": _s(node.get("status")).upper(),
        "wsOrderId": _s(node.get("orderId")),
        "avgFill": _num(node.get("averageFillPrice"), None),
        "source": "wealthsimple",
    }


_orders_refreshed_at = ""
_orders_refreshing = threading.Lock()


def kick_orders_refresh():
    """A read in the background when the last one is older than the loop's tick and
    none is running: the Orders tab opening does not wait for the loop."""
    if _orders_refreshed_at:
        try:
            age = (datetime.now(timezone.utc) - datetime.strptime(_orders_refreshed_at, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)).total_seconds()
        except ValueError:
            age = ORDERS_REFRESH_SEC
        if age < ORDERS_REFRESH_SEC:
            return False
    with _lock:
        connected = bool(_state["connected"]) and not _state["syncing"]
    if not connected or not _orders_refreshing.acquire(blocking=False):
        return False

    def go():
        try:
            refresh_orders()
        finally:
            _orders_refreshing.release()
    threading.Thread(target=go, name="bagholder-orders-refresh", daemon=True).start()
    return True


def refresh_orders(only_id=""):
    """Read every live order's state back from Wealthsimple, and the pending-order
    feed so an order placed in Wealthsimple's own app is a row too. Never raises;
    what failed is on the terminal. Returns what it did."""
    global _orders_refreshed_at
    sess = _ticket_session()
    if not sess:
        return {"ok": False, "skipped": "no session"}
    live = [o for o in store.list_orders() if o["status"] in LIVE_STATUSES and (not only_id or o["id"] == only_id)]
    read, failed = 0, 0
    for o in live:
        try:
            upd = fetch_extended_order(sess, o["id"])
        except PermissionError:
            sys.stderr.write("bagholder orders: Wealthsimple refused the session\n")
            return {"ok": False, "skipped": "refused"}
        except Exception as e:
            failed += 1
            sys.stderr.write("bagholder orders: %s status failed: %s\n" % (o["id"], str(e) or e.__class__.__name__))
            continue
        if upd:
            patch = {k: upd[k] for k in ("wsStatus", "status", "filledQty", "avgFill", "submittedAt", "expiresAt") if upd.get(k) is not None}
            if upd.get("error"):
                patch["error"] = upd["error"]
            if o.get("source") == "wealthsimple" or o.get("role") in ("stop", "target"):
                # a feed row learns its terms; a bracket's own exit learns a change made by hand
                for k in ("tif", "quantity", "limitPrice", "stopPrice", "currency"):
                    if upd.get(k) not in (None, ""):
                        patch[k] = upd[k]
                # a row that arrived before the book knew the contract learns its name
                name = store.symbol_for_security(o.get("securityId"))
                if name and name != o.get("symbol"):
                    patch["symbol"] = name
            store.update_order(o["id"], patch)
            read += 1
    added = 0
    if not only_id:
        identity = _identity_from(sess)
        if identity:
            try:
                rows = store.list_orders()
                known = {o["id"] for o in rows} | {o["wsOrderId"] for o in rows if o.get("wsOrderId")}
                for node in fetch_order_feed(sess, identity):
                    # an order Bagholder sent is known by the external id it gave, and by the
                    # order id Wealthsimple answered with: either match, it is the same order
                    if node["id"] in known or _s(node.get("orderId")) in known:
                        continue
                    store.insert_order(feed_order_row(node))
                    added += 1
            except PermissionError:
                return {"ok": False, "skipped": "refused"}
            except Exception as e:
                failed += 1
                sys.stderr.write("bagholder orders: pending-order feed failed: %s\n" % (str(e) or e.__class__.__name__))
    if not only_id:   # one order's read after a send or a cancel is not a check of everything
        _orders_refreshed_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    if read or added or failed:
        sys.stderr.write("bagholder orders: %d read, %d found pending at Wealthsimple, %d failed\n" % (read, added, failed))
    return {"ok": not failed, "read": read, "added": added, "failed": failed}


def orders_loop():
    """Every ORDERS_REFRESH_SEC while connected: the live orders' state and the feed."""
    while not _stop.wait(ORDERS_REFRESH_SEC):
        with _lock:
            connected = bool(_state["connected"]) and not _state["syncing"]
        if not connected:
            continue
        if not any(o["status"] in LIVE_STATUSES for o in store.list_orders()) and _orders_refreshed_at:
            # nothing live: the feed alone, every tenth tick, catches an order placed elsewhere
            if int(time.time() / ORDERS_REFRESH_SEC) % 10:
                continue
        refresh_orders()


def cancel_order(order_id):
    """Ask Wealthsimple to cancel a live order. Only with ORDERS_LIVE: without it the
    app never writes to Wealthsimple, and a dry row was never there to cancel."""
    row = store.get_order(order_id)
    if not row:
        return {"ok": False, "error": "No such order."}
    if row["status"] not in LIVE_STATUSES:
        return {"ok": False, "error": "That order is not open."}
    if not ORDERS_LIVE:
        return {"ok": False, "error": "Orders are off (BAGHOLDER_DRY_ORDERS): nothing is sent to Wealthsimple."}
    sess = _ticket_session()
    if not sess:
        return {"ok": False, "error": "Not connected."}
    try:
        data = graphql(sess, "SoOrdersOrderCancel", {"cancelOrderRequest": {"externalId": row["id"]}})
    except PermissionError:
        return {"ok": False, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}
    except Exception as e:
        msg = str(e) or e.__class__.__name__
        sys.stderr.write("bagholder orders: cancel %s failed: %s\n" % (row["id"], msg))
        return {"ok": False, "error": "Cancel failed: " + msg}
    result = (data or {}).get("orderServiceCancelOrder") or {}
    errs = result.get("errors") or []
    if errs:
        first = errs[0] if isinstance(errs[0], dict) else {"message": str(errs[0])}
        msg = _s(first.get("message") or first.get("code"))
        sys.stderr.write("bagholder orders: cancel %s refused: %s\n" % (row["id"], msg))
        return {"ok": False, "error": "Wealthsimple refused the cancel: " + msg}
    store.update_order(row["id"], {"status": "cancelling", "wsStatus": "CANCEL_PENDING"})
    sys.stderr.write("bagholder orders: cancel %s accepted\n" % row["id"])
    threading.Thread(target=refresh_orders, args=(row["id"],), name="bagholder-order-refresh", daemon=True).start()
    return {"ok": True, "id": row["id"], "status": "cancelling"}


def orders_payload(kick=False):
    if kick:
        kick_orders_refresh()
    exchanges = {s["id"]: s.get("primaryExchange") or "" for s in store.list_securities()}
    orders = store.list_orders()
    for o in orders:
        o["exchange"] = exchanges.get(o.get("securityId") or "", "")
    return {"ok": True, "orders": orders, "brackets": store.list_brackets(), "live": ORDERS_LIVE, "refreshedAt": _orders_refreshed_at}


# ---------------------------------------------------------------------------
# brackets: the stop loss and take profit Bagholder watches for a filled order
# ---------------------------------------------------------------------------
BRACKET_POLL_SEC = 5
BRACKET_RETRY_SEC = (60, 300, 900, 3600)   # the wait after a refused placement: a minute, five, fifteen, then every hour
TRAIL_MIN_MOVE = 0.005          # a trailing stop moves only when it would rise by half a percent of its level: each move is a cancel and a new order
TARGET_BACK_OFF = 0.01          # the limit sell resting at the target gives way to the stop order again when the bid is this far under the target
BRACKET_LIVE = ("waiting", "armed", "firing", "target_placed", "stopping", "closing")
BRACKET_RESTING = ("sent", "pending")            # an exit order Wealthsimple holds
BRACKET_INFLIGHT = ("sent", "pending", "cancelling")   # ... or is still deciding about
# Wealthsimple ends a good-till-cancelled order ninety days after it is placed and reports
# the moment with the order. A resting exit this close to it is placed again before it:
# outside the regular session when there is time, in any session in the last two days.
BRACKET_ROLL_SEC = 7 * 86400
BRACKET_ROLL_LAST_SEC = 2 * 86400
GTC_DAYS = 90
_bracket_lock = threading.Lock()
_bracket_said = set()   # dry-run lines already printed, so the terminal is not flooded every tick


# An exit always goes out good till cancelled, whatever the entry's time in force: a
# stop that lapsed at the close would leave the position unprotected overnight.
# Wealthsimple keeps such an order ninety days; the engine places it again at that boundary.
BRACKET_TIF = "UNTIL_CANCEL"


def _release_shares(b, sold):
    """Part of a bracket's shares are being sold from the ticket: its resting exit is
    cancelled (it held them all), the bracket keeps the rest, armed, and the arm step
    places a stop on the remainder once the cancel is confirmed."""
    remaining = round((_num(b.get("quantity"), 0.0) or 0.0) - sold, 6)
    for oid in (b.get("slOrderId"), b.get("tpOrderId")):
        err = _cancel_exit(oid)
        if err:
            sys.stderr.write("bagholder bracket: %s for %s: cancel of %s refused: %s\n" % (b["id"], b["symbol"], oid, err))
    store.update_bracket(b["id"], {"quantity": remaining, "slOrderId": "", "tpOrderId": "", "status": "armed", "error": "", "attempts": 0})
    sys.stderr.write("bagholder bracket: %s for %s: %s of its shares sold from the ticket; the stop is placed again on the %s left\n" % (b["id"], b["symbol"], qty_text(sold), qty_text(remaining)))


def _await_cancels(b, seconds=8):
    """Wait, up to a few seconds, for Wealthsimple to confirm the cancel of the bracket's
    resting exits, reading them back each second; the ticket's sell follows either way
    (Wealthsimple refuses it itself while an order still holds the shares)."""
    if not ORDERS_LIVE:
        return
    for _ in range(seconds):
        open_rows = [o for o in _own_exit_rows(b) if o["status"] in BRACKET_INFLIGHT]
        if not open_rows:
            return
        for o in open_rows:
            refresh_orders(only_id=o["id"])
        time.sleep(1)


def create_bracket(order_row):
    sl, tp = order_row.get("stopLoss"), order_row.get("takeProfit")
    b = {
        "id": "bracket-" + str(uuid.uuid4()), "orderId": order_row["id"], "accountId": order_row["accountId"], "securityId": order_row["securityId"],
        "symbol": order_row.get("symbol") or "", "currency": order_row.get("currency") or "", "quantity": order_row.get("quantity"), "tif": BRACKET_TIF,
        "slKind": (sl or {}).get("kind") or "", "slPrice": (sl or {}).get("price"), "slTrail": (sl or {}).get("trail"), "slTrailUnit": (sl or {}).get("trailUnit") or "pct",
        "tpPrice": (tp or {}).get("price"), "status": "waiting",
    }
    store.insert_bracket(b)
    return store.get_bracket(b["id"])


def _say_once(key, line):
    if key in _bracket_said:
        return
    _bracket_said.add(key)
    sys.stderr.write(line + "\n")


def _trail_distance(b, price):
    if b["slKind"] != "trail" or not b.get("slTrail"):
        return None
    return price * b["slTrail"] / 100.0 if b.get("slTrailUnit") == "pct" else b["slTrail"]


def _exit_body(b, exec_type, price, role):
    body = {"symbol": b["symbol"], "securityId": b["securityId"], "accountId": b["accountId"], "side": "SELL", "type": exec_type, "tif": BRACKET_TIF,
            "quantity": b["quantity"], "currency": b["currency"]}
    if exec_type == "LIMIT":
        body["limitPrice"] = price
    if exec_type == "STOP":
        body["stopPrice"] = price
    row, req, err = order_request(body)
    if err:
        return None, None, err
    row["role"], row["parentId"] = role, b["orderId"]
    return row, req, ""


def _place_exit(b, exec_type, price, role):
    """One exit order for the bracket. Without ORDERS_LIVE the line is printed once and
    nothing is placed; the bracket keeps waiting. Returns (order id, error)."""
    row, req, err = _exit_body(b, exec_type, price, role)
    if err:
        return "", err
    if not ORDERS_LIVE:
        _say_once((b["id"], role, round(price, 4)), "bagholder bracket (orders are off, not placed): %s %s for %s: %s" % (role, exec_type, b["symbol"], json.dumps(req, sort_keys=True)))
        return "", ""
    r = submit_order(row, req)
    if not r.get("ok"):
        return "", r.get("error") or "not sent"
    return r["id"], ""


def _cancel_exit(order_id):
    """Cancel one of the bracket's own resting orders. Returns "" when it is cancelled
    or already on its way, else the error text."""
    if not order_id:
        return ""
    row = store.get_order(order_id)
    if not row or row["status"] not in ("sent", "pending"):
        return ""   # nothing resting: filled, cancelled, cancelling or never sent
    r = cancel_order(order_id)
    if r.get("ok") or "not open" in (r.get("error") or ""):
        return ""
    return r.get("error") or "cancel failed"


def _exit_row(b, role):
    """The bracket's order of that role: the one it currently holds when it holds one,
    else its latest (a moved trailing stop leaves the old one behind, cancelled)."""
    held = b.get("slOrderId") if role == "stop" else b.get("tpOrderId")
    if held:
        row = store.get_order(held)
        if row:
            return row
    rows = [o for o in store.list_orders() if o.get("parentId") == b["orderId"] and o.get("role") == role]
    return rows[0] if rows else None


def _may_retry(b):
    """After a refused placement the next try waits: a minute, then five, fifteen, an hour."""
    attempts = int(b.get("attempts") or 0)
    if not attempts:
        return True
    wait = BRACKET_RETRY_SEC[min(attempts, len(BRACKET_RETRY_SEC)) - 1]
    try:
        since = (datetime.now(timezone.utc) - datetime.strptime(b.get("updatedAt") or "", "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)).total_seconds()
    except ValueError:
        return True
    return since >= wait


def _fail(b, msg):
    """A refused placement: the reason is kept on the bracket and the leg is tried again,
    a minute later, then five, fifteen, and every hour after that, for as long as the
    bracket lives. A stop that only waits for the market to take it is never given up."""
    attempts = int(b.get("attempts") or 0) + 1
    store.update_bracket(b["id"], {"error": msg, "attempts": attempts})
    sys.stderr.write("bagholder bracket: %s for %s: %s (attempt %d; next in %d s)\n" % (b["id"], b["symbol"], msg, attempts, BRACKET_RETRY_SEC[min(attempts, len(BRACKET_RETRY_SEC)) - 1]))


def _arm_step(b, entry):
    """Waiting: the entry filled, or ended with a partial fill, arms the bracket for the
    filled quantity. Armed without a resting stop: places it (native when the security
    allows a stop order, else it is watched here)."""
    if b["status"] == "waiting":
        if entry is None:
            store.update_bracket(b["id"], {"status": "cancelled", "outcome": "entry not found"})
            return
        if entry["status"] in ("pending", "sent", "cancelling", "dry"):
            return
        filled = _num(entry.get("filledQty"), 0.0) or 0.0
        if entry["status"] == "filled" and not filled:
            filled = _num(entry.get("quantity"), 0.0) or 0.0
        if not filled or filled <= 0:
            store.update_bracket(b["id"], {"status": "cancelled", "outcome": "entry " + entry["status"]})
            sys.stderr.write("bagholder bracket: %s for %s off: entry %s without a fill\n" % (b["id"], b["symbol"], entry["status"]))
            return
        b = dict(b, quantity=filled, status="armed", armedAt=datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))
        patch = {"quantity": filled, "status": "armed", "armedAt": b["armedAt"]}
        if b["slKind"] == "trail":
            # the trail starts from what was actually paid, not the ticket's working price
            high = _num(entry.get("avgFill"), None) or _num(entry.get("limitPrice"), None) or b.get("slPrice")
            if high:
                patch["highWater"] = high
                patch["slPrice"] = round(high - (_trail_distance(b, high) or 0.0), 2)
                b = dict(b, highWater=high, slPrice=patch["slPrice"])
        store.update_bracket(b["id"], patch)
        sys.stderr.write("bagholder bracket: %s armed for %s x %s\n" % (b["id"], qty_text(filled), b["symbol"]))
    if b["status"] != "armed" or not b["slKind"] or b.get("slOrderId"):
        return
    # a stop leg with no resting order: nothing of the bracket's may still be in flight at
    # Wealthsimple (the previous stop, or the limit sell giving way): one order on the shares
    if not _nothing_resting(b):
        return
    native = _stop_allowed(b["securityId"])
    if not native:
        if b.get("slMode") != "watched":
            store.update_bracket(b["id"], {"slMode": "watched", "slNative": False})
            sys.stderr.write("bagholder bracket: %s for %s: Wealthsimple takes no stop order for it; the stop is watched here\n" % (b["id"], b["symbol"]))
        return
    if not _may_retry(b):
        return
    oid, err = _place_exit(b, "STOP", b["slPrice"], "stop")
    if err:
        _fail(b, "stop not placed: " + err)
    elif oid:
        store.update_bracket(b["id"], {"slOrderId": oid, "slNative": True, "slMode": "native", "error": "", "attempts": 0, "movedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")})
        sys.stderr.write("bagholder bracket: %s stop placed at %s for %s\n" % (b["id"], b["slPrice"], b["symbol"]))


_stop_allowed_cache = {}


def _stop_allowed(security_id):
    """Whether Wealthsimple takes a stop order for the security (its allowedOrderSubtypes), remembered."""
    if security_id in _stop_allowed_cache:
        return _stop_allowed_cache[security_id]
    sess = _ticket_session()
    if not sess:
        return False
    try:
        md = parse_market_data(graphql(sess, "FetchSecurityMarketData", {"id": security_id}))
        ok = "STOP" in md["orderTypes"]
    except Exception as e:
        sys.stderr.write("bagholder bracket: order types for %s unknown: %s\n" % (security_id, e))
        return False
    _stop_allowed_cache[security_id] = ok
    return ok


def _own_exit_rows(b):
    """Every exit order the bracket ever placed, newest first."""
    return [o for o in store.list_orders() if o.get("parentId") == b["orderId"] and o.get("role") in ("stop", "target")]


def _end_bracket(b, outcome, note=""):
    """The bracket is over: every exit of its own still resting at Wealthsimple is
    cancelled, and the bracket reads closing until Wealthsimple confirms each cancel,
    then done. Nothing of the bracket's may outlive the position."""
    pending = False
    for o in _own_exit_rows(b):
        if o["status"] in BRACKET_RESTING:
            err = _cancel_exit(o["id"])
            if err:
                sys.stderr.write("bagholder bracket: %s for %s: cancel of %s refused: %s; tried again on the next check\n" % (b["id"], b["symbol"], o["id"], err))
            pending = True
        elif o["status"] == "cancelling":
            pending = True
    status = "closing" if pending else "done"
    store.update_bracket(b["id"], {"status": status, "outcome": outcome, "error": note, "slOrderId": "", "tpOrderId": ""})
    sys.stderr.write("bagholder bracket: %s for %s: %s%s\n" % (b["id"], b["symbol"], outcome, "; its resting exit is being cancelled" if pending else ""))
    return status


def _closing_step(b):
    """Closing: a cancel that was refused is sent again; once nothing of the bracket's
    rests at Wealthsimple, done."""
    open_rows = [o for o in _own_exit_rows(b) if o["status"] in BRACKET_INFLIGHT]
    for o in open_rows:
        if o["status"] in BRACKET_RESTING:
            err = _cancel_exit(o["id"])
            if err:
                sys.stderr.write("bagholder bracket: %s for %s: cancel of %s refused again: %s\n" % (b["id"], b["symbol"], o["id"], err))
    if not open_rows:
        store.update_bracket(b["id"], {"status": "done"})
        sys.stderr.write("bagholder bracket: %s for %s: nothing rests at Wealthsimple; done\n" % (b["id"], b["symbol"]))


def _sweep_exits():
    """Every check: an exit order of Bagholder's own that rests at Wealthsimple with no
    live bracket holding it is cancelled. The last line of defence, never the first."""
    for o in store.list_orders():
        if o.get("role") not in ("stop", "target") or o["status"] not in BRACKET_RESTING:
            continue
        b = store.bracket_for_order(o.get("parentId") or "")
        held_by = b and b["status"] in BRACKET_LIVE and (b["status"] == "closing" or o["id"] in (b.get("slOrderId"), b.get("tpOrderId")))
        if held_by:
            continue
        err = _cancel_exit(o["id"])
        _say_once((o["id"], "orphan"), "bagholder bracket: %s for %s rests at Wealthsimple with no bracket holding it; cancelled%s\n" % (o["id"], o.get("symbol"), (" (refused: " + err + ")") if err else ""))


def _nothing_resting(b):
    return not any(o["status"] in BRACKET_INFLIGHT for o in _own_exit_rows(b))


def _closed_elsewhere(b):
    """Only a bracket with nothing resting at Wealthsimple can see its shares sold
    elsewhere (a resting order of ours holds them). Then the sale shows in the
    activity feed, or the balances omit the position on two successive reads."""
    if b["status"] not in ("armed", "firing", "target_placed", "stopping") or not b.get("armedAt") or not _nothing_resting(b):
        return ""
    sold = store.sold_since(b["accountId"], b["securityId"], b["armedAt"], b.get("symbol"))
    if sold and sold >= (b.get("quantity") or 0):
        return "sold: %s shares in the activity feed" % qty_text(sold)
    read_at = store.get_meta("balances_read_at", "")
    if not read_at or read_at <= b["armedAt"]:
        return ""
    held = store.position_quantity(b["accountId"], b["securityId"])
    if held is not None and held > 0:
        if not b.get("seenHeld") or b.get("missedAt"):
            store.update_bracket(b["id"], {"seenHeld": True, "missedAt": ""})
        return ""
    if not b.get("seenHeld"):
        return ""
    missed = b.get("missedAt") or ""
    if not missed:
        store.update_bracket(b["id"], {"missedAt": read_at})
        sys.stderr.write("bagholder bracket: %s for %s: the balances read at %s does not list the position; a second read decides\n" % (b["id"], b["symbol"], read_at))
        return ""
    if read_at > missed:
        return "position gone: two balance reads without it (%s, %s)" % (missed, read_at)
    return ""


def _parse_utc(text):
    text = _s(text).strip()
    if not text:
        return None
    try:
        return datetime.strptime(text[:19], "%Y-%m-%dT%H:%M:%S").replace(tzinfo=timezone.utc)
    except ValueError:
        return None


def _expires_in(row, now):
    """Seconds until Wealthsimple ends the order: its reported expiry, else ninety days
    from its submission for a good-till-cancelled order; None when unknown."""
    exp = _parse_utc(row.get("expiresAt"))
    if exp is None and _s(row.get("tif")).upper() == "UNTIL_CANCEL":
        sub = _parse_utc(row.get("submittedAt") or row.get("createdAt"))
        exp = sub + timedelta(days=GTC_DAYS) if sub else None
    return (exp - now).total_seconds() if exp else None


def _roll_due(row, quote, now):
    """Whether a resting exit is placed again now: within seven days of its end and the
    market not in its regular session, or within two days whatever the session."""
    if not row or row["status"] not in ("sent", "pending"):
        return False
    left = _expires_in(row, now)
    if left is None or left > BRACKET_ROLL_SEC:
        return False
    if left <= BRACKET_ROLL_LAST_SEC:
        return True
    return _s((quote or {}).get("marketStatus")).upper() != "OPEN"


def _roll_step(b, quote):
    """A stop or target resting at Wealthsimple that is about to reach its ninety days is
    cancelled and placed again at the same level, before it ends: the stop by the arm
    step once the cancel is confirmed (a trailing stop keeps its level and its high), the
    target here the same way. Nothing waits for the expiry."""
    now = datetime.now(timezone.utc)
    now_s = now.strftime("%Y-%m-%dT%H:%M:%SZ")
    if b["status"] == "armed" and b.get("slMode") == "native" and b.get("slOrderId"):
        row = store.get_order(b["slOrderId"])
        if _roll_due(row, quote, now):
            err = _cancel_exit(b["slOrderId"])
            if err:
                _fail(b, "stop not rolled: " + err)
                return
            store.update_bracket(b["id"], {"slOrderId": "", "movedAt": now_s, "error": ""})
            sys.stderr.write("bagholder bracket: %s for %s: stop at %s nears Wealthsimple's ninety days; cancelled, placed again at the same level\n" % (b["id"], b["symbol"], b.get("slPrice")))
    elif b["status"] == "target_placed":
        if b.get("tpOrderId"):
            row = store.get_order(b["tpOrderId"])
            if _roll_due(row, quote, now):
                err = _cancel_exit(b["tpOrderId"])
                if err:
                    _fail(b, "target not rolled: " + err)
                    return
                store.update_bracket(b["id"], {"tpOrderId": "", "movedAt": now_s, "error": ""})
                sys.stderr.write("bagholder bracket: %s for %s: target at %s nears Wealthsimple's ninety days; cancelled, placed again\n" % (b["id"], b["symbol"], b.get("tpPrice")))
        else:
            # the rolled target: placed again the moment Wealthsimple confirms the cancel
            tp_row = _exit_row(b, "target")
            if tp_row and tp_row["status"] in ("cancelled", "expired"):
                _fire_target(b)


def _reconcile_step(b, entry):
    """What Wealthsimple and the book say: an exit of ours filled ends the bracket; a
    stop cancelled by hand leaves the target watched; a position gone ends it."""
    if b["status"] == "closing":
        _closing_step(b)
        return "done"
    stop_row, tp_row = _exit_row(b, "stop"), _exit_row(b, "target")
    if stop_row and stop_row["status"] == "filled":
        _end_bracket(b, "stopped")
        return "done"
    if tp_row and tp_row["status"] == "filled":
        _end_bracket(b, "target")
        return "done"
    # a change made by hand at Wealthsimple to the bracket's own resting order is adopted
    if b.get("slOrderId") and stop_row and stop_row["status"] in BRACKET_RESTING and stop_row.get("stopPrice") and b.get("slPrice") and abs(stop_row["stopPrice"] - b["slPrice"]) > 0.005:
        store.update_bracket(b["id"], {"slPrice": stop_row["stopPrice"]})
        sys.stderr.write("bagholder bracket: %s for %s: stop moved by hand to %s; the bracket follows\n" % (b["id"], b["symbol"], stop_row["stopPrice"]))
        b = dict(b, slPrice=stop_row["stopPrice"])
    if b.get("tpOrderId") and tp_row and tp_row["status"] in BRACKET_RESTING and tp_row.get("limitPrice") and b.get("tpPrice") and abs(tp_row["limitPrice"] - b["tpPrice"]) > 0.005:
        store.update_bracket(b["id"], {"tpPrice": tp_row["limitPrice"]})
        sys.stderr.write("bagholder bracket: %s for %s: target moved by hand to %s; the bracket follows\n" % (b["id"], b["symbol"], tp_row["limitPrice"]))
    if b["status"] == "armed" and b.get("slMode") == "native" and b.get("slOrderId") and stop_row and stop_row["status"] == "expired":
        # a stop Wealthsimple ended (a Day stop from before every exit went good till cancelled,
        # or ninety days reached while the app was off): placed again on the next check
        store.update_bracket(b["id"], {"slOrderId": "", "error": ""})
        sys.stderr.write("bagholder bracket: %s for %s: stop expired at Wealthsimple; placed again\n" % (b["id"], b["symbol"]))
    elif b["status"] == "armed" and b.get("slMode") == "native" and b.get("slOrderId") and stop_row and stop_row["status"] in ("cancelled", "rejected", "failed"):
        # not our doing (a move or a roll clears slOrderId first): a cancel by hand is the
        # person taking the position over; a refusal means the shares are not there. Either
        # way the bracket is over, and no watched leg fires later against those shares.
        why = "stop cancelled at Wealthsimple by hand" if stop_row["status"] == "cancelled" else "stop " + stop_row["status"] + " at Wealthsimple" + ((": " + stop_row["error"]) if stop_row.get("error") else "")
        _end_bracket(b, why)
        return "done"
    if b["status"] == "target_placed" and b.get("tpOrderId") and tp_row and tp_row["status"] == "expired":
        # Wealthsimple ended it (the app was off at its ninety days): placed again on the next check
        store.update_bracket(b["id"], {"tpOrderId": "", "error": ""})
        sys.stderr.write("bagholder bracket: %s for %s: target expired at Wealthsimple; placed again\n" % (b["id"], b["symbol"]))
    elif b["status"] == "target_placed" and b.get("tpOrderId") and tp_row and tp_row["status"] in ("cancelled", "rejected", "failed"):
        why = "target cancelled at Wealthsimple by hand" if tp_row["status"] == "cancelled" else "target " + tp_row["status"] + " at Wealthsimple" + ((": " + tp_row["error"]) if tp_row.get("error") else "")
        _end_bracket(b, why)
        return "done"
    # A position can only be sold elsewhere while nothing of the bracket's rests at
    # Wealthsimple (a resting order of ours holds the shares). Then the activity feed,
    # or two successive balance reads without the position, end the bracket; one read
    # never does: that feed omitted a held position on one read and listed it on the next.
    why = _closed_elsewhere(b)
    if why:
        _end_bracket(b, why)
        return "done"
    return ""


def _watch_step(b, quote):
    """Armed, with Wealthsimple's quote and the market open: the trailing stop follows the
    high, a watched stop fires as a market sell, the target fires as a limit sell."""
    if not quote or _s(quote.get("marketStatus")).upper() != "OPEN":
        return
    last, bid = quote.get("last"), quote.get("bid")
    if last is None:
        return
    now = datetime.now(timezone.utc)
    now_s = now.strftime("%Y-%m-%dT%H:%M:%SZ")
    trigger = bid if bid is not None else last
    at_target = bool(b.get("tpPrice")) and trigger >= b["tpPrice"]
    # trailing: the level follows the high (while the limit sell rests too: the level is
    # what Bagholder watches for then); not on the check that reaches the target, whose
    # cancel of the stop takes the place of a move
    if b["slKind"] == "trail" and (b["status"] == "target_placed" or (b["status"] == "armed" and not at_target)):
        high = max(b.get("highWater") or 0.0, last)
        if high != b.get("highWater"):
            store.update_bracket(b["id"], {"highWater": high})
        new_stop = round(high - (_trail_distance(b, high) or 0.0), 2)
        cur = b.get("slPrice") or 0.0
        if new_stop > cur + max(0.01, cur * TRAIL_MIN_MOVE):
            if b.get("slOrderId"):
                err = _cancel_exit(b["slOrderId"])
                if err:
                    _fail(b, "stop not moved: " + err)
                    return
            store.update_bracket(b["id"], {"slPrice": new_stop, "slOrderId": "", "movedAt": now_s})
            sys.stderr.write("bagholder bracket: %s for %s: stop moves to %s (high %s)\n" % (b["id"], b["symbol"], new_stop, high))
            b = dict(b, slPrice=new_stop, slOrderId="")
    # a stop watched here (no native stop order): fires at market
    if b["status"] == "armed" and b["slKind"] and b.get("slMode") == "watched" and not b.get("slOrderId") and b.get("slPrice"):
        trigger = bid if bid is not None else last
        if trigger <= b["slPrice"]:
            if not _may_retry(b):
                return
            oid, err = _place_exit(b, "MARKET", b["slPrice"], "stop")
            if err:
                _fail(b, "stop not placed: " + err)
            elif oid:
                store.update_bracket(b["id"], {"slOrderId": oid, "status": "firing", "error": ""})
                sys.stderr.write("bagholder bracket: %s for %s: stop hit at %s, market sell placed\n" % (b["id"], b["symbol"], trigger))
            return
    # the target: cancel the resting stop first, then the limit sell
    if b["status"] == "armed" and b.get("tpPrice"):
        trigger = bid if bid is not None else last
        if trigger >= b["tpPrice"]:
            if b.get("slOrderId"):
                err = _cancel_exit(b["slOrderId"])
                if err:
                    _fail(b, "stop not cancelled for the target: " + err)
                    return
                store.update_bracket(b["id"], {"status": "firing", "error": ""})
                sys.stderr.write("bagholder bracket: %s for %s: target reached at %s, stop cancel sent\n" % (b["id"], b["symbol"], trigger))
                return
            _fire_target(b)
    # the target after the stop's cancel is confirmed
    if b["status"] == "firing" and b.get("tpPrice") and not b.get("tpOrderId"):
        stop_row = _exit_row(b, "stop")
        if stop_row and stop_row["status"] == "cancelled":
            _fire_target(b)
    # while the limit sell rests at the target there is no stop at Wealthsimple: the stop
    # level is watched here, and reaching it cancels the limit sell for a market sell; and
    # once the target is out of reach, a percent under it, the limit sell gives way to the
    # stop order again, so the state with no stop at Wealthsimple lasts only while the
    # target is actually in reach
    if b["status"] == "target_placed" and b["slKind"] and b.get("slPrice") and b.get("tpOrderId"):
        trigger = bid if bid is not None else last
        if trigger <= b["slPrice"]:
            err = _cancel_exit(b["tpOrderId"])
            if err:
                _fail(b, "target not cancelled for the stop: " + err)
                return
            store.update_bracket(b["id"], {"status": "stopping", "tpOrderId": "", "error": "", "attempts": 0})
            sys.stderr.write("bagholder bracket: %s for %s: stop level %s reached at %s while the limit sell rested; its cancel sent, market sell follows\n" % (b["id"], b["symbol"], b["slPrice"], trigger))
            return
        if b.get("tpPrice") and trigger < b["tpPrice"] * (1 - TARGET_BACK_OFF):
            err = _cancel_exit(b["tpOrderId"])
            if err:
                _fail(b, "target not cancelled for the stop: " + err)
                return
            store.update_bracket(b["id"], {"status": "armed", "tpOrderId": "", "slOrderId": "", "error": "", "attempts": 0})
            sys.stderr.write("bagholder bracket: %s for %s: target out of reach at %s; the limit sell's cancel sent, the stop order goes back\n" % (b["id"], b["symbol"], trigger))
            return
    # the market sell once the limit sell's cancel is confirmed
    if b["status"] == "stopping":
        tp_row = _exit_row(b, "target")
        if tp_row and tp_row["status"] in ("cancelled", "expired"):
            if not _may_retry(b) or not _nothing_resting(b):
                return
            oid, err = _place_exit(b, "MARKET", b.get("slPrice"), "stop")
            if err:
                _fail(b, "stop not placed: " + err)
            elif oid:
                store.update_bracket(b["id"], {"slOrderId": oid, "status": "firing", "error": "", "attempts": 0})
                sys.stderr.write("bagholder bracket: %s for %s: market sell placed at the stop\n" % (b["id"], b["symbol"]))


def _fire_target(b):
    if not _may_retry(b) or not _nothing_resting(b):
        return
    oid, err = _place_exit(b, "LIMIT", b["tpPrice"], "target")
    if err:
        _fail(b, "target not placed: " + err)
    elif oid:
        store.update_bracket(b["id"], {"tpOrderId": oid, "status": "target_placed", "error": "", "attempts": 0})
        sys.stderr.write("bagholder bracket: %s for %s: limit sell at %s placed\n" % (b["id"], b["symbol"], b["tpPrice"]))


def bracket_tick(quotes=None):
    """One pass over the live brackets. quotes: security id -> quote, fetched here when None."""
    if not _bracket_lock.acquire(blocking=False):
        return {"ok": False, "skipped": "running"}
    try:
        live = store.list_brackets(BRACKET_LIVE)
        if not live:
            try:
                _sweep_exits()
            except Exception as e:
                sys.stderr.write("bagholder bracket: sweep failed: %s\n" % (str(e) or e.__class__.__name__))
            return {"ok": True, "brackets": 0}
        # what the engine is waiting on is read now, not on the orders loop's next pass:
        # a waiting bracket's entry, and the stop whose cancel must confirm before the target
        if ORDERS_LIVE:
            for b in live:
                if b["status"] == "waiting":
                    refresh_orders(only_id=b["orderId"])
                elif b["status"] == "firing" and b.get("slOrderId") and not b.get("tpOrderId"):
                    refresh_orders(only_id=b["slOrderId"])
                elif b["status"] == "armed" and b["slKind"] and not b.get("slOrderId"):
                    prev = _exit_row(b, "stop")   # a moved or rolled stop: its cancel must confirm before the new one goes
                    if prev and prev["status"] in ("sent", "pending", "cancelling"):
                        refresh_orders(only_id=prev["id"])
                elif b["status"] == "target_placed" and not b.get("tpOrderId"):
                    prev = _exit_row(b, "target")
                    if prev and prev["status"] in ("sent", "pending", "cancelling"):
                        refresh_orders(only_id=prev["id"])
                elif b["status"] == "stopping":
                    prev = _exit_row(b, "target")
                    if prev and prev["status"] in ("sent", "pending", "cancelling"):
                        refresh_orders(only_id=prev["id"])
                elif b["status"] == "closing":
                    for o in _own_exit_rows(b):
                        if o["status"] == "cancelling":
                            refresh_orders(only_id=o["id"])
        orders = {o["id"]: o for o in store.list_orders()}
        if quotes is None:
            ids = sorted({b["securityId"] for b in live if b["status"] in ("armed", "firing", "target_placed", "stopping")})
            quotes = {}
            if ids:
                sess = _ticket_session()
                if sess:
                    try:
                        quotes = fetch_quotes(sess, ids)
                    except Exception as e:
                        sys.stderr.write("bagholder bracket: quotes failed: %s\n" % (str(e) or e.__class__.__name__))
        for b in live:
            try:
                entry = orders.get(b["orderId"])
                r = _reconcile_step(b, entry)
                if r == "done":
                    continue
                b = store.get_bracket(b["id"])
                _roll_step(b, quotes.get(b["securityId"]))
                b = store.get_bracket(b["id"])
                _arm_step(b, entry)
                b = store.get_bracket(b["id"])
                if b["status"] in ("armed", "firing", "target_placed", "stopping"):
                    _watch_step(b, quotes.get(b["securityId"]))
            except Exception as e:
                sys.stderr.write("bagholder bracket: %s tick failed: %s\n" % (b["id"], str(e) or e.__class__.__name__))
        try:
            _sweep_exits()
        except Exception as e:
            sys.stderr.write("bagholder bracket: sweep failed: %s\n" % (str(e) or e.__class__.__name__))
        return {"ok": True, "brackets": len(live)}
    finally:
        _bracket_lock.release()


def bracket_loop():
    """Every BRACKET_POLL_SEC while connected: the live brackets, with one quote request
    for every armed symbol. Exit orders are read back by the orders loop."""
    while not _stop.wait(BRACKET_POLL_SEC):
        with _lock:
            connected = bool(_state["connected"]) and not _state["syncing"]
        if not connected:
            continue
        try:
            bracket_tick()
        except Exception as e:
            sys.stderr.write("bagholder bracket: tick failed: %s\n" % (str(e) or e.__class__.__name__))


def cancel_bracket(bracket_id):
    """Stop watching: the resting exit orders of the bracket are cancelled at Wealthsimple."""
    b = store.get_bracket(bracket_id)
    if not b:
        return {"ok": False, "error": "No such bracket."}
    if b["status"] not in BRACKET_LIVE:
        return {"ok": False, "error": "That bracket is not live."}
    _end_bracket(b, "cancelled by the user")
    return {"ok": True, "id": b["id"]}


def modify_order(order_id, quantity=None, limit_price=None):
    """Change a resting order's quantity or limit price at Wealthsimple, as its web app
    does (SoOrdersOrderModify with the external id, newLimitPrice, newQuantity)."""
    row = store.get_order(order_id)
    if not row:
        return {"ok": False, "error": "No such order."}
    if row["status"] not in ("sent", "pending"):
        return {"ok": False, "error": "That order is not open."}
    if row["type"] == "STOP":
        return {"ok": False, "error": "A stop order cannot be changed; cancel it and place another."}
    q = _num(quantity, None)
    lp = order_tick(_num(limit_price, None))
    if q is not None and q <= 0:
        return {"ok": False, "error": "Shares must be more than zero."}
    if lp is not None and lp <= 0:
        return {"ok": False, "error": "A limit price must be more than zero."}
    if row["type"] in ("LIMIT", "STOP_LIMIT") and lp is None and q is None:
        return {"ok": False, "error": "Nothing to change."}
    inp = {"externalId": row["id"]}
    if lp is not None and row["type"] in ("LIMIT", "STOP_LIMIT") and lp != row.get("limitPrice"):
        inp["newLimitPrice"] = lp
    if q is not None and q != row.get("quantity"):
        inp["newQuantity"] = q
    if len(inp) == 1:
        return {"ok": True, "id": row["id"], "unchanged": True}
    if not ORDERS_LIVE:
        return {"ok": False, "error": "Orders are off (BAGHOLDER_DRY_ORDERS): nothing is sent to Wealthsimple."}
    sess = _ticket_session()
    if not sess:
        return {"ok": False, "error": "Not connected."}
    try:
        data = graphql(sess, "SoOrdersOrderModify", {"input": inp})
    except PermissionError:
        return {"ok": False, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}
    except Exception as e:
        msg = str(e) or e.__class__.__name__
        sys.stderr.write("bagholder orders: modify %s failed: %s\n" % (row["id"], msg))
        return {"ok": False, "error": "Change failed: " + msg}
    errs = ((data or {}).get("soOrdersModifyOrder") or {}).get("errors") or []
    if errs:
        first = errs[0] if isinstance(errs[0], dict) else {"message": str(errs[0])}
        msg = _s(first.get("message") or first.get("code"))
        sys.stderr.write("bagholder orders: modify %s refused: %s\n" % (row["id"], msg))
        return {"ok": False, "error": "Wealthsimple refused the change: " + msg}
    patch = {}
    if "newLimitPrice" in inp:
        patch["limitPrice"] = lp
    if "newQuantity" in inp:
        patch["quantity"] = q
    store.update_order(row["id"], patch)
    b = store.bracket_for_order(row["id"])
    if b and b["status"] == "waiting" and "newQuantity" in inp:
        store.update_bracket(b["id"], {"quantity": q})
    sys.stderr.write("bagholder orders: modify %s accepted: %s\n" % (row["id"], json.dumps({k: v for k, v in inp.items() if k != "externalId"}, sort_keys=True)))
    threading.Thread(target=refresh_orders, args=(row["id"],), name="bagholder-order-refresh", daemon=True).start()
    return {"ok": True, "id": row["id"]}


def adjust_bracket(bracket_id, leg, price=None, trail=None, remove=False):
    """Move one leg of a live bracket, or remove it. A resting stop is cancelled and the
    engine places it again at the new level on its next check; a placed target order is
    cancelled and the target is watched again at the new level."""
    b = store.get_bracket(bracket_id)
    if not b:
        return {"ok": False, "error": "No such bracket."}
    if b["status"] not in BRACKET_LIVE:
        return {"ok": False, "error": "That bracket is not live."}
    leg = _s(leg).lower()
    if leg not in ("sl", "tp"):
        return {"ok": False, "error": "Which leg?"}
    if remove:
        if leg == "sl":
            err = _cancel_exit(b.get("slOrderId"))
            if err:
                return {"ok": False, "error": err}
            patch = {"slKind": "", "slOrderId": "", "slMode": "", "error": ""}
            if not b.get("tpPrice"):
                patch.update({"status": "cancelled", "outcome": "both legs removed"})
            store.update_bracket(b["id"], patch)
        else:
            err = _cancel_exit(b.get("tpOrderId"))
            if err:
                return {"ok": False, "error": err}
            patch = {"tpPrice": None, "tpOrderId": "", "error": ""}
            if b["status"] == "target_placed":
                patch["status"] = "armed"
            if not b.get("slKind"):
                patch.update({"status": "cancelled", "outcome": "both legs removed"})
            store.update_bracket(b["id"], patch)
        sys.stderr.write("bagholder bracket: %s for %s: %s removed by the user\n" % (b["id"], b["symbol"], "stop loss" if leg == "sl" else "take profit"))
        return {"ok": True, "id": b["id"]}
    if leg == "sl":
        if not b.get("slKind"):
            return {"ok": False, "error": "This bracket has no stop loss."}
        if b["slKind"] == "trail":
            t = _num(trail, None)
            if not t or t <= 0:
                return {"ok": False, "error": "A trail is required."}
            high = b.get("highWater") or b.get("slPrice") or 0.0
            nb = dict(b, slTrail=t)
            new_price = round(high - (_trail_distance(nb, high) or 0.0), 2) if high else b.get("slPrice")
            patch = {"slTrail": t, "slPrice": new_price}
        else:
            p = _num(price, None)
            if not p or p <= 0:
                return {"ok": False, "error": "A stop price is required."}
            patch = {"slPrice": p}
        if b.get("slOrderId") and b["status"] == "armed":
            err = _cancel_exit(b["slOrderId"])
            if err:
                return {"ok": False, "error": err}
            patch["slOrderId"] = ""
            patch["movedAt"] = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        patch["error"] = ""
        store.update_bracket(b["id"], patch)
        sys.stderr.write("bagholder bracket: %s for %s: stop moved to %s by the user\n" % (b["id"], b["symbol"], patch.get("slPrice")))
        return {"ok": True, "id": b["id"]}
    p = _num(price, None)
    if not p or p <= 0:
        return {"ok": False, "error": "A limit price is required."}
    patch = {"tpPrice": p, "error": ""}
    if b["status"] == "target_placed" and b.get("tpOrderId"):
        err = _cancel_exit(b["tpOrderId"])
        if err:
            return {"ok": False, "error": err}
        patch.update({"tpOrderId": "", "status": "armed"})
    store.update_bracket(b["id"], patch)
    sys.stderr.write("bagholder bracket: %s for %s: target moved to %s by the user\n" % (b["id"], b["symbol"], p))
    return {"ok": True, "id": b["id"]}


# ---------------------------------------------------------------------------
# watchlist: listings followed without being held; quoted and classified like a holding
# ---------------------------------------------------------------------------
def watch_add(body):
    body = body if isinstance(body, dict) else {}
    sym = market.tmx_symbol(body.get("symbol"))   # the bare ticker: Wealthsimple's `.TO` on a dual listing is not the app's convention
    if not sym:
        return {"ok": False, "error": "symbol required"}
    inst = instruments.find(sym, body.get("exchange"))
    row = store.add_watch(sym, body.get("exchange"), (inst or {}).get("name") or body.get("name"), (inst or {}).get("currency") or body.get("currency"), body.get("securityId"))
    model.invalidate()
    def fetch():
        # its quote and its sector, from the same public sources a holding uses; shown as they land
        try:
            market.refresh_quotes(model.quote_symbols(), _ssl_context())
            model.invalidate()
        except Exception as e:
            sys.stderr.write("bagholder watchlist: quote for %s failed: %s\n" % (sym, e))
        if inst or _s(body.get("exchange")).upper() == "CRYPTO":
            return   # an index, a future or a coin has no sector record to read
        try:
            exposure.share_exposure(row["symbol"], row.get("exchange") or "", row.get("currency") or "")
            model.invalidate()
        except Exception as e:
            sys.stderr.write("bagholder watchlist: sector for %s failed: %s\n" % (sym, e))
    threading.Thread(target=fetch, name="watch-fetch", daemon=True).start()
    return {"ok": True, "watchlist": store.list_watchlist()}


def watch_remove(body):
    body = body if isinstance(body, dict) else {}
    sym = market.tmx_symbol(body.get("symbol"))
    if not sym:
        return {"ok": False, "error": "symbol required"}
    store.remove_watch(sym, body.get("exchange"))
    store.remove_watch(_s(body.get("symbol")).strip().upper(), body.get("exchange"))   # a row kept under Wealthsimple's form
    store.forget_news(sym, body.get("exchange"))
    model.invalidate()
    return {"ok": True, "watchlist": store.list_watchlist()}


def tiles_set(body):
    """The Markets tab's tile row, in order, from the page: only instruments the directory knows, twelve at most."""
    body = body if isinstance(body, dict) else {}
    rows, seen = [], set()
    for r in body.get("tiles") if isinstance(body.get("tiles"), list) else []:
        inst = instruments.find((r or {}).get("symbol"), (r or {}).get("exchange")) if isinstance(r, dict) else None
        if inst and inst["symbol"] not in seen:
            seen.add(inst["symbol"])
            rows.append({"symbol": inst["symbol"], "exchange": inst["exchange"]})
    if len(rows) > model.TILES_MAX:
        return {"ok": False, "error": "at most %d tiles" % model.TILES_MAX}
    store.save_tiles(rows)
    model.invalidate()
    def fetch():
        try:
            market.refresh_quotes(model.quote_symbols(), _ssl_context())
            model.invalidate()
        except Exception as e:
            sys.stderr.write("bagholder tiles: quotes failed: %s\n" % e)
    threading.Thread(target=fetch, name="tiles-fetch", daemon=True).start()
    return {"ok": True, "tiles": model.tile_rows(model.base_model())}


def news_listings():
    """Every listing whose news is wanted: the market feed, the shares and funds held, and the watched ones."""
    base = model.base_model()
    seen, out = set(), [news.MARKET]
    # one listing, one read, under its bare ticker: the book's QNC.TO and the watchlist's QNC are the same wire
    for p in base.get("positions") or []:
        if p.get("kind") != "Shares":
            continue
        key = (market.tmx_symbol(p["symbol"]), _s(p.get("exchange")).upper())
        if key[0] and key not in seen:
            seen.add(key)
            out.append((key[0], p.get("exchange") or "", p.get("currency") or ""))
    for w in base.get("watchlist") or []:
        key = (market.tmx_symbol(w["symbol"]), _s(w.get("exchange")).upper())
        if key[0] and key not in seen and not instruments.find(w["symbol"], w.get("exchange")) and key[1] != "CRYPTO":
            seen.add(key)
            out.append((key[0], w.get("exchange") or "", w.get("currency") or ""))
    return out


@single_flight("news")
def refresh_news():
    """The wires for every listing whose news is older than fifteen minutes. Never raises."""
    try:
        n = news.refresh(news_listings(), _ssl_context())
        if n:
            model.invalidate()
        return n
    except Exception as e:
        sys.stderr.write("bagholder news: refresh failed: %s\n" % e)
        return 0


def news_loop():
    """At start, then every five minutes, each listing read once per fifteen."""
    while not _stop.is_set():
        refresh_news()
        if _stop.wait(300):
            return


@single_flight("universes")
def refresh_universes():
    """The market heatmaps' tiles: the TSX 60 from TMX, the US market from Nasdaq's screener. Never raises."""
    try:
        done = universes.refresh(_ssl_context())
        if done:
            model.invalidate()
        return done
    except Exception as e:
        sys.stderr.write("bagholder universes: refresh failed: %s\n" % e)
        return []


_universe_kick = threading.Event()


def universe_loop():
    """At start, then every thirty minutes, or sooner when the page asks for a universe it has never seen."""
    while not _stop.is_set():
        _universe_kick.clear()
        refresh_universes()
        _universe_kick.wait(1800)
        if _stop.is_set():
            return


def kick_universes():
    """The page shows a universe with no rows yet: read now rather than at the next half hour."""
    _universe_kick.set()
    return {"ok": True}


def open_orders_count():
    """The Orders panel's Pending cards: every entry resting at Wealthsimple, and every live bracket once its entry has filled."""
    entries = sum(1 for o in store.list_orders() if o["status"] in LIVE_STATUSES and o.get("role", "entry") == "entry")
    brackets = sum(1 for b in store.list_brackets() if b["status"] in BRACKET_LIVE and b["status"] != "waiting")
    return entries + brackets


def qty_text(q):
    return ("%d" % q) if float(q).is_integer() else ("%.6f" % q).rstrip("0").rstrip(".")


def status_payload():
    counts = store.status_counts()
    update = update_status()
    sess = load_session()
    with _lock:
        connected = bool(_state["connected"] and sess and sess.get("access_token"))
        return {
            "ok": True,
            "connected": connected,
            "email": _state["email"] or (sess or {}).get("email") or "",
            "lastSync": _state["lastSync"] or counts["syncedAt"] or "",
            "activityCount": counts["activityCount"],
            "accountCount": counts["accountCount"],
            "capturing": bool(_state["capturing"]),
            "syncing": bool(_state["syncing"]),
            "listingsFilling": bool(_state.get("listingsFilling")),
            "syncStep": _state.get("syncStep") or "",
            "error": _state["error"] or "",
            "dataVersion": store.data_version() + "|" + model.today_local(),
            "protocol": PROTOCOL,
            "startedAt": STARTED_AT,
            "version": APP_VERSION,
            "latestVersion": str(update.get("latest") or ""),
            "updateAvailable": bool(update.get("updateAvailable")),
            "updateUrl": IMAGE_PAGE if UPDATES_OFF else str(update.get("url") or REPO_URL),
            "canUpdate": can_update(),
            "updateBy": "image" if UPDATES_OFF else "app",
            "loginView": LOGIN_VIEW,
            "ordersLive": ORDERS_LIVE,
            "openOrders": open_orders_count(),
            "updating": str(_state.get("updating") or ""),
            "updateError": str(_state.get("updateError") or ""),
        }


def ledger_path():
    return Path(__file__).resolve().parent / "ledger.html"


def _payer_symbols():
    try:
        return model.payer_symbols()
    except Exception:
        return []


@single_flight("market", busy={})
def refresh_market_data():
    """USD/CAD, S&P 500 and declared distributions for the derived model. Never raises."""
    try:
        out = market.refresh_all(_ssl_context(), _payer_symbols())
        out["quotes"] = refresh_quotes()
        if out.get("distributions") or out.get("quotes"):
            model.invalidate()
        return out
    except Exception:
        return {"fx": 0, "benchmark": 0, "distributions": 0, "quotes": 0, "skipped": True}


@single_flight("quotes")
def refresh_quotes():
    """Prices for held positions and watched listings, at most every QUOTE_REFRESH_MINUTES. Never raises."""
    try:
        base = model.base_model()
        n = market.refresh_quotes(model.held_symbols(base) + model.quote_symbols(base), _ssl_context())
        if n:
            model.invalidate()
        return n
    except Exception:
        return 0


@single_flight("periodic")
def refresh_periodic_market():
    """USD/CAD, S&P 500 and declared distributions on their own clocks. Never raises."""
    try:
        out = market.refresh_periodic(_ssl_context(), _payer_symbols())
        if out.get("fx") or out.get("benchmark") or out.get("distributions"):
            model.invalidate()
        return out
    except Exception:
        return {"fx": 0, "benchmark": 0, "distributions": 0, "skipped": True}


@single_flight("archive", busy=())
def archive_intraday_bars(limit=None):
    """Keep intraday bars for everything traded or held in the past year, a few
    instruments per call so the sources are never hammered. Never raises."""
    try:
        recs = model.intraday_archive_symbols()
        limit = market.ARCHIVE_BATCH if limit is None else max(1, int(limit))
        return (market.archive_daily(recs, _ssl_context(), limit=limit)
                + market.archive_intraday(recs, _ssl_context(), limit=limit))
    except Exception:
        return []


# The backfill is paced by the processor time it actually costs, not by a count
# of instruments: the same batch is a blink on a laptop and seconds on a small
# board, and only the board should slow down. After each pass the loop rests as
# long as that pass burned, so the backfill never takes more than half of one
# core whatever the machine, and a quick one still sweeps in well under a
# minute. Waiting on a source is not processor time, so a slow network does not
# slow the sweep down.
ARCHIVE_DUTY = 1.0
ARCHIVE_PASS_SEC = 1.0      # a pass this long keeps the page from ever queueing behind one
ARCHIVE_MIN_SEC = 0.5
ARCHIVE_IDLE_SEC = 5 * 60   # nothing to do: look again in five minutes


def _cpu_clock():
    """Processor time of the calling thread, or the wall clock where the
    platform has no such counter."""
    try:
        return time.thread_time()
    except (AttributeError, OSError):
        return time.monotonic()


def archive_loop():
    """Sweeps the archive until every instrument is kept, then tops up once a day
    per instrument. Each pass is measured: the batch grows on a machine that
    swallows it and shrinks on one that labours, and the rest between passes is
    proportional to the work done, so a fast machine finishes the sweep in
    seconds and a slow one stays answerable while it catches up."""
    delay = 20
    batch = market.ARCHIVE_BATCH
    while not _stop.wait(delay):
        started = _cpu_clock()
        worked = archive_intraday_bars(limit=batch)
        spent = max(0.0, _cpu_clock() - started)
        if not worked:
            delay = ARCHIVE_IDLE_SEC
            batch = market.ARCHIVE_BATCH
            continue
        if spent > ARCHIVE_PASS_SEC and batch > 1:
            batch = max(1, batch // 2)
        elif spent < ARCHIVE_PASS_SEC / 3 and batch < market.ARCHIVE_BATCH:
            batch = min(market.ARCHIVE_BATCH, batch * 2)
        delay = max(ARCHIVE_MIN_SEC, spent * ARCHIVE_DUTY)


def quote_loop():
    """Prices, every QUOTE_REFRESH_MINUTES."""
    while not _stop.wait(60 * market.QUOTE_REFRESH_MINUTES):
        refresh_quotes()


def market_loop():
    """Declared distributions (20-hour records), USD/CAD and the S&P 500 (6 hours),
    checked once an hour on their own clock, apart from prices."""
    while not _stop.wait(60 * market.MARKET_CHECK_MINUTES):
        refresh_periodic_market()
        check_for_update_if_due()


WATCH_SCAN_SEC = 10 * 60


def scan_watched_folder():
    """Import new or changed CSVs from the watched folder. Never raises."""
    try:
        if not csvimport.watch_folder():
            return None
        result = csvimport.scan_folder()
        if result.get("ok") and result.get("added"):
            model.invalidate(book=True)
        return result
    except Exception:
        return None


def watch_loop():
    scan_watched_folder()
    while not _stop.wait(WATCH_SCAN_SEC):
        scan_watched_folder()


def sync_then_market():
    ok = run_sync()
    refresh_market_data()
    return ok


def parse_version(tag):
    """'v1.2.3' -> (1, 2, 3); anything else -> None."""
    m = re.match(r"^v?(\d+)\.(\d+)\.(\d+)$", str(tag or "").strip())
    return tuple(int(x) for x in m.groups()) if m else None


def check_for_update(now=None):
    """Ask GitHub for the latest release and compare its tag with APP_VERSION.
    Returns the record stored in meta: {checkedAt, ok, latest, url, updateAvailable}. Never raises."""
    now = now or datetime.now(timezone.utc)
    record = {"checkedAt": now.strftime("%Y-%m-%dT%H:%M:%SZ"), "ok": False, "latest": "", "url": REPO_URL + "/releases/latest", "updateAvailable": False}
    try:
        rel = _http_json("GET", RELEASE_URL, headers={"Accept": "application/vnd.github+json", "User-Agent": "Bagholder/" + APP_VERSION}, timeout=30)
        latest = parse_version((rel or {}).get("tag_name"))
        if latest is None:
            raise ValueError("no release")
        record.update({"ok": True, "latest": str(rel.get("tag_name")), "url": str(rel.get("html_url") or record["url"]), "updateAvailable": latest > parse_version(APP_VERSION)})
        record["assets"] = release_assets(rel)
    except Exception:
        pass
    try:
        store.set_meta("update_check", json.dumps(record))
    except Exception:
        pass
    return record


def update_status():
    try:
        raw = store.get_meta("update_check")
        rec = json.loads(raw) if raw else {}
        return rec if isinstance(rec, dict) else {}
    except Exception:
        return {}


def release_assets(rel):
    """{zip, sha} download URLs of a release's web archive and its .sha256, when present.

    The archive is bagholder-<tag>-web.zip; releases before the name carried a
    platform were bagholder-<tag>.zip, still accepted. Other assets on the page
    (the Android build, bagholder-<tag>-android.apk) are ignored.
    """
    tag = str((rel or {}).get("tag_name") or "")
    found = {}
    for a in (rel or {}).get("assets") or []:
        found[str(a.get("name") or "")] = str(a.get("browser_download_url") or "")
    for stem in ("bagholder-%s-web.zip" % tag, "bagholder-%s.zip" % tag):
        if found.get(stem) and found.get(stem + ".sha256"):
            return {"zip": found[stem], "sha": found[stem + ".sha256"]}
    return {}


# --------------------------------------------------------------------------
# in-app update
# --------------------------------------------------------------------------


def update_mode():
    """'git' when this copy is a git checkout with git on the path, else 'release'."""
    return "git" if (APP_DIR / ".git").exists() and shutil.which("git") else "release"


def _git(*args):
    return subprocess.run(["git"] + list(args), cwd=str(APP_DIR), capture_output=True, text=True, timeout=120)


def git_update_ready():
    """(ok, reason) for updating a git checkout: clean tree on master."""
    try:
        if _git("status", "--porcelain").stdout.strip():
            return False, "This copy is a git checkout with local changes; pull it yourself."
        if _git("rev-parse", "--abbrev-ref", "HEAD").stdout.strip() != "master":
            return False, "This copy is a git checkout on another branch; pull it yourself."
        return True, ""
    except Exception as e:
        return False, "git: %s" % e


def can_update(rec=None):
    rec = rec if rec is not None else update_status()
    if not rec.get("updateAvailable") or UPDATES_OFF:
        return False
    if update_mode() == "git":
        return git_update_ready()[0]
    return bool(rec.get("assets"))


def _download(url, dest, max_bytes=UPDATE_MAX_BYTES):
    req = Request(url, headers={"User-Agent": "Bagholder/" + APP_VERSION, "Accept": "application/octet-stream"})
    with urlopen(req, timeout=120, context=_ssl_context()) as resp, open(dest, "wb") as f:
        total = 0
        while True:
            chunk = resp.read(65536)
            if not chunk:
                break
            total += len(chunk)
            if total > max_bytes:
                raise ValueError("release archive is larger than expected")
            f.write(chunk)


def _extract_release(zip_path, staging):
    """The archive's regular files into `staging`, refusing anything that would
    land outside it. Returns the relative paths written."""
    import zipfile
    written = []
    with zipfile.ZipFile(zip_path) as z:
        for info in z.infolist():
            name = info.filename
            if info.is_dir() or not name or name.startswith("/") or ".." in name.split("/"):
                continue
            target = staging / name
            target.parent.mkdir(parents=True, exist_ok=True)
            with z.open(info) as src, open(target, "wb") as dst:
                shutil.copyfileobj(src, dst)
            written.append(name)
    if not written:
        raise ValueError("release archive is empty")
    return written


def _check_python(staging, names):
    import py_compile
    for name in names:
        if name.endswith(".py"):
            py_compile.compile(str(staging / name), doraise=True)


def _install_files(staging, names, tag):
    """Keep the current copies under HOME/previous, then put the new files in
    place, and leave the marker the supervisor watches for."""
    previous = HOME / "previous"
    if previous.exists():
        shutil.rmtree(previous)
    for name in names:
        cur = APP_DIR / name
        if cur.exists():
            (previous / name).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(cur, previous / name)
    try:
        for name in names:
            (APP_DIR / name).parent.mkdir(parents=True, exist_ok=True)
            os.replace(staging / name, APP_DIR / name)
    except Exception:
        # a replace failed part way (a file held open, a permission): every file
        # goes back to its previous copy before the failure is reported
        _rollback()
        raise
    (HOME / "update-pending").write_text(tag)


def _rollback():
    """Put the previous copies back (a restarted server that died at once)."""
    previous = HOME / "previous"
    if not previous.exists():
        return False
    for p in previous.rglob("*"):
        if p.is_file():
            rel = p.relative_to(previous)
            (APP_DIR / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(p, APP_DIR / rel)
    shutil.rmtree(previous, ignore_errors=True)
    return True


def request_restart():
    """Finish the response in flight, then stop serving so the supervisor restarts us."""
    _exit_code[0] = RESTART_CODE
    def go():
        time.sleep(0.5)
        _stop.set()
        try:
            if _httpd is not None:
                _httpd.shutdown()
        except Exception:
            pass
    threading.Thread(target=go, name="bagholder-restart", daemon=True).start()


def perform_update(tag, rec):
    """Bring this copy to `tag`: a git checkout pulls, anything else downloads the
    release, checks it, swaps the files. Then a restart. Never raises; failures
    land in _state['updateError'] and nothing is changed."""
    try:
        if update_mode() == "git":
            with _lock:
                _state["updating"] = "Updating to %s…" % tag
            ok, why = git_update_ready()
            if not ok:
                raise RuntimeError(why)
            r = _git("pull", "--ff-only")
            if r.returncode != 0:
                raise RuntimeError("git pull failed: " + (r.stderr or r.stdout).strip()[:200])
            (HOME / "update-pending").write_text(tag)
        else:
            assets = rec.get("assets") or {}
            if not assets:
                raise RuntimeError("This release has no downloadable archive.")
            with _lock:
                _state["updating"] = "Downloading %s…" % tag
            staging = HOME / "staging"
            if staging.exists():
                shutil.rmtree(staging)
            staging.mkdir(parents=True)
            zip_path = HOME / ("bagholder-%s.zip" % tag)
            _download(assets["zip"], zip_path)
            _download(assets["sha"], HOME / "release.sha256", max_bytes=4096)
            import hashlib
            want = (HOME / "release.sha256").read_text().split()[0].strip().lower()
            got = hashlib.sha256(zip_path.read_bytes()).hexdigest()
            if want != got:
                raise RuntimeError("The download did not match the release's checksum.")
            with _lock:
                _state["updating"] = "Installing %s…" % tag
            names = _extract_release(zip_path, staging)
            _check_python(staging, names)
            _install_files(staging, names, tag)
            shutil.rmtree(staging, ignore_errors=True)
            try:
                zip_path.unlink()
                (HOME / "release.sha256").unlink()
            except Exception:
                pass
        with _lock:
            _state["updating"] = "Restarting…"
        sys.stderr.write("bagholder update: %s installed, restarting\n" % tag)
        request_restart()
    except Exception as e:
        with _lock:
            _state["updating"] = ""
            _state["updateError"] = "Update failed: %s" % e
        sys.stderr.write("bagholder update failed: %s\n" % e)


def start_update():
    """Begin the update the page asked for; the work runs in the background."""
    if UPDATES_OFF:
        return {"ok": False, "error": UPDATES_OFF_MESSAGE}
    rec = update_status()
    with _lock:
        if _state.get("updating"):
            return {"ok": True}
        if _state.get("syncing"):
            return {"ok": False, "error": "Wait for the sync to finish, then update."}
        if not rec.get("updateAvailable") or not rec.get("latest"):
            return {"ok": False, "error": "No update to install."}
        if not can_update(rec):
            return {"ok": False, "error": git_update_ready()[1] if update_mode() == "git" else "This release has no downloadable archive."}
        _state["updateError"] = ""
        _state["updating"] = "Updating to %s…" % rec["latest"]
    threading.Thread(target=perform_update, args=(rec["latest"], rec), name="bagholder-update", daemon=True).start()
    return {"ok": True}


def supervise(spawn=None, healthy_sec=UPDATE_HEALTHY_SEC):
    """Run the server as a child and start it again whenever it exits asking to
    be restarted (an update). A restarted server that dies within `healthy_sec`
    of an update gets the previous files put back and is started once more."""
    def default_spawn():
        env = dict(os.environ)
        env["BAGHOLDER_CHILD"] = "1"
        return subprocess.Popen([sys.executable, str(Path(__file__).resolve())], env=env)
    spawn = spawn or default_spawn
    marker = HOME / "update-pending"
    while True:
        child = spawn()
        pending = marker.exists()
        try:
            if pending:
                try:
                    child.wait(timeout=healthy_sec)
                except subprocess.TimeoutExpired:
                    # alive past the window: the update took
                    try:
                        marker.unlink()
                    except Exception:
                        pass
                    shutil.rmtree(HOME / "previous", ignore_errors=True)
                    pending = False
                    child.wait()
            else:
                child.wait()
        except KeyboardInterrupt:
            try:
                child.terminate()
                child.wait(timeout=10)
            except Exception:
                pass
            return 0
        code = child.returncode
        if code == RESTART_CODE:
            continue
        if pending and code != 0:
            try:
                marker.unlink()
            except Exception:
                pass
            if _rollback():
                sys.stderr.write("bagholder update: the new version did not start; the previous one is back\n")
                continue
        return code or 0


def check_for_update_if_due(now=None):
    now = now or datetime.now(timezone.utc)
    rec = update_status()
    try:
        last = datetime.fromisoformat(str(rec.get("checkedAt") or "").replace("Z", "+00:00"))
        if now - last < timedelta(hours=UPDATE_CHECK_HOURS):
            return rec
    except ValueError:
        pass
    return check_for_update(now)


def history_payload(query):
    """Daily bars for one instrument over a date span, fetched and cached on demand."""
    q = parse_qs(query or "")
    one = lambda k: (q.get(k) or [""])[0].strip()
    rec = {"symbol": one("symbol"), "exchange": one("exchange"), "currency": one("currency") or "CAD", "kind": one("kind") or "Shares"}
    start, end = one("from")[:10], one("to")[:10]
    tf = one("tf") or "1d"
    if not rec["symbol"] or len(start) != 10 or len(end) != 10 or tf not in market.TIMEFRAMES:
        return {"ok": False, "error": "symbol, from, to and a known tf are required"}
    inst = market.chart_instrument(rec)
    src = market.history_source(inst)
    available = market.offered_timeframes(inst, start)
    pending = False
    try:
        if not (src and tf in available):
            bars = []
        elif tf in market.INTRADAY_SECONDS and not market.intraday_ready(inst, tf, start):
            # never block the chart on a minute-data fetch: hand back what is stored,
            # fetch the rest in the background, and let the page ask again
            market.ensure_intraday_in_background(inst, tf, start, end, _ssl_context())
            pending = True
            bars = []
        else:
            bars = market.ensure_bars(inst, tf, start, end, _ssl_context())
    except Exception:
        bars = []
    return {"ok": True, "symbol": rec["symbol"], "chartSymbol": inst["symbol"], "source": src[0] if src else "",
            "tf": tf, "available": available, "bars": bars, "pending": pending,
            "reason": "" if bars or pending else market.chart_reason(inst, tf)}


def _query_param(query, name):
    return ((parse_qs(query or "").get(name) or [""])[0] or "").strip() or None


def _model_filters(query):
    raw = (parse_qs(query or "").get("filters") or [""])[0]
    if not raw:
        return None
    try:
        return json.loads(raw)
    except ValueError:
        return None


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        sys.stderr.write("bagholder %s - %s\n" % (self.address_string(), fmt % args))

    def _local(self):
        if BIND_HOST != "127.0.0.1":
            return True   # bound beyond loopback on purpose (a container); the peer is its bridge
        ip = self.client_address[0]
        return ip in ("127.0.0.1", "::1")

    def _host_ok(self):
        raw = (self.headers.get("Host") or "").strip().lower()
        if not raw or "," in raw:
            return False
        if BIND_HOST != "127.0.0.1":
            # a container's port may be published under another number; the name must still be 127.0.0.1
            return bool(re.fullmatch(r"127\.0\.0\.1:\d{1,5}", raw))
        port = self.server.server_address[1]
        return raw == "127.0.0.1:%s" % port

    def _write_ok(self):
        site = (self.headers.get("Sec-Fetch-Site") or "").strip().lower()
        if site == "same-origin":
            return True
        return bool((self.headers.get("X-Bagholder") or "").strip())

    def _gate(self, write=False):
        if not self._local() or not self._host_ok():
            return False
        if write and not self._write_ok():
            return False
        return True

    def _send(self, code, body, content_type="application/json; charset=utf-8"):
        if isinstance(body, (dict, list)):
            raw = json.dumps(body).encode("utf-8")
        elif isinstance(body, str):
            raw = body.encode("utf-8")
        else:
            raw = body
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(raw)

    def _read_json(self):
        try:
            n = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            n = 0
        if n < 0 or n > 1_048_576:
            return {}
        raw = self.rfile.read(n) if n else b""
        if not raw:
            return {}
        try:
            return json.loads(raw.decode("utf-8"))
        except ValueError:
            return {}

    def do_OPTIONS(self):
        self._send(403, {"ok": False})

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == "/api/login/stream":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            self.send_response(200)
            self.send_header("Content-Type", "multipart/x-mixed-replace; boundary=frame")
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.end_headers()
            try:
                login_stream(lambda chunk: (self.wfile.write(chunk), self.wfile.flush()), lambda: not self.wfile.closed)
            except (BrokenPipeError, ConnectionResetError, OSError):
                pass
            return
        if path == "/api/login/frame":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            data = login_frame()
            if data:
                self._send(200, data, "image/jpeg")
            else:
                self._send(204, b"")
            return
        if path in ("/", "/index.html", "/ledger.html", "/v2", "/v2/"):
            if not self._gate():
                self._send(403, {"ok": False})
                return
            p = ledger_path()
            try:
                data = p.read_bytes()
            except OSError:
                self._send(404, {"ok": False, "error": "ledger.html missing"})
                return
            self._send(200, data, "text/html; charset=utf-8")
            return
        if path == "/api/order/quote":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            qs = parse_qs(self.path.split("?", 1)[1] if "?" in self.path else "")
            first = lambda k: (qs.get(k) or [""])[0]
            self._send(200, ticket_quote(first("symbol"), first("security"), first("account"), first("exchange")))
            return
        if path == "/api/symbols/search":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            qs = parse_qs(self.path.split("?", 1)[1] if "?" in self.path else "")
            self._send(200, symbol_search((qs.get("q") or [""])[0]))
            return
        if path == "/api/symbols/quote":
            # a glance at a listing the watchlist's add row offers: its price and day change, not stored
            if not self._gate():
                self._send(403, {"ok": False})
                return
            query = self.path.split("?", 1)[1] if "?" in self.path else ""
            rec = {"symbol": _query_param(query, "symbol") or "", "exchange": _query_param(query, "exchange") or "", "currency": _query_param(query, "currency") or "", "kind": "Shares"}
            q = market.peek_quote(rec, _ssl_context()) if rec["symbol"] else None
            self._send(200, dict({"ok": True, "price": None, "priceChange": None, "percentChange": None}, **(q or {})))
            return
        if path == "/api/orders":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            self._send(200, orders_payload(kick=True))
            return
        if path == "/api/status":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            self._send(200, status_payload())
            return
        if path == "/api/history":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            self._send(200, history_payload(self.path.split("?", 1)[1] if "?" in self.path else ""))
            return
        if path == "/lightweight-charts.js":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            lib = Path(__file__).resolve().parent / "lightweight-charts.js"
            try:
                data = lib.read_bytes()
            except OSError:
                self._send(404, {"ok": False, "error": "lightweight-charts.js missing"})
                return
            self._send(200, data, "application/javascript; charset=utf-8")
            return
        if path == "/api/watch":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            self._send(200, csvimport.status())
            return
        if path == "/api/data":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            summary = store.data_summary()
            summary["ok"] = True
            summary["sessionPresent"] = bool(load_session())
            self._send(200, summary)
            return
        if path == "/api/model":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            query = self.path.split("?", 1)[1] if "?" in self.path else ""
            try:
                if market.is_stale(symbols=_payer_symbols()):
                    kick("market", refresh_market_data)
                elif market.quote_symbols_needing_refresh(model.held_symbols() + model.quote_symbols()):   # the watchlist's and the tile row's quotes too
                    kick("quotes", refresh_quotes)
            except Exception:
                pass
            try:
                payload = model.view(_model_filters(query), _query_param(query, "trade"))
            except Exception as e:
                sys.stderr.write("model failed: %r\n" % (e,))
                self._send(500, {"ok": False, "error": "model failed: %s" % type(e).__name__})
                return
            payload["status"] = status_payload()
            self._send(200, payload)
            return
        if path == "/api/trade":
            # the legs and fills of one trade or holding, fetched when its page opens
            if not self._gate():
                self._send(403, {"ok": False})
                return
            query = self.path.split("?", 1)[1] if "?" in self.path else ""
            try:
                detail = model.trade_detail(_query_param(query, "id"))
            except Exception as e:
                sys.stderr.write("model failed: %r\n" % (e,))
                self._send(500, {"ok": False, "error": "model failed: %s" % type(e).__name__})
                return
            if detail is None:
                self._send(404, {"ok": False, "error": "no such trade"})
                return
            detail["ok"] = True
            self._send(200, detail)
            return
        if path in ("/favicon.png", "/favicon.ico"):
            if not self._gate():
                self._send(403, {"ok": False})
                return
            icon = Path(__file__).resolve().parent / "favicon.png"
            try:
                data = icon.read_bytes()
            except OSError:
                self._send(404, {"ok": False, "error": "favicon missing"})
                return
            self._send(200, data, "image/png")
            return
        if path == "/api/book":
            if not self._gate():
                self._send(403, {"ok": False})
                return
            book = load_book()
            self._send(
                200,
                {
                    "ok": True,
                    "activities": book.get("activities") or [],
                    "accounts": book.get("accounts") or [],
                    "balances": book.get("balances") or [],
                    "navHistory": book.get("navHistory") or [],
                    "navByAccount": book.get("navByAccount") or {},
                    "syncedAt": book.get("syncedAt") or "",
                    "tradeGroups": book.get("tradeGroups") or [],
                    "notes": book.get("notes") or {},
                    "securities": book.get("securities") or [],
                },
            )
            return
        self._send(404, {"ok": False, "error": "not found"})

    def do_POST(self):
        path = self.path.split("?", 1)[0]
        if not self._gate(write=True):
            self._send(403, {"ok": False})
            return
        if path == "/api/login/start":
            self._read_json()
            result = start_login_browser()
            self._send(200 if result.get("ok") else 200, result)
            return
        if path == "/api/login/cancel":
            self._read_json()
            self._send(200, cancel_login())
            return
        if path == "/api/login/input":
            self._send(200, login_input(self._read_json() or {}))
            return
        if path == "/api/update":
            self._read_json()
            self._send(200, start_update())
            return
        if path == "/api/capture":
            body = self._read_json()
            result = capture_tokens(body)
            self._send(200, result)
            return
        if path == "/api/refresh":
            self._read_json()
            result = refresh_now()
            self._send(200, result)
            return
        if path == "/api/sync":
            self._read_json()
            sess = load_session()
            if not sess:
                self._send(200, {"ok": False, "error": "not connected"})
                return
            with _lock:
                _state["error"] = ""
            threading.Thread(target=sync_then_market, name="bagholder-sync", daemon=True).start()
            self._send(200, {"ok": True, "syncing": True})
            return
        if path == "/api/data/clear":
            body = self._read_json()
            body = body if isinstance(body, dict) else {}
            with _lock:
                if _state["syncing"]:
                    self._send(409, {"ok": False, "error": "A sync is running. Wait for it to finish."})
                    return
            summary = store.clear_synced_data(
                keep_journal=not bool(body.get("journal")),
                keep_market=not bool(body.get("market")),
            )
            if body.get("session"):
                delete_session_and_book()
            with _lock:
                _state["lastSync"] = ""
                _state["error"] = ""
            model.invalidate(book=True)
            summary["ok"] = True
            summary["sessionPresent"] = bool(load_session())
            self._send(200, summary)
            return
        if path == "/api/order":
            body = self._read_json()
            self._send(200, place_order(body))
            return
        if path == "/api/order/cancel":
            body = self._read_json()
            self._send(200, cancel_order((body or {}).get("id") if isinstance(body, dict) else ""))
            return
        if path == "/api/order/modify":
            body = self._read_json()
            body = body if isinstance(body, dict) else {}
            self._send(200, modify_order(body.get("id"), body.get("quantity"), body.get("limitPrice")))
            return
        if path == "/api/bracket/adjust":
            body = self._read_json()
            body = body if isinstance(body, dict) else {}
            self._send(200, adjust_bracket(body.get("id"), body.get("leg"), body.get("price"), body.get("trail"), bool(body.get("remove"))))
            return
        if path == "/api/bracket/cancel":
            body = self._read_json()
            self._send(200, cancel_bracket((body or {}).get("id") if isinstance(body, dict) else ""))
            return
        if path == "/api/orders/refresh":
            self._read_json()
            r = refresh_orders()
            r.update(orders_payload())
            self._send(200, r)
            return
        if path == "/api/markets/refresh":
            self._send(200, kick_universes())
            return
        if path == "/api/watchlist/add":
            self._send(200, watch_add(self._read_json()))
            return
        if path == "/api/watchlist/remove":
            self._send(200, watch_remove(self._read_json()))
            return
        if path == "/api/tiles/set":
            self._send(200, tiles_set(self._read_json()))
            return
        if path == "/api/journal":
            body = self._read_json()
            if not isinstance(body, dict) or not _s(body.get("id")).strip():
                self._send(400, {"ok": False, "error": "id required"})
                return
            entries = store.save_journal_entry(
                body.get("id"),
                {
                    "thesis": body.get("thesis"),
                    "tags": body.get("tags"),
                    "grade": body.get("grade"),
                },
            )
            model.apply_journal(entries)
            self._send(200, {"ok": True, "journal": entries})
            return
        if path == "/api/disconnect":
            self._read_json()
            delete_session_and_book()
            self._send(200, {"ok": True})
            return
        if path == "/api/book/append":
            body = self._read_json()
            result = append_manual(body)
            model.invalidate(book=True)
            self._send(200, result)
            return
        if path == "/api/import":
            body = self._read_json()
            body = body if isinstance(body, dict) else {}
            text = body.get("text")
            if not isinstance(text, str) or not text.strip():
                self._send(400, {"ok": False, "error": "text required"})
                return
            report = csvimport.import_text(_s(body.get("name")) or "upload.csv", text)
            if report.get("added"):
                model.invalidate(book=True)
            self._send(200, report)
            return
        if path == "/api/watch":
            body = self._read_json()
            body = body if isinstance(body, dict) else {}
            set_result = csvimport.set_watch_folder(body.get("path"))
            if not set_result.get("ok"):
                self._send(400, set_result)
                return
            result = csvimport.scan_folder(force=True)
            if result.get("added"):
                model.invalidate(book=True)
            result["status"] = csvimport.status()
            self._send(200, result)
            return
        if path == "/api/watch/scan":
            self._read_json()
            result = csvimport.scan_folder(force=True)
            if result.get("ok") and result.get("added"):
                model.invalidate(book=True)
            result["status"] = csvimport.status()
            self._send(200 if result.get("ok") else 400, result)
            return
        if path == "/api/watch/clear":
            self._read_json()
            csvimport.clear_watch_folder()
            self._send(200, csvimport.status())
            return
        if path == "/api/groups":
            body = self._read_json()
            groups = store.save_trade_groups(body.get("groups") if isinstance(body, dict) else [])
            self._send(200, {"ok": True, "groups": groups})
            return
        if path == "/api/notes":
            body = self._read_json()
            notes = store.save_trade_notes(body.get("notes") if isinstance(body, dict) else {})
            self._send(200, {"ok": True, "notes": notes})
            return
        self._send(404, {"ok": False, "error": "not found"})



def refresh_now():
    """Token POST only. Always POST, even if expiry is not near."""
    sess = load_session()
    if not sess or not sess.get("refresh_token"):
        with _lock:
            _state["connected"] = False
            _state["error"] = "not connected"
        return {"ok": False, "error": "not connected", "connected": False}
    ok = refresh_session(sess)
    with _lock:
        _state["connected"] = bool(ok)
        if ok:
            _state["error"] = ""
        err = (_state.get("error") or "").strip()
    return {"ok": bool(ok), "error": err, "connected": bool(ok)}


def auto_sync_loop():
    delay = TOKEN_CHECK_SEC
    fail_delay = TOKEN_CHECK_SEC
    while not _stop.wait(delay):
        sess = load_session()
        if sess and sess.get("refresh_token"):
            try:
                ensure_fresh_token(sess)
            except Exception:
                pass
        with _lock:
            connected = _state["connected"]
            syncing = _state["syncing"]
        if connected and not syncing and store.activity_pull_due(interval_sec=ACTIVITY_PULL_SEC):
            try:
                ok = run_sync(force_activity=True)
            except Exception:
                ok = False
            refresh_market_data()
            fail_delay = TOKEN_CHECK_SEC if ok else min(max(fail_delay, TOKEN_CHECK_SEC) * 2, 1800)
            delay = fail_delay
        else:
            delay = TOKEN_CHECK_SEC
            fail_delay = TOKEN_CHECK_SEC


def port_choices():
    """The ports to try: BAGHOLDER_PORT when set (a second instance for testing
    beside the live app), else the usual three."""
    env = (os.environ.get("BAGHOLDER_PORT") or "").strip()
    if env.isdigit() and 1024 <= int(env) <= 65535:
        return (int(env),)
    return PORTS


def bind_server():
    last = None
    for port in port_choices():
        try:
            httpd = ThreadingHTTPServer((BIND_HOST, port), Handler)
            return httpd, port
        except OSError as e:
            last = e
            continue
    raise SystemExit("Could not bind %s:%s (%s)" % (BIND_HOST, "-".join(str(p) for p in port_choices()), last))


def main():
    global _httpd
    _ensure_home()
    store.ensure()
    boot_session()
    httpd, port = bind_server()
    _httpd = httpd
    t = threading.Thread(target=auto_sync_loop, name="bagholder-auto-sync", daemon=True)
    t.start()
    threading.Thread(target=refresh_market_data, name="bagholder-market", daemon=True).start()
    threading.Thread(target=check_for_update, name="bagholder-update-check", daemon=True).start()
    threading.Thread(target=quote_loop, name="bagholder-quote-loop", daemon=True).start()
    threading.Thread(target=portfolio_loop, name="bagholder-portfolio-loop", daemon=True).start()
    threading.Thread(target=orders_loop, name="bagholder-orders-loop", daemon=True).start()
    threading.Thread(target=bracket_loop, name="bagholder-bracket-loop", daemon=True).start()
    threading.Thread(target=exposure_loop, name="bagholder-exposure-loop", daemon=True).start()
    # rows added before the bare-ticker convention (Wealthsimple's `.TO` on a dual listing) take it now
    for w in store.list_watchlist():
        bare = market.tmx_symbol(w["symbol"])
        if bare and bare != w["symbol"]:
            store.remove_watch(w["symbol"], w.get("exchange"))
            store.add_watch(bare, w.get("exchange"), w.get("name"), w.get("currency"), w.get("securityId"), now=w.get("addedAt"))
    threading.Thread(target=news_loop, name="bagholder-news-loop", daemon=True).start()
    threading.Thread(target=universe_loop, name="bagholder-universe-loop", daemon=True).start()
    threading.Thread(target=market_loop, name="bagholder-market-loop", daemon=True).start()
    threading.Thread(target=archive_loop, name="bagholder-archive", daemon=True).start()
    threading.Thread(target=watch_loop, name="bagholder-watch", daemon=True).start()
    url = "http://127.0.0.1:%s" % port
    print("Bagholder  %s" % url, flush=True)
    # A second instance run for verification (BAGHOLDER_NO_BROWSER=1) must not open
    # anyone's browser: the user's own copy is what they are looking at.
    if not (os.environ.get("BAGHOLDER_NO_BROWSER") or "").strip():
        try:
            webbrowser.open(url)
        except Exception:
            pass
    if _state.get("connected"):
        if store.activity_pull_due(interval_sec=ACTIVITY_PULL_SEC):
            threading.Thread(target=run_sync, name="bagholder-boot-sync", daemon=True).start()
        else:
            threading.Thread(
                target=fill_listings,
                args=(load_session(),),
                name="bagholder-listings",
                daemon=True,
            ).start()
    # a container stops its process with SIGTERM, which PID 1 would otherwise ignore: same exit as Ctrl-C
    def _on_sigterm(*_):
        raise KeyboardInterrupt
    try:
        signal.signal(signal.SIGTERM, _on_sigterm)
    except (ValueError, OSError):
        pass
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\nBagholder stopped.", flush=True)
    finally:
        _stop.set()
        try:
            httpd.shutdown()
        except Exception:
            pass
        try:
            httpd.server_close()
        except Exception:
            pass
    if _exit_code[0]:
        sys.exit(_exit_code[0])


if __name__ == "__main__":
    if os.environ.get("BAGHOLDER_CHILD") == "1" or UPDATES_OFF:
        # the supervisor exists to restart an updated server; a copy that never updates runs plain
        main()
    else:
        sys.exit(supervise())
