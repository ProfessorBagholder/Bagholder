// The view half of the model, a port of crates/model/src/view.rs filters, NAV analytics and
// build_view: one filter set applied to the base, and everything a page shows
// computed from the same filtered lists.
import Foundation

struct BHEquityPoint {
    var d = ""
    var v = 0.0
    var dep: Double?
}

struct BHRange: Equatable {
    var op = ">"
    var v: Double?
}

/// One filter set, the shape clean_filters produces.
struct BHFilters: Equatable {
    static let listKeys = ["account", "symbol", "grade", "tag", "kind", "exchange", "side", "result"]
    static let rangeKeys = ["price", "hold", "pnl", "qty"]
    var lists: [String: [String]] = Dictionary(uniqueKeysWithValues: listKeys.map { ($0, []) })
    var ranges: [String: BHRange] = Dictionary(uniqueKeysWithValues: rangeKeys.map { ($0, BHRange()) })
    var preset = "all"
    var years: [String] = []
    var from = ""
    var to = ""
    var search = ""
    var benchmark = "SP500"

    init() {}

    /// clean_filters: a loose dictionary (JSON) into a full filter set.
    init(json raw: [String: Any]) {
        if let lists = raw["lists"] as? [String: Any] {
            for k in Self.listKeys {
                if let vals = lists[k] as? [Any] {
                    self.lists[k] = vals.compactMap { v -> String? in
                        let s = BHFilters.str(v)
                        return s.isEmpty ? nil : s
                    }
                }
            }
        }
        if let ranges = raw["ranges"] as? [String: Any] {
            for k in Self.rangeKeys {
                if let r = ranges[k] as? [String: Any] {
                    let op = Self.str(r["op"])
                    self.ranges[k]!.op = (op == ">" || op == "<") ? op : ">"
                    if let v = r["v"], !(v is NSNull), !(Self.str(v).isEmpty), let n = (v as? NSNumber)?.doubleValue ?? Double(Self.str(v)) {
                        self.ranges[k]!.v = n
                    } else {
                        self.ranges[k]!.v = nil
                    }
                }
            }
        }
        let preset = Self.str(raw["preset"]).lowercased()
        self.preset = (BHModel.presetDays[preset] != nil || preset == "ytd" || preset == "all") ? preset : "all"
        if let years = raw["years"] as? [Any] {
            self.years = Array(Set(years.map { String(Self.str($0).prefix(4)) }.filter { BHModel.reTest("^\\d{4}$", $0) })).sorted()
        }
        for k in ["from", "to"] {
            let v = String(Self.str(raw[k]).prefix(10))
            let ok = BHModel.reTest("^\\d{4}-\\d{2}-\\d{2}$", v) ? v : ""
            if k == "from" { self.from = ok } else { self.to = ok }
        }
        self.search = Self.str(raw["search"]).trimmingCharacters(in: .whitespacesAndNewlines)
        let b = Self.str(raw["benchmark"]).trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        self.benchmark = BHModel.benchmarkLabels[b] != nil ? b : "SP500"
    }

    private static func str(_ v: Any?) -> String {
        guard let v = v, !(v is NSNull) else { return "" }
        if let s = v as? String { return s }
        if let n = v as? NSNumber { return n.stringValue }
        return "\(v)"
    }

    var isActive: Bool {
        lists.values.contains { !$0.isEmpty } || ranges.values.contains { $0.v != nil } || preset != "all" || !years.isEmpty || !from.isEmpty || !to.isEmpty || !search.isEmpty
    }
}

struct BHYearRow {
    var year = ""
    var r = 0.0
    var days = 0
    var from = "", to = ""
    var flow: Double?
    var endV: Double?
    var spR: Double?
}

struct BHAnnualized {
    var rate: Double?
    var years = 0.0
    var count = 0
    var first = "", last = ""
}

struct BHDrawdown {
    var pct: Double?
    var abs: Double?
    var at = "", peakAt = ""
}

struct BHMonthBucket {
    var key = "", label = ""
    var value = 0.0
    var count = 0
    var tradeIds: [String] = []
}

struct BHSymbolRow {
    var symbol = ""
    var pnl = 0.0
    var n = 0, legs = 0
    var winRate = 0.0, avgHold = 0.0
    var tradeIds: [String] = []
}

struct BHGradeBucket {
    var grade = ""
    var n = 0
    var pnl = 0.0
    var tradeIds: [String] = []
}

struct BHGrades {
    var buckets: [BHGradeBucket] = []
    var ungraded = 0, graded = 0
}

struct BHQueueRow {
    var id = "", symbol = "", date = ""
    var pnl = 0.0
    var currency = "CAD"
    var missing = ""
}

struct BHPositionsSummary {
    var count = 0
    var book = 0.0, mv = 0.0, unreal = 0.0
}

