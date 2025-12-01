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
		ServerURL: "ws://example.com:9876/ws",
		Secret:    "my-secret-key",
	}

	if err := cfg.Save(path); err != nil {
		t.Fatalf("Save failed: %v", err)
	}

	loaded, err := LoadConfig(path)
	if err != nil {
		t.Fatalf("Load failed: %v", err)
	}

	if loaded.ServerURL != cfg.ServerURL {
		t.Errorf("ServerURL mismatch: got %q, want %q", loaded.ServerURL, cfg.ServerURL)
	}
	if loaded.Secret != cfg.Secret {
		t.Errorf("Secret mismatch: got %q, want %q", loaded.Secret, cfg.Secret)
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
		ServerURL: "ws://example.com/ws",
		Secret:    "secret",
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

	if loaded.ServerURL != "" || loaded.Secret != "" {
		t.Errorf("expected empty config, got %+v", loaded)
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
