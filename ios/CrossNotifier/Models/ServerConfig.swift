import Foundation

/// User-editable server configuration. The secret lives in the keychain,
/// never in UserDefaults — this struct holds the non-sensitive rest.
struct ServerConfig: Codable, Equatable {
    /// WebSocket / HTTP base URL of the notification server. We store
    /// whatever the user typed (with trailing slash normalisation) and
    /// derive endpoint URLs from it.
    var baseURL: String
    /// Human-readable device label shown in the server's /devices list.
    /// Defaults to the device name; the user can override.
    var deviceLabel: String

    static let empty = ServerConfig(baseURL: "", deviceLabel: "")

    var isConfigured: Bool {
        !baseURL.isEmpty
    }

    /// Build an endpoint URL by joining `path` onto `baseURL`. Returns
    /// nil if baseURL isn't an absolute http/https URL — URLSession
    /// won't go anywhere without a scheme and host, so we reject early
    /// and surface a validation error in the UI.
    func endpoint(_ path: String) -> URL? {
        var normalised = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        while normalised.hasSuffix("/") {
            normalised.removeLast()
        }
        if normalised.isEmpty { return nil }
        let leading = path.hasPrefix("/") ? path : "/\(path)"
        guard let url = URL(string: normalised + leading) else { return nil }
        // Require an absolute URL. Foundation happily parses bare
        // "/devices" as a relative URL with a nil scheme; URLSession
        // can't resolve that, so reject it here.
        guard let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https",
              url.host != nil else {
            return nil
        }
        return url
    }
}

/// Persists `ServerConfig` in UserDefaults. Secret lives in Keychain
/// (see `Keychain.swift`), so it's not part of this type.
enum ServerConfigStore {
    private static let key = "com.serialexp.crossnotifier.serverConfig"

    static func load() -> ServerConfig {
        guard let data = UserDefaults.standard.data(forKey: key),
              let decoded = try? JSONDecoder().decode(ServerConfig.self, from: data) else {
            return ServerConfig.empty
        }
        return decoded
    }

    static func save(_ config: ServerConfig) {
        guard let data = try? JSONEncoder().encode(config) else { return }
        UserDefaults.standard.set(data, forKey: key)
    }
}
