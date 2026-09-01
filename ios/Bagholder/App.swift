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
            List {
                Section("Closed trades") {
                    Text("None yet")
                        .foregroundStyle(.secondary)
                }
            }
            .navigationTitle("Bagholder")
        }
    }
}
