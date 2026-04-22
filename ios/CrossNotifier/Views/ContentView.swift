import SwiftUI

/// The entire app surface is the settings screen — once configured,
/// notifications arrive via iOS and the app itself has nothing else to
/// display. Wrapping in a NavigationStack gives us a title bar "for
/// free" and makes later expansion (per-server list, debug tools)
/// straightforward.
struct ContentView: View {
    var body: some View {
        NavigationStack {
            SettingsView()
        }
    }
}

#Preview {
    ContentView()
        .environmentObject(PushManager())
        .environmentObject(ServerConfigModel())
}
