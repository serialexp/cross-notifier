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
- Calendar integration: reminders and daily agenda from ICS/CalDAV

## Installation

### Quick Install (Linux/macOS)

**Standard install:**
```bash
curl -fsSL https://raw.githubusercontent.com/serialexp/cross-notifier/main/install.sh | bash
```

**Install with auto-start on login:**
```bash
curl -fsSL https://raw.githubusercontent.com/serialexp/cross-notifier/main/install.sh | bash -s -- --enable-service
```

This will automatically download and install the latest release for your platform.

**On macOS**, the installer will:
- Install the app to `/Applications`
- Optionally enable auto-start on login via LaunchAgent (with `--enable-service` flag or interactive prompt)

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
| `status` | string | No | - | Visual style: `info`, `success`, `warning`, `error` |
| `iconPath` | string | No | - | Local file path to icon (PNG, JPG) |
| `iconHref` | string | No | - | URL to fetch icon from (server fetches and resizes) |
| `iconData` | string | No | - | Base64-encoded icon image |
| `duration` | int | No | 5 | Seconds before auto-dismiss |
| `actions` | array | No | - | Action buttons (see Actions section) |
| `exclusive` | bool | No | false | Server coordinates actions across all clients |
| `wait` | int | No | 25 | Seconds POST blocks waiting for a client response. Implies `exclusive`. |
| `maxWait` | int | No | = `wait` | Total lifetime (seconds). After this, the server broadcasts `expired`. |

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

### Waiting for a Response (long polling)

Senders that need to know *which* action was taken can ask the server to
hold the HTTP response open until a client resolves the notification.

Set `wait` (seconds the initial POST blocks) and optionally `maxWait`
(total lifetime). Any non-zero value implies `exclusive`.

**Flow:**

1. `POST /notify` with `wait` / `maxWait` — server holds the connection.
2. If a client resolves within `wait` seconds → **200 OK** with a
   `ResolvedMessage` body: `{id, resolvedBy, actionLabel, success, error?}`.
3. If `wait` elapses but the notification is still live → **202 Accepted**
   with `{id, status: "pending"}` and a `Location: /notify/{id}/wait`
   header. Sender falls back to polling.
4. `GET /notify/{id}/wait?timeout=N` — blocks up to `N` seconds.
   Same three response shapes.
5. If `maxWait` elapses without resolution → **410 Gone** with
   `{id}`, plus an `expired` WebSocket broadcast to all clients so
   their UI stops offering the action buttons.

**Defaults are friendly to reverse proxies** (25s default for `wait`),
but no ceiling is enforced — if you're talking to localhost or own the
proxy, set `wait` to whatever you want.

```bash
curl -X POST http://server:9876/notify \
  -H "Authorization: Bearer mysecret" \
  -d '{
    "title": "Deploy to Production",
    "message": "v1.2.3 — approve?",
    "wait": 25,
    "maxWait": 300,
    "actions": [
      {"label": "Approve", "url": "https://api.example.com/deploy", "method": "POST"},
      {"label": "Reject",  "url": "https://api.example.com/reject", "method": "POST"}
    ]
  }'
```

### OpenAPI

The server exposes its spec at `GET /openapi.yaml` and `GET /openapi.json` (no auth required).

### Non-Exclusive vs Exclusive

| Behavior | Non-Exclusive | Exclusive |
|----------|---------------|-----------|
| Action execution | Client executes directly | Server executes |
| Other clients | Unaffected | Notification dismissed |
| Use case | Personal notifications | Team coordination |

## Calendar Integration

CrossNotifier can watch an ICS file, ICS URL, or CalDAV endpoint and fire
reminder notifications using each event's own VALARMs. Recurring events
(RRULE + EXDATE) are expanded locally. State is persisted as JSON so
in-flight snoozes survive restarts.

Both the daemon and the headless server can run the integration — in
each case delivery goes through the same notification pipeline
`/notify` uses, so calendar reminders look and behave like any other
notification, including on any connected remote client.

### Setting it up

**Daemon (desktop):** open Settings → Calendar, pick a source kind
(CalDAV / ICS URL / ICS File), fill in the URL (+ user/password for
CalDAV), and save. Changes take effect immediately — no daemon restart.

**Server (headless):** configure via environment variables. CalDAV takes
priority over ICS URL which takes priority over ICS file.

