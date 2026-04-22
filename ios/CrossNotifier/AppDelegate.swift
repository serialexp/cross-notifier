import UIKit
import UserNotifications

/// Thin bridge between UIApplication callbacks and the SwiftUI side of
/// the app. SwiftUI can't receive the remote-notification token
/// directly, so we keep a lightweight UIApplicationDelegate and hand
/// the token off to `PushManager` via a shared reference.
final class AppDelegate: NSObject, UIApplicationDelegate {
    /// Injected by `CrossNotifierApp` immediately after construction.
    /// Until that runs we buffer the token (iOS can call
    /// didRegisterForRemoteNotifications *before* SwiftUI has finished
    /// wiring up state).
    var pushManager: PushManager?
    var configProvider: (() -> (config: ServerConfig, secret: String?))?

    private var pendingToken: Data?
    private var pendingError: Error?

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        // Delegate assignment lets us observe foreground delivery. For
        // v1 we let iOS own the entire presentation, so this is just a
        // placeholder for future foreground handling.
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    func application(
        _ application: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        if let pushManager, let provider = configProvider {
            let ctx = provider()
            Task { @MainActor in
                pushManager.didReceive(deviceToken: deviceToken, config: ctx.config, secret: ctx.secret)
            }
        } else {
            // PushManager not attached yet — buffer and flush on attach.
            pendingToken = deviceToken
        }
    }

    func application(
        _ application: UIApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        if let pushManager {
            Task { @MainActor in pushManager.didFailToRegister(error: error) }
        } else {
            pendingError = error
        }
    }

    /// Called by `CrossNotifierApp` once the SwiftUI state is alive.
    /// Flushes any buffered APNS callback so a slow-starting UI doesn't
    /// lose the very first registration.
    func attach(pushManager: PushManager, configProvider: @escaping () -> (config: ServerConfig, secret: String?)) {
        self.pushManager = pushManager
        self.configProvider = configProvider
        if let token = pendingToken {
            let ctx = configProvider()
            Task { @MainActor in
                pushManager.didReceive(deviceToken: token, config: ctx.config, secret: ctx.secret)
            }
            pendingToken = nil
        }
        if let error = pendingError {
            Task { @MainActor in pushManager.didFailToRegister(error: error) }
            pendingError = nil
        }
    }
}

extension AppDelegate: UNUserNotificationCenterDelegate {
    /// Decide what to do when a push arrives while the app is
    /// foregrounded. For v1 we let iOS show its usual banner — the
    /// whole point of this app is to surface notifications, not
    /// suppress them.
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound, .badge, .list])
    }
}
