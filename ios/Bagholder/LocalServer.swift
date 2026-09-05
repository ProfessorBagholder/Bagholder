import Combine
import Foundation
import Network

enum LocalServerError: Error {
    case bindFailed
}

/// Loopback HTTP/1.1 listener. Ports match bagholder.py: 8765, then 8766, then 8767.
final class LocalServer: ObservableObject {
    static let shared = LocalServer()
    static let ports: [UInt16] = [8765, 8766, 8767]

    @Published private(set) var port: UInt16?
    @Published private(set) var errorMessage: String?

    private(set) var boundPort: UInt16?
    var url: URL? {
        guard let boundPort else { return nil }
        return URL(string: "http://127.0.0.1:\(boundPort)/")
    }

    private let queue = DispatchQueue(label: "com.bagholder.local-server")
    private var listener: NWListener?
    private let book: BookAPI
    private let ledgerHTML: Data?
    private let favicon: Data?

    init(book: BookAPI = .shared) {
        self.book = book
        self.ledgerHTML = Self.loadResource("ledger", ext: "html")
        self.favicon = Self.loadResource("favicon", ext: "png")
    }

    func startIfNeeded() {
        guard boundPort == nil else { return }
        do {
            _ = try start()
        } catch {
            let message = "Could not bind 127.0.0.1:8765-8767"
            errorMessage = message
            DispatchQueue.main.async { [weak self] in
                self?.errorMessage = message
            }
        }
    }

    @discardableResult
    func start() throws -> UInt16 {
        if let boundPort { return boundPort }
        var lastError: Error = LocalServerError.bindFailed
        for candidate in Self.ports {
            do {
                try bind(port: candidate)
                boundPort = candidate
                DispatchQueue.main.async { [weak self] in
                    self?.port = candidate
                }
                return candidate
            } catch {
                lastError = error
            }
        }
        throw lastError
    }

    func stop() {
        listener?.cancel()
        listener = nil
        boundPort = nil
        DispatchQueue.main.async { [weak self] in
            self?.port = nil
        }
    }

    private static func loadResource(_ name: String, ext: String) -> Data? {
        let url = Bundle.main.url(forResource: name, withExtension: ext, subdirectory: "Resources")
            ?? Bundle.main.url(forResource: name, withExtension: ext)
        guard let url else { return nil }
        return try? Data(contentsOf: url)
    }

    private func bind(port: UInt16) throws {
        let params = NWParameters.tcp
        params.allowLocalEndpointReuse = true
        if let ip = params.defaultProtocolStack.internetProtocol as? NWProtocolIP.Options {
            ip.version = .v4
        }
        let nwPort = NWEndpoint.Port(rawValue: port)!
        params.requiredLocalEndpoint = NWEndpoint.hostPort(
            host: NWEndpoint.Host("127.0.0.1"),
            port: nwPort
        )
        let listener = try NWListener(using: params)
        let box = BindBox()
        listener.stateUpdateHandler = { state in
            switch state {
            case .ready:
                box.finish(nil)
            case .failed(let error):
                box.finish(error)
            default:
                break
            }
        }
        listener.newConnectionHandler = { [weak self] connection in
            self?.handle(connection)
        }
        listener.start(queue: queue)
        if box.sem.wait(timeout: .now() + 2) == .timedOut {
            listener.cancel()
            throw LocalServerError.bindFailed
        }
        if let error = box.error {
            listener.cancel()
            throw error
        }
        self.boundPort = port
        self.listener = listener
    }

    private func handle(_ connection: NWConnection) {
        connection.start(queue: queue)
        receive(on: connection, buffer: Data())
    }

