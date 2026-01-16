// ABOUTME: HTTP client for sending notifications to a cross-notifier server.
// ABOUTME: Uses native fetch to POST notifications with Bearer token auth.
/**
 * Client for sending notifications to a cross-notifier server.
 */
export class NotifyClient {
    serverUrl;
    token;
    defaultSource;
    constructor(options) {
        // Normalize server URL: remove trailing slash
        this.serverUrl = options.server.replace(/\/+$/, "");
        this.token = options.token;
        this.defaultSource = options.defaultSource;
    }
    /**
     * Send a notification to the server.
     * @param notification The notification to send
     * @returns Result indicating success or failure
     */
    async send(notification) {
        const payload = {
            ...notification,
            source: notification.source || this.defaultSource || "",
        };
        const response = await fetch(`${this.serverUrl}/notify`, {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
                Authorization: `Bearer ${this.token}`,
            },
            body: JSON.stringify(payload),
        });
        const ok = response.ok;
        let error;
        if (!ok) {
            try {
                const body = await response.json();
                error = body.error || `HTTP ${response.status}`;
            }
            catch {
                error = `HTTP ${response.status}`;
            }
        }
        return {
            ok,
            status: response.status,
            error,
        };
    }
}
//# sourceMappingURL=client.js.map