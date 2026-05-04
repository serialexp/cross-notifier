# TODO

## Expired notification UI (daemon)

When the server broadcasts an `expired` message (new WS message type added
alongside `wait` / `maxWait` long-polling), the daemon currently dismisses
the notification as a stub. Bart's intended behavior: keep the card on
screen but swap the action buttons for a single disabled "Timed out" pill
so the user still sees what happened but can no longer respond.

Touches:
- `daemon/src/notification.rs` — add a `timed_out: bool` state on the in-memory notification.
- Action button rendering (search for where `payload.actions` are drawn) — branch on `timed_out`.
- `daemon/src/main.rs::AppEvent::NotificationExpired` — set the flag instead of dismissing.

## Theme-aware tray icon (Rust port of the work formerly done in Go)

A previous Go-side iteration shipped both a black ("for light themes") and a
white ("for dark themes") tray icon, picked the variant based on the desktop
color-scheme, and exposed an explicit `trayIconStyle` config override (auto /
light / dark) for desktops where the panel doesn't match the global app
theme — notably KDE Plasma, where Plasma panels often render dark even when
the global color-scheme is light. That work needs porting to the Rust daemon.

Assets already in the repo:
- `tray@2x.png` / `tray-notification@2x.png` — black variant (current default;
  also the macOS template icon, which the OS auto-inverts).
- `tray-dark@2x.png` / `tray-notification-dark@2x.png` — white variant.
- `tray*.svg` — design source.

Touches:
- `daemon/src/tray.rs` — embed both PNG pairs (currently only embeds the light
  pair via `include_bytes!`); add a picker that returns the right one.
- New theme-detection module — probe in this order:
  1. XDG portal `org.freedesktop.appearance/color-scheme` via `zbus` (or shell
     out to `dbus-send`). Returns 1=dark, 2=light, 0=no-preference.
  2. KDE: `kreadconfig6` / `kreadconfig5` `kdeglobals General/ColorScheme`
     (name contains "dark").
  3. GNOME: `gsettings org.gnome.desktop.interface color-scheme`.
  4. GNOME fallback: `gsettings ... gtk-theme` name contains "dark".
  5. Default: dark.
  Cache the result for ~2s so the tray tick doesn't spam subprocesses.
  macOS should always return the black template icon — the OS handles
  inversion; auto-detecting and shipping the inverted icon double-inverts it.
- `daemon/src/config.rs` — add a `tray_icon_style` field (`auto` / `light` /
  `dark`, default `auto`).
- `daemon/src/settings.rs` — surface the override as a dropdown in the egui
  settings UI, in an "Appearance" section.
- Re-set the tray icon when the config or detected theme changes (the tray
  loop probably already wakes periodically; otherwise wire it through
  `AppEvent`).

Notes from the original work:
- KDE Plasma users frequently have the global color-scheme set to "Light" but
  a dark panel (Plasma uses a complementary color group for the panel). Auto
  detection alone won't fix them; the manual override is the escape hatch.
- The 2s cache TTL was chosen so a manual theme switch propagates within a
  tray tick without burning subprocesses constantly.
