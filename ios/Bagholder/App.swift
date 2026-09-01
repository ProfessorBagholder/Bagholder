import SwiftUI
import UIKit
import WebKit
import Security

@main
struct BagholderApp: App {
    var body: some Scene {
        WindowGroup {
            RootView()
        }
    }
}

private enum HomeColor {
    static let page = Color(red: 242 / 255, green: 242 / 255, blue: 247 / 255)
    static let tile = Color.white
    static let muted = Color(red: 142 / 255, green: 142 / 255, blue: 147 / 255)
    static let profit = Color(red: 52 / 255, green: 199 / 255, blue: 89 / 255)
    static let loss = Color(red: 255 / 255, green: 59 / 255, blue: 48 / 255)
    static let selected = Color(red: 0 / 255, green: 122 / 255, blue: 255 / 255)
    static let hairline = Color(red: 60 / 255, green: 60 / 255, blue: 67 / 255).opacity(0.18)
}

private enum HomeLayout {
    static let side: CGFloat = 16
    static let gutter: CGFloat = 8
    static let corner: CGFloat = 14
    static let chartHeight: CGFloat = 72
}

struct RootView: View {
    @StateObject private var session: SessionStore
    @StateObject private var journal: Journal

    init() {
        _session = StateObject(wrappedValue: SessionStore())
        _journal = StateObject(wrappedValue: Journal())
        let page = UIColor(red: 242 / 255, green: 242 / 255, blue: 247 / 255, alpha: 1)
        let muted = UIColor(red: 142 / 255, green: 142 / 255, blue: 147 / 255, alpha: 1)
        let appearance = UITabBarAppearance()
        appearance.configureWithOpaqueBackground()
        appearance.backgroundColor = page
        appearance.shadowColor = .clear
        appearance.stackedLayoutAppearance.normal.iconColor = muted
        appearance.stackedLayoutAppearance.normal.titleTextAttributes = [
            .foregroundColor: muted
        ]
        appearance.inlineLayoutAppearance.normal.iconColor = muted
        appearance.compactInlineLayoutAppearance.normal.iconColor = muted
        UITabBar.appearance().standardAppearance = appearance
        UITabBar.appearance().scrollEdgeAppearance = appearance
        UITabBar.appearance().backgroundColor = page
    }

    var body: some View {
        TabView {
            NavigationStack {
                HomeView(session: session, journal: journal)
            }
            .tabItem { Label("Home", systemImage: "square.grid.2x2") }

            NavigationStack {
                ClosedTradesView(journal: journal)
            }
            .tabItem { Label("Closed trades", systemImage: "line.3.horizontal") }

            NavigationStack {
                EmptyTabView(title: "Activity")
            }
            .tabItem { Label("Activity", systemImage: "doc.text") }

            NavigationStack {
                EmptyTabView(title: "Open lots")
            }
            .tabItem { Label("Open lots", systemImage: "hexagon") }
        }
        .tint(HomeColor.selected)
        .toolbarBackground(HomeColor.page, for: .tabBar)
        .toolbarBackground(.visible, for: .tabBar)
        .background(HomeColor.page.ignoresSafeArea())
        .onAppear { journal.handleAppear(session: session) }
        .onChange(of: session.pullToken) { _, _ in
            journal.handleSessionChange(session: session)
        }
    }
}

struct HomeView: View {
    @ObservedObject var session: SessionStore
    @ObservedObject var journal: Journal
    @State private var showSettings = false
    @State private var showLogin = false

    var body: some View {
        Group {
            if journal.isLive, let live = journal.result {
                liveHome(live)
            } else {
                waitingHome
            }
        }
        .background(HomeColor.page)
        .sheet(isPresented: $showSettings) {
            SettingsView(session: session)
        }
        .fullScreenCover(isPresented: $showLogin) {
            ConnectLoginView(session: session, isPresented: $showLogin)
        }
    }

