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
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(HomeExample.realizedPnL)
                        .font(.system(size: 34, weight: .bold, design: .rounded))
                    Text("Realized P&L")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)

                LazyVGrid(
                    columns: [
                        GridItem(.flexible(), spacing: 12),
                        GridItem(.flexible(), spacing: 12),
                    ],
                    spacing: 12
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
                .padding(.horizontal, 16)

                EquityCurveCard()
                    .padding(.horizontal, 16)
                MonthlyPnLCard()
                    .padding(.horizontal, 16)
            }
            .padding(.vertical, 8)
        }
        .background(Color(uiColor: .systemGroupedBackground))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showSettings = true
                } label: {
                    Image(systemName: "gearshape")
                }
                .accessibilityLabel("Settings")
            }
        }
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
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.footnote)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.title3.weight(.semibold))
                .foregroundStyle(valueColor)
            if let subtitle {
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, minHeight: 88, alignment: .topLeading)
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Color(uiColor: .secondarySystemGroupedBackground))
        )
    }
}

struct EquityCurveCard: View {
    private let profit = Color(red: 52 / 255, green: 199 / 255, blue: 89 / 255)

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Equity Curve")
                .font(.subheadline)
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
            .frame(height: 140)
            .accessibilityLabel("Example equity curve")
        }
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Color(uiColor: .secondarySystemGroupedBackground))
        )
    }
}

struct MonthlyPnLCard: View {
    private let profit = Color(red: 52 / 255, green: 199 / 255, blue: 89 / 255)
    private let loss = Color(red: 255 / 255, green: 59 / 255, blue: 48 / 255)

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Monthly P&L")
                .font(.subheadline)
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
            .frame(height: 140)
            .accessibilityLabel("Example monthly P&L")
        }
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
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
