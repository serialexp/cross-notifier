# Cross Notifier — iOS

Minimal iOS client for the cross-notifier server. Registers the device
with the server, then lets iOS handle notification delivery via APNS.

## Building

You'll need a Mac with Xcode 15+ and an Apple Developer account (paid —
APNS requires it).

```bash
brew install xcodegen
cd ios
xcodegen generate
open CrossNotifier.xcodeproj
```

In Xcode, under **Signing & Capabilities** for both the `CrossNotifier`
and `CrossNotifierServiceExtension` targets:

1. Set **Team** to your developer account.
2. Xcode will automatically generate provisioning profiles. The app
   already declares the **Push Notifications** capability via the
   entitlements file.

The simulator can't receive real APNS pushes, so testing requires a
physical device. Build and run on an iPhone attached over cable or
Wi-Fi.

## Server setup

The server needs APNS configured for the app's bundle ID
(`com.serialexp.crossnotifier`) or push won't actually go anywhere.
See the root [CLAUDE.md](../CLAUDE.md) and the server binary's
`--help` output — minimally:

```
APNS_TEAM_ID=…          # 10-char team ID from developer.apple.com
APNS_KEY_ID=…           # 10-char key ID for a .p8 token auth key
APNS_BUNDLE_ID=com.serialexp.crossnotifier
APNS_P8_KEY_PATH=/path/to/AuthKey_XXXXXXXXXX.p8
APNS_ENVIRONMENT=sandbox   # 'sandbox' while sideloading; 'production' for TestFlight/App Store
CROSS_NOTIFIER_DEVICES_FILE=/data/devices.json
```

The `aps-environment` entitlement in `CrossNotifier.entitlements` is
set to `development` — Apple rewrites this to `production` when you
archive for TestFlight or the App Store, so you don't need to flip it
manually, but you *do* need to flip `APNS_ENVIRONMENT` on the server
when you move between builds.

## Architecture

```
CrossNotifier/                          Main app (SwiftUI + AppDelegate bridge)
├── CrossNotifierApp.swift              @main + ServerConfigModel
├── AppDelegate.swift                   receives APNS token, hands off to PushManager
├── Models/
│   ├── ServerConfig.swift              URL + label, UserDefaults-backed
│   ├── Keychain.swift                  secret storage
│   └── DeviceToken.swift               hex encoding + last-token memory
├── Networking/CrossNotifierClient.swift HTTP calls to /devices
├── Push/PushManager.swift              permission + registration state machine
└── Views/
    ├── ContentView.swift
    └── SettingsView.swift              URL/secret form + status pill

CrossNotifierServiceExtension/          NSE: downloads iconHref, attaches as image
└── NotificationService.swift

CrossNotifierTests/                     XCTest — pure-logic units
```

The app does nothing visible once configured. Notifications arrive as
standard iOS banners; the Notification Service Extension attaches the
sender's icon if the payload includes `iconHref`. Action buttons on
iOS are deliberately not implemented in v1 — they need a
`UNNotificationCategory` negotiation protocol with the server that
we haven't designed.

## Testing against a local server

For end-to-end testing on a physical device, the server needs to be
reachable from the phone. Options:

- Run the server on a LAN host and use its IP in the app.
- Expose a local server via `tailscale`, `ngrok`, or `cloudflared`.
- Deploy to the production server and use `APNS_ENVIRONMENT=sandbox`.

Sandbox APNS pushes go to development-signed builds (Xcode run /
ad-hoc TestFlight internal); production APNS pushes go to App Store
and TestFlight external builds. If pushes silently don't arrive,
double-check that your server's `APNS_ENVIRONMENT` matches how the
installed build is signed.

## Notes

- **App icon** is a single 1024×1024 image (`logo.png` copied into
  the asset catalog). Xcode 15+ synthesises the smaller iOS sizes at
  build time — fine for development and ad-hoc distribution. If we
  ever ship to the App Store we'll need to either verify that
  synthesis covers every required idiom or generate the full set.
- **Binary name** is `CrossNotifier` (no space) so `BUNDLE_LOADER`
  and shell paths stay sane; the friendly "Cross Notifier" lives in
  `CFBundleDisplayName` only.
- **Pure-logic tests** live in `CrossNotifierTests/`. Anything
  touching `UIKit` / `UserNotifications` has to run on a simulator
  or device and isn't covered here — the networking and config
  layers are deliberately factored so they can be tested as plain
  values.
