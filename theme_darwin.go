// ABOUTME: macOS theme detection.
// ABOUTME: Checks AppleInterfaceStyle to determine dark/light mode.

//go:build darwin

package main

import (
	"os/exec"
	"strings"
)

func isDarkMode() bool {
	cmd := exec.Command("defaults", "read", "-g", "AppleInterfaceStyle")
	output, err := cmd.Output()
	if err != nil {
		// Property doesn't exist = light mode
		return false
	}
	return strings.TrimSpace(string(output)) == "Dark"
}
