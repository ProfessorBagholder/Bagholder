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

enum AppearanceChoice: String, CaseIterable {
    case light
    case dark
    case system

    var preferredColorScheme: ColorScheme? {
        switch self {
        case .light: return .light
        case .dark: return .dark
        case .system: return nil
        }
    }

    var segmentIndex: Int {
        switch self {
        case .light: return 0
        case .dark: return 1
        case .system: return 2
        }
    }

    static func fromSegment(_ index: Int) -> AppearanceChoice {
        switch index {
        case 0: return .light
        case 1: return .dark
        default: return .system
        }
    }
}

@MainActor
final class AppearanceStore: ObservableObject {
    static let defaultsKey = "bagholder.appearance"
    @Published var choice: AppearanceChoice {
        didSet { UserDefaults.standard.set(choice.rawValue, forKey: Self.defaultsKey) }
    }

    init() {
        if let raw = UserDefaults.standard.string(forKey: Self.defaultsKey),
           let stored = AppearanceChoice(rawValue: raw) {
            choice = stored
        } else {
            choice = .system
        }
    }

    var preferredColorScheme: ColorScheme? { choice.preferredColorScheme }
}

/// Named tokens for both appearances. Resolve from colorScheme + Appearance setting
/// via preferredColorScheme (nil = System) and explicit RGB — nothing inverts.
private enum HomeUIColor {
    static func token(
        light rL: CGFloat, _ gL: CGFloat, _ bL: CGFloat, _ aL: CGFloat,
        dark rD: CGFloat, _ gD: CGFloat, _ bD: CGFloat, _ aD: CGFloat
    ) -> UIColor {
        let light = UIColor(red: rL, green: gL, blue: bL, alpha: aL)
        let dark = UIColor(red: rD, green: gD, blue: bD, alpha: aD)
        return UIColor { tc in
            tc.userInterfaceStyle == .dark ? dark : light
        }
    }

    static let page = token(light: 242 / 255, 242 / 255, 247 / 255, 1, dark: 0, 0, 0, 1)
    static let tile = token(light: 1, 1, 1, 1, dark: 28 / 255, 28 / 255, 30 / 255, 1)
    static let ink = token(light: 0, 0, 0, 1, dark: 1, 1, 1, 1)
    static let muted = token(light: 142 / 255, 142 / 255, 147 / 255, 1, dark: 142 / 255, 142 / 255, 147 / 255, 1)
    static let profit = token(light: 52 / 255, 199 / 255, 89 / 255, 1, dark: 52 / 255, 199 / 255, 89 / 255, 1)
    static let loss = token(light: 255 / 255, 59 / 255, 48 / 255, 1, dark: 255 / 255, 59 / 255, 48 / 255, 1)
    static let selected = token(light: 0, 122 / 255, 1, 1, dark: 10 / 255, 132 / 255, 1, 1)
    static let hairline = token(
        light: 60 / 255, 60 / 255, 67 / 255, 0.18,
        dark: 84 / 255, 84 / 255, 88 / 255, 0.65
    )
    /// Dark: #2C2C2E Cancel chip + selected-tab pill. Light Cancel chip is page.
    static let elevated = token(light: 242 / 255, 242 / 255, 247 / 255, 1, dark: 44 / 255, 44 / 255, 46 / 255, 1)
}

/// Two live RGB sets. RootView publishes the active pack through the
/// environment every time Light / Dark / System (or the system style) changes.
/// UIKit chrome still uses HomeUIColor dynamicProvider.
private enum HomeColor {
    struct Pack: Equatable {
        let page, tile, ink, muted, profit, loss, selected, hairline, elevated: Color

        func signed(_ n: Double) -> Color {
            if n < -0.0001 { return loss }
            if n > 0.0001 { return profit }
            return ink
        }
    }

    private static func rgb(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat, _ a: CGFloat = 1) -> Color {
        Color(.sRGB, red: r, green: g, blue: b, opacity: a)
    }

    private static let lightPack = Pack(
        page: rgb(242 / 255, 242 / 255, 247 / 255),
        tile: rgb(1, 1, 1),
        ink: rgb(0, 0, 0),
        muted: rgb(142 / 255, 142 / 255, 147 / 255),
        profit: rgb(52 / 255, 199 / 255, 89 / 255),
        loss: rgb(255 / 255, 59 / 255, 48 / 255),
        selected: rgb(0, 122 / 255, 1),
        hairline: rgb(60 / 255, 60 / 255, 67 / 255, 0.18),
        elevated: rgb(242 / 255, 242 / 255, 247 / 255)
    )
    private static let darkPack = Pack(
        page: rgb(0, 0, 0),
        tile: rgb(28 / 255, 28 / 255, 30 / 255),
        ink: rgb(1, 1, 1),
        muted: rgb(142 / 255, 142 / 255, 147 / 255),
        profit: rgb(52 / 255, 199 / 255, 89 / 255),
        loss: rgb(255 / 255, 59 / 255, 48 / 255),
        selected: rgb(10 / 255, 132 / 255, 1),
        hairline: rgb(84 / 255, 84 / 255, 88 / 255, 0.65),
        elevated: rgb(44 / 255, 44 / 255, 46 / 255)
    )

    static func pack(for scheme: ColorScheme) -> Pack {
        scheme == .dark ? darkPack : lightPack
    }

}

private struct HomeTokensKey: EnvironmentKey {
    static let defaultValue = HomeColor.pack(for: .light)
}

private extension EnvironmentValues {
    var homeTokens: HomeColor.Pack {
        get { self[HomeTokensKey.self] }
        set { self[HomeTokensKey.self] = newValue }
    }
}

private enum HomeChrome {
    static func apply() {
        let page = HomeUIColor.page
        let muted = HomeUIColor.muted
        let selected = HomeUIColor.selected
        let ink = HomeUIColor.ink
        let hairline = HomeUIColor.hairline

        let tab = UITabBarAppearance()
        tab.configureWithTransparentBackground()
        tab.backgroundColor = .clear
        tab.shadowColor = .clear
        let item = UITabBarItemAppearance()
        item.normal.iconColor = muted
        item.normal.titleTextAttributes = [.foregroundColor: muted]
        item.selected.iconColor = selected
        item.selected.titleTextAttributes = [.foregroundColor: selected]
        tab.stackedLayoutAppearance = item
        tab.inlineLayoutAppearance = item
        tab.compactInlineLayoutAppearance = item
        UITabBar.appearance().standardAppearance = tab
        UITabBar.appearance().scrollEdgeAppearance = tab
        UITabBar.appearance().backgroundColor = .clear
        UITabBar.appearance().unselectedItemTintColor = muted
        UITabBar.appearance().tintColor = selected
        UITabBar.appearance().isTranslucent = true

        let nav = UINavigationBarAppearance()
        nav.configureWithOpaqueBackground()
        nav.backgroundColor = page
        nav.shadowColor = .clear
        nav.titleTextAttributes = [.foregroundColor: ink]
        nav.largeTitleTextAttributes = [.foregroundColor: ink]
        UINavigationBar.appearance().standardAppearance = nav
        UINavigationBar.appearance().scrollEdgeAppearance = nav
        UINavigationBar.appearance().compactAppearance = nav
        UINavigationBar.appearance().tintColor = selected
        UINavigationBar.appearance().isTranslucent = false

        UITableView.appearance().backgroundColor = page
        UITableView.appearance().separatorColor = hairline
    }
}

private struct AppearanceSegment: UIViewRepresentable {
    @Binding var selection: AppearanceChoice
    var scheme: ColorScheme

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator: NSObject {
        var binding: Binding<AppearanceChoice> = .constant(.system)
        @objc func changed(_ sender: UISegmentedControl) {
            binding.wrappedValue = AppearanceChoice.fromSegment(sender.selectedSegmentIndex)
        }
    }