    private var waitingHome: some View {
        VStack(spacing: 0) {
            settingsBar
            Spacer(minLength: 0)
            waitingBody
                .padding(.horizontal, 40)
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(HomeColor.page)
    }

    @ViewBuilder
    private var waitingBody: some View {
        if journal.phase == .pulling && journal.result == nil {
            VStack(spacing: 14) {
                ProgressView()
                Text("Getting trades from Wealthsimple")
                    .font(.system(size: 17))
                    .foregroundStyle(.black)
                    .multilineTextAlignment(.center)
            }
        } else if case .failed(let msg) = journal.phase {
            VStack(spacing: 16) {
                Text(msg)
                    .font(.system(size: 17))
                    .foregroundStyle(HomeColor.loss)
                    .multilineTextAlignment(.center)
                connectButton
            }
        } else {
            VStack(spacing: 16) {
                Text("Connect Wealthsimple")
                    .font(.system(size: 22, weight: .semibold))
                    .foregroundStyle(.black)
                    .multilineTextAlignment(.center)
                Text("Sign in to see Home.")
                    .font(.system(size: 15))
                    .foregroundStyle(HomeColor.muted)
                    .multilineTextAlignment(.center)
                connectButton
            }
        }
    }

    private var connectButton: some View {
        Button {
            showLogin = true
        } label: {
            Text("Connect")
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(.white)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 14)
                .background(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(HomeColor.selected)
                )
        }
        .frame(maxWidth: 280)
        .accessibilityLabel("Connect")
    }

    private var settingsBar: some View {
        HStack {
            Spacer(minLength: 0)
            Button {
                showSettings = true
            } label: {
                Image(systemName: "gearshape")
                    .font(.system(size: 17, weight: .regular))
                    .foregroundStyle(.black)
                    .frame(width: 24, height: 24)
            }
            .accessibilityLabel("Settings")
        }
        .padding(.horizontal, 14)
        .frame(height: 36)
    }

    private func liveHome(_ live: WSPullResult) -> some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: 0) {
                settingsBar

                VStack(alignment: .leading, spacing: 3) {
                    Text(WSPull.formatCad(live.metrics.realizedPnlCad, digits: 2))
                        .font(.system(size: 38, weight: .bold))
                        .tracking(-1.4)
                        .lineSpacing(0)
                    Text("Realized P&L")
                        .font(.system(size: 15))
                        .foregroundStyle(HomeColor.muted)
                }
                .padding(.horizontal, HomeLayout.side)

                LazyVGrid(
                    columns: [
                        GridItem(.flexible(), spacing: HomeLayout.gutter),
                        GridItem(.flexible(), spacing: HomeLayout.gutter),
                    ],
                    spacing: HomeLayout.gutter
                ) {
                    MetricCard(
                        title: "Biggest winner",
                        value: live.metrics.maxWinSymbol.isEmpty ? WSPull.emDash : WSPull.formatCad(live.metrics.maxWinPnl, digits: 2),
                        subtitle: live.metrics.maxWinSymbol.isEmpty ? WSPull.emDash : live.metrics.maxWinSymbol,
                        valueColor: HomeColor.profit
                    )
                    MetricCard(
                        title: "Biggest loser",
                        value: live.metrics.maxLossSymbol.isEmpty ? WSPull.emDash : WSPull.formatCad(live.metrics.maxLossPnl, digits: 2),
                        subtitle: live.metrics.maxLossSymbol.isEmpty ? WSPull.emDash : live.metrics.maxLossSymbol,
                        valueColor: HomeColor.loss
                    )
                    MetricCard(
                        title: "Profit factor",
                        value: WSPull.formatProfitFactor(live.metrics.profitFactor),
                        subtitle: WSPull.formatCad(live.metrics.grossProfit, digits: 2) + " W, " + WSPull.formatCad(live.metrics.grossLoss, digits: 2) + " L"
                    )
                    MetricCard(
                        title: "Expectancy",
                        value: WSPull.formatCad(live.metrics.expectancy, digits: 2),
                        subtitle: WSPull.formatCad(live.metrics.avgWin, digits: 2) + " \u{00B7} " + WSPull.formatCad(live.metrics.avgLoss, digits: 2)
                    )
                    MetricCard(
                        title: "Win rate",
                        value: WSPull.formatWinRate(live.metrics.winRate),
                        subtitle: "\(live.metrics.winCount) W, \(live.metrics.lossCount) L"
                    )
                    MetricCard(
                        title: "Avg annualized",
                        value: live.avgAnnualized,
                        subtitle: live.avgAnnualizedSubtitle.isEmpty ? WSPull.emDash : live.avgAnnualizedSubtitle
                    )
                }
                .padding(.horizontal, HomeLayout.side)
                .padding(.top, 12)

                EquityCurveCard(points: live.nav)
                    .padding(.horizontal, HomeLayout.side)
                    .padding(.top, HomeLayout.gutter)
                MonthlyPnLCard(bars: live.monthly)
                    .padding(.horizontal, HomeLayout.side)
                    .padding(.top, HomeLayout.gutter)
                AnnualPerformanceCard(years: live.years)
                    .padding(.horizontal, HomeLayout.side)
                    .padding(.top, HomeLayout.gutter)
                    .padding(.bottom, 12)
            }
        }
        .background(HomeColor.page)
    }
}

