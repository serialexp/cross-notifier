# Current Task: Rust Daemon Rewrite - Phase 1

## Status: Phase 1 near-complete — notifications, rendering, tray, icons, click-to-dismiss

## What's Done

The Rust daemon in `daemon/` receives notifications and renders them with text:

- **Windowing**: winit `ApplicationHandler`, borderless transparent window, top-right positioning
- **Rendering**: wgpu 24 with batched 2D renderer (solid rects, textured quads, WGSL shaders)
- **Font**: `fontdue`-based glyph atlas, embedded Hack-Regular.ttf, text measurement/truncation/wrapping
- **Cards**: Notification cards with status color bar, icon, title, 2-line message, source text
- **Networking**: EventLoopProxy-based event flow (async -> main thread, wakes event loop)
  - axum HTTP server on `:9876` (POST /notify, GET /status)
  - tokio-tungstenite WebSocket client with Bearer auth, exponential backoff
- **Config**: serde JSON matching Go format, loads from same config path
- **Protocol**: Full WebSocket message envelope (notification, action, resolved)
- **Queue**: Notification lifecycle (add, dismiss, expiry pruning)
- **Window sizing**: Dynamic height based on notification count
- **System tray**: tray-icon + muda, Settings/Quit menu, icon swaps on notification state
- **Icons**: base64/file (sync) and URL (async) icon loading, 48x48 scaling, GPU texture upload
- **Click-to-dismiss**: Hit testing on topmost card, left-click dismisses

### Key architecture decision
Switched from mpsc channel to `EventLoopProxy::send_event()` for async->main communication. The mpsc approach didn't wake the winit event loop when the window was hidden, causing notifications to be silently dropped.

## Files

```
daemon/
  Cargo.toml
  fonts/Hack-Regular.ttf
  shaders/quad.wgsl
  src/
    main.rs         - winit event loop, App struct, render_frame()
    app.rs          - AppEvent enum
    card.rs         - Notification card layout and rendering
    client.rs       - WebSocket client (EventLoopProxy)
    config.rs       - Config types (Go-compatible JSON)
    font.rs         - fontdue glyph atlas, text measurement
    gpu.rs          - wgpu device/surface init (PostMultiplied alpha on macOS)
    notification.rs - Notification data model, queue
    protocol.rs     - WebSocket message types
    renderer.rs     - Batched 2D renderer (solid, textured, text batch)
    server.rs       - axum HTTP server (EventLoopProxy)
    tray.rs         - System tray (tray-icon + muda, Settings/Quit menu)
    icon.rs         - Icon loading (base64, file, URL), scaling, GPU upload
```

## What's Next

1. **macOS platform** - Click-through for transparent areas
2. **Phase 2**: Notification center (store, panel, slide animation, scroll, dismiss)

## Build & Test

```bash
cd daemon && cargo build
cargo run  # Listens on :9876
curl -X POST http://localhost:9876/notify -H "Content-Type: application/json" \
  -d '{"title":"Test","message":"Hello world","status":"info","duration":5}'
```

## Architecture

```
Main Thread (winit event loop)
  ├── App state (notifications, renderer, GPU, font atlas)
  ├── Receives AppEvent via EventLoopProxy (wakes event loop)
  └── Renders via wgpu

Tokio Runtime (background)
  ├── HTTP Server (axum, :9876)
  ├── WebSocket Clients (per server, reconnecting)
  └── Send AppEvent via EventLoopProxy::send_event()
```
