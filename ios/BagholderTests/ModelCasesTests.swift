import XCTest
@testable import Bagholder

/// The shared model cases in ../../tests/cases, run through the Swift model.
/// The same files run through the Python model (python/tests/test_cases.py) and the
/// Kotlin one; a rule changed in one place fails here. tests/README.md
/// describes the format: `expect` is the view for the case's filters, floats
/// rounded to six places.
final class ModelCasesTests: XCTestCase {
    static let casesDir: URL = {
        var u = URL(fileURLWithPath: #filePath)
        for _ in 0..<3 { u.deleteLastPathComponent() }   // BagholderTests -> ios -> repo root
        return u.appendingPathComponent("tests/cases")
    }()

    // MARK: reading a case

    func str(_ d: [String: Any], _ k: String) -> String { (d[k] as? String) ?? "" }
    func num(_ d: [String: Any], _ k: String) -> Double { (d[k] as? NSNumber)?.doubleValue ?? 0 }
    func numOpt(_ d: [String: Any], _ k: String) -> Double? { (d[k] as? NSNumber)?.doubleValue }

    func activity(_ d: [String: Any]) -> BHAct {
        let a = BHAct()
        a.id = str(d, "id"); a.occurredAt = str(d, "occurredAt"); a.transactionDate = str(d, "transactionDate")
        a.accountId = str(d, "accountId"); a.fifoId = str(d, "fifoId"); a.accountType = str(d, "accountType")
        a.activityType = str(d, "activityType"); a.activitySubType = str(d, "activitySubType")
        a.description = str(d, "description"); a.direction = str(d, "direction"); a.symbol = str(d, "symbol")
        a.name = str(d, "name"); a.currency = str(d, "currency")
        a.quantity = num(d, "quantity"); a.unitPrice = num(d, "unitPrice"); a.commission = num(d, "commission")
        a.netCashAmount = num(d, "netCashAmount")
        a.category = str(d, "category"); a.rawType = str(d, "rawType"); a.aftType = str(d, "aftType")
        a.securityId = str(d, "securityId")
        return a
    }

    func security(_ d: [String: Any]) -> BHSecurity {
        BHSecurity(id: str(d, "id"), symbol: str(d, "symbol"), name: str(d, "name"), underlyingId: str(d, "underlyingId"),
                   primaryExchange: str(d, "primaryExchange"), primaryMic: str(d, "primaryMic"), currency: str(d, "currency"))
    }

    func navPoint(_ d: [String: Any]) -> BHNavPoint {
        BHNavPoint(date: str(d, "date"), equity: numOpt(d, "equity"), netDeposits: numOpt(d, "netDeposits"))
    }

    func closes(_ d: Any?) -> [String: Double] {
        var out: [String: Double] = [:]
        for (k, v) in (d as? [String: Any]) ?? [:] { if let n = (v as? NSNumber)?.doubleValue { out[k] = n } }
        return out
    }

    func market(_ d: [String: Any]) -> BHMarket {
        var m = BHMarket()
        m.fx = closes(d["fx"])
        m.benchmark = closes(d["benchmark"])
        for (k, v) in (d["benchmarks"] as? [String: Any]) ?? [:] { m.benchmarks[k] = closes(v) }
        for (sym, rows) in (d["distributions"] as? [String: Any]) ?? [:] {
            m.distributions[sym] = ((rows as? [[String: Any]]) ?? []).map {
                BHDistribution(exDate: str($0, "exDate"), payDate: str($0, "payDate"), amount: num($0, "amount"), currency: str($0, "currency"))
            }
        }
        for (sym, q) in (d["quotes"] as? [String: Any]) ?? [:] {
            let qd = (q as? [String: Any]) ?? [:]
            m.quotes[sym] = BHQuote(price: numOpt(qd, "price"), priceChange: numOpt(qd, "priceChange"), percentChange: numOpt(qd, "percentChange"),
                                    fetchedAt: str(qd, "fetchedAt"), exDividendDate: str(qd, "exDividendDate"))
        }
        return m
    }

    func journal(_ d: [String: Any]) -> [String: BHJournalEntry] {
        var out: [String: BHJournalEntry] = [:]
        for (k, v) in d {
            let e = (v as? [String: Any]) ?? [:]
            out[k] = BHJournalEntry(grade: str(e, "grade"), thesis: str(e, "thesis"), tags: (e["tags"] as? [String]) ?? [])
        }
        return out
    }

    // MARK: what the Swift model produces, in the fixture's shape

    func opt(_ v: Double?) -> Any { v.map { $0 as Any } ?? NSNull() }

    func expect(_ v: BHView) -> [String: Any] {
        let trades = v.trades.sorted { ($0.entryDate, $0.exitDate, $0.symbol) < ($1.entryDate, $1.exitDate, $1.symbol) }
        let k = v.kpi
        let cf = v.cashflow
        return [
            "kpi": [
                "count": k.count, "wins": k.wins, "losses": k.losses, "breakeven": k.breakeven, "winRate": opt(k.winRate), "realized": k.realized,
                "expectancy": opt(k.expectancy), "profitFactor": opt(k.profitFactor), "avgHold": opt(k.avgHold),
                "avgWin": k.avgWin, "avgLoss": k.avgLoss, "grossWin": k.grossWin, "grossLoss": k.grossLoss,
            ] as [String: Any],
            "trades": trades.map { t -> [String: Any] in
                [
                    "id": t.id, "symbol": t.symbol, "kind": t.kind, "currency": t.currency, "side": t.side, "qty": t.qty, "mult": t.mult,
                    "entry": t.entry, "exit": t.exit, "entryDate": t.entryDate, "exitDate": t.exitDate, "holdDays": t.holdDays,
                    "pnl": t.pnl, "pnlCad": t.pnlCad, "pnlPct": opt(t.pnlPct), "status": t.status, "fees": t.fees,
                    "account": t.account, "exchange": t.exchange, "grade": t.grade, "tags": t.tags,
                    "fills": t.fills.sorted { $0.when < $1.when }.map { $0.sub },
                ]
            },
            "positions": v.positions.sorted { ($0.symbol, $0.account) < ($1.symbol, $1.account) }.map { p -> [String: Any] in
                ["id": p.id, "symbol": p.symbol, "kind": p.kind, "currency": p.currency, "account": p.account, "exchange": p.exchange,
                 "qty": p.qty, "avg": p.avg, "cost": p.cost, "held": p.held, "alloc": p.alloc, "short": p.short,
                 "dayChange": opt(p.dayChange), "grade": p.grade, "fills": p.fills.sorted { $0.when < $1.when }.map { $0.sub }]
            },
            "positionsSummary": ["count": v.positionsSummary.count, "book": v.positionsSummary.book, "mv": v.positionsSummary.mv, "unreal": v.positionsSummary.unreal] as [String: Any],
            "portfolio": [
                "marketValue": v.portfolio.marketValue, "costBasis": v.portfolio.costBasis, "unrealized": v.portfolio.unrealized, "unrealizedPct": opt(v.portfolio.unrealizedPct),
                "positionCount": v.portfolio.positionCount, "accountCount": v.portfolio.accountCount, "nav": opt(v.portfolio.nav), "navAccounts": v.portfolio.navAccounts,
                "marginUsed": v.portfolio.marginUsed, "marginUsedBy": v.portfolio.marginUsedBy, "marginUsedPct": opt(v.portfolio.marginUsedPct),
                "availableMargin": opt(v.portfolio.availableMargin), "availableMarginUnavailable": v.portfolio.availableMarginUnavailable,
                "hasMargin": v.portfolio.hasMargin, "cash": v.portfolio.cash, "cashPct": opt(v.portfolio.cashPct), "dayChange": opt(v.portfolio.dayChange), "dayChangePct": opt(v.portfolio.dayChangePct),
                "allocation": v.portfolio.allocation.map { ["id": $0.id, "symbol": $0.symbol, "account": $0.account, "value": $0.value, "share": $0.share] as [String: Any] },
            ] as [String: Any],
            "equity": [
                "label": v.equity.label,
                "series": v.equity.series.map { ["d": $0.d, "v": $0.v] as [String: Any] },
                "drawdown": ["pct": opt(v.equity.drawdown.pct), "abs": opt(v.equity.drawdown.abs), "at": v.equity.drawdown.at, "peakAt": v.equity.drawdown.peakAt] as [String: Any],
                "annualized": ["rate": opt(v.equity.annualized.rate), "years": v.equity.annualized.years, "count": v.equity.annualized.count,
                               "first": v.equity.annualized.first, "last": v.equity.annualized.last] as [String: Any],
            ] as [String: Any],
            "years": v.years.map { y -> [String: Any] in
                ["year": y.year, "r": y.r, "days": y.days, "from": y.from, "to": y.to, "flow": opt(y.flow), "endV": opt(y.endV), "spR": opt(y.spR)]
            },
            "benchmark": ["key": v.benchmarkKey, "label": v.benchmarkLabel],
            "monthly": v.monthly.map { ["key": $0.key, "label": $0.label, "value": $0.value, "count": $0.count] as [String: Any] },
            "bySymbol": v.bySymbol.map { ["symbol": $0.symbol, "pnl": $0.pnl, "n": $0.n, "legs": $0.legs, "winRate": $0.winRate, "avgHold": $0.avgHold] as [String: Any] },
            "grades": ["buckets": v.grades.buckets.map { ["grade": $0.grade, "n": $0.n, "pnl": $0.pnl] as [String: Any] },
                       "ungraded": v.grades.ungraded, "graded": v.grades.graded] as [String: Any],
            "queue": v.queue.map { ["id": $0.id, "symbol": $0.symbol, "date": $0.date, "pnl": $0.pnl, "missing": $0.missing] as [String: Any] },
            "options": ["accounts": v.options.accounts, "symbols": v.options.symbols, "tags": v.options.tags, "exchanges": v.options.exchanges,
                        "kinds": v.options.kinds, "years": v.options.years] as [String: Any],
            "cashflowHoldings": cf.holdings.sorted { $0.symbol < $1.symbol }.map { h -> [String: Any] in
                [
                    "symbol": h.symbol, "qty": h.qty, "per": opt(h.per), "freq": h.freq.map { $0 as Any } ?? NSNull(),
                    "freqVerified": h.freqVerified, "annual": opt(h.annual), "yoc": opt(h.yoc), "ytd": h.ytd, "ttm": h.ttm, "all": h.all,
                    "nextExDate": h.nextExDate, "nextPayDate": h.nextPayDate, "exPast": h.exPast, "payPast": h.payPast,
                ]
            },
            "cashflowTiles": cf.tiles.map { t -> [String: Any] in
                var d: [String: Any] = ["label": t.label]
                if let x = t.total { d["total"] = x }
                if let x = t.perMonth { d["perMonth"] = x }
                if let x = t.count { d["count"] = x }
                if t.label == "Margin used" {
                    d["marginUsed"] = t.marginUsed ?? 0
                    d["interestPerMonth"] = t.interestPerMonth ?? 0
                    d["interestMonths"] = t.interestMonths ?? 0
                }
                if t.label == "Yield on cost" {
                    d["yield"] = opt(t.yield)
                    d["projected"] = t.projected ?? 0
                    d["earned"] = t.earned ?? 0
                    d["book"] = t.book ?? 0
                }
                return d
            },
            "cashflowMonths": cf.months.map { ["key": $0.key, "label": $0.label, "value": $0.value, "count": $0.count] as [String: Any] },
            "cashflowTotal": cf.total,
            "cashflowCount": cf.count,
            "cashflowSkipped": cf.skipped,
        ]
    }

    // MARK: comparing

    func diff(_ got: Any, _ want: Any, _ path: String, _ out: inout [String]) {
        if let g = got as? [String: Any] {
            guard let w = want as? [String: Any] else { out.append("\(path): expected \(want), got object"); return }
            for key in Set(g.keys).union(w.keys).sorted() {
                guard let gv = g[key] else { out.append("\(path).\(key): missing on the Swift side"); continue }
                guard let wv = w[key] else { out.append("\(path).\(key): not in the case"); continue }
                diff(gv, wv, "\(path).\(key)", &out)
            }
            return
        }
        if let g = got as? [Any] {
            guard let w = want as? [Any] else { out.append("\(path): expected \(want), got list"); return }
            if g.count != w.count { out.append("\(path): \(w.count) expected, got \(g.count)") }
            for (i, (gv, wv)) in zip(g, w).enumerated() { diff(gv, wv, "\(path)[\(i)]", &out) }
            return
        }
        if got is NSNull {
            if !(want is NSNull) { out.append("\(path): expected \(want), got null") }
            return
        }
        if let g = got as? String {
            if g != (want as? String) { out.append("\(path): expected \(want), got \"\(g)\"") }
            return
        }
        if let g = got as? Bool, type(of: got) == Bool.self {
            if g != ((want as? NSNumber)?.boolValue) { out.append("\(path): expected \(want), got \(g)") }
            return
        }
        if let g = (got as? NSNumber)?.doubleValue {
            guard !(want is NSNull), let w = (want as? NSNumber)?.doubleValue else { out.append("\(path): expected \(want), got \(g)"); return }
            if abs(g - w) > 2e-6 { out.append("\(path): expected \(w), got \(g)") }
            return
        }
        out.append("\(path): cannot compare \(got) with \(want)")
    }

    func testEveryCaseMatches() throws {
        let files = try FileManager.default.contentsOfDirectory(at: Self.casesDir, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "json" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
        XCTAssertFalse(files.isEmpty, "no cases found at \(Self.casesDir.path)")
        for file in files {
            let doc = try JSONSerialization.jsonObject(with: Data(contentsOf: file)) as! [String: Any]
            let snapshot = doc["snapshot"] as! [String: Any]
            let acts = (snapshot["activities"] as! [[String: Any]]).map(activity)
            let secs = ((snapshot["securities"] as? [[String: Any]]) ?? []).map(security)
            let nav = ((snapshot["navHistory"] as? [[String: Any]]) ?? []).map(navPoint)
            var navByAccount: [String: [BHNavPoint]] = [:]
            for (nick, pts) in (snapshot["navByAccount"] as? [String: Any]) ?? [:] {
                navByAccount[nick] = ((pts as? [[String: Any]]) ?? []).map(navPoint)
            }
            let accounts = ((snapshot["accounts"] as? [[String: Any]]) ?? []).map { a in
                BHAccountInfo(id: str(a, "id"), name: str(a, "nickname"), currency: str(a, "currency"), nav: numOpt(a, "netLiquidationValue"),
                              type: str(a, "unifiedAccountType"), status: str(a, "status"))
            }
            let balances = ((snapshot["balances"] as? [[String: Any]]) ?? []).map { b in
                BHBalanceRow(accountId: str(b, "accountId"), securityId: str(b, "securityId"), quantity: num(b, "quantity"))
            }
            let margin = ((snapshot["margin"] as? [[String: Any]]) ?? []).map { m in
                BHMarginRow(accountId: str(m, "accountId"), buyingPower: numOpt(m, "buyingPower"), currency: str(m, "currency"), unavailable: str(m, "unavailable"))
            }
            let base = BHModel.buildBase(activities: acts, securities: secs, market: market(doc["market"] as! [String: Any]), today: doc["today"] as! String,
                                         navHistory: nav, navByAccount: navByAccount, journal: journal((doc["journal"] as? [String: Any]) ?? [:]),
                                         accounts: accounts, balances: balances, margin: margin)
            let view = BHModel.buildView(base, BHFilters(json: (doc["filters"] as? [String: Any]) ?? [:]))
            var problems: [String] = []
            diff(expect(view), doc["expect"] as! [String: Any], file.lastPathComponent, &problems)
            XCTAssertTrue(problems.isEmpty, problems.joined(separator: "\n"))
        }
    }

    func testDateArithmetic() {
        XCTAssertEqual(BHModel.daysBetween("2026-01-10", "2026-02-10"), 31)
        XCTAssertEqual(BHModel.daysBetween("2026-02-10", "2026-01-10"), 0)
        XCTAssertEqual(BHModel.shiftDate("2026-03-01", -1), "2026-02-28")
        XCTAssertEqual(BHModel.shiftDate("2024-02-28", 1), "2024-02-29")
        XCTAssertEqual(BHModel.shiftDate("2025-12-31", 1), "2026-01-01")
        XCTAssertEqual(BHModel.optionExpiry("LUNR 29AUG25 11.50 CALL"), "2025-08-29")
        XCTAssertEqual(BHModel.optionExpiry("BBAI 02JAN26 5.50 PUT"), "2026-01-02")
        XCTAssertEqual(BHModel.optionExpiry("AAPL"), "")
    }
}