    func makeUIView(context: Context) -> UISegmentedControl {
        let control = UISegmentedControl(items: ["Light", "Dark", "System"])
        context.coordinator.binding = $selection
        control.selectedSegmentIndex = selection.segmentIndex
        control.addTarget(context.coordinator, action: #selector(Coordinator.changed(_:)), for: .valueChanged)
        control.setContentHuggingPriority(.defaultLow, for: .horizontal)
        control.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        applyChrome(control)
        return control
    }

    func updateUIView(_ control: UISegmentedControl, context: Context) {
        context.coordinator.binding = $selection
        if control.selectedSegmentIndex != selection.segmentIndex {
            control.selectedSegmentIndex = selection.segmentIndex
        }
        applyChrome(control)
    }

    private func applyChrome(_ control: UISegmentedControl) {
        let dark = scheme == .dark
        let page = dark ? UIColor.black : UIColor(red: 242 / 255, green: 242 / 255, blue: 247 / 255, alpha: 1)
        let thumb = dark
            ? UIColor(red: 44 / 255, green: 44 / 255, blue: 46 / 255, alpha: 1)
            : UIColor.white
        let muted = UIColor(red: 142 / 255, green: 142 / 255, blue: 147 / 255, alpha: 1)
        let ink = dark ? UIColor.white : UIColor.black
        let font = UIFont.systemFont(ofSize: 13, weight: .medium)
        control.overrideUserInterfaceStyle = dark ? .dark : .light
        control.backgroundColor = page
        control.selectedSegmentTintColor = thumb
        let trackImg = Self.resizableFill(page, corner: 7)
        let thumbImg = Self.resizableFill(thumb, corner: 7)
        control.setBackgroundImage(trackImg, for: .normal, barMetrics: .default)
        control.setBackgroundImage(trackImg, for: .highlighted, barMetrics: .default)
        control.setBackgroundImage(thumbImg, for: .selected, barMetrics: .default)
        control.setBackgroundImage(thumbImg, for: [.selected, .highlighted], barMetrics: .default)
        let div = Self.resizableFill(page, size: CGSize(width: 1, height: 1), corner: 0)
        control.setDividerImage(div, forLeftSegmentState: .normal, rightSegmentState: .normal, barMetrics: .default)
        control.setDividerImage(div, forLeftSegmentState: .selected, rightSegmentState: .normal, barMetrics: .default)
        control.setDividerImage(div, forLeftSegmentState: .normal, rightSegmentState: .selected, barMetrics: .default)
        control.setTitleTextAttributes([.font: font, .foregroundColor: muted], for: .normal)
        control.setTitleTextAttributes([.font: font, .foregroundColor: ink], for: .selected)
    }

    private static func resizableFill(_ color: UIColor, size: CGSize = CGSize(width: 16, height: 28), corner: CGFloat) -> UIImage {
        let renderer = UIGraphicsImageRenderer(size: size)
        let img = renderer.image { _ in
            color.setFill()
            UIBezierPath(roundedRect: CGRect(origin: .zero, size: size), cornerRadius: corner).fill()
        }
        let inset = max(corner, 1)
        return img.resizableImage(withCapInsets: UIEdgeInsets(top: inset, left: inset, bottom: inset, right: inset))
    }
}

private struct BagholderListChrome: ViewModifier {
    @Environment(\.homeTokens) private var tokens

    func body(content: Content) -> some View {
        content
            .scrollContentBackground(.hidden)
            .background(tokens.page)
            .listRowSeparatorTint(tokens.hairline)
            .tint(tokens.selected)
    }
}

private extension View {
    func bagholderListChrome() -> some View {
        modifier(BagholderListChrome())
    }

    func bagholderBottomScrollClearance() -> some View {
        safeAreaInset(edge: .bottom, spacing: 0) {
            Color.clear
                .frame(height: HomeLayout.scrollBottomInset)
                .allowsHitTesting(false)
        }
    }

    func bagholderHideNavBar() -> some View {
        self
            .toolbar(.hidden, for: .navigationBar)
            .navigationBarTitleDisplayMode(.inline)
    }
}

private enum HomeLayout {
    static let side: CGFloat = 16
    static let gutter: CGFloat = 8
    static let corner: CGFloat = 14
    static let chartHeight: CGFloat = 72
    static let tabBarHeight: CGFloat = 49
    static let tabBarLift: CGFloat = 8
    static let tabBarSide: CGFloat = 16
    static let scrollBottomInset: CGFloat = 65
}

struct RootView: View {
    @StateObject private var session: SessionStore
    @StateObject private var journal: Journal
    @StateObject private var filters: FilterStore
    @StateObject private var appearance: AppearanceStore
    @State private var tab = 0
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.colorScheme) private var colorScheme

    init() {
        _session = StateObject(wrappedValue: SessionStore())
        _journal = StateObject(wrappedValue: Journal())
        _filters = StateObject(wrappedValue: FilterStore())
        _appearance = StateObject(wrappedValue: AppearanceStore())
        HomeChrome.apply()
    }

    var body: some View {
        let scheme: ColorScheme = appearance.choice.preferredColorScheme ?? colorScheme
        let tokens = HomeColor.pack(for: scheme)
        ZStack(alignment: .bottom) {
            ZStack {
                NavigationStack {
                    HomeView(session: session, journal: journal)
                }
                .opacity(tab == 0 ? 1 : 0)
                .allowsHitTesting(tab == 0)
                .zIndex(tab == 0 ? 1 : 0)

                NavigationStack {
                    ClosedTradesView(session: session, journal: journal)
                }
                .opacity(tab == 1 ? 1 : 0)
                .allowsHitTesting(tab == 1)
                .zIndex(tab == 1 ? 1 : 0)

                NavigationStack {
                    ActivityView(session: session, journal: journal)
                }
                .opacity(tab == 2 ? 1 : 0)
                .allowsHitTesting(tab == 2)
                .zIndex(tab == 2 ? 1 : 0)

                NavigationStack {
                    OpenLotsView(session: session, journal: journal)
                }
                .opacity(tab == 3 ? 1 : 0)
                .allowsHitTesting(tab == 3)
                .zIndex(tab == 3 ? 1 : 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .ignoresSafeArea(.container, edges: .bottom)

            FloatingTabBar(selection: $tab)
                .padding(.horizontal, HomeLayout.tabBarSide)
                .padding(.bottom, HomeLayout.tabBarLift)
        }
        .ignoresSafeArea(.container, edges: .bottom)
        .environment(\.homeTokens, tokens)
        .environment(\.colorScheme, scheme)
        .environmentObject(filters)
        .environmentObject(appearance)
        .preferredColorScheme(appearance.preferredColorScheme)
        .tint(tokens.selected)
        .toolbarBackground(tokens.page, for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
        .background(tokens.page.ignoresSafeArea())
        .onAppear { journal.handleAppear(session: session) }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active {
                journal.handleAppear(session: session)
            }
        }
        .onChange(of: session.pullToken) { _, _ in
            journal.handleSessionChange(session: session)
        }
    }
}

struct HomeView: View {
    @Environment(\.homeTokens) private var tokens
    @ObservedObject var session: SessionStore
    @ObservedObject var journal: Journal
    @EnvironmentObject private var filters: FilterStore
    @State private var showSettings = false
    @State private var showFilters = false
    @State private var showLogin = false
    @State private var shown: WSPullResult?

    var body: some View {
        Group {
            if journal.result != nil || session.connected || journal.phase == .pulling {
                liveHome(shown)
            } else {
                waitingHome
            }
        }
        .background(tokens.page)
        .bagholderHideNavBar()
        .onAppear { refreshShown() }
        .onChange(of: filters.current) { _, _ in refreshShown() }
        .onChange(of: journal.result?.closed.count) { _, _ in refreshShown() }
        .onChange(of: journal.result?.nav.count) { _, _ in refreshShown() }
        .onChange(of: journal.result?.listings.count) { _, _ in refreshShown() }
        .onChange(of: journal.phase) { _, _ in refreshShown() }
        .sheet(isPresented: $showSettings) {
            SettingsView(session: session, journal: journal)
        }
        .sheet(isPresented: $showFilters) {
            FiltersSheet(journal: journal)
                .environmentObject(filters)
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
                .presentationBackground(tokens.tile)
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
        .bagholderBottomScrollClearance()
        .background(tokens.page)
    }

    @ViewBuilder
    private var waitingBody: some View {
        if case .failed(let msg) = journal.phase {
            VStack(spacing: 16) {
                Text(msg)
                    .font(.system(size: 17))
                    .foregroundStyle(tokens.loss)
                    .multilineTextAlignment(.center)
                connectButton
            }
        } else {
            VStack(spacing: 16) {
                Text("Connect Wealthsimple")
                    .font(.system(size: 22, weight: .semibold))
                    .foregroundStyle(tokens.ink)
                    .multilineTextAlignment(.center)
                Text("Sign in to see Home.")
                    .font(.system(size: 15))
                    .foregroundStyle(tokens.muted)
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
                        .fill(tokens.selected)
                )
        }
        .frame(maxWidth: 280)
        .accessibilityLabel("Connect")
    }

    private var settingsBar: some View {
        SettingsBar(
            journal: journal,
            filtersOn: filters.current.isActive,
            onFilters: { showFilters = true },
            onSettings: { showSettings = true }
        )
    }

    private func refreshShown() {
        shown = journal.result.map { WSPull.dashboard($0, filters: filters.current) }
    }

    private func liveHome(_ shown: WSPullResult?) -> some View {
        let dash = WSPull.emDash
        let m = shown?.metrics
        return ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: 0) {
                settingsBar

                VStack(alignment: .leading, spacing: 3) {
                    Text(shown.map { WSPull.formatCad($0.metrics.realizedPnlCad, digits: 2) } ?? dash)
                        .font(.system(size: 38, weight: .bold))
                        .tracking(-1.4)
                        .lineSpacing(0)
                        .foregroundStyle(tokens.ink)
                    Text("Realized P&L")
                        .font(.system(size: 15))
                        .foregroundStyle(tokens.muted)
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
                        value: (m?.maxWinSymbol.isEmpty == false) ? WSPull.formatCad(m!.maxWinPnl, digits: 2) : dash,
                        subtitle: (m?.maxWinSymbol.isEmpty == false) ? m!.maxWinSymbol : dash,
                        valueColor: tokens.profit
                    )
                    MetricCard(
                        title: "Biggest loser",
                        value: (m?.maxLossSymbol.isEmpty == false) ? WSPull.formatCad(m!.maxLossPnl, digits: 2) : dash,
                        subtitle: (m?.maxLossSymbol.isEmpty == false) ? m!.maxLossSymbol : dash,
                        valueColor: tokens.loss
                    )
                    MetricCard(
                        title: "Profit factor",
                        value: shown.map { WSPull.formatProfitFactor($0.metrics.profitFactor) } ?? dash,
                        subtitle: shown.map { WSPull.formatCad($0.metrics.grossProfit, digits: 2) + " W, " + WSPull.formatCad($0.metrics.grossLoss, digits: 2) + " L" } ?? dash
                    )
                    MetricCard(
                        title: "Expectancy",
                        value: shown.map { WSPull.formatCad($0.metrics.expectancy, digits: 2) } ?? dash,
                        subtitle: shown.map { WSPull.formatCad($0.metrics.avgWin, digits: 2) + " \u{00B7} " + WSPull.formatCad($0.metrics.avgLoss, digits: 2) } ?? dash
                    )
                    MetricCard(
                        title: "Win rate",
                        value: shown.map { WSPull.formatWinRate($0.metrics.winRate) } ?? dash,
                        subtitle: shown.map { "\($0.metrics.winCount) W, \($0.metrics.lossCount) L, \($0.metrics.evenCount) BE" } ?? dash
                    )
                    MetricCard(
                        title: "Avg annualized",
                        value: shown.map { $0.avgAnnualized } ?? dash,
                        subtitle: (shown?.avgAnnualizedSubtitle.isEmpty == false) ? shown!.avgAnnualizedSubtitle : dash
                    )
                }
                .padding(.horizontal, HomeLayout.side)
                .padding(.top, 12)

                EquityCurveCard(points: shown?.nav)
                    .padding(.horizontal, HomeLayout.side)
                    .padding(.top, HomeLayout.gutter)
                MonthlyPnLCard(bars: shown?.monthly)
                    .padding(.horizontal, HomeLayout.side)
                    .padding(.top, HomeLayout.gutter)
                AnnualPerformanceCard(years: shown?.years)
                    .padding(.horizontal, HomeLayout.side)
                    .padding(.top, HomeLayout.gutter)
            }
        }
        .bagholderBottomScrollClearance()
        .background(tokens.page)
    }
}

