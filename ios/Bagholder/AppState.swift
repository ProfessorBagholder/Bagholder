// The app's state: the Wealthsimple session in the keychain, the last pull on
// disk, the journal, the filter set, and the model built from them. Screens
// read `Book`; the pull runs in the background and writes it back. Nothing
// leaves the device except the calls to Wealthsimple and the market sources.
import SwiftUI
import WebKit
import Security

// MARK: - the session

enum Keychain {
    static let service = "ca.bagholder.ios"
    static let account = "ws_oauth_cookie"

    static func hasSession() -> Bool {
        guard let raw = load()?["oauth_cookie"] as? String else { return false }
        return WSPull.jsonWithAccessToken(raw) != nil
    }

    static func save(oauthCookie: String, wssdi: String?) {
        guard WSPull.jsonWithAccessToken(oauthCookie) != nil else { return }
        var body: [String: String] = ["oauth_cookie": oauthCookie]
        if let wssdi, !wssdi.isEmpty { body["wssdi"] = wssdi }
        guard let data = try? JSONSerialization.data(withJSONObject: body) else { return }
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: account]
        SecItemDelete(query as CFDictionary)
        var add = query
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(add as CFDictionary, nil)
    }

    static func load() -> [String: Any]? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: account,
            kSecReturnData as String: true, kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &out)
        guard status == errSecSuccess, let data = out as? Data, let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return obj
    }

    static func clear() {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: account]
        SecItemDelete(query as CFDictionary)
    }
}

// MARK: - what is kept on disk

enum AppFiles {
    static var dir: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first ?? URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
        let d = base.appendingPathComponent("Bagholder", isDirectory: true)
        try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
        return d
    }
}

/// The last pull, token-free, so the next launch shows the last numbers at once.
enum LastPullStore {
    static var url: URL { AppFiles.dir.appendingPathComponent("last-pull.json") }

    static func load() -> WSPullResult? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(WSPullResult.self, from: data)
    }

    static func save(_ result: WSPullResult) {
        let snap = result
        Task.detached {
            if let data = try? JSONEncoder().encode(snap) { try? data.write(to: url, options: .atomic) }
        }
    }

    static func clear() { try? FileManager.default.removeItem(at: url) }

    static func modifiedAt() -> Date? { (try? FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate]) as? Date }
}

/// Grades, theses and tags, keyed by the round trip that opened the trade.
enum JournalStore {
    static var url: URL { AppFiles.dir.appendingPathComponent("journal.json") }

    static func load() -> [String: BHJournalEntry] {
        guard let data = try? Data(contentsOf: url), let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        var out: [String: BHJournalEntry] = [:]
        for (k, v) in obj {
            let e = (v as? [String: Any]) ?? [:]
            out[k] = BHJournalEntry(grade: (e["grade"] as? String) ?? "", thesis: (e["thesis"] as? String) ?? "", tags: (e["tags"] as? [String]) ?? [])
        }
        return out
    }

    static func save(_ journal: [String: BHJournalEntry]) {
        var obj: [String: Any] = [:]
        for (k, e) in journal where !(e.grade.isEmpty && e.thesis.isEmpty && e.tags.isEmpty) {
            obj[k] = ["grade": e.grade, "thesis": e.thesis, "tags": e.tags]
        }
        if let data = try? JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys]) { try? data.write(to: url, options: .atomic) }
    }
}

extension BHFilters {
    func toJSON() -> [String: Any] {
        var r: [String: Any] = [:]
        for (k, v) in ranges { r[k] = ["op": v.op, "v": v.v.map { $0 as Any } ?? NSNull()] }
        return ["lists": lists, "ranges": r, "preset": preset, "years": years, "from": from, "to": to, "search": search, "benchmark": benchmark]
    }

    static func load() -> BHFilters {
        guard let data = UserDefaults.standard.data(forKey: "bagholder.filters.v2"), let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return BHFilters() }
        return BHFilters(json: obj)
    }

    func persist() {
        if let data = try? JSONSerialization.data(withJSONObject: toJSON()) { UserDefaults.standard.set(data, forKey: "bagholder.filters.v2") }
    }
}


