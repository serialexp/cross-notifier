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