struct MetricCard: View {
    @Environment(\.homeTokens) private var tokens
    let title: String
    let value: String
    var subtitle: String? = nil
    var valueColor: Color? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(tokens.muted)
                .lineLimit(1)
                .padding(.bottom, 4)
            Text(value)
                .font(.system(size: 20, weight: .bold))
                .tracking(-0.6)
                .foregroundStyle(valueColor ?? tokens.ink)
                .lineLimit(1)
                .minimumScaleFactor(0.85)
            if let subtitle {
                Text(subtitle)
                    .font(.system(size: 12))
                    .foregroundStyle(tokens.muted)
                    .lineLimit(1)
                    .allowsTightening(true)
                    .minimumScaleFactor(0.65)
                    .padding(.top, 3)
            }
        }
        .padding(EdgeInsets(top: 11, leading: 12, bottom: 10, trailing: 12))
        .frame(maxWidth: .infinity, minHeight: 72, maxHeight: .infinity, alignment: .topLeading)
        .background(
            RoundedRectangle(cornerRadius: HomeLayout.corner, style: .continuous)
                .fill(tokens.tile)
        )
    }
}

struct EquityCurveCard: View {
    @Environment(\.homeTokens) private var tokens
    var points: [WSNavPoint]? = nil
    @Environment(\.colorScheme) private var colorScheme
    @State private var pressActive = false
    @State private var finger = CGPoint.zero
    @State private var chartSize: CGSize = .zero

    private var selectedIndex: Int? {
        guard pressActive, let points, !points.isEmpty, chartSize.width > 0 else { return nil }
        let n = CGFloat(max(points.count - 1, 1))
        let i = Int((finger.x / chartSize.width * n).rounded())
        return min(max(i, 0), points.count - 1)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Equity Curve")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(tokens.muted)
            VStack(alignment: .leading, spacing: 1) {
                if let points, let i = selectedIndex {
                    Text(WSPull.formatCad(points[i].equity, digits: 2))
                        .font(.system(size: 22, weight: .bold))
                        .foregroundStyle(tokens.ink)
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                    Text(WSPull.formatChartDate(points[i].date))
                        .font(.system(size: 13))
                        .foregroundStyle(tokens.muted)
                }
            }
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .frame(height: 44, alignment: .topLeading)
            if let points, !points.isEmpty {
                LiveEquityCurveDrawing(points: points, selectedIndex: selectedIndex, scheme: colorScheme)
                    .equatable()
                    .frame(height: HomeLayout.chartHeight)
                    .contentShape(Rectangle())
                    .chartLongPress(active: $pressActive, location: $finger, size: $chartSize)
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
                .fill(tokens.tile)
        )
    }
}

struct MonthlyPnLCard: View {
    @Environment(\.homeTokens) private var tokens
    var bars: [WSMonthBar]? = nil
    @Environment(\.colorScheme) private var colorScheme
    @State private var pressActive = false
    @State private var finger = CGPoint.zero
    @State private var chartSize: CGSize = .zero

    private var shown: [WSMonthBar] {
        Array((bars ?? []).suffix(18))
    }

    private var selectedIndex: Int? {
        guard pressActive, !shown.isEmpty, chartSize.width > 0 else { return nil }
        let sx = chartSize.width / 330
        let n = CGFloat(max(shown.count, 1))
        let pitch: CGFloat = shown.count <= 7 ? 40 : (300 / n)
        let barW: CGFloat = min(22, max(8, pitch * 0.55))
        let startX: CGFloat = shown.count <= 7 ? 22 : 15
        var best = 0
        var bestDist = CGFloat.infinity
        for i in shown.indices {
            let x = (startX + CGFloat(i) * pitch) * sx
            let cx = x + (barW * sx) / 2
            let d = abs(finger.x - cx)
            if d < bestDist {
                bestDist = d
                best = i
            }
        }
        return best
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Monthly P&L")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(tokens.muted)
            VStack(alignment: .leading, spacing: 1) {
                if let i = selectedIndex {
                    let bar = shown[i]
                    Text(WSPull.formatAxisCad(bar.pnl, digits: 2))
                        .font(.system(size: 22, weight: .bold))
                        .foregroundStyle(tokens.signed(bar.pnl))
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                    Text(WSPull.monthTick(bar.month))
                        .font(.system(size: 13))
                        .foregroundStyle(tokens.muted)
                }
            }
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .frame(height: 44, alignment: .topLeading)
            if let bars, !bars.isEmpty {
                LiveMonthlyPnLDrawing(bars: bars, selectedIndex: selectedIndex, scheme: colorScheme)
                    .equatable()
                    .frame(height: HomeLayout.chartHeight)
                    .contentShape(Rectangle())
                    .chartLongPress(active: $pressActive, location: $finger, size: $chartSize)
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
                .fill(tokens.tile)
        )
    }
}

struct AnnualPerformanceCard: View {
    @Environment(\.homeTokens) private var tokens
    var years: [WSYearRow]? = nil

    private var rows: [(year: String, ret: String, spy: String, vs: String)] {
        (years ?? []).map { ($0.year, $0.ret, $0.spy, $0.vs) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Annual performance")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(tokens.muted)
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
            .foregroundStyle(tokens.muted)
            .padding(.bottom, 4)
            ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                HStack {
                    Text(row.year)
                        .foregroundStyle(tokens.ink)
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
                .fill(tokens.tile)
        )
        .accessibilityLabel("Annual performance")
    }

    private func percentText(_ value: String) -> Text {
        let loss = value.contains("\u{2212}") || value.hasPrefix("-")
        let profit = value.hasPrefix("+")
        return Text(value)
            .foregroundStyle(loss ? tokens.loss : profit ? tokens.profit : tokens.muted)
    }
}

struct ClosedTradesView: View {
    @Environment(\.homeTokens) private var tokens
    @ObservedObject var session: SessionStore
    @ObservedObject var journal: Journal
    @EnvironmentObject private var filters: FilterStore
    @State private var showSettings = false
    @State private var showFilters = false

