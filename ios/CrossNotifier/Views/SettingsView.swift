import SwiftUI
import UIKit

/// Lone settings screen. Three responsibilities:
///   1. Let the user enter server URL + secret + device label.
///   2. Surface the current registration / permission status.
///   3. Provide Save / Retry / Sign-out actions.
struct SettingsView: View {
    @EnvironmentObject private var configModel: ServerConfigModel
    @EnvironmentObject private var pushManager: PushManager
    @State private var isSaving = false

    var body: some View {
        Form {
            Section {
                statusRow
            } header: {
                Text("Status")
            } footer: {
                Text(footerForStatus)
            }

            Section("Server") {
                TextField("https://example.com", text: $configModel.config.baseURL)
                    .textContentType(.URL)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()

                SecureField("Shared secret", text: $configModel.secret)
                    .textContentType(.password)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()

                TextField("Device label", text: $configModel.config.deviceLabel)
                    .autocorrectionDisabled()
            }

            Section {
                Button(action: save) {
                    HStack {
                        if isSaving {
                            ProgressView().padding(.trailing, 4)
                        }
                        Text(isSaving ? "Registering…" : "Save and register")
                    }
                }
                .disabled(isSaving || !configModel.config.isConfigured)

                if pushManager.permission == .denied {
                    Button("Open notification settings") {
                        if let url = URL(string: UIApplication.openSettingsURLString) {
                            UIApplication.shared.open(url)
                        }
                    }
                }
            }

            if case .registered = pushManager.registration {
                Section {
                    Button("Sign out", role: .destructive) {
                        Task { await signOut() }
                    }
                }
            }

            Section("About") {
                LabeledContent("Version", value: appVersion)
                if let token = pushManager.deviceToken {
                    LabeledContent("Device token") {
                        Text(token.prefix(12) + "…")
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
        .navigationTitle("Cross Notifier")
        .onAppear {
            if configModel.config.deviceLabel.isEmpty {
                // Default label: the device's user-visible name. Bart
                // can overwrite it in the field.
                configModel.config.deviceLabel = UIDevice.current.name
            }
        }
    }

    // MARK: - Pieces

    private var statusRow: some View {
        HStack(spacing: 10) {
            Circle()
                .fill(indicatorColor)
                .frame(width: 10, height: 10)
            VStack(alignment: .leading, spacing: 2) {
                Text(statusLabel).font(.subheadline.bold())
                if let detail = statusDetail {
                    Text(detail).font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer()
        }
    }

    private var indicatorColor: Color {
        if pushManager.permission == .denied { return .red }
        switch pushManager.registration {
        case .registered: return .green
        case .registering: return .yellow
        case .failed: return .red
        case .idle:
            return pushManager.deviceToken == nil ? .gray : .yellow
        }
    }

    private var statusLabel: String {
        if pushManager.permission == .denied {
            return "Notifications disabled in system settings"
        }
        switch pushManager.registration {
        case .registered: return "Registered"
        case .registering: return "Registering…"
        case .failed: return "Registration failed"
        case .idle:
            return pushManager.deviceToken == nil ? "Waiting for device token" : "Not registered"
        }
    }

    private var statusDetail: String? {
        switch pushManager.registration {
        case .failed(let msg): return msg
        case .registered(let label?): return "as \(label)"
        default: return nil
        }
    }

    private var footerForStatus: String {
        switch pushManager.permission {
        case .denied:
            return "Open Settings to re-enable notification permission, then return here and tap Save."
        case .unknown, .requesting:
            return "We'll ask for notification permission the first time you save."
        case .granted:
            return "Enter your server's URL and the shared secret, then tap Save."
        }
    }

    private var appVersion: String {
        let short = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "?"
        let build = Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "?"
        return "\(short) (\(build))"
    }

    // MARK: - Actions

    private func save() {
        Task {
            isSaving = true
            defer { isSaving = false }

            configModel.save()

            if pushManager.permission != .granted {
                await pushManager.requestPermission()
            }

            // If we already have a device token, register immediately;
            // otherwise the AppDelegate callback will call register()
            // once APNS responds.
            if pushManager.deviceToken != nil {
                await pushManager.register(config: configModel.config, secret: configModel.secret)
            } else {
                // Kick a registerForRemoteNotifications in case the user
                // had previously denied and now re-granted.
                UIApplication.shared.registerForRemoteNotifications()
            }
        }
    }

    private func signOut() async {
        if !configModel.config.baseURL.isEmpty, !configModel.secret.isEmpty {
            await pushManager.unregister(config: configModel.config, secret: configModel.secret)
        }
        configModel.clear()
    }
}

#Preview {
    NavigationStack {
        SettingsView()
            .environmentObject(PushManager())
            .environmentObject(ServerConfigModel())
    }
}
