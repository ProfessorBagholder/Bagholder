import SwiftUI

@main
struct BagholderApp: App {
    @ObservedObject private var server = LocalServer.shared

    init() {
        LocalServer.shared.startIfNeeded()
    }

    var body: some Scene {
        WindowGroup {
            Group {
                if let url = server.url {
                    WebShell(url: url)
                } else if let message = server.errorMessage {
                    Text(message)
                        .padding()
                } else {
                    ProgressView("Starting…")
                }
            }
            .ignoresSafeArea()
        }
    }
}
