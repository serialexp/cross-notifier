// ABOUTME: Tests for configuration file loading and saving.
// ABOUTME: Covers config persistence and defaults.

package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestConfigSaveAndLoad(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.json")

	cfg := &Config{
		Name: "TestClient",
		Servers: []Server{
			{URL: "ws://example.com:9876/ws", Secret: "my-secret-key", Label: "Test Server"},
		},
	}

	if err := cfg.Save(path); err != nil {
		t.Fatalf("Save failed: %v", err)
	}

	loaded, err := LoadConfig(path)
	if err != nil {
		t.Fatalf("Load failed: %v", err)
	}

	if loaded.Name != cfg.Name {
		t.Errorf("Name mismatch: got %q, want %q", loaded.Name, cfg.Name)
	}
	if len(loaded.Servers) != 1 {
		t.Fatalf("Servers count mismatch: got %d, want 1", len(loaded.Servers))
	}
	if loaded.Servers[0].URL != cfg.Servers[0].URL {
		t.Errorf("Server URL mismatch: got %q, want %q", loaded.Servers[0].URL, cfg.Servers[0].URL)
	}
	if loaded.Servers[0].Secret != cfg.Servers[0].Secret {
		t.Errorf("Server Secret mismatch: got %q, want %q", loaded.Servers[0].Secret, cfg.Servers[0].Secret)
	}
	if loaded.Servers[0].Label != cfg.Servers[0].Label {
		t.Errorf("Server Label mismatch: got %q, want %q", loaded.Servers[0].Label, cfg.Servers[0].Label)
	}
}

func TestConfigLoadNonexistent(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nonexistent.json")

	_, err := LoadConfig(path)
	if err == nil {
		t.Error("expected error for nonexistent file")
	}
	if !os.IsNotExist(err) {
		t.Errorf("expected os.IsNotExist error, got %v", err)
	}
}

func TestConfigSaveCreatesDirectory(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "subdir", "config.json")

	cfg := &Config{
		Servers: []Server{
			{URL: "ws://example.com/ws", Secret: "secret"},
		},
	}

	if err := cfg.Save(path); err != nil {
		t.Fatalf("Save failed: %v", err)
	}

	if _, err := os.Stat(path); err != nil {
		t.Errorf("config file not created: %v", err)
	}
}

func TestConfigEmpty(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.json")

	cfg := &Config{}
	if err := cfg.Save(path); err != nil {
		t.Fatalf("Save failed: %v", err)
	}

	loaded, err := LoadConfig(path)
	if err != nil {
		t.Fatalf("Load failed: %v", err)
	}

	if loaded.Name != "" || len(loaded.Servers) != 0 {
		t.Errorf("expected empty config, got %+v", loaded)
	}
}

func TestConfigMultipleServers(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.json")

	cfg := &Config{
		Name: "MultiClient",
		Servers: []Server{
			{URL: "ws://server1:9876/ws", Secret: "secret1", Label: "Work"},
			{URL: "ws://server2:9876/ws", Secret: "secret2", Label: "Home"},
			{URL: "ws://server3:9876/ws", Secret: "secret3"},
		},
	}

	if err := cfg.Save(path); err != nil {
		t.Fatalf("Save failed: %v", err)
	}

	loaded, err := LoadConfig(path)
	if err != nil {
		t.Fatalf("Load failed: %v", err)
	}

	if len(loaded.Servers) != 3 {
		t.Fatalf("Servers count mismatch: got %d, want 3", len(loaded.Servers))
	}

	// Verify each server
	expected := []struct {
		url, secret, label string
	}{
		{"ws://server1:9876/ws", "secret1", "Work"},
		{"ws://server2:9876/ws", "secret2", "Home"},
		{"ws://server3:9876/ws", "secret3", ""},
	}

	for i, exp := range expected {
		if loaded.Servers[i].URL != exp.url {
			t.Errorf("Server[%d] URL mismatch: got %q, want %q", i, loaded.Servers[i].URL, exp.url)
		}
		if loaded.Servers[i].Secret != exp.secret {
			t.Errorf("Server[%d] Secret mismatch: got %q, want %q", i, loaded.Servers[i].Secret, exp.secret)
		}
		if loaded.Servers[i].Label != exp.label {
			t.Errorf("Server[%d] Label mismatch: got %q, want %q", i, loaded.Servers[i].Label, exp.label)
		}
	}
}

func TestConfigPath(t *testing.T) {
	path := ConfigPath()
	if path == "" {
		t.Error("ConfigPath returned empty string")
	}
	if filepath.Base(path) != "config.json" {
		t.Errorf("expected config.json, got %s", filepath.Base(path))
	}
}
