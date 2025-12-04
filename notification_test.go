// ABOUTME: Tests for notification struct and status handling.
// ABOUTME: Covers JSON parsing and status field validation.

package main

import (
	"encoding/json"
	"testing"
)

func TestNotificationStatusParsing(t *testing.T) {
	tests := []struct {
		name     string
		json     string
		expected string
	}{
		{
			name:     "info status",
			json:     `{"title": "Test", "status": "info"}`,
			expected: "info",
		},
		{
			name:     "success status",
			json:     `{"title": "Test", "status": "success"}`,
			expected: "success",
		},
		{
			name:     "warning status",
			json:     `{"title": "Test", "status": "warning"}`,
			expected: "warning",
		},
		{
			name:     "error status",
			json:     `{"title": "Test", "status": "error"}`,
			expected: "error",
		},
		{
			name:     "empty status defaults to empty",
			json:     `{"title": "Test"}`,
			expected: "",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var n Notification
			if err := json.Unmarshal([]byte(tt.json), &n); err != nil {
				t.Fatalf("failed to unmarshal: %v", err)
			}
			if n.Status != tt.expected {
				t.Errorf("got status %q, want %q", n.Status, tt.expected)
			}
		})
	}
}