struct BHOptions {
    var accounts: [String] = [], symbols: [String] = [], tags: [String] = [], exchanges: [String] = [], kinds: [String] = []
    var grades: [String] = [], sides: [String] = [], results: [String] = [], years: [String] = []
}

struct BHEquityView {
    var label = ""
    var series: [BHEquityPoint] = []
    var drawdown = BHDrawdown()
    var annualized = BHAnnualized()
}

/// Everything a page shows for one filter set.
struct BHView {
    var today = ""
    var filters = BHFilters()
    var options = BHOptions()
    var kpi = BHKPI()
    var equity = BHEquityView()
    var years: [BHYearRow] = []
    var benchmarkKey = "SP500", benchmarkLabel = "S&P 500"
    var monthly: [BHMonthBucket] = []
    var bySymbol: [BHSymbolRow] = []
    var grades = BHGrades()
    var queue: [BHQueueRow] = []
    var trades: [BHTrade] = []
    var tradeTotal = 0
    var positions: [BHPosition] = []
    var positionsSummary = BHPositionsSummary()
    var portfolio = BHPortfolio()
    var cashflow = BHCashflowView()
    var unmatched: [BHUnmatched] = []
}

extension BHCashflowView {
    // the filters Cashflow could not apply, named
    var skippedFilters: [String] { skipped }
}

extension BHModel {
    static let grades = ["A", "B", "C", "F"]
    static let benchmarkLabels = ["SP500": "S&P 500", "TSX": "S&P/TSX", "TSX60": "TSX 60"]
    static let presetDays = ["1d": 1, "1w": 7, "1m": 30, "3m": 90, "6m": 180, "1y": 365, "5y": 1826]

    // MARK: NAV: equity series, yearly time-weighted returns, drawdown

    static func equitySeries(_ points: [BHNavPoint]) -> [BHEquityPoint] {
        var out: [BHEquityPoint] = []
        for p in points {
            let d = String(p.date.prefix(10))
            guard !d.isEmpty, let v = p.equity else { continue }
            out.append(BHEquityPoint(d: d, v: v, dep: p.netDeposits))
        }
        out.sort { $0.d < $1.d }
        return out
    }

    static func navOn(_ series: [BHEquityPoint], _ day: String) -> Double? {
        var v: Double?
        for p in series {
            if p.d > day { break }
            v = p.v
        }
        return v
    }

    static func depositsOn(_ series: [BHEquityPoint], _ day: String) -> Double? {
        var v: Double?
        for p in series {
            if p.d > day { break }
            if let dep = p.dep { v = dep }
        }
        return v
    }

    /// Daily chain-linked return for one calendar year, net of deposits.
    static func yearReturn(_ series: [BHEquityPoint], _ year: String, _ today: String) -> (r: Double, from: String, to: String, days: Int)? {
        let cal = year + "-01-01"
        let to = min(year + "-12-31", today)
        if series.isEmpty { return nil }
        // A balance under 1% of the account's peak is pre-history (a few dollars
        // parked before the real start): a chain that began there would turn the
        // first big deposit into a wild return, so the chain starts at the first
        // point that clears the floor, and the year is measured from there.
        let floor = series.map { $0.v }.max()! * 0.01
        let startDay = shiftDate(cal, -1)
        var start = navOn(series, startDay)
        var after = startDay
        if !(start != nil && start! != 0 && start! > floor) {
            guard let first = series.first(where: { cal <= $0.d && $0.d <= to && $0.v > floor }) else { return nil }
            start = first.v
            after = first.d
        }
        let pts = series.filter { after < $0.d && $0.d <= to }
        if pts.isEmpty { return nil }
        var prevEq = start!
        var prevDep = depositsOn(series, after)
        var factor = 1.0
        for p in pts {
            let eq = p.v
            if !(prevEq > 0) { return nil }
            var cf = 0.0
            if let dep = p.dep, let pd = prevDep { cf = dep - pd }
            factor *= 1 + (eq - prevEq - cf) / prevEq
            prevEq = eq
            if let dep = p.dep { prevDep = dep }
        }
        let r = factor - 1
        if r.isNaN || r.isInfinite { return nil }
        let spanFrom = after == startDay ? cal : after
        return (r, spanFrom, to, daysBetween(spanFrom, to))
    }

    /// The index over the same span the account's year covers: the calendar year,
    /// or from `start` when the account was funded part way through it.
    static func benchmarkReturn(_ bench: [String: Double], _ year: String, _ today: String, start: String? = nil) -> Double? {
        if bench.isEmpty { return nil }
        let days = bench.keys.sorted()
        let s = String((start ?? "").prefix(10))
        let cal = s.isEmpty ? year + "-01-01" : s
        let to = min(year + "-12-31", today)
        var prev: Double?
        var end: Double?
        for d in days {
            if d < cal {
                prev = bench[d]
            } else if d <= to {
                end = bench[d]
            }
        }
        if prev == nil {
            let firsts = days.filter { cal <= $0 && $0 <= to }
            guard let f = firsts.first else { return nil }
            prev = bench[f]
        }
        guard let p = prev, p != 0, let e = end else { return nil }
        return e / p - 1
    }

