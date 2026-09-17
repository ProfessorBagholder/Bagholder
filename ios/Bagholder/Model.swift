// The derived model, a port of model.py (the reference), function for
// function. Everything a screen shows comes from here so that one list of
// trades feeds every tile, table and chart, and so the numbers can be tested:
// ModelCasesTests runs tests/cases through it and compares with the Python.
//
// Pipeline
//     activities  -> normalizeActivities  (crypto, options, stock-dividend notices)
//                 -> synthesizeAssignmentShares
//                 -> matchFifo            (FIFO lots per account+symbol+currency,
//                                          round-trip ids, option rolls)
//                 -> synthesizeExpiries   (then matchFifo again)
//                 -> applyFx              (P&L in CAD on the fill dates)
//                 -> buildTrades          (round trips)
//                 -> buildPositions       (open lots rolled up per symbol+account)
//                 -> buildCashflow        (dividends, interest, withholding tax)
//     kpi(trades), cashflowView(base)
//
// Currency: per-trade numbers are native. Anything that adds trades together
// uses the CAD value converted on the fill date with the Bank of Canada rate.
import Foundation

// MARK: - rows

/// A normalized activity. A class, as in the Python where fills, lots and the
/// trade's fill list all see the row the matcher amended (a multileg's
/// inferred quantity and side, for one).
final class BHAct {
    var id = "", occurredAt = "", transactionDate = "", accountId = "", fifoId = "", accountType = ""
    var activityType = "", activitySubType = "", description = "", direction = "", symbol = "", name = "", currency = ""
    var quantity = 0.0, unitPrice = 0.0, commission = 0.0, netCashAmount = 0.0
    var category = "", rawType = "", aftType = "", securityId = ""
    var kind = ""
    var flags: [String] = []

    init() {}

    convenience init(_ w: WSActivity) {
        self.init()
        id = w.id; occurredAt = w.occurredAt; transactionDate = w.transactionDate; accountId = w.accountId
        fifoId = w.fifoId; accountType = w.accountType; activityType = w.activityType; activitySubType = w.activitySubType
        description = w.description; direction = w.direction; symbol = w.symbol; name = w.name; currency = w.currency
        quantity = w.quantity; unitPrice = w.unitPrice; commission = w.commission; netCashAmount = w.netCashAmount
        category = w.category; rawType = w.rawType; aftType = w.aftType; securityId = w.securityId
    }

    func copy() -> BHAct {
        let a = BHAct()
        a.id = id; a.occurredAt = occurredAt; a.transactionDate = transactionDate; a.accountId = accountId
        a.fifoId = fifoId; a.accountType = accountType; a.activityType = activityType; a.activitySubType = activitySubType
        a.description = description; a.direction = direction; a.symbol = symbol; a.name = name; a.currency = currency
        a.quantity = quantity; a.unitPrice = unitPrice; a.commission = commission; a.netCashAmount = netCashAmount
        a.category = category; a.rawType = rawType; a.aftType = aftType; a.securityId = securityId
        a.kind = kind; a.flags = flags
        return a
    }
}

struct BHSecurity {
    var id = "", symbol = "", name = "", underlyingId = "", primaryExchange = "", primaryMic = "", currency = ""
}

struct BHDistribution {
    var exDate = "", payDate = "", amount = 0.0, currency = ""
}

struct BHQuote {
    var price: Double?
    var priceChange: Double?
    var percentChange: Double?
    var fetchedAt = ""
    var exDividendDate = ""
}

struct BHMarket {
    var fx: [String: Double] = [:]
    var benchmark: [String: Double] = [:]
    var benchmarks: [String: [String: Double]] = [:]
    var distributions: [String: [BHDistribution]] = [:]
    var quotes: [String: BHQuote] = [:]
}

/// One day of Wealthsimple's NAV history.
struct BHNavPoint {
    var date = ""
    var equity: Double?
    var netDeposits: Double?
}

/// A journal entry, keyed by the round trip that opened the trade or position.
struct BHJournalEntry {
    var grade = "", thesis = ""
    var tags: [String] = []
}

struct BHLot {
    var qty: Double, price: Double, date: String, when: String, commission: Double, direction: String
    var accountId: String, accountType: String, symbol: String, name: String, currency: String, kind: String
    var activityId: String, securityId: String
    var rt: String?
    var flags: [String]
}

struct BHSlice {
    var id = "", rt: String?
    var accountId = "", accountType = "", account = "", symbol = "", name = "", currency = "", kind = "", side = ""
    var quantity = 0.0, entryPrice = 0.0, exitPrice = 0.0
    var entryDate = "", exitDate = "", entryWhen = "", exitWhen = ""
    var holdDays = 0
    var commission = 0.0, entryCommission = 0.0, exitCommission = 0.0, pnl = 0.0, pnlCad = 0.0, feesCad = 0.0
    var openDirection = "", buyActivityId = "", sellActivityId = "", securityId = ""
    var flags: [String] = []
}

struct BHUnmatched {
    var symbol = "", currency = "", side = "", quantity = 0.0, price = 0.0, date = "", description = ""
    var accountId = "", account = "", activityId = ""
}

struct BHFillRow {
    var id = "", when = "", date = "", time = "", side = "", sub = ""
    var qty = 0.0, price = 0.0, amount = 0.0, fees = 0.0
    var currency = ""
    var flags: [String] = []
}

struct BHTrade {
    var id = "", status = "closed", symbol = "", underlying = "", name = "", exchange = "", kind = "", currency = ""
    var account = "", accountId = "", securityId = "", side = "", openDirection = ""
    var qty = 0.0, mult = 1.0, entry = 0.0, exit = 0.0
    var entryDate = "", exitDate = "", entryWhen = "", exitWhen = ""
    var holdDays = 0
    var pnl = 0.0, pnlCad = 0.0, fees = 0.0, feesCad = 0.0
    var pnlPct: Double?
    var legCount = 0
    var fills: [BHFillRow] = []
    var flags: [String] = []
    var grade = "", thesis = ""
    var tags: [String] = []
}

struct BHPositionLot {
    var opened = "", qty = 0.0, price = 0.0, basis = 0.0, held = 0, flags: [String] = [], activityId = ""
}

struct BHPosition {
    var id = "", symbol = "", underlying = "", name = "", exchange = "", kind = "", account = "", accountId = ""
    var currency = "", securityId = ""
    var short = false
    var qty = 0.0, mult = 1.0, avg = 0.0, cost = 0.0, fees = 0.0, last = 0.0
    var lastAt = "", priceSource = ""
    var priceChange: Double?, percentChange: Double?
    var mv = 0.0, unreal = 0.0
    var unrealPct: Double?
    var held = 0
    var opened = ""
    var rt: String?
    var lots: [BHPositionLot] = []
    var alloc = 0.0
    var dayChange: Double?
    var fills: [BHFillRow] = []
    var grade = "", thesis = ""
    var tags: [String] = []
}

/// What Wealthsimple states per account: its net liquidation value, in its currency.
struct BHAccountInfo {
    var id = "", name = "", currency = ""
    var nav: Double?
    var type = "", status = ""
}

struct BHBalanceRow {
    var accountId = "", securityId = ""
    var quantity = 0.0
}

/// Wealthsimple's buying power for an account, or why it has none.
struct BHMarginRow {
    var accountId = ""
    var buyingPower: Double?
    var currency = "CAD"
    var unavailable = ""
}

struct BHAllocationRow {
    var id = "", symbol = "", account = ""
    var value = 0.0, share = 0.0
}

/// The Portfolio tiles: CAD aggregates over the accounts in scope (model.py portfolio_view).
struct BHPortfolio {
    var allocation: [BHAllocationRow] = []
    var marketValue = 0.0, costBasis = 0.0, unrealized = 0.0
    var unrealizedPct: Double?
    var positionCount = 0, accountCount = 0
    var nav: Double?
    var navAccounts = 0
    var marginUsed = 0.0
    var marginUsedBy: [String: Double] = [:]
    var marginUsedPct: Double?
    var availableMargin: Double?
    var availableMarginUnavailable: [String] = []
    var hasMargin = false
    var cash = 0.0
    var cashPct: Double?, dayChange: Double?, dayChangePct: Double?
}

struct BHCashRow {
    var id = "", date = "", time = "", symbol = "", name = "", kind = "", account = "", accountId = ""
    var qty: Double?, per: Double?
    var amount = 0.0, currency = "", amountCad = 0.0
}

struct BHHolding {
    var id = "", symbol = "", account = ""
    var qty = 0.0
    var per: Double?
    var freq: Int?
    var freqVerified = false
    var rateSource = ""
    var cost = 0.0, avg = 0.0, last = 0.0
    var priceSource = ""
    var ytd = 0.0, ttm = 0.0, all = 0.0
    var nextExDate = "", nextPayDate = ""
    var exPast = false, payPast = false
    var yob: Double?, annual: Double?, yoc: Double?, currentYield: Double?
}

struct BHTile {
    var label = ""
    var total: Double?, perMonth: Double?
    var count: Int?
    var yield: Double?, projected: Double?, earned: Double?, book: Double?
    var marginUsed: Double?, interestPerMonth: Double?, interestMonths: Int?
}

struct BHMonth {
    var key = "", label = "", value = 0.0, count = 0
}

struct BHKPI {
    var realized = 0.0
    var count = 0, wins = 0, losses = 0, breakeven = 0
    var winRate: Double?
    var grossWin = 0.0, grossLoss = 0.0
    var profitFactor: Double?
    var profitFactorInfinite = false
    var expectancy: Double?
    var avgWin = 0.0, avgLoss = 0.0, fees = 0.0
    var avgHold: Double?
    var openCount = 0
}

struct BHCashflowView {
    var skipped: [String] = []
    var tiles: [BHTile] = []
    var months: [BHMonth] = []
    var holdings: [BHHolding] = []
    var rows: [BHCashRow] = []
    var other: [BHCashRow] = []
    var total = 0.0
    var count = 0
    var interest = 0.0, withholding = 0.0
}

struct BHBase {
    var today = ""
    var fx: [String: Double] = [:]
    var benchmark: [String: Double] = [:]
    var benchmarks: [String: [String: Double]] = [:]
    var equity: [BHEquityPoint] = []
    var equityByAccount: [String: [BHEquityPoint]] = [:]
    var journal: [String: BHJournalEntry] = [:]
    var distributions: [String: [BHDistribution]] = [:]
    var quotes: [String: BHQuote] = [:]
    var activities: [BHAct] = []
    var closed: [BHSlice] = []
    var openLots: [BHLot] = []
    var unmatched: [BHUnmatched] = []
    var trades: [BHTrade] = []
    var positions: [BHPosition] = []
    var cashflow: [BHCashRow] = []
    var accounts: [BHAccountInfo] = []
    var balances: [BHBalanceRow] = []
    var margin: [BHMarginRow] = []
    var cashCurrencies: [String: String] = [:]
}

// MARK: - the model

enum BHModel {
    static let eps = 1e-10
    static let fxFallback = 1.35
    static let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
    static let kinds = ["Shares", "Options", "Crypto", "Futures"]
    static let schedules = [52, 26, 24, 12, 6, 4, 2, 1]
    static let timeZone = TimeZone(identifier: "America/Edmonton")!

    // MARK: small helpers

    private static var regexCache: [String: NSRegularExpression] = [:]
    private static let regexLock = NSLock()

