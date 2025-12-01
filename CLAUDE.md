# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Test Commands

```bash
# Build
go build -o cross-notifier .

# Run tests
go test ./...

# Run a single test
go test -run TestName ./...

# Build macOS app bundle and DMG
./build-macos.sh --dmg

# Build server-only binary (no GUI dependencies)
just server

# Build Docker image via Depot
just docker

# Build Docker image locally
just docker-local
```

## Running

```bash
# Daemon mode (displays notifications)
./cross-notifier

# Server mode (forwards to connected clients)
./cross-notifier -server -secret <key>

# Daemon connected to remote server
./cross-notifier -connect ws://host:9876/ws -secret <key>

# Open settings window
./cross-notifier -setup
```

## Architecture

**Modes of operation:**
- **Daemon mode** (default): Displays notifications locally via giu/imgui GUI. Accepts HTTP POST on `:9876/notify`. Optionally connects to a remote server to receive notifications.
- **Server mode** (`-server`): Headless. Accepts HTTP POST notifications and broadcasts to connected WebSocket clients. Does not display notifications itself.

**Key files:**
- `main.go` - Entry point, CLI flags, daemon GUI loop
- `server.go` - WebSocket server for broadcasting notifications
- `client.go` - WebSocket client with reconnection logic
- `icon.go` - Icon loading from file paths, URLs, and base64 data
- `config.go` - Persistent configuration (server URL, secret)
- `settings.go` - First-run settings window

**Notification flow:**
1. Service sends POST to server's `/notify` endpoint with auth header
2. Server fetches/resizes any `iconHref` URLs, converts to base64 `iconData`
3. Server broadcasts notification JSON to all connected WebSocket clients
4. Daemon receives via WebSocket, calls `addNotification()`, renders via giu

**Icon sources** (priority order):
- `iconData` - base64 encoded image (used for remote notifications)
- `iconHref` - URL (server fetches and converts to base64)
- `iconPath` - local file path (daemon only)

**Authentication:** Shared secret via `Authorization: Bearer <secret>` header for both HTTP and WebSocket connections.

**Config location:** `~/Library/Application Support/cross-notifier/config.json` (macOS)
