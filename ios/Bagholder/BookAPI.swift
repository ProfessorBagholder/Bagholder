import Foundation

/// In-memory empty book for M1. sqlite is M2; Connect is M3.
final class BookAPI {
    static let shared = BookAPI()

    private let lock = NSLock()
    private var notes: [String: Any] = [:]
    private var groups: [Any] = []

    func status() -> [String: Any] {
        [
            "ok": true,
            "connected": false,
            "email": "",
            "lastSync": "",
            "activityCount": 0,
            "accountCount": 0,
            "capturing": false,
            "syncing": false,
            "listingsFilling": false,
            "syncStep": "",
            "error": "",
        ]
    }

    func book() -> [String: Any] {
        lock.lock()
        defer { lock.unlock() }
        return [
            "ok": true,
            "activities": [] as [Any],
            "accounts": [] as [Any],
            "balances": [] as [Any],
            "navHistory": [] as [Any],
            "navByAccount": [:] as [String: Any],
            "syncedAt": "",
            "tradeGroups": groups,
            "notes": notes,
            "securities": [] as [Any],
        ]
    }

    func saveNotes(_ body: [String: Any]) -> [String: Any] {
        lock.lock()
        defer { lock.unlock() }
        if let incoming = body["notes"] as? [String: Any] {
            notes = incoming
        }
        return ["ok": true, "notes": notes]
    }

    func saveGroups(_ body: [String: Any]) -> [String: Any] {
        lock.lock()
        defer { lock.unlock() }
        if let incoming = body["groups"] as? [Any] {
            groups = incoming
        }
        return ["ok": true, "groups": groups]
    }
}
