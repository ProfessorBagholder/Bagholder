import SwiftUI

@main
struct BagholderApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    var body: some View {
        NavigationStack {
            ContentUnavailableView(
                "No activity loaded",
                systemImage: "chart.xyaxis.line",
                description: Text("Wealthsimple sign-in is not in this iPhone build yet. This screen is native Swift, not the computer dashboard.")
            )
            .navigationTitle("Bagholder")
        }
    }
}
