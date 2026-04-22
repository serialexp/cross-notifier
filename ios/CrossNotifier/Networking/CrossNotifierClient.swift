import Foundation

/// Errors the networking layer surfaces to the UI. Designed to be
/// exhaustive-friendly so SettingsView can render a precise status line.
enum ClientError: Error, LocalizedError, Equatable {
    case notConfigured
    case invalidURL
    case network(String)
    case unauthorized
    case notFound  // registry not configured on the server
    case server(status: Int, body: String)
    case decoding(String)

    var errorDescription: String? {
        switch self {
        case .notConfigured: return "Server URL not set."
        case .invalidURL: return "Server URL is not valid."
        case .network(let msg): return "Network error: \(msg)"
        case .unauthorized: return "Server rejected the secret (401)."
        case .notFound: return "Server doesn't have push enabled (404)."
        case .server(let status, let body):
            return body.isEmpty ? "Server returned \(status)." : "Server returned \(status): \(body)"
        case .decoding(let msg): return "Couldn't parse server response: \(msg)"
        }
    }
}

/// Response of a successful /devices upsert. Mirrors `Device` in the
/// server's OpenAPI — unused fields are decoded leniently.
struct DeviceResponse: Decodable, Equatable {
    let token: String
    let label: String?
    let platform: String
    let registeredAt: String
    let lastPushAt: String?
}

/// Stateless HTTP client. Takes explicit config on every call instead
/// of storing it — avoids an invalidation dance when the user edits
/// the URL or secret.
struct CrossNotifierClient {
    let session: URLSession

    init(session: URLSession = .shared) {
        self.session = session
    }

    /// Upsert this device on the server. Safe to call on every launch;
    /// the server preserves `registeredAt` when re-registering the same
    /// token.
    func registerDevice(
        config: ServerConfig,
        secret: String,
        deviceToken: String
    ) async throws -> DeviceResponse {
        guard config.isConfigured else { throw ClientError.notConfigured }
        guard let url = config.endpoint("/devices") else { throw ClientError.invalidURL }

        let body: [String: Any] = [
            "deviceToken": deviceToken,
            "label": config.deviceLabel,
            "platform": "ios",
        ]
        let (data, status) = try await post(url: url, secret: secret, jsonBody: body)
        try mapStatus(status, data: data)
        do {
            return try JSONDecoder().decode(DeviceResponse.self, from: data)
        } catch {
            throw ClientError.decoding(String(describing: error))
        }
    }

    /// Remove a device by token. A missing token is reported as success
    /// — the caller's intent is "make sure this token is gone", which
    /// is already true.
    func unregisterDevice(
        config: ServerConfig,
        secret: String,
        deviceToken: String
    ) async throws {
        guard config.isConfigured else { throw ClientError.notConfigured }
        guard let url = config.endpoint("/devices/\(deviceToken)") else {
            throw ClientError.invalidURL
        }

        var req = URLRequest(url: url)
        req.httpMethod = "DELETE"
        req.setValue("Bearer \(secret)", forHTTPHeaderField: "Authorization")
        let (data, resp) = try await dataWrapped(for: req)
        guard let http = resp as? HTTPURLResponse else {
            throw ClientError.network("non-HTTP response")
        }
        if http.statusCode == 204 || http.statusCode == 404 {
            return
        }
        try mapStatus(http.statusCode, data: data)
    }

    // MARK: - Internals

    private func post(
        url: URL,
        secret: String,
        jsonBody: [String: Any]
    ) async throws -> (Data, Int) {
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("Bearer \(secret)", forHTTPHeaderField: "Authorization")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try? JSONSerialization.data(withJSONObject: jsonBody)
        let (data, resp) = try await dataWrapped(for: req)
        guard let http = resp as? HTTPURLResponse else {
            throw ClientError.network("non-HTTP response")
        }
        return (data, http.statusCode)
    }

    private func dataWrapped(for req: URLRequest) async throws -> (Data, URLResponse) {
        do {
            return try await session.data(for: req)
        } catch {
            throw ClientError.network(error.localizedDescription)
        }
    }

    private func mapStatus(_ status: Int, data: Data) throws {
        switch status {
        case 200..<300: return
        case 401: throw ClientError.unauthorized
        case 404: throw ClientError.notFound
        default:
            let body = String(data: data, encoding: .utf8) ?? ""
            throw ClientError.server(status: status, body: body)
        }
    }
}
