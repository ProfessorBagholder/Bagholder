// Market data on the phone: what the desktop's crates/market fetches server-side,
// fetched here from the same sources with the same rules, and kept in the
// app's files. Every request records its outcome, and that is what a chart's
// empty state reports. Nothing is hand-mapped per ticker.
import Foundation

/// A daily bar as the sources answer it.
struct BHDailyBar: Equatable {
    var date: String
    var open: Double?, high: Double?, low: Double?, close: Double, volume: Double?
}

/// What a held instrument needs for a quote or a chart.
struct BHInstrument {
    var symbol: String
    var exchange: String
    var currency: String
    var kind: String
}

enum MarketError: Error {
    case http(Int)
    case backoff
    case badBody
}

enum MarketData {
    static let tmxURL = URL(string: "https://app-money.tmx.com/graphql")!
    static let tmxQuoteQuery = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name exchangeName price priceChange percentChange prevClose currency dividendFrequency dividendYield dividendAmount exDividendDate } }"
    static let tmxDividendsQuery = "query getDividendsForSymbol($symbol: String!, $page: Int, $batch: Int) { dividends: getDividendsForSymbol(symbol: $symbol, page: $page, batch: $batch) { dividends { exDate payableDate amount currency } } }"
    static let tmxHistoryQuery = "query getTimeSeriesData($symbol: String!, $freq: String, $interval: Int, $start: String, $end: String) { getTimeSeriesData(symbol: $symbol, freq: $freq, interval: $interval, start: $start, end: $end) { dateTime open high low close volume } }"
    static let tmxHeaders = ["locale": "en", "Origin": "https://money.tmx.com", "Referer": "https://money.tmx.com/"]
    static let tmxBatch = 24
    static let coinbaseSpot = "https://api.coinbase.com/v2/prices/%@/spot"
    static let coinbaseProduct = "https://api.exchange.coinbase.com/products/%@"
    static let coinbaseCandles = "https://api.exchange.coinbase.com/products/%@/candles?granularity=%d&start=%@&end=%@"
    static let cboeCA = "https://www-api.cboe.com/ca/equities/securities-1/%@/quote/"
    static let cboeOptions = "https://cdn.cboe.com/api/global/delayed_quotes/options/%@.json"
    static let yahooChart = "https://query1.finance.yahoo.com/v8/finance/chart/%@?period1=%d&period2=%d&interval=%@"
    static let yahooHeaders = ["User-Agent": "Mozilla/5.0", "Accept": "application/json"]
    static let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
    static let usExchanges: Set<String> = ["NASDAQ", "NYSE", "NYSE AMERICAN", "NYSE ARCA", "BATS", "AMEX", "ARCA", "CBOE", "IEX"]
    static let cboeCanadaExchanges: Set<String> = ["CBOE CANADA", "NEO"]
    static let canadianExchanges: Set<String> = ["TSX", "TSX-V", "TSXV", "CSE", "CBOE CANADA", "NEO", "ALPHA EXCHANGE"]
    static let tmxForms = ["CAD": ["", ":CNX", ":AQL"], "USD": [":US"]]
    static let tmxVenueOfForm = ["": ["TORONTO STOCK EXCHANGE", "TSX VENTURE"], ":CNX": ["CANADIAN SECURITIES EXCHANGE"], ":AQL": ["CBOE", "NEO"], ":US": ["NYSE", "NASDAQ", "NEW YORK"]]
    static let tmxIndices = ["TSX": "^TSX", "TSX60": "^TX60"]
    static let yahooSuffix = ["TSX": ".TO", "TSX-V": ".V", "TSXV": ".V", "CSE": ".CN", "CBOE CANADA": ".NE", "NEO": ".NE"]
    static let yahooForms = ["CAD": [".TO", ".V", ".CN", ".NE"], "USD": [""]]
    static let sourceLabels = ["tmx": "TMX Money", "yahoo": "Yahoo Finance", "coinbase": "Coinbase", "cboe": "Cboe", "boc": "Bank of Canada", "fred": "FRED"]
    static let coverageSlackDays = 7
    static let quoteRefreshMinutes = 1.0
    static let recordStaleHours = 20.0
    static let historyStaleHours = 20.0
    static let yahooMinIntervalSec = 2.0
    static let yahooBackoffSec = 600.0

    static var dir: URL { AppFiles.dir }

    // MARK: files

    private static let lock = NSLock()

    private static func readJSON(_ name: String) -> [String: Any] {
        lock.lock(); defer { lock.unlock() }
        guard let data = try? Data(contentsOf: dir.appendingPathComponent(name)), let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        return obj
    }

    private static func writeJSON(_ name: String, _ obj: [String: Any]) {
        lock.lock(); defer { lock.unlock() }
        if let data = try? JSONSerialization.data(withJSONObject: obj) { try? data.write(to: dir.appendingPathComponent(name), options: .atomic) }
    }

    private static func meta(_ key: String) -> String { (readJSON("meta.json")[key] as? String) ?? "" }

    private static func setMeta(_ key: String, _ value: String) {
        var m = readJSON("meta.json")
        m[key] = value
        writeJSON("meta.json", m)
    }

    static func nowStamp() -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f.string(from: Date())
    }

    static func age(_ stamp: String) -> TimeInterval? {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        guard let d = f.date(from: stamp) else { return nil }
        return Date().timeIntervalSince(d)
    }

    static func today() -> String { BHModel.todayLocal() }

    // MARK: source outcomes

    struct Outcome { var ok: Bool; var at: String; var error: String }
    private static var health: [String: Outcome] = [:]
    private static var chartNotes: [String: [(String, String, String?)]] = [:]   // symbol -> [(source, key, failure or nil)]

    static func sourceOfURL(_ url: URL) -> String {
        let host = (url.host ?? "").lowercased()
        for (key, needle) in [("tmx", "tmx.com"), ("yahoo", "yahoo.com"), ("coinbase", "coinbase.com"), ("cboe", "cboe.com"), ("boc", "bankofcanada.ca"), ("fred", "stlouisfed.org")] {
            if host.contains(needle) { return key }
        }
        return host.isEmpty ? "other" : host
    }

    static func describeFailure(_ e: Error) -> String {
        if case MarketError.http(let code) = e {
            if code == 429 { return "refused the request (too many)" }
            return "answered with an error (\(code))"
        }
        if case MarketError.backoff = e { return "refused the request; asked again in ten minutes" }
        return "could not be reached"
    }

    static func note(_ source: String, ok: Bool, error: Error? = nil) {
        lock.lock(); defer { lock.unlock() }
        health[source] = Outcome(ok: ok, at: nowStamp(), error: ok ? "" : describeFailure(error ?? MarketError.badBody))
    }

    // MARK: HTTP

    private static func request(_ url: URL, method: String = "GET", body: [String: Any]? = nil, headers: [String: String] = [:]) async throws -> Data {
        var req = URLRequest(url: url, timeoutInterval: 30)
        req.httpMethod = method
        req.setValue(ua, forHTTPHeaderField: "User-Agent")
        req.setValue(body == nil ? "text/csv,application/json,*/*;q=0.8" : "*/*", forHTTPHeaderField: "Accept")
        for (k, v) in headers { req.setValue(v, forHTTPHeaderField: k) }
        if let body {
            req.httpBody = try JSONSerialization.data(withJSONObject: body)
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        }
        let source = sourceOfURL(url)
        do {
            let (data, resp) = try await URLSession.shared.data(for: req)
            let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
            if status >= 400 {
                let e = MarketError.http(status)
                if status != 404 { note(source, ok: false, error: e) }   // a symbol a source does not carry is not the source failing
                throw e
            }
            note(source, ok: true)
            return data
        } catch let e as MarketError {
            throw e
        } catch {
            note(source, ok: false, error: error)
            throw error
        }
    }

    private static func getText(_ url: String, headers: [String: String] = [:]) async throws -> String {
        guard let u = URL(string: url) else { throw MarketError.badBody }
        return String(decoding: try await request(u, headers: headers), as: UTF8.self)
    }

    private static func postJSON(_ url: URL, _ payload: [String: Any], headers: [String: String]) async throws -> [String: Any] {
        let data = try await request(url, method: "POST", body: payload, headers: headers)
        return (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }

    private static func tmx(_ operation: String, _ variables: [String: Any], _ query: String) async throws -> [String: Any] {
        try await postJSON(tmxURL, ["operationName": operation, "variables": variables, "query": query], headers: tmxHeaders)
    }

    static func num(_ v: Any?) -> Double? {
        if let n = v as? NSNumber { let d = n.doubleValue; return d.isNaN ? nil : d }
        if let s = v as? String { return Double(s) }
        return nil
    }

    // MARK: TMX symbol forms

    /// Wealthsimple's Canadian tickers already match TMX Money's (no suffix).
    static func tmxSymbol(_ symbol: String) -> String {
        var s = symbol.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        for suffix in [".TO", ".V", ".CN", ".NE"] where s.hasSuffix(suffix) { s = String(s.dropLast(suffix.count)) }
        return s
    }

    /// TMX's symbol suffix for a listing venue, or nil when TMX does not carry it.
    static func tmxForm(exchange: String, currency: String) -> String? {
        let ex = exchange.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        let ccy = currency.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        if usExchanges.contains(ex) || (ex.isEmpty && ccy == "USD") { return ":US" }
        if cboeCanadaExchanges.contains(ex) { return ":AQL" }
        if ex == "CSE" { return ":CNX" }
        if ex == "TSX" || ex == "TSX-V" || ex == "TSXV" { return "" }
        if ccy == "CAD" { return "" }
        if ccy == "USD" { return ":US" }
        return nil
    }

    static func tmxQuoteSymbol(_ i: BHInstrument) -> String? {
        let s = tmxSymbol(i.symbol)
        if s.isEmpty || s.contains(" ") { return nil }
        guard let form = tmxForm(exchange: i.exchange, currency: i.currency) else { return nil }
        return s + form
    }

    static func tmxRecordSymbol(_ symbol: String, exchange: String) -> String? {
        let s = tmxSymbol(symbol)
        if s.isEmpty { return nil }
        guard let form = tmxForm(exchange: exchange, currency: "CAD") else { return nil }
        return s + form
    }

    static func tmxBare(_ key: String) -> String { String(key.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false).first ?? "") }

    static func tmxRemembered(_ key: String) -> String {
        if key.isEmpty || key.hasPrefix("^") { return key }
        let v = meta("tmx_form:" + tmxBare(key))
        return v.hasPrefix("@") ? tmxBare(key) + v.dropFirst() : key
    }

    /// Which of TMX's forms of a symbol answers, checked by the venue its quote
    /// names; remembered for good, and a miss remembered for a day.
    static func tmxResolve(_ key: String) async -> String {
        if key.isEmpty || key.hasPrefix("^") { return key }
        let bare = tmxBare(key)
        let suffix = String(key.dropFirst(bare.count))
        var forms = suffix == ":US" ? tmxForms["USD"]! : tmxForms["CAD"]!
        if forms.contains(suffix) { forms = [suffix] + forms.filter { $0 != suffix } }
        let metaKey = "tmx_form:" + bare
        let v = meta(metaKey)
        if v.hasPrefix("@") { return bare + v.dropFirst() }
        if v.hasPrefix("none@"), String(v.dropFirst(5)) > BHModel.shiftDate(today(), -1) { return "" }
        for form in forms {
            let cand = bare + form
            let q = ((try? await tmx("getQuoteBySymbol", ["symbol": cand, "locale": "en"], tmxQuoteQuery))?["data"] as? [String: Any])?["getQuoteBySymbol"] as? [String: Any] ?? [:]
            let venue = ((q["exchangeName"] as? String) ?? "").uppercased()
            if !venue.isEmpty, (tmxVenueOfForm[form] ?? []).contains(where: { venue.contains($0) }) {
                setMeta(metaKey, "@" + form)
                return cand
            }
        }
        setMeta(metaKey, "none@" + today())
        return ""
    }

    /// The remembered or given form first; when it answers nothing, the form TMX resolves.
    static func tmxLookup<T>(_ key: String, _ fn: (String) async -> T?) async -> (T?, String) {
        let first = tmxRemembered(key)
        if let r = await fn(first) { return (r, first) }
        if key.isEmpty || key.hasPrefix("^") { return (nil, first) }
        let alt = await tmxResolve(key)
        if !alt.isEmpty && alt != first { return (await fn(alt), alt) }
        return (nil, first)
    }

    // MARK: quotes

    static func parseTMXQuote(_ data: [String: Any]) -> BHQuote? {
        guard let q = (data["data"] as? [String: Any])?["getQuoteBySymbol"] as? [String: Any], !q.isEmpty else { return nil }
        var out = BHQuote()
        out.price = num(q["price"])
        out.priceChange = num(q["priceChange"])
        out.percentChange = num(q["percentChange"])
        out.exDividendDate = String(((q["exDividendDate"] as? String) ?? "").prefix(10))
        out.fetchedAt = nowStamp()
        return out
    }

    static func tmxQuote(_ key: String) async -> BHQuote? {
        guard let data = try? await tmx("getQuoteBySymbol", ["symbol": key, "locale": "en"], tmxQuoteQuery) else { return nil }
        let q = parseTMXQuote(data)
        return q?.price == nil ? nil : q
    }

    static func cboeCAQuote(_ sym: String) async -> BHQuote? {
        guard let text = try? await getText(String(format: cboeCA, sym)), let d = (try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any])?["data"] as? [String: Any] else { return nil }
        let last = num(d["last"]) ?? 0
        let prev = num(d["prev_close"])
        let px = last > 0 ? last : (prev ?? 0)
        if px <= 0 { return nil }
        return BHQuote(price: px, priceChange: num(d["change"]), percentChange: num(d["change_pct"]), fetchedAt: nowStamp(), exDividendDate: "")
    }

    static func coinbaseSpotQuote(_ pair: String) async -> BHQuote? {
        guard let text = try? await getText(String(format: coinbaseSpot, pair)), let d = (try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any])?["data"] as? [String: Any] else { return nil }
        guard let px = num(d["amount"]), px > 0 else { return nil }
        return BHQuote(price: px, priceChange: nil, percentChange: nil, fetchedAt: nowStamp(), exDividendDate: "")
    }

    /// 'QNC 20NOV26 3.00 CALL' -> 'QNC261120C00003000' (the OCC code Cboe keys its chains by).
    static func occCode(_ symbol: String) -> String {
        let u = BHModel.reSub("\\s+", symbol.trimmingCharacters(in: .whitespacesAndNewlines).uppercased(), " ")
        if let m = BHModel.re("^([A-Z][A-Z0-9.]{0,9}) (\\d{6}[CP]\\d{8})$").firstMatch(in: u, range: NSRange(u.startIndex..., in: u)) {
            return String(u[Range(m.range(at: 1), in: u)!]) + String(u[Range(m.range(at: 2), in: u)!])
        }
        guard let m = BHModel.re("^([A-Z][A-Z0-9.]{0,9}) (\\d{1,2})([A-Z]{3})(\\d{2}) (\\d+(?:\\.\\d+)?) (CALL|PUT|C|P)$").firstMatch(in: u, range: NSRange(u.startIndex..., in: u)) else { return "" }
        func g(_ i: Int) -> String { String(u[Range(m.range(at: i), in: u)!]) }
        guard let mi = BHModel.months.map({ $0.uppercased() }).firstIndex(of: g(3)), let day = Int(g(2)), let strike = Double(g(5)) else { return "" }
        return g(1) + g(4) + String(format: "%02d%02d", mi + 1, day) + String(g(6).prefix(1)) + String(format: "%08d", Int((strike * 1000).rounded()))
    }

    static func occRoot(_ code: String) -> String { BHModel.reGroup("^([A-Z][A-Z0-9.]{0,9})\\d{6}[CP]\\d{8}$", code) ?? "" }

    static func optionMark(_ row: [String: Any]) -> BHQuote? {
        let bid = num(row["bid"]) ?? 0, ask = num(row["ask"]) ?? 0
        let prev = num(row["prev_day_close"])
        var px: Double?
        if bid > 0 && ask > 0 { px = (bid + ask) / 2 } else { px = num(row["last_trade_price"]) ?? prev }
        guard let p = px, p > 0 else { return nil }
        return BHQuote(price: p, priceChange: prev.map { p - $0 }, percentChange: prev.map { (p / $0 - 1) * 100 }, fetchedAt: nowStamp(), exDividendDate: "")
    }

    /// (source, key) for a held instrument, or nil when no public source covers it.
    static func quoteSource(_ i: BHInstrument) -> (String, String)? {
        let sym = tmxSymbol(i.symbol)
        let ccy = i.currency.isEmpty ? "CAD" : i.currency.uppercased()
        if sym.isEmpty { return nil }
        if i.kind == "Crypto" { return ("coinbase", sym + "-" + ccy) }
        if i.kind == "Options" {
            let code = occCode(i.symbol)
            return (!code.isEmpty && ccy == "USD") ? ("cboe_options", code) : nil
        }
        if i.kind != "Shares" { return nil }
        if cboeCanadaExchanges.contains(i.exchange.uppercased()) { return ("cboe_ca", sym) }
        if let q = tmxQuoteSymbol(i) { return ("tmx", q) }
        return nil
    }

    static func loadQuotes() -> [String: BHQuote] {
        var out: [String: BHQuote] = [:]
        for (sym, v) in readJSON("quotes.json") {
            let d = (v as? [String: Any]) ?? [:]
            out[sym] = BHQuote(price: num(d["price"]), priceChange: num(d["priceChange"]), percentChange: num(d["percentChange"]), fetchedAt: (d["fetchedAt"] as? String) ?? "", exDividendDate: (d["exDividendDate"] as? String) ?? "")
        }
        return out
    }

    private static func saveQuotes(_ quotes: [String: BHQuote]) {
        var obj: [String: Any] = [:]
        for (sym, q) in quotes {
            obj[sym] = ["price": q.price as Any, "priceChange": q.priceChange as Any, "percentChange": q.percentChange as Any, "fetchedAt": q.fetchedAt, "exDividendDate": q.exDividendDate]
        }
        writeJSON("quotes.json", obj)
    }

    /// Live-ish prices for held positions, at most every minute each.
    static func refreshQuotes(_ instruments: [BHInstrument]) async -> [String: BHQuote] {
        var quotes = loadQuotes()
        var chains: [String: [String: [String: Any]]] = [:]
        var seen = Set<String>()
        for i in instruments {
            let sym = tmxSymbol(i.symbol)
            guard !sym.isEmpty, !seen.contains(sym), let (source, key) = quoteSource(i) else { continue }
            seen.insert(sym)
            if let old = quotes[sym], let a = age(old.fetchedAt), a < quoteRefreshMinutes * 60 { continue }
            var q: BHQuote?
            switch source {
            case "tmx": q = await tmxLookup(key, tmxQuote).0
            case "cboe_ca": q = await cboeCAQuote(key)
            case "coinbase": q = await coinbaseSpotQuote(key)
            case "cboe_options":
                let root = occRoot(key)
                if chains[root] == nil {
                    var chain: [String: [String: Any]] = [:]
                    if let text = try? await getText(String(format: cboeOptions, root)), let d = (try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any])?["data"] as? [String: Any] {
                        for o in (d["options"] as? [[String: Any]]) ?? [] { chain[(o["option"] as? String) ?? ""] = o }
                    }
                    chains[root] = chain
                }
                q = chains[root]?[key].flatMap(optionMark)
            default: q = nil
            }
            if var got = q, got.price != nil {
                got.exDividendDate = got.exDividendDate.isEmpty ? (quotes[sym]?.exDividendDate ?? "") : got.exDividendDate
                quotes[sym] = got
            }
        }
        saveQuotes(quotes)
        return quotes
    }

    // MARK: declared distribution records

    static func distributions() -> [String: [BHDistribution]] {
        var out: [String: [BHDistribution]] = [:]
        for (sym, rows) in readJSON("distributions.json") {
            out[sym] = ((rows as? [[String: Any]]) ?? []).map {
                BHDistribution(exDate: ($0["exDate"] as? String) ?? "", payDate: ($0["payDate"] as? String) ?? "", amount: num($0["amount"]) ?? 0, currency: ($0["currency"] as? String) ?? "")
            }
        }
        return out
    }

    static func isCanadianListing(exchange: String, currency: String) -> Bool {
        let ex = exchange.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        if !ex.isEmpty { return canadianExchanges.contains(ex) }
        return currency.uppercased() == "CAD"
    }

    /// The declared records of the Canadian dividend payers whose copy is older than 20 hours.
    static func refreshDistributions(_ payers: [BHInstrument], force: Bool = false) async {
        var stamps = readJSON("distributions_fetched.json")
        var records = readJSON("distributions.json")
        var quotes = loadQuotes()
        for i in payers {
            let sym = tmxSymbol(i.symbol)
            guard !sym.isEmpty, isCanadianListing(exchange: i.exchange, currency: i.currency) else { continue }
            if !force, let a = age((stamps[sym] as? String) ?? ""), a < recordStaleHours * 3600 { continue }
            guard let recSym = tmxRecordSymbol(sym, exchange: i.exchange) else { continue }
            let (quote, form) = await tmxLookup(recSym, tmxQuote)
            var rows: [[String: Any]] = []
            if let data = try? await tmx("getDividendsForSymbol", ["symbol": form, "page": 1, "batch": tmxBatch], tmxDividendsQuery),
               let block = (data["data"] as? [String: Any])?["dividends"] as? [String: Any], let list = block["dividends"] as? [[String: Any]] {
                for r in list {
                    let ex = String(((r["exDate"] as? String) ?? "").prefix(10))
                    guard ex.count == 10, let amt = num(r["amount"]), amt > 0 else { continue }
                    rows.append(["exDate": ex, "payDate": String(((r["payableDate"] as? String) ?? "").prefix(10)), "amount": amt, "currency": (r["currency"] as? String) ?? ""])
                }
            }
            // A Cboe Canada listing's price comes from Cboe's own feed; TMX's delayed
            // quote for it must not replace that, only its record is kept.
            if let q = quote, !cboeCanadaExchanges.contains(i.exchange.uppercased()) { quotes[sym] = q }
            if !rows.isEmpty { records[sym] = rows }
            if quote != nil || !rows.isEmpty { stamps[sym] = nowStamp() }
        }
        writeJSON("distributions.json", records)
        writeJSON("distributions_fetched.json", stamps)
        saveQuotes(quotes)
    }

    // MARK: the indexes

    static func indexCloses() -> [String: [String: Double]] {
        var out: [String: [String: Double]] = [:]
        for (k, v) in readJSON("indexes.json") {
            var m: [String: Double] = [:]
            for (d, c) in (v as? [String: Any]) ?? [:] { if let n = num(c) { m[d] = n } }
            out[k] = m
        }
        return out
    }

    /// The S&P/TSX Composite and the TSX 60 from TMX Money, appended from a week before the newest stored day.
    static func refreshIndexes() async {
        var all = readJSON("indexes.json")
        if let a = age(meta("indexes_fetched")), a < 6 * 3600 { return }
        for (key, sym) in tmxIndices {
            var have = (all[key] as? [String: Any]) ?? [:]
            let last = have.keys.max()
            let start = last.map { BHModel.shiftDate($0, -7) } ?? "2016-01-01"
            guard let data = try? await tmx("getTimeSeriesData", ["symbol": sym, "freq": "day", "interval": 1, "start": start, "end": today()], tmxHistoryQuery) else { continue }
            for b in parseTMXHistory(data) { have[b.date] = b.close }
            all[key] = have
        }
        writeJSON("indexes.json", all)
        setMeta("indexes_fetched", nowStamp())
    }

    // MARK: bars

    static func parseTMXHistory(_ data: [String: Any]) -> [BHDailyBar] {
        let rows = (data["data"] as? [String: Any])?["getTimeSeriesData"] as? [[String: Any]] ?? []
        var out: [BHDailyBar] = []
        for r in rows {
            let d = String(((r["dateTime"] as? String) ?? "").prefix(10))
            guard d.count == 10, let close = num(r["close"]) else { continue }
            out.append(BHDailyBar(date: d, open: num(r["open"]), high: num(r["high"]), low: num(r["low"]), close: close, volume: num(r["volume"])))
        }
        return out.sorted { $0.date < $1.date }
    }

    static func wholeBars(_ bars: [BHDailyBar]) -> [BHDailyBar] { bars.filter { $0.open != nil && $0.high != nil && $0.low != nil } }

    /// What the trade chart draws: the instrument, or an option's underlying stock.
    static func chartInstrument(_ i: BHInstrument) -> BHInstrument {
        if i.kind == "Options" {
            let under = BHModel.underlyingSymbol(i.symbol)
            if !under.isEmpty && under != "—" { return BHInstrument(symbol: under, exchange: i.exchange, currency: i.currency.isEmpty ? "USD" : i.currency, kind: "Shares") }
        }
        return i
    }

    static func yahooForms(_ i: BHInstrument) -> [String] {
        let root = tmxSymbol(i.symbol).replacingOccurrences(of: ".", with: "-")
        let ccy = i.currency.isEmpty ? "CAD" : i.currency.uppercased()
        guard !root.isEmpty, !root.contains(" "), var forms = yahooForms[ccy] else { return [] }
        if let first = yahooSuffix[i.exchange.uppercased()], forms.contains(first) { forms = [first] + forms.filter { $0 != first } }
        return forms.map { root + $0 }
    }

    /// Where an instrument's bars can come from, in order of preference.
    static func historyCandidates(_ i: BHInstrument) -> [(String, String)] {
        let sym = tmxSymbol(i.symbol)
        let ccy = i.currency.isEmpty ? "CAD" : i.currency.uppercased()
        if sym.isEmpty { return [] }
        if i.kind == "Crypto" {
            var out = [("coinbase", sym + "-" + ccy), ("yahoo", sym + "-" + ccy)]
            if ccy != "USD" { out += [("coinbase", sym + "-USD"), ("yahoo", sym + "-USD")] }
            return out
        }
        if i.kind != "Shares" { return [] }
        var out: [(String, String)] = []
        if let k = tmxQuoteSymbol(i) { out.append(("tmx", k)) }
        out += yahooForms(i).map { ("yahoo", $0) }
        return out
    }

    static func orderedCandidates(_ i: BHInstrument) -> [(String, String)] {
        let cands = historyCandidates(i)
        let v = cands.isEmpty ? "" : meta("bars_source:" + tmxSymbol(i.symbol))
        if let bar = v.firstIndex(of: "|") {
            let win = (String(v[..<bar]), String(v[v.index(after: bar)...]))
            if cands.contains(where: { $0 == win }) { return [win] + cands.filter { $0 != win } }
        }
        return cands
    }

    static func barCurrency(_ source: String, _ key: String, _ i: BHInstrument) -> String {
        if i.kind == "Crypto", let dash = key.firstIndex(of: "-") { return String(key[key.index(after: dash)...]) }
        return i.currency.isEmpty ? "CAD" : i.currency.uppercased()
    }

    /// Bars in the position's currency: USD bars into CAD at the Bank of Canada rate of the bar's day.
    static func inPositionCurrency(_ bars: [BHDailyBar], _ barCcy: String, _ currency: String) -> [BHDailyBar] {
        let quote = barCcy.uppercased(), ccy = (currency.isEmpty ? "CAD" : currency).uppercased()
        if quote == ccy { return bars }
        guard quote == "USD", ccy == "CAD" else { return [] }
        let fx = WSPull.loadCachedFx()
        var out: [BHDailyBar] = []
        for b in bars {
            var rate: Double?
            for i in 0..<7 { if let r = fx[BHModel.shiftDate(b.date, -i)], r > 0 { rate = r; break } }
            guard let r = rate else { continue }
            out.append(BHDailyBar(date: b.date, open: b.open.map { $0 * r }, high: b.high.map { $0 * r }, low: b.low.map { $0 * r }, close: b.close * r, volume: b.volume))
        }
        return out
    }

    private static var yahooNextAt: TimeInterval = 0
    private static var yahooBackoffUntil: TimeInterval = 0

    /// One Yahoo request at a time, spaced two seconds apart; after a 429 nothing is asked for ten minutes.
    static func yahooGet(_ url: String) async throws -> String {
        let now = Date().timeIntervalSince1970
        if now < yahooBackoffUntil { note("yahoo", ok: false, error: MarketError.backoff); throw MarketError.backoff }
        let wait = yahooNextAt - now
        if wait > 0 { try? await Task.sleep(nanoseconds: UInt64(wait * 1e9)) }
        yahooNextAt = Date().timeIntervalSince1970 + yahooMinIntervalSec
        do {
            return try await getText(url, headers: yahooHeaders)
        } catch MarketError.http(let code) {
            if code == 429 { yahooBackoffUntil = Date().timeIntervalSince1970 + yahooBackoffSec }
            throw MarketError.http(code)
        }
    }

    static func parseYahooDaily(_ text: String) -> [BHDailyBar] {
        guard let d = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any],
              let r = ((d["chart"] as? [String: Any])?["result"] as? [[String: Any]])?.first else { return [] }
        let ts = (r["timestamp"] as? [Any]) ?? []
        let q = (((r["indicators"] as? [String: Any])?["quote"] as? [[String: Any]]) ?? [[:]]).first ?? [:]
        let meta = (r["meta"] as? [String: Any]) ?? [:]
        let tz = TimeZone(identifier: (meta["exchangeTimezoneName"] as? String) ?? "") ?? TimeZone(secondsFromGMT: Int(num(meta["gmtoffset"]) ?? 0))!
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = tz
        f.dateFormat = "yyyy-MM-dd"
        func col(_ k: String, _ i: Int) -> Double? { let a = (q[k] as? [Any]) ?? []; return i < a.count ? num(a[i]) : nil }
        var out: [BHDailyBar] = []
        for (i, t) in ts.enumerated() {
            guard let sec = num(t), let close = col("close", i), close > 0 else { continue }
            let day = f.string(from: Date(timeIntervalSince1970: sec))
            out.append(BHDailyBar(date: day, open: col("open", i), high: col("high", i), low: col("low", i), close: close, volume: col("volume", i)))
        }
        return out.sorted { $0.date < $1.date }
    }

    static func fetchYahooDaily(_ symbol: String, _ start: String, _ end: String) async throws -> [BHDailyBar] {
        let missKey = "yahoo_miss:" + symbol
        if meta(missKey) == today() { return [] }
        let s = Int(BHModel.dayNumber(start) ?? 0) * 86400, e = (Int(BHModel.dayNumber(end) ?? 0) + 1) * 86400
        do {
            return parseYahooDaily(try await yahooGet(String(format: yahooChart, symbol, s, e, "1d")))
        } catch MarketError.http(let code) where code == 404 {
            setMeta(missKey, today())
            return []
        }
    }

    static func coinbaseMarket(_ pair: String) async -> String {
        let p = pair.uppercased()
        guard p.contains("-") else { return "" }
        let key = "coinbase_product:" + p
        let v = meta(key)
        if v.hasPrefix("@") { return String(v.dropFirst()) }
        if v.hasPrefix("none@"), String(v.dropFirst(5)) > BHModel.shiftDate(today(), -1) { return "" }
        if let text = try? await getText(String(format: coinbaseProduct, p)), let d = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any], ((d["id"] as? String) ?? "").uppercased() == p {
            setMeta(key, "@" + p)
            return p
        }
        setMeta(key, "none@" + today())
        return ""
    }

    static func fetchCoinbaseDaily(_ product: String, _ start: String, _ end: String) async throws -> [BHDailyBar] {
        let s = Int(BHModel.dayNumber(start) ?? 0) * 86400, e = (Int(BHModel.dayNumber(end) ?? 0) + 1) * 86400
        let span = 300 * 86400
        var out: [Int: BHDailyBar] = [:]
        var cur = s
        let iso = ISO8601DateFormatter()
        iso.formatOptions = [.withInternetDateTime]
        while cur < e {
            let to = min(cur + span, e)
            let text = try await getText(String(format: coinbaseCandles, product, 86400, iso.string(from: Date(timeIntervalSince1970: Double(cur))), iso.string(from: Date(timeIntervalSince1970: Double(to)))))
            for r in (try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [[Any]]) ?? [] where r.count >= 6 {
                guard let t = num(r[0]), let lo = num(r[1]), let hi = num(r[2]), let op = num(r[3]), let cl = num(r[4]), cl > 0 else { continue }
                out[Int(t)] = BHDailyBar(date: BHModel.isoDate(fromDayNumber: Int(t) / 86400), open: op, high: hi, low: lo, close: cl, volume: num(r[5]))
            }
            cur += span
        }
        return out.keys.sorted().map { out[$0]! }
    }

    /// Daily bars of one candidate over [start, end], oldest first; [] when it has none.
    static func fetchDailyFrom(_ source: String, _ key: String, _ i: BHInstrument, _ start: String, _ end: String) async throws -> [BHDailyBar] {
        switch source {
        case "tmx":
            var failure: Error?
            let (bars, _) = await tmxLookup(key) { form -> [BHDailyBar]? in
                do {
                    let data = try await tmx("getTimeSeriesData", ["symbol": form, "freq": "day", "interval": 1, "start": start, "end": end], tmxHistoryQuery)
                    let b = parseTMXHistory(data)
                    return b.isEmpty ? nil : b
                } catch {
                    failure = error
                    return nil
                }
            }
            if let b = bars { return wholeBars(b) }
            if let f = failure { throw f }
            return []
        case "coinbase":
            let product = await coinbaseMarket(key)
            if product.isEmpty { return [] }
            return inPositionCurrency(try await fetchCoinbaseDaily(product, start, end), barCurrency(source, key, i), i.currency)
        case "yahoo":
            return inPositionCurrency(wholeBars(try await fetchYahooDaily(key, start, end)), barCurrency(source, key, i), i.currency)
        default:
            return []
        }
    }

    /// Daily bars for one instrument between two dates from the chain: the first
    /// candidate whose bars cover the span, else the one covering most of it; the
    /// winner is remembered. On an empty answer the notes say why.
    static func fetchHistory(_ i: BHInstrument, _ start: String, _ end: String) async -> ([BHDailyBar], String) {
        let spanStart = BHModel.dayNumber(start) ?? 0
        var answers: [(String, String, [BHDailyBar])] = []
        var notes: [(String, String, String?)] = []
        for (source, key) in orderedCandidates(i) {
            var bars: [BHDailyBar] = []
            do {
                bars = try await fetchDailyFrom(source, key, i, start, end)
                notes.append((source, key, nil))
            } catch {
                notes.append((source, key, describeFailure(error)))
            }
            answers.append((source, key, bars))
            if let first = bars.first, (BHModel.dayNumber(first.date) ?? 0) <= spanStart + coverageSlackDays { break }
        }
        var best: (Int, String, String, [BHDailyBar])?
        for (source, key, bars) in answers {
            guard let first = bars.first else { continue }
            let f = BHModel.dayNumber(first.date) ?? 0
            if f <= spanStart + coverageSlackDays {
                setMeta("bars_source:" + tmxSymbol(i.symbol), source + "|" + key)
                return (bars, source)
            }
            if best == nil || f < best!.0 { best = (f, source, key, bars) }
        }
        if let b = best {
            setMeta("bars_source:" + tmxSymbol(i.symbol), b.1 + "|" + b.2)
            return (b.3, b.1)
        }
        lock.lock(); chartNotes[tmxSymbol(i.symbol)] = notes; lock.unlock()
        return ([], "")
    }

    /// Why a chart has no bars, in one sentence for the page.
    static func chartReason(_ i: BHInstrument) -> String {
        lock.lock(); let notes = chartNotes[tmxSymbol(i.symbol)] ?? []; lock.unlock()
        var failed: [String] = []
        for (source, _, failure) in notes {
            if let f = failure {
                let line = (sourceLabels[source] ?? source) + " " + f
                if !failed.contains(line) { failed.append(line) }
            }
        }
        if !failed.isEmpty { return failed.joined(separator: "; ") + "." }
        var names: [String] = []
        for (source, _) in historyCandidates(i) {
            let n = sourceLabels[source] ?? source
            if !names.contains(n) { names.append(n) }
        }
        if names.isEmpty { return "No price source covers this instrument." }
        let list = names.count <= 2 ? names.joined(separator: " or ") : names.dropLast().joined(separator: ", ") + " or " + names.last!
        return "No bars for this span from " + list + "."
    }

    /// Stored bars for [start, end], fetched when the span was never fetched or the
    /// copy is older than 20 hours and the span reaches the present.
    static func ensureHistory(_ i: BHInstrument, _ start: String, _ end: String) async -> [BHDailyBar] {
        let sym = tmxSymbol(i.symbol)
        guard !sym.isEmpty, start.count == 10, end.count == 10 else { return [] }
        let name = "bars-" + sym.replacingOccurrences(of: "/", with: "_") + ".json"
        var file = readJSON(name)
        let coveredFrom = (file["start"] as? String) ?? ""
        let covered = !coveredFrom.isEmpty && coveredFrom <= start
        let fresh = age((file["fetchedAt"] as? String) ?? "").map { $0 < historyStaleHours * 3600 } ?? false
        let needsRecent = end >= BHModel.shiftDate(today(), -3)
        if !covered || (needsRecent && !fresh) {
            let fetchFrom = covered ? min(start, coveredFrom) : start
            let (bars, source) = await fetchHistory(i, fetchFrom, today())
            if !bars.isEmpty {
                var stored: [String: [String: Any]] = [:]
                for (d, v) in (file["bars"] as? [String: Any]) ?? [:] { stored[d] = v as? [String: Any] }
                for b in bars { stored[b.date] = ["o": b.open as Any, "h": b.high as Any, "l": b.low as Any, "c": b.close, "v": b.volume as Any] }
                let gotFrom = bars[0].date
                let from = (BHModel.dayNumber(gotFrom) ?? 0) <= (BHModel.dayNumber(fetchFrom) ?? 0) + coverageSlackDays ? fetchFrom : gotFrom
                file = ["start": from, "fetchedAt": nowStamp(), "source": source, "bars": stored]
                writeJSON(name, file)
            }
        }
        var out: [BHDailyBar] = []
        for (d, v) in (file["bars"] as? [String: Any]) ?? [:] where d >= start && d <= end {
            let b = (v as? [String: Any]) ?? [:]
            guard let c = num(b["c"]) else { continue }
            out.append(BHDailyBar(date: d, open: num(b["o"]), high: num(b["h"]), low: num(b["l"]), close: c, volume: num(b["v"])))
        }
        return out.sorted { $0.date < $1.date }
    }

    /// Weekly (Monday start) or monthly bars from daily ones.
    static func aggregateDaily(_ bars: [BHDailyBar], _ tf: String) -> [BHDailyBar] {
        var out: [BHDailyBar] = []
        for b in bars {
            guard let dn = BHModel.dayNumber(b.date) else { continue }
            let key: String
            if tf == "1w" {
                let weekday = ((dn % 7) + 7 + 3) % 7   // day 0 (1970-01-01) was a Thursday
                key = BHModel.isoDate(fromDayNumber: dn - weekday)
            } else {
                key = String(b.date.prefix(7)) + "-01"
            }
            if var cur = out.last, cur.date == key {
                cur.close = b.close
                if let h = b.high { cur.high = cur.high.map { max($0, h) } ?? h }
                if let l = b.low { cur.low = cur.low.map { min($0, l) } ?? l }
                if let v = b.volume { cur.volume = (cur.volume ?? 0) + v }
                out[out.count - 1] = cur
            } else {
                out.append(BHDailyBar(date: key, open: b.open, high: b.high, low: b.low, close: b.close, volume: b.volume))
            }
        }
        return out
    }

    /// The trade chart: real bars for the trade's span, ten days either side, at the
    /// timeframe the trade's length calls for (1D up to 180 days, 1W up to about
    /// three years, 1M beyond). Intraday timeframes are not on the phone yet.
    static func bars(for trade: BHTrade, timeframe: String? = nil) async -> (bars: [BHBar], reason: String, timeframe: String) {
        let inst = chartInstrument(BHInstrument(symbol: trade.symbol, exchange: trade.exchange, currency: trade.currency, kind: trade.kind))
        let start = BHModel.shiftDate(trade.entryDate, -10)
        let end = min(BHModel.shiftDate(trade.exitDate, 10), today())
        let tf = timeframe ?? (trade.holdDays <= 180 ? "1d" : (trade.holdDays <= 1100 ? "1w" : "1M"))
        let daily = await ensureHistory(inst, start, end)
        if daily.isEmpty { return ([], chartReason(inst), tf) }
        let shown = tf == "1d" ? daily : aggregateDaily(daily, tf)
        return (shown.compactMap { b in
            guard let o = b.open, let h = b.high, let l = b.low else { return nil }
            return BHBar(time: b.date, open: o, high: h, low: l, close: b.close)
        }, "", tf)
    }
}