    private func receive(on connection: NWConnection, buffer: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, isComplete, error in
            guard let self else {
                connection.cancel()
                return
            }
            if error != nil {
                connection.cancel()
                return
            }
            var buf = buffer
            if let data {
                buf.append(data)
            }
            if buf.count > 2_000_000 {
                connection.cancel()
                return
            }
            guard let request = HTTPRequest.parse(buf) else {
                if isComplete {
                    connection.cancel()
                } else {
                    self.receive(on: connection, buffer: buf)
                }
                return
            }
            let response = self.route(request)
            self.send(response, on: connection)
        }
    }

    private func send(_ response: HTTPResponse, on connection: NWConnection) {
        connection.send(
            content: response.data(),
            contentContext: .defaultMessage,
            isComplete: true,
            completion: .contentProcessed { _ in
                connection.cancel()
            }
        )
    }

    func route(_ request: HTTPRequest) -> HTTPResponse {
        guard hostOK(request.headers) else {
            return HTTPResponse.json(403, ["ok": false])
        }
        let path = request.path
        switch (request.method, path) {
        case ("GET", "/"), ("GET", "/ledger.html"):
            if let ledgerHTML {
                return HTTPResponse(status: 200, contentType: "text/html; charset=utf-8", body: ledgerHTML)
            }
            return HTTPResponse.json(404, ["ok": false, "error": "ledger.html missing"])
        case ("GET", "/favicon.png"), ("GET", "/favicon.ico"):
            if let favicon {
                return HTTPResponse(status: 200, contentType: "image/png", body: favicon)
            }
            return HTTPResponse.json(404, ["ok": false, "error": "favicon missing"])
        case ("GET", "/health"):
            return HTTPResponse(status: 200, contentType: "text/plain; charset=utf-8", body: Data("ok".utf8))
        case ("GET", "/api/status"):
            return HTTPResponse.json(200, book.status())
        case ("GET", "/api/book"):
            return HTTPResponse.json(200, book.book())
        case ("POST", "/api/notes"):
            guard writeOK(request.headers) else {
                return HTTPResponse.json(403, ["ok": false])
            }
            return HTTPResponse.json(200, book.saveNotes(request.jsonObject()))
        case ("POST", "/api/groups"):
            guard writeOK(request.headers) else {
                return HTTPResponse.json(403, ["ok": false])
            }
            return HTTPResponse.json(200, book.saveGroups(request.jsonObject()))
        case ("OPTIONS", _):
            return HTTPResponse.json(403, ["ok": false])
        default:
            return HTTPResponse.json(404, ["ok": false, "error": "not found"])
        }
    }

    private func writeOK(_ headers: [String: String]) -> Bool {
        let site = (headers["sec-fetch-site"] ?? "").lowercased()
        if site == "same-origin" { return true }
        return !(headers["x-bagholder"] ?? "").isEmpty
    }

    private func hostOK(_ headers: [String: String]) -> Bool {
        let raw = (headers["host"] ?? "").trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if raw.isEmpty || raw.contains(",") { return false }
        guard let boundPort else { return false }
        return raw == "127.0.0.1:\(boundPort)"
    }
}

private final class BindBox {
    let sem = DispatchSemaphore(value: 0)
    var error: Error?
    private var done = false
    private let lock = NSLock()

    func finish(_ error: Error?) {
        lock.lock()
        defer { lock.unlock() }
        guard !done else { return }
        done = true
        self.error = error
        sem.signal()
    }
}

struct HTTPRequest {
    var method: String
    var path: String
    var headers: [String: String]
    var body: Data

    func jsonObject() -> [String: Any] {
        guard !body.isEmpty,
              let obj = try? JSONSerialization.jsonObject(with: body),
              let dict = obj as? [String: Any]
        else { return [:] }
        return dict
    }

    static func parse(_ data: Data) -> HTTPRequest? {
        let separator = Data("\r\n\r\n".utf8)
        guard let range = data.range(of: separator) else { return nil }
        let head = data.subdata(in: data.startIndex..<range.lowerBound)
        let rest = data.subdata(in: range.upperBound..<data.endIndex)
        guard let headText = String(data: head, encoding: .utf8) else { return nil }
        let lines = headText.split(separator: "\r\n", omittingEmptySubsequences: false)
        guard let first = lines.first else { return nil }
        let parts = first.split(separator: " ", maxSplits: 2, omittingEmptySubsequences: true)
        guard parts.count >= 2 else { return nil }
        let method = String(parts[0]).uppercased()
        var path = String(parts[1])
        if let q = path.firstIndex(of: "?") {
            path = String(path[..<q])
        }
        var headers: [String: String] = [:]
        for line in lines.dropFirst() {
            guard let colon = line.firstIndex(of: ":") else { continue }
            let name = line[..<colon].trimmingCharacters(in: .whitespaces).lowercased()
            let value = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
            headers[name] = String(value)
        }
        let length = Int(headers["content-length"] ?? "0") ?? 0
        if length < 0 || length > 1_048_576 { return nil }
        if rest.count < length { return nil }
        let body = rest.prefix(length)
        return HTTPRequest(method: method, path: path, headers: headers, body: Data(body))
    }
}

struct HTTPResponse {
    var status: Int
    var contentType: String
    var body: Data

    static func json(_ status: Int, _ object: [String: Any]) -> HTTPResponse {
        let data = (try? JSONSerialization.data(withJSONObject: object, options: [])) ?? Data("{}".utf8)
        return HTTPResponse(status: status, contentType: "application/json; charset=utf-8", body: data)
    }

    func data() -> Data {
        let reason: String
        switch status {
        case 200: reason = "OK"
        case 403: reason = "Forbidden"
        case 404: reason = "Not Found"
        default: reason = "OK"
        }
        var header = "HTTP/1.1 \(status) \(reason)\r\n"
        header += "Content-Type: \(contentType)\r\n"
        header += "Content-Length: \(body.count)\r\n"
        header += "Cache-Control: no-store\r\n"
        header += "X-Content-Type-Options: nosniff\r\n"
        header += "Connection: close\r\n"
        header += "\r\n"
        var out = Data(header.utf8)
        out.append(body)
        return out
    }
}
