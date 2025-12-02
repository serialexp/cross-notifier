# CrossNotifier

A cross-platform notification daemon that displays desktop notifications via HTTP API. Supports both local notifications and remote notifications from a central server.

## Features

- HTTP API on `localhost:9876`
- Supports title, message, and icon (file path, URL, or base64)
- Configurable display duration
- Auto-dismiss with click-to-dismiss
- Stacks up to 4 notifications (queues extras)
- Adapts to OS dark/light theme
- Remote server mode for broadcasting to multiple clients
- Automatic reconnection with exponential backoff
- Action buttons that trigger HTTP requests or open URLs
- Exclusive notifications for coordinated actions across multiple clients

## Installation

### Quick Install (Linux/macOS)

**Standard install:**
```bash
curl -fsSL https://raw.githubusercontent.com/serialexp/cross-notifier/main/install.sh | bash
```

**Install with auto-start on login (Linux only):**
```bash
curl -fsSL https://raw.githubusercontent.com/serialexp/cross-notifier/main/install.sh | bash -s -- --enable-service
```

This will automatically download and install the latest release for your platform.

**On Linux**, the installer will:
- Install the binary to `/usr/local/bin` or `~/.local/bin`
- Create a desktop entry for easy launching from your application menu
- Create a systemd user service file
- Optionally enable auto-start on login (with `--enable-service` flag or interactive prompt)

### Manual Installation

#### macOS