// MARK: - the book

@MainActor
final class Book: ObservableObject {
    enum Phase: Equatable { case idle, pulling, ready, failed(String) }

    static let appVersion = (Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String) ?? ""

    @Published var connected = false
    @Published var phase: Phase = .idle
    @Published var syncStep = ""
    @Published var lastSync: Date?
    @Published private(set) var view: BHView?
    @Published private(set) var filters = BHFilters.load()
    @Published private(set) var quotes: [String: BHQuote] = MarketData.loadQuotes()

    private(set) var base: BHBase?
    private(set) var result: WSPullResult?
    private(set) var journal = JournalStore.load()
    private var task: Task<Void, Never>?
    private var marketTask: Task<Void, Never>?
    private var loginActive = false
    private var generation = 0
    private var buildGeneration = 0
    private var noNewShownAt: Date?
    private static let lastSyncKey = "bagholder.lastSync"

    init() {
        connected = Keychain.hasSession()
        lastSync = UserDefaults.standard.object(forKey: Self.lastSyncKey) as? Date ?? LastPullStore.modifiedAt()
        if let saved = LastPullStore.load() {
            result = saved
            phase = .ready
            rebuild()
        }
    }

    // MARK: status

    /// The header's status: the sync step while pulling, an error once, else when the last sync was.
    var headerStatus: String {
        if !syncStep.isEmpty {
            if syncStep == "No new transactions", let shown = noNewShownAt, Date().timeIntervalSince(shown) >= 45 {
            } else {
                return syncStep
            }
        }
        if case .failed(let msg) = phase { return msg }
        if let lastSync { return BHFmt.syncedLabel(lastSync) }
        if phase == .pulling { return "Fetching accounts…" }
        return ""
    }

    var statusIsError: Bool {
        if case .failed = phase { return true }
        return false
    }

    // MARK: lifecycle

    /// On appear: a saved pull is shown at once; the session pulls when a sync is due.
    func handleAppear() {
        // the password manager's sheet takes the scene inactive and back; nothing restarts behind the login
        if loginActive { return }
        startMarketLoop()
        startPortfolioLoop()
        guard connected, phase != .pulling else { return }
        if result == nil || Self.activityPullDue(lastSync: lastSync) { pull() }
    }

    func handleBackground() {
        marketTask?.cancel()
        marketTask = nil
    }

    // MARK: market data, every minute while the app is up

    /// The held instruments a quote is wanted for, and the dividend payers whose declared record is.
    private func instruments() -> (held: [BHInstrument], payers: [BHInstrument]) {
        guard let b = base else { return ([], []) }
        let held = b.positions.map { BHInstrument(symbol: $0.symbol, exchange: $0.exchange, currency: $0.currency, kind: $0.kind) }
        let paying = Set(b.cashflow.filter { $0.kind == "Dividend" }.map { $0.symbol })
        let payers = held.filter { paying.contains($0.symbol) && $0.kind == "Shares" }
        return (held, payers)
    }

    // MARK: the Portfolio figures Wealthsimple states, every five minutes while the app is up

    private var portfolioTask: Task<Void, Never>?

    private func startPortfolioLoop() {
        if portfolioTask != nil { return }
        portfolioTask = Task { [weak self] in
            // the first read as soon as the app is ready, then every five minutes; a
            // failed read is said on the console, not swallowed
            while !Task.isCancelled {
                guard let self, !Task.isCancelled else { return }
                guard self.result != nil, self.phase == .ready, let rec = Keychain.load(), let cookie = rec["oauth_cookie"] as? String else {
                    try? await Task.sleep(nanoseconds: 5_000_000_000)
                    continue
                }
                do {
                    let snap = try await WSPull.refreshPortfolio(oauthCookie: cookie, wssdi: rec["wssdi"] as? String)
                    if Task.isCancelled { return }
                    await MainActor.run { [weak self] in
                        guard let self, var result = self.result else { return }
                        result.accounts = snap.accounts
                        result.balances = snap.balances
                        result.margin = snap.margin
                        self.result = result
                        LastPullStore.save(result)
                        self.rebuild()
                    }
                } catch {
                    print("bagholder portfolio: refresh failed: \(error)")
                }
                try? await Task.sleep(nanoseconds: 300_000_000_000)
            }
        }
    }

