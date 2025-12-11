#!/bin/bash
# ABOUTME: Stress test script for sending randomized notifications to the daemon.
# ABOUTME: Usage: ./test-notify.sh [count]

set -e

# Load .env file if it exists
if [ -f .env ]; then
    export $(grep -v '^#' .env | xargs)
fi

COUNT="${1:-10}"

# Validate count is a number
if ! [[ "$COUNT" =~ ^[0-9]+$ ]]; then
    echo "Usage: ./test-notify.sh [count]"
    echo "  count: number of notifications to send (default: 10)"
    exit 1
fi

# Server URL and secret (from .env or env vars)
SERVER_URL="${CROSS_NOTIFIER_SERVER:-http://localhost:9876}"
SECRET="${CROSS_NOTIFIER_SECRET:-}"

# --- Random data pools ---

TITLES=(
    "Build Complete"
    "Deployment Failed"
    "New Message"
    "Alert"
    "System Update"
    "Task Finished"
    "Warning: High CPU Usage"
    "Error in Production"
    "PR Review Requested"
    "Tests Passed"
    "Database Backup Complete"
    "Security Alert: Unusual Login Detected"
    "This is an extremely long title that goes on and on to test how the UI handles very long title text overflow"
)

MESSAGES_SHORT=(
    "Done."
    "OK"
    "Failed"
    "Check now"
    "Action required"
)

MESSAGES_MEDIUM=(
    "The build completed successfully in 2m 34s."
    "Deployment to staging failed. Check logs for details."
    "You have 3 new messages from the team."
    "CPU usage has exceeded 90% for the last 5 minutes."
    "All 47 tests passed. Coverage: 89%."
)

MESSAGES_LONG=(
    "The nightly backup job completed successfully. 1.2TB of data was backed up to the offsite storage in 3 hours and 47 minutes. Next backup scheduled for tomorrow at 2:00 AM."
    "CRITICAL: Multiple failed login attempts detected from IP 192.168.1.100. The account has been temporarily locked. Please review the security logs and take appropriate action if this was not you."
    "Your pull request #1234 'Refactor authentication module' has been reviewed by 3 team members. There are 2 comments requiring your attention and 1 approval. The CI pipeline is currently running."
)

MESSAGES_MULTILINE=(
    "Line 1: Build started\nLine 2: Compiling sources\nLine 3: Running tests\nLine 4: Build complete"
    "Error Details:\n- Connection refused\n- Retry count: 3\n- Last attempt: 10:45 AM"
    "Summary:\n• 15 files changed\n• 234 insertions\n• 89 deletions\n• 3 conflicts resolved"
)

STATUSES=("" "info" "success" "warning" "error")

SOURCES=("ci" "deploy" "monitor" "security" "backup" "test" "github" "slack")

ICON_URLS=(
    ""
    "https://cdn-icons-png.flaticon.com/128/1828/1828640.png"
    "https://cdn-icons-png.flaticon.com/128/1828/1828643.png"
    "https://cdn-icons-png.flaticon.com/128/1828/1828665.png"
    "https://cdn-icons-png.flaticon.com/128/3388/3388621.png"
    "https://cdn-icons-png.flaticon.com/128/2810/2810051.png"
)

ACTION_LABELS=("View" "Open" "Dismiss" "Retry" "Details" "Approve" "Reject" "Snooze")

ACTION_URLS=(
    "https://example.com"
    "https://github.com"
    "https://httpbin.org/post"
    "https://httpbin.org/get"
)

# --- Helper functions ---

