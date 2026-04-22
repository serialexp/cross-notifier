import Foundation
import UIKit
import UserNotifications

/// Observable state machine for "do we have a token, is the server
/// happy with it, what should the UI show."
@MainActor
final class PushManager: ObservableObject {
    enum PermissionState: Equatable {
        case unknown
        case requesting
        case granted
        case denied
    }

    /// What we know about the current registration with the server.
    /// Discriminated so SettingsView can pick a colour + label without
    /// ambiguity.
    enum RegistrationState: Equatable {
        case idle                         // no token yet, or no config
        case registering
        case registered(label: String?)   // server accepted our token
        case failed(String)               // last attempt failed; message shown inline
    }

    @Published private(set) var permission: PermissionState = .unknown
    @Published private(set) var deviceToken: String? = DeviceTokenStore.current
    @Published private(set) var registration: RegistrationState = .idle

    private let client: CrossNotifierClient

    init(client: CrossNotifierClient = CrossNotifierClient()) {
        self.client = client
    }

    /// Read cached permission state on launch so the UI doesn't briefly
    /// render "unknown" while iOS is consulted.
    func refreshPermission() async {
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        switch settings.authorizationStatus {
        case .authorized, .provisional, .ephemeral:
            permission = .granted
        case .denied:
            permission = .denied
        case .notDetermined:
            permission = .unknown
        @unknown default:
            permission = .unknown
        }
    }

    /// Ask iOS for alert/badge/sound permission. Safe to call every
    /// launch — iOS only prompts once; subsequent calls resolve
    /// immediately with the stored decision.
    func requestPermission() async {
        permission = .requesting
        do {
            let granted = try await UNUserNotificationCenter.current()
                .requestAuthorization(options: [.alert, .sound, .badge])
            permission = granted ? .granted : .denied
            if granted {
                UIApplication.shared.registerForRemoteNotifications()
            }
        } catch {
            permission = .denied
        }
    }

    /// Called by AppDelegate when iOS hands us a fresh APNS token.
    /// Kicks off re-registration if we have enough config to do so.
    func didReceive(deviceToken raw: Data, config: ServerConfig, secret: String?) {
        let token = hexString(from: raw)
        deviceToken = token
        DeviceTokenStore.current = token

        guard config.isConfigured, let secret, !secret.isEmpty else {
            registration = .idle
            return
        }
        Task { await self.register(config: config, secret: secret) }
    }

    /// Called when iOS fails to register (no network, APNS sandbox
    /// unavailable during provisioning, etc.).
    func didFailToRegister(error: Error) {
        registration = .failed("APNS registration failed: \(error.localizedDescription)")
    }

    /// POST /devices with the current token. Exposed publicly so the
    /// settings screen can trigger a retry after the user fixes config.
    func register(config: ServerConfig, secret: String) async {
        guard let token = deviceToken else {
            registration = .failed("No device token yet — waiting for APNS.")
            return
        }
        registration = .registering
        do {
            let resp = try await client.registerDevice(
                config: config,
                secret: secret,
                deviceToken: token
            )
            registration = .registered(label: resp.label)
        } catch let err as ClientError {
            registration = .failed(err.errorDescription ?? "Unknown error")
        } catch {
            registration = .failed(error.localizedDescription)
        }
    }

    /// DELETE /devices/{token} — called from sign-out / server change.
    /// Clears local state even if the server call fails, since the
    /// user's intent is "stop sending here."
    func unregister(config: ServerConfig, secret: String) async {
        if let token = deviceToken {
            try? await client.unregisterDevice(
                config: config,
                secret: secret,
                deviceToken: token
            )
        }
        deviceToken = nil
        DeviceTokenStore.current = nil
        registration = .idle
    }
}