    private var rows: [WSClosedTrade] {
        guard let result = journal.result else { return [] }
        let visible = WSPull.filteredTrades(
            result.closed,
            filters: filters.current,
            activities: result.activities,
            listings: result.listings
        )
        return WSPull.closedTradesTable(visible, activities: result.activities)
    }

    var body: some View {
        VStack(spacing: 0) {
            ScreenTitleBar(
                title: "Closed trades",
                filtersOn: filters.current.isActive,
                onFilters: { showFilters = true },
                onSettings: { showSettings = true }
            )
            List {
                if !rows.isEmpty {
                    Section {
                        ForEach(rows) { t in
                            NavigationLink {
                                ClosedTradeDetailView(journal: journal, trade: t)
                            } label: {
                                ClosedTradeRow(trade: t)
                            }
                            .listRowInsets(EdgeInsets(top: 8, leading: 16, bottom: 8, trailing: 16))
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .bagholderListChrome()
            .listRowBackground(tokens.tile)
            .overlay {
                if rows.isEmpty {
                    ContentUnavailableView(
                        "No closed trades",
                        systemImage: "list.bullet",
                        description: Text("They show up here.")
                    )
                }
            }
            .bagholderBottomScrollClearance()
        }
        .background(tokens.page)
        .bagholderHideNavBar()
        .sheet(isPresented: $showSettings) {
            SettingsView(session: session, journal: journal)
        }
        .sheet(isPresented: $showFilters) {
            FiltersSheet(journal: journal)
                .environmentObject(filters)
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
                .presentationBackground(tokens.tile)
        }
    }
}

private struct ClosedTradeRow: View {
    @Environment(\.homeTokens) private var tokens
    let trade: WSClosedTrade

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(trade.symbol)
                    .font(.headline)
                    .foregroundStyle(tokens.ink)
                    .lineLimit(2)
                    .truncationMode(.tail)
                Text(trade.displaySide + " \u{00B7} " + WSPull.formatCloseDate(trade.exitDate))
                    .font(.subheadline)
                    .foregroundStyle(tokens.muted)
            }
            Spacer(minLength: 8)
            Text(WSPull.formatCad(trade.pnl, digits: 2))
                .font(.system(size: 17, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(tokens.signed(trade.pnl))
        }
    }
}

private struct ClosedTradeDetailView: View {
    @Environment(\.homeTokens) private var tokens
    @ObservedObject var journal: Journal
    let trade: WSClosedTrade

    private var activities: [WSActivity] {
        journal.result?.activities ?? []
    }

    private var listings: [WSSecurityListing] {
        journal.result?.listings ?? []
    }

    private var executionActs: [WSActivity] {
        WSPull.activitiesInGroup(trade, activities: activities)
    }

    private var executionCount: Int {
        executionActs.isEmpty ? trade.slices.count : executionActs.count
    }

    private var listingLine: String {
        WSPull.listingLine(trade, activities: activities, listings: listings)
    }

    var body: some View {
        List {
            Section {
                fact("In", trade.entryDate)
                fact("Out", trade.exitDate)
                fact("Entry", WSPull.formatCad(trade.entryPrice, digits: 2))
                fact("Exit", WSPull.formatCad(trade.exitPrice, digits: 2))
                LabeledContent("P&L $") {
                    Text(WSPull.formatCad(trade.pnl, digits: 2))
                        .foregroundStyle(tokens.signed(trade.pnl))
                        .monospacedDigit()
                }
                LabeledContent("P&L %") {
                    Text(WSPull.formatPct(WSPull.closedPnlPct(trade)))
                        .foregroundStyle(tokens.signed(trade.pnl))
                        .monospacedDigit()
                }
                fact("Hold", WSPull.formatHold(trade.holdDays))
                fact("Currency", trade.currency)
            } header: {
                VStack(alignment: .leading, spacing: 2) {
                    Text(trade.symbol)
                        .font(.headline)
                        .foregroundStyle(tokens.ink)
                        .lineLimit(2)
                        .truncationMode(.tail)
                    if !listingLine.isEmpty {
                        Text(listingLine)
                            .font(.subheadline)
                            .foregroundStyle(tokens.muted)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                }
                .textCase(nil)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.bottom, 8)
            }
            Section {
                if executionActs.isEmpty && trade.slices.isEmpty {
                    ContentUnavailableView("No executions", systemImage: "list.bullet")
                } else if !executionActs.isEmpty {
                    ForEach(executionActs, id: \.id) { a in
                        ExecutionActivityRow(activity: a)
                    }
                } else {
                    ForEach(trade.slices) { s in
                        ExecutionSliceRow(slice: s)
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .bagholderListChrome()
        .listRowBackground(tokens.tile)
        .bagholderBottomScrollClearance()
        .navigationTitle("Executions (\(executionCount))")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .toolbarBackground(tokens.page, for: .navigationBar)
        .toolbarBackground(.visible, for: .navigationBar)
    }

    private func fact(_ label: String, _ value: String) -> some View {
        LabeledContent(label) {
            Text(value)
                .monospacedDigit()
        }
    }
}

private struct ExecutionActivityRow: View {
    @Environment(\.homeTokens) private var tokens
    let activity: WSActivity

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(WSPull.formatExecutionWhen(activity))
                    .font(.body)
                    .foregroundStyle(tokens.ink)
                Text(
                    WSPull.executionSide(activity)
                        + " · "
                        + WSPull.formatQty(activity.quantity)
                        + " · "
                        + WSPull.formatCad(activity.unitPrice, digits: 2)
                )
                .font(.subheadline)
                .foregroundStyle(tokens.muted)
                .monospacedDigit()
            }
            Spacer(minLength: 8)
            Text(WSPull.formatCad(activity.netCashAmount, digits: 2))
                .font(.system(size: 17, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(tokens.signed(activity.netCashAmount))
        }
        .listRowInsets(EdgeInsets(top: 8, leading: 16, bottom: 8, trailing: 16))
    }
}

private struct ExecutionSliceRow: View {
    @Environment(\.homeTokens) private var tokens
    let slice: WSClosedTrade

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(WSPull.formatCloseDate(slice.exitDate))
                    .font(.body)
                    .foregroundStyle(tokens.ink)
                Text(
                    WSPull.executionSideName(slice.side)
                        + " · "
                        + WSPull.formatQty(slice.quantity)
                        + " · "
                        + WSPull.formatCad(slice.exitPrice, digits: 2)
                )
                .font(.subheadline)
                .foregroundStyle(tokens.muted)
                .monospacedDigit()
            }
            Spacer(minLength: 8)
            Text(WSPull.formatCad(slice.pnl, digits: 2))
                .font(.system(size: 17, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(tokens.signed(slice.pnl))
        }
        .listRowInsets(EdgeInsets(top: 8, leading: 16, bottom: 8, trailing: 16))
    }
}

struct ActivityView: View {
    @Environment(\.homeTokens) private var tokens
    @ObservedObject var session: SessionStore
    @ObservedObject var journal: Journal
    @EnvironmentObject private var filters: FilterStore
    @State private var showSettings = false
    @State private var showFilters = false

    private var rows: [WSActivity] {
        guard let result = journal.result else { return [] }
        return WSPull.filteredActivities(result.activities, filters: filters.current, listings: result.listings)
            .sorted { a, b in
                let da = WSPull.activityWhen(a)
                let db = WSPull.activityWhen(b)
                if da != db { return da > db }
                return a.id > b.id
            }
    }

    var body: some View {
        VStack(spacing: 0) {
            ScreenTitleBar(
                title: "Activity",
                filtersOn: filters.current.isActive,
                onFilters: { showFilters = true },
                onSettings: { showSettings = true }
            )
            List {
                if !rows.isEmpty {
                    Section {
                        ForEach(rows, id: \.id) { a in
                            HStack(alignment: .center, spacing: 12) {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(a.symbol.isEmpty ? (a.activityType.isEmpty ? "Activity" : a.activityType) : a.symbol)
                                        .font(.headline)
                                        .foregroundStyle(tokens.ink)
                                        .lineLimit(2)
                                    Text(WSPull.activityWhen(a) + (a.accountType.isEmpty ? "" : " · " + a.accountType))
                                        .font(.subheadline)
                                        .foregroundStyle(tokens.muted)
                                }
                                Spacer(minLength: 8)
                                Text(WSPull.formatCad(a.netCashAmount, digits: 2))
                                    .font(.system(size: 17, weight: .semibold))
                                    .monospacedDigit()
                                    .foregroundStyle(tokens.signed(a.netCashAmount))
                            }
                            .listRowInsets(EdgeInsets(top: 8, leading: 16, bottom: 8, trailing: 16))
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .bagholderListChrome()
            .listRowBackground(tokens.tile)
            .overlay {
                if rows.isEmpty {
                    ContentUnavailableView(
                        "No activity",
                        systemImage: "doc.text",
                        description: Text("It shows up here.")
                    )
                }
            }
            .bagholderBottomScrollClearance()
        }
        .background(tokens.page)
        .bagholderHideNavBar()
        .sheet(isPresented: $showSettings) {
            SettingsView(session: session, journal: journal)
        }
        .sheet(isPresented: $showFilters) {
            FiltersSheet(journal: journal)
                .environmentObject(filters)
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
                .presentationBackground(tokens.tile)
        }
    }
}

struct OpenLotsView: View {
    @Environment(\.homeTokens) private var tokens
    @ObservedObject var session: SessionStore
    @ObservedObject var journal: Journal
    @EnvironmentObject private var filters: FilterStore
    @State private var showSettings = false
    @State private var showFilters = false

    private var rows: [WSOpenLot] {
        guard let result = journal.result else { return [] }
        let lots = WSPull.openLots(from: result.activities)
        return WSPull.filteredOpenLots(lots, filters: filters.current, activities: result.activities, listings: result.listings)
            .sorted { a, b in
                if a.date != b.date { return a.date > b.date }
                return a.id > b.id
            }
    }

    var body: some View {
        VStack(spacing: 0) {
            ScreenTitleBar(
                title: "Open lots",
                filtersOn: filters.current.isActive,
                onFilters: { showFilters = true },
                onSettings: { showSettings = true }
            )
            List {
                if !rows.isEmpty {
                    Section {
                        ForEach(rows) { lot in
                            HStack(alignment: .center, spacing: 12) {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(lot.symbol)
                                        .font(.headline)
                                        .foregroundStyle(tokens.ink)
                                        .lineLimit(2)
                                    Text(lot.direction + " · " + WSPull.formatQty(lot.quantity) + " · " + WSPull.formatCad(lot.price, digits: 2))
                                        .font(.subheadline)
                                        .foregroundStyle(tokens.muted)
                                        .monospacedDigit()
                                }
                                Spacer(minLength: 8)
                                Text(WSPull.formatCloseDate(lot.date))
                                    .font(.subheadline)
                                    .foregroundStyle(tokens.muted)
                            }
                            .listRowInsets(EdgeInsets(top: 8, leading: 16, bottom: 8, trailing: 16))
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .bagholderListChrome()
            .listRowBackground(tokens.tile)
            .overlay {
                if rows.isEmpty {
                    ContentUnavailableView(
                        "No open lots",
                        systemImage: "hexagon",
                        description: Text("They show up here.")
                    )
                }
            }
            .bagholderBottomScrollClearance()
        }
        .background(tokens.page)
        .bagholderHideNavBar()
        .sheet(isPresented: $showSettings) {
            SettingsView(session: session, journal: journal)
        }
        .sheet(isPresented: $showFilters) {
            FiltersSheet(journal: journal)
                .environmentObject(filters)
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
                .presentationBackground(tokens.tile)
        }
    }
}



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
        guard let snap = try? JSONDecoder().decode(WSPullResult.self, from: data) else { return nil }
        let rematched = WSPull.rematchStored(snap)
        if let out = try? JSONEncoder().encode(rematched) {
            try? out.write(to: url, options: .atomic)
        }
        return rematched
    }

    static func save(_ result: WSPullResult) {
        let snap = result
        Task.detached {
            guard let data = try? JSONEncoder().encode(snap) else { return }
            try? data.write(to: url, options: .atomic)
        }
    }

    static func clear() {
        try? FileManager.default.removeItem(at: url)
    }

    static func modifiedAt() -> Date? {
        (try? FileManager.default.attributesOfItem(atPath: url.path)[.modificationDate]) as? Date
    }
}

@MainActor
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
    @Published var syncStep: String = ""
    @Published var lastSync: Date?
    private var noNewShownAt: Date?
    private var task: Task<Void, Never>?
    private var listingsTask: Task<Void, Never>?
    private var pullGeneration = 0
    private static let lastSyncKey = "bagholder.lastSync"

    var isLive: Bool { result != nil }

    init() {
        if let snap = LastPullStore.load() {
            result = snap
            phase = .ready
        }
        lastSync = UserDefaults.standard.object(forKey: Self.lastSyncKey) as? Date
            ?? LastPullStore.modifiedAt()
    }

    /// ledger.html wsStatusText / relSync.
    func headerStatus(now: Date = Date()) -> String {
        if !syncStep.isEmpty {
            if syncStep == "No new transactions",
               let shown = noNewShownAt,
               now.timeIntervalSince(shown) >= 45 {
                // fall through to relSync of the last real pull
            } else {
                return syncStep
            }
        }
        if case .failed(let msg) = phase { return msg }
        if let lastSync { return Self.relSync(lastSync, now: now) }
        if phase == .pulling { return "Fetching accounts…" }
        return ""
    }

    /// ledger.html relSync (4168-4176).
    static func relSync(_ date: Date, now: Date) -> String {
        let sec = Int((now.timeIntervalSince(date)).rounded())
        if sec < 45 { return "Synced just now" }
        if sec < 120 { return "Synced 1 min ago" }
        if sec < 3600 { return "Synced \(Int((Double(sec) / 60.0).rounded())) min ago" }
        return "Synced \(Int((Double(sec) / 3600.0).rounded()))h ago"
    }

    /// First paint: leftover Keychain must not start a download.
    /// A saved result is shown immediately and refreshed in the background.
    func handleAppear(session: SessionStore) {
        guard session.connected else { return }
        if phase == .pulling { return }
        if result == nil {
            pull(showProgress: true)
            return
        }
        if Self.activityPullDue(lastSync: lastSync) {
            pull(showProgress: false)
        }
    }

    /// store.py activity_pull_due: America/Edmonton, Mon-Fri, at/after 14:00.
    static func activityPullDue(lastSync: Date?, now: Date = Date()) -> Bool {
        guard let tz = TimeZone(identifier: "America/Edmonton") else { return false }
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = tz
        let weekday = cal.component(.weekday, from: now)
        if weekday == 1 || weekday == 7 { return false }
        var parts = cal.dateComponents([.year, .month, .day], from: now)
        parts.hour = 14
        parts.minute = 0
        parts.second = 0
        parts.nanosecond = 0
        parts.timeZone = tz
        guard let close = cal.date(from: parts) else { return false }
        if now < close { return false }
        guard let last = lastSync else { return true }
        return last < close
    }

    /// Sign-in (`saveOAuthCookie`) starts a download. Disconnect clears the snapshot.
    func handleSessionChange(session: SessionStore) {
        if !session.connected {
            pullGeneration += 1
            task?.cancel()
            task = nil
            listingsTask?.cancel()
            listingsTask = nil
            phase = .idle
            result = nil
            syncStep = ""
            lastSync = nil
            UserDefaults.standard.removeObject(forKey: Self.lastSyncKey)
            LastPullStore.clear()
            return
        }
        pull(showProgress: result == nil)
    }

    func refresh() {
        phase = .pulling
        syncStep = "Fetching accounts…"
        pull(showProgress: true)
    }

    private func pull(showProgress: Bool) {
        task?.cancel()
        listingsTask?.cancel()
        pullGeneration += 1
        let gen = pullGeneration
        noNewShownAt = nil
        if showProgress {
            phase = .pulling
            syncStep = "Fetching accounts…"
        }
        task = Task { [weak self] in
            guard let self else { return }
            guard let rec = Keychain.load(),
                  let cookie = rec["oauth_cookie"] as? String
            else {
                self.showPullError(gen: gen, "no session")
                return
            }
            let wssdi = rec["wssdi"] as? String
            let storedActs = self.result?.activities ?? []
            let storedNav = self.result?.nav ?? []
            let storedNavByAccount = self.result?.navByAccount ?? [:]
            let storedListings = self.result?.listings ?? []
            do {
                let snap = try await WSPull.run(
                    oauthCookie: cookie,
                    wssdi: wssdi,
                    storedActivities: storedActs,
                    storedNav: storedNav,
                    storedNavByAccount: storedNavByAccount,
                    storedListings: storedListings
                ) { [weak self] step in
                    Task { @MainActor [weak self] in
                        guard let self else { return }
                        if gen == self.pullGeneration {
                            self.syncStep = step
                        }
                    }
                }
                if Task.isCancelled {
                    self.endPullCancelled(gen: gen)
                    return
                }
                var applied = snap
                if applied.listings.isEmpty, let old = self.result?.listings, !old.isEmpty {
                    applied.listings = old
                }
                self.result = applied
                self.phase = .ready
                if applied.transferredNew {
                    self.syncStep = ""
                    self.noNewShownAt = nil
                    self.lastSync = Date()
                    UserDefaults.standard.set(self.lastSync, forKey: Self.lastSyncKey)
                } else {
                    self.syncStep = "No new transactions"
                    self.noNewShownAt = Date()
                }
                LastPullStore.save(applied)
                let recNow = Keychain.load()
                let cookieNow = (recNow?["oauth_cookie"] as? String) ?? cookie
                let wssdiNow = (recNow?["wssdi"] as? String) ?? wssdi
                let acts = applied.activities
                self.startListings(cookie: cookieNow, wssdi: wssdiNow, activities: acts)
            } catch is CancellationError {
                self.endPullCancelled(gen: gen)
            } catch let url as URLError where url.code == .cancelled {
                self.endPullCancelled(gen: gen)
            } catch WSPullError.graphql(let msg) {
                self.showPullError(gen: gen, msg)
            } catch WSPullError.refresh(let msg) {
                self.showPullError(gen: gen, msg)
            } catch WSPullError.unauthorized {
                self.showPullError(gen: gen, "Wealthsimple token refresh failed")
            } catch WSPullError.noIdentity {
                self.showPullError(gen: gen, "no identity")
            } catch WSPullError.noSession {
                self.showPullError(gen: gen, "no session")
            } catch let url as URLError {
                self.showPullError(gen: gen, url.localizedDescription)
            } catch {
                self.showPullError(gen: gen, error.localizedDescription)
            }
        }
    }

    private func endPullCancelled(gen: Int) {
        guard gen == pullGeneration else { return }
        if result != nil {
            phase = .ready
            syncStep = ""
        }
    }

    private func showPullError(gen: Int, _ msg: String) {
        guard gen == pullGeneration else { return }
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

    private func startListings(cookie: String, wssdi: String?, activities: [WSActivity]) {
        let have = result?.listings ?? []
        if !WSPull.needsListingFetch(activities: activities, have: have) { return }
        listingsTask?.cancel()
        let cookieCopy = cookie
        let wssdiCopy = wssdi
        let actsCopy = activities
        let haveCopy = have
        listingsTask = Task { [weak self] in
            guard let self else { return }
            self.syncStep = "Fetching listings…"
            let fetched = await WSPull.fetchListings(
                oauthCookie: cookieCopy,
                wssdi: wssdiCopy,
                activities: actsCopy,
                have: haveCopy
            )
            if Task.isCancelled { return }
            self.mergeListings(fetched)
            if self.syncStep == "Fetching listings…" {
                self.syncStep = ""
            }
        }
    }

    private func mergeListings(_ extra: [WSSecurityListing]) {
        guard var snap = result else { return }
        var byId: [String: WSSecurityListing] = [:]
        for s in snap.listings { byId[s.id] = s }
        for s in extra {
            if s.name.isEmpty, let old = byId[s.id], !old.name.isEmpty {
                continue
            }
            byId[s.id] = s
        }
        snap.listings = Array(byId.values)
        result = snap
        LastPullStore.save(snap)
    }
}

private extension View {
    func chartLongPress(active: Binding<Bool>, location: Binding<CGPoint>, size: Binding<CGSize>) -> some View {
        overlay {
            ChartLongPressOverlay(active: active, location: location, size: size)
        }
    }
}

/// UILongPress reports location on `.began`. Sequenced SwiftUI LongPress+Drag
/// often has a nil drag until the finger moves, so the hairline missed the first hold.
/// touchesBegan captures x immediately; when the hold recognizes, hairline is already at that x.
private struct ChartLongPressOverlay: UIViewRepresentable {
    @Binding var active: Bool
    @Binding var location: CGPoint
    @Binding var size: CGSize

    func makeCoordinator() -> Coordinator {
        Coordinator(active: $active, location: $location, size: $size)
    }

    func makeUIView(context: Context) -> ChartLongPressUIView {
        let view = ChartLongPressUIView()
        view.coordinator = context.coordinator
        return view
    }

    func updateUIView(_ uiView: ChartLongPressUIView, context: Context) {
        context.coordinator.active = $active
        context.coordinator.location = $location
        context.coordinator.size = $size
        uiView.coordinator = context.coordinator
    }

    final class Coordinator {
        var active: Binding<Bool>
        var location: Binding<CGPoint>
        var size: Binding<CGSize>
        var lastSize: CGSize = .zero
        init(active: Binding<Bool>, location: Binding<CGPoint>, size: Binding<CGSize>) {
            self.active = active
            self.location = location
            self.size = size
        }

        func publishSize(_ s: CGSize) {
            guard s.width > 0, s.height > 0 else { return }
            guard s != lastSize else { return }
            lastSize = s
            size.wrappedValue = s
        }
    }
}

private final class ChartLongPressUIView: UIView {
    var coordinator: ChartLongPressOverlay.Coordinator?
    private var origin: CGPoint?

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        isOpaque = false
        isMultipleTouchEnabled = false
        isUserInteractionEnabled = true
        let press = UILongPressGestureRecognizer(target: self, action: #selector(handleLongPress))
        press.minimumPressDuration = 0.28
        press.allowableMovement = 24
        press.cancelsTouchesInView = false
        press.delaysTouchesBegan = false
        addGestureRecognizer(press)
    }

    required init?(coder: NSCoder) { nil }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        origin = touches.first.map { $0.location(in: self) }
        coordinator?.publishSize(bounds.size)
        super.touchesBegan(touches, with: event)
    }

    @objc private func handleLongPress(_ gesture: UILongPressGestureRecognizer) {
        switch gesture.state {
        case .began:
            let pt = origin ?? gesture.location(in: self)
            coordinator?.publishSize(bounds.size)
            coordinator?.location.wrappedValue = pt
            coordinator?.active.wrappedValue = true
        case .changed:
            coordinator?.location.wrappedValue = gesture.location(in: self)
            coordinator?.active.wrappedValue = true
        case .ended, .cancelled, .failed:
            origin = nil
            coordinator?.active.wrappedValue = false
        default:
            break
        }
    }
}

private struct LiveEquityCurveDrawing: View, Equatable {
    let points: [WSNavPoint]
    var selectedIndex: Int? = nil
    var scheme: ColorScheme

    private static let dim: Double = 0.30

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.selectedIndex == rhs.selectedIndex
            && lhs.scheme == rhs.scheme
            && lhs.points.count == rhs.points.count
            && lhs.points.last?.date == rhs.points.last?.date
            && lhs.points.last?.equity == rhs.points.last?.equity
    }

    var body: some View {
        let tokens = HomeColor.pack(for: scheme)
        let vals = points.map(\.equity)
        let lo = vals.min() ?? 0
        let hi = vals.max() ?? 1
        let span: Double = (hi - lo) == 0 ? 1.0 : (hi - lo)
        let fillOpacity = selectedIndex != nil ? 0.12 * Self.dim : 0.12
        let lineColor = selectedIndex != nil ? tokens.profit.opacity(Self.dim) : tokens.profit
        let profit = tokens.profit
        let hairColor = tokens.hairline
        Canvas { context, size in
            guard !vals.isEmpty else { return }
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
            context.fill(fill, with: .color(profit.opacity(fillOpacity)))
            var line = Path()
            line.move(to: pt(0))
            if points.count == 1 {
                line.addLine(to: CGPoint(x: size.width, y: pt(0).y))
            }
            for i in 1..<points.count { line.addLine(to: pt(i)) }
            context.stroke(
                line,
                with: .color(lineColor),
                style: StrokeStyle(lineWidth: 2.4, lineCap: .round, lineJoin: .round)
            )
            if let i = selectedIndex, points.indices.contains(i) {
                let hit = pt(i)
                var fullStroke = Path()
                let loI = max(0, i - 1)
                let hiI = min(points.count - 1, i + 1)
                fullStroke.move(to: pt(loI))
                if loI != hiI {
                    for j in (loI + 1)...hiI { fullStroke.addLine(to: pt(j)) }
                } else {
                    fullStroke.addLine(to: CGPoint(x: min(hit.x + 6, size.width), y: hit.y))
                    fullStroke.move(to: hit)
                    fullStroke.addLine(to: CGPoint(x: max(hit.x - 6, 0), y: hit.y))
                }
                context.stroke(
                    fullStroke,
                    with: .color(profit),
                    style: StrokeStyle(lineWidth: 2.4, lineCap: .round, lineJoin: .round)
                )
                var hairlinePath = Path()
                hairlinePath.move(to: CGPoint(x: hit.x, y: 0))
                hairlinePath.addLine(to: CGPoint(x: hit.x, y: size.height))
                context.stroke(
                    hairlinePath,
                    with: .color(hairColor),
                    style: StrokeStyle(lineWidth: 1, dash: [3.5, 2.5])
                )
                let r: CGFloat = 4
                let dot = Path(ellipseIn: CGRect(x: hit.x - r, y: hit.y - r, width: r * 2, height: r * 2))
                context.fill(dot, with: .color(profit))
            }
        }
    }
}

private struct LiveMonthlyPnLDrawing: View, Equatable {
    let bars: [WSMonthBar]
    var selectedIndex: Int? = nil
    var scheme: ColorScheme

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.selectedIndex == rhs.selectedIndex
            && lhs.scheme == rhs.scheme
            && lhs.bars.count == rhs.bars.count
            && lhs.bars.last?.month == rhs.bars.last?.month
            && lhs.bars.last?.pnl == rhs.bars.last?.pnl
    }

    var body: some View {
        let tokens = HomeColor.pack(for: scheme)
        let profit = tokens.profit
        let loss = tokens.loss
        let hairColor = tokens.hairline
        Canvas { context, size in
            let shown = Array(bars.suffix(18))
            let sx = size.width / 330
            let sy = size.height / 72
            var zero = Path()
            zero.move(to: CGPoint(x: 8 * sx, y: 36 * sy))
            zero.addLine(to: CGPoint(x: 322 * sx, y: 36 * sy))
            context.stroke(zero, with: .color(hairColor), lineWidth: 1)
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
                let faded = selectedIndex != nil && selectedIndex != i
                let fill = (bar.pnl >= 0 ? profit : loss).opacity(faded ? 0.32 : 1)
                context.fill(path, with: .color(fill))
            }
            if let i = selectedIndex, shown.indices.contains(i) {
                let x = (startX + CGFloat(i) * pitch) * sx
                let cx = x + (barW * sx) / 2
                var hairlinePath = Path()
                hairlinePath.move(to: CGPoint(x: cx, y: 0))
                hairlinePath.addLine(to: CGPoint(x: cx, y: size.height))
                context.stroke(
                    hairlinePath,
                    with: .color(hairColor),
                    style: StrokeStyle(lineWidth: 1, dash: [3.5, 2.5])
                )
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
        let cookieCopy = value
        let wssdiCopy = wssdi
        Task {
            await WSPull.stampClientId(oauthCookie: cookieCopy, wssdi: wssdiCopy)
        }
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

enum Keychain {
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

@MainActor
final class FilterStore: ObservableObject {
    static let defaultsKey = "bagholder.filters"
    @Published var current: JournalFilters {
        didSet { persist() }
    }

    init() {
        if let data = UserDefaults.standard.data(forKey: Self.defaultsKey),
           let decoded = try? JSONDecoder().decode(JournalFilters.self, from: data) {
            current = decoded
        } else {
            current = JournalFilters()
        }
    }

    func persist() {
        guard let data = try? JSONEncoder().encode(current) else { return }
        UserDefaults.standard.set(data, forKey: Self.defaultsKey)
    }

    func reset() {
        current = JournalFilters()
    }

    func setClosedIn(_ year: String) {
        if year.isEmpty {
            current.from = ""
            current.to = ""
        } else {
            current.from = year + "-01-01"
            current.to = year + "-12-31"
        }
    }
}

private struct FilterGearButtons: View {
    @Environment(\.homeTokens) private var tokens
    var filtersOn: Bool
    var onFilters: () -> Void
    var onSettings: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Button(action: onFilters) {
                ZStack(alignment: .topTrailing) {
                    Image(systemName: "line.3.horizontal.decrease")
                        .font(.system(size: 17, weight: .regular))
                        .foregroundStyle(tokens.muted)
                        .frame(width: 24, height: 24)
                    if filtersOn {
                        Circle()
                            .fill(tokens.profit)
                            .frame(width: 8, height: 8)
                            .offset(x: 2, y: -2)
                    }
                }
            }
            .accessibilityLabel("Filters")
            Button(action: onSettings) {
                Image(systemName: "gearshape")
                    .font(.system(size: 17, weight: .regular))
                    .foregroundStyle(tokens.muted)
                    .frame(width: 24, height: 24)
            }
            .accessibilityLabel("Settings")
        }
    }
}

struct SettingsBar: View {
    @Environment(\.homeTokens) private var tokens
    @ObservedObject var journal: Journal
    var filtersOn: Bool
    var onFilters: () -> Void
    var onSettings: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            TimelineView(.periodic(from: .now, by: 30)) { context in
                Text(journal.headerStatus(now: context.date))
                    .font(.system(size: 12))
                    .foregroundStyle(tokens.muted)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            FilterGearButtons(filtersOn: filtersOn, onFilters: onFilters, onSettings: onSettings)
        }
        .padding(.horizontal, 14)
        .frame(height: 36)
    }
}

private struct ScreenTitleBar: View {
    @Environment(\.homeTokens) private var tokens
    let title: String
    var filtersOn: Bool
    var onFilters: () -> Void
    var onSettings: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Text(title)
                .font(.system(size: 34, weight: .bold))
                .foregroundStyle(tokens.ink)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
            Spacer(minLength: 8)
            FilterGearButtons(filtersOn: filtersOn, onFilters: onFilters, onSettings: onSettings)
        }
        .padding(.leading, 16)
        .padding(.trailing, 14)
    }
}

private struct FloatingTabBar: View {
    @Environment(\.homeTokens) private var tokens
    @Binding var selection: Int

    var body: some View {
        HStack(spacing: 0) {
            tabButton(0, title: "Home", systemImage: "square.grid.2x2")
            tabButton(1, title: "Closed trades", systemImage: "line.3.horizontal")
            tabButton(2, title: "Activity", systemImage: "doc.text")
            tabButton(3, title: "Open lots", systemImage: "hexagon")
        }
        .frame(height: HomeLayout.tabBarHeight)
        .background(tokens.page)
    }

    private func tabButton(_ index: Int, title: String, systemImage: String) -> some View {
        let on = selection == index
        return Button {
            selection = index
        } label: {
            VStack(spacing: 3) {
                Image(systemName: systemImage)
                    .font(.system(size: 17, weight: .regular))
                    .frame(height: 22)
                Text(title)
                    .font(.system(size: 10, weight: .medium))
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            .foregroundStyle(on ? tokens.selected : tokens.muted)
            .frame(maxWidth: .infinity)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

struct FiltersSheet: View {
    @Environment(\.homeTokens) private var tokens
    @ObservedObject var journal: Journal
    @EnvironmentObject private var filters: FilterStore
    @Environment(\.dismiss) private var dismiss

    private var activities: [WSActivity] { journal.result?.activities ?? [] }
    private var listings: [WSSecurityListing] { journal.result?.listings ?? [] }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    NavigationLink {
                        FilterStringPick(
                            title: "Account",
                            allLabel: "All",
                            options: WSPull.filterAccountNames(activities),
                            selection: accountBinding
                        )
                    } label: {
                        LabeledContent("Account", value: filters.current.account.isEmpty ? "All" : filters.current.account)
                    }
                    NavigationLink {
                        FilterStringPick(
                            title: "Exchange",
                            allLabel: "All",
                            options: WSPull.filterExchangeNames(listings),
                            selection: exchangeBinding
                        )
                    } label: {
                        LabeledContent("Exchange", value: filters.current.exchange.isEmpty ? "All" : filters.current.exchange)
                    }
                    NavigationLink {
                        FilterStringPick(
                            title: "Symbol",
                            allLabel: "All",
                            options: WSPull.filterSymbolNames(activities),
                            selection: symbolBinding
                        )
                    } label: {
                        LabeledContent("Symbol", value: filters.current.symbol.isEmpty ? "All" : filters.current.symbol)
                    }
                    NavigationLink {
                        ClosedInPick(years: WSPull.filterClosedYears(activities))
                    } label: {
                        LabeledContent("Closed in", value: closedInLabel)
                    }
                    NavigationLink {
                        DateFilterPick(title: "From", value: fromBinding)
                    } label: {
                        LabeledContent("From", value: filters.current.from.isEmpty ? "All" : filters.current.from)
                    }
                    NavigationLink {
                        DateFilterPick(title: "To", value: toBinding)
                    } label: {
                        LabeledContent("To", value: filters.current.to.isEmpty ? "All" : filters.current.to)
                    }
                    NavigationLink {
                        PriceOpPick()
                    } label: {
                        LabeledContent("Price", value: priceOpLabel)
                    }
                    if filters.current.priceOp == "between" {
                        TextField("From", text: priceMinBinding)
                            .keyboardType(.decimalPad)
                        TextField("To", text: priceMaxBinding)
                            .keyboardType(.decimalPad)
                    } else if !filters.current.priceOp.isEmpty {
                        TextField("Price", text: priceMinBinding)
                            .keyboardType(.decimalPad)
                    }
                }
                if filters.current.isActive {
                    Section {
                        Button("Reset filters") {
                            filters.reset()
                        }
                        .foregroundStyle(tokens.loss)
                    }
                }
            }
            .navigationTitle("Filters")
            .navigationBarTitleDisplayMode(.inline)
            .bagholderListChrome()
            .listRowBackground(tokens.tile)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                        .foregroundStyle(tokens.selected)
                }
            }
        }
        .presentationBackground(tokens.tile)
        .tint(tokens.selected)
    }

    private var closedInLabel: String {
        let y = WSPull.yearFromRange(from: filters.current.from, to: filters.current.to)
        return y.isEmpty ? "All years" : y
    }

    private var priceOpLabel: String {
        switch filters.current.priceOp {
        case "between": return "Between"
        case "under": return "Under"
        case "equal": return "Equal"
        case "over": return "Over"
        default: return "All"
        }
    }

    private var accountBinding: Binding<String> {
        Binding(get: { filters.current.account }, set: { filters.current.account = $0 })
    }
    private var exchangeBinding: Binding<String> {
        Binding(get: { filters.current.exchange }, set: { filters.current.exchange = $0 })
    }
    private var symbolBinding: Binding<String> {
        Binding(get: { filters.current.symbol }, set: { filters.current.symbol = $0 })
    }
    private var fromBinding: Binding<String> {
        Binding(get: { filters.current.from }, set: { filters.current.from = $0 })
    }
    private var toBinding: Binding<String> {
        Binding(get: { filters.current.to }, set: { filters.current.to = $0 })
    }
    private var priceMinBinding: Binding<String> {
        Binding(get: { filters.current.priceMin }, set: { filters.current.priceMin = $0 })
    }
    private var priceMaxBinding: Binding<String> {
        Binding(get: { filters.current.priceMax }, set: { filters.current.priceMax = $0 })
    }
}

private struct FilterStringPick: View {
    @Environment(\.homeTokens) private var tokens
    let title: String
    let allLabel: String
    let options: [String]
    @Binding var selection: String

    var body: some View {
        List {
            row(allLabel, value: "")
            ForEach(options, id: \.self) { opt in
                row(opt, value: opt)
            }
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .bagholderListChrome()
        .listRowBackground(tokens.tile)
    }

    private func row(_ label: String, value: String) -> some View {
        Button {
            selection = value
        } label: {
            HStack {
                Text(label)
                    .foregroundStyle(tokens.ink)
                Spacer()
                if selection == value {
                    Image(systemName: "checkmark")
                        .foregroundStyle(tokens.selected)
                }
            }
        }
    }
}

private struct ClosedInPick: View {
    @Environment(\.homeTokens) private var tokens
    let years: [String]
    @EnvironmentObject private var filters: FilterStore

    private var selected: String {
        WSPull.yearFromRange(from: filters.current.from, to: filters.current.to)
    }

    var body: some View {
        List {
            Button {
                filters.setClosedIn("")
            } label: {
                HStack {
                    Text("All years")
                        .foregroundStyle(tokens.ink)
                    Spacer()
                    if selected.isEmpty {
                        Image(systemName: "checkmark")
                            .foregroundStyle(tokens.selected)
                    }
                }
            }
            ForEach(years, id: \.self) { y in
                Button {
                    filters.setClosedIn(y)
                } label: {
                    HStack {
                        Text(y)
                            .foregroundStyle(tokens.ink)
                        Spacer()
                        if selected == y {
                            Image(systemName: "checkmark")
                                .foregroundStyle(tokens.selected)
                        }
                    }
                }
            }
        }
        .navigationTitle("Closed in")
        .navigationBarTitleDisplayMode(.inline)
        .bagholderListChrome()
        .listRowBackground(tokens.tile)
    }
}

private struct DateFilterPick: View {
    @Environment(\.homeTokens) private var tokens
    let title: String
    @Binding var value: String

    private var picked: Date {
        isoDate(value) ?? Date()
    }

    var body: some View {
        List {
            Button {
                value = ""
            } label: {
                HStack {
                    Text("All")
                        .foregroundStyle(tokens.ink)
                    Spacer()
                    if value.isEmpty {
                        Image(systemName: "checkmark")
                            .foregroundStyle(tokens.selected)
                    }
                }
            }
            DatePicker(
                "Date",
                selection: Binding(
                    get: { picked },
                    set: { value = isoString($0) }
                ),
                displayedComponents: .date
            )
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .bagholderListChrome()
        .listRowBackground(tokens.tile)
    }

    private func isoDate(_ s: String) -> Date? {
        let f = DateFormatter()
        f.calendar = Calendar(identifier: .gregorian)
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(identifier: "America/Edmonton") ?? .current
        f.dateFormat = "yyyy-MM-dd"
        return f.date(from: String(s.prefix(10)))
    }

    private func isoString(_ d: Date) -> String {
        let f = DateFormatter()
        f.calendar = Calendar(identifier: .gregorian)
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = TimeZone(identifier: "America/Edmonton") ?? .current
        f.dateFormat = "yyyy-MM-dd"
        return f.string(from: d)
    }
}

private struct PriceOpPick: View {
    @Environment(\.homeTokens) private var tokens
    @EnvironmentObject private var filters: FilterStore

    private let ops: [(String, String)] = [
        ("", "All"),
        ("between", "Between"),
        ("under", "Under"),
        ("equal", "Equal"),
        ("over", "Over"),
    ]

    var body: some View {
        List {
            ForEach(ops, id: \.0) { op in
                Button {
                    filters.current.priceOp = op.0
                    if op.0.isEmpty {
                        filters.current.priceMin = ""
                        filters.current.priceMax = ""
                    } else if op.0 != "between" {
                        filters.current.priceMax = ""
                    }
                } label: {
                    HStack {
                        Text(op.1)
                            .foregroundStyle(tokens.ink)
                        Spacer()
                        if filters.current.priceOp == op.0 {
                            Image(systemName: "checkmark")
                                .foregroundStyle(tokens.selected)
                        }
                    }
                }
            }
        }
        .navigationTitle("Price")
        .navigationBarTitleDisplayMode(.inline)
        .bagholderListChrome()
        .listRowBackground(tokens.tile)
    }
}

struct SettingsView: View {
    @ObservedObject var session: SessionStore
    @ObservedObject var journal: Journal
    @EnvironmentObject private var appearance: AppearanceStore
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.dismiss) private var dismiss
    @State private var showLogin = false

    var body: some View {
        let scheme: ColorScheme = appearance.choice.preferredColorScheme ?? colorScheme
        let tokens = HomeColor.pack(for: scheme)
        NavigationStack {
            VStack(spacing: 0) {
                ZStack {
                    Text("Settings")
                        .font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(tokens.ink)
                    HStack {
                        Spacer()
                        Button("Done") { dismiss() }
                            .font(.system(size: 17, weight: .regular))
                            .foregroundStyle(tokens.selected)
                            .buttonStyle(.plain)
                    }
                    .padding(.horizontal, 16)
                }
                .frame(height: 44)
                .background(tokens.page)
                List {
                    Section {
                        if session.connected {
                            Button("Refresh") {
                                journal.refresh()
                                dismiss()
                            }
                            .foregroundStyle(tokens.selected)
                            .disabled(journal.phase == .pulling)
                            Button("Disconnect") {
                                session.disconnect()
                            }
                            .foregroundStyle(tokens.loss)
                        } else {
                            Button("Connect") {
                                showLogin = true
                            }
                            .foregroundStyle(tokens.selected)
                        }
                    }
                    .listRowBackground(tokens.tile)
                    .listRowSeparatorTint(tokens.hairline)
                    Section {
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Appearance")
                                .font(.system(size: 13))
                                .foregroundStyle(tokens.muted)
                            AppearanceSegment(selection: $appearance.choice, scheme: scheme)
                                .frame(maxWidth: .infinity)
                                .frame(height: 32)
                        }
                        .listRowInsets(EdgeInsets(top: 12, leading: 16, bottom: 12, trailing: 16))
                        .listRowSeparator(.hidden)
                    }
                    .listRowBackground(tokens.tile)
                }
                .listStyle(.insetGrouped)
                .scrollContentBackground(.hidden)
                .background(tokens.page)
                .listRowSeparatorTint(tokens.hairline)
                .tint(tokens.selected)
            }
            .background(tokens.page)
            .toolbar(.hidden, for: .navigationBar)
            .fullScreenCover(isPresented: $showLogin) {
                ConnectLoginView(session: session, isPresented: $showLogin)
                    .environment(\.homeTokens, tokens)
                    .environment(\.colorScheme, scheme)
                    .preferredColorScheme(appearance.preferredColorScheme)
            }
        }
        .environment(\.homeTokens, tokens)
        .environment(\.colorScheme, scheme)
        .preferredColorScheme(appearance.preferredColorScheme)
        .presentationBackground(tokens.page)
        .presentationDragIndicator(.visible)
        .tint(tokens.selected)
        .background(tokens.page)
    }
}

struct ConnectLoginView: View {
    @Environment(\.homeTokens) private var tokens
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
                ToolbarItem(placement: .topBarLeading) {
                    Button("Cancel") { isPresented = false }
                        .font(.system(size: 17, weight: .regular))
                        .foregroundStyle(tokens.selected)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(Capsule().fill(tokens.elevated))
                        .buttonStyle(.plain)
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