struct MetricCard: View {
    let title: String
    let value: String
    var subtitle: String? = nil
    var valueColor: Color = .black

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(HomeColor.muted)
                .padding(.bottom, 4)
            Text(value)
                .font(.system(size: 20, weight: .bold))
                .tracking(-0.6)
                .foregroundStyle(valueColor)
            if let subtitle {
                Text(subtitle)
                    .font(.system(size: 12))
                    .foregroundStyle(HomeColor.muted)
                    .padding(.top, 3)
            }
        }
        .padding(EdgeInsets(top: 11, leading: 12, bottom: 10, trailing: 12))
        .frame(maxWidth: .infinity, minHeight: 72, alignment: .topLeading)
        .background(
            RoundedRectangle(cornerRadius: HomeLayout.corner, style: .continuous)
                .fill(HomeColor.tile)
        )
    }
}

struct EquityCurveCard: View {
    var points: [WSNavPoint]? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Equity Curve")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(HomeColor.muted)
                .padding(.bottom, 4)
            if let points, !points.isEmpty {
                LiveEquityCurveDrawing(points: points)
                    .frame(height: HomeLayout.chartHeight)
                    .accessibilityLabel("Equity curve")
            } else {
                Color.clear
                    .frame(height: HomeLayout.chartHeight)
                    .accessibilityLabel("Equity curve")
            }
        }
        .padding(EdgeInsets(top: 10, leading: 12, bottom: 8, trailing: 12))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: HomeLayout.corner, style: .continuous)
                .fill(HomeColor.tile)
        )
    }
}

struct MonthlyPnLCard: View {
    var bars: [WSMonthBar]? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Monthly P&L")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(HomeColor.muted)
                .padding(.bottom, 4)
            if let bars, !bars.isEmpty {
                LiveMonthlyPnLDrawing(bars: bars)
                    .frame(height: HomeLayout.chartHeight)
                    .accessibilityLabel("Monthly P&L")
            } else {
                Color.clear
                    .frame(height: HomeLayout.chartHeight)
                    .accessibilityLabel("Monthly P&L")
            }
        }
        .padding(EdgeInsets(top: 10, leading: 12, bottom: 8, trailing: 12))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: HomeLayout.corner, style: .continuous)
                .fill(HomeColor.tile)
        )
    }
}

struct AnnualPerformanceCard: View {
    var years: [WSYearRow]? = nil

    private var rows: [(year: String, ret: String, spy: String, vs: String)] {
        (years ?? []).map { ($0.year, $0.ret, $0.spy, $0.vs) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Annual performance")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(HomeColor.muted)
                .padding(.bottom, 6)
            HStack {
                Text("Year")
                    .frame(width: 48, alignment: .leading)
                Text("Return")
                    .frame(maxWidth: .infinity, alignment: .trailing)
                Text("S&P 500")
                    .frame(maxWidth: .infinity, alignment: .trailing)
                Text("vs S&P")
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
            .font(.system(size: 11, weight: .medium))
            .foregroundStyle(HomeColor.muted)
            .padding(.bottom, 4)
            ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                HStack {
                    Text(row.year)
                        .foregroundStyle(.black)
                        .frame(width: 48, alignment: .leading)
                    percentText(row.ret)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                    percentText(row.spy)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                    percentText(row.vs)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
                .font(.system(size: 13, weight: .semibold))
                .padding(.vertical, 5)
            }
        }
        .padding(EdgeInsets(top: 10, leading: 12, bottom: 8, trailing: 12))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: HomeLayout.corner, style: .continuous)
                .fill(HomeColor.tile)
        )
        .accessibilityLabel("Annual performance")
    }

    private func percentText(_ value: String) -> Text {
        let loss = value.contains("\u{2212}") || value.hasPrefix("-")
        let profit = value.hasPrefix("+")
        return Text(value)
            .foregroundStyle(loss ? HomeColor.loss : profit ? HomeColor.profit : HomeColor.muted)
    }
}

