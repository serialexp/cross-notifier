import Foundation
import Security

/// Minimal keychain wrapper — stores one Bearer secret keyed by the
/// server's base URL. Switching servers keeps both secrets around;
/// deleting a server removes only its secret.
///
/// Intentionally not generalised: the surface here is exactly what the
/// app needs and nothing more. If we ever add a second secret kind
/// (API keys, OAuth refresh tokens) we can widen this.
enum Keychain {
    private static let service = "com.serialexp.crossnotifier"

    /// Store (or overwrite) the secret for `server`. Returns false on
    /// any keychain failure — caller should surface that to the user,
    /// since a silently-failed save means /devices calls will 401.
    @discardableResult
    static func setSecret(_ secret: String, for server: String) -> Bool {
        guard let data = secret.data(using: .utf8) else { return false }
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: server,
        ]
        // Try update first; fall back to add if the item doesn't exist.
        let update: [CFString: Any] = [kSecValueData: data]
        let updateStatus = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if updateStatus == errSecSuccess { return true }
        if updateStatus == errSecItemNotFound {
            var addQuery = query
            addQuery[kSecValueData] = data
            addQuery[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlock
            return SecItemAdd(addQuery as CFDictionary, nil) == errSecSuccess
        }
        return false
    }

    /// Fetch the secret for `server`. Returns nil if none is set, the
    /// data is corrupt, or keychain access fails.
    static func secret(for server: String) -> String? {
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: server,
            kSecReturnData: true,
            kSecMatchLimit: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let s = String(data: data, encoding: .utf8) else {
            return nil
        }
        return s
    }

    /// Remove the secret for `server`. No-op if none was stored.
    static func removeSecret(for server: String) {
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: server,
        ]
        SecItemDelete(query as CFDictionary)
    }
}
