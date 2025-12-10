# Notification Center Implementation Plan

## Overview

Add a notification center that persists notifications until explicitly dismissed. Notifications no longer vanish when they time out - they move to the center for later review.

## Architecture

```
┌─────────────────┐     HTTP API      ┌────────────────────┐
│  Daemon Process │◄─────────────────►│ Center Window      │
│                 │                   │ (separate process) │
│  - Popup UI     │  GET /center      │                    │
│  - Notification │  DELETE /center/N │  - List view       │
│    Store        │  DELETE /center   │  - Dismiss actions │
│  - JSON persist │                   │                    │
└─────────────────┘                   └────────────────────┘
```

The daemon owns the notification store. The center window queries via HTTP (same pattern as settings).

## Components

### 1. Notification Store (`store.go`)

New file for persistence:

```go
type NotificationStore struct {
    Notifications []StoredNotification
    path          string
    mu            sync.RWMutex
}

type StoredNotification struct {
    ID        int64           `json:"id"`
    Payload   json.RawMessage `json:"payload"`   // Original JSON for reprocessing
    CreatedAt time.Time       `json:"createdAt"`
}
```

- `Add(notification, rawJSON)` - save to store + write to disk
- `Remove(id)` - remove from store + write to disk
- `Clear()` - remove all + write to disk
- `List()` - return all stored notifications
- `Load()` - read from disk on startup, reprocess through rules

Storage path: `~/Library/Application Support/cross-notifier/notifications.json`

### 2. HTTP API (in `main.go`)

New endpoints:

- `GET /center` - list all stored notifications (JSON array)
- `DELETE /center/{id}` - dismiss single notification
- `DELETE /center` - dismiss all (requires `?confirm=true` query param)

### 3. Tray Menu (in `tray.go`)

Add menu item between Status and Settings:

```
Connected to 1 server
─────────────────────
Notifications (3)      ← NEW: opens center window
─────────────────────
Settings...
Quit
```

- Update count via periodic polling (reuse existing status update goroutine)
- Click launches center window process: `./cross-notifier -center`

### 4. Center Window (`center.go`)

New file, similar structure to `settings.go`:

- Separate giu window launched with `-center` flag
- Fetches notifications from daemon via `GET /center`
- Displays scrollable list of notification cards
- Click notification → execute action (same as popup) → `DELETE /center/{id}`
- "Dismiss All" button → confirmation dialog → `DELETE /center?confirm=true`
- Auto-refresh every few seconds (or use polling)

### 5. Notification Lifecycle Changes (`main.go`)

Current flow:
```
arrive → addNotification() → popup → timeout/click → gone
```

New flow:
```
arrive → store.Add() → addNotification() → popup → timeout → still in store
                                                 → click  → store.Remove() + dismiss popup
```

Changes to `addNotification()`:
- Before adding to popup list, save to store (unless rule says dismiss)
- Pass raw JSON payload for storage

Changes to `dismissNotification()`:
- Also call `store.Remove(id)`

Changes to `pruneExpired()`:
- Only removes from popup list, NOT from store
- Notification stays in center until explicitly dismissed

### 6. Rule Changes (`config.go`)

Expand `NotificationRule`:

```go
type NotificationRule struct {
    // ... existing filters ...

    Sound    string `json:"sound,omitempty"`
    Action   string `json:"action,omitempty"` // "normal" (default), "silent", "dismiss"
}
```

- **normal** (default): popup + center + sound
- **silent**: center only (no popup, no sound) - for low-priority batch review
- **dismiss**: neither (current suppress behavior) - for true spam

Migrate existing `Suppress bool` → `Action: "dismiss"`

### 7. Settings UI Updates (`settings.go`)

Change rule action from checkbox to dropdown:
- Remove "Suppress" checkbox
- Add "Action" dropdown: Normal / Silent / Dismiss

## File Changes Summary

| File | Change |
|------|--------|
| `store.go` | NEW - notification persistence |
| `center.go` | NEW - center window UI |
| `main.go` | Add HTTP endpoints, integrate store |
| `tray.go` | Add "Notifications (N)" menu item |
| `config.go` | Add `Action` field to rules |
| `settings.go` | Replace Suppress checkbox with Action dropdown |
| `sound.go` | Update rule matching for new Action field |

## Implementation Order

1. `store.go` - persistence layer (can test independently)
2. HTTP API endpoints in `main.go`
3. `tray.go` - menu item (launches placeholder)
4. `center.go` - basic window that lists notifications
5. Wire up lifecycle (store on arrive, remove on dismiss)
6. Rule changes (action field, silent option)
7. Settings UI update
8. Polish: dismiss all, auto-refresh, badge count on tray

## Open Questions

- **Tray badge**: `fyne.io/systray` may not support badges. Investigate or skip for v1.
- **Max history**: Optional setting, off by default. Implement after core works.

## Rule Reprocessing

On normal startup, stored notifications load as-is (no rule reprocessing).

Settings UI gets a "Reprocess Rules" button that:
1. Triggers daemon to reload stored notifications through current rules
2. Notifications matching "dismiss" rules get removed
3. Notifications matching "silent" rules stay in center (already there)
4. Useful when user adds new rules and wants them applied retroactively

Implementation: new HTTP endpoint `POST /center/reprocess` that the settings window calls.

## Config Migration

Existing configs with `"suppress": true` should be migrated to `"action": "dismiss"`. Handle in `LoadConfig()` or document as breaking change for v0.6.0.