struct ClosedTradesView: View {
    @ObservedObject var journal: Journal

    var body: some View {
        List {
            Section("Closed trades") {
                if let result = journal.result, !result.closed.isEmpty {
                    ForEach(Array(result.closed.reversed().enumerated()), id: \.offset) { _, t in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(t.symbol)
                                    .font(.body.weight(.semibold))
                                Text(t.displaySide)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Text(WSPull.formatCad(t.pnlCad, digits: 2))
                                .foregroundStyle(t.pnlCad < 0 ? HomeColor.loss : HomeColor.profit)
                        }
                    }
                } else {
                    Text("None yet")
                        .foregroundStyle(.secondary)
                }
            }
        }
        .navigationTitle("Closed trades")
    }
}

struct EmptyTabView: View {
    let title: String

    var body: some View {
        ContentUnavailableView(
            "None yet",
            systemImage: "tray",
            description: Text("Nothing to show until sign-in.")
        )
        .navigationTitle(title)
    }
}


extension WSClosedTrade: Codable {}
extension WSNavPoint: Codable {}
extension WSMetrics: Codable {}
extension WSMonthBar: Codable {}
extension WSYearRow: Codable {}
extension WSPullResult: Codable {}

/// Token-free Home snapshot so the next launch can show last numbers immediately.
private enum LastPullStore {
    private static var url: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
        let dir = base.appendingPathComponent("Bagholder", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("last-pull.json")
    }

    static func load() -> WSPullResult? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(WSPullResult.self, from: data)
    }

    static func save(_ result: WSPullResult) {
        guard let data = try? JSONEncoder().encode(result) else { return }
        try? data.write(to: url, options: .atomic)
    }

    static func clear() {
        try? FileManager.default.removeItem(at: url)
    }
}

final class Journal: ObservableObject {
    enum Phase: Equatable {
        case idle
        case pulling
        case ready
        case needsConnect
        case failed(String)
    }

    @Published var phase: Phase = .idle
    @Published var result: WSPullResult?
    private var task: Task<Void, Never>?

    var isLive: Bool { result != nil }

    init() {
        if let snap = LastPullStore.load() {
            result = snap
            phase = .ready
        }
    }

    /// First paint: leftover Keychain must not start a download.
    /// A saved result is shown immediately and refreshed in the background.
    func handleAppear(session: SessionStore) {
        guard session.connected, result != nil else { return }
        pull(showProgress: false)
    }

    /// Sign-in (`saveOAuthCookie`) starts a download. Disconnect clears the snapshot.
    func handleSessionChange(session: SessionStore) {
        if !session.connected {
            task?.cancel()
            task = nil
            phase = .idle
            result = nil
            LastPullStore.clear()
            return
        }
        pull(showProgress: result == nil)
    }

    private func pull(showProgress: Bool) {
        task?.cancel()
        if showProgress {
            phase = .pulling
        }
        task = Task { [weak self] in
            guard let rec = Keychain.load(),
                  let cookie = rec["oauth_cookie"] as? String
            else {
                await MainActor.run {
                    if self?.result == nil {
                        self?.phase = .needsConnect
                    }
                }
                return
            }
            let wssdi = rec["wssdi"] as? String
            do {
                let snap = try await WSPull.run(oauthCookie: cookie, wssdi: wssdi)
                if Task.isCancelled { return }
                await MainActor.run {
                    self?.result = snap
                    self?.phase = .ready
                    LastPullStore.save(snap)
                }
            } catch WSPullError.unauthorized, WSPullError.noIdentity, WSPullError.noSession {
                if Task.isCancelled { return }
                await MainActor.run {
                    if self?.result == nil {
                        self?.phase = .needsConnect
                    }
                }
            } catch {
                if Task.isCancelled { return }
                await MainActor.run {
                    if self?.result == nil {
                        self?.phase = .failed("Pull failed")
                    }
                }
            }
        }
    }
}

