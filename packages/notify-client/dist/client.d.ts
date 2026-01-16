import type { Notification, ClientOptions, SendResult } from "./types.js";
/**
 * Client for sending notifications to a cross-notifier server.
 */
export declare class NotifyClient {
    private readonly serverUrl;
    private readonly token;
    private readonly defaultSource?;
    constructor(options: ClientOptions);
    /**
     * Send a notification to the server.
     * @param notification The notification to send
     * @returns Result indicating success or failure
     */
    send(notification: Notification): Promise<SendResult>;
}
//# sourceMappingURL=client.d.ts.map