import XCTest
@testable import Bagholder

final class SpikeLoopbackTests: XCTestCase {
    func testLoopbackStatusOK() async throws {
        let server = LocalServer()
        let port = try server.start()
        defer { server.stop() }

        let url = URL(string: "http://127.0.0.1:\(port)/api/status")!
        let (data, response) = try await URLSession.shared.data(from: url)
        let http = try XCTUnwrap(response as? HTTPURLResponse)
        XCTAssertEqual(http.statusCode, 200)
        let obj = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(obj["ok"] as? Bool, true)
        XCTAssertEqual(obj["connected"] as? Bool, false)
        XCTAssertEqual(obj["activityCount"] as? Int, 0)
    }

    func testProcessCanGETLoopbackHTTP() async throws {
        // WKWebView App Transport Security is the real gate for the dashboard
        // (Info.plist NSAllowsLocalNetworking + NSExceptionDomains for 127.0.0.1).
        // This test only proves URLSession in the test process can GET loopback HTTP.
        let server = LocalServer()
        let port = try server.start()
        defer { server.stop() }

        let url = URL(string: "http://127.0.0.1:\(port)/health")!
        let (data, response) = try await URLSession.shared.data(from: url)
        let http = try XCTUnwrap(response as? HTTPURLResponse)
        XCTAssertEqual(http.statusCode, 200)
        XCTAssertEqual(String(data: data, encoding: .utf8), "ok")
    }

    func testEmptyBook() async throws {
        let server = LocalServer()
        let port = try server.start()
        defer { server.stop() }

        let url = URL(string: "http://127.0.0.1:\(port)/api/book")!
        let (data, response) = try await URLSession.shared.data(from: url)
        let http = try XCTUnwrap(response as? HTTPURLResponse)
        XCTAssertEqual(http.statusCode, 200)
        let obj = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(obj["ok"] as? Bool, true)
        XCTAssertEqual((obj["activities"] as? [Any])?.count, 0)
    }
}