private struct LiveEquityCurveDrawing: View {
    let points: [WSNavPoint]

    var body: some View {
        Canvas { context, size in
            guard !points.isEmpty else { return }
            let vals = points.map(\.equity)
            let lo = vals.min() ?? 0
            let hi = vals.max() ?? 1
            let span: Double = (hi - lo) == 0 ? 1.0 : (hi - lo)
            let n = CGFloat(max(points.count - 1, 1))
            func pt(_ i: Int) -> CGPoint {
                let x = CGFloat(i) / n * size.width
                let y = size.height - CGFloat((vals[i] - lo) / span) * (size.height - 4) - 2
                return CGPoint(x: x, y: y)
            }
            var fill = Path()
            fill.move(to: CGPoint(x: 0, y: size.height))
            for i in points.indices { fill.addLine(to: pt(i)) }
            fill.addLine(to: CGPoint(x: size.width, y: size.height))
            fill.closeSubpath()
            context.fill(fill, with: .color(HomeColor.profit.opacity(0.12)))
            var line = Path()
            line.move(to: pt(0))
            if points.count == 1 {
                line.addLine(to: CGPoint(x: size.width, y: pt(0).y))
            }
            for i in 1..<points.count { line.addLine(to: pt(i)) }
            context.stroke(
                line,
                with: .color(HomeColor.profit),
                style: StrokeStyle(lineWidth: 2.4, lineCap: .round, lineJoin: .round)
            )
        }
    }
}

private struct LiveMonthlyPnLDrawing: View {
    let bars: [WSMonthBar]

    var body: some View {
        Canvas { context, size in
            let shown = Array(bars.suffix(18))
            let sx = size.width / 330
            let sy = size.height / 72
            var zero = Path()
            zero.move(to: CGPoint(x: 8 * sx, y: 36 * sy))
            zero.addLine(to: CGPoint(x: 322 * sx, y: 36 * sy))
            context.stroke(zero, with: .color(HomeColor.hairline), lineWidth: 1)
            let peak = shown.map { abs($0.pnl) }.max() ?? 1
            let scale = peak > 0 ? 28.0 / peak : 0.0
            let n = CGFloat(max(shown.count, 1))
            let pitch: CGFloat = shown.count <= 7 ? 40 : (300 / n)
            let barW: CGFloat = min(22, max(8, pitch * 0.55))
            let startX: CGFloat = shown.count <= 7 ? 22 : 15
            for (i, bar) in shown.enumerated() {
                let h = CGFloat(abs(bar.pnl) * scale)
                let x = (startX + CGFloat(i) * pitch) * sx
                let y = bar.pnl >= 0 ? (36 - h) * sy : 36 * sy
                let rect = CGRect(x: x, y: y, width: barW * sx, height: max(h, 1) * sy)
                let path = Path(roundedRect: rect, cornerRadius: 3)
                context.fill(path, with: .color(bar.pnl >= 0 ? HomeColor.profit : HomeColor.loss))
            }
        }
    }
}

final class SessionStore: ObservableObject {
    @Published var connected = false
    @Published var pullToken = 0

    init() {
        connected = Keychain.hasSession()
    }

    func saveOAuthCookie(_ value: String, wssdi: String?) {
        Keychain.save(oauthCookie: value, wssdi: wssdi)
        connected = true
        pullToken += 1
    }

    func disconnect() {
        Keychain.clear()
        let store = WKWebsiteDataStore.default()
        store.httpCookieStore.getAllCookies { cookies in
            for cookie in cookies {
                store.httpCookieStore.delete(cookie)
            }
        }
        WKWebsiteDataStore.default().removeData(
            ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(),
            modifiedSince: Date.distantPast,
            completionHandler: {}
        )
        connected = false
        pullToken += 1
    }
}

private enum Keychain {
    static let service = "ca.bagholder.ios"
    static let account = "ws_oauth_cookie"

    static func hasSession() -> Bool {
        guard let raw = load()?["oauth_cookie"] as? String else { return false }
        return jsonWithAccessToken(raw) != nil
    }

