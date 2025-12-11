// ABOUTME: macOS LaunchAgent management for auto-starting the daemon on login.
// ABOUTME: Provides install/uninstall functions for the LaunchAgent plist.

//go:build darwin

package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"text/template"
)

const (
	launchAgentLabel = "com.crossnotifier.daemon"
)

// launchAgentPath returns the path to the LaunchAgent plist file.
func launchAgentPath() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, "Library", "LaunchAgents", launchAgentLabel+".plist")
}

// plistTemplate is the LaunchAgent plist template.
var plistTemplate = template.Must(template.New("plist").Parse(`<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{{.Label}}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{{.ExecutablePath}}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Interactive</string>
    <key>StandardOutPath</key>
    <string>{{.LogPath}}/cross-notifier.log</string>
    <key>StandardErrorPath</key>
    <string>{{.LogPath}}/cross-notifier.log</string>
</dict>
</plist>
`))

type plistData struct {
	Label          string
	ExecutablePath string
	LogPath        string
}

// InstallAutostart creates and loads the LaunchAgent plist.
func InstallAutostart() error {
	plistPath := launchAgentPath()
	if plistPath == "" {
		return fmt.Errorf("could not determine LaunchAgent path")
	}

	// Determine executable path
	execPath, err := os.Executable()
	if err != nil {
		return fmt.Errorf("could not determine executable path: %w", err)
	}

	// Resolve symlinks to get the real path
	execPath, err = filepath.EvalSymlinks(execPath)
	if err != nil {
		return fmt.Errorf("could not resolve executable path: %w", err)
	}

	// Log path
	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("could not determine home directory: %w", err)
	}
	logPath := filepath.Join(home, "Library", "Logs")

	// Ensure LaunchAgents directory exists
	launchAgentsDir := filepath.Dir(plistPath)
	if err := os.MkdirAll(launchAgentsDir, 0755); err != nil {
		return fmt.Errorf("could not create LaunchAgents directory: %w", err)
	}

	// Unload existing agent if present (ignore errors)
	_ = exec.Command("launchctl", "unload", plistPath).Run()

	// Create plist file
	f, err := os.Create(plistPath)
	if err != nil {
		return fmt.Errorf("could not create plist file: %w", err)
	}
	defer f.Close()

	data := plistData{
		Label:          launchAgentLabel,
		ExecutablePath: execPath,
		LogPath:        logPath,
	}

	if err := plistTemplate.Execute(f, data); err != nil {
		return fmt.Errorf("could not write plist file: %w", err)
	}

	// Load the agent
	cmd := exec.Command("launchctl", "load", plistPath)
	if output, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("could not load LaunchAgent: %w (output: %s)", err, string(output))
	}

	return nil
}

// UninstallAutostart unloads and removes the LaunchAgent plist.
func UninstallAutostart() error {
	plistPath := launchAgentPath()
	if plistPath == "" {
		return fmt.Errorf("could not determine LaunchAgent path")
	}

	// Unload the agent (ignore errors if not loaded)
	_ = exec.Command("launchctl", "unload", plistPath).Run()

	// Remove the plist file
	if err := os.Remove(plistPath); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("could not remove plist file: %w", err)
	}

	return nil
}

// IsAutostartInstalled returns true if auto-start is enabled.
func IsAutostartInstalled() bool {
	plistPath := launchAgentPath()
	if plistPath == "" {
		return false
	}
	_, err := os.Stat(plistPath)
	return err == nil
}
