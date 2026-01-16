/**
 * Notification status levels for visual styling.
 */
export type NotificationStatus = "info" | "success" | "warning" | "error";
/**
 * An action button displayed on a notification.
 * When clicked, either opens a URL in the browser or makes an HTTP request.
 */
export interface Action {
    /** Button label text */
    label: string;
    /** URL to open or HTTP endpoint to call */
    url: string;
    /** HTTP method for the request. Defaults to GET. Ignored if open is true. */
    method?: string;
    /** HTTP headers to include in the request. Ignored if open is true. */
    headers?: Record<string, string>;
    /** HTTP request body. Ignored if open is true. */
    body?: string;
    /** If true, opens URL in browser instead of making HTTP request */
    open?: boolean;
}
/**
 * A notification to display on connected clients.
 */
export interface Notification {
    /** Server-assigned ID. Optional when sending; filled by server if omitted. */
    id?: string;
    /** Identifier of the service sending this notification (e.g., "jenkins", "github") */
    source: string;
    /** Notification title */
    title: string;
    /** Notification message body */
    message: string;
    /** Visual status level. Affects styling. */
    status?: NotificationStatus;
    /** Base64-encoded image data for the icon */
    iconData?: string;
    /** URL to fetch icon from. Server fetches and converts to iconData. */
    iconHref?: string;
    /** Auto-dismiss duration in seconds. 0 or omitted = persistent until dismissed. */
    duration?: number;
    /** Action buttons to display on the notification */
    actions?: Action[];
    /** If true, notification is resolved when any connected client takes action */
    exclusive?: boolean;
}
/**
 * Options for the notification client.
 */
export interface ClientOptions {
    /** Server URL (e.g., "http://localhost:9876") */
    server: string;
    /** Authentication token (Bearer token) */
    token: string;
    /** Default source for notifications if not specified per-notification */
    defaultSource?: string;
}
/**
 * Response from the server after sending a notification.
 */
export interface SendResult {
    /** Whether the notification was accepted */
    ok: boolean;
    /** Error message if not ok */
    error?: string;
    /** HTTP status code */
    status: number;
}
//# sourceMappingURL=types.d.ts.map