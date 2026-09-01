import SwiftUI
import UIKit
import WebKit

@main
struct BagholderApp: App {
    var body: some Scene {
        WindowGroup {
            RootView()
        }
    }
}

/// Example numbers from the locked home mock. Not live Wealthsimple data.
private enum HomeExample {
    static let realizedPnL = "$12,450.10"
    static let biggestWinner = "$4,120.00"
    static let biggestWinnerSymbol = "PLTR"
    static let biggestLoser = "\u{2212}$890.00"
    static let biggestLoserSymbol = "CCO"
    static let profitFactor = "1.8"
    static let profitFactorSubtitle = "$18,200 W, $10,100 L"
    static let expectancy = "$210.00"
    static let expectancySubtitle = "$1,820 \u{00B7} \u{2212}$1,260"
    static let winRate = "62%"
    static let winRateSubtitle = "8 W, 5 L"
    static let avgAnnualized = "14.2%"
    static let avgAnnualizedSubtitle = "4.2 yrs"
    static let years: [(year: String, ret: String, spy: String, vs: String)] = [
        ("2026", "+18.4%", "+9.2%", "+9.2%"),
        ("2025", "+11.0%", "+14.1%", "\u{2212}3.1%"),
        ("2024", "+22.6%", "+23.3%", "\u{2212}0.7%"),
    ]
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
    init() {
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
                HomeView()
            }
            .tabItem { Label("Home", systemImage: "square.grid.2x2") }

            NavigationStack {
                ClosedTradesView()
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
    }
}

struct HomeView: View {
    @State private var showSettings = false
    @StateObject private var session = SessionStore()

    var body: some View {
        ScrollView(showsIndicators: false) {
        VStack(alignment: .leading, spacing: 0) {
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

            VStack(alignment: .leading, spacing: 3) {
                Text(HomeExample.realizedPnL)
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
                    value: HomeExample.biggestWinner,
                    subtitle: HomeExample.biggestWinnerSymbol,
                    valueColor: HomeColor.profit
                )
                MetricCard(
                    title: "Biggest loser",
                    value: HomeExample.biggestLoser,
                    subtitle: HomeExample.biggestLoserSymbol,
                    valueColor: HomeColor.loss
                )
                MetricCard(
                    title: "Profit factor",
                    value: HomeExample.profitFactor,
                    subtitle: HomeExample.profitFactorSubtitle
                )
                MetricCard(
                    title: "Expectancy",
                    value: HomeExample.expectancy,
                    subtitle: HomeExample.expectancySubtitle
                )
                MetricCard(
                    title: "Win rate",
                    value: HomeExample.winRate,
                    subtitle: HomeExample.winRateSubtitle
                )
                MetricCard(
                    title: "Avg annualized",
                    value: HomeExample.avgAnnualized,
                    subtitle: HomeExample.avgAnnualizedSubtitle
                )
            }
            .padding(.horizontal, HomeLayout.side)
            .padding(.top, 12)

            EquityCurveCard()
                .padding(.horizontal, HomeLayout.side)
                .padding(.top, HomeLayout.gutter)
            MonthlyPnLCard()
                .padding(.horizontal, HomeLayout.side)
                .padding(.top, HomeLayout.gutter)
            AnnualPerformanceCard()
                .padding(.horizontal, HomeLayout.side)
                .padding(.top, HomeLayout.gutter)
                .padding(.bottom, 12)
        }
        }
        .background(HomeColor.page)
        .sheet(isPresented: $showSettings) {
            SettingsView(session: session)
        }
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
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Equity Curve")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(HomeColor.muted)
                .padding(.bottom, 4)
            EquityCurveDrawing()
                .frame(height: HomeLayout.chartHeight)
                .accessibilityLabel("Example equity curve")
        }
        .padding(EdgeInsets(top: 10, leading: 12, bottom: 8, trailing: 12))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: HomeLayout.corner, style: .continuous)
                .fill(HomeColor.tile)
        )
    }
}

