// ABOUTME: Theme detection stub for non-macOS platforms.
// ABOUTME: Defaults to dark mode.

//go:build !darwin

package main

func isDarkMode() bool {
	// Default to dark mode on non-macOS platforms
	// Could add Windows/Linux detection later
	return true
}

func refreshThemeIfStale() {
	// No-op on non-macOS
}

func forceRefreshTheme() {
	// No-op on non-macOS
}