    static func save(oauthCookie: String, wssdi: String?) {
        guard jsonWithAccessToken(oauthCookie) != nil else { return }
        var body: [String: String] = ["oauth_cookie": oauthCookie]
        if let wssdi, !wssdi.isEmpty {
            body["wssdi"] = wssdi
        }
        guard let data = try? JSONSerialization.data(withJSONObject: body) else { return }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
        var add = query
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(add as CFDictionary, nil)
    }

    static func load() -> [String: Any]? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &out)
        guard status == errSecSuccess, let data = out as? Data,
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return obj
    }

    static func clear() {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
    }
}

private func jsonWithAccessToken(_ raw: String) -> [String: Any]? {
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

struct SettingsView: View {
    @ObservedObject var session: SessionStore
    @Environment(\.dismiss) private var dismiss
    @State private var showLogin = false

    var body: some View {
        NavigationStack {
            List {
                Section {
                    if session.connected {
                        Text("Connected")
                        Button("Disconnect", role: .destructive) {
                            session.disconnect()
                        }
                    } else {
                        Button("Connect") {
                            showLogin = true
                        }
                        Text("Opens Wealthsimple’s login. After you sign in, this screen closes.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .navigationTitle("Settings")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .fullScreenCover(isPresented: $showLogin) {
                ConnectLoginView(session: session, isPresented: $showLogin)
            }
        }
    }
}

struct ConnectLoginView: View {
    @ObservedObject var session: SessionStore
    @Binding var isPresented: Bool

    var body: some View {
        NavigationStack {
            WealthsimpleLoginWebView { oauth, wssdi in
                session.saveOAuthCookie(oauth, wssdi: wssdi)
                isPresented = false
            }
            .ignoresSafeArea(edges: .bottom)
            .navigationTitle("Connect")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { isPresented = false }
                }
            }
        }
    }
}

private struct WealthsimpleLoginWebView: UIViewRepresentable {
    var onSession: (String, String?) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onSession: onSession)
    }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        config.defaultWebpagePreferences.allowsContentJavaScript = true
        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = context.coordinator
        web.uiDelegate = context.coordinator
        context.coordinator.webView = web
        context.coordinator.start()
        web.load(URLRequest(url: URL(string: "https://my.wealthsimple.com/app/login")!))
        return web
    }

    func updateUIView(_ uiView: WKWebView, context: Context) {}

    static func dismantleUIView(_ uiView: WKWebView, coordinator: Coordinator) {
        coordinator.stop()
    }

    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate, WKHTTPCookieStoreObserver {
        let onSession: (String, String?) -> Void
        weak var webView: WKWebView?
        private var done = false
        private var timer: Timer?

        init(onSession: @escaping (String, String?) -> Void) {
            self.onSession = onSession
        }

        func start() {
            WKWebsiteDataStore.default().httpCookieStore.add(self)
            timer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
                self?.inspectCookies()
            }
        }

        func stop() {
            timer?.invalidate()
            timer = nil
            WKWebsiteDataStore.default().httpCookieStore.remove(self)
        }

        func cookiesDidChange(in cookieStore: WKHTTPCookieStore) {
            inspectCookies()
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            inspectCookies()
        }

        func webView(
            _ webView: WKWebView,
            createWebViewWith configuration: WKWebViewConfiguration,
            for navigationAction: WKNavigationAction,
            windowFeatures: WKWindowFeatures
        ) -> WKWebView? {
            if navigationAction.targetFrame == nil {
                webView.load(navigationAction.request)
            }
            return nil
        }

        private func inspectCookies() {
            WKWebsiteDataStore.default().httpCookieStore.getAllCookies { [weak self] cookies in
                guard let self, !self.done else { return }
                var oauth: String?
                var wssdi: String?
                for cookie in cookies {
                    if cookie.name == "wssdi", !cookie.value.isEmpty {
                        wssdi = cookie.value
                    }
                    if cookie.name == "_oauth2_access_v2", jsonWithAccessToken(cookie.value) != nil {
                        oauth = cookie.value
                    } else if oauth == nil, jsonWithAccessToken(cookie.value) != nil {
                        oauth = cookie.value
                    }
                }
                guard let oauth else { return }
                self.done = true
                DispatchQueue.main.async {
                    self.stop()
                    self.onSession(oauth, wssdi)
                }
            }
        }
    }
}