    static func re(_ pattern: String) -> NSRegularExpression {
        regexLock.lock(); defer { regexLock.unlock() }
        if let r = regexCache[pattern] { return r }
        let r = try! NSRegularExpression(pattern: pattern)
        regexCache[pattern] = r
        return r
    }

    static func reTest(_ pattern: String, _ s: String) -> Bool {
        re(pattern).firstMatch(in: s, range: NSRange(s.startIndex..., in: s)) != nil
    }

    static func reGroup(_ pattern: String, _ s: String, _ group: Int = 1) -> String? {
        guard let m = re(pattern).firstMatch(in: s, range: NSRange(s.startIndex..., in: s)) else { return nil }
        guard let r = Range(m.range(at: group), in: s) else { return nil }
        return String(s[r])
    }

    static func reSub(_ pattern: String, _ s: String, _ with: String) -> String {
        re(pattern).stringByReplacingMatches(in: s, range: NSRange(s.startIndex..., in: s), withTemplate: with)
    }

    static let spacePattern = "[\\s\\u00a0\\u2000-\\u200b\\u202f\\u205f\\u3000]+"

    /// The unicode spaces the page's regex collapses (`_SPACE_RE` in model.py).
    static func isSpaceLike(_ c: Character) -> Bool {
        if c.isWhitespace || c.isNewline { return true }
        guard let v = c.unicodeScalars.first?.value else { return false }
        return v == 0xA0 || (0x2000...0x200B).contains(v) || v == 0x202F || v == 0x205F || v == 0x3000
    }

    /// Upper case with every space, underscore and hyphen removed (no regex: this runs on every row, many times).
    static func compact(_ s: String) -> String {
        var out = ""
        out.reserveCapacity(s.utf8.count)
        for c in s.uppercased() where !(isSpaceLike(c) || c == "_" || c == "-") { out.append(c) }
        return out
    }

    /// Runs of space-like characters collapsed to one space, trimmed.
    static func collapseSpaces(_ s: String) -> String {
        var out = ""
        out.reserveCapacity(s.utf8.count)
        var pendingSpace = false
        for c in s {
            if isSpaceLike(c) { pendingSpace = !out.isEmpty; continue }
            if pendingSpace { out.append(" "); pendingSpace = false }
            out.append(c)
        }
        return out
    }

    static func normAccountName(_ s: String) -> String { collapseSpaces(s) }

    static func spaced(_ symbol: String) -> String { collapseSpaces(symbol.uppercased()) }

    // The symbol questions are asked for every fill, lot and slice: answered once per symbol.
    private static var symbolCache: [String: (option: Bool, underlying: String, right: String)] = [:]
    private static let symbolLock = NSLock()

    private static func symbolFacts(_ symbol: String) -> (option: Bool, underlying: String, right: String) {
        symbolLock.lock(); defer { symbolLock.unlock() }
        if let f = symbolCache[symbol] { return f }
        let f = (isOptionSymbolSlow(symbol), underlyingSymbolSlow(symbol), optionRightSlow(symbol))
        symbolCache[symbol] = f
        return f
    }

    static func isOptionSymbol(_ symbol: String) -> Bool { symbolFacts(symbol).option }
    static func underlyingSymbol(_ symbol: String) -> String { symbolFacts(symbol).underlying }
    static func optionRight(_ symbol: String) -> String { symbolFacts(symbol).right }

    private static func isOptionSymbolSlow(_ symbol: String) -> Bool {
        let u = spaced(symbol)
        if u.isEmpty { return false }
        if reTest("\\b(PUT|CALL)\\b", u) || reTest("\\s[CP]$", u) { return true }
        if reTest("^[A-Z][A-Z0-9.]{0,9} \\d{6}[CP]\\d+", u) { return true }
        return false
    }

    private static func underlyingSymbolSlow(_ symbol: String) -> String {
        let s = symbol.trimmingCharacters(in: .whitespacesAndNewlines)
        if s.isEmpty { return "—" }
        let u = spaced(s)
        if reTest("\\b(PUT|CALL)\\b", u) || reTest("\\s[CP]$", u) {
            let first = u.components(separatedBy: " ")[0]
            return first.isEmpty ? s : first
        }
        if let m = reGroup("^([A-Z][A-Z0-9.]{0,9}) \\d{6}[CP]\\d+", u) { return m }
        if let m = reGroup("^([A-Z][A-Z0-9.]{0,9}) \\d{1,2}[A-Z]{3}\\d{2}\\b", u) { return m }
        return s
    }

    static func optionMultiplier(_ symbol: String) -> Double { isOptionSymbol(symbol) ? 100 : 1 }

    private static func optionRightSlow(_ symbol: String) -> String {
        let u = spaced(symbol)
        if u.hasSuffix(" PUT") || u.hasSuffix(" P") || reTest(" \\d{6}P\\d+", u) { return "PUT" }
        return "CALL"
    }

    static func isMultileg(_ a: BHAct) -> Bool { compact(a.rawType).contains("MULTILEG") }

    static func rollKey(_ a: BHAct) -> String {
        fifoAccount(a) + "\u{1}" + underlyingSymbol(a.symbol) + "\u{1}" + optionRight(a.symbol)
    }

    /// Days since the civil epoch for an ISO date, nil when it is not one.
    private static var dayCache: [String: Int] = [:]

    static func dayNumber(_ iso: String) -> Int? {
        let s = String(iso.prefix(10))
        let u = Array(s.utf8)
        guard u.count == 10, u[4] == 45, u[7] == 45 else { return nil }
        for (i, b) in u.enumerated() where i != 4 && i != 7 { if b < 48 || b > 57 { return nil } }
        symbolLock.lock()
        if let cached = dayCache[s] { symbolLock.unlock(); return cached }
        symbolLock.unlock()
        var y = Int(u[0] - 48) * 1000 + Int(u[1] - 48) * 100 + Int(u[2] - 48) * 10 + Int(u[3] - 48)
        let m = Int(u[5] - 48) * 10 + Int(u[6] - 48), d = Int(u[8] - 48) * 10 + Int(u[9] - 48)
        guard (1...12).contains(m), (1...31).contains(d) else { return nil }
        if d > daysInMonth(y, m) { return nil }
        if m <= 2 { y -= 1 }
        let era = (y >= 0 ? y : y - 399) / 400
        let yoe = y - era * 400
        let mp = (m + 9) % 12
        let doy = (153 * mp + 2) / 5 + d - 1
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
        let n = era * 146097 + doe - 719468
        symbolLock.lock(); dayCache[s] = n; symbolLock.unlock()
        return n
    }