    static func yearlyReturns(_ series: [BHEquityPoint], _ bench: [String: Double], _ today: String) -> [BHYearRow] {
        if series.isEmpty { return [] }
        let years = Array(Set(series.map { String($0.d.prefix(4)) })).sorted()
        let peak = series.map { $0.v }.max()!
        var out: [BHYearRow] = []
        for y in years {
            // a year in which the account never held more than 1% of its peak is
            // pre-history (a few hundred dollars parked before the real start)
            let yearPeak = series.filter { String($0.d.prefix(4)) == y }.map { $0.v }.max() ?? 0
            if peak > 0 && yearPeak < peak * 0.01 { continue }
            guard let yr = yearReturn(series, y, today) else { continue }
            let startDep = depositsOn(series, shiftDate(y + "-01-01", -1))
            let endDep = depositsOn(series, yr.to)
            var flow: Double?
            if let s = startDep, let e = endDep { flow = e - s }
            var row = BHYearRow()
            row.year = y
            row.r = yr.r
            row.days = yr.days
            row.from = yr.from
            row.to = yr.to
            row.flow = flow
            row.endV = navOn(series, yr.to)
            row.spR = benchmarkReturn(bench, y, today, start: yr.from != y + "-01-01" ? yr.from : nil)
            out.append(row)
        }
        return out
    }

    static func annualized(_ years: [BHYearRow]) -> BHAnnualized {
        var prod = 1.0
        var days = 0
        var used: [String] = []
        for y in years {
            if y.r <= -1 || y.days < 30 { continue }
            prod *= 1 + y.r
            days += y.days
            used.append(y.year)
        }
        if days == 0 { return BHAnnualized(rate: nil, years: 0, count: 0, first: "", last: "") }
        let yrs = Double(days) / 365.25
        let rate = yrs >= 1.0 / 12.0 ? pow(prod, 1 / yrs) - 1 : prod - 1
        return BHAnnualized(rate: rate, years: yrs, count: used.count, first: used.first!, last: used.last!)
    }

    /// Net deposit change per day, moved one day later when the equity
    /// series only reflects the money a day after the deposit record does.
    static func pairedFlows(_ series: [BHEquityPoint]) -> [Double] {
        let n = series.count
        var flows = [Double](repeating: 0, count: n)
        if n < 2 { return flows }
        for i in 1..<n {
            let p = series[i], prev = series[i - 1]
            guard let dep = p.dep, let pd = prev.dep else { continue }
            let cf = dep - pd
            if abs(cf) < eps { continue }
            let changeToday = p.v - prev.v
            if i + 1 < n {
                let changeNext = series[i + 1].v - p.v
                if abs(changeToday - cf) > abs(changeNext - cf) && abs(changeToday) < abs(cf) * 0.5 {
                    flows[i + 1] += cf
                    continue
                }
            }
            flows[i] += cf
        }
        return flows
    }

    /// Max drawdown of the flow-adjusted equity: daily returns are taken net
    /// of deposits and withdrawals and chain-linked into an index, so money
    /// moved in or out of the account is not counted as a gain or a loss.
    static func drawdown(_ series: [BHEquityPoint]) -> BHDrawdown {
        if series.isEmpty { return BHDrawdown() }
        let peakV = series.map { $0.v }.max()!
        let floor = peakV * 0.01
        var idx = 1.0
        var prev: BHEquityPoint?
        var peakIdx = 0.0
        var peakAt = ""
        var peakEquity = 0.0
        var dd = 0.0, ddAbs = 0.0
        var ddAt = "", ddPeakAt = ""
        let flows = pairedFlows(series)
        for (i, p) in series.enumerated() {
            if let pr = prev, pr.v > floor, pr.v > 0 {
                idx *= 1 + (p.v - pr.v - flows[i]) / pr.v
            }
            prev = p
            if p.v < floor { continue }
            if idx >= peakIdx {
                peakIdx = idx
                peakAt = p.d
                peakEquity = p.v
            }
            if peakIdx <= 0 { continue }
            let drop = idx / peakIdx - 1
            if drop < dd {
                dd = drop
                ddAbs = drop * peakEquity
                ddAt = p.d
                ddPeakAt = peakAt
            }
        }
        return BHDrawdown(pct: dd, abs: ddAbs, at: ddAt, peakAt: ddPeakAt)
    }

    // MARK: filters

    static func dateBounds(_ f: BHFilters, _ today: String) -> (String, String)? {
        if !f.from.isEmpty || !f.to.isEmpty {
            return (f.from.isEmpty ? "0000-01-01" : f.from, f.to.isEmpty ? "9999-12-31" : f.to)
        }
        if !f.years.isEmpty { return nil }
        if f.preset == "ytd" { return (String(today.prefix(4)) + "-01-01", today) }
        if let days = presetDays[f.preset] { return (shiftDate(today, -days), today) }
        return nil
    }

