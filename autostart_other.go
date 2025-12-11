// ABOUTME: Stub for auto-start functions on unsupported platforms (BSD, etc).
// ABOUTME: These functions return errors since auto-start is not yet implemented.

//go:build !darwin && !linux && !windows

package main

import "fmt"

// InstallAutostart is not supported on this platform.
func InstallAutostart() error {
	return fmt.Errorf("auto-start is not yet supported on this platform")
}

// UninstallAutostart is not supported on this platform.
func UninstallAutostart() error {
	return fmt.Errorf("auto-start is not yet supported on this platform")
}

// IsAutostartInstalled always returns false on unsupported platforms.
func IsAutostartInstalled() bool {
	return false
}
