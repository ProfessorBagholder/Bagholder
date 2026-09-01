import Foundation

enum WSPullError: Error {
    case unauthorized
    case noIdentity
    case noSession
    case graphql(String)
}

struct WSSession {
    var accessToken: String
    var refreshToken: String
    var clientId: String
    var identityCanonicalId: String
    var expiresAt: String
    var sessionId: String
    var wssdi: String
}

struct WSActivity: Equatable, Codable {
    var id: String
    var canonicalId: String
    var occurredAt: String
    var transactionDate: String
    var accountId: String
    var fifoId: String
    var accountType: String
    var activityType: String
    var activitySubType: String
    var description: String
    var direction: String
    var symbol: String
    var name: String
    var currency: String
    var quantity: Double
    var unitPrice: Double
    var commission: Double
    var netCashAmount: Double
    var category: String
    var rawType: String
    var aftType: String
    var counterSymbol: String
}

struct WSClosedTrade: Identifiable, Codable {
    var id: String
    var accountId: String
    var accountType: String
    var symbol: String
    var name: String
    var currency: String
    var side: String
    var quantity: Double
    var entryPrice: Double
    var exitPrice: Double
    var entryDate: String
    var exitDate: String
    var holdDays: Int
    var commission: Double
    var entryCommission: Double
    var exitCommission: Double
    var pnl: Double
    var pnlCad: Double
    var openDirection: String
    var buyActivityId: String
    var sellActivityId: String
    /// FIFO member rows for a grouped Closed trades line. Not persisted.
    var slices: [WSClosedTrade] = []
    var displaySide: String { openDirection == "SHORT" ? "COVER" : side }

    enum CodingKeys: String, CodingKey {
        case id, accountId, accountType, symbol, name, currency, side, quantity
        case entryPrice, exitPrice, entryDate, exitDate, holdDays, commission
        case entryCommission, exitCommission, pnl, pnlCad, openDirection
        case buyActivityId, sellActivityId
    }
}

struct WSNavPoint: Codable {
    var date: String
    var equity: Double
    var currency: String
    var netDeposits: Double?
}

struct WSMetrics: Codable {
    var realizedPnlCad: Double
    var tradeCount: Int
    var winCount: Int
    var lossCount: Int
    var evenCount: Int
    var grossProfit: Double
    var grossLoss: Double
    var winRate: Double
    var profitFactor: Double
    var avgWin: Double
    var avgLoss: Double
    var expectancy: Double
    var maxWinPnl: Double
    var maxWinSymbol: String
    var maxLossPnl: Double
    var maxLossSymbol: String
    var avgHoldDays: Double
}

struct WSMonthBar: Codable {
    var month: String
    var pnl: Double
}

struct WSYearRow: Codable {
    var year: String
    var ret: String
    var spy: String
    var vs: String
}

struct WSPullResult: Codable {
    var closed: [WSClosedTrade]
    var metrics: WSMetrics
    var nav: [WSNavPoint]
    var monthly: [WSMonthBar]
    var years: [WSYearRow]
    var avgAnnualized: String
    var avgAnnualizedSubtitle: String
    var activities: [WSActivity] = []

    enum CodingKeys: String, CodingKey {
        case closed, metrics, nav, monthly, years, avgAnnualized, avgAnnualizedSubtitle, activities
    }

    init(
        closed: [WSClosedTrade],
        metrics: WSMetrics,
        nav: [WSNavPoint],
        monthly: [WSMonthBar],
        years: [WSYearRow],
        avgAnnualized: String,
        avgAnnualizedSubtitle: String,
        activities: [WSActivity] = []
    ) {
        self.closed = closed
        self.metrics = metrics
        self.nav = nav
        self.monthly = monthly
        self.years = years
        self.avgAnnualized = avgAnnualized
        self.avgAnnualizedSubtitle = avgAnnualizedSubtitle
        self.activities = activities
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        closed = try c.decode([WSClosedTrade].self, forKey: .closed)
        metrics = try c.decode(WSMetrics.self, forKey: .metrics)
        nav = try c.decode([WSNavPoint].self, forKey: .nav)
        monthly = try c.decode([WSMonthBar].self, forKey: .monthly)
        years = try c.decode([WSYearRow].self, forKey: .years)
        avgAnnualized = try c.decode(String.self, forKey: .avgAnnualized)
        avgAnnualizedSubtitle = try c.decode(String.self, forKey: .avgAnnualizedSubtitle)
        activities = try c.decodeIfPresent([WSActivity].self, forKey: .activities) ?? []
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(closed, forKey: .closed)
        try c.encode(metrics, forKey: .metrics)
        try c.encode(nav, forKey: .nav)
        try c.encode(monthly, forKey: .monthly)
        try c.encode(years, forKey: .years)
        try c.encode(avgAnnualized, forKey: .avgAnnualized)
        try c.encode(avgAnnualizedSubtitle, forKey: .avgAnnualizedSubtitle)
        try c.encode(activities, forKey: .activities)
    }
}

enum WSPull {
    static let graphqlURL = URL(string: "https://my.wealthsimple.com/graphql")!
    static let tokenInfoURL = URL(string: "https://api.production.wealthsimple.com/v1/oauth/v2/token/info")!
    static let wsClient = "@wealthsimple/wealthsimple"
    static let graphqlVersion = "12"
    static let minus = "\u{2212}"
    static let emDash = "\u{2014}"

    static let qFetchAllAccountFinancials = """
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
"""

    static let qFetchActivityFeedItems = """
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
"""

    static let qIdentityHistoricalFinancials = """
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
"""

    private static let months = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"]
    private static let keepStatus: Set<String> = [
        "POSTED", "COMPLETED", "SETTLED", "COMPLETE", "FILLED", "EXECUTED",
        "PROCESSED", "CONFIRMED", "BOOKED", "SUCCEEDED", "SUCCESS",
    ]
    private static let corpBlobs = [
        "STKDIS", "STOCKDISTRIBUTION", "STOCKDIV", "SPINOFF", "SPIN",
        "DIVIDENDINKIND", "INKIND", "CORPORATEACTION", "CODECHANGE",
        "SYMBOLCHANGE", "TICKERCHANGE", "LISTINGSTATUS", "SECURITYSWAP",
        "MANDATORYEXCHANGE", "NAMECHANGE",
    ]
    private static let skipMarkers = ["SHARE_LENDING", "SHARELENDING", "STOCK_LENDING", "STOCKLENDING"]
    private static let identityKeys = [
        "identity_canonical_id", "identityCanonicalId", "canonical_id",
        "identity_id", "resource_owner_id", "sub",
    ]

    // MARK: - JSON cookie / session

    static func jsonWithAccessToken(_ raw: String) -> [String: Any]? {
        var cur = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        for _ in 0..<3 {
            if cur.contains("access_token"),
               let data = cur.data(using: .utf8),
               let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let token = obj["access_token"] as? String,
               !token.isEmpty
            {
                return obj
            }
            let nxt = cur.removingPercentEncoding ?? cur
            if nxt == cur { break }
            cur = nxt
        }
        return nil
    }