    static func inDateScope(_ f: BHFilters, _ today: String, _ day: String) -> Bool {
        if let b = dateBounds(f, today) { return b.0 <= day && day <= b.1 }
        if !f.years.isEmpty { return f.years.contains(String(day.prefix(4))) }
        return true
    }

    static func tradeMatches(_ t: BHTrade, _ f: BHFilters, _ today: String) -> Bool {
        let s = f.search.uppercased()
        if !s.isEmpty && !t.symbol.uppercased().contains(s) && !t.underlying.uppercased().contains(s) && !t.name.uppercased().contains(s) { return false }
        let L = f.lists
        if let a = L["account"], !a.isEmpty, !a.contains(t.account) { return false }
        if let sy = L["symbol"], !sy.isEmpty, !sy.contains(t.symbol), !sy.contains(t.underlying) { return false }
        if let g = L["grade"], !g.isEmpty, !g.contains(t.grade.isEmpty ? "Ungraded" : t.grade) { return false }
        if let tg = L["tag"], !tg.isEmpty {
            let tags = t.tags.isEmpty ? ["untagged"] : t.tags
            if !tags.contains(where: { tg.contains($0) }) { return false }
        }
        if let k = L["kind"], !k.isEmpty, !k.contains(t.kind) { return false }
        if let e = L["exchange"], !e.isEmpty, !e.contains(t.exchange) { return false }
        if let sd = L["side"], !sd.isEmpty, !sd.contains(t.side) { return false }
        if let r = L["result"], !r.isEmpty {
            let res = t.pnlCad > 0 ? "Winners" : (t.pnlCad < 0 ? "Losers" : "Breakeven")
            if !r.contains(res) { return false }
        }
        for (key, val) in [("price", t.entry), ("hold", Double(t.holdDays)), ("pnl", t.pnlCad), ("qty", t.qty)] {
            guard let r = f.ranges[key], let v = r.v else { continue }
            if r.op == ">" && !(val > v) { return false }
            if r.op == "<" && !(val < v) { return false }
        }
        return inDateScope(f, today, t.exitDate)
    }

    static func positionMatches(_ p: BHPosition, _ f: BHFilters) -> Bool {
        let s = f.search.uppercased()
        if !s.isEmpty && !p.symbol.uppercased().contains(s) && !p.name.uppercased().contains(s) { return false }
        let L = f.lists
        if let a = L["account"], !a.isEmpty, !a.contains(p.account) { return false }
        if let sy = L["symbol"], !sy.isEmpty, !sy.contains(p.symbol), !sy.contains(p.underlying) { return false }
        if let k = L["kind"], !k.isEmpty, !k.contains(p.kind) { return false }
        if let e = L["exchange"], !e.isEmpty, !e.contains(p.exchange) { return false }
        return true
    }

    // MARK: the dashboard cards

    static func bySymbol(_ trades: [BHTrade]) -> [BHSymbolRow] {
        var by: [String: (pnl: Double, n: Int, wins: Int, hold: Int, legs: Int, ids: [String])] = [:]
        var order: [String] = []
        for t in trades {
            let k = t.underlying
            if by[k] == nil {
                by[k] = (0, 0, 0, 0, 0, [])
                order.append(k)
            }
            by[k]!.pnl += t.pnlCad
            by[k]!.n += 1
            by[k]!.legs += t.legCount
            by[k]!.hold += t.holdDays
            by[k]!.ids.append(t.id)
            if t.pnlCad > 0 { by[k]!.wins += 1 }
        }
        var rows = order.map { k -> BHSymbolRow in
            let g = by[k]!
            return BHSymbolRow(symbol: k, pnl: g.pnl, n: g.n, legs: g.legs, winRate: g.n > 0 ? Double(g.wins) / Double(g.n) : 0,
                               avgHold: g.n > 0 ? Double(g.hold) / Double(g.n) : 0, tradeIds: g.ids)
        }
        rows.sort { $0.pnl > $1.pnl }
        return rows
    }

    static func monthly(_ trades: [BHTrade]) -> [BHMonthBucket] {
        var by: [String: BHMonthBucket] = [:]
        for t in trades {
            let k = String(t.exitDate.prefix(7))
            if k.count < 7 { continue }
            if by[k] == nil { by[k] = BHMonthBucket(key: k, label: monthLabel(k)) }
            by[k]!.value += t.pnlCad
            by[k]!.count += 1
            by[k]!.tradeIds.append(t.id)
        }
        return by.keys.sorted().map { by[$0]! }
    }

