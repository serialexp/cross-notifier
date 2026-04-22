import SwiftUI
import UIKit

@main
struct CrossNotifierApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var pushManager = PushManager()
    @StateObject private var configModel = ServerConfigModel()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(pushManager)
                .environmentObject(configModel)
                .task {
                    // Wire AppDelegate → PushManager as early as we
                    // can. `didRegisterForRemoteNotifications` can fire
                    // between app launch and the first view render; the
                    // delegate buffers the token in that window and
                    // flushes it here. configModel is a @StateObject
                    // that lives as long as the process, so no weak
                    // capture is needed.
                    appDelegate.attach(
                        pushManager: pushManager,
                        configProvider: { (configModel.config, configModel.secret) }
                    )
                    await pushManager.refreshPermission()
                }
        }
    }
}

/// Shared UI-layer model for the server config + secret. Kept separate
/// from the persistence primitives (`ServerConfigStore`, `Keychain`) so
/// views can bind to published fields without every write round-tripping
/// through storage.
@MainActor
final class ServerConfigModel: ObservableObject {
    @Published var config: ServerConfig
    @Published var secret: String

    init() {
        let loaded = ServerConfigStore.load()
        self.config = loaded
        self.secret = Keychain.secret(for: loaded.baseURL) ?? ""
    }

    /// Persist both the config and the secret. Call when the user hits
    /// "Save" in SettingsView.
    func save() {
        ServerConfigStore.save(config)
        if !secret.isEmpty {
            Keychain.setSecret(secret, for: config.baseURL)
        }
    }

    /// Wipe config + secret from storage. Used by the "Sign out" button
    /// after an unregister round-trip.
    func clear() {
        let oldURL = config.baseURL
        config = .empty
        secret = ""
        ServerConfigStore.save(.empty)
        if !oldURL.isEmpty {
            Keychain.removeSecret(for: oldURL)
        }
    }
}