    static func session(fromCookie raw: String, wssdi: String?) -> WSSession? {
        guard let oauth = jsonWithAccessToken(raw) else { return nil }
        let access = J.str(oauth, "access_token")
        if access.isEmpty { return nil }
        return WSSession(
            accessToken: access,
            refreshToken: J.str(oauth, "refresh_token"),
            clientId: J.str(oauth, "client_id"),
            identityCanonicalId: identityFrom(oauth),
            expiresAt: expiresAtString(oauth["expires_at"]),
            sessionId: J.str(oauth, "session_id"),
            wssdi: (wssdi ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        )
    }

    private static func identityFrom(_ obj: [String: Any]) -> String {
        for k in identityKeys {
            let v = J.str(obj, k)
            if !v.isEmpty { return v }
        }
        return ""
    }

    private static func clientIdFromTokenInfo(_ info: [String: Any]) -> String {
        let uid = J.str(info, "application_uid")
        if !uid.isEmpty { return uid }
        let app = J.dict(info["application"])
        return J.str(app, "uid")
    }

    private static func expiresAtString(_ raw: Any?) -> String {
        if let s = raw as? String, s.contains("T") { return s.trimmingCharacters(in: .whitespacesAndNewlines) }
        return J.str(raw)
    }

    // MARK: - HTTP

    private static func httpJSON(
        method: String,
        url: URL,
        headers: [String: String],
        body: [String: Any]?,
        timeout: TimeInterval
    ) async throws -> [String: Any] {
        var req = URLRequest(url: url, timeoutInterval: timeout)
        req.httpMethod = method
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        for (k, v) in headers {
            req.setValue(v, forHTTPHeaderField: k)
        }
        if let body {
            req.httpBody = try JSONSerialization.data(withJSONObject: body)
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        let (data, resp) = try await URLSession.shared.data(for: req)
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        if status == 401 || status == 403 {
            throw WSPullError.unauthorized
        }
        if data.isEmpty { return [:] }
        guard let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return ["error": "invalid_json", "_http_status": status]
        }
        var out = obj
        out["_http_status"] = status
        return out
    }

    private static func sessionHeaders(_ sess: WSSession, extra: [String: String]) -> [String: String] {
        var h = extra
        if !sess.wssdi.isEmpty { h["x-ws-device-id"] = sess.wssdi }
        if !sess.sessionId.isEmpty { h["x-ws-session-id"] = sess.sessionId }
        return h
    }

    private static func tokenInfo(_ sess: WSSession) async throws -> [String: Any] {
        let headers = sessionHeaders(sess, extra: [
            "Authorization": "Bearer " + sess.accessToken,
            "x-wealthsimple-client": wsClient,
        ])
        return try await httpJSON(method: "GET", url: tokenInfoURL, headers: headers, body: nil, timeout: 60)
    }

    private static func graphql(_ sess: WSSession, operation: String, variables: [String: Any], query: String) async throws -> [String: Any] {
        var headers: [String: String] = [
            "Authorization": "Bearer " + sess.accessToken,
            "x-wealthsimple-client": wsClient,
            "x-ws-profile": "trade",
            "x-ws-api-version": graphqlVersion,
            "x-ws-locale": "en-CA",
            "x-platform-os": "web",
            "Content-Type": "application/json",
            "Origin": "https://my.wealthsimple.com",
            "Referer": "https://my.wealthsimple.com/app/trade",
        ]
        headers = sessionHeaders(sess, extra: headers)
        var vars: [String: Any] = [:]
        for (k, v) in variables {
            if v is NSNull { continue }
            vars[k] = v
        }
        let body: [String: Any] = [
            "operationName": operation,
            "query": query,
            "variables": vars,
        ]
        let data = try await httpJSON(method: "POST", url: graphqlURL, headers: headers, body: body, timeout: 90)
        if let status = data["_http_status"] as? Int, status == 401 || status == 403 {
            throw WSPullError.unauthorized
        }
        if let errs = data["errors"] {
            let first: Any
            if let arr = errs as? [Any], let f = arr.first {
                first = f
            } else {
                first = errs
            }
            var emsg = "\(first)"
            if let d = first as? [String: Any] {
                emsg = J.str(d, "message")
                if emsg.isEmpty { emsg = J.str(d, "error") }
                if emsg.isEmpty { emsg = "\(first)" }
            }
            throw WSPullError.graphql("\(operation): \(emsg)")
        }
        guard let payload = data["data"] as? [String: Any] else {
            throw WSPullError.graphql("graphql failed: \(operation)")
        }
        return payload
    }

    // MARK: - Public pull

    static func run(
        oauthCookie: String,
        wssdi: String?,
        onProgress: (@Sendable (String) -> Void)? = nil
    ) async throws -> WSPullResult {
        func progress(_ msg: String) { onProgress?(msg) }
        guard var sess = session(fromCookie: oauthCookie, wssdi: wssdi) else {
            throw WSPullError.noSession
        }
        if sess.identityCanonicalId.isEmpty {
            let info = try await tokenInfo(sess)
            sess.identityCanonicalId = identityFrom(info)
            if sess.clientId.isEmpty {
                sess.clientId = clientIdFromTokenInfo(info)
            }
        }
        if sess.identityCanonicalId.isEmpty {
            throw WSPullError.noIdentity
        }

        progress("Fetching accounts…")
        let accounts = try await fetchAllAccounts(sess, identityId: sess.identityCanonicalId)
        var accById: [String: [String: Any]] = [:]
        for a in accounts {
            let aid = J.str(a, "id")
            if !aid.isEmpty { accById[aid] = a }
        }
        let pools = fifoPoolIds(accounts)
        var mapped: [WSActivity] = []
        progress("Syncing transactions")
        for acc in accounts {
            let aid = J.str(acc, "id")
            if aid.isEmpty { continue }
            let rawItems = try await fetchActivities(sess, accountId: aid)
            for it in rawItems {
                mapped.append(contentsOf: mapActivityRows(it, accounts: accById))
            }
        }
        for i in mapped.indices {
            let aid = mapped[i].accountId
            mapped[i].fifoId = pools[aid] ?? aid
        }
        let fifo = matchFifo(mapped)
        let activities = mapped
        progress("Fetching balances…")
        progress("Fetching equity history…")
        let nav = try await fetchNavHistory(sess, identityId: sess.identityCanonicalId)
        progress("Fetching S&P 500…")
        async let spyTask = ensureSpyPrices()
        async let fxTask = ensureFxRates(activities: activities)
        let spy = await spyTask
        let fx = await fxTask
        // ledger.html render(): applyFx(closed) then groupClosedByClose(visible) for computeMetrics.
        let closedFx = applyFx(fifo.closed, fx: fx)
        let closedForMetrics = groupClosedByClose(closedFx)
        let metrics = computeMetrics(closedForMetrics)
        let monthly = monthlyPnl(closedForMetrics)
        let yearRows = annualRows(nav: nav, spy: spy, activities: mapped)
        let ann = accountAnnualizedReturn(nav: nav, years: yearRows.map(\.year).sorted())
        return WSPullResult(
            closed: closedFx,
            metrics: metrics,
            nav: nav,
            monthly: monthly,
            years: yearRows,
            avgAnnualized: formatReturn(ann.rate),
            avgAnnualizedSubtitle: formatYearSpan(ann.years),
            activities: activities
        )
    }

    // MARK: - Fetch

    private static func fetchAllAccounts(_ sess: WSSession, identityId: String) async throws -> [[String: Any]] {
        var accounts: [[String: Any]] = []
        var cursor: String? = nil
        while true {
            var variables: [String: Any] = [
                "identityId": identityId,
                "pageSize": 25,
                "startDate": "2015-01-01",
            ]
            if let cursor { variables["cursor"] = cursor }
            let data = try await graphql(sess, operation: "FetchAllAccountFinancials", variables: variables, query: qFetchAllAccountFinancials)
            let ident = J.dict(data["identity"])
            let conn = J.dict(ident["accounts"])
            for edge in J.arr(conn["edges"]) {
                let e = J.dict(edge)
                let node = J.dict(e["node"])
                if !node.isEmpty { accounts.append(node) }
            }
            let page = J.dict(conn["pageInfo"])
            if !J.bool(page["hasNextPage"]) { break }
            let next = J.str(page, "endCursor")
            if next.isEmpty { break }
            cursor = next
        }
        return accounts
    }

    private static func fetchActivities(_ sess: WSSession, accountId: String) async throws -> [[String: Any]] {
        var items: [[String: Any]] = []
        var cursor: String? = nil
        let end = Calendar(identifier: .gregorian).date(byAdding: .day, value: 1, to: Date()) ?? Date()
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(secondsFromGMT: 0)!
        let c = cal.dateComponents([.year, .month, .day, .hour, .minute, .second], from: end)
        let endDate = String(format: "%04d-%02d-%02dT%02d:%02d:%02d.999Z", c.year ?? 0, c.month ?? 0, c.day ?? 0, c.hour ?? 0, c.minute ?? 0, c.second ?? 0)
        while true {
            var variables: [String: Any] = [
                "first": 100,
                "orderBy": "OCCURRED_AT_DESC",
                "condition": [
                    "endDate": endDate,
                    "accountIds": [accountId],
                ] as [String: Any],
            ]
            if let cursor { variables["cursor"] = cursor }
            let data = try await graphql(sess, operation: "FetchActivityFeedItems", variables: variables, query: qFetchActivityFeedItems)
            let feed = J.dict(data["activityFeedItems"])
            for edge in J.arr(feed["edges"]) {
                let node = J.dict(J.dict(edge)["node"])
                if !node.isEmpty { items.append(node) }
            }
            let page = J.dict(feed["pageInfo"])
            if !J.bool(page["hasNextPage"]) { break }
            let next = J.str(page, "endCursor")
            if next.isEmpty { break }
            cursor = next
        }
        return items
    }

    private static func fetchNavHistory(_ sess: WSSession, identityId: String) async throws -> [WSNavPoint] {
        let today = isoDay(Date())
        let year0 = 2015
        let year1 = Int(today.prefix(4)) ?? year0
        var points: [WSNavPoint] = []
        if year1 < year0 { return [] }
        for year in year0...year1 {
            let start = "\(year)-01-01"
            let end = year == year1 ? today : "\(year)-12-31"
            if start > end { continue }
            var cursor: String? = nil
            for _ in 0..<8 {
                var variables: [String: Any] = [
                    "identityId": identityId,
                    "currency": "CAD",
                    "limit": 400,
                    "includeNetDeposits": true,
                    "startDate": start,
                    "endDate": end,
                ]
                if let cursor { variables["cursor"] = cursor }
                let data = try await graphql(
                    sess,
                    operation: "IdentityHistoricalFinancialsQuery",
                    variables: variables,
                    query: qIdentityHistoricalFinancials
                )
                let (chunk, page) = navPointsFromPayload(data)
                points.append(contentsOf: chunk)
                if !J.bool(page["hasNextPage"]) { break }
                let next = J.str(page, "endCursor")
                if next.isEmpty { break }
                cursor = next
            }
        }
        var byDate: [String: WSNavPoint] = [:]
        for rec in points { byDate[rec.date] = rec }
        return byDate.keys.sorted().compactMap { byDate[$0] }
    }

    private static func navPointsFromPayload(_ data: [String: Any]) -> ([WSNavPoint], [String: Any]) {
        var points: [WSNavPoint] = []
        let ident = J.dict(data["identity"])
        let acc = J.dict(data["account"])
        let fin = ident["financials"] != nil ? J.dict(ident["financials"]) : J.dict(acc["financials"])
        let hist = J.dict(fin["historicalDaily"])
        for edge in J.arr(hist["edges"]) {
            let node = J.dict(J.dict(edge)["node"])
            let (amt, cur) = moneyAmount(node, keys: ["netLiquidationValue", "netLiquidationValueV2"])
            let d = String(J.str(node, "date").prefix(10))
            if d.isEmpty || amt == nil { continue }
            var rec = WSNavPoint(date: d, equity: amt!, currency: cur.isEmpty ? "CAD" : cur, netDeposits: nil)
            let (nd, _) = moneyAmount(node, keys: ["netDeposits", "netDepositsV2"])
            rec.netDeposits = nd
            points.append(rec)
        }
        return (points, J.dict(hist["pageInfo"]))
    }

    private static func moneyAmount(_ node: [String: Any], keys: [String]) -> (Double?, String) {
        for key in keys {
            let money = J.dict(node[key])
            if money["amount"] == nil { continue }
            let amt = J.num(money["amount"], default: Double.nan)
            if amt.isNaN { continue }
            return (amt, J.str(money, "currency"))
        }
        return (nil, "")
    }

    // MARK: - Mapping (bagholder.py)

    private static func num(_ v: Any?, default def: Double = 0) -> Double { J.num(v, default: def) }
    private static func s(_ v: Any?) -> String { J.str(v) }
    private static func upper(_ v: Any?) -> String { s(v).trimmingCharacters(in: .whitespacesAndNewlines).uppercased() }
    private static func compact(_ v: Any?) -> String {
        upper(v).replacingOccurrences(of: "[\\s_\\-]+", with: "", options: .regularExpression)
    }
    private static func dateOnly(_ occurred: String) -> String {
        var t = occurred.trimmingCharacters(in: .whitespacesAndNewlines)
        if t.isEmpty { return "" }
        if let i = t.firstIndex(of: "T") { t = String(t[..<i]) }
        return String(t.prefix(10))
    }
    private static func assetSymbol(_ item: [String: Any], key: String = "assetSymbol") -> String {
        var raw = s(item[key]).trimmingCharacters(in: .whitespacesAndNewlines)
        if raw.uppercased().hasPrefix("EXCHANGE:") {
            raw = raw.split(separator: ":", maxSplits: 1).last.map(String.init) ?? raw
        }
        return raw.uppercased().trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func typeBlob(_ item: [String: Any]) -> (String, String, String) {
        let typ = upper(item["type"]).replacingOccurrences(of: "-", with: "_")
        let sub = upper(item["subType"]).replacingOccurrences(of: "-", with: "_")
        let parts = [typ, sub, s(item["aftTransactionType"]), s(item["aftTransactionCategory"])]
            .filter { !$0.isEmpty }
            .map { compact($0) }
        return (typ, sub, parts.joined(separator: "_"))
    }

    private static func isCorpShareMove(_ item: [String: Any]) -> Bool {
        let (_, _, blob) = typeBlob(item)
        if corpBlobs.contains(where: { blob.contains($0) }) { return true }
        let qty = abs(num(item["assetQuantity"]))
        let cash = abs(num(item["amount"]))
        if qty > 0 && !assetSymbol(item).isEmpty && cash == 0 &&
            (compact(item["type"]).contains("DIVIDEND") || blob.contains("DISTRIBUT")) {
            return true
        }
        return false
    }

    private static func isCodeChange(_ item: [String: Any]) -> Bool {
        let (_, _, blob) = typeBlob(item)
        for k in ["CODECHANGE", "SYMBOLCHANGE", "TICKERCHANGE", "LISTINGSTATUS", "SECURITYSWAP", "MANDATORYEXCHANGE", "NAMECHANGE"] {
            if blob.contains(k) { return true }
        }
        return false
    }

    private static func skipActivity(_ item: [String: Any]?) -> Bool {
        guard let item, !item.isEmpty else { return true }
        if s(item["occurredAt"]).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return true }
        let status = compact(item["status"])
        let (typ, sub, blob) = typeBlob(item)
        if isCorpShareMove(item) {
            if status.contains("REJECT") || status.contains("CANCEL") || status.contains("FAIL") || status.contains("VOID") {
                return true
            }
        } else if status.isEmpty || !keepStatus.contains(status) {
            return true
        }
        if typ == "LOAN" || typ == "RECALL" || sub == "LOAN" || sub == "RECALL" { return true }
        if typ.hasSuffix("_LOAN") || sub.hasSuffix("_LOAN") { return true }
        if typ.hasSuffix("_RECALL") || sub.hasSuffix("_RECALL") { return true }
        for marker in skipMarkers {
            if blob.contains(marker) { return true }
        }
        if blob.contains("SHARE_LENDING") || blob.contains("SHARELENDING") { return true }
        return false
    }

    private static func optionSymbol(_ item: [String: Any]) -> String {
        let under = assetSymbol(item)
        let contract = item["contractType"]
        let strike = item["strikePrice"]
        let expiry = item["expiryDate"]
        let hasContract = !(s(contract).isEmpty)
        if !hasContract || strike == nil || expiry == nil || under.isEmpty { return under }
        var ds = s(expiry).trimmingCharacters(in: .whitespacesAndNewlines)
        if let i = ds.firstIndex(of: "T") { ds = String(ds[..<i]) }
        let parts = ds.replacingOccurrences(of: "/", with: "-").prefix(10).split(separator: "-").map(String.init)
        if parts.count != 3 { return under }
        guard let year = Int(parts[0]), let month = Int(parts[1]), let day = Int(parts[2]),
              month >= 1, month <= 12 else { return under }
        let mon = months[month - 1]
        let yy = String(format: "%02d", year % 100)
        let dd = String(format: "%02d", day)
        let strikeF = num(strike, default: Double.nan)
        if strikeF.isNaN { return under }
        let strikeS = String(format: "%.2f", strikeF)
        var cp = upper(contract)
        if cp == "C" || cp == "CALL" { cp = "CALL" }
        else if cp == "P" || cp == "PUT" { cp = "PUT" }
        return "\(under) \(dd)\(mon)\(yy) \(strikeS) \(cp)"
    }

    private static func signedCash(_ item: [String: Any]) -> Double {
        let amount = abs(num(item["amount"]))
        let typ = upper(item["type"]).replacingOccurrences(of: "-", with: "_")
        let sub = upper(item["subType"]).replacingOccurrences(of: "-", with: "_")
        if typ == "DIY_BUY" || typ == "OPTIONS_BUY" || typ == "WITHDRAWAL" ||
            (typ == "INTERNAL_TRANSFER" && sub.contains("SOURCE")) {
            return -amount
        }
        if ["DIY_SELL", "OPTIONS_SELL", "DEPOSIT", "CONTRIBUTION", "DIVIDEND", "INTEREST"].contains(typ) ||
            (typ == "INTERNAL_TRANSFER" && sub.contains("DESTINATION")) {
            return amount
        }
        let sign = s(item["amountSign"]).trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if ["negative", "debit", "-", "neg"].contains(sign) { return -amount }
        if ["positive", "credit", "+", "pos"].contains(sign) { return amount }
        if item["amount"] == nil || s(item["amount"]).isEmpty { return 0 }
        return num(item["amount"])
    }

    private static func accountType(_ accountId: String, accounts: [String: [String: Any]]) -> String {
        guard let rec = accounts[accountId] else { return "" }
        let nick = J.str(rec, "nickname")
        if !nick.isEmpty { return nick }
        let u = J.str(rec, "unifiedAccountType")
        if !u.isEmpty { return u }
        return J.str(rec, "type")
    }

    private static func fifoPoolIds(_ recs: [[String: Any]]) -> [String: String] {
        var parent: [String: String] = [:]
        func find(_ x: String) -> String {
            var x = x
            if parent[x] == nil { parent[x] = x }
            while parent[x] != x {
                parent[x] = parent[parent[x] ?? x] ?? x
                x = parent[x] ?? x
            }
            return x
        }
        func union(_ a: String, _ b: String) {
            if a.isEmpty || b.isEmpty { return }
            let ra = find(a), rb = find(b)
            if ra != rb { parent[max(ra, rb)] = min(ra, rb) }
        }
        var byNick: [String: [String]] = [:]
        for a in recs {
            let aid = J.str(a, "id")
            if aid.isEmpty { continue }
            _ = find(aid)
            let linked = J.dict(a["linkedAccount"])
            let lid = J.str(linked, "id")
            if !lid.isEmpty { union(aid, lid) }
            let nick = J.str(a, "nickname").trimmingCharacters(in: .whitespacesAndNewlines)
            if !nick.isEmpty { byNick[nick, default: []].append(aid) }
        }
        for ids in byNick.values {
            guard let root = ids.first else { continue }
            for other in ids.dropFirst() { union(root, other) }
        }
        var out: [String: String] = [:]
        for aid in parent.keys { out[aid] = find(aid) }
        return out
    }

    private static func isOption(_ item: [String: Any]) -> Bool {
        !s(item["contractType"]).isEmpty
    }
    private static func isToClose(_ sub: String) -> Bool {
        let c = compact(sub)
        return c.contains("TOCLOSE") || c == "BTC" || c == "STC" || c == "BUYTOCLOSE" || c == "SELLTOCLOSE"
    }

    private static func mapActivityRows(_ item: [String: Any], accounts: [String: [String: Any]]) -> [WSActivity] {
        if item.isEmpty { return [] }
        let src = assetSymbol(item)
        let dst = assetSymbol(item, key: "counterAssetSymbol")
        let qty = abs(num(item["assetQuantity"]))
        if !src.isEmpty && !dst.isEmpty && src != dst && qty > 0 && isCorpShareMove(item) {
            let cid = s(item["canonicalId"]).trimmingCharacters(in: .whitespacesAndNewlines)
            let base = cid.isEmpty ? "swap" : cid
            var outgoing = item
            outgoing["assetSymbol"] = src
            outgoing["counterAssetSymbol"] = ""
            outgoing["type"] = "STKDIS"
            outgoing["subType"] = "STKDIS"
            outgoing["assetQuantity"] = -qty
            outgoing["amount"] = 0
            outgoing["amountSign"] = "negative"
            outgoing["canonicalId"] = base + ":out"
            var incoming = item
            incoming["assetSymbol"] = dst
            incoming["counterAssetSymbol"] = ""
            incoming["type"] = "STKDIS"
            incoming["subType"] = "STKDIS"
            incoming["assetQuantity"] = qty
            incoming["amount"] = 0
            incoming["amountSign"] = "positive"
            incoming["canonicalId"] = base + ":in"
            return [mapActivity(outgoing, accounts: accounts), mapActivity(incoming, accounts: accounts)].compactMap { $0 }
        }
        if let row = mapActivity(item, accounts: accounts) { return [row] }
        return []
    }

    private static func mapActivity(_ item: [String: Any], accounts: [String: [String: Any]]) -> WSActivity? {
        if skipActivity(item) { return nil }
        let occurred = s(item["occurredAt"]).trimmingCharacters(in: .whitespacesAndNewlines)
        // Keep Wealthsimple date and time. transactionDate stays the calendar day
        // for FIFO / groupClosedByClose, same as bagholder.py _date_only.
        let transactionDate = dateOnly(occurred)
        if transactionDate.isEmpty { return nil }
        let accountId = s(item["accountId"])
        let typ = upper(item["type"]).replacingOccurrences(of: "-", with: "_")
        let sub = upper(item["subType"]).replacingOccurrences(of: "-", with: "_")
        let qtyRaw = num(item["assetQuantity"])
        let qtyAbs = abs(qtyRaw)
        let cash = signedCash(item)
        let amountAbs = abs(num(item["amount"]))
        let fees = abs(num(item["fees"]))
        let opt = isOption(item)
        var symbol = opt ? optionSymbol(item) : assetSymbol(item)
        var cur = upper(item["currency"])
        if cur != "CAD" && cur != "USD" { cur = opt ? "USD" : "CAD" }
        var unitPrice = 0.0
        if qtyAbs > 0 {
            unitPrice = amountAbs / qtyAbs
            if opt && unitPrice > 20 { unitPrice = unitPrice / 100.0 }
        }
        var activityType = "Other"
        var activitySub = sub.isEmpty ? typ : sub
        var category = "other"
        var quantity = qtyAbs

        if typ == "DIY_BUY" {
            category = "trade"
            if opt {
                if isToClose(sub) { activityType = "Trade"; activitySub = "BUYTOCLOSE" }
                else { activityType = "Trade"; activitySub = "BUYTOOPEN" }
            } else {
                activityType = "Trade"; activitySub = "BUY"
            }
            quantity = abs(qtyAbs)
        } else if typ == "DIY_SELL" {
            category = "trade"
            if opt {
                if isToClose(sub) { activityType = "Trade"; activitySub = "SELLTOCLOSE" }
                else { activityType = "Trade"; activitySub = "SELLTOOPEN" }
            } else {
                activityType = "Trade"; activitySub = "SELL"
            }
            quantity = -abs(qtyAbs)
        } else if typ == "OPTIONS_BUY" {
            category = "trade"
            if isToClose(sub) { activityType = "OPTIONS_BUY"; activitySub = "BUYTOCLOSE" }
            else { activityType = "OPTIONS_BUY"; activitySub = "BUYTOOPEN" }
            quantity = abs(qtyAbs)
        } else if typ == "OPTIONS_SELL" {
            category = "trade"
            if isToClose(sub) { activityType = "OPTIONS_SELL"; activitySub = "SELLTOCLOSE" }
            else { activityType = "OPTIONS_SELL"; activitySub = "SELLTOOPEN" }
            quantity = -abs(qtyAbs)
        } else if ["EXPIR", "EXPIRY", "EXPIRE", "ASSIGN", "ASSIGNMENT", "EXERCISE"].contains(typ) {
            category = "option_event"
            let keep = typ.contains("ASSIGN") ? "ASSIGN" : (typ.contains("EXERCISE") ? "EXERCISE" : "EXPIR")
            activityType = keep
            let covering = typ.contains("ASSIGN") || compact(sub).contains("COVER") || isToClose(sub)
            activitySub = covering ? "BUY" : "SELL"
            quantity = activitySub == "SELL" ? -abs(qtyAbs) : abs(qtyAbs)
            if !opt { symbol = symbol.isEmpty ? assetSymbol(item) : symbol }
        } else if typ == "DEPOSIT" || typ == "CONTRIBUTION" {
            activityType = "Deposit"; activitySub = "deposit"; category = "deposit"
        } else if typ == "WITHDRAWAL" {
            activityType = "Withdrawal"; activitySub = "withdrawal"; category = "withdrawal"
        } else if typ == "INTERNAL_TRANSFER" || ["TRFIN", "TRFOUT", "TRANSFERIN", "TRANSFEROUT", "INTERNALTRANSFER"].contains(compact(typ)) {
            activityType = "Transfer"; activitySub = "transfer"; category = "transfer"
        } else if typ == "DIVIDEND" && !isCorpShareMove(item) {
            activityType = "Dividend"; activitySub = "dividend"; category = "dividend"
        } else if typ == "INTEREST" || sub.contains("FPL_INTEREST") || compact(typ) == "FPLINTEREST" {
            activityType = "Interest"; activitySub = "interest"; category = "interest"
        } else if typ == "FUNDS_CONVERSION" {
            activityType = "FxExchange"; activitySub = "fx"; category = "fx"
        } else if typ == "FEE" || typ == "REFUND" {
            activityType = typ == "REFUND" ? "Refund" : "Fee"
            activitySub = "fee"; category = "fee"
        } else if isCorpShareMove(item) ||
                    ["STOCK_DISTRIBUTION", "STKDIS", "SPIN", "SPINOFF", "STK_DIS"].contains(typ) ||
                    compact(item["type"]).contains("STKDIS") ||
                    compact(item["type"]).contains("STOCKDISTRIBUTION") ||
                    compact(item["subType"]).contains("STOCKDISTRIBUTION") {
            activityType = "STKDIS"; category = "trade"
            unitPrice = 0
            let sign = s(item["amountSign"]).trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            var outgoing = qtyRaw < 0 || ["negative", "debit", "-", "neg"].contains(sign)
            if !outgoing && isCodeChange(item) && assetSymbol(item, key: "counterAssetSymbol").isEmpty &&
                !compact(item["type"]).contains("STKDIS") {
                outgoing = true
            }
            if outgoing {
                activitySub = "SELL"; quantity = -qtyAbs
            } else {
                activitySub = "BUY"; quantity = qtyAbs
            }
        } else {
            activityType = s(item["type"]).isEmpty ? "Other" : s(item["type"])
            activitySub = s(item["subType"]).isEmpty ? "other" : s(item["subType"])
            category = "other"
        }

        if ["SELL", "SELLTOOPEN", "SELLTOCLOSE"].contains(activitySub) {
            quantity = qtyAbs > 0 ? -abs(qtyAbs) : quantity
        }

        let sign = s(item["amountSign"]).trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        var direction = ""
        if ["negative", "debit", "-", "neg"].contains(sign) || cash < 0 { direction = "DEBIT" }
        else if ["positive", "credit", "+", "pos"].contains(sign) || cash > 0 { direction = "CREDIT" }

        let cid = s(item["canonicalId"]).trimmingCharacters(in: .whitespacesAndNewlines)
        let id = cid.isEmpty ? "\(occurred)|\(accountId)|\(symbol)|\(quantity)" : cid
        return WSActivity(
            id: id,
            canonicalId: cid,
            occurredAt: occurred,
            transactionDate: transactionDate,
            accountId: accountId,
            fifoId: accountId,
            accountType: accountType(accountId, accounts: accounts),
            activityType: activityType,
            activitySubType: activitySub,
            description: "",
            direction: direction,
            symbol: symbol,
            name: {
                let n = s(item["aftOriginatorName"])
                if !n.isEmpty { return n }
                let n2 = s(item["institutionName"])
                return n2.isEmpty ? symbol : n2
            }(),
            currency: cur,
            quantity: quantity,
            unitPrice: unitPrice,
            commission: fees,
            netCashAmount: cash,
            category: category,
            rawType: s(item["type"]),
            aftType: s(item["aftTransactionType"]),
            counterSymbol: assetSymbol(item, key: "counterAssetSymbol")
        )
    }

    // MARK: - FIFO (ledger.html matchFifo)

    private static func compactType(_ s: String) -> String {
        s.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
            .replacingOccurrences(of: "[\\s_\\-]+", with: "", options: .regularExpression)
    }
    private static func tradeSide(_ a: WSActivity) -> String? {
        let s = compactType(a.activitySubType)
        if ["BUY", "BUYTOOPEN", "BTO", "BUYTOCLOSE", "BTC"].contains(s) { return "BUY" }
        if ["SELL", "SELLTOOPEN", "STO", "SELLTOCLOSE", "STC"].contains(s) { return "SELL" }
        let t = compactType(a.activityType)
        if ["BUYTOOPEN", "BTO", "BUYTOCLOSE", "BTC"].contains(t) { return "BUY" }
        if ["SELLTOOPEN", "STO", "SELLTOCLOSE", "STC"].contains(t) { return "SELL" }
        return nil
    }
    private static func isIntentionalOpen(_ a: WSActivity) -> Bool {
        let fields = [compactType(a.activityType), compactType(a.activitySubType)]
        if fields.contains(where: { $0.contains("TOOPEN") }) { return true }
        if fields.contains(where: { $0 == "STO" || $0 == "BTO" }) { return true }
        return false
    }
    private static func isCloseOnly(_ a: WSActivity) -> Bool {
        let fields = [compactType(a.activityType), compactType(a.activitySubType)]
        if fields.contains(where: { $0.contains("TOCLOSE") || $0 == "BTC" || $0 == "STC" }) { return true }
        if fields.contains(where: { $0.contains("EXPIR") || $0.contains("ASSIGN") || $0.contains("EXERCISE") }) { return true }
        return false
    }
    private static func openingDirection(_ a: WSActivity, side: String) -> String? {
        if side == "BUY" {
            if isCloseOnly(a) { return nil }
            return "LONG"
        }
        if isIntentionalOpen(a) { return "SHORT" }
        return nil
    }
    private static func fifoAccount(_ a: WSActivity) -> String {
        let nick = a.accountType.trimmingCharacters(in: .whitespacesAndNewlines)
        if !nick.isEmpty { return nick }
        if !a.fifoId.isEmpty { return a.fifoId }
        return a.accountId
    }
    static func isOptionSymbol(_ symbol: String) -> Bool {
        let u = symbol.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
            .replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
        if u.isEmpty { return false }
        if u.range(of: "\\b(PUT|CALL)\\b", options: .regularExpression) != nil { return true }
        if u.range(of: "\\s[CP]$", options: .regularExpression) != nil { return true }
        if u.range(of: "^[A-Z][A-Z0-9.]{0,9} \\d{6}[CP]\\d+", options: .regularExpression) != nil { return true }
        return false
    }
    static func optionMultiplier(_ symbol: String) -> Double { isOptionSymbol(symbol) ? 100 : 1 }
    static func underlyingSymbol(_ symbol: String) -> String {
        let s0 = symbol.trimmingCharacters(in: .whitespacesAndNewlines)
        if s0.isEmpty { return emDash }
        let u = s0.uppercased().replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
        if u.range(of: "\\b(PUT|CALL)\\b", options: .regularExpression) != nil || u.range(of: "\\s[CP]$", options: .regularExpression) != nil {
            return u.split(separator: " ").first.map(String.init) ?? s0
        }
        if let m = u.range(of: "^([A-Z][A-Z0-9.]{0,9}) \\d{6}[CP]\\d+", options: .regularExpression) {
            return String(u[m]).split(separator: " ").first.map(String.init) ?? s0
        }
        if let m = u.range(of: "^([A-Z][A-Z0-9.]{0,9}) \\d{1,2}[A-Z]{3}\\d{2}\\b", options: .regularExpression) {
            return String(u[m]).split(separator: " ").first.map(String.init) ?? s0
        }
        return s0
    }

    private static func foldStkdis(_ activities: [WSActivity]) -> [WSActivity] {
        var rest: [WSActivity] = []
        var groups: [String: (pos: Double, neg: Double, sample: WSActivity)] = [:]
        for a in activities {
            if compactType(a.activityType) != "STKDIS" {
                rest.append(a)
                continue
            }
            let k = [a.symbol, a.transactionDate, a.currency].joined(separator: "|")
            // ledger.html foldStkdis: sample is the first STKDIS in the group.
            if groups[k] == nil { groups[k] = (0, 0, a) }
            var g = groups[k]!
            let q = a.quantity
            if a.activitySubType == "SELL" || q < 0 { g.neg += abs(q) }
            else { g.pos += abs(q) }
            groups[k] = g
        }
        for g in groups.values {
            let net = g.pos - g.neg
            if net > 1e-10 {
                var a = g.sample
                a.quantity = net
                a.activitySubType = "BUY"
                a.unitPrice = 0
                a.netCashAmount = 0
                a.category = "trade"
                rest.append(a)
            }
        }
        return rest
    }

    private static func daysBetween(_ a: String, _ b: String) -> Int {
        guard let da = ymd(a), let db = ymd(b) else { return 0 }
        let ms = db.timeIntervalSince(da)
        return max(0, Int((ms / 86400.0).rounded()))
    }

    private static func ymd(_ s: String) -> Date? {
        let f = DateFormatter()
        f.calendar = Calendar(identifier: .gregorian)
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(secondsFromGMT: 0)
        f.dateFormat = "yyyy-MM-dd"
        return f.date(from: String(s.prefix(10)))
    }

    private static func isoDay(_ d: Date) -> String {
        let f = DateFormatter()
        f.calendar = Calendar(identifier: .gregorian)
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(secondsFromGMT: 0)
        f.dateFormat = "yyyy-MM-dd"
        return f.string(from: d)
    }

    private static func stableTradeId(_ t: WSClosedTrade) -> String {
        [
            t.accountId, t.symbol, t.currency, t.entryDate, t.exitDate,
            jsToFixed(t.quantity, digits: 8),
            jsToFixed(t.entryPrice, digits: 8),
            jsToFixed(t.exitPrice, digits: 8),
            t.side,
        ].joined(separator: "|")
    }

    private struct Lot {
        var qty: Double
        var price: Double
        var date: String
        var commission: Double
        var direction: String
        var accountId: String
        var accountType: String
        var symbol: String
        var name: String
        var currency: String
        var activityId: String
    }

    private static func tickerWasReplaced(_ activities: [WSActivity], accountId: String, symbol: String, currency: String, byDate: String) -> Bool {
        var removedOn = ""
        for a in activities {
            if fifoAccount(a) != accountId { continue }
            if a.symbol != symbol || a.currency != currency { continue }
            let t = compactType(a.activityType)
            let raw = compactType(a.rawType) + compactType(a.aftType)
            let sub = compactType(a.activitySubType)
            let q = a.quantity
            let removal = (t == "STKDIS" && (sub == "SELL" || q < 0)) ||
                raw.range(of: "CODECHANGE|SYMBOLCHANGE|TICKERCHANGE|LISTINGSTATUS|SECURITYSWAP", options: .regularExpression) != nil
            if removal && !a.transactionDate.isEmpty && (removedOn.isEmpty || a.transactionDate < removedOn) {
                removedOn = a.transactionDate
            }
        }
        if removedOn.isEmpty || removedOn > byDate { return false }
        for a in activities {
            if fifoAccount(a) != accountId { continue }
            if a.symbol != symbol || a.currency != currency { continue }
            if a.transactionDate <= removedOn { continue }
            if (a.category == "trade" || a.category == "option_event") && compactType(a.activityType) != "STKDIS" && tradeSide(a) != nil {
                return false
            }
        }
        return true
    }

    private static func matchFifo(_ activities: [WSActivity]) -> (closed: [WSClosedTrade], openCount: Int) {
        struct Fill { var activity: WSActivity; var side: String; var qty: Double }
        func fillRank(_ f: Fill) -> Int {
            let t = compactType(f.activity.activityType)
            let s = compactType(f.activity.activitySubType)
            let blob = t + s
            if (blob.contains("TOOPEN") || t == "STO" || s == "STO") && f.side == "SELL" { return 0 }
            if isCloseOnly(f.activity) && f.side == "BUY" { return 1 }
            if f.side == "BUY" { return 2 }
            if blob.contains("TOCLOSE") || t == "STC" || s == "STC" { return 3 }
            return 4
        }
        var fills: [Fill] = foldStkdis(activities).compactMap { a in
            guard a.category == "trade" || a.category == "option_event", !a.symbol.isEmpty else { return nil }
            guard let side = tradeSide(a) else { return nil }
            let qty = abs(a.quantity)
            if qty <= 0 { return nil }
            return Fill(activity: a, side: side, qty: qty)
        }
        fills.sort { a, b in
            // ledger.html 1565-1571: transactionDate, fillRank, id. Date only.
            let d = a.activity.transactionDate.compare(b.activity.transactionDate)
            if d != .orderedSame { return d == .orderedAscending }
            let ra = fillRank(a), rb = fillRank(b)
            if ra != rb { return ra < rb }
            return a.activity.id.compare(b.activity.id) == .orderedAscending
        }
        var books: [String: [Lot]] = [:]
        var bookOrder: [String] = []
        func bookKey(_ a: WSActivity) -> String { fifoAccount(a) + "::" + a.symbol + "::" + a.currency }
        func getBook(_ a: WSActivity) -> String {
            let k = bookKey(a)
            if books[k] == nil {
                books[k] = []
                bookOrder.append(k)
            }
            return k
        }
        var closed: [WSClosedTrade] = []
        func makeTrade(lot: Lot, a: WSActivity, fillQty: Double, matched: Double, side: String, symbol: String, name: String, multSymbol: String) -> WSClosedTrade {
            let exitCommission = fillQty > 0 ? a.commission * (matched / fillQty) : 0
            let entryCommission = lot.qty > 0 ? lot.commission * (matched / lot.qty) : 0
            let commission = entryCommission + exitCommission
            let rawPnl = (lot.direction == "LONG" ? (a.unitPrice - lot.price) * matched : (lot.price - a.unitPrice) * matched) * optionMultiplier(multSymbol)
            var trade = WSClosedTrade(
                id: "",
                accountId: lot.accountId,
                accountType: lot.accountType,
                symbol: symbol,
                name: name,
                currency: lot.currency,
                side: side,
                quantity: matched,
                entryPrice: lot.price,
                exitPrice: a.unitPrice,
                entryDate: lot.date,
                exitDate: a.transactionDate,
                holdDays: daysBetween(lot.date, a.transactionDate),
                commission: commission,
                entryCommission: entryCommission,
                exitCommission: exitCommission,
                pnl: rawPnl - commission,
                pnlCad: rawPnl - commission,
                openDirection: lot.direction,
                buyActivityId: lot.activityId,
                sellActivityId: a.id
            )
            trade.id = stableTradeId(trade)
            return trade
        }
        for fill in fills {
            let a = fill.activity
            let k = getBook(a)
            var book = books[k] ?? []
            let closingDir = fill.side == "BUY" ? "SHORT" : "LONG"
            var remaining = fill.qty
            while remaining > 0 && !book.isEmpty && book[0].direction == closingDir {
                var lot = book[0]
                let matched = min(lot.qty, remaining)
                closed.append(makeTrade(lot: lot, a: a, fillQty: fill.qty, matched: matched, side: fill.side, symbol: lot.symbol, name: lot.name, multSymbol: lot.symbol))
                lot.commission *= (lot.qty - matched) / lot.qty
                lot.qty -= matched
                remaining -= matched
                if lot.qty <= 1e-10 { book.removeFirst() }
                else { book[0] = lot }
            }
            if remaining > 1e-10 && fill.side == "SELL" {
                for dk in bookOrder {
                    if remaining <= 1e-10 { break }
                    if dk == bookKey(a) { continue }
                    let parts = dk.components(separatedBy: "::")
                    if parts.count != 3 { continue }
                    if parts[0] != fifoAccount(a) || parts[2] != a.currency { continue }
                    if !tickerWasReplaced(activities, accountId: parts[0], symbol: parts[1], currency: parts[2], byDate: a.transactionDate) { continue }
                    var dbook = books[dk] ?? []
                    while remaining > 1e-10 && !dbook.isEmpty && dbook[0].direction == closingDir {
                        var lot = dbook[0]
                        let matched = min(lot.qty, remaining)
                        closed.append(makeTrade(lot: lot, a: a, fillQty: fill.qty, matched: matched, side: fill.side, symbol: a.symbol, name: a.name.isEmpty ? lot.name : a.name, multSymbol: a.symbol))
                        lot.commission *= (lot.qty - matched) / lot.qty
                        lot.qty -= matched
                        remaining -= matched
                        if lot.qty <= 1e-10 { dbook.removeFirst() }
                        else { dbook[0] = lot }
                    }
                    books[dk] = dbook
                }
            }
            if remaining > 1e-10 {
                if let opening = openingDirection(a, side: fill.side) {
                    book.append(Lot(
                        qty: remaining,
                        price: a.unitPrice,
                        date: a.transactionDate,
                        commission: fill.qty > 0 ? a.commission * (remaining / fill.qty) : 0,
                        direction: opening,
                        accountId: a.accountId,
                        accountType: a.accountType,
                        symbol: a.symbol,
                        name: a.name,
                        currency: a.currency,
                        activityId: a.id
                    ))
                }
            }
            books[k] = book
        }
        var openCount = 0
        for book in books.values {
            for lot in book where lot.qty > 1e-10 { openCount += 1 }
        }
        closed.sort { a, b in
            if a.exitDate != b.exitDate { return a.exitDate < b.exitDate }
            return a.id < b.id
        }
        return (closed, openCount)
    }

    // MARK: - Metrics / NAV years (ledger.html)

    static func formatCad(_ n: Double, digits: Int = 2) -> String {
        let f = NumberFormatter()
        f.locale = Locale(identifier: "en_CA")
        f.numberStyle = .decimal
        f.minimumFractionDigits = digits
        f.maximumFractionDigits = digits
        let absS = f.string(from: NSNumber(value: abs(n))) ?? String(format: "%.\(digits)f", abs(n))
        if n < 0 { return minus + "$" + absS }
        return "$" + absS
    }

    static func activityWhen(_ a: WSActivity) -> String {
        let raw = (a.occurredAt.isEmpty ? a.transactionDate : a.occurredAt).trimmingCharacters(in: .whitespacesAndNewlines)
        if raw.isEmpty { return "" }
        if let t = raw.firstIndex(of: "T") {
            let day = String(raw.prefix(10))
            let after = raw[raw.index(after: t)...]
            let clock = String(after.prefix(5))
            return clock.count == 5 ? day + " " + clock : day
        }
        return String(raw.prefix(10))
    }

    static func formatHold(_ days: Int) -> String {
        if days == 0 { return "intraday" }
        if days == 1 { return "1d" }
        if days < 60 { return "\(days)d" }
        let months = Int((Double(days) / 30.44).rounded())
        return "\(days)d (~\(months)mo)"
    }

    static func formatCloseDate(_ ymd: String) -> String {
        let raw = String(ymd.prefix(10))
        let parts = raw.split(separator: "-")
        guard parts.count == 3, let year = Int(parts[0]), let month = Int(parts[1]), let day = Int(parts[2]) else {
            return raw
        }
        let names = ["", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        let mon = (month >= 1 && month <= 12) ? names[month] : String(parts[1])
        let thisYear = Calendar.current.component(.year, from: Date())
        if year == thisYear { return "\(mon) \(day)" }
        return "\(mon) \(day), \(year)"
    }

    static func formatQty(_ n: Double) -> String {
        if n == n.rounded() {
            return String(Int(n.rounded()))
        }
        let f = NumberFormatter()
        f.locale = Locale(identifier: "en_CA")
        f.numberStyle = .decimal
        f.minimumFractionDigits = 0
        f.maximumFractionDigits = 8
        return f.string(from: NSNumber(value: n)) ?? String(n)
    }

    static func closedPnlPct(_ t: WSClosedTrade) -> Double? {
        let den = t.entryPrice * t.quantity * optionMultiplier(t.symbol)
        if den == 0 { return nil }
        return t.pnl / den
    }

    static func formatPct(_ r: Double?) -> String {
        guard let r, r.isFinite else { return emDash }
        let body = jsToFixed(abs(r) * 100, digits: 1) + "%"
        if r < 0 { return minus + body }
        return body
    }

    static func activitiesInGroup(_ g: WSClosedTrade, activities: [WSActivity]) -> [WSActivity] {
        var byId: [String: WSActivity] = [:]
        for a in activities { byId[a.id] = a }
        var seen = Set<String>()
        var acts: [WSActivity] = []
        let members = g.slices.isEmpty ? [g] : g.slices
        for s in members {
            for id in [s.buyActivityId, s.sellActivityId] {
                if id.isEmpty || seen.contains(id) { continue }
                seen.insert(id)
                if let a = byId[id] { acts.append(a) }
            }
        }
        acts.sort { a, b in
            let wa = activityWhen(a)
            let wb = activityWhen(b)
            if wa != wb { return wa < wb }
            return a.id < b.id
        }
        return acts
    }

    static func executionSide(_ a: WSActivity) -> String {
        if let s = tradeSide(a) { return s }
        let raw = compactType(a.activitySubType.isEmpty ? a.activityType : a.activitySubType)
        if raw.contains("SELL") { return "SELL" }
        return "BUY"
    }


    static func formatReturn(_ r: Double?) -> String {
        // ledger.html formatReturn: (r * 100).toFixed(1) + "%"
        guard let r, r.isFinite else { return emDash }
        let pct = jsToFixed(r * 100, digits: 1) + "%"
        if r > 0 { return "+" + pct }
        return pct
    }

    static func formatYearSpan(_ years: Double) -> String {
        if !(years > 0) { return "" }
        if years >= 1 { return String(format: "%.1f yrs", years) }
        let months = max(1, Int((years * 12).rounded()))
        return "\(months) mo"
    }

    /// ledger.html formatPct: (n * 100).toFixed(1) + "%"
    static func formatWinRate(_ r: Double) -> String {
        if !r.isFinite { return emDash }
        return jsToFixed(r * 100, digits: 1) + "%"
    }

    static func formatProfitFactor(_ pf: Double) -> String {
        if pf == Double.infinity { return "∞" }
        if !pf.isFinite { return emDash }
        return String(format: "%.2f", pf)
    }

    private static func computeMetrics(_ trades: [WSClosedTrade]) -> WSMetrics {
        let wins = trades.filter { $0.pnlCad > 0 }
        let losses = trades.filter { $0.pnlCad < 0 }
        let grossProfit = wins.reduce(0.0) { $0 + $1.pnlCad }
        let grossLoss = abs(losses.reduce(0.0) { $0 + $1.pnlCad })
        let avgWin = wins.isEmpty ? 0.0 : grossProfit / Double(wins.count)
        let avgLoss = losses.isEmpty ? 0.0 : -grossLoss / Double(losses.count)
        let winRate = trades.isEmpty ? 0.0 : Double(wins.count) / Double(trades.count)
        let lossRate = trades.isEmpty ? 0.0 : Double(losses.count) / Double(trades.count)
        let profitFactor: Double
        if grossLoss > 0 { profitFactor = grossProfit / grossLoss }
        else if grossProfit > 0 { profitFactor = .infinity }
        else { profitFactor = 0 }
        let expectancy = trades.isEmpty ? 0.0 : winRate * avgWin + lossRate * avgLoss
        let realized = trades.reduce(0.0) { $0 + $1.pnlCad }
        let avgHold = trades.isEmpty ? 0.0 : Double(trades.reduce(0) { $0 + $1.holdDays }) / Double(trades.count)
        var bySym: [String: Double] = [:]
        var bySymOrder: [String] = []
        for t in trades {
            let k = underlyingSymbol(t.symbol)
            if bySym[k] == nil { bySymOrder.append(k) }
            bySym[k, default: 0] += t.pnlCad
        }
        var maxWinSymbol = ""
        var maxWinPnl = 0.0
        var maxLossSymbol = ""
        var maxLossPnl = 0.0
        for k in bySymOrder {
            let pnl = bySym[k] ?? 0
            if pnl > maxWinPnl { maxWinPnl = pnl; maxWinSymbol = k }
            if pnl < maxLossPnl { maxLossPnl = pnl; maxLossSymbol = k }
        }
        return WSMetrics(
            realizedPnlCad: realized,
            tradeCount: trades.count,
            winCount: wins.count,
            lossCount: losses.count,
            evenCount: trades.count - wins.count - losses.count,
            grossProfit: grossProfit,
            grossLoss: grossLoss,
            winRate: winRate,
            profitFactor: profitFactor,
            avgWin: avgWin,
            avgLoss: avgLoss,
            expectancy: expectancy,
            maxWinPnl: maxWinPnl,
            maxWinSymbol: maxWinSymbol,
            maxLossPnl: maxLossPnl,
            maxLossSymbol: maxLossSymbol,
            avgHoldDays: avgHold
        )
    }

    private static func monthlyPnl(_ trades: [WSClosedTrade]) -> [WSMonthBar] {
        var map: [String: Double] = [:]
        var order: [String] = []
        for t in trades {
            let month = String(t.exitDate.prefix(7))
            if map[month] == nil { order.append(month) }
            map[month, default: 0] += t.pnlCad
        }
        return map.keys.sorted().map { WSMonthBar(month: $0, pnl: map[$0] ?? 0) }
    }

    private static func histNavOn(_ hist: [WSNavPoint], day: String) -> Double? {
        var v: Double? = nil
        for p in hist {
            if p.date > day { break }
            v = p.equity
        }
        return v
    }
    private static func histNetDepositsOn(_ hist: [WSNavPoint], day: String) -> Double? {
        var v: Double? = nil
        for p in hist {
            if p.date > day { break }
            if let nd = p.netDeposits { v = nd }
        }
        return v
    }
    private static func isoDateOnly(_ s: String) -> String {
        String(s.trimmingCharacters(in: .whitespacesAndNewlines).prefix(10))
    }

    /// JavaScript Number.prototype.toFixed. POSIX decimal, half-up, no grouping.
    private static func jsToFixed(_ n: Double, digits: Int) -> String {
        if n.isNaN { return "NaN" }
        if n.isInfinite { return n < 0 ? "-Infinity" : "Infinity" }
        let f = NumberFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.numberStyle = .decimal
        f.minimumIntegerDigits = 1
        f.minimumFractionDigits = digits
        f.maximumFractionDigits = digits
        f.roundingMode = .halfUp
        f.usesGroupingSeparator = false
        return f.string(from: NSNumber(value: n)) ?? ("0." + String(repeating: "0", count: digits))
    }

    private static func shiftIsoDate(_ iso: String, _ days: Int) -> String {
        let day = isoDateOnly(iso)
        guard let d = ymd(day) else { return day }
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = TimeZone(secondsFromGMT: 0)!
        guard let n = cal.date(byAdding: .day, value: days, to: d) else { return day }
        return isoDay(n)
    }
    private static func clampToToday(_ iso: String) -> String {
        let today = isoDay(Date())
        return iso > today ? today : iso
    }
    private static func yearWindow(_ hist: [WSNavPoint], year: String) -> (from: String, to: String, start: Double?, flowAfter: String) {
        let to = clampToToday(year + "-12-31")
        let cal = year + "-01-01"
        if hist.isEmpty { return (cal, to, nil, cal) }
        let startDay = shiftIsoDate(cal, -1)
        var start = histNavOn(hist, day: startDay)
        var from = cal
        var flowAfter = startDay
        if !(start ?? 0 > 0) {
            let first = hist.first { isoDateOnly($0.date) >= cal && isoDateOnly($0.date) <= to }
            guard let first else { return (cal, to, nil, cal) }
            from = isoDateOnly(first.date)
            start = first.equity
            flowAfter = from
        }
        return (isoDateOnly(from), isoDateOnly(to), start, isoDateOnly(flowAfter))
    }
    private static func bookReturn(_ hist: [WSNavPoint], from: String, to: String, start: Double?, flowAfter: String) -> Double? {
        if hist.isEmpty { return nil }
        let after = flowAfter.isEmpty ? shiftIsoDate(from, -1) : flowAfter
        var prevEq = start ?? histNavOn(hist, day: after) ?? 0
        if !(prevEq > 0) { return nil }
        let pts = hist.filter { $0.date > after && $0.date <= to }
        if pts.isEmpty { return nil }
        var prevNd = histNetDepositsOn(hist, day: after)
        var factor = 1.0
        for p in pts {
            let eq = p.equity
            if !eq.isFinite || !(prevEq > 0) { return nil }
            var cf = 0.0
            if let nd = p.netDeposits, let pn = prevNd { cf = nd - pn }
            factor *= 1 + (eq - prevEq - cf) / prevEq
            prevEq = eq
            if let nd = p.netDeposits { prevNd = nd }
        }
        let r = factor - 1
        if !r.isFinite { return nil }
        return r
    }
    private static func isoDayDiff(_ from: String, _ to: String) -> Double {
        // ledger.html isoDayDiff: Date.parse(day + "T12:00:00") so both ends are noon.
        guard let a = ymd(from), let b = ymd(to) else { return 0 }
        let noon: TimeInterval = 12 * 3600
        return (b.addingTimeInterval(noon).timeIntervalSince(a.addingTimeInterval(noon))) / 86400.0
    }
    private static func accountAnnualizedReturn(nav: [WSNavPoint], years: [String]) -> (rate: Double?, years: Double) {
        let ys = years.sorted()
        var prod = 1.0
        var days = 0.0
        var from = ""
        for y in ys {
            let w = yearWindow(nav, year: y)
            guard let r = bookReturn(nav, from: w.from, to: w.to, start: w.start, flowAfter: w.flowAfter),
                  r.isFinite, r > -1 else { continue }
            let d = isoDayDiff(w.from, w.to)
            if !(d >= 30) { continue }
            prod *= (1 + r)
            days += d
            if from.isEmpty { from = w.from }
        }
        if from.isEmpty || !(days > 0) { return (nil, 0) }
        let yrs = days / 365.25
        let rate = yrs >= 1.0 / 12.0 ? pow(prod, 1 / yrs) - 1 : prod - 1
        return (rate, yrs)
    }
    /// FRED S&P 500 daily closes (index, not SPY ETF).
    /// GET https://fred.stlouisfed.org/graph/fredgraph.csv?id=SP500
    /// Header: observation_date,SP500. Skip empty and "." holiday rows.
    private static let sp500FredURL = "https://fred.stlouisfed.org/graph/fredgraph.csv?id=SP500"
    private static let sp500CacheKey = "bagholder.sp500.fred.csv"

    /// Parse FRED graph CSV into YYYY-MM-DD → close. Never mix with SPY.
    private static func parseFredSP500Csv(_ text: String) -> [String: Double] {
        var map: [String: Double] = [:]
        let normalized = text.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
        for line in normalized.components(separatedBy: "\n") {
            let parts = line.split(separator: ",", omittingEmptySubsequences: false)
            if parts.count < 2 { continue }
            let quotes = CharacterSet(charactersIn: "\"")
            var d = parts[0].trimmingCharacters(in: .whitespacesAndNewlines).trimmingCharacters(in: quotes)
            d = isoDateOnly(d)
            let raw = parts[1].trimmingCharacters(in: .whitespacesAndNewlines).trimmingCharacters(in: quotes)
            if d.range(of: #"^\d{4}-\d{2}-\d{2}$"#, options: .regularExpression) == nil { continue }
            if raw.isEmpty || raw == "." { continue }
            guard let px = Double(raw), px > 0 else { continue }
            map[d] = px
        }
        return map
    }

    /// Last successful live FRED CSV (device cache, not a bundled price list).
    private static func loadCachedSP500() -> [String: Double] {
        guard let csv = UserDefaults.standard.string(forKey: sp500CacheKey), !csv.isEmpty else { return [:] }
        return parseFredSP500Csv(csv)
    }

    private static func saveCachedSP500(_ csv: String) {
        UserDefaults.standard.set(csv, forKey: sp500CacheKey)
    }

    /// Every Home pull: live FRED first. On success, cache CSV. If live fails, reuse last live cache. Else empty (em dash). Never load bundled ETF seed.
    private static func ensureSpyPrices() async -> [String: Double] {
        guard let url = URL(string: sp500FredURL) else { return loadCachedSP500() }
        var req = URLRequest(url: url, timeoutInterval: 45)
        req.httpMethod = "GET"
        req.setValue("text/csv,*/*;q=0.8", forHTTPHeaderField: "Accept")
        do {
            let (data, resp) = try await URLSession.shared.data(for: req)
            let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
            if status >= 200 && status < 300 {
                let text = String(data: data, encoding: .utf8)
                    ?? String(data: data, encoding: .isoLatin1)
                    ?? ""
                let live = parseFredSP500Csv(text)
                if !live.isEmpty {
                    saveCachedSP500(text)
                    return live
                }
            }
        } catch {
            // live failed; fall back to last successful live cache
        }
        return loadCachedSP500()
    }

    /// ledger.html spyOn (1914–1923): YYYY-MM-DD lookup, walk back up to 18 calendar days.
    private static func spyOn(_ spy: [String: Double], date: String) -> Double? {
        var d = isoDateOnly(date)
        if d.count != 10 { return nil }
        for _ in 0..<18 {
            if let v = spy[d], v > 0 { return v }
            d = shiftIsoDate(d, -1)
        }
        return nil
    }

    /// ledger.html spyReturn (1925–1930): b/a - 1 if both prices > 0 else null.
    private static func spyReturn(_ spy: [String: Double], from: String, to: String) -> Double? {
        guard let a = spyOn(spy, date: from), let b = spyOn(spy, date: to), a > 0, b > 0 else { return nil }
        return b / a - 1
    }

    /// ledger.html yearsFromActivities (2602–2608): years from activity dates, newest first.
    private static func yearsFromActivities(_ activities: [WSActivity]) -> [String] {
        var years = Set<String>()
        for a in activities {
            let d = String(isoDateOnly(a.transactionDate).prefix(4))
            if d.count == 4, d.allSatisfy(\.isNumber) { years.insert(d) }
        }
        return Array(years.sorted().reversed())
    }

    private static func annualRows(nav: [WSNavPoint], spy: [String: Double], activities: [WSActivity]) -> [WSYearRow] {
        let years = yearsFromActivities(activities)
        return years.map { y in
            let w = yearWindow(nav, year: y)
            let mine = bookReturn(nav, from: w.from, to: w.to, start: w.start, flowAfter: w.flowAfter)
            // compareRow else branch: unfiltered NAV, idxFrom=w.from, idxTo=w.to (same yearWindow as bookReturn)
            let idx = spyReturn(spy, from: isoDateOnly(w.from), to: isoDateOnly(w.to))
            let vs: Double?
            if let mine, let idx {
                vs = mine - idx
            } else {
                vs = nil
            }
            return WSYearRow(year: y, ret: formatReturn(mine), spy: formatReturn(idx), vs: formatReturn(vs))
        }
    }

    // MARK: - FX (ledger.html toCad / applyFx / ensureFxRates)

    private static let fxFallback = 1.35
    private static let fxCacheKey = "bagholder.fx.boc.v1"

    /// ledger.html rateOn (1731–1739): walk back up to 12 calendar days, else 1.35.
    private static func rateOn(_ fx: [String: Double], date: String) -> Double {
        var d = String(date.prefix(10))
        if d.isEmpty { return fxFallback }
        for _ in 0..<12 {
            if let r = fx[d], r > 0 { return r }
            d = shiftIsoDate(d, -1)
        }
        return fxFallback
    }

    /// ledger.html toCad (1742–1746): CAD (and anything not USD) uses the native amount; USD * FXUSDCAD.
    private static func toCad(_ amount: Double, currency: String, date: String, fx: [String: Double]) -> Double {
        if currency.uppercased() != "USD" { return amount }
        return amount * rateOn(fx, date: date)
    }

    /// ledger.html applyFx (1748–1764): convert USD entry/exit notionals on their own dates. CAD pnlCad = pnl.
    private static func applyFx(_ trades: [WSClosedTrade], fx: [String: Double]) -> [WSClosedTrade] {
        trades.map { t in
            var t = t
            let ccy = t.currency.uppercased()
            if ccy != "USD" {
                t.pnlCad = t.pnl
                return t
            }
            let qty = t.quantity
            let entryC = t.entryCommission
            let exitC = t.exitCommission
            let entryNotional = t.entryPrice * qty * optionMultiplier(t.symbol)
            let exitNotional = t.exitPrice * qty * optionMultiplier(t.symbol)
            if t.openDirection == "SHORT" {
                t.pnlCad = toCad(entryNotional - entryC, currency: ccy, date: t.entryDate, fx: fx)
                    - toCad(exitNotional + exitC, currency: ccy, date: t.exitDate, fx: fx)
            } else {
                t.pnlCad = toCad(exitNotional - exitC, currency: ccy, date: t.exitDate, fx: fx)
                    - toCad(entryNotional + entryC, currency: ccy, date: t.entryDate, fx: fx)
            }
            return t
        }
    }

    /// ledger.html groupClosedByClose (2776–2819): one metrics row per close (symbol/ccy/exitDate/exitPrice/side/dir).
    /// Sums pnlCad. Not the Closed-trades table grouping (groupClosedByExit).
    private static func groupClosedByClose(_ trades: [WSClosedTrade]) -> [WSClosedTrade] {
        var map: [String: WSClosedTrade] = [:]
        var order: [String] = []
        var entryNotional: [String: Double] = [:]
        for s in trades {
            // ledger.html 2780: [symbol, currency, exitDate, Number(exitPrice).toFixed(8), side, openDirection]
            let k = [
                s.symbol,
                s.currency,
                s.exitDate,
                jsToFixed(s.exitPrice, digits: 8),
                s.side,
                s.openDirection,
            ].joined(separator: "|")
            if map[k] == nil {
                var g = s
                g.quantity = 0
                g.pnl = 0
                g.pnlCad = 0
                g.commission = 0
                map[k] = g
                order.append(k)
                entryNotional[k] = 0
            }
            var g = map[k]!
            g.quantity += s.quantity
            g.pnl += s.pnl
            g.pnlCad += s.pnlCad
            g.commission += s.commission
            entryNotional[k, default: 0] += s.entryPrice * s.quantity
            if s.entryDate < g.entryDate { g.entryDate = s.entryDate }
            map[k] = g
        }
        return order.map { k in
            var g = map[k]!
            let en = entryNotional[k] ?? 0
            g.entryPrice = g.quantity != 0 ? en / g.quantity : 0
            g.holdDays = daysBetween(g.entryDate, g.exitDate)
            return g
        }
    }

    /// ledger.html groupClosedByExit with no saved groups: defaultGroupsUntilSideChange.
    static func closedTradesTable(_ trades: [WSClosedTrade], activities _: [WSActivity] = []) -> [WSClosedTrade] {
        let grouped = defaultGroupsUntilSideChange(trades)
        return grouped.sorted { a, b in
            if a.exitDate != b.exitDate { return a.exitDate > b.exitDate }
            return a.id > b.id
        }
    }

    private static func sliceMemberKey(_ t: WSClosedTrade) -> String {
        if !t.buyActivityId.isEmpty && !t.sellActivityId.isEmpty {
            return [t.buyActivityId, t.sellActivityId, jsToFixed(t.quantity, digits: 8)].joined(separator: "|")
        }
        return t.id
    }

    private static func groupLaneKey(_ t: WSClosedTrade) -> String {
        [t.accountId, t.symbol, t.currency].joined(separator: "|")
    }

    private static func groupIdForKeys(_ keys: [String]) -> String {
        let s = keys.sorted().joined(separator: "\n")
        var h: UInt32 = 2_166_136_261
        for u in s.utf8 {
            h ^= UInt32(u)
            h = h &* 16_777_619
        }
        return "g_" + String(h, radix: 16) + "_" + String(keys.count)
    }

    private static func collapseClosedGroup(_ slices: [WSClosedTrade], id: String) -> WSClosedTrade {
        var g = slices[0]
        g.id = id
        g.quantity = 0
        g.pnl = 0
        g.pnlCad = 0
        g.commission = 0
        var entryN = 0.0
        var exitN = 0.0
        g.entryDate = slices[0].entryDate
        g.exitDate = slices[0].exitDate
        for s in slices {
            g.quantity += s.quantity
            g.pnl += s.pnl
            g.pnlCad += s.pnlCad
            g.commission += s.commission
            entryN += s.entryPrice * s.quantity
            exitN += s.exitPrice * s.quantity
            if s.entryDate < g.entryDate { g.entryDate = s.entryDate }
            if s.exitDate > g.exitDate { g.exitDate = s.exitDate }
        }
        g.entryPrice = g.quantity != 0 ? entryN / g.quantity : 0
        g.exitPrice = g.quantity != 0 ? exitN / g.quantity : slices[0].exitPrice
        g.holdDays = daysBetween(g.entryDate, g.exitDate)
        g.slices = slices
        return g
    }

    private static func defaultGroupsUntilSideChange(_ slices: [WSClosedTrade]) -> [WSClosedTrade] {
        var lanes: [String: [WSClosedTrade]] = [:]
        var laneOrder: [String] = []
        for s in slices {
            let k = groupLaneKey(s)
            if lanes[k] == nil { laneOrder.append(k) }
            lanes[k, default: []].append(s)
        }
        var out: [WSClosedTrade] = []
        for k in laneOrder {
            var list = lanes[k] ?? []
            list.sort { a, b in
                if a.exitDate != b.exitDate { return a.exitDate < b.exitDate }
                if a.entryDate != b.entryDate { return a.entryDate < b.entryDate }
                return sliceMemberKey(a) < sliceMemberKey(b)
            }
            var cur: [WSClosedTrade] = []
            var dir: String?
            func flush() {
                if cur.isEmpty { return }
                out.append(collapseClosedGroup(cur, id: groupIdForKeys(cur.map(sliceMemberKey))))
                cur = []
            }
            for s in list {
                if let d = dir, s.openDirection != d { flush() }
                dir = s.openDirection
                cur.append(s)
            }
            flush()
        }
        return out
    }

    private static func loadCachedFx() -> [String: Double] {
        guard let obj = UserDefaults.standard.dictionary(forKey: fxCacheKey) else { return [:] }
        var map: [String: Double] = [:]
        for (k, v) in obj {
            let n = J.num(v, default: Double.nan)
            if n > 0 { map[k] = n }
        }
        return map
    }

    private static func saveCachedFx(_ map: [String: Double]) {
        UserDefaults.standard.set(map, forKey: fxCacheKey)
    }

    /// Bank of Canada FXUSDCAD daily. ledger.html ensureFxRates (1767–1786).
    private static func ensureFxRates(activities: [WSActivity]) async -> [String: Double] {
        let dates = activities.map(\.transactionDate).filter { !$0.isEmpty }.sorted()
        let start = dates.first ?? "2020-01-01"
        let end = isoDay(Date())
        var map = loadCachedFx()
        let keys = map.keys.sorted()
        if let first = keys.first, let last = keys.last, first <= start, last >= shiftIsoDate(end, -5) {
            return map
        }
        var comps = URLComponents(string: "https://www.bankofcanada.ca/valet/observations/FXUSDCAD/json")
        comps?.queryItems = [
            URLQueryItem(name: "start_date", value: start),
            URLQueryItem(name: "end_date", value: end),
        ]
        guard let url = comps?.url else { return map }
        var req = URLRequest(url: url, timeoutInterval: 45)
        req.httpMethod = "GET"
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        do {
            let (data, resp) = try await URLSession.shared.data(for: req)
            let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
            guard status >= 200 && status < 300,
                  let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any]
            else { return map }
            for ob in J.arr(obj["observations"]) {
                let d = J.dict(ob)
                let day = J.str(d, "d")
                let fx = J.dict(d["FXUSDCAD"])
                let v = J.num(fx["v"], default: Double.nan)
                if day.count == 10, v > 0 { map[day] = v }
            }
            if !map.isEmpty { saveCachedFx(map) }
        } catch {
            // live failed; keep cache (rateOn falls back to 1.35)
        }
        return map
    }
}

private enum J {
    static func dict(_ any: Any?) -> [String: Any] { any as? [String: Any] ?? [:] }
    static func arr(_ any: Any?) -> [Any] { any as? [Any] ?? [] }
    static func str(_ any: Any?) -> String {
        if any == nil || any is NSNull { return "" }
        if let s = any as? String { return s }
        if let n = any as? NSNumber { return n.stringValue }
        return "\(any!)"
    }
    static func str(_ d: [String: Any], _ k: String) -> String { str(d[k]) }
    static func num(_ any: Any?, default def: Double = 0) -> Double {
        if any == nil || any is NSNull { return def }
        if let n = any as? NSNumber { return n.doubleValue }
        if let s = any as? String {
            if s.isEmpty { return def }
            return Double(s) ?? def
        }
        return def
    }
    static func bool(_ any: Any?) -> Bool {
        if let b = any as? Bool { return b }
        if let n = any as? NSNumber { return n.boolValue }
        return false
    }
}