    static func daysInMonth(_ y: Int, _ m: Int) -> Int {
        switch m {
        case 2: return (y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)) ? 29 : 28
        case 4, 6, 9, 11: return 30
        default: return 31
        }
    }

    static func isoDate(fromDayNumber z0: Int) -> String {
        let z = z0 + 719468
        let era = (z >= 0 ? z : z - 146096) / 146097
        let doe = z - era * 146097
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365
        let y = yoe + era * 400
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100)
        let mp = (5 * doy + 2) / 153
        let d = doy - (153 * mp + 2) / 5 + 1
        let m = mp < 10 ? mp + 3 : mp - 9
        return String(format: "%04d-%02d-%02d", m <= 2 ? y + 1 : y, m, d)
    }

    static func daysBetween(_ a: String, _ b: String) -> Int {
        guard let da = dayNumber(a), let db = dayNumber(b) else { return 0 }
        return max(0, db - da)
    }

    static func shiftDate(_ iso: String, _ days: Int) -> String {
        guard let d = dayNumber(iso) else { return String(iso.prefix(10)) }
        return isoDate(fromDayNumber: d + days)
    }

    static func todayLocal() -> String {
        let f = DateFormatter()
        f.timeZone = timeZone
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd"
        return f.string(from: Date())
    }

    private static let isoFractional: ISO8601DateFormatter = { let f = ISO8601DateFormatter(); f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]; return f }()
    private static let isoPlain: ISO8601DateFormatter = { let f = ISO8601DateFormatter(); f.formatOptions = [.withInternetDateTime]; return f }()
    private static let localDay: DateFormatter = { let f = DateFormatter(); f.timeZone = timeZone; f.locale = Locale(identifier: "en_US_POSIX"); f.dateFormat = "yyyy-MM-dd"; return f }()
    private static let localClock: DateFormatter = { let f = DateFormatter(); f.timeZone = timeZone; f.locale = Locale(identifier: "en_US_POSIX"); f.dateFormat = "HH:mm"; return f }()
    private static var whenCache: [String: (String, String)] = [:]

    /// ISO instant -> (YYYY-MM-DD, HH:MM) in the app's local time zone.
    static func whenParts(_ occurred: String) -> (String, String) {
        let s = occurred.trimmingCharacters(in: .whitespacesAndNewlines)
        if s.isEmpty { return ("", "") }
        if !s.contains("T") { return (String(s.prefix(10)), "") }
        symbolLock.lock()
        if let hit = whenCache[s] { symbolLock.unlock(); return hit }
        symbolLock.unlock()
        let dt = isoFractional.date(from: s) ?? isoPlain.date(from: s) ?? isoPlain.date(from: s + "Z")
        guard let date = dt else { return (String(s.prefix(10)), "") }
        let out = (localDay.string(from: date), localClock.string(from: date))
        symbolLock.lock(); whenCache[s] = out; symbolLock.unlock()
        return out
    }

    /// `%.8f`, without the formatter: this keys every slice.
    static func fmt8(_ v: Double) -> String {
        if !v.isFinite || abs(v) > 9e9 { return String(format: "%.8f", v) }
        let scaled = (abs(v) * 1e8).rounded()
        let whole = Int64(scaled / 1e8), frac = Int64(scaled.truncatingRemainder(dividingBy: 1e8))
        var f = String(frac)
        if f.count < 8 { f = String(repeating: "0", count: 8 - f.count) + f }
        return (v < 0 && scaled > 0 ? "-" : "") + String(whole) + "." + f
    }

    /// store.trade_side.
    static func tradeSide(_ a: BHAct) -> String {
        let sub = a.activitySubType.uppercased().replacingOccurrences(of: " ", with: "").replacingOccurrences(of: "_", with: "").replacingOccurrences(of: "-", with: "")
        if ["BUY", "BUYTOOPEN", "BTO", "BUYTOCLOSE", "BTC"].contains(sub) || sub.hasPrefix("BUY") { return "BUY" }
        if ["SELL", "SELLTOOPEN", "STO", "SELLTOCLOSE", "STC"].contains(sub) || sub.hasPrefix("SELL") { return "SELL" }
        let typ = a.activityType.uppercased().replacingOccurrences(of: " ", with: "").replacingOccurrences(of: "_", with: "").replacingOccurrences(of: "-", with: "")
        if typ.hasPrefix("BUY") { return "BUY" }
        if typ.hasPrefix("SELL") { return "SELL" }
        if a.quantity > 0 { return "BUY" }
        if a.quantity < 0 { return "SELL" }
        return ""
    }

    // MARK: activity normalization

    static func isCryptoActivity(_ a: BHAct) -> Bool {
        compact(a.rawType).hasPrefix("CRYPTO") || compact(a.activityType).hasPrefix("CRYPTO")
    }

    static func kindOf(_ a: BHAct) -> String {
        if kinds.contains(a.kind) { return a.kind }
        if isCryptoActivity(a) { return "Crypto" }
        if isOptionSymbol(a.symbol) { return "Options" }
        return "Shares"
    }

    static func isIntentionalOpen(_ a: BHAct) -> Bool {
        let fields = [compact(a.activityType), compact(a.activitySubType)]
        if fields.contains(where: { $0.contains("TOOPEN") }) { return true }
        return fields.contains(where: { $0 == "STO" || $0 == "BTO" })
    }

    static func isCloseOnly(_ a: BHAct) -> Bool {
        let fields = [compact(a.activityType), compact(a.activitySubType)]
        if fields.contains(where: { $0.contains("TOCLOSE") || $0 == "BTC" || $0 == "STC" }) { return true }
        return fields.contains(where: { $0.contains("EXPIR") || $0.contains("ASSIGN") || $0.contains("EXERCISE") })
    }

    static func openingDirection(_ a: BHAct, _ side: String) -> String? {
        if side == "BUY" { return isCloseOnly(a) ? nil : "LONG" }
        if side == "SELL" {
            if isOptionSymbol(a.symbol) { return isCloseOnly(a) ? nil : "SHORT" }
            if isIntentionalOpen(a) { return "SHORT" }
            return nil
        }
        return nil
    }

    /// Copy of the row with crypto and option events expressed as trade fills.
    static func normalizeActivity(_ activity: BHAct) -> BHAct {
        let a = activity.copy()
        a.accountType = normAccountName(a.accountType)
        let rt = compact(a.rawType)
        let at = compact(a.activityType)
        let cash = a.netCashAmount
        let qty = abs(a.quantity)
        a.flags = []

        if rt == "CRYPTOBUY" || at == "CRYPTOBUY" {
            a.category = "trade"; a.activityType = "Trade"; a.activitySubType = "BUY"; a.kind = "Crypto"
            a.quantity = qty
            a.netCashAmount = -abs(cash)
            return a
        }
        if rt == "CRYPTOSELL" || at == "CRYPTOSELL" {
            a.category = "trade"; a.activityType = "Trade"; a.activitySubType = "SELL"; a.kind = "Crypto"
            a.quantity = -qty
            a.netCashAmount = abs(cash)
            return a
        }
        if rt == "CRYPTOTRANSFER" || at == "CRYPTOTRANSFER" {
            let sub = compact(a.activitySubType)
            a.category = "trade"; a.activityType = "Trade"; a.kind = "Crypto"
            a.flags.append("transfer")
            if sub.contains("OUT") || cash < 0 {
                a.activitySubType = "SELL"
                a.flags.append("transfer-out")
                a.quantity = -qty
                a.netCashAmount = abs(cash)
            } else {
                a.activitySubType = "BUY"
                a.quantity = qty
                a.netCashAmount = -abs(cash)
            }
            return a
        }
        if rt == "CRYPTOSTAKINGREWARD" || at == "CRYPTOSTAKINGREWARD" {
            a.category = "trade"; a.activityType = "Trade"; a.activitySubType = "BUY"; a.kind = "Crypto"
            a.flags.append("reward")
            a.quantity = qty
            a.unitPrice = 0
            a.netCashAmount = 0
            return a
        }
        if rt.hasPrefix("CRYPTO") {
            a.category = "other"
            a.kind = "Crypto"
            return a
        }

        if at == "STKDIS" && rt == "DIVIDEND" && abs(cash) < eps {
            // A distribution posted in units with no cash is a pending notice,
            // not a share delivery: Wealthsimple's balance does not grow by it.
            a.category = "other"
            a.flags.append("pending-distribution")
            return a
        }

        let raw = rt + at
        if raw.contains("MULTILEG") {
            a.category = "trade"
            if cash < 0 || compact(a.direction) == "DEBIT" {
                a.activityType = "OPTIONS_BUY"
                a.activitySubType = "BUYTOCLOSE"
            } else {
                a.activityType = "OPTIONS_SELL"
                a.activitySubType = "SELLTOOPEN"
            }
        } else if raw.contains("EXPIR") || raw.contains("ASSIGN") || raw.contains("EXERCISE") {
            a.category = "option_event"
            if raw.contains("ASSIGN") {
                a.activityType = "ASSIGN"
                a.activitySubType = "BUYTOCLOSE"
                a.unitPrice = 0
            } else if raw.contains("SHORTEXPIR") {
                a.activityType = "EXPIR"
                a.activitySubType = "BUY"
            } else if raw.contains("EXPIR") {
                a.activityType = "EXPIR"
                a.activitySubType = "SELL"
            } else {
                a.activityType = "EXERCISE"
                a.activitySubType = "SELL"
            }
            if raw.contains("ASSIGN") || abs(cash) < 1e-12 {
                a.unitPrice = 0
            }
            if qty > 0 {
                a.quantity = a.activitySubType == "SELL" ? -qty : qty
            }
        }
        a.kind = kindOf(a)
        return a
    }

    static func normalizeActivities(_ activities: [BHAct]) -> [BHAct] {
        activities.map(normalizeActivity)
    }

    /// Net +N/-N name-change rows on one day; leftover +N opens at $0.
    static func foldStkdis(_ activities: [BHAct]) -> [BHAct] {
        var rest: [BHAct] = []
        struct Group { var pos = 0.0, neg = 0.0; var sample: BHAct }
        var groups: [String: Group] = [:]
        var order: [String] = []
        for a in activities {
            if compact(a.activityType) != "STKDIS" {
                rest.append(a)
                continue
            }
            let k = a.symbol + "\u{1}" + a.transactionDate + "\u{1}" + a.currency
            if groups[k] == nil {
                groups[k] = Group(sample: a)
                order.append(k)
            }
            let q = a.quantity
            if a.activitySubType == "SELL" || q < 0 {
                groups[k]!.neg += abs(q)
            } else {
                groups[k]!.pos += abs(q)
            }
        }
        for k in order {
            let g = groups[k]!
            let net = g.pos - g.neg
            if net > eps {
                let a = g.sample.copy()
                a.quantity = net; a.activitySubType = "BUY"; a.unitPrice = 0; a.netCashAmount = 0; a.category = "trade"
                rest.append(a)
            }
        }
        return rest
    }

    /// Wealthsimple posts a share split as a CORPORATE_ACTION with quantity 0
    /// and no ratio. Infer the ratio from the fill prices on either side and
    /// return (account, symbol, date) -> factor.
    static func splitMarkers(_ activities: [BHAct]) -> [String: Double] {
        var out: [String: Double] = [:]
        var byBook: [String: [BHAct]] = [:]
        for a in activities {
            if !(a.category == "trade" || a.category == "option_event") || a.symbol.isEmpty { continue }
            byBook[fifoAccount(a) + "\u{1}" + a.symbol, default: []].append(a)
        }
        for a in activities {
            if compact(a.activityType) != "STKDIS" || compact(a.rawType) != "CORPORATEACTION" { continue }
            if abs(a.quantity) > eps { continue }
            let day = a.transactionDate
            let key = fifoAccount(a) + "\u{1}" + a.symbol
            let priced = (byBook[key] ?? []).filter { $0.unitPrice > 0 && compact($0.activityType) != "STKDIS" }
                .sorted { ($0.transactionDate, $0.occurredAt) < ($1.transactionDate, $1.occurredAt) }
            var before = priced.filter { $0.transactionDate < day }.map { $0.unitPrice }
            var after = priced.filter { $0.transactionDate >= day }.map { $0.unitPrice }
            before = Array(before.suffix(3))
            after = Array(after.prefix(3))
            if before.isEmpty || after.isEmpty { continue }
            before.sort()
            after.sort()
            let pre = before[before.count / 2]
            let post = after[after.count / 2]
            if !(pre > 0) || !(post > 0) { continue }
            let ratio = post / pre
            var n: Int
            var factor: Double
            if ratio >= 1.5 {
                n = Int(ratio.rounded(.toNearestOrEven))
                factor = 1.0 / Double(n)
            } else if ratio <= 1 / 1.5 {
                n = Int((1 / ratio).rounded(.toNearestOrEven))
                factor = Double(n)
            } else {
                continue
            }
            if n < 2 || abs(ratio - (1 / factor)) / (1 / factor) > 0.35 { continue }
            out[fifoAccount(a) + "\u{1}" + a.symbol + "\u{1}" + day] = factor
        }
        return out
    }

    static func fifoAccount(_ a: BHAct) -> String {
        let nick = normAccountName(a.accountType)
        if !nick.isEmpty { return nick }
        if !a.fifoId.isEmpty { return a.fifoId }
        return a.accountId
    }

    static func bookKey(_ a: BHAct) -> String { fifoAccount(a) + "::" + a.symbol + "::" + a.currency }

    struct ReplacementIndex {
        var removed: [String: String] = [:]
        var trades: [String: [String]] = [:]
    }

    static func replacementIndex(_ activities: [BHAct]) -> ReplacementIndex {
        var idx = ReplacementIndex()
        for a in activities {
            let key = fifoAccount(a) + "\u{1}" + a.symbol + "\u{1}" + a.currency
            let t = compact(a.activityType)
            let d = a.transactionDate
            if t == "STKDIS" {
                let sub = compact(a.activitySubType)
                if sub == "SELL" || a.quantity < 0 {
                    if !d.isEmpty && (idx.removed[key] == nil || d < idx.removed[key]!) { idx.removed[key] = d }
                }
                continue
            }
            let raw = compact(a.rawType) + compact(a.aftType)
            if ["CODECHANGE", "SYMBOLCHANGE", "TICKERCHANGE", "LISTINGSTATUS", "SECURITYSWAP"].contains(where: { raw.contains($0) }) {
                if !d.isEmpty && (idx.removed[key] == nil || d < idx.removed[key]!) { idx.removed[key] = d }
            }
            if (a.category == "trade" || a.category == "option_event") && !tradeSide(a).isEmpty {
                idx.trades[key, default: []].append(d)
            }
        }
        return idx
    }

    static func tickerWasReplaced(_ index: ReplacementIndex, account: String, symbol: String, currency: String, byDate: String) -> Bool {
        let key = account + "\u{1}" + symbol + "\u{1}" + currency
        guard let removedOn = index.removed[key], removedOn <= byDate else { return false }
        return !(index.trades[key] ?? []).contains { $0 > removedOn }
    }

    // MARK: option quantity inference (WS multileg rows often carry qty 0)

    final class Fill {
        var a: BHAct
        var side: String
        var qty: Double
        var rollDirection: String?
        var rtBefore: String?
        init(a: BHAct, side: String, qty: Double) { self.a = a; self.side = side; self.qty = qty }
    }

    private static func setFillSide(_ f: Fill, _ side: String, _ sub: String) {
        f.side = side
        f.a.activitySubType = sub
        let q = abs(f.a.quantity)
        if q > 0 { f.a.quantity = side == "SELL" ? -q : q }
    }

    private static func resolveOptionFillSide(_ f: Fill, _ rem: [String: Double]) {
        let a = f.a
        let raw = compact(a.rawType) + compact(a.activityType)
        let expirish = raw.contains("EXPIR") || raw.contains("ASSIGN") || raw.contains("EXERCISE")
        if expirish {
            if raw.contains("ASSIGN") || raw.contains("SHORTEXPIR") {
                setFillSide(f, "BUY", raw.contains("ASSIGN") ? "BUYTOCLOSE" : "BUY")
            } else if raw.contains("EXPIR") && !raw.contains("SHORT") {
                setFillSide(f, "SELL", "SELL")
            } else if f.side == "BUY" && rem["LONG"]! > eps && rem["SHORT"]! <= eps {
                setFillSide(f, "SELL", "SELL")
            } else if f.side == "SELL" && rem["SHORT"]! > eps && rem["LONG"]! <= eps {
                setFillSide(f, "BUY", "BUY")
            }
            return
        }
        if !(raw.contains("MULTILEG") || isCloseOnly(a)) { return }
        if f.side == "BUY" {
            a.activitySubType = rem["SHORT"]! > eps ? "BUYTOCLOSE" : "BUYTOOPEN"
        } else if f.side == "SELL" {
            a.activitySubType = rem["LONG"]! > eps ? "SELLTOCLOSE" : "SELLTOOPEN"
        }
    }

    private static func isCleanOptionQty(_ cash: Double, _ qty: Double) -> Bool {
        if !(qty > 0) { return false }
        let px = abs(cash) / (qty * 100.0)
        if px < 0 { return false }
        if abs(px * 100 - (px * 100).rounded(.toNearestOrEven)) < 1e-6 { return true }
        if abs(px * 10000 - (px * 10000).rounded(.toNearestOrEven)) < 1e-4 { return true }
        return false
    }

    private static func inferStandaloneOptionQty(_ cash: Double) -> Double {
        let absCash = abs(cash)
        if !(absCash > 0) { return 0 }
        let maxQty = min(10000, max(1, Int(absCash.rounded(.toNearestOrEven))))
        for q in 1...maxQty {
            if isCleanOptionQty(absCash, Double(q)) { return Double(q) }
        }
        return 1
    }

    static func inferZeroQtyOptionFills(_ fills: [Fill]) {
        var remaining: [String: [String: Double]] = [:]
        func remOf(_ a: BHAct) -> [String: Double] {
            let k = bookKey(a)
            if remaining[k] == nil { remaining[k] = ["LONG": 0, "SHORT": 0] }
            return remaining[k]!
        }
        var zerosByBook: [String: [Int]] = [:]
        var pools: [String: [String: Double]] = [:]
        func poolOf(_ a: BHAct) -> [String: Double] {
            let k = rollKey(a)
            if pools[k] == nil { pools[k] = ["LONG": 0, "SHORT": 0] }
            return pools[k]!
        }

        for (i, f) in fills.enumerated() {
            let a = f.a
            let qty = abs(a.quantity)
            let cash = a.netCashAmount
            if !isOptionSymbol(a.symbol) || f.side.isEmpty { continue }
            if qty == 0 && abs(cash) > 1e-9 {
                zerosByBook[bookKey(a), default: []].append(i)
            }
        }

        for (i, f) in fills.enumerated() {
            let a = f.a
            if f.side.isEmpty { continue }
            var rem = remOf(a)
            let remKey = bookKey(a)
            if isOptionSymbol(a.symbol) && isMultileg(a) {
                // A roll: this row closes what the contract holds (or what an earlier
                // roll carried forward), and the same quantity moves to the next contract.
                var pool = poolOf(a)
                let poolKey = rollKey(a)
                let direction = rem["SHORT"]! > eps ? "SHORT" : (rem["LONG"]! > eps ? "LONG" : (pool["SHORT"]! >= pool["LONG"]! ? "SHORT" : "LONG"))
                let openSz = rem[direction]! + pool[direction]!
                var qty = abs(a.quantity)
                let cash = a.netCashAmount
                if qty == 0 {
                    let upcoming = (zerosByBook[remKey] ?? []).filter { $0 > i }.count
                    if rem[direction]! > eps && upcoming == 0 {
                        qty = rem[direction]!
                    } else if openSz > eps {
                        var picked = 0.0
                        let cap = max(1, Int(openSz + 1e-9))
                        for q in 1...cap {
                            if isCleanOptionQty(cash, Double(q)) { picked = Double(q); break }
                        }
                        qty = picked > 0 ? picked : inferStandaloneOptionQty(cash)
                        if qty > openSz { qty = openSz }
                    } else {
                        qty = inferStandaloneOptionQty(cash)
                    }
                    a.unitPrice = qty > 0 ? abs(cash) / (qty * 100.0) : 0
                }
                f.side = direction == "SHORT" ? "BUY" : "SELL"
                a.activitySubType = direction == "SHORT" ? "BUYTOCLOSE" : "SELLTOCLOSE"
                a.quantity = direction == "SHORT" ? qty : -qty
                f.qty = qty
                f.rollDirection = direction
                let closed = min(qty, rem[direction]!)
                rem[direction]! -= closed
                pool[direction]! -= min(qty - closed, pool[direction]!)
                if closed > eps || qty > eps {
                    pool[direction]! += qty
                }
                remaining[remKey] = rem
                pools[poolKey] = pool
                continue
            }
            if isOptionSymbol(a.symbol) {
                resolveOptionFillSide(f, rem)
            }
            var qty = abs(a.quantity)
            let cash = a.netCashAmount
            if isOptionSymbol(a.symbol) && qty == 0 {
                let closingDir = f.side == "BUY" ? "SHORT" : "LONG"
                let openSz = rem[closingDir] ?? 0
                if abs(cash) > 1e-9 {
                    let upcoming = (zerosByBook[remKey] ?? []).filter { $0 > i }.count
                    if openSz > 0 && upcoming == 0 {
                        qty = openSz
                    } else if openSz > 0 {
                        var picked = 0.0
                        let cap = max(1, Int(openSz + 1e-9))
                        for q in 1...cap {
                            if isCleanOptionQty(cash, Double(q)) { picked = Double(q); break }
                        }
                        qty = picked > 0 ? picked : inferStandaloneOptionQty(cash)
                        if qty > openSz { qty = openSz }
                    } else {
                        qty = inferStandaloneOptionQty(cash)
                    }
                    a.unitPrice = qty > 0 ? abs(cash) / (qty * 100.0) : 0
                } else if openSz > 0 && isCloseOnly(a) {
                    qty = openSz
                    a.unitPrice = 0
                }
                if qty > 0 {
                    a.quantity = f.side == "SELL" ? -qty : qty
                    f.qty = qty
                }
            }
            if isOptionSymbol(a.symbol) && (compact(a.rawType).contains("ASSIGN") || compact(a.activityType).contains("ASSIGN")) {
                a.unitPrice = 0
            }
            if f.qty > 0 {
                let closingDir = f.side == "BUY" ? "SHORT" : "LONG"
                let opening = openingDirection(a, f.side)
                var left = f.qty
                let closeAmt = min(left, rem[closingDir] ?? 0)
                rem[closingDir]! -= closeAmt
                left -= closeAmt
                if left > eps && isOptionSymbol(a.symbol) {
                    var pool = poolOf(a)
                    let pooled = min(left, pool[closingDir]!)
                    pool[closingDir]! -= pooled
                    left -= pooled
                    pools[rollKey(a)] = pool
                }
                if left > eps, let opening = opening {
                    rem[opening]! += left
                }
            }
            remaining[remKey] = rem
        }
    }

    // MARK: FIFO matching

    static func stableTradeId(accountId: String, symbol: String, currency: String, entryDate: String, exitDate: String, quantity: Double, entryPrice: Double, exitPrice: Double, side: String) -> String {
        [accountId, symbol, currency, entryDate, exitDate, fmt8(quantity), fmt8(entryPrice), fmt8(exitPrice), side].joined(separator: "|")
    }

    static func stableTradeId(_ t: BHSlice) -> String {
        stableTradeId(accountId: t.accountId, symbol: t.symbol, currency: t.currency, entryDate: t.entryDate, exitDate: t.exitDate, quantity: t.quantity, entryPrice: t.entryPrice, exitPrice: t.exitPrice, side: t.side)
    }

    static func sliceMemberKey(_ t: BHSlice) -> String {
        if !t.buyActivityId.isEmpty && !t.sellActivityId.isEmpty {
            return [t.buyActivityId, t.sellActivityId, fmt8(t.quantity)].joined(separator: "|")
        }
        return t.id
    }

    static func fillRank(_ f: Fill) -> Int {
        let a = f.a
        let t = compact(a.activityType)
        let s = compact(a.activitySubType)
        let blob = t + s
        if (blob.contains("TOOPEN") || t == "STO" || s == "STO") && f.side == "SELL" { return 0 }
        if isCloseOnly(a) && f.side == "BUY" { return 1 }
        if f.side == "BUY" { return 2 }
        if blob.contains("TOCLOSE") || t == "STC" || s == "STC" { return 3 }
        return 4
    }

    static func fillSortKey(_ f: Fill) -> (String, Int, String, String) {
        (f.a.transactionDate, fillRank(f), f.a.occurredAt, f.a.id)
    }

    static func makeSlice(_ lot: BHLot, _ fill: Fill, _ a: BHAct, _ matched: Double, symbol: String? = nil) -> BHSlice {
        let fillQty = fill.qty
        let exitCommission = fillQty > 0 ? a.commission * (matched / fillQty) : 0
        let entryCommission = lot.qty > 0 ? lot.commission * (matched / lot.qty) : 0
        let commission = entryCommission + exitCommission
        let sym = symbol ?? lot.symbol
        let mult = optionMultiplier(sym)
        let exitPx = a.unitPrice
        let rawPnl = lot.direction == "LONG" ? (exitPx - lot.price) * matched * mult : (lot.price - exitPx) * matched * mult
        var t = BHSlice()
        t.rt = lot.rt
        t.accountId = lot.accountId
        t.accountType = lot.accountType
        t.account = lot.accountType
        t.symbol = sym
        t.name = symbol != nil ? (a.name.isEmpty ? lot.name : a.name) : lot.name
        t.currency = lot.currency
        t.kind = lot.kind
        t.side = fill.side
        t.quantity = matched
        t.entryPrice = lot.price
        t.exitPrice = exitPx
        t.entryDate = lot.date
        t.exitDate = a.transactionDate
        t.entryWhen = lot.when
        t.exitWhen = a.occurredAt
        t.holdDays = daysBetween(lot.date, a.transactionDate)
        t.commission = commission
        t.entryCommission = entryCommission
        t.exitCommission = exitCommission
        t.pnl = rawPnl - commission
        t.pnlCad = rawPnl - commission
        t.openDirection = lot.direction
        t.buyActivityId = lot.activityId
        t.sellActivityId = a.id
        t.securityId = !lot.securityId.isEmpty ? lot.securityId : a.securityId
        t.flags = Array(Set(lot.flags).union(a.flags)).sorted()
        t.id = stableTradeId(t)
        return t
    }

    /// Sells that exceed the lots by a residue are rounding, not a short.
    static func dust(_ remaining: Double, _ fill: Fill, _ a: BHAct) -> Bool {
        let qty = fill.qty
        let px = abs(a.unitPrice)
        if remaining <= 1e-6 * max(1.0, qty) { return true }
        if (a.kind.isEmpty ? kindOf(a) : a.kind) == "Crypto" && remaining <= 0.01 * qty { return true }
        return px > 0 && remaining * px * optionMultiplier(a.symbol) < 0.01
    }

    final class RollPool {
        var lots: [String: [BHLot]] = ["LONG": [], "SHORT": []]
        var rt: String?
    }

    struct FifoResult {
        var closed: [BHSlice]
        var open: [BHLot]
        var unmatched: [BHUnmatched]
    }

    /// FIFO per (account, symbol, currency). Returns closed slices, open lots, unmatched.
    static func matchFifo(_ input: [BHAct]) -> FifoResult {
        let normalized = input.filter { !$0.flags.contains("pending-distribution") }
        let folded = foldStkdis(normalized)
        var fills: [Fill] = []
        for a in folded {
            if !(a.category == "trade" || a.category == "option_event") || a.symbol.isEmpty { continue }
            let side = tradeSide(a)
            if side.isEmpty { continue }
            fills.append(Fill(a: a, side: side, qty: abs(a.quantity)))
        }
        let keyed = fills.map { ($0, fillSortKey($0)) }.sorted { $0.1 < $1.1 }
        fills = keyed.map { $0.0 }
        inferZeroQtyOptionFills(fills)
        let usable = fills.filter { $0.qty > 0 }

        var books: [String: [BHLot]] = [:]
        var bookOrder: [String] = []
        var rtOpen: [String: String?] = [:]
        var closed: [BHSlice] = []
        var unmatched: [BHUnmatched] = []
        var rolled: [String: RollPool] = [:]
        var rolledOrder: [String] = []
        var rolledKeys = Set<String>()

        func rolledOf(_ a: BHAct) -> RollPool {
            let k = rollKey(a)
            if let p = rolled[k] { return p }
            let p = RollPool()
            rolled[k] = p
            rolledOrder.append(k)
            return p
        }

        func book(_ key: String) -> [BHLot] {
            if books[key] == nil {
                books[key] = []
                bookOrder.append(key)
            }
            return books[key]!
        }

        /// Close carried-forward legs against this fill; they take this contract's
        /// symbol. When this chain has been rolled, a buy-back beyond the known
        /// shorts also closes the chain's older contracts (nearest expiry first).
        func closeRolled(_ fill: Fill, _ a: BHAct, _ remainingIn: Double, _ closingDir: String) -> Double {
            var remaining = remainingIn
            let pool = rolledOf(a)
            let key = bookKey(a)
            var rt = fill.rtBefore ?? pool.rt
            if rt == nil, let first = pool.lots[closingDir]!.first {
                rt = first.rt ?? ("rt:" + first.activityId)
            }
            if rt != nil { pool.rt = rt }
            while remaining > eps && !pool.lots[closingDir]!.isEmpty {
                var lot = pool.lots[closingDir]![0]
                lot.symbol = a.symbol
                lot.rt = rt ?? lot.rt ?? ("rt:" + lot.activityId)
                let matched = min(lot.qty, remaining)
                closed.append(makeSlice(lot, fill, a, matched))
                lot.qty -= matched
                remaining -= matched
                if lot.qty <= eps {
                    pool.lots[closingDir]!.removeFirst()
                } else {
                    pool.lots[closingDir]![0] = lot
                }
            }
            if remaining > eps && rolledKeys.contains(rollKey(a)) {
                var others: [(String, String)] = []
                for k2 in bookOrder {
                    if k2 == key || (books[k2] ?? []).isEmpty { continue }
                    let bits = k2.components(separatedBy: "::")
                    if bits[0] != fifoAccount(a) || bits[2] != a.currency { continue }
                    if !isOptionSymbol(bits[1]) || underlyingSymbol(bits[1]) != underlyingSymbol(a.symbol) || optionRight(bits[1]) != optionRight(a.symbol) { continue }
                    others.append((optionExpiry(bits[1]), k2))
                }
                for (_, k2) in others.sorted(by: { $0 < $1 }) {
                    while remaining > eps && !(books[k2] ?? []).isEmpty && books[k2]![0].direction == closingDir {
                        var lot = books[k2]![0]
                        let matched = min(lot.qty, remaining)
                        var s = makeSlice(lot, fill, a, matched, symbol: a.symbol)
                        s.flags = Array(Set(s.flags).union(["rolled-in"])).sorted()
                        if let rt = rt { s.rt = rt }
                        closed.append(s)
                        lot.qty -= matched
                        remaining -= matched
                        if lot.qty <= eps {
                            books[k2]!.removeFirst()
                        } else {
                            books[k2]![0] = lot
                        }
                    }
                    if (books[k2] ?? []).isEmpty {
                        rtOpen[k2] = .some(nil)
                    }
                }
            }
            if pool.lots["LONG"]!.isEmpty && pool.lots["SHORT"]!.isEmpty && (books[key] ?? []).isEmpty {
                pool.rt = nil
            }
            return remaining
        }

        let replaced = replacementIndex(normalized)
        let splits = splitMarkers(normalized)
        var pendingSplits: [String: [(String, Double)]] = [:]
        for (k, factor) in splits {
            let bits = k.components(separatedBy: "\u{1}")
            pendingSplits[bits[0] + "::" + bits[1], default: []].append((bits[2], factor))
        }

        func applySplits(_ key: String, _ day: String) {
            let skey = key.components(separatedBy: "::").prefix(2).joined(separator: "::")
            guard let todo = pendingSplits[skey] else { return }
            var keep: [(String, Double)] = []
            for (splitDay, factor) in todo.sorted(by: { $0.0 < $1.0 }) {
                if splitDay <= day {
                    var lots = books[key] ?? []
                    for i in lots.indices {
                        lots[i].qty *= factor
                        lots[i].price /= factor
                        let label = "split " + (factor < 1 ? "1:\(Int((1 / factor).rounded(.toNearestOrEven)))" : "\(Int(factor.rounded(.toNearestOrEven))):1")
                        if !lots[i].flags.contains(label) { lots[i].flags.append(label) }
                    }
                    if books[key] != nil { books[key] = lots }
                } else {
                    keep.append((splitDay, factor))
                }
            }
            if keep.isEmpty {
                pendingSplits[skey] = nil
            } else {
                pendingSplits[skey] = keep
            }
        }

        func closeAgainst(_ key: String, _ fill: Fill, _ a: BHAct, _ remainingIn: Double, symbolOverride: String? = nil) -> Double {
            var remaining = remainingIn
            let closingDir = fill.side == "BUY" ? "SHORT" : "LONG"
            while remaining > eps && !(books[key] ?? []).isEmpty && books[key]![0].direction == closingDir {
                var lot = books[key]![0]
                let matched = min(lot.qty, remaining)
                closed.append(makeSlice(lot, fill, a, matched, symbol: symbolOverride))
                lot.commission *= lot.qty > 0 ? (lot.qty - matched) / lot.qty : 0
                lot.qty -= matched
                remaining -= matched
                if lot.qty <= eps {
                    books[key]!.removeFirst()
                } else {
                    books[key]![0] = lot
                }
            }
            if (books[key] ?? []).isEmpty {
                rtOpen[key] = .some(nil)
            }
            return remaining
        }

        func openRt(_ key: String) -> String? {
            if let v = rtOpen[key] { return v }
            return nil
        }

        for fill in usable {
            let a = fill.a
            let key = bookKey(a)
            _ = book(key)
            applySplits(key, a.transactionDate)
            if isOptionSymbol(a.symbol) && isMultileg(a), let direction = fill.rollDirection {
                // Roll: close this contract (book, then carried-forward legs) and carry
                // the same quantity to the unposted new leg. A debit belongs to the
                // closed leg's exit, a credit to the new leg's entry.
                let cash = a.netCashAmount
                let per = fill.qty > 0 ? abs(cash) / (fill.qty * 100.0) : 0
                let debit = cash < 0
                let exitPx = (direction == "SHORT") == debit ? per : 0
                let entryPx = (direction == "SHORT") != debit ? per : 0
                a.unitPrice = exitPx
                let before = closed.count
                fill.rtBefore = books[key]!.isEmpty ? nil : openRt(key)
                var remaining = closeAgainst(key, fill, a, fill.qty)
                remaining = closeRolled(fill, a, remaining, direction)
                let moved = fill.qty - remaining
                rolledKeys.insert(rollKey(a))
                if moved > eps {
                    for i in before..<closed.count where !closed[i].flags.contains("rolled") {
                        closed[i].flags.append("rolled")
                    }
                    let chainRt = fill.rtBefore ?? rolledOf(a).rt ?? (closed.count > before ? closed[before].rt : nil)
                    rolledOf(a).rt = chainRt
                    rolledOf(a).lots[direction]!.append(BHLot(
                        qty: moved, price: entryPx, date: a.transactionDate, when: a.occurredAt, commission: 0, direction: direction,
                        accountId: a.accountId, accountType: fifoAccount(a), symbol: a.symbol, name: a.name, currency: a.currency,
                        kind: "Options", activityId: a.id, securityId: "", rt: chainRt, flags: ["rolled-in"]))
                }
                if remaining > eps {
                    // nothing to roll: this multileg simply opened a position
                    let opening = debit ? "LONG" : "SHORT"
                    a.unitPrice = per
                    fill.side = opening == "LONG" ? "BUY" : "SELL"
                    if books[key]!.isEmpty || openRt(key) == nil {
                        rtOpen[key] = "rt:" + a.id
                    }
                    books[key]!.append(BHLot(
                        qty: remaining, price: per, date: a.transactionDate, when: a.occurredAt, commission: 0, direction: opening,
                        accountId: a.accountId, accountType: fifoAccount(a), symbol: a.symbol, name: a.name, currency: a.currency,
                        kind: "Options", activityId: a.id, securityId: a.securityId, rt: openRt(key), flags: a.flags))
                }
                continue
            }
            if a.flags.contains("transfer-out") {
                // coins sent out of the account leave at cost: off the open lots
                // first-in first-out, no slice, no P&L, not a fill of the trade
                var remaining = fill.qty
                while remaining > eps && !(books[key] ?? []).isEmpty && books[key]![0].direction == "LONG" {
                    var lot = books[key]![0]
                    let matched = min(lot.qty, remaining)
                    lot.commission *= lot.qty > 0 ? (lot.qty - matched) / lot.qty : 0
                    lot.qty -= matched
                    remaining -= matched
                    if lot.qty <= eps {
                        books[key]!.removeFirst()
                    } else {
                        books[key]![0] = lot
                    }
                }
                if (books[key] ?? []).isEmpty {
                    rtOpen[key] = .some(nil)
                }
                continue
            }
            fill.rtBefore = books[key]!.isEmpty ? nil : openRt(key)
            var remaining = closeAgainst(key, fill, a, fill.qty)
            if remaining > eps && isOptionSymbol(a.symbol) {
                remaining = closeRolled(fill, a, remaining, fill.side == "BUY" ? "SHORT" : "LONG")
            }
            if remaining > eps && fill.side == "SELL" {
                for dk in bookOrder {
                    if (books[dk] ?? []).isEmpty || dk == key { continue }
                    let bits = dk.components(separatedBy: "::")
                    if bits[0] != fifoAccount(a) || bits[2] != a.currency { continue }
                    if !tickerWasReplaced(replaced, account: bits[0], symbol: bits[1], currency: bits[2], byDate: a.transactionDate) { continue }
                    remaining = closeAgainst(dk, fill, a, remaining, symbolOverride: a.symbol)
                    if remaining <= eps { break }
                }
            }
            if remaining > eps && fill.side == "SELL" && openingDirection(a, fill.side) == nil && dust(remaining, fill, a) {
                remaining = 0
            }
            if remaining > eps {
                if let opening = openingDirection(a, fill.side) {
                    if books[key]!.isEmpty || openRt(key) == nil {
                        rtOpen[key] = "rt:" + a.id
                    }
                    books[key]!.append(BHLot(
                        qty: remaining, price: a.unitPrice, date: a.transactionDate, when: a.occurredAt,
                        commission: fill.qty > 0 ? a.commission * (remaining / fill.qty) : 0, direction: opening,
                        accountId: a.accountId, accountType: fifoAccount(a), symbol: a.symbol, name: a.name, currency: a.currency,
                        kind: a.kind.isEmpty ? kindOf(a) : a.kind, activityId: a.id, securityId: a.securityId, rt: openRt(key), flags: a.flags))
                } else {
                    unmatched.append(BHUnmatched(symbol: a.symbol, currency: a.currency, side: fill.side, quantity: remaining, price: a.unitPrice,
                                                 date: a.transactionDate, description: a.description, accountId: a.accountId,
                                                 account: fifoAccount(a), activityId: a.id))
                }
            }
        }

        for key in bookOrder {
            applySplits(key, "9999-12-31")
        }
        for k in rolledOrder {
            let pool = rolled[k]!
            for direction in ["LONG", "SHORT"] {
                for var lot in pool.lots[direction]! {
                    if lot.qty <= eps { continue }
                    // the closing leg of this roll was never posted; the credit (or
                    // nothing, for a debit roll) is what it earned
                    let pa = BHAct()
                    pa.id = "roll-out:" + lot.activityId
                    pa.unitPrice = 0
                    pa.commission = 0
                    pa.transactionDate = lot.date
                    pa.occurredAt = lot.when
                    pa.name = lot.name
                    pa.flags = ["rolled-out"]
                    let pseudo = Fill(a: pa, side: direction == "SHORT" ? "BUY" : "SELL", qty: lot.qty)
                    lot.rt = lot.rt ?? ("rt:" + lot.activityId)
                    var s = makeSlice(lot, pseudo, pa, lot.qty)
                    s.sellActivityId = ""
                    closed.append(s)
                }
            }
        }
        var openLots: [BHLot] = []
        for key in bookOrder {
            for lot in books[key] ?? [] {
                if lot.qty <= 1e-6 { continue }
                // crypto residue from in-kind fees: a lot worth under a dollar is not a position
                if lot.kind == "Crypto" && lot.qty * lot.price < 1.0 { continue }
                openLots.append(lot)
            }
        }
        closed.sort { ($0.exitDate, $0.id) < ($1.exitDate, $1.id) }
        foldOptionRolls(&closed, &openLots)
        return FifoResult(closed: closed, open: openLots, unmatched: unmatched)
    }

    /// Same-day cover + new short on the same underlying is a roll: fold the
    /// cover's P&L into the far contract's basis and drop the cover row.
    static func foldOptionRolls(_ closed: inout [BHSlice], _ openLots: inout [BHLot]) {
        if closed.isEmpty { return }
        func rollBook(_ t: BHSlice) -> String {
            [t.account.isEmpty ? t.accountType : t.account, t.currency, underlyingSymbol(t.symbol)].joined(separator: "::")
        }
        func dayOf(_ s: String) -> String { String(s.prefix(10)) }

        // the covers are visited in an order fixed now, but each is read as it is
        // when its turn comes: a fold into a row that is itself a later cover
        // changes that cover's basis, P&L and id (as the Python's shared rows do)
        let shortOption = closed.map { $0.openDirection == "SHORT" && isOptionSymbol($0.symbol) }
        let books = closed.map(rollBook)
        var coverIdx = closed.indices.filter { shortOption[$0] }
        coverIdx.sort { (dayOf(closed[$0].entryDate), dayOf(closed[$0].exitDate), closed[$0].id) < (dayOf(closed[$1].entryDate), dayOf(closed[$1].exitDate), closed[$1].id) }
        // the short option slices by (book, entry day): the candidates a cover can fold into
        var byBookDay: [String: [Int]] = [:]
        for i in coverIdx { byBookDay[books[i] + "@" + dayOf(closed[i].entryDate), default: []].append(i) }
        var drop = Set<String>()
        for ci in coverIdx {
            let cover = closed[ci]
            if drop.contains(cover.id) { continue }
            let d = dayOf(cover.exitDate)
            if d.isEmpty { continue }
            let under = underlyingSymbol(cover.symbol)
            if under.isEmpty || under == "—" { continue }
            let ck = books[ci]
            let closedCands = (byBookDay[ck + "@" + d] ?? []).filter { i in
                let t = closed[i]
                return t.id != cover.id && !drop.contains(t.id) && t.symbol != cover.symbol
            }
            let openCands = openLots.indices.filter { i in
                let l = openLots[i]
                return l.direction == "SHORT" && isOptionSymbol(l.symbol) && l.symbol != cover.symbol
                    && [l.accountType, l.currency, underlyingSymbol(l.symbol)].joined(separator: "::") == ck && dayOf(l.date) == d
            }
            let cq = abs(cover.quantity)
            if !closedCands.isEmpty {
                let sorted = closedCands.sorted { a, b in
                    (abs(abs(closed[a].quantity) - cq), closed[a].symbol) < (abs(abs(closed[b].quantity) - cq), closed[b].symbol)
                }
                let i = sorted[0]
                let qty = abs(closed[i].quantity)
                if !(qty > 0) { continue }
                let adj = cover.pnl / (qty * optionMultiplier(closed[i].symbol))
                closed[i].entryPrice += adj
                let mult = optionMultiplier(closed[i].symbol)
                let raw = (closed[i].openDirection == "SHORT" ? (closed[i].entryPrice - closed[i].exitPrice) : (closed[i].exitPrice - closed[i].entryPrice)) * qty * mult
                closed[i].pnl = raw - closed[i].commission
                closed[i].pnlCad = closed[i].pnl
                closed[i].id = stableTradeId(closed[i])
                if !closed[i].flags.contains("rolled") { closed[i].flags.append("rolled") }
            } else if !openCands.isEmpty {
                let sorted = openCands.sorted { a, b in
                    (abs(abs(openLots[a].qty) - cq), openLots[a].symbol) < (abs(abs(openLots[b].qty) - cq), openLots[b].symbol)
                }
                let i = sorted[0]
                let qty = abs(openLots[i].qty)
                if !(qty > 0) { continue }
                let adj = cover.pnl / (qty * optionMultiplier(openLots[i].symbol))
                openLots[i].price += adj
                if !openLots[i].flags.contains("rolled") { openLots[i].flags.append("rolled") }
            } else {
                continue
            }
            drop.insert(cover.id)
        }
        if !drop.isEmpty {
            closed = closed.filter { !drop.contains($0.id) }
        }
    }

    /// 'LUNR 29AUG25 11.50 CALL' -> '2025-08-29'.
    static func optionExpiry(_ symbol: String) -> String {
        let u = spaced(symbol)
        guard let m = re("^\\S+ (\\d{2})([A-Z]{3})(\\d{2}) ").firstMatch(in: u, range: NSRange(u.startIndex..., in: u)) else { return "" }
        let day = String(u[Range(m.range(at: 1), in: u)!])
        let mon = String(u[Range(m.range(at: 2), in: u)!])
        let yr = String(u[Range(m.range(at: 3), in: u)!])
        guard let month = months.firstIndex(of: mon.capitalized) else { return "" }
        return String(format: "20%@-%02d-%@", yr, month + 1, day)
    }

    /// An assigned short option delivers shares, but Wealthsimple posts only
    /// the option row (with the strike cash on it). Add the share leg.
    static func synthesizeAssignmentShares(_ activities: [BHAct], _ securities: Securities) -> [BHAct] {
        var out: [BHAct] = []
        for a in activities {
            if a.category != "option_event" || compact(a.activityType) != "ASSIGN" { continue }
            let symbol = a.symbol
            if !isOptionSymbol(symbol) { continue }
            let contracts = abs(a.quantity)
            if contracts <= 0 { continue }
            let shares = contracts * 100
            let cash = a.netCashAmount
            var strike = abs(cash) > eps ? abs(cash) / shares : 0
            if strike <= 0 {
                if let m = reGroup(" (\\d+(?:\\.\\d+)?) (CALL|PUT)$", spaced(symbol)) { strike = Double(m) ?? 0 }
            }
            if strike <= 0 { continue }
            let up = symbol.uppercased().replacingOccurrences(of: "\\s+$", with: "", options: .regularExpression)
            let isCall = up.hasSuffix("CALL") || up.hasSuffix(" C")
            let sell = abs(cash) <= eps ? isCall : cash > 0
            let under = underlyingSymbol(symbol)
            let sec = securities.byId[a.securityId]
            let underId = sec?.underlyingId ?? ""
            let n = BHAct()
            n.id = "assign-shares:" + a.id
            n.occurredAt = a.occurredAt.isEmpty ? a.transactionDate + "T21:30:00+00:00" : a.occurredAt
            n.transactionDate = a.transactionDate
            n.accountId = a.accountId
            n.fifoId = a.fifoId.isEmpty ? a.accountId : a.fifoId
            n.accountType = a.accountType
            n.activityType = "Trade"
            n.activitySubType = sell ? "SELL" : "BUY"
            n.description = (sell ? "Called away" : "Put to you") + ": \(shares) \(under) @ \(strike)"
            n.direction = sell ? "CREDIT" : "DEBIT"
            n.symbol = under
            n.name = under
            n.currency = a.currency
            n.quantity = sell ? -shares : shares
            n.unitPrice = strike
            n.commission = 0
            n.netCashAmount = sell ? shares * strike : -shares * strike
            n.category = "trade"
            n.rawType = "OPTIONS_ASSIGN_SHARES"
            n.securityId = underId
            n.kind = "Shares"
            n.flags = ["assignment"]
            out.append(n)
        }
        return out
    }

    /// Wealthsimple does not always post an expiry row. An option lot still
    /// open after its expiry date is closed at $0 on that date.
    static func synthesizeExpiries(_ openLots: [BHLot], today: String) -> [BHAct] {
        var out: [BHAct] = []
        var seen = Set<String>()
        for lot in openLots {
            let exp = optionExpiry(lot.symbol)
            if exp.isEmpty || exp >= today { continue }
            let key = lot.accountType + "\u{1}" + lot.symbol + "\u{1}" + lot.currency
            if seen.contains(key) { continue }
            seen.insert(key)
            let qty = openLots.filter { $0.accountType + "\u{1}" + $0.symbol + "\u{1}" + $0.currency == key && $0.direction == lot.direction }
                .reduce(0.0) { $0 + $1.qty }
            if qty <= eps { continue }
            let short = lot.direction == "SHORT"
            let n = BHAct()
            n.id = "expiry:\(lot.accountType)|\(lot.symbol)|\(lot.currency)"
            n.occurredAt = exp + "T21:30:00+00:00"
            n.transactionDate = exp
            n.accountId = lot.accountId
            n.fifoId = lot.accountId
            n.accountType = lot.accountType
            n.activityType = "EXPIR"
            n.activitySubType = short ? "BUY" : "SELL"
            n.description = "Expired (assumed): " + lot.symbol
            n.symbol = lot.symbol
            n.name = lot.name
            n.currency = lot.currency
            n.quantity = short ? qty : -qty
            n.unitPrice = 0
            n.commission = 0
            n.netCashAmount = 0
            n.category = "option_event"
            n.rawType = short ? "OPTIONS_SHORT_EXPIRY" : "OPTIONS_EXPIRY"
            n.securityId = lot.securityId
            n.kind = "Options"
            n.flags = ["assumed-expiry"]
            out.append(n)
        }
        return out
    }

    // MARK: FX

    static func rateOn(_ fx: [String: Double], _ day: String) -> Double {
        var d = String(day.prefix(10))
        if d.isEmpty { return fxFallback }
        for _ in 0..<12 {
            if let r = fx[d], r > 0 { return r }
            d = shiftDate(d, -1)
        }
        return fxFallback
    }

    static func toCad(_ fx: [String: Double], _ amount: Double, _ currency: String, _ day: String) -> Double {
        let ccy = (currency.isEmpty ? "CAD" : currency).uppercased()
        if ccy != "USD" { return amount }
        return amount * rateOn(fx, day)
    }

    static func applyFx(_ slices: inout [BHSlice], _ fx: [String: Double]) {
        for i in slices.indices {
            let t = slices[i]
            let ccy = (t.currency.isEmpty ? "CAD" : t.currency).uppercased()
            if ccy != "USD" {
                slices[i].pnlCad = t.pnl
                slices[i].feesCad = t.commission
                continue
            }
            let qty = t.quantity
            let mult = optionMultiplier(t.symbol)
            let entryC = t.entryCommission
            let exitC = t.exitCommission
            let entryNotional = t.entryPrice * qty * mult
            let exitNotional = t.exitPrice * qty * mult
            let pnlCad: Double
            if t.openDirection == "SHORT" {
                pnlCad = toCad(fx, entryNotional - entryC, ccy, t.entryDate) - toCad(fx, exitNotional + exitC, ccy, t.exitDate)
            } else {
                pnlCad = toCad(fx, exitNotional - exitC, ccy, t.exitDate) - toCad(fx, entryNotional + entryC, ccy, t.entryDate)
            }
            slices[i].pnlCad = pnlCad
            slices[i].feesCad = toCad(fx, entryC, ccy, t.entryDate) + toCad(fx, exitC, ccy, t.exitDate)
        }
    }

    // MARK: securities / exchange labels

    static let exchAlias: [String: String] = [
        "TSXV": "TSX-V", "TSX-V": "TSX-V", "TSX VENTURE": "TSX-V", "CDNX": "TSX-V", "VENTURE": "TSX-V",
        "TORONTO": "TSX", "TSX": "TSX", "CBOE CANADA": "Cboe Canada", "CBOE CA": "Cboe Canada", "NEO": "Cboe Canada",
    ]
    static let micMap: [String: String] = [
        "XTSV": "TSX-V", "XTSX": "TSX", "XNAS": "NASDAQ", "XNYS": "NYSE", "XASE": "NYSE American",
        "ARCX": "NYSE Arca", "XCNQ": "CSE", "NEOE": "Cboe Canada",
    ]

    static func exchangeLabel(_ sec: BHSecurity?) -> String {
        let raw = (sec?.primaryExchange ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        let up = raw.uppercased()
        if let a = exchAlias[up] { return a }
        if !raw.isEmpty { return raw }
        return micMap[(sec?.primaryMic ?? "").uppercased()] ?? ""
    }

    static func listingTicker(_ sym: String) -> String {
        let s = sym.trimmingCharacters(in: .whitespacesAndNewlines)
        if let m = re("^(.+)\\.(TO|V|CN|NE)$").firstMatch(in: s, range: NSRange(s.startIndex..., in: s)), let r = Range(m.range(at: 1), in: s) {
            return String(s[r])
        }
        return s
    }

    static func isAlphaVenue(_ sec: BHSecurity?) -> Bool {
        let exch = (sec?.primaryExchange ?? "").uppercased()
        let mic = (sec?.primaryMic ?? "").uppercased()
        return exch == "ALPHA EXCHANGE" || exch == "ALPHA" || mic == "XATS"
    }

    struct Securities {
        var byId: [String: BHSecurity] = [:]
        var order: [String] = []

        init(_ rows: [BHSecurity]) {
            for r in rows where !r.id.isEmpty {
                if byId[r.id] == nil { order.append(r.id) }
                byId[r.id] = r
            }
        }

        /// Security id -> currency for the cash rows Wealthsimple lists as securities (CAD, USD).
        func cashCurrencies() -> [String: String] {
            var out: [String: String] = [:]
            for (sid, sec) in byId {
                let sym = sec.symbol.uppercased()
                if sym == "CAD" || sym == "USD" || sid.hasPrefix("sec-c-") {
                    out[sid] = sec.currency.uppercased().isEmpty ? sym : sec.currency.uppercased()
                }
            }
            return out
        }

        func preferred(_ sec: BHSecurity?) -> BHSecurity? {
            guard let sec = sec, isAlphaVenue(sec) else { return sec }
            let sym = listingTicker(sec.symbol)
            let ccy = sec.currency
            if sym.isEmpty { return sec }
            for id in order {
                let other = byId[id]!
                if other.id == sec.id || !other.underlyingId.isEmpty { continue }
                if listingTicker(other.symbol) != sym { continue }
                if !ccy.isEmpty && !other.currency.isEmpty && other.currency != ccy { continue }
                if isAlphaVenue(other) || exchangeLabel(other).isEmpty { continue }
                return other
            }
            return sec
        }

        func listing(_ securityId: String) -> BHSecurity? {
            var sec = byId[securityId]
            if let s = sec, !s.underlyingId.isEmpty, let under = byId[s.underlyingId] { sec = under }
            return preferred(sec)
        }

        func exchange(_ securityId: String) -> String { exchangeLabel(listing(securityId)) }

        func name(_ securityId: String, _ fallback: String = "") -> String {
            let n = listing(securityId)?.name ?? ""
            return n.isEmpty ? fallback : n
        }
    }

    // MARK: trades (round trips), positions, cashflow

    static func fillRow(_ a: BHAct) -> BHFillRow {
        let (day, clock) = whenParts(a.occurredAt.isEmpty ? a.transactionDate : a.occurredAt)
        let side = tradeSide(a)
        let qty = abs(a.quantity)
        var f = BHFillRow()
        f.id = a.id
        f.when = a.occurredAt.isEmpty ? a.transactionDate : a.occurredAt
        f.date = day.isEmpty ? a.transactionDate : day
        f.time = clock
        f.side = side
        f.sub = a.activitySubType
        f.qty = side == "SELL" ? -qty : qty
        f.price = a.unitPrice
        f.amount = a.netCashAmount
        f.fees = a.commission
        f.currency = a.currency
        f.flags = a.flags
        return f
    }

    static func collapseTrade(_ gid: String, _ members: [BHSlice], status: String, actsById: [String: BHAct], securities: Securities, journal: [String: BHJournalEntry] = [:]) -> BHTrade {
        let slices = members.sorted { ($0.exitDate, $0.entryDate, sliceMemberKey($0)) < ($1.exitDate, $1.entryDate, sliceMemberKey($1)) }
        let t0 = slices[0]
        let qty = slices.reduce(0.0) { $0 + $1.quantity }
        let entryNotional = slices.reduce(0.0) { $0 + $1.entryPrice * $1.quantity }
        let exitNotional = slices.reduce(0.0) { $0 + $1.exitPrice * $1.quantity }
        let pnl = slices.reduce(0.0) { $0 + $1.pnl }
        let pnlCad = slices.reduce(0.0) { $0 + $1.pnlCad }
        let fees = slices.reduce(0.0) { $0 + $1.commission }
        let feesCad = slices.reduce(0.0) { $0 + $1.feesCad }
        let entryDate = slices.map { $0.entryDate }.min()!
        let exitDate = slices.map { $0.exitDate }.max()!
        let entryWhen = slices.map { $0.entryWhen.isEmpty ? $0.entryDate : $0.entryWhen }.min()!
        let exitWhen = slices.map { $0.exitWhen.isEmpty ? $0.exitDate : $0.exitWhen }.max()!
        let mult = optionMultiplier(t0.symbol)
        let entry = qty != 0 ? entryNotional / qty : 0
        let exitPx = qty != 0 ? exitNotional / qty : t0.exitPrice
        let basis = abs(entry * qty * mult)
        let secId = slices.first { !$0.securityId.isEmpty }?.securityId ?? ""
        var ids: [String] = []
        for s in slices {
            for k in [s.buyActivityId, s.sellActivityId] where !k.isEmpty && !ids.contains(k) { ids.append(k) }
        }
        var fills = ids.compactMap { actsById[$0] }.map(fillRow)
        // label each fill by what it did in this trade, not by the broker's order
        // type: the open/close order types are option language, shares and crypto
        // fills are simply bought or sold
        let openedIds = Set(slices.map { $0.buyActivityId })
        let closedIds = Set(slices.map { $0.sellActivityId })
        for i in fills.indices {
            let opened = openedIds.contains(fills[i].id), closed = closedIds.contains(fills[i].id)
            let side = fills[i].side == "BUY" ? "BUY" : "SELL"
            if t0.kind != "Options" {
                fills[i].sub = side + (opened && closed ? " (close + open)" : "")
            } else if closed && !opened {
                fills[i].sub = side + " TO CLOSE"
            } else if opened && !closed {
                fills[i].sub = side + " TO OPEN"
            } else if opened && closed {
                fills[i].sub = side + " (close + open)"
            }
        }
        fills.sort { $0.when > $1.when }
        var t = BHTrade()
        t.id = gid
        t.status = status
        t.symbol = t0.symbol
        t.underlying = underlyingSymbol(t0.symbol)
        t.name = securities.name(secId, t0.name.isEmpty ? t0.symbol : t0.name)
        t.exchange = t0.kind != "Crypto" ? securities.exchange(secId) : "Crypto"
        t.kind = t0.kind
        t.currency = t0.currency
        t.account = t0.account
        t.accountId = t0.accountId
        t.securityId = secId
        t.side = t0.openDirection == "LONG" ? "SELL" : "COVER"
        t.openDirection = t0.openDirection
        t.qty = qty
        t.mult = mult
        t.entry = entry
        t.exit = exitPx
        t.entryDate = entryDate
        t.exitDate = exitDate
        t.entryWhen = entryWhen
        t.exitWhen = exitWhen
        t.holdDays = daysBetween(entryDate, exitDate)
        t.pnl = pnl
        t.pnlCad = pnlCad
        t.fees = fees
        t.feesCad = feesCad
        t.pnlPct = basis > 0 ? pnl / basis : nil
        t.legCount = slices.count
        t.fills = fills
        t.flags = Array(Set(slices.flatMap { $0.flags })).sorted()
        let note = journal[gid]
        t.grade = note?.grade ?? ""
        t.thesis = note?.thesis ?? ""
        t.tags = note?.tags ?? []
        return t
    }

    static func buildTrades(_ closed: [BHSlice], actsById: [String: BHAct], securities: Securities, journal: [String: BHJournalEntry] = [:]) -> [BHTrade] {
        var byRt: [String: [BHSlice]] = [:]
        var order: [String] = []
        for s in closed {
            let rt = s.rt ?? ("rt:" + sliceMemberKey(s))
            if byRt[rt] == nil {
                byRt[rt] = []
                order.append(rt)
            }
            byRt[rt]!.append(s)
        }
        var trades = order.map { collapseTrade($0, byRt[$0]!, status: "closed", actsById: actsById, securities: securities, journal: journal) }
        trades.sort { ($0.exitDate, $0.id) > ($1.exitDate, $1.id) }
        return trades
    }

    /// symbol -> (price, date) from the newest fill with a price.
    static func lastFillPrices(_ activities: [BHAct]) -> [String: (price: Double, date: String)] {
        var out: [String: (price: Double, date: String)] = [:]
        for a in activities.sorted(by: { ($0.transactionDate, $0.occurredAt) < ($1.transactionDate, $1.occurredAt) }) {
            if !(a.category == "trade" || a.category == "option_event") { continue }
            if a.unitPrice > 0 && !a.symbol.isEmpty {
                out[a.symbol] = (a.unitPrice, a.transactionDate)
            }
        }
        return out
    }

    static func buildPositions(_ openLots: [BHLot], lastPrices: [String: (price: Double, date: String)], securities: Securities, today: String, quotes: [String: BHQuote], journal: [String: BHJournalEntry] = [:], actsById: [String: BHAct] = [:]) -> [BHPosition] {
        var groups: [String: [BHLot]] = [:]
        var order: [String] = []
        for lot in openLots {
            let k = [lot.symbol, lot.accountType, lot.currency, lot.direction].joined(separator: "\u{1}")
            if groups[k] == nil {
                groups[k] = []
                order.append(k)
            }
            groups[k]!.append(lot)
        }
        var rows: [BHPosition] = []
        for k in order {
            let lots = groups[k]!.sorted { ($0.date, $0.when) < ($1.date, $1.when) }
            let bits = k.components(separatedBy: "\u{1}")
            let symbol = bits[0], account = bits[1], currency = bits[2], direction = bits[3]
            let mult = optionMultiplier(symbol)
            let qty = lots.reduce(0.0) { $0 + $1.qty }
            if qty <= 1e-9 { continue }
            let cost = lots.reduce(0.0) { $0 + $1.qty * $1.price * mult }
            let fees = lots.reduce(0.0) { $0 + $1.commission }
            let secId = lots.first { !$0.securityId.isEmpty }?.securityId ?? ""
            let last = lastPrices[symbol]
            var lastPx = last?.price ?? (qty != 0 ? cost / (qty * mult) : 0)
            var lastAt = last?.date ?? ""
            var priceSource = "fill"
            let quote = bySymbol(quotes, symbol)
            if let q = quote, let px = q.price, px != 0 {
                lastPx = px
                lastAt = q.fetchedAt
                priceSource = "quote"
            }
            let mv = qty * lastPx * mult
            let unreal = direction == "LONG" ? mv - cost : cost - mv
            let held = lots.reduce(0.0) { $0 + $1.qty * Double(daysBetween($1.date, today)) }
            let legacyPid = "pos:" + [account, symbol, currency].joined(separator: "|")
            var p = BHPosition()
            p.id = lots[0].rt ?? legacyPid
            p.symbol = symbol
            p.underlying = underlyingSymbol(symbol)
            p.name = securities.name(secId, lots[0].name.isEmpty ? symbol : lots[0].name)
            p.exchange = lots[0].kind != "Crypto" ? securities.exchange(secId) : "Crypto"
            p.kind = lots[0].kind
            p.account = account
            p.accountId = lots[0].accountId
            p.currency = currency
            p.securityId = secId
            p.short = direction == "SHORT"
            p.qty = qty
            p.mult = mult
            p.avg = qty != 0 ? cost / (qty * mult) : 0
            p.cost = cost
            p.fees = fees
            p.last = lastPx
            p.lastAt = lastAt
            p.priceSource = priceSource
            p.priceChange = quote?.priceChange
            p.percentChange = quote?.percentChange
            // the day's move on the whole position, in its own currency, from the quote's change
            p.dayChange = quote?.priceChange.map { qty * $0 * mult * (direction == "SHORT" ? -1 : 1) }
            p.mv = mv
            p.unreal = unreal
            p.unrealPct = cost != 0 ? unreal / cost : nil
            p.held = qty != 0 ? Int((held / qty).rounded(.toNearestOrEven)) : 0
            p.opened = lots[0].date
            p.rt = lots[0].rt
            p.lots = lots.map { l in
                BHPositionLot(opened: l.date, qty: l.qty, price: l.price, basis: l.qty * l.price * mult, held: daysBetween(l.date, today), flags: l.flags, activityId: l.activityId)
            }
            // A position and the trade it becomes when it closes share one journal
            // entry: both are keyed by the round trip that opened the position.
            let note = journal[p.id] ?? journal[legacyPid]
            p.grade = note?.grade ?? ""
            p.thesis = note?.thesis ?? ""
            p.tags = note?.tags ?? []
            p.fills = lots.compactMap { actsById[$0.activityId] }.map(fillRow).sorted { $0.when > $1.when }
            rows.append(p)
        }
        let book = rows.reduce(0.0) { $0 + abs($1.cost) }
        for i in rows.indices {
            rows[i].alloc = book != 0 ? abs(rows[i].cost) / book : 0
        }
        rows.sort { $0.alloc > $1.alloc }
        return rows
    }

    static func buildCashflow(_ activities: [BHAct], securities: Securities, fx: [String: Double]) -> [BHCashRow] {
        var rows: [BHCashRow] = []
        for a in activities {
            let cat = a.category
            let raw = compact(a.rawType)
            let at = compact(a.activityType)
            let cash = a.netCashAmount
            var kind = ""
            if cat == "dividend" {
                kind = "Dividend"
            } else if cat == "interest" {
                kind = "Interest"
            } else if raw == "WITHHOLDINGTAX" || at == "WITHHOLDINGTAX" {
                kind = "Withholding tax"
            } else if raw == "INTERESTCHARGE" || at == "INTERESTCHARGE" {
                kind = "Interest charge"
            } else {
                continue
            }
            if abs(cash) < eps { continue }
            let (day, clock) = whenParts(a.occurredAt.isEmpty ? a.transactionDate : a.occurredAt)
            var symbol = a.symbol.trimmingCharacters(in: .whitespacesAndNewlines)
            if symbol.isEmpty && (kind == "Interest" || kind == "Interest charge") { symbol = "Cash" }
            var r = BHCashRow()
            r.id = a.id
            r.date = a.transactionDate.isEmpty ? day : a.transactionDate
            r.time = clock
            r.symbol = symbol.isEmpty ? "—" : symbol
            r.name = securities.name(a.securityId, a.name != symbol ? a.name : "")
            r.kind = kind
            r.account = normAccountName(a.accountType).isEmpty ? a.accountId : normAccountName(a.accountType)
            r.accountId = a.accountId
            r.qty = a.quantity != 0 ? a.quantity : nil
            r.per = a.unitPrice != 0 ? a.unitPrice : nil
            r.amount = cash
            r.currency = a.currency.isEmpty ? "CAD" : a.currency
            r.amountCad = toCad(fx, cash, a.currency, a.transactionDate)
            rows.append(r)
        }
        rows.sort { ($0.date, $0.id) > ($1.date, $1.id) }
        return rows
    }

    // MARK: build

    static func buildBase(activities raw: [BHAct], securities secRows: [BHSecurity], market: BHMarket, today: String, navHistory: [BHNavPoint] = [], navByAccount: [String: [BHNavPoint]] = [:], journal: [String: BHJournalEntry] = [:], accounts: [BHAccountInfo] = [], balances: [BHBalanceRow] = [], margin: [BHMarginRow] = []) -> BHBase {
        var acts = normalizeActivities(raw)
        let securities = Securities(secRows)
        let delivered = synthesizeAssignmentShares(acts, securities)
        if !delivered.isEmpty { acts += delivered }
        var fifo = matchFifo(acts)
        let synthetic = synthesizeExpiries(fifo.open, today: today)
        if !synthetic.isEmpty {
            acts += synthetic
            fifo = matchFifo(acts)
        }
        var actsById: [String: BHAct] = [:]
        for a in acts { actsById[a.id] = a }
        applyFx(&fifo.closed, market.fx)
        var base = BHBase()
        base.today = today
        base.fx = market.fx
        base.benchmark = market.benchmark
        base.benchmarks = market.benchmarks
        if base.benchmarks["SP500"] == nil { base.benchmarks["SP500"] = market.benchmark }
        base.equity = equitySeries(navHistory)
        for (nick, pts) in navByAccount { base.equityByAccount[normAccountName(nick)] = equitySeries(pts) }
        base.journal = journal
        base.distributions = market.distributions
        base.quotes = market.quotes
        base.activities = acts
        base.closed = fifo.closed
        base.openLots = fifo.open
        base.unmatched = fifo.unmatched
        base.trades = buildTrades(fifo.closed, actsById: actsById, securities: securities, journal: journal)
        base.positions = buildPositions(fifo.open, lastPrices: lastFillPrices(acts), securities: securities, today: today, quotes: market.quotes, journal: journal, actsById: actsById)
        base.cashflow = buildCashflow(acts, securities: securities, fx: market.fx)
        base.accounts = accounts.map { a in BHAccountInfo(id: a.id, name: normAccountName(a.name), currency: a.currency, nav: a.nav, type: a.type, status: a.status) }
        base.balances = balances
        base.margin = margin
        base.cashCurrencies = securities.cashCurrencies()
        return base
    }

    static func kpi(_ trades: [BHTrade]) -> BHKPI {
        let vals = trades.map { $0.pnlCad }
        let wins = vals.filter { $0 > 0 }
        let losses = vals.filter { $0 < 0 }
        let be = vals.filter { $0 == 0 }
        let gw = wins.reduce(0, +)
        let gl = abs(losses.reduce(0, +))
        let n = vals.count
        let total = vals.reduce(0, +)
        var k = BHKPI()
        k.realized = total
        k.count = n
        k.wins = wins.count
        k.losses = losses.count
        k.breakeven = be.count
        k.winRate = n > 0 ? Double(wins.count) / Double(n) : nil
        k.grossWin = gw
        k.grossLoss = gl
        k.profitFactor = gl > 0 ? gw / gl : (gw > 0 ? nil : 0)
        k.profitFactorInfinite = gl == 0 && gw > 0
        k.expectancy = n > 0 ? total / Double(n) : nil
        k.avgWin = wins.isEmpty ? 0 : gw / Double(wins.count)
        k.avgLoss = losses.isEmpty ? 0 : -gl / Double(losses.count)
        k.fees = trades.reduce(0.0) { $0 + $1.feesCad }
        k.avgHold = n > 0 ? Double(trades.reduce(0) { $0 + $1.holdDays }) / Double(n) : nil
        k.openCount = trades.filter { $0.status == "open" }.count
        return k
    }

    static func monthLabel(_ key: String) -> String {
        let m = Int(key.dropFirst(5).prefix(2)) ?? 1
        return "\(months[m - 1]) '\(key.dropFirst(2).prefix(2))"
    }

    /// Verified payment frequency from actual payment dates (any order).
    /// Only the most recent gaps count (the last three), so a fund that changes
    /// its schedule is re-read after two payments at the new cadence.
    static func bareTicker(_ symbol: String) -> String {
        var s = symbol.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        for suffix in [".TO", ".V", ".CN", ".NE"] where s.hasSuffix(suffix) { s = String(s.dropLast(suffix.count)) }
        return s
    }

    static func bySymbol<T>(_ mapping: [String: T], _ symbol: String) -> T? {
        return mapping[symbol] ?? mapping[bareTicker(symbol)]
    }

    static func paymentsPerYear(_ dates: [String]) -> Int? {
        let days = Array(Set(dates.map { String($0.prefix(10)) }.filter { !$0.isEmpty })).sorted()
        if days.count < 2 { return nil }
        var gaps = zip(days, days.dropFirst()).map { daysBetween($0, $1) }.filter { $0 > 0 }
        gaps = Array(gaps.suffix(3))
        if gaps.isEmpty { return nil }
        gaps.sort()
        let median = gaps[gaps.count / 2]
        let perYear = 365.25 / Double(median)
        var best = schedules[0]
        for s in schedules where abs(Double(s) - perYear) < abs(Double(best) - perYear) { best = s }
        return best
    }
}
