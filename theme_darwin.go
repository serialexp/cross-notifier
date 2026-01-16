// ABOUTME: macOS theme detection.
// ABOUTME: Checks AppleInterfaceStyle to determine dark/light mode.

//go:build darwin

package main

import (
	"os/exec"
	"strings"
	"sync"
	"time"
)

var (
	cachedDarkMode    bool
	darkModeLastCheck time.Time
	darkModeCacheMu   sync.Mutex
	darkModeStaleAge  = 5 * time.Minute
)

// isDarkMode returns the cached dark mode value.
func isDarkMode() bool {
	darkModeCacheMu.Lock()
	defer darkModeCacheMu.Unlock()
	return cachedDarkMode
}

// refreshThemeIfStale checks the system theme if it hasn't been checked recently.
// Call this on notification arrival or when opening the notification center.
func refreshThemeIfStale() {
	darkModeCacheMu.Lock()
	defer darkModeCacheMu.Unlock()

	if time.Since(darkModeLastCheck) < darkModeStaleAge {
		return
	}

	cachedDarkMode = checkDarkMode()
	darkModeLastCheck = time.Now()
}

// forceRefreshTheme always checks the system theme.
// Call this when opening the notification center.
func forceRefreshTheme() {
	darkModeCacheMu.Lock()
	defer darkModeCacheMu.Unlock()

	cachedDarkMode = checkDarkMode()
	darkModeLastCheck = time.Now()
}

func checkDarkMode() bool {
	cmd := exec.Command("defaults", "read", "-g", "AppleInterfaceStyle")
	output, err := cmd.Output()
	if err != nil {
		// Property doesn't exist = light mode
		return false
	}
	return strings.TrimSpace(string(output)) == "Dark"
}

func init() {
	// Initialize on startup
	cachedDarkMode = checkDarkMode()
	darkModeLastCheck = time.Now()
}
