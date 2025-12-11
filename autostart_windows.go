// ABOUTME: Windows auto-start management using the registry Run key.
// ABOUTME: Adds/removes entry from HKCU\Software\Microsoft\Windows\CurrentVersion\Run.

//go:build windows

package main

import (
	"fmt"
	"os"
	"path/filepath"

	"golang.org/x/sys/windows/registry"
)

const (
	registryKeyPath = `Software\Microsoft\Windows\CurrentVersion\Run`
	registryValue   = "CrossNotifier"
)

// InstallAutostart adds the executable to the Windows registry Run key.
func InstallAutostart() error {
	execPath, err := os.Executable()
	if err != nil {
		return fmt.Errorf("could not determine executable path: %w", err)
	}

	execPath, err = filepath.EvalSymlinks(execPath)
	if err != nil {
		return fmt.Errorf("could not resolve executable path: %w", err)
	}

	key, _, err := registry.CreateKey(registry.CURRENT_USER, registryKeyPath, registry.SET_VALUE)
	if err != nil {
		return fmt.Errorf("could not open registry key: %w", err)
	}
	defer key.Close()

	if err := key.SetStringValue(registryValue, execPath); err != nil {
		return fmt.Errorf("could not set registry value: %w", err)
	}

	return nil
}

// UninstallAutostart removes the executable from the Windows registry Run key.
func UninstallAutostart() error {
	key, err := registry.OpenKey(registry.CURRENT_USER, registryKeyPath, registry.SET_VALUE)
	if err != nil {
		return fmt.Errorf("could not open registry key: %w", err)
	}
	defer key.Close()

	if err := key.DeleteValue(registryValue); err != nil && err != registry.ErrNotExist {
		return fmt.Errorf("could not delete registry value: %w", err)
	}

	return nil
}

// IsAutostartInstalled returns true if auto-start is enabled.
func IsAutostartInstalled() bool {
	key, err := registry.OpenKey(registry.CURRENT_USER, registryKeyPath, registry.QUERY_VALUE)
	if err != nil {
		return false
	}
	defer key.Close()

	_, _, err = key.GetStringValue(registryValue)
	return err == nil
}