    static func gradeBuckets(_ trades: [BHTrade]) -> BHGrades {
        var g = BHGrades()
        for grade in grades {
            let rows = trades.filter { $0.grade == grade }
            g.buckets.append(BHGradeBucket(grade: grade, n: rows.count, pnl: rows.reduce(0.0) { $0 + $1.pnlCad }, tradeIds: rows.map { $0.id }))
        }
        g.ungraded = trades.filter { $0.grade.isEmpty }.count
        g.graded = trades.count - g.ungraded
        return g
    }

    static func reviewQueue(_ trades: [BHTrade]) -> [BHQueueRow] {
        var out: [BHQueueRow] = []
        for t in trades {
            let noGrade = t.grade.isEmpty
            let noThesis = t.thesis.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            if !(noGrade || noThesis) { continue }
            out.append(BHQueueRow(id: t.id, symbol: t.symbol, date: t.exitDate, pnl: t.pnlCad, currency: "CAD",
                                  missing: noGrade && noThesis ? "no grade or thesis" : (noGrade ? "no grade" : "no thesis")))
        }
        out.sort { $0.date > $1.date }
        return out
    }

    // MARK: the Cashflow page for one filter set

    static func cashflowView(_ base: BHBase, _ f: BHFilters, _ positionsAll: [BHPosition], marginUsed: Double = 0, hasMargin: Bool = true) -> BHCashflowView {
        let today = base.today
        let accts = f.lists["account"] ?? []
        let symbolsF = f.lists["symbol"] ?? []
        let search = f.search.uppercased()

        func inScope(_ r: BHCashRow) -> Bool {
            if !accts.isEmpty && !accts.contains(r.account) { return false }
            if !search.isEmpty && !r.symbol.uppercased().contains(search) { return false }
            if !symbolsF.isEmpty && !symbolsF.contains(r.symbol) { return false }
            return inDateScope(f, today, r.date)
        }

        let everything = base.cashflow.filter(inScope)
        let recs = everything.filter { $0.kind == "Dividend" }
        var skipped = ["grade", "tag", "kind", "exchange", "side", "result"].filter { !(f.lists[$0] ?? []).isEmpty }
        skipped += BHFilters.rangeKeys.filter { f.ranges[$0]?.v != nil }

        var keys: [String] = []
        var bucket: [String: (sum: Double, n: Int)] = [:]
        if !recs.isEmpty {
            let monthsSeen = Array(Set(recs.map { String($0.date.prefix(7)) })).sorted()
            let first = monthsSeen[0]
            var last = monthsSeen[monthsSeen.count - 1]
            // the chart runs to the current month (or the end of the date filter), with
            // an empty bar for a month that has not paid yet
            var endDay = today
            if let b = dateBounds(f, today) {
                endDay = min(b.1, today)
            } else if !f.years.isEmpty {
                endDay = min(f.years.max()! + "-12-31", today)
            }
            last = max(last, String(endDay.prefix(7)))
            var y = Int(first.prefix(4))!, m = Int(first.dropFirst(5).prefix(2))!
            while true {
                let k = String(format: "%04d-%02d", y, m)
                if k > last { break }
                keys.append(k)
                bucket[k] = (0, 0)
                m += 1
                if m > 12 { m = 1; y += 1 }
            }
        }
        for r in recs {
            let k = String(r.date.prefix(7))
            if bucket[k] != nil {
                bucket[k]!.sum += r.amountCad
                bucket[k]!.n += 1
            }
        }
        let months = keys.map { BHMonth(key: $0, label: monthLabel($0), value: bucket[$0]!.sum, count: bucket[$0]!.n) }

        let payers = Set(base.cashflow.filter { $0.kind == "Dividend" }.map { $0.symbol })
        var held = positionsAll.filter { payers.contains($0.symbol) && !$0.short }
        held = held.filter { (accts.isEmpty || accts.contains($0.account)) && (search.isEmpty || $0.symbol.uppercased().contains(search)) }
        let forYoc = base.cashflow.filter { $0.kind == "Dividend" && (accts.isEmpty || accts.contains($0.account)) && (search.isEmpty || $0.symbol.uppercased().contains(search)) }
        let lastRec = recs.first?.date ?? today
        var cm = Int(lastRec.dropFirst(5).prefix(2))! - 11
        var cy = Int(lastRec.prefix(4))!
        while cm <= 0 { cm += 12; cy -= 1 }
        let cut = String(format: "%04d-%02d", cy, cm)
        let thisYear = String(today.prefix(4))

        func sumFor(_ sym: String, _ pred: (BHCashRow) -> Bool) -> Double {
            forYoc.filter { $0.symbol == sym && pred($0) }.reduce(0.0) { $0 + $1.amountCad }
        }

        let pub = base.distributions
        let quotes = base.quotes

        struct Rate { var per: Double; var freq: Int; var annual: Double; var verified: Bool; var source: String }

        func rateFor(_ sym: String) -> Rate? {
            // Preferred: the fund's own declared record (TMX Money): the latest
            // distribution that has gone ex, and payments per year from the gaps
            // between its recent ex-dates, so a schedule change shows at once.
            var declared = (pub[sym] ?? []).filter { $0.exDate <= today }
            if !declared.isEmpty {
                declared.sort { $0.exDate > $1.exDate }
                let per = declared[0].amount
                let freq = paymentsPerYear((pub[sym] ?? []).map { $0.exDate })
                if per != 0, let freq = freq {
                    return Rate(per: per, freq: freq, annual: per * Double(freq), verified: true, source: "declared")
                }
            }
            // Otherwise this holding's own payment rows.
            let rs = forYoc.filter { $0.symbol == sym && ($0.per ?? 0) != 0 }.sorted { $0.date > $1.date }
            guard let per = rs.first?.per, per != 0 else { return nil }
            let freq = paymentsPerYear(forYoc.filter { $0.symbol == sym }.map { $0.date })
            let verified = freq != nil
            let fq = freq ?? 12
            return Rate(per: per, freq: fq, annual: per * Double(fq), verified: verified, source: "payments")
        }

        /// (ex-date, pay date, ex passed, pay passed): the next distribution still
        /// to be paid, whether or not it has gone ex, else the last known one.
        func distributionDates(_ sym: String) -> (String, String, Bool, Bool) {
            func payOf(_ d: BHDistribution) -> String { let p = String(d.payDate.prefix(10)); return p.isEmpty ? d.exDate : p }
            let recs_ = (pub[sym] ?? []).sorted { (payOf($0), $0.exDate) < (payOf($1), $1.exDate) }
            let unpaid = recs_.filter { payOf($0) >= today }
            let pick = unpaid.first ?? recs_.last
            var ex = "", pay = ""
            if let p = pick {
                ex = p.exDate
                pay = String(p.payDate.prefix(10))
            } else {
                ex = String((quotes[sym]?.exDividendDate ?? "").prefix(10))
                let paid = forYoc.filter { $0.symbol == sym }.map { $0.date }.sorted()
                pay = paid.last ?? ""
            }
            return (ex, pay, !ex.isEmpty && ex < today, !pay.isEmpty && pay < today)
        }

        func lastPrice(_ p: BHPosition) -> (Double, String) {
            if let px = quotes[p.symbol]?.price, px > 0 { return (px, "close") }
            return (p.last, "fill")
        }

        var holdings: [BHHolding] = []
        for p in held {
            let r = rateFor(p.symbol)
            let basis = p.cost
            let avg = p.avg
            let (lastPx, priceSource) = lastPrice(p)
            let dd = distributionDates(p.symbol)
            var h = BHHolding()
            h.id = p.id
            h.symbol = p.symbol
            h.account = p.account
            h.qty = p.qty
            h.per = r?.per
            h.freq = r?.freq
            h.freqVerified = r?.verified ?? false
            h.rateSource = r?.source ?? ""
            h.cost = basis
            h.avg = avg
            h.last = lastPx
            h.priceSource = priceSource
            h.ytd = sumFor(p.symbol) { String($0.date.prefix(4)) == thisYear }
            h.ttm = sumFor(p.symbol) { String($0.date.prefix(7)) >= cut }
            h.all = sumFor(p.symbol) { _ in true }
            h.nextExDate = dd.0
            h.nextPayDate = dd.1
            h.exPast = dd.2
            h.payPast = dd.3
            h.yob = r.map { $0.per * p.qty }
            h.annual = r.map { $0.annual * p.qty }
            h.yoc = (r != nil && avg != 0) ? r!.annual / avg : nil
            h.currentYield = (r != nil && lastPx != 0) ? r!.annual / lastPx : nil
            holdings.append(h)
        }
        let verified = holdings.filter { $0.annual != nil }
        let basisAll = verified.reduce(0.0) { $0 + $1.cost }
        let earnedAll = verified.reduce(0.0) { $0 + $1.ttm }
        let annualAll = verified.reduce(0.0) { $0 + $1.annual! }
        let total = recs.reduce(0.0) { $0 + $1.amountCad }
        let thisYr = Int(thisYear)!
        var tiles: [BHTile] = []
        for y in [thisYr - 2, thisYr - 1, thisYr] {
            let ys = String(y)
            let rs = recs.filter { String($0.date.prefix(4)) == ys }
            let sm = rs.reduce(0.0) { $0 + $1.amountCad }
            var paid = keys.filter { String($0.prefix(4)) == ys && bucket[$0]!.n > 0 }.count
            if paid == 0 { paid = 1 }
            tiles.append(BHTile(label: y == thisYr ? "\(y) YTD" : ys, total: sm, perMonth: sm / Double(paid), count: rs.count))
        }
        var monthsInScope = keys.filter { bucket[$0]!.n > 0 }.count
        if monthsInScope == 0 { monthsInScope = 1 }
        tiles.append(BHTile(label: "All time", total: total, perMonth: total / Double(monthsInScope), count: recs.count))
        if hasMargin {
            // margin used is the Portfolio tab's figure; under it the average margin interest per charged month
            let charges = everything.filter { $0.kind == "Interest charge" }
            let chargeMonths = Set(charges.map { String($0.date.prefix(7)) }).count
            let charged = charges.reduce(0.0) { $0 - $1.amountCad }
            tiles.append(BHTile(label: "Margin used", marginUsed: marginUsed, interestPerMonth: chargeMonths > 0 ? charged / Double(chargeMonths) : 0, interestMonths: chargeMonths))
        } else {
            // without a margin account: the trailing twelve months, averaged over the months that paid
            let since = BHModel.shiftDate(today, -365)
            let window = recs.filter { $0.date > since && $0.date <= today }
            let sm = window.reduce(0.0) { $0 + $1.amountCad }
            let paid = max(1, Set(window.map { String($0.date.prefix(7)) }).count)
            tiles.append(BHTile(label: "Last 12 months", total: sm, perMonth: sm / Double(paid), count: window.count))
        }
        tiles.append(BHTile(label: "Yield on cost", yield: basisAll != 0 ? annualAll / basisAll : nil, projected: annualAll / 12, earned: earnedAll, book: basisAll))
        let other = everything.filter { $0.kind != "Dividend" }
        var v = BHCashflowView()
        v.tiles = tiles
        v.months = months
        v.holdings = holdings
        v.rows = recs
        v.other = other
        v.total = total
        v.count = recs.count
        v.skipped = skipped
        v.interest = other.filter { $0.kind == "Interest" }.reduce(0.0) { $0 + $1.amountCad }
        v.withholding = other.filter { $0.kind == "Withholding tax" }.reduce(0.0) { $0 + $1.amountCad }
        return v
    }

