// ABOUTME: Tests for notification sound matching logic.
// ABOUTME: Verifies rule matching with server, status, and pattern filters.

package main

import (
	"testing"
)

func TestMatchSoundNoRules(t *testing.T) {
	cfg := SoundConfig{Enabled: true, Rules: nil}
	n := Notification{Title: "Test", Message: "Hello"}

	sound := MatchSound(n, cfg)
	if sound != "" {
		t.Errorf("expected empty sound for no rules, got %q", sound)
	}
}

func TestMatchSoundDisabled(t *testing.T) {
	cfg := SoundConfig{
		Enabled: false,
		Rules:   []SoundRule{{Sound: "Ping"}},
	}
	n := Notification{Title: "Test", Message: "Hello"}

	sound := MatchSound(n, cfg)
	if sound != "" {
		t.Errorf("expected empty sound when disabled, got %q", sound)
	}
}

func TestMatchSoundWildcardRule(t *testing.T) {
	cfg := SoundConfig{
		Enabled: true,
		Rules:   []SoundRule{{Sound: "Ping"}},
	}
	n := Notification{Title: "Test", Message: "Hello"}

	sound := MatchSound(n, cfg)
	if sound != "Ping" {
		t.Errorf("expected Ping, got %q", sound)
	}
}

func TestMatchSoundServerFilter(t *testing.T) {
	cfg := SoundConfig{
		Enabled: true,
		Rules: []SoundRule{
			{Server: "Work", Sound: "Basso"},
			{Sound: "Ping"}, // fallback
		},
	}

	tests := []struct {
		serverLabel string
		want        string
	}{
		{"Work", "Basso"},
		{"Home", "Ping"},
		{"", "Ping"},
	}

	for _, tt := range tests {
		n := Notification{Title: "Test", ServerLabel: tt.serverLabel}
		sound := MatchSound(n, cfg)
		if sound != tt.want {
			t.Errorf("serverLabel=%q: got %q, want %q", tt.serverLabel, sound, tt.want)
		}
	}
}

func TestMatchSoundStatusFilter(t *testing.T) {
	cfg := SoundConfig{
		Enabled: true,
		Rules: []SoundRule{
			{Status: "error", Sound: "Basso"},
			{Status: "warning", Sound: "Funk"},
			{Sound: "Ping"},
		},
	}

	tests := []struct {
		status string
		want   string
	}{
		{"error", "Basso"},
		{"warning", "Funk"},
		{"info", "Ping"},
		{"success", "Ping"},
		{"", "Ping"},
	}

	for _, tt := range tests {
		n := Notification{Title: "Test", Status: tt.status}
		sound := MatchSound(n, cfg)
		if sound != tt.want {
			t.Errorf("status=%q: got %q, want %q", tt.status, sound, tt.want)
		}
	}
}

func TestMatchSoundPatternFilter(t *testing.T) {
	cfg := SoundConfig{
		Enabled: true,
		Rules: []SoundRule{
			{Pattern: "(?i)urgent", Sound: "Basso"},
			{Pattern: "(?i)deploy", Sound: "Hero"},
			{Sound: "Ping"},
		},
	}

	tests := []struct {
		title   string
		message string
		want    string
	}{
		{"URGENT: Server down", "", "Basso"},
		{"Info", "This is urgent!", "Basso"},
		{"Deploy complete", "v1.2.3", "Hero"},
		{"Info", "deployment finished", "Hero"},
		{"Hello", "World", "Ping"},
	}

	for _, tt := range tests {
		n := Notification{Title: tt.title, Message: tt.message}
		sound := MatchSound(n, cfg)
		if sound != tt.want {
			t.Errorf("title=%q message=%q: got %q, want %q", tt.title, tt.message, sound, tt.want)
		}
	}
}

func TestMatchSoundCombinedFilters(t *testing.T) {
	cfg := SoundConfig{
		Enabled: true,
		Rules: []SoundRule{
			{Server: "Work", Status: "error", Sound: "Basso"},
			{Server: "Work", Sound: "Funk"},
			{Status: "error", Sound: "Glass"},
			{Sound: "Ping"},
		},
	}

	tests := []struct {
		serverLabel string
		status      string
		want        string
	}{
		{"Work", "error", "Basso"}, // matches first rule
		{"Work", "info", "Funk"},   // matches second rule
		{"Home", "error", "Glass"}, // matches third rule
		{"Home", "info", "Ping"},   // matches fallback
		{"", "error", "Glass"},     // matches third rule
		{"Work", "", "Funk"},       // matches second rule (status empty)
	}

	for _, tt := range tests {
		n := Notification{Title: "Test", ServerLabel: tt.serverLabel, Status: tt.status}
		sound := MatchSound(n, cfg)
		if sound != tt.want {
			t.Errorf("server=%q status=%q: got %q, want %q", tt.serverLabel, tt.status, sound, tt.want)
		}
	}
}

func TestMatchSoundNoneValue(t *testing.T) {
	cfg := SoundConfig{
		Enabled: true,
		Rules: []SoundRule{
			{Status: "info", Sound: "none"},
			{Sound: "Ping"},
		},
	}

	n := Notification{Title: "Test", Status: "info"}
	sound := MatchSound(n, cfg)
	if sound != "none" {
		t.Errorf("expected 'none' for silent rule, got %q", sound)
	}
}

func TestMatchSoundFirstMatchWins(t *testing.T) {
	cfg := SoundConfig{
		Enabled: true,
		Rules: []SoundRule{
			{Sound: "First"},
			{Sound: "Second"},
			{Sound: "Third"},
		},
	}

	n := Notification{Title: "Test"}
	sound := MatchSound(n, cfg)
	if sound != "First" {
		t.Errorf("expected first rule to win, got %q", sound)
	}
}

func TestMatchSoundInvalidRegexSkipsRule(t *testing.T) {
	cfg := SoundConfig{
		Enabled: true,
		Rules: []SoundRule{
			{Pattern: "[invalid", Sound: "Bad"},
			{Sound: "Good"},
		},
	}

	n := Notification{Title: "Test"}
	sound := MatchSound(n, cfg)
	if sound != "Good" {
		t.Errorf("expected invalid regex rule to be skipped, got %q", sound)
	}
}
