// ABOUTME: Tests for notification action execution.
// ABOUTME: Verifies HTTP requests are made correctly and responses are handled.

package main

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestActionJSONParsing(t *testing.T) {
	tests := []struct {
		name     string
		json     string
		expected Action
	}{
		{
			name: "minimal action with just label and url",
			json: `{"label": "View", "url": "https://example.com"}`,
			expected: Action{
				Label: "View",
				URL:   "https://example.com",
			},
		},
		{
			name: "POST action with body",
			json: `{"label": "Approve", "url": "https://api.example.com/approve", "method": "POST", "body": "{\"approved\": true}"}`,
			expected: Action{
				Label:  "Approve",
				URL:    "https://api.example.com/approve",
				Method: "POST",
				Body:   `{"approved": true}`,
			},
		},
		{
			name: "action with headers",
			json: `{"label": "Submit", "url": "https://api.example.com", "method": "POST", "headers": {"Content-Type": "application/json", "X-Custom": "value"}}`,
			expected: Action{
				Label:  "Submit",
				URL:    "https://api.example.com",
				Method: "POST",
				Headers: map[string]string{
					"Content-Type": "application/json",
					"X-Custom":     "value",
				},
			},
		},
		{
			name: "open action",
			json: `{"label": "Open in Browser", "url": "https://example.com", "open": true}`,
			expected: Action{
				Label: "Open in Browser",
				URL:   "https://example.com",
				Open:  true,
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var action Action
			err := json.Unmarshal([]byte(tt.json), &action)
			if err != nil {
				t.Fatalf("failed to parse JSON: %v", err)
			}

			if action.Label != tt.expected.Label {
				t.Errorf("Label: got %q, want %q", action.Label, tt.expected.Label)
			}
			if action.URL != tt.expected.URL {
				t.Errorf("URL: got %q, want %q", action.URL, tt.expected.URL)
			}
			if action.Method != tt.expected.Method {
				t.Errorf("Method: got %q, want %q", action.Method, tt.expected.Method)
			}
			if action.Body != tt.expected.Body {
				t.Errorf("Body: got %q, want %q", action.Body, tt.expected.Body)
			}
			if action.Open != tt.expected.Open {
				t.Errorf("Open: got %v, want %v", action.Open, tt.expected.Open)
			}
			for k, v := range tt.expected.Headers {
				if action.Headers[k] != v {
					t.Errorf("Headers[%q]: got %q, want %q", k, action.Headers[k], v)
				}
			}
		})
	}
}

func TestNotificationWithActionsJSONParsing(t *testing.T) {
	jsonStr := `{
		"title": "PR Ready",
		"message": "PR #123 needs review",
		"actions": [
			{"label": "Approve", "url": "https://api.example.com/approve", "method": "POST"},
			{"label": "View", "url": "https://github.com/owner/repo/pull/123", "open": true}
		]
	}`

	var n Notification
	err := json.Unmarshal([]byte(jsonStr), &n)
	if err != nil {
		t.Fatalf("failed to parse notification JSON: %v", err)
	}

	if n.Title != "PR Ready" {
		t.Errorf("Title: got %q, want %q", n.Title, "PR Ready")
	}
	if len(n.Actions) != 2 {
		t.Fatalf("Actions: got %d, want 2", len(n.Actions))
	}
	if n.Actions[0].Label != "Approve" {
		t.Errorf("Actions[0].Label: got %q, want %q", n.Actions[0].Label, "Approve")
	}
	if n.Actions[1].Open != true {
		t.Errorf("Actions[1].Open: got %v, want true", n.Actions[1].Open)
	}
}

func TestExecuteActionHTTPRequest(t *testing.T) {
	tests := []struct {
		name           string
		action         Action
		serverResponse int
		wantErr        bool
	}{
		{
			name: "GET request success",
			action: Action{
				Label: "Check",
				URL:   "", // will be set to test server URL
			},
			serverResponse: http.StatusOK,
			wantErr:        false,
		},
		{
			name: "POST request success",
			action: Action{
				Label:  "Submit",
				URL:    "",
				Method: "POST",
				Body:   `{"data": "test"}`,
			},
			serverResponse: http.StatusCreated,
			wantErr:        false,
		},
		{
			name: "request with headers",
			action: Action{
				Label:  "Auth Request",
				URL:    "",
				Method: "POST",
				Headers: map[string]string{
					"Authorization": "Bearer token123",
					"Content-Type":  "application/json",
				},
			},
			serverResponse: http.StatusOK,
			wantErr:        false,
		},
		{
			name: "server error",
			action: Action{
				Label: "Fail",
				URL:   "",
			},
			serverResponse: http.StatusInternalServerError,
			wantErr:        true,
		},
		{
			name: "client error",
			action: Action{
				Label: "NotFound",
				URL:   "",
			},
			serverResponse: http.StatusNotFound,
			wantErr:        true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var receivedMethod string
			var receivedBody string
			var receivedHeaders http.Header

			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				receivedMethod = r.Method
				receivedHeaders = r.Header
				body, _ := io.ReadAll(r.Body)
				receivedBody = string(body)
				w.WriteHeader(tt.serverResponse)
			}))
			defer server.Close()

			action := tt.action
			action.URL = server.URL

			err := ExecuteAction(action)

			if tt.wantErr && err == nil {
				t.Error("expected error, got nil")
			}
			if !tt.wantErr && err != nil {
				t.Errorf("unexpected error: %v", err)
			}

			// Verify request details
			expectedMethod := action.Method
			if expectedMethod == "" {
				expectedMethod = "GET"
			}
			if receivedMethod != expectedMethod {
				t.Errorf("Method: got %q, want %q", receivedMethod, expectedMethod)
			}

			if action.Body != "" && receivedBody != action.Body {
				t.Errorf("Body: got %q, want %q", receivedBody, action.Body)
			}

			for k, v := range action.Headers {
				if receivedHeaders.Get(k) != v {
					t.Errorf("Header %q: got %q, want %q", k, receivedHeaders.Get(k), v)
				}
			}
		})
	}
}

func TestExecuteActionOpenURL(t *testing.T) {
	// For open actions, we need to verify that it doesn't make an HTTP request
	// and returns successfully (the actual browser opening is OS-dependent)
	requestReceived := false
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requestReceived = true
		w.WriteHeader(http.StatusOK)
	}))
	defer server.Close()

	action := Action{
		Label: "Open",
		URL:   server.URL,
		Open:  true,
	}

	// ExecuteAction with Open=true should not make an HTTP request
	// It should call the OS to open the URL in a browser
	// For testing, we'll verify it doesn't hit our test server
	err := ExecuteAction(action)
	if err != nil {
		t.Errorf("unexpected error: %v", err)
	}

	if requestReceived {
		t.Error("Open action should not make HTTP request")
	}
}

func TestActionEffectiveMethod(t *testing.T) {
	tests := []struct {
		action   Action
		expected string
	}{
		{Action{Method: ""}, "GET"},
		{Action{Method: "GET"}, "GET"},
		{Action{Method: "POST"}, "POST"},
		{Action{Method: "PUT"}, "PUT"},
		{Action{Method: "DELETE"}, "DELETE"},
		{Action{Method: "get"}, "GET"}, // should be normalized to uppercase
	}

	for _, tt := range tests {
		got := tt.action.EffectiveMethod()
		if got != tt.expected {
			t.Errorf("EffectiveMethod() for %q: got %q, want %q", tt.action.Method, got, tt.expected)
		}
	}
}
