// ABOUTME: Configuration file handling for persistent settings.
// ABOUTME: Stores server URL and authentication secret.

package main

import (
	"encoding/json"
	"os"
	"path/filepath"
)

// Server holds connection details for a single notification server.
type Server struct {
	URL    string `json:"url"`
	Secret string `json:"secret"`
	Label  string `json:"label,omitempty"` // optional display name for the server
}

// SoundRule defines conditions for playing a specific sound.
type SoundRule struct {
	Server  string `json:"server,omitempty"`  // server label filter (empty = any)
	Status  string `json:"status,omitempty"`  // status filter: info/success/warning/error (empty = any)
	Pattern string `json:"pattern,omitempty"` // regex on title+message (empty = any)
	Sound   string `json:"sound"`             // sound name, path, or "none"
}

// SoundConfig holds notification sound settings.
type SoundConfig struct {
	Enabled bool        `json:"enabled"` // master toggle
	Rules   []SoundRule `json:"rules"`   // evaluated in order, first match wins
}

// Config holds the persistent configuration for the daemon.
type Config struct {
	Name    string      `json:"name,omitempty"` // client display name for identification
	Servers []Server    `json:"servers,omitempty"`
	Sound   SoundConfig `json:"sound,omitempty"`
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
