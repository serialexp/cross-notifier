#!/bin/bash
# ABOUTME: Test script for sending notifications to the daemon.
# ABOUTME: Usage: ./test-notify.sh [title] [message] [icon_path]

TITLE="${1:-Test Notification}"
MESSAGE="${2:-This is a test message}"
ICON="${3:-}"

if [ -n "$ICON" ]; then
    curl -s -X POST http://localhost:9876/notify \
        -H "Content-Type: application/json" \
        -d "{\"title\":\"$TITLE\",\"message\":\"$MESSAGE\",\"icon\":\"$ICON\"}"
else
    curl -s -X POST http://localhost:9876/notify \
        -H "Content-Type: application/json" \
        -d "{\"title\":\"$TITLE\",\"message\":\"$MESSAGE\"}"
fi

echo ""