private struct EquityCurveDrawing: View {
    var body: some View {
        Canvas { context, size in
            let sx = size.width / 330
            let sy = size.height / 72
            func pt(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
                CGPoint(x: x * sx, y: y * sy)
            }
            var fill = Path()
            fill.move(to: pt(4, 58))
            fill.addCurve(to: pt(80, 48), control1: pt(40, 56), control2: pt(55, 52))
            fill.addCurve(to: pt(155, 34), control1: pt(110, 42), control2: pt(130, 46))
            fill.addCurve(to: pt(240, 16), control1: pt(185, 18), control2: pt(210, 24))
            fill.addCurve(to: pt(326, 6), control1: pt(270, 8), control2: pt(300, 12))
            fill.addLine(to: pt(326, 72))
            fill.addLine(to: pt(4, 72))
            fill.closeSubpath()
            context.fill(fill, with: .color(HomeColor.profit.opacity(0.12)))

            var line = Path()
            line.move(to: pt(4, 58))
            line.addCurve(to: pt(80, 48), control1: pt(40, 56), control2: pt(55, 52))
            line.addCurve(to: pt(155, 34), control1: pt(110, 42), control2: pt(130, 46))
            line.addCurve(to: pt(240, 16), control1: pt(185, 18), control2: pt(210, 24))
            line.addCurve(to: pt(326, 6), control1: pt(270, 8), control2: pt(300, 12))
            context.stroke(
                line,
                with: .color(HomeColor.profit),
                style: StrokeStyle(lineWidth: 2.4, lineCap: .round, lineJoin: .round)
            )
        }
    }
}

struct MonthlyPnLCard: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Monthly P&L")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(HomeColor.muted)
                .padding(.bottom, 4)
            MonthlyPnLDrawing()
                .frame(height: HomeLayout.chartHeight)
                .accessibilityLabel("Example monthly P&L")
        }
        .padding(EdgeInsets(top: 10, leading: 12, bottom: 8, trailing: 12))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: HomeLayout.corner, style: .continuous)
                .fill(HomeColor.tile)
        )
    }
}

private struct MonthlyPnLDrawing: View {
    private struct Bar {
        let x: CGFloat
        let y: CGFloat
        let w: CGFloat
        let h: CGFloat
        let profit: Bool
    }

    private let bars: [Bar] = [
        Bar(x: 22, y: 20, w: 22, h: 16, profit: true),
        Bar(x: 62, y: 12, w: 22, h: 24, profit: true),
        Bar(x: 102, y: 36, w: 22, h: 18, profit: false),
        Bar(x: 142, y: 8, w: 22, h: 28, profit: true),
        Bar(x: 182, y: 24, w: 22, h: 12, profit: true),
        Bar(x: 222, y: 36, w: 22, h: 14, profit: false),
        Bar(x: 262, y: 16, w: 22, h: 20, profit: true),
    ]

    var body: some View {
        Canvas { context, size in
            let sx = size.width / 330
            let sy = size.height / 72
            var zero = Path()
            zero.move(to: CGPoint(x: 8 * sx, y: 36 * sy))
            zero.addLine(to: CGPoint(x: 322 * sx, y: 36 * sy))
            context.stroke(zero, with: .color(HomeColor.hairline), lineWidth: 1)

            for bar in bars {
                let rect = CGRect(
                    x: bar.x * sx,
                    y: bar.y * sy,
                    width: bar.w * sx,
                    height: bar.h * sy
                )
                let path = Path(roundedRect: rect, cornerRadius: 3)
                context.fill(
                    path,
                    with: .color(bar.profit ? HomeColor.profit : HomeColor.loss)
                )
            }
        }
    }
}


struct AnnualPerformanceCard: View {
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
            ForEach(Array(HomeExample.years.enumerated()), id: \.offset) { _, row in
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
        .accessibilityLabel("Example annual performance")
    }

    private func percentText(_ value: String) -> Text {
        let loss = value.contains("\u{2212}") || value.hasPrefix("-")
        let profit = value.hasPrefix("+")
        return Text(value)
            .foregroundStyle(loss ? HomeColor.loss : profit ? HomeColor.profit : HomeColor.muted)
    }
}

struct ClosedTradesView: View {
    var body: some View {
        List {
            Section("Closed trades") {
                Text("None yet")
                    .foregroundStyle(.secondary)
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

final class SessionStore: ObservableObject {
    @Published var connected = false

    init() {
        connected = Keychain.hasSession()
    }

    func saveOAuthCookie(_ value: String, wssdi: String?) {
        Keychain.save(oauthCookie: value, wssdi: wssdi)
        connected = true
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