    private func startMarketLoop() {
        if marketTask != nil { return }
        marketTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                let (held, payers) = self.instruments()
                await MarketData.refreshIndexes()
                await MarketData.refreshDistributions(payers)
                let quotes = await MarketData.refreshQuotes(held)
                if Task.isCancelled { return }
                self.setQuotes(quotes)
                try? await Task.sleep(nanoseconds: 60_000_000_000)
            }
        }
    }

    /// crates/store/src/admin.rs activity_pull_due: America/Edmonton, Mon-Fri, at or after 14:00.
    static func activityPullDue(lastSync: Date?, now: Date = Date()) -> Bool {
        guard let tz = TimeZone(identifier: "America/Edmonton") else { return false }
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = tz
        let weekday = cal.component(.weekday, from: now)
        if weekday == 1 || weekday == 7 { return false }
        var parts = cal.dateComponents([.year, .month, .day], from: now)
        parts.hour = 14; parts.minute = 0; parts.second = 0; parts.nanosecond = 0
        parts.timeZone = tz
        guard let close = cal.date(from: parts), now >= close else { return false }
        guard let last = lastSync else { return true }
        return last < close
    }

    func connect(cookie: String, wssdi: String?) {
        Keychain.save(oauthCookie: cookie, wssdi: wssdi)
        connected = true
        Task { await WSPull.stampClientId(oauthCookie: cookie, wssdi: wssdi) }
        pull()
    }

    /// Get the login page loading before the user taps Connect, so it appears at once.
    func warmLogin() { LoginWeb.shared.warm() }

    /// While the login sheet is up nothing behind it needs quotes; the loop resumes when it closes.
    func loginShown() { loginActive = true; marketTask?.cancel(); marketTask = nil }
    func loginHidden() { loginActive = false; startMarketLoop(); startPortfolioLoop() }

    func disconnect() {
        task?.cancel()
        generation += 1
        Keychain.clear()
        // clear the session and anything that could hold a token, but keep the HTTP cache
        // (scripts, fonts, the bot-check assets) so the next login is not a cold re-download
        let store = WKWebsiteDataStore.default()
        let sessionTypes: Set<String> = [
            WKWebsiteDataTypeCookies,
            WKWebsiteDataTypeLocalStorage,
            WKWebsiteDataTypeSessionStorage,
            WKWebsiteDataTypeIndexedDBDatabases,
            WKWebsiteDataTypeWebSQLDatabases,
            WKWebsiteDataTypeServiceWorkerRegistrations,
        ]
        store.removeData(ofTypes: sessionTypes, modifiedSince: .distantPast) {}
        connected = false
        result = nil
        base = nil
        view = nil
        phase = .idle
        syncStep = ""
        lastSync = nil
        UserDefaults.standard.removeObject(forKey: Self.lastSyncKey)
        LastPullStore.clear()
        LoginWeb.shared.reset()
        LoginWeb.shared.warm()
    }

    func syncNow() {
        guard connected else { return }
        pull()
    }

    // MARK: the pull

    private func pull() {
        task?.cancel()
        generation += 1
        let gen = generation
        noNewShownAt = nil
        phase = .pulling
        syncStep = "Fetching accounts…"
        task = Task { [weak self] in
            guard let self else { return }
            guard let rec = Keychain.load(), let cookie = rec["oauth_cookie"] as? String else {
                self.showError(gen, "No Wealthsimple session"); return
            }
            let wssdi = rec["wssdi"] as? String
            let stored = self.result
            do {
                var snap = try await WSPull.run(
                    oauthCookie: cookie, wssdi: wssdi,
                    storedActivities: stored?.activities ?? [], storedNav: stored?.nav ?? [],
                    storedNavByAccount: stored?.navByAccount ?? [:], storedListings: stored?.listings ?? []
                ) { [weak self] step in
                    Task { @MainActor [weak self] in
                        guard let self, gen == self.generation else { return }
                        self.syncStep = step
                    }
                }
                if Task.isCancelled || gen != self.generation { return }
                if snap.listings.isEmpty, let old = stored?.listings, !old.isEmpty { snap.listings = old }
                self.result = snap
                LastPullStore.save(snap)
                if snap.transferredNew {
                    self.lastSync = Date()
                    UserDefaults.standard.set(self.lastSync, forKey: Self.lastSyncKey)
                    self.syncStep = ""
                } else {
                    self.syncStep = "No new transactions"
                    self.noNewShownAt = Date()
                }
                self.phase = .ready
                self.rebuild()
                // the listings the trades name, the rates and the index, then the model again
                if WSPull.needsListingFetch(activities: snap.activities, have: snap.listings) {
                    self.syncStep = "Fetching listings…"
                    let recNow = Keychain.load()
                    let fetched = await WSPull.fetchListings(oauthCookie: (recNow?["oauth_cookie"] as? String) ?? cookie, wssdi: (recNow?["wssdi"] as? String) ?? wssdi,
                                                            activities: snap.activities, have: snap.listings)
                    if Task.isCancelled || gen != self.generation { return }
                    self.mergeListings(fetched)
                    if self.syncStep == "Fetching listings…" { self.syncStep = "" }
                }
                self.syncStep = self.syncStep.isEmpty ? "Fetching exchange rates…" : self.syncStep
                _ = await WSPull.ensureFxRates(activities: snap.activities)
                _ = await WSPull.ensureSpyPrices()
                if Task.isCancelled || gen != self.generation { return }
                if self.syncStep == "Fetching exchange rates…" { self.syncStep = "" }
                self.rebuild()
            } catch is CancellationError {
            } catch let url as URLError where url.code == .cancelled {
            } catch WSPullError.graphql(let msg) {
                self.showError(gen, msg)
            } catch WSPullError.refresh(let msg) {
                self.showError(gen, msg)
            } catch WSPullError.unauthorized {
                self.showError(gen, "Wealthsimple token refresh failed")
            } catch WSPullError.noIdentity {
                self.showError(gen, "Wealthsimple session has no identity")
            } catch WSPullError.noSession {
                self.showError(gen, "No Wealthsimple session")
            } catch {
                self.showError(gen, error.localizedDescription)
            }
        }
    }

    private func showError(_ gen: Int, _ msg: String) {
        guard gen == generation else { return }
        let line = msg.trimmingCharacters(in: .whitespacesAndNewlines)
        let shown = line.isEmpty ? "Wealthsimple token refresh failed" : line
        if result != nil {
            phase = .ready
            syncStep = shown
        } else {
            phase = .failed(shown)
            syncStep = ""
        }
    }

    private func mergeListings(_ extra: [WSSecurityListing]) {
        guard var snap = result else { return }
        var byId: [String: WSSecurityListing] = [:]
        for s in snap.listings { byId[s.id] = s }
        for s in extra {
            if s.name.isEmpty, let old = byId[s.id], !old.name.isEmpty { continue }
            byId[s.id] = s
        }
        snap.listings = Array(byId.values)
        result = snap
        LastPullStore.save(snap)
    }

    // MARK: the model

    /// The base is built off the main thread; the screens keep the last view
    /// until the new one is ready, and a build overtaken by a newer one is dropped.
    func rebuild() {
        guard let snap = result else { view = nil; base = nil; return }
        buildGeneration += 1
        let gen = buildGeneration
        let quotes = self.quotes, journal = self.journal, filters = self.filters
        Task.detached(priority: .userInitiated) { [weak self] in
            var market = BHMarket()
            market.fx = WSPull.loadCachedFx()
            market.benchmark = WSPull.loadCachedSP500()
            market.benchmarks = ["SP500": market.benchmark]
            for (k, v) in MarketData.indexCloses() { market.benchmarks[k] = v }
            market.quotes = quotes
            market.distributions = MarketData.distributions()
            let nav = snap.nav.map { BHNavPoint(date: $0.date, equity: $0.equity, netDeposits: $0.netDeposits) }
            var navBy: [String: [BHNavPoint]] = [:]
            for (nick, pts) in snap.navByAccount { navBy[nick] = pts.map { BHNavPoint(date: $0.date, equity: $0.equity, netDeposits: $0.netDeposits) } }
            let accounts = (snap.accounts ?? []).map { BHAccountInfo(id: $0.id, name: $0.nickname, currency: $0.currency, nav: $0.netLiquidationValue, type: $0.unifiedAccountType, status: $0.status) }
            let balances = (snap.balances ?? []).map { BHBalanceRow(accountId: $0.accountId, securityId: $0.securityId, quantity: $0.quantity) }
            let margin = (snap.margin ?? []).map { BHMarginRow(accountId: $0.accountId, buyingPower: $0.buyingPower, currency: $0.currency, unavailable: $0.unavailable) }
            let b = BHModel.buildBase(activities: snap.activities.map(BHAct.init), securities: snap.listings.map(WSPull.security), market: market,
                                      today: BHModel.todayLocal(), navHistory: nav, navByAccount: navBy, journal: journal,
                                      accounts: accounts, balances: balances, margin: margin)
            let v = BHModel.buildView(b, filters)
            await MainActor.run { [weak self] in
                guard let self, gen == self.buildGeneration else { return }
                self.base = b
                // the view was built off the main thread; only redo it here if the filters moved meanwhile
                self.view = self.filters == filters ? v : BHModel.buildView(b, self.filters)
            }
        }
    }

    func setFilters(_ f: BHFilters) {
        filters = f
        f.persist()
        if let b = base { view = BHModel.buildView(b, f) }
    }

    func clearFilters() {
        var f = BHFilters()
        f.benchmark = filters.benchmark
        setFilters(f)
    }

    func setBenchmark(_ key: String) {
        var f = filters
        f.benchmark = key
        setFilters(f)
    }

    /// A journal entry changes only the trade's or position's own fields: no rematching.
    func saveJournal(id: String, _ entry: BHJournalEntry) {
        journal[id] = entry
        JournalStore.save(journal)
        guard var b = base else { return }
        if let i = b.trades.firstIndex(where: { $0.id == id }) {
            b.trades[i].grade = entry.grade
            b.trades[i].thesis = entry.thesis
            b.trades[i].tags = entry.tags
        }
        for i in b.positions.indices where b.positions[i].id == id {
            b.positions[i].grade = entry.grade
            b.positions[i].thesis = entry.thesis
            b.positions[i].tags = entry.tags
        }
        base = b
        view = BHModel.buildView(b, filters)
    }

    /// New quotes rebuild the book only when a price actually moved.
    func setQuotes(_ q: [String: BHQuote]) {
        let changed = q.contains { sym, quote in quotes[sym]?.price != quote.price || quotes[sym]?.exDividendDate != quote.exDividendDate } || q.count != quotes.count
        quotes = q
        if changed { rebuild() }
    }

    func trade(id: String) -> BHTrade? { base?.trades.first { $0.id == id } }
    func position(id: String) -> BHPosition? { base?.positions.first { $0.id == id } }
}

