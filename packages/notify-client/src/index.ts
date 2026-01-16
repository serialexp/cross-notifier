// ABOUTME: Public API exports for the cross-notifier client library.
// ABOUTME: Re-exports the NotifyClient class and all related types.

export { NotifyClient } from "./client.js";
export type {
  Notification,
  Action,
  NotificationStatus,
  ClientOptions,
  SendResult,
} from "./types.js";
