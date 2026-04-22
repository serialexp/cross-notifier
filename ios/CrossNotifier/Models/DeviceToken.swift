import Foundation

/// Turns an APNS device token (raw `Data` from
/// `didRegisterForRemoteNotificationsWithDeviceToken`) into the lowercase
/// hex string the server expects.
///
/// Factored out into a free function so it's trivial to unit-test
/// without dragging `UIApplication` into the test bundle.
func hexString(from deviceToken: Data) -> String {
    deviceToken.map { String(format: "%02x", $0) }.joined()
}

/// Remembers the APNS token across launches so we can issue a DELETE
/// /devices on sign-out even after the OS has rotated the token out
/// from under us.
enum DeviceTokenStore {
    private static let key = "com.serialexp.crossnotifier.lastDeviceToken"

    static var current: String? {
        get { UserDefaults.standard.string(forKey: key) }
        set {
            if let v = newValue {
                UserDefaults.standard.set(v, forKey: key)
            } else {
                UserDefaults.standard.removeObject(forKey: key)
            }
        }
    }
}