// MARK: - the Wealthsimple login in a web view

/// One login web view, kept alive and warm across opens so the engine is already
/// running and the page (and its bot-check) is already past by the time Connect is
/// tapped, the way a browser feels instant. `warm()` starts the load early; capture
/// runs only while the sheet is open; `reset()` drops the page after a login or a
/// disconnect so the next warm loads a fresh one.
@MainActor
final class LoginWeb {
    static let shared = LoginWeb()

    let webView: WKWebView
    private let coordinator: Coordinator
    private var timer: Timer?
    private var done = false
    var onSession: ((String, String?) -> Void)?

    private static let loginURL = URL(string: "https://my.wealthsimple.com/app/login")!

    init() {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        config.defaultWebpagePreferences.allowsContentJavaScript = true
        webView = WKWebView(frame: .zero, configuration: config)
        coordinator = Coordinator()
        webView.navigationDelegate = coordinator
        webView.uiDelegate = coordinator
    }

    /// Start loading the login page if it is not already loaded or loading.
    func warm() {
        guard webView.url == nil || webView.url?.absoluteString == "about:blank" else { return }
        if webView.isLoading { return }
        park()
        webView.load(URLRequest(url: Self.loginURL))
    }

    private static var keyWindow: UIWindow? {
        UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow }.first
    }

    /// A web view outside any window is suspended by WebKit on the device and never finishes
    /// loading; while warming it sits behind the app's opaque root, in the window, unseen.
    private func park() {
        guard webView.superview == nil, let window = Self.keyWindow else { return }
        webView.frame = window.bounds
        webView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        webView.accessibilityElementsHidden = true
        window.insertSubview(webView, at: 0)
    }


    /// While the sheet is open: watch for the session cookie and hand it back once.
    func beginCapture(_ onSession: @escaping (String, String?) -> Void) {
        self.onSession = onSession
        done = false
        warm()
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in self?.inspectCookies() }
    }

    func endCapture() {
        timer?.invalidate(); timer = nil
        onSession = nil
    }

    /// Drop the current page (after a successful login, or on disconnect) so a later warm reloads fresh.
    func reset() {
        endCapture()
        done = false
        webView.load(URLRequest(url: URL(string: "about:blank")!))
    }

    private func inspectCookies() {
        WKWebsiteDataStore.default().httpCookieStore.getAllCookies { [weak self] cookies in
            guard let self, !self.done else { return }
            var oauth: String?
            var wssdi: String?
            for cookie in cookies {
                if cookie.name == "wssdi", !cookie.value.isEmpty { wssdi = cookie.value }
                else if cookie.name == "_oauth2_access_v2", WSPull.jsonWithAccessToken(cookie.value) != nil { oauth = cookie.value }
            }
            if oauth == nil {
                for cookie in cookies where cookie.name != "wssdi" {
                    if WSPull.jsonWithAccessToken(cookie.value) != nil { oauth = cookie.value; break }
                }
            }
            guard let oauth else { return }
            self.done = true
            let hand = self.onSession
            self.endCapture()
            hand?(oauth, wssdi)
        }
    }

    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate {
        func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
            if navigationAction.targetFrame == nil { webView.load(navigationAction.request) }
            return nil
        }
    }
}