    // MARK: build_view

    static func buildView(_ base: BHBase, _ f: BHFilters) -> BHView {
        let today = base.today
        let tradesAll = base.trades
        let trades = tradesAll.filter { tradeMatches($0, f, today) }
        let positionsAll = base.positions
        let positions = positionsAll.filter { positionMatches($0, f) }

        let accts = f.lists["account"] ?? []
        var series = base.equity
        var seriesLabel = "All accounts"
        if accts.count == 1, let s = base.equityByAccount[accts[0]] {
            series = s
            seriesLabel = accts[0]
        }
        let benchKey = f.benchmark
        let years = yearlyReturns(series, base.benchmarks[benchKey] ?? [:], today)
        let ann = annualized(years)
        let dd = drawdown(series)
        var shown = series
        if let b = dateBounds(f, today) {
            shown = series.filter { b.0 <= $0.d && $0.d <= b.1 }
        } else if !f.years.isEmpty {
            shown = series.filter { f.years.contains(String($0.d.prefix(4))) }
        }
        if !shown.isEmpty {
            let peak = shown.map { $0.v }.max()!
            let firstIdx = shown.firstIndex { $0.v > peak * 0.01 } ?? 0
            shown = Array(shown[firstIdx...])
        }

        var o = BHOptions()
        o.tags = Array(Set(tradesAll.flatMap { $0.tags })).sorted()
        o.symbols = Array(Set(tradesAll.map { $0.symbol }).union(positionsAll.map { $0.symbol })).sorted()
        o.accounts = Array(Set(tradesAll.map { $0.account }).union(positionsAll.map { $0.account }).union(base.cashflow.map { $0.account })).sorted()
        o.exchanges = Array(Set(tradesAll.map { $0.exchange }.filter { !$0.isEmpty }).union(positionsAll.map { $0.exchange }.filter { !$0.isEmpty })).sorted()
        o.kinds = kinds.filter { k in tradesAll.contains { $0.kind == k } || positionsAll.contains { $0.kind == k } }
        o.grades = grades + ["Ungraded"]
        o.sides = ["SELL", "COVER"]
        o.results = ["Winners", "Losers", "Breakeven"]
        o.years = Array(Set(tradesAll.filter { !$0.exitDate.isEmpty }.map { String($0.exitDate.prefix(4)) })).sorted().reversed()

        var v = BHView()
        v.today = today
        v.filters = f
        v.options = o
        v.kpi = kpi(trades)
        v.equity = BHEquityView(label: seriesLabel, series: shown, drawdown: dd, annualized: ann)
        v.years = years
        v.benchmarkKey = benchKey
        v.benchmarkLabel = benchmarkLabels[benchKey] ?? "S&P 500"
        v.monthly = monthly(trades)
        v.bySymbol = bySymbol(trades)
        v.grades = gradeBuckets(trades)
        v.queue = reviewQueue(trades)
        v.trades = trades
        v.tradeTotal = tradesAll.count
        v.positions = positions
        v.positionsSummary = BHPositionsSummary(
            count: positions.count,
            book: positions.reduce(0.0) { $0 + abs($1.cost) },
            mv: positions.reduce(0.0) { $0 + ($1.short ? -$1.mv : $1.mv) },
            unreal: positions.reduce(0.0) { $0 + $1.unreal })
        v.portfolio = portfolioView(base, f, positions)
        v.cashflow = cashflowView(base, f, positionsAll, marginUsed: v.portfolio.marginUsed, hasMargin: v.portfolio.hasMargin)
        v.unmatched = base.unmatched
        return v
    }

