// ABOUTME: Defines notification actions and their execution.
// ABOUTME: Actions allow notifications to trigger HTTP requests or open URLs.

package main

import (
	"fmt"
	"net/http"
	"os/exec"
	"runtime"
	"strings"
	"sync"
	"time"
)

// ActionState represents the execution state of an action button.
type ActionState int

const (
	ActionIdle ActionState = iota
	ActionLoading
	ActionSuccess
	ActionError
)

// ActionStateInfo holds the state and error for an action.
type ActionStateInfo struct {
	State ActionState
	Error error
}

var (
	actionStates   = make(map[int64]map[int]*ActionStateInfo) // notification ID -> action index -> state
	actionStatesMu sync.Mutex
)

// GetActionState returns the current state of an action.
func GetActionState(notifID int64, actionIdx int) *ActionStateInfo {
	actionStatesMu.Lock()
	defer actionStatesMu.Unlock()

	if states, ok := actionStates[notifID]; ok {
		if info, ok := states[actionIdx]; ok {
			return info
		}
	}
	return &ActionStateInfo{State: ActionIdle}
}

// SetActionState updates the state of an action.
func SetActionState(notifID int64, actionIdx int, state ActionState, err error) {
	actionStatesMu.Lock()
	defer actionStatesMu.Unlock()

	if actionStates[notifID] == nil {
		actionStates[notifID] = make(map[int]*ActionStateInfo)
	}
	actionStates[notifID][actionIdx] = &ActionStateInfo{State: state, Error: err}
}

// CleanupActionStates removes state tracking for a notification.
func CleanupActionStates(notifID int64) {
	actionStatesMu.Lock()
	defer actionStatesMu.Unlock()
	delete(actionStates, notifID)
}

// ExecuteActionAsync runs an action in the background and updates state.
// Returns immediately. Calls onComplete when done (success or error).
func ExecuteActionAsync(notifID int64, actionIdx int, action Action, onSuccess func(), onError func(error)) {
	SetActionState(notifID, actionIdx, ActionLoading, nil)

	go func() {
		err := ExecuteAction(action)
		if err != nil {
			SetActionState(notifID, actionIdx, ActionError, err)
			// Brief delay to show error state before callback
			time.Sleep(100 * time.Millisecond)
			if onError != nil {
				onError(err)
			}
		} else {
			SetActionState(notifID, actionIdx, ActionSuccess, nil)
			// Brief delay to show success state before callback
			time.Sleep(300 * time.Millisecond)
			if onSuccess != nil {
				onSuccess()
			}
		}
	}()
}

// Action represents a clickable action button on a notification.
type Action struct {
	Label   string            `json:"label"`
	URL     string            `json:"url"`
	Method  string            `json:"method,omitempty"`  // HTTP method, defaults to GET
	Headers map[string]string `json:"headers,omitempty"` // HTTP headers
	Body    string            `json:"body,omitempty"`    // HTTP request body
	Open    bool              `json:"open,omitempty"`    // Open URL in browser instead of HTTP request
}

// EffectiveMethod returns the HTTP method to use, defaulting to GET.
func (a Action) EffectiveMethod() string {
	if a.Method == "" {
		return "GET"
	}
	return strings.ToUpper(a.Method)
}

// ExecuteAction performs the action, either opening a URL or making an HTTP request.
func ExecuteAction(a Action) error {
	if a.Open {
		return openURL(a.URL)
	}
	return executeHTTPAction(a)
}

// openURLFunc is the function used to open URLs. Replaceable for testing.
var openURLFunc = openURLDefault

// openURL opens the URL in the default browser.
func openURL(url string) error {
	return openURLFunc(url)
}

// openURLDefault is the default implementation that opens URLs in the browser.
func openURLDefault(url string) error {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("open", url)
	case "linux":
		cmd = exec.Command("xdg-open", url)
	case "windows":
		cmd = exec.Command("rundll32", "url.dll,FileProtocolHandler", url)
	default:
		return fmt.Errorf("unsupported platform: %s", runtime.GOOS)
	}
	return cmd.Start()
}

// executeHTTPAction makes an HTTP request based on the action configuration.
func executeHTTPAction(a Action) error {
	method := a.EffectiveMethod()
	var bodyReader *strings.Reader
	if a.Body != "" {
		bodyReader = strings.NewReader(a.Body)
	}

	var req *http.Request
	var err error
	if bodyReader != nil {
		req, err = http.NewRequest(method, a.URL, bodyReader)
	} else {
		req, err = http.NewRequest(method, a.URL, nil)
	}
	if err != nil {
		return fmt.Errorf("failed to create request: %w", err)
	}

	for k, v := range a.Headers {
		req.Header.Set(k, v)
	}

	client := &http.Client{}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("request failed: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("request returned status %d", resp.StatusCode)
	}

	return nil
}
