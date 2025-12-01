#!/bin/bash
# ABOUTME: Test script for sending notifications to the daemon.
# ABOUTME: Usage: ./test-notify.sh [title] [message] [icon_path] [count]

TITLE="${1:-Test Notification}"
MESSAGE="${2:-This is a test message}"
ICON="${3:-}"
COUNT="${4:-1}"

send_notification() {
    local num="$1"
    if [ -n "$ICON" ]; then
        curl -s -X POST http://localhost:9876/notify \
            -H "Content-Type: application/json" \
            -d "{\"title\":\"$TITLE #$num\",\"message\":\"$MESSAGE\",\"iconPath\":\"$ICON\"}"
    else
        curl -s -X POST http://localhost:9876/notify \
            -H "Content-Type: application/json" \
            -d "{\"title\":\"$TITLE #$num\",\"message\":\"$MESSAGE\"}"
    fi
}

# Send notifications in parallel
for i in $(seq 1 $COUNT); do
    send_notification "$i" &
done

wait
echo "Sent $COUNT notification(s)"
