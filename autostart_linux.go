// ABOUTME: Linux auto-start management using systemd user services.
// ABOUTME: Provides install/uninstall functions for the systemd user service.

//go:build linux

package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"text/template"
)

const (
	serviceName = "cross-notifier.service"
)

// serviceDir returns the path to the systemd user service directory.
func serviceDir() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, ".config", "systemd", "user")
}

// servicePath returns the path to the systemd service file.
func servicePath() string {
	dir := serviceDir()
	if dir == "" {
		return ""
	}
	return filepath.Join(dir, serviceName)
}

// serviceTemplate is the systemd user service template.
var serviceTemplate = template.Must(template.New("service").Parse(`[Unit]
Description=CrossNotifier - Desktop notification daemon
After=graphical-session.target

[Service]
Type=simple
ExecStart={{.ExecutablePath}}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
`))

type serviceData struct {
	ExecutablePath string
}

// InstallAutostart creates and enables the systemd user service.
func InstallAutostart() error {
	svcPath := servicePath()
	if svcPath == "" {
		return fmt.Errorf("could not determine service path")
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

	// Ensure service directory exists
	svcDir := serviceDir()
	if err := os.MkdirAll(svcDir, 0755); err != nil {
		return fmt.Errorf("could not create service directory: %w", err)
	}

	// Stop existing service if running (ignore errors)
	_ = exec.Command("systemctl", "--user", "stop", serviceName).Run()

	// Create service file
	f, err := os.Create(svcPath)
	if err != nil {
		return fmt.Errorf("could not create service file: %w", err)
	}
	defer f.Close()

	data := serviceData{
		ExecutablePath: execPath,
	}

	if err := serviceTemplate.Execute(f, data); err != nil {
		return fmt.Errorf("could not write service file: %w", err)
	}

	// Reload systemd
	if err := exec.Command("systemctl", "--user", "daemon-reload").Run(); err != nil {
		return fmt.Errorf("could not reload systemd: %w", err)
	}

	// Enable the service
	if err := exec.Command("systemctl", "--user", "enable", serviceName).Run(); err != nil {
		return fmt.Errorf("could not enable service: %w", err)
	}

	// Start the service
	if err := exec.Command("systemctl", "--user", "start", serviceName).Run(); err != nil {
		return fmt.Errorf("could not start service: %w", err)
	}

	return nil
}

// UninstallAutostart stops, disables, and removes the systemd user service.
func UninstallAutostart() error {
	svcPath := servicePath()
	if svcPath == "" {
		return fmt.Errorf("could not determine service path")
	}

	// Stop the service (ignore errors if not running)
	_ = exec.Command("systemctl", "--user", "stop", serviceName).Run()

	// Disable the service (ignore errors if not enabled)
	_ = exec.Command("systemctl", "--user", "disable", serviceName).Run()

	// Remove the service file
	if err := os.Remove(svcPath); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("could not remove service file: %w", err)
	}

	// Reload systemd
	_ = exec.Command("systemctl", "--user", "daemon-reload").Run()

	return nil
}

// IsAutostartInstalled returns true if auto-start is enabled.
func IsAutostartInstalled() bool {
	svcPath := servicePath()
	if svcPath == "" {
		return false
	}
	_, err := os.Stat(svcPath)
	return err == nil
}