struct ConnectLoginView: View {
    @Environment(\.theme) private var t
    @ObservedObject var book: Book
    @Binding var isPresented: Bool
    @State private var attached = false

    var body: some View {
        NavigationStack {
            Group {
                // the sheet comes up on its own first; the web view is attached a beat later, so the
                // tap's response never waits on WebKit
                if attached {
                    WealthsimpleLoginWebView { oauth, wssdi in
                        book.connect(cookie: oauth, wssdi: wssdi)
                        LoginWeb.shared.reset()
                        isPresented = false
                    }
                } else {
                    Color.white
                }
            }
            .onAppear { DispatchQueue.main.async { attached = true } }
            .ignoresSafeArea(edges: .bottom)
            .ignoresSafeArea(.keyboard)   // the web view scrolls its own focused field; no relayout for the keyboard
            .onAppear { book.loginShown() }
            .onDisappear { book.loginHidden() }
            .navigationTitle("Connect Wealthsimple")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .topBarLeading) { Button("Cancel") { LoginWeb.shared.endCapture(); isPresented = false } } }
        }
    }
}

/// Hosts the one warm login web view; it is never recreated, so opens after the first are instant.
struct WealthsimpleLoginWebView: UIViewRepresentable {
    var onSession: (String, String?) -> Void

    func makeUIView(context: Context) -> WKWebView {
        LoginWeb.shared.webView.removeFromSuperview()   // out from behind the root, into the sheet
        LoginWeb.shared.beginCapture(onSession)
        return LoginWeb.shared.webView
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {}
}
