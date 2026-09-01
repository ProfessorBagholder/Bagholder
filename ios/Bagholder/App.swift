import Charts
import SwiftUI

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
    static let equity: [Double] = [1, 1.6, 2.1, 2.4, 3.2, 3.8, 4.5]
    // Mock monthly bars: green, green, red, green (tallest), green, red, green.
    static let monthly: [Double] = [0.6, 0.9, -0.45, 1.2, 0.55, -0.5, 0.7]
}

private enum HomeLayout {
    static let side: CGFloat = 16
    static let gutter: CGFloat = 8
    static let corner: CGFloat = 14
    static let tilePadding: CGFloat = 11
    static let tileMinHeight: CGFloat = 72
    static let chartHeight: CGFloat = 72
    static let titleSize: CGFloat = 12
    static let valueSize: CGFloat = 20
    static let pnlSize: CGFloat = 38
}

struct RootView: View {
    var body: some View {
        TabView {
            NavigationStack {
                HomeView()
            }
            .tabItem { Label("Home", systemImage: "square.grid.2x2") }

            NavigationStack {
                ClosedTradesView()
            }
            .tabItem { Label("Closed trades", systemImage: "list.bullet") }

            NavigationStack {
                EmptyTabView(title: "Activity")
            }
            .tabItem { Label("Activity", systemImage: "doc.text") }

            NavigationStack {
                EmptyTabView(title: "Open lots")
            }
            .tabItem { Label("Open lots", systemImage: "hexagon") }
        }
    }
}

struct HomeView: View {
    @State private var showSettings = false

    var body: some View {
        VStack(alignment: .leading, spacing: HomeLayout.gutter) {
            HStack(alignment: .center, spacing: HomeLayout.gutter) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(HomeExample.realizedPnL)
                        .font(.system(size: HomeLayout.pnlSize, weight: .bold))
                    Text("Realized P&L")
                        .font(.system(size: HomeLayout.titleSize))
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                Button {
                    showSettings = true
                } label: {
                    Image(systemName: "gearshape")
                        .font(.system(size: 17, weight: .regular))
                        .foregroundStyle(.primary)
                        .frame(width: 36, height: 36)
                        .background(
                            Circle()
                                .fill(Color(uiColor: .secondarySystemGroupedBackground))
                        )
                }
                .accessibilityLabel("Settings")
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
                    valueColor: Color(red: 52 / 255, green: 199 / 255, blue: 89 / 255)
                )
                MetricCard(
                    title: "Biggest loser",
                    value: HomeExample.biggestLoser,
                    subtitle: HomeExample.biggestLoserSymbol,
                    valueColor: Color(red: 255 / 255, green: 59 / 255, blue: 48 / 255)
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

            EquityCurveCard()
                .padding(.horizontal, HomeLayout.side)
            MonthlyPnLCard()
                .padding(.horizontal, HomeLayout.side)

            Spacer(minLength: 0)
        }
        .padding(.top, 4)
        .background(Color(uiColor: .systemGroupedBackground))
        .toolbar(.hidden, for: .navigationBar)
        .sheet(isPresented: $showSettings) {
            SettingsStubView()
        }
    }
}

struct MetricCard: View {
    let title: String
    let value: String
    var subtitle: String? = nil
    var valueColor: Color = .primary

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.system(size: HomeLayout.titleSize))
                .foregroundStyle(.secondary)
            Text(value)
                .font(.system(size: HomeLayout.valueSize, weight: .bold))
                .foregroundStyle(valueColor)
            if let subtitle {
                Text(subtitle)
                    .font(.system(size: HomeLayout.titleSize))
                    .foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, minHeight: HomeLayout.tileMinHeight, alignment: .topLeading)
        .padding(HomeLayout.tilePadding)
        .background(
            RoundedRectangle(cornerRadius: HomeLayout.corner, style: .continuous)
                .fill(Color(uiColor: .secondarySystemGroupedBackground))
        )
    }
}

struct EquityCurveCard: View {
    private let profit = Color(red: 52 / 255, green: 199 / 255, blue: 89 / 255)

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Equity Curve")
                .font(.system(size: HomeLayout.titleSize))
                .foregroundStyle(.secondary)
            Chart {
                ForEach(Array(HomeExample.equity.enumerated()), id: \.offset) { index, value in
                    LineMark(
                        x: .value("Point", index),
                        y: .value("Equity", value)
                    )
                    .foregroundStyle(profit)
                    .interpolationMethod(.catmullRom)
                    AreaMark(
                        x: .value("Point", index),
                        y: .value("Equity", value)
                    )
                    .foregroundStyle(profit.opacity(0.18))
                    .interpolationMethod(.catmullRom)
                }
            }
            .chartXAxis(.hidden)
            .chartYAxis(.hidden)
            .frame(height: HomeLayout.chartHeight)
            .accessibilityLabel("Example equity curve")
        }
        .padding(HomeLayout.tilePadding)
        .background(
            RoundedRectangle(cornerRadius: HomeLayout.corner, style: .continuous)
                .fill(Color(uiColor: .secondarySystemGroupedBackground))
        )
    }
}

struct MonthlyPnLCard: View {
    private let profit = Color(red: 52 / 255, green: 199 / 255, blue: 89 / 255)
    private let loss = Color(red: 255 / 255, green: 59 / 255, blue: 48 / 255)

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Monthly P&L")
                .font(.system(size: HomeLayout.titleSize))
                .foregroundStyle(.secondary)
            Chart {
                ForEach(Array(HomeExample.monthly.enumerated()), id: \.offset) { index, value in
                    BarMark(
                        x: .value("Month", index),
                        y: .value("P&L", value)
                    )
                    .foregroundStyle(value >= 0 ? profit : loss)
                    .cornerRadius(3)
                }
            }
            .chartXAxis(.hidden)
            .chartYAxis(.hidden)
            .frame(height: HomeLayout.chartHeight)
            .accessibilityLabel("Example monthly P&L")
        }
        .padding(HomeLayout.tilePadding)
        .background(
            RoundedRectangle(cornerRadius: HomeLayout.corner, style: .continuous)
                .fill(Color(uiColor: .secondarySystemGroupedBackground))
        )
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

struct SettingsStubView: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Text("Connect is not in this build yet.")
                    .foregroundStyle(.secondary)
            }
            .navigationTitle("Settings")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}