    /// crates/model/src/view.rs portfolio_view: CAD aggregates over the accounts the filter has on, every account when it has none.
    static func portfolioView(_ base: BHBase, _ f: BHFilters, _ positions: [BHPosition]) -> BHPortfolio {
        let fx = base.fx, today = base.today
        func cad(_ amount: Double, _ currency: String) -> Double { toCad(fx, amount, currency, today) }
        let names = f.lists["account"] ?? []
        // closed accounts hold nothing and count for nothing here
        let accounts = base.accounts.filter { $0.status.lowercased() != "closed" && (names.isEmpty || names.contains($0.name)) }
        let ids = Set(accounts.map { $0.id })
        var nameOf: [String: String] = [:]
        for a in accounts { nameOf[a.id] = a.name }
        var out = BHPortfolio()
        out.marketValue = positions.reduce(0.0) { $0 + cad($1.short ? -$1.mv : $1.mv, $1.currency) }
        out.costBasis = positions.reduce(0.0) { $0 + cad(abs($1.cost), $1.currency) }
        out.unrealized = positions.reduce(0.0) { $0 + cad($1.unreal, $1.currency) }
        out.unrealizedPct = out.costBasis != 0 ? out.unrealized / out.costBasis : nil
        out.positionCount = positions.count
        out.accountCount = Set(positions.map { $0.account }).count
        let navs = accounts.compactMap { a in a.nav.map { cad($0, a.currency) } }
        out.nav = navs.isEmpty ? nil : navs.reduce(0.0, +)
        out.navAccounts = navs.count
        var used: [String: Double] = [:]
        for b in base.balances {
            guard ids.contains(b.accountId), let ccy = base.cashCurrencies[b.securityId], b.quantity < 0 else { continue }
            used[ccy, default: 0.0] += -b.quantity
        }
        out.marginUsed = used.reduce(0.0) { $0 + cad($1.value, $1.key) }
        // the positive cash balances, the other side of the same rows
        var cashBy: [String: Double] = [:]
        for b in base.balances {
            guard ids.contains(b.accountId), let ccy = base.cashCurrencies[b.securityId], b.quantity > 0 else { continue }
            cashBy[ccy, default: 0.0] += b.quantity
        }
        out.cash = cashBy.reduce(0.0) { $0 + cad($1.value, $1.key) }
        out.cashPct = (out.nav ?? 0) != 0 ? out.cash / out.nav! : nil
        // the day's change: each quoted position's, summed, over what those positions were worth at the previous close
        let quoted = positions.filter { $0.dayChange != nil }
        if !quoted.isEmpty {
            let dc = quoted.reduce(0.0) { $0 + cad($1.dayChange!, $1.currency) }
            let prev = quoted.reduce(0.0) { $0 + cad($1.short ? -$1.mv : $1.mv, $1.currency) } - dc
            out.dayChange = dc
            out.dayChangePct = prev != 0 ? dc / prev : nil
        }
        out.marginUsedBy = used.mapValues { ($0 * 100).rounded() / 100 }
        out.marginUsedPct = out.marketValue != 0 ? out.marginUsed / out.marketValue : nil
        var avail: [Double] = []
        var unavailable: [String] = []
        // only a margin account's buying power is margin available; any other row is cash to buy with
        let marginIds = Set(accounts.filter { $0.type.uppercased().contains("MARGIN") }.map { $0.id })
        out.hasMargin = !marginIds.isEmpty
        for m in base.margin where marginIds.contains(m.accountId) {
            if let bp = m.buyingPower { avail.append(cad(bp, m.currency.isEmpty ? "CAD" : m.currency)) }
            else { unavailable.append(nameOf[m.accountId] ?? m.accountId) }
        }
        out.availableMargin = avail.isEmpty ? nil : avail.reduce(0.0, +)
        out.availableMarginUnavailable = unavailable.sorted()
        var alloc: [BHAllocationRow] = []
        for p in positions {
            let v = cad(p.mv, p.currency)
            if v > 0 { alloc.append(BHAllocationRow(id: p.id, symbol: p.symbol, account: p.account, value: v, share: 0)) }
        }
        alloc.sort { $0.value > $1.value }
        let total = alloc.reduce(0.0) { $0 + $1.value }
        for i in alloc.indices { alloc[i].share = total != 0 ? alloc[i].value / total : 0 }
        out.allocation = alloc
        return out
    }
}
