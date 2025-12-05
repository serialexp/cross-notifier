// ABOUTME: Sound playback and notification sound matching logic.
// ABOUTME: Plays sounds via macOS NSSound and matches notifications to sound rules.

package main

/*
#cgo CFLAGS: -x objective-c
#cgo LDFLAGS: -framework AppKit
#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>

// Cache for preloaded sounds
static NSMutableDictionary *soundCache = nil;

void initSoundCache() {
    if (soundCache == nil) {
        soundCache = [[NSMutableDictionary alloc] init];
    }
}

void preloadSound(const char* path) {
    @autoreleasepool {
        initSoundCache();
        NSString *nsPath = [NSString stringWithUTF8String:path];
        if (soundCache[nsPath] == nil) {
            NSSound *sound = [[NSSound alloc] initWithContentsOfFile:nsPath byReference:NO];
            if (sound != nil) {
                soundCache[nsPath] = sound;
            }
        }
    }
}

void playSound(const char* path) {
    @autoreleasepool {
        initSoundCache();
        NSString *nsPath = [NSString stringWithUTF8String:path];
        NSSound *sound = soundCache[nsPath];
        if (sound == nil) {
            sound = [[NSSound alloc] initWithContentsOfFile:nsPath byReference:NO];
            if (sound != nil) {
                soundCache[nsPath] = sound;
            }
        }
        if (sound != nil) {
            [sound stop];  // Stop if already playing
            [sound play];
        }
    }
}
*/
import "C"

import (
	"path/filepath"
	"regexp"
	"strings"
)

func init() {
	// Preload built-in sounds for instant playback
	for _, name := range BuiltinSounds {
		path := resolveSoundPath(name)
		C.preloadSound(C.CString(path))
	}
}

// BuiltinSounds lists available macOS system sounds.
var BuiltinSounds = []string{
	"Basso", "Blow", "Bottle", "Frog", "Funk", "Glass",
	"Hero", "Morse", "Ping", "Pop", "Purr", "Sosumi", "Submarine", "Tink",
}

// MatchSound finds the first matching sound rule for a notification.
// Returns the sound to play, or empty string if no match or disabled.
func MatchSound(n Notification, cfg SoundConfig) string {
	if !cfg.Enabled {
		return ""
	}

	for _, rule := range cfg.Rules {
		if matchesRule(n, rule) {
			return rule.Sound
		}
	}

	return ""
}

// matchesRule checks if a notification matches all conditions of a rule.
func matchesRule(n Notification, rule SoundRule) bool {
	// Server filter
	if rule.Server != "" && rule.Server != n.ServerLabel {
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
// name can be a built-in sound name or an absolute path to a sound file.
// Returns immediately; sound plays in background.
func PlaySound(name string) {
	if name == "" || name == "none" {
		return
	}

	path := resolveSoundPath(name)
	if path == "" {
		return
	}

	// Play via NSSound (fast, no process spawn)
	C.playSound(C.CString(path))
}

// resolveSoundPath converts a sound name to a file path.
func resolveSoundPath(name string) string {
	// Check if it's already an absolute path
	if strings.HasPrefix(name, "/") {
		return name
	}

	// Try as a built-in system sound
	systemPath := filepath.Join("/System/Library/Sounds", name+".aiff")
	return systemPath
}

// IsBuiltinSound returns true if the name is a known built-in sound.
func IsBuiltinSound(name string) bool {
	for _, s := range BuiltinSounds {
		if s == name {
			return true
		}
	}
	return false
}