Download the DMG for your architecture from the [latest release](https://github.com/serialexp/cross-notifier/releases/latest):
- **Apple Silicon (M1/M2/M3)**: `CrossNotifier-vX.X.X-macos-arm64.dmg`
- **Intel**: `CrossNotifier-vX.X.X-macos-amd64.dmg`

Open the DMG and drag CrossNotifier to Applications.

#### Linux

Download the tar.gz for your architecture from the [latest release](https://github.com/serialexp/cross-notifier/releases/latest):

```bash
# For x86_64
curl -LO https://github.com/serialexp/cross-notifier/releases/latest/download/cross-notifier-vX.X.X-linux-amd64.tar.gz
tar -xzf cross-notifier-vX.X.X-linux-amd64.tar.gz
sudo mv cross-notifier /usr/local/bin/
```

#### Windows

Download `cross-notifier-vX.X.X-windows-amd64.zip` from the [latest release](https://github.com/serialexp/cross-notifier/releases/latest) and extract it.

### From Source

```bash
go build -o cross-notifier .
./cross-notifier
```

### Server-Only Binary

For running a headless server without GUI dependencies, download the server binary from the [latest release](https://github.com/serialexp/cross-notifier/releases/latest):

```bash
# For x86_64
curl -LO https://github.com/serialexp/cross-notifier/releases/latest/download/cross-notifier-server-vX.X.X-linux-amd64.tar.gz
tar -xzf cross-notifier-server-vX.X.X-linux-amd64.tar.gz
./server -server -secret mysecret -port 9876

# For ARM64 (Raspberry Pi, etc.)
curl -LO https://github.com/serialexp/cross-notifier/releases/latest/download/cross-notifier-server-vX.X.X-linux-arm64.tar.gz
tar -xzf cross-notifier-server-vX.X.X-linux-arm64.tar.gz
./server -server -secret mysecret -port 9876
```

Or build from source:

```bash
just server
./server -server -secret mysecret -port 9876
```

### Docker (Server Only)

Build with Depot:

```bash
just docker
```

Run the published image:

```bash
docker run -d -p 9876:9876 -e CROSS_NOTIFIER_SECRET=mysecret aeolun/cross-notifier-server
```

Or build locally:

```bash
just docker-local
docker run -d -p 9876:9876 -e CROSS_NOTIFIER_SECRET=mysecret cross-notifier-server
```

## Usage

### Daemon Mode (Default)

Start the daemon to display notifications locally:

```bash
./cross-notifier
```

On first run, a settings window appears to configure an optional remote server connection.

### Server Mode

Run as a notification server that broadcasts to connected clients:

```bash
./cross-notifier -server -secret <shared-secret> -port 9876
```

### Daemon with Remote Server

Connect to a remote server while still accepting local notifications:

```bash
./cross-notifier -connect ws://server:9876/ws -secret <shared-secret>
```

### Configuration

Settings are stored in `~/Library/Application Support/cross-notifier/config.json` (macOS).

To reconfigure, delete the config file or run:

```bash
./cross-notifier -setup
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `CROSS_NOTIFIER_SECRET` | Shared secret for authentication |
| `CROSS_NOTIFIER_PORT` | Port to listen on (default: 9876) |
| `CROSS_NOTIFIER_SERVER` | WebSocket URL for daemon to connect to |
| `CROSS_NOTIFIER_NAME` | Client display name for identification |

Environment variables are used as fallbacks when CLI flags are not provided.

## API

### Local Daemon Endpoint

```
POST http://localhost:9876/notify
Content-Type: application/json
```

No authentication required for local notifications.

### Remote Server Endpoint

```
POST http://server:9876/notify
Content-Type: application/json
Authorization: Bearer <shared-secret>
```

### JSON Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `title` | string | No* | - | Notification title |
| `message` | string | No* | - | Notification body text |
| `iconPath` | string | No | - | Local file path to icon (PNG, JPG) |
| `iconHref` | string | No | - | URL to fetch icon from (server fetches and resizes) |
| `iconData` | string | No | - | Base64-encoded icon image |
| `duration` | int | No | 5 | Seconds before auto-dismiss |
| `actions` | array | No | - | Action buttons (see Actions section) |
| `exclusive` | bool | No | false | Server coordinates actions across all clients |

*At least one of `title` or `message` is required.

**Icon priority:** `iconData` > `iconHref` > `iconPath`

When sending to a remote server, use `iconHref` (URL) or `iconData` (base64). The server fetches URLs and resizes images to 48x48 before broadcasting to clients.

### Examples

#### Local Notification

```bash
curl -X POST http://localhost:9876/notify \
  -H "Content-Type: application/json" \
  -d '{"title":"Hello","message":"World"}'
```

#### Local with Icon

```bash
curl -X POST http://localhost:9876/notify \
  -H "Content-Type: application/json" \
  -d '{"title":"Build Complete","iconPath":"/path/to/icon.png"}'
```

#### Remote Notification

```bash
curl -X POST http://server:9876/notify \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer mysecret" \
  -d '{"title":"CI Build","message":"Success!","iconHref":"https://example.com/icon.png"}'
```

#### Python

```python
import requests

# Local
requests.post("http://localhost:9876/notify", json={
    "title": "Task Complete",
    "message": "Your build finished successfully",
    "iconPath": "/path/to/icon.png"
})

# Remote
requests.post("http://server:9876/notify",
    headers={"Authorization": "Bearer mysecret"},
    json={
        "title": "Deploy Complete",
        "message": "Production updated",
        "iconHref": "https://example.com/success.png"
    }
)
```

## Actions

Notifications can include action buttons that trigger HTTP requests or open URLs when clicked.

### Action Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `label` | string | Yes | - | Button text |
| `url` | string | Yes | - | Target URL |
| `method` | string | No | GET | HTTP method (GET, POST, PUT, DELETE) |
| `headers` | object | No | - | HTTP headers to include |
| `body` | string | No | - | HTTP request body |
| `open` | bool | No | false | Open URL in browser instead of HTTP request |

### Action Examples

#### Simple Action

```bash
curl -X POST http://localhost:9876/notify \
  -H "Content-Type: application/json" \
  -d '{
    "title": "PR Ready for Review",
    "message": "#123 needs your approval",
    "actions": [
      {"label": "View", "url": "https://github.com/org/repo/pull/123", "open": true}
    ]
  }'
```

#### Multiple Actions with HTTP Requests

```bash
curl -X POST http://localhost:9876/notify \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Deployment Request",
    "message": "v1.2.3 ready for production",
    "actions": [
      {"label": "Approve", "url": "https://api.example.com/deploy/123/approve", "method": "POST"},
      {"label": "Reject", "url": "https://api.example.com/deploy/123/reject", "method": "POST"}
    ]
  }'
```

#### Action with Headers and Body

```bash
curl -X POST http://localhost:9876/notify \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Review Required",
    "message": "Merge request waiting",
    "actions": [
      {
        "label": "Approve",
        "url": "https://api.example.com/reviews",
        "method": "POST",
        "headers": {"Authorization": "Bearer token123"},
        "body": "{\"action\": \"approve\", \"id\": 456}"
      }
    ]
  }'
```

### Action Behavior

- **Button click**: Shows loading indicator while request runs
- **Success (2xx)**: Notification flashes green, then dismisses
- **Error**: Notification dismisses, error notification appears
- **Open action**: Opens URL in default browser (no HTTP request)

## Exclusive Notifications

Exclusive notifications coordinate actions across multiple connected clients. When any client takes an action on an exclusive notification, it resolves for everyone.

This is useful for scenarios like deployment approvals where only one person should take the action.

### How It Works

1. Send notification with `"exclusive": true` to the server
2. Server assigns a unique ID and broadcasts to all clients
3. When any client clicks an action button:
   - Client sends action request to server
   - Server executes the HTTP request
   - Server broadcasts "resolved" to all clients
   - All clients dismiss the notification

### Client Identity

Clients identify themselves with a display name for logging and tracking:

```bash
./cross-notifier -connect ws://server:9876/ws -secret mysecret -name "Bart"
```

Or via environment variable:

```bash
export CROSS_NOTIFIER_NAME="Bart"
```

### Exclusive Notification Example

```bash
curl -X POST http://server:9876/notify \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer mysecret" \
  -d '{
    "title": "Deploy to Production",
    "message": "v1.2.3 ready for deployment",
    "exclusive": true,
    "actions": [
      {"label": "Approve", "url": "https://api.example.com/deploy", "method": "POST"},
      {"label": "Reject", "url": "https://api.example.com/reject", "method": "POST"}
    ]
  }'
```

When Bart clicks "Approve":
- Server executes POST to `https://api.example.com/deploy`
- All connected clients see the notification dismissed
- Server logs: `Client 'Bart' executing action 'Approve' on notification <id>`

### Non-Exclusive vs Exclusive

| Behavior | Non-Exclusive | Exclusive |
|----------|---------------|-----------|
| Action execution | Client executes directly | Server executes |
| Other clients | Unaffected | Notification dismissed |
| Use case | Personal notifications | Team coordination |

## Testing

Use the included test script for local notifications:

```bash
./test-notify.sh "Title" "Message"
./test-notify.sh "With Icon" "Nice!" /path/to/icon.png
```

## Interaction

- **Click** a notification to dismiss it
- Notifications auto-dismiss after their duration expires
- When more than 4 notifications are queued, a "+ N more" indicator appears
- Connection status notifications appear when connecting/disconnecting from a remote server
