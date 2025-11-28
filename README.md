# CrossNotifier

A cross-platform notification daemon that displays desktop notifications via HTTP API.

## Features

- HTTP API on `localhost:9876`
- Supports title, message, and icon
- Configurable display duration
- Auto-dismiss with click-to-dismiss
- Stacks up to 4 notifications (queues extras)
- Adapts to OS dark/light theme
- Transparent, rounded notification cards

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

Start the daemon, then send HTTP POST requests to display notifications.

### API Endpoint

```
POST http://localhost:9876/notify
Content-Type: application/json
```

### JSON Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `title` | string | No* | - | Notification title |
| `message` | string | No* | - | Notification body text |
| `icon` | string | No | - | Absolute path to icon image (PNG, JPG) |
| `duration` | int | No | 5 | Seconds before auto-dismiss |

*At least one of `title` or `message` is required.

### Examples

#### curl

```bash
# Simple notification
curl -X POST http://localhost:9876/notify \
  -H "Content-Type: application/json" \
  -d '{"title":"Hello","message":"World"}'

# With icon
curl -X POST http://localhost:9876/notify \
  -H "Content-Type: application/json" \
  -d '{"title":"Build Complete","message":"Success!","icon":"/path/to/icon.png"}'

# Custom duration (10 seconds)
curl -X POST http://localhost:9876/notify \
  -H "Content-Type: application/json" \
  -d '{"title":"Slow","message":"Stays for 10s","duration":10}'
```

#### Python

```python
import requests

requests.post("http://localhost:9876/notify", json={
    "title": "Task Complete",
    "message": "Your build finished successfully",
    "icon": "/path/to/icon.png"
})
```

#### JavaScript/Node

```javascript
fetch("http://localhost:9876/notify", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    title: "New Message",
    message: "You have a notification"
  })
});
```

#### Go

```go
import "net/http"
import "strings"

http.Post("http://localhost:9876/notify", "application/json",
    strings.NewReader(`{"title":"Hello","message":"From Go"}`))
```

## Testing

Use the included test script:

```bash
./test-notify.sh "Title" "Message"
./test-notify.sh "With Icon" "Nice!" /path/to/icon.png
```

## Interaction

- **Click** a notification to dismiss it
- Notifications auto-dismiss after their duration expires
- When more than 4 notifications are queued, a "+ N more" indicator appears