# Pick random element from array (bash 3.2 compatible)
random_element() {
    local arr_name=$1
    eval "local arr=(\"\${${arr_name}[@]}\")"
    local count=${#arr[@]}
    echo "${arr[$RANDOM % $count]}"
}

random_bool() {
    (( RANDOM % 2 ))
}

random_range() {
    local min=$1 max=$2
    echo $(( RANDOM % (max - min + 1) + min ))
}

# Escape string for JSON
json_escape() {
    local str="$1"
    str="${str//\\/\\\\}"
    str="${str//\"/\\\"}"
    str="${str//$'\n'/\\n}"
    str="${str//$'\t'/\\t}"
    echo "$str"
}

# Pick a random message (short, medium, long, or multiline)
random_message() {
    local type=$(( RANDOM % 4 ))
    case $type in
        0) random_element MESSAGES_SHORT ;;
        1) random_element MESSAGES_MEDIUM ;;
        2) random_element MESSAGES_LONG ;;
        3) random_element MESSAGES_MULTILINE ;;
    esac
}

# Generate random actions (0-3 actions)
generate_actions() {
    local num_actions=$(( RANDOM % 4 ))  # 0-3 actions

    if [ $num_actions -eq 0 ]; then
        echo ""
        return
    fi

    local actions=""
    for (( i=0; i<num_actions; i++ )); do
        local label=$(random_element ACTION_LABELS)
        local url=$(random_element ACTION_URLS)
        local open_in_browser=$(random_bool && echo "true" || echo "false")

        local action="{\"label\":\"$label\",\"url\":\"$url\""

        # Sometimes add HTTP method and body for POST URLs
        if [[ "$url" == *"post"* ]] && ! $open_in_browser; then
            action="$action,\"method\":\"POST\",\"body\":\"{\\\"test\\\":true}\""
            action="$action,\"headers\":{\"Content-Type\":\"application/json\"}"
        fi

        if [ "$open_in_browser" = "true" ]; then
            action="$action,\"open\":true"
        fi

        action="$action}"

        if [ -n "$actions" ]; then
            actions="$actions,$action"
        else
            actions="$action"
        fi
    done

    echo "[$actions]"
}

send_notification() {
    local num="$1"

    # Pick random values
    local title=$(json_escape "$(random_element TITLES)")
    local message=$(json_escape "$(random_message)")
    local status=$(random_element STATUSES)
    local source=$(random_element SOURCES)
    local icon_url=$(random_element ICON_URLS)
    local actions=$(generate_actions)

    # Random duration: 0 (persistent), or 5-30 seconds
    local duration=0
    if random_bool; then
        duration=$(random_range 5 30)
    fi

    # Random exclusive flag (less common)
    local exclusive="false"
    if (( RANDOM % 5 == 0 )); then
        exclusive="true"
    fi

    # Build JSON
    local json="{\"title\":\"$title\",\"message\":\"$message\",\"source\":\"$source\""

    if [ -n "$status" ]; then
        json="$json,\"status\":\"$status\""
    fi

    if [ -n "$icon_url" ]; then
        json="$json,\"iconHref\":\"$icon_url\""
    fi

    if [ $duration -gt 0 ]; then
        json="$json,\"duration\":$duration"
    fi

    if [ "$exclusive" = "true" ]; then
        json="$json,\"exclusive\":true"
    fi

    if [ -n "$actions" ]; then
        json="$json,\"actions\":$actions"
    fi

    json="$json}"

    # Send request
    local auth_header=""
    if [ -n "$SECRET" ]; then
        auth_header="-H \"Authorization: Bearer $SECRET\""
    fi

    if [ -n "$SECRET" ]; then
        curl -s -X POST "$SERVER_URL/notify" \
            -H "Content-Type: application/json" \
            -H "Authorization: Bearer $SECRET" \
            -d "$json" > /dev/null
    else
        curl -s -X POST "$SERVER_URL/notify" \
            -H "Content-Type: application/json" \
            -d "$json" > /dev/null
    fi

    echo "[$num] Sent: $title (status=$status, duration=$duration, actions=${actions:+yes}${actions:-no}, exclusive=$exclusive)"
}

echo "Sending $COUNT random notifications to $SERVER_URL..."
echo ""

for i in $(seq 1 $COUNT); do
    send_notification "$i"
    # Small delay to avoid overwhelming
    sleep 0.1
done

echo ""
echo "Done. Sent $COUNT notification(s)."
