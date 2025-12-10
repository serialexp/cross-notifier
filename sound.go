// ABOUTME: Sound matching and playback for notification sounds.
// ABOUTME: Supports embedded wav files and generated tones via beep library.

package main

import (
	"bytes"
	_ "embed"
	"log"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/gopxl/beep/v2"
	"github.com/gopxl/beep/v2/generators"
	"github.com/gopxl/beep/v2/speaker"
	"github.com/gopxl/beep/v2/wav"
)

//go:embed sounds/mixkit-bell-notification-933.wav
var soundBell []byte

//go:embed sounds/mixkit-confirmation-tone-2867.wav
var soundConfirmation []byte

//go:embed sounds/mixkit-correct-answer-tone-2870.wav
var soundCorrect []byte

//go:embed sounds/mixkit-digital-quick-tone-2866.wav
var soundDigital []byte

//go:embed sounds/mixkit-happy-bells-notification-937.wav
var soundHappyBells []byte

//go:embed sounds/mixkit-arabian-mystery-harp-notification-2489.wav
var soundHarp []byte

//go:embed sounds/mixkit-long-pop-2358.wav
var soundPop []byte

//go:embed sounds/mixkit-positive-notification-951.wav
var soundPositive []byte

//go:embed sounds/mixkit-software-interface-start-2574.wav
var soundInterface []byte

// builtinWavSounds maps friendly sound names to embedded wav data.
var builtinWavSounds = map[string][]byte{
	"Bell":         soundBell,
	"Confirmation": soundConfirmation,
	"Correct":      soundCorrect,
	"Digital":      soundDigital,
	"Happy Bells":  soundHappyBells,
	"Harp":         soundHarp,
	"Pop":          soundPop,
	"Positive":     soundPositive,
	"Interface":    soundInterface,
}

// Tone defines a generated sound with frequency and duration.
type Tone struct {
	Frequency float64
	Duration  time.Duration
}

// generatedTones maps sound names to tone parameters (prefixed with "tone:" when used).
var generatedTones = map[string]Tone{
	"ping":   {Frequency: 880, Duration: 100 * time.Millisecond},
	"alert":  {Frequency: 440, Duration: 200 * time.Millisecond},
	"low":    {Frequency: 220, Duration: 200 * time.Millisecond},
	"chime":  {Frequency: 659, Duration: 150 * time.Millisecond}, // E5
	"beep":   {Frequency: 523, Duration: 100 * time.Millisecond}, // C5
	"notify": {Frequency: 587, Duration: 120 * time.Millisecond}, // D5
}

// BuiltinSounds lists available sound names (wav sounds first, then generated tones).
var BuiltinSounds = []string{
	"Bell", "Confirmation", "Correct", "Digital", "Happy Bells",
	"Harp", "Pop", "Positive", "Interface",
	"tone:ping", "tone:alert", "tone:low", "tone:chime", "tone:beep", "tone:notify",
}

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
// name can be a wav sound name, or a generated tone prefixed with "tone:".
func PlaySound(name string) {
	if name == "" || name == "none" {
		return
	}

	if err := initSpeaker(); err != nil {
		log.Printf("Failed to initialize speaker: %v", err)
		return
	}

	// Check for generated tone (prefixed with "tone:")
	if strings.HasPrefix(name, "tone:") {
		toneName := strings.TrimPrefix(name, "tone:")
		playGeneratedTone(toneName)
		return
	}

	// Check for wav sound
	if wavData, ok := builtinWavSounds[name]; ok {
		playWavSound(wavData)
		return
	}

	log.Printf("Unknown sound: %s", name)
}

// playWavSound plays embedded wav data.
func playWavSound(data []byte) {
	reader := bytes.NewReader(data)
	streamer, format, err := wav.Decode(reader)
	if err != nil {
		log.Printf("Failed to decode wav: %v", err)
		return
	}

	// Resample if needed to match speaker sample rate
	if format.SampleRate != sampleRate {
		resampled := beep.Resample(4, format.SampleRate, sampleRate, streamer)
		speaker.Play(beep.Seq(resampled, beep.Callback(func() {
			streamer.Close()
		})))
	} else {
		speaker.Play(beep.Seq(streamer, beep.Callback(func() {
			streamer.Close()
		})))
	}
}

// playGeneratedTone plays a generated sine wave tone.
func playGeneratedTone(name string) {
	tone, ok := generatedTones[name]
	if !ok {
		log.Printf("Unknown generated tone: %s", name)
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
	// Check wav sounds
	if _, ok := builtinWavSounds[name]; ok {
		return true
	}
	// Check generated tones (with "tone:" prefix)
	if strings.HasPrefix(name, "tone:") {
		toneName := strings.TrimPrefix(name, "tone:")
		_, ok := generatedTones[toneName]
		return ok
	}
	return false
}
