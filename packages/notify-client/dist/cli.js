#!/usr/bin/env node
// ABOUTME: CLI for sending notifications to a cross-notifier server.
// ABOUTME: Uses parseArgs from node:util for argument parsing.
import { parseArgs } from "node:util";
import { NotifyClient } from "./client.js";
const { values, positionals } = parseArgs({
    options: {
        server: { type: "string", short: "s" },
        token: { type: "string", short: "t" },
        title: { type: "string" },
        message: { type: "string", short: "m" },
        source: { type: "string" },
        status: { type: "string" },
        icon: { type: "string", short: "i" },
        duration: { type: "string", short: "d" },
        exclusive: { type: "boolean", short: "x" },
        action: { type: "string", multiple: true, short: "a" },
        help: { type: "boolean", short: "h" },
    },
    allowPositionals: true,
});
function printHelp() {
    console.log(`crossnotify - Send notifications to a cross-notifier server

Usage:
  crossnotify --server URL --token SECRET --title "Title" --message "Message"

Required:
  --server, -s URL       Server URL (e.g., http://localhost:9876)
  --token, -t SECRET     Authentication token
  --title TITLE          Notification title
  --message, -m MSG      Notification message

Optional:
  --source NAME          Source identifier (e.g., "jenkins", "github")
  --status STATUS        Status level: info, success, warning, error
  --icon, -i URL         Icon URL
  --duration, -d SECS    Auto-dismiss after N seconds (0 = persistent)
  --exclusive, -x        Resolve when any client takes action
  --action, -a JSON      Action button (can be repeated). Format:
                         '{"label":"View","url":"https://...","open":true}'
  --help, -h             Show this help message

Examples:
  crossnotify -s http://localhost:9876 -t secret --title "Build" -m "Build complete"

  crossnotify -s http://server:9876 -t secret \\
    --title "Deploy" --message "Deployed to prod" \\
    --status success --source jenkins \\
    -a '{"label":"View","url":"https://jenkins/build/123","open":true}'
`);
}
async function main() {
    if (values.help) {
        printHelp();
        process.exit(0);
    }
    const missing = [];
    if (!values.server)
        missing.push("--server");
    if (!values.token)
        missing.push("--token");
    if (!values.title)
        missing.push("--title");
    if (!values.message)
        missing.push("--message");
    if (missing.length > 0) {
        console.error(`Error: Missing required arguments: ${missing.join(", ")}`);
        console.error("Use --help for usage information.");
        process.exit(1);
    }
    // Parse actions from JSON strings
    let actions;
    if (values.action && values.action.length > 0) {
        actions = [];
        for (const actionJson of values.action) {
            try {
                const action = JSON.parse(actionJson);
                if (!action.label || !action.url) {
                    console.error(`Error: Action must have "label" and "url" fields: ${actionJson}`);
                    process.exit(1);
                }
                actions.push(action);
            }
            catch (e) {
                console.error(`Error: Invalid JSON for --action: ${actionJson}`);
                process.exit(1);
            }
        }
    }
    // Validate status
    const validStatuses = ["info", "success", "warning", "error"];
    if (values.status && !validStatuses.includes(values.status)) {
        console.error(`Error: Invalid status "${values.status}". Must be one of: ${validStatuses.join(", ")}`);
        process.exit(1);
    }
    // Parse duration
    let duration;
    if (values.duration) {
        duration = parseInt(values.duration, 10);
        if (isNaN(duration) || duration < 0) {
            console.error(`Error: Invalid duration "${values.duration}". Must be a non-negative integer.`);
            process.exit(1);
        }
    }
    const client = new NotifyClient({
        server: values.server,
        token: values.token,
    });
    const notification = {
        source: values.source || "cli",
        title: values.title,
        message: values.message,
        status: values.status,
        iconHref: values.icon,
        duration,
        exclusive: values.exclusive,
        actions,
    };
    try {
        const result = await client.send(notification);
        if (result.ok) {
            console.log("Notification sent successfully.");
        }
        else {
            console.error(`Error: Server returned ${result.status}${result.error ? `: ${result.error}` : ""}`);
            process.exit(1);
        }
    }
    catch (e) {
        console.error(`Error: Failed to send notification: ${e instanceof Error ? e.message : e}`);
        process.exit(1);
    }
}
main();
//# sourceMappingURL=cli.js.map