| Variable | Description |
|----------|-------------|
| `CALDAV_ENDPOINT` / `CALDAV_USER` / `CALDAV_PASSWORD` | CalDAV source (e.g. Fastmail). Endpoint points at a specific calendar collection. |
| `CAL_ICS_URL` (+ optional `CAL_ICS_USER` / `CAL_ICS_PASSWORD`) | Plain HTTP(S) ICS feed. |
| `CAL_ICS_FILE` | Absolute path to a local `.ics` file. |
| `CAL_HORIZON_HOURS` | Look-ahead window (default `48`). Events further out wait for the next refresh. |
| `CAL_REFRESH_MINUTES` | Re-fetch interval (default `5`). |
| `CAL_STATE_FILE` | Where to persist scheduler state (default `./calendar-state.json`). |
| `CAL_ACTION_BASE_URL` | Public URL the server hosts `/calendar/action` on. Defaults to `http://127.0.0.1:<port>/calendar/action`. Remote daemons need this to point at the server's real hostname. |
| `CAL_SUMMARY_AT` | `HH:MM` to enable the daily agenda summary (see below). |

### Reminders

Each matched VALARM fires a notification at its trigger time, titled with
the event summary and carrying two action buttons:

- **Snooze Nh** — reschedules the reminder `N` hours out (default 4,
  configurable per-daemon). Survives a calendar refresh that would
  otherwise drop the event.
- **Dismiss** — marks the occurrence delivered for the current cycle.

Actions POST to `/calendar/action/{snooze,dismiss}` on the same host
that delivered the notification. On the daemon these are localhost +
no-auth; on the server they require the shared secret.

### Daily Summary

Opt-in feature that fires one notification per day at a configured local
time listing tomorrow's events (with location when present). Empty days
still fire a "Nothing scheduled." card so you know the feature is alive.

- **Daemon:** Settings → Calendar → check *Daily summary* and pick a
  time. Stored as `calendar.dailySummary: { hour, minute }` in
  `config.json`.
- **Server:** set `CAL_SUMMARY_AT=HH:MM`. Leaving it unset disables.

Local-time based, so DST transitions are handled automatically — the
next fire is always rebased from `Local::now()`.

### Introspection

`GET /calendar/action/upcoming` returns the scheduler's pending fires as
JSON, sorted by effective fire time (soonest first). Same auth rule as
the action routes — localhost daemon is open; public server requires the
shared secret.

```bash
# Daemon
curl -s http://127.0.0.1:9876/calendar/action/upcoming | jq

# Server
curl -s -H "Authorization: Bearer $SECRET" \
  https://notifier.example.com/calendar/action/upcoming | jq
```

Each row:

```json
{
  "id": "a9f3b0…",
  "summary": "Dentist",
  "location": "Main St",
  "fireAt": "2026-04-24T08:45:00Z",
  "eventStart": "2026-04-24T09:00:00Z",
  "eventEnd": "2026-04-24T09:30:00Z",
  "snoozedUntil": "2026-04-24T13:00:00Z",
  "firedAt": "2026-04-24T09:00:00Z"
}
```

`snoozedUntil` + `firedAt` both set means "delivered once and re-armed
by a snooze." `firedAt` alone means "handled, in GC retention." No
`firedAt` means "still waiting to fire."

### Matching calendar notifications in rules

Every calendar notification — reminder or daily summary — is delivered
with `source: "calendar"`, so the daemon's rule engine (Settings →
Rules) can target them the same way it targets any other source. Use
this to pick a distinctive sound, silence them when heads-down, etc.

```json
{
  "rules": {
    "enabled": true,
    "rules": [
      { "source": "calendar", "pattern": "^Agenda ",  "sound": "Bell" },
      { "source": "calendar", "pattern": "Standup",   "action": "silent" },
      { "source": "calendar", "sound": "Chime" }
    ]
  }
}
```

First-match-wins, so order matters — put the narrow rules first. Useful
selectors given how the notifications are formatted:

- Daily summary: title always starts with `Agenda — `, so `^Agenda `
  isolates it from reminders.
- Reminder with a location: message format is `HH:MM · location`, so
  the literal `·` matches exactly those.
- Specific event: `pattern` is a regex over `title + message`.

### Scope and limitations

- Only `DISPLAY` / `AUDIO` VALARMs with a relative `TRIGGER` duration
  are honored. `EMAIL` alarms and absolute-datetime triggers are
  skipped.
- Floating (`TZID=`-less, non-UTC) times are treated as UTC with a
  debug log. Good enough for Fastmail-hosted personal calendars;
  non-UTC TZID handling is tracked for later.
- `RECURRENCE-ID` overrides aren't yet applied — the master RRULE's
  instance still emits at the original time.

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
