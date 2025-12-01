// ABOUTME: Configuration file handling for persistent settings.
// ABOUTME: Stores server URL and authentication secret.

package main

import (
	"encoding/json"
	"os"
	"path/filepath"
)

// Config holds the persistent configuration for the daemon.
type Config struct {
	ServerURL string `json:"serverUrl,omitempty"`
	Secret    string `json:"secret,omitempty"`
}

// ConfigPath returns the platform-appropriate path for the config file.
func ConfigPath() string {
	configDir, err := os.UserConfigDir()
	if err != nil {
		configDir = "."
	}
	return filepath.Join(configDir, "cross-notifier", "config.json")
}

// LoadConfig reads the configuration from the given path.
func LoadConfig(path string) (*Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}

	var cfg Config
	if err := json.Unmarshal(data, &cfg); err != nil {
		return nil, err
	}

	return &cfg, nil
}

// Save writes the configuration to the given path, creating directories as needed.
func (c *Config) Save(path string) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}

	data, err := json.MarshalIndent(c, "", "  ")
	if err != nil {
		return err
	}

	return os.WriteFile(path, data, 0600)
}
