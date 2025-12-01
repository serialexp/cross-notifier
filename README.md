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

## Installation

### macOS

Build the app bundle and DMG:

```bash
./build-macos.sh --dmg
```

Then open `CrossNotifier-1.0.0.dmg` and drag to Applications.

### From source

```bash
go build -o cross-notifier .
./cross-notifier
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
