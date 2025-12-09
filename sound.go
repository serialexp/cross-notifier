// ABOUTME: Sound matching and playback for notification sounds.
// ABOUTME: Uses beep library for cross-platform tone generation.

package main

import (
	"log"
	"regexp"
	"sync"
	"time"

	"github.com/gopxl/beep/v2"
	"github.com/gopxl/beep/v2/generators"
	"github.com/gopxl/beep/v2/speaker"
)

// Tone defines a generated sound with frequency and duration.
type Tone struct {
	Frequency float64
	Duration  time.Duration
}

// builtinTones maps sound names to tone parameters.
var builtinTones = map[string]Tone{
	"ping":   {Frequency: 880, Duration: 100 * time.Millisecond},
	"alert":  {Frequency: 440, Duration: 200 * time.Millisecond},
	"low":    {Frequency: 220, Duration: 200 * time.Millisecond},
	"chime":  {Frequency: 659, Duration: 150 * time.Millisecond}, // E5
	"beep":   {Frequency: 523, Duration: 100 * time.Millisecond}, // C5
	"notify": {Frequency: 587, Duration: 120 * time.Millisecond}, // D5
}

// BuiltinSounds lists available sound names.
var BuiltinSounds = []string{"ping", "alert", "low", "chime", "beep", "notify"}

const sampleRate = beep.SampleRate(44100)

var (
	speakerOnce sync.Once
	speakerErr  error
)

// initSpeaker initializes the audio speaker (called once).
func initSpeaker() error {
	speakerOnce.Do(func() {
		speakerErr = speaker.Init(sampleRate, sampleRate.N(50*time.Millisecond))
	})
	return speakerErr
}

// MatchRule finds the first matching rule for a notification.
// Returns nil if rules are disabled or no rule matches.
func MatchRule(n Notification, cfg RulesConfig) *NotificationRule {
	if !cfg.Enabled {
		return nil
	}

	for i := range cfg.Rules {
		if matchesRule(n, cfg.Rules[i]) {
			return &cfg.Rules[i]
		}
	}

	return nil
}

// MatchSound finds the first matching sound rule for a notification.
// Returns the sound to play, or empty string if no match or disabled.
func MatchSound(n Notification, cfg RulesConfig) string {
	rule := MatchRule(n, cfg)
	if rule == nil {
		return ""
	}
	return rule.Sound
}

// matchesRule checks if a notification matches all conditions of a rule.
func matchesRule(n Notification, rule NotificationRule) bool {
	// Server filter
	if rule.Server != "" && rule.Server != n.ServerLabel {
		return false
	}

	// Source filter
	if rule.Source != "" && rule.Source != n.Source {
		return false
	}

	// Status filter
	if rule.Status != "" && rule.Status != n.Status {
		return false
	}

	// Pattern filter (regex on title + message)
	if rule.Pattern != "" {
		re, err := regexp.Compile(rule.Pattern)
		if err != nil {
			// Invalid regex, skip this rule
			return false
		}
		combined := n.Title + " " + n.Message
		if !re.MatchString(combined) {
			return false
		}
	}

	return true
}

// PlaySound plays the specified sound asynchronously.
// name should be one of the built-in sound names.
func PlaySound(name string) {
	if name == "" || name == "none" {
		return
	}

	tone, ok := builtinTones[name]
	if !ok {
		log.Printf("Unknown sound: %s", name)
		return
	}

	if err := initSpeaker(); err != nil {
		log.Printf("Failed to initialize speaker: %v", err)
		return
	}

	streamer, err := generators.SineTone(sampleRate, tone.Frequency)
	if err != nil {
		log.Printf("Failed to generate tone: %v", err)
		return
	}

	// Limit to the specified duration
	limited := beep.Take(sampleRate.N(tone.Duration), streamer)

	speaker.Play(limited)
}

// IsBuiltinSound returns true if the name is a known built-in sound.
func IsBuiltinSound(name string) bool {
	_, ok := builtinTones[name]
	return ok
}
