# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Test Commands

The repo is a Rust workspace (`Cargo.toml` at root). Common commands:

```bash
# Build the desktop daemon (GUI)
just build           # cargo build --release -p cross-notifier-daemon

# Run the desktop daemon locally
just dev             # cargo run -p cross-notifier-daemon

# Run all workspace tests
just test            # cargo test --workspace

# Lint (clippy with -D warnings, matching CI)
just lint

# Format
just fmt

# Build server-only binary (headless, no GUI deps)
just server          # cargo build --release -p cross-notifier-server

# Build macOS app bundle and DMG
just macos           # ./build-macos.sh --dmg

# Build and push Docker image via Depot
just docker

# Build Docker image locally
just docker-local
```

## Running

```bash
# Daemon mode (displays notifications)
cargo run -p cross-notifier-daemon

# Daemon connected to remote server
cargo run -p cross-notifier-daemon -- -connect ws://host:9876/ws -secret <key>

# Run standalone server (Docker / headless)
cargo run -p cross-notifier-server -- -secret <key>
```

## Workspace Layout

Libraries live in `crates/`, binaries in `bin/`.

```
crates/
  core/      — shared types and utilities (no GUI deps)
  calendar/  — CalDAV-based calendar reminder source
bin/
  server/    — headless WebSocket+HTTP server (used in Docker)
  daemon/    — desktop GUI daemon (winit + wgpu + tray-icon + egui settings)
ios/                   — iOS app (Swift, separate from the Rust workspace)
packages/notify-client — TypeScript client SDK
```

## Daemon Internals

**Stack:**
- `winit` — windowing and event loop
- `wgpu` — GPU rendering (custom batched 2D renderer with WGSL shaders)
- `fontdue` — glyph atlas / text rendering
- `tray-icon` + `muda` — system tray (requires GTK on Linux)
- `egui` — settings window UI
- `axum` — local HTTP server (`:9876/notify`, `/center` endpoints)
- `tokio-tungstenite` — WebSocket client (per-server reconnecting, Bearer auth)

**Async ↔ main thread:** background tokio tasks deliver `AppEvent`s into the
winit event loop via `EventLoopProxy::send_event()` so the loop wakes even
when the window is hidden.

**Key files (under `bin/daemon/src/`):**
- `main.rs` — winit `ApplicationHandler`, App struct, render frame
- `app.rs` — `AppEvent` enum
- `client.rs` — WebSocket client with reconnection
- `server.rs` — local axum HTTP server
- `tray.rs` — system tray icon + menu
- `card.rs` — notification card layout/rendering
- `center.rs` — notification center panel
- `store.rs` — persisted notification store
- `config.rs` — config types + JSON load/save
- `settings.rs` — egui settings window
- `icon.rs` — base64/file/URL icon loading + GPU upload
- `font.rs` — glyph atlas (embeds `bin/daemon/fonts/Hack-Regular.ttf`)
- `sound.rs` — embedded sound playback (rodio)
- `protocol.rs` — WebSocket message envelope types
- `rules.rs` — notification rule matching
- `autostart.rs` — login-item / autostart integration
- `gpu.rs` — wgpu device/surface init (PostMultiplied alpha on macOS)
- `renderer.rs` — batched 2D renderer (solid + textured + text)
- `theme.rs` — desktop color-scheme detection (XDG portal → KDE → GNOME)
  used by the tray to pick between the light/dark icon variants

## Notification Flow

1. Service POSTs to server's `/notify` with `Authorization: Bearer <secret>`.
2. Server resolves any `iconHref` URL (fetches + base64-encodes), then broadcasts
   the notification JSON to all connected WebSocket clients.
3. Daemon receives via WebSocket, enqueues an `AppEvent::Notification`, renders
   via wgpu.

**Icon source priority:** `iconData` (base64) → `iconHref` (URL, server-side
fetched) → `iconPath` (local path, daemon only).

**Auth:** shared secret via `Authorization: Bearer <secret>` for both HTTP and
WebSocket connections.

**Config location:** platform-specific config dir under `cross-notifier/` —
`~/Library/Application Support/cross-notifier/config.json` on macOS,
`~/.config/cross-notifier/config.json` on Linux.

## Tray Icon Assets

Stored at the repo root and pulled into the daemon via `include_bytes!`:

- `tray@2x.png` / `tray-notification@2x.png` — black "for light themes" variant
  (also serves as the macOS template icon, which the OS auto-inverts)
- `tray-dark@2x.png` / `tray-notification-dark@2x.png` — white "for dark themes"
  variant (Linux/Windows panels don't auto-recolor)
- `tray*.svg` — source SVGs; ImageMagick (`convert -background none -density 576
  -resize 44x44 ...`) renders them to the @2x PNGs.

The daemon embeds both variant pairs and picks at runtime. The picker
resolves a `tray_icon_style` config field (`auto` / `light` / `dark`,
default `auto`) — `auto` defers to `theme::detect()`, which walks the
XDG portal → KDE → GNOME chain. macOS short-circuits to the black
icon plus `with_icon_as_template(true)`; the OS handles inversion.

The override exists because some Linux desktops (notably KDE Plasma) run
a light global color-scheme with a dark panel — auto-detection alone
can't tell, so the user picks manually. The Linux GTK idle handler
re-evaluates the variant every ~1.5s while in `Auto` mode, so a desktop
theme switch propagates without a daemon restart.

## Git Commits

Follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:

```
<type>(<scope>): <description>

[optional body]
```

Common types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `ci`.
