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

// RuleAction defines how a notification should be handled.
type RuleAction string

const (
	RuleActionNormal  RuleAction = "normal"  // Show popup + sound + add to center
	RuleActionSilent  RuleAction = "silent"  // Add to center only (no popup, no sound)
	RuleActionDismiss RuleAction = "dismiss" // Don't show at all
)

// NotificationRule defines conditions and actions for matching notifications.
type NotificationRule struct {
	// Filters (all must match, empty = any)
	Server  string `json:"server,omitempty"`  // server label filter
	Source  string `json:"source,omitempty"`  // notification source filter
	Status  string `json:"status,omitempty"`  // status filter: info/success/warning/error
	Pattern string `json:"pattern,omitempty"` // regex on title+message

	// Actions
	Sound  string     `json:"sound,omitempty"`  // sound to play (empty = no sound)
	Action RuleAction `json:"action,omitempty"` // normal, silent, or dismiss (default: normal)

	// Deprecated: use Action = "dismiss" instead
	Suppress bool `json:"suppress,omitempty"`
}

// EffectiveAction returns the action to take, handling backward compatibility.
func (r *NotificationRule) EffectiveAction() RuleAction {
	// Backward compatibility: Suppress = true maps to dismiss
	if r.Suppress {
		return RuleActionDismiss
	}
	if r.Action == "" {
		return RuleActionNormal
	}
	return r.Action
}

// RulesConfig holds notification rule settings.
type RulesConfig struct {
	Enabled bool               `json:"enabled"` // master toggle
	Rules   []NotificationRule `json:"rules"`   // evaluated in order, first match wins
}

// Config holds the persistent configuration for the daemon.
type Config struct {
	Name    string      `json:"name,omitempty"` // client display name for identification
	Servers []Server    `json:"servers,omitempty"`
	Rules   RulesConfig `json:"rules,omitempty"`
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
