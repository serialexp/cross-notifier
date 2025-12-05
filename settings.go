// ABOUTME: Settings window for configuring the notification daemon.
// ABOUTME: Allows users to configure client name and multiple notification servers.

package main

import (
	_ "embed"
	"fmt"
	"image/color"

	g "github.com/AllenDang/giu"
	"github.com/sqweek/dialog"
)

//go:embed Hack-Regular.ttf
var hackFont []byte

// SettingsResult holds the outcome of the settings window.
type SettingsResult struct {
	Config    *Config
	Cancelled bool
}

// settingsState holds the editable state for the settings window.
type settingsState struct {
	name         string
	servers      []serverEntry
	soundEnabled bool
	soundRules   []soundRuleEntry
}

// serverEntry holds editable fields for a single server.
type serverEntry struct {
	url       string
	secret    string
	label     string
	connected bool // connection status
}

// soundRuleEntry holds editable fields for a single sound rule.
type soundRuleEntry struct {
	serverIdx  int32  // 0 = Any, 1+ = server index + 1
	statusIdx  int32  // 0 = Any, 1+ = status index + 1
	pattern    string // regex pattern
	soundIdx   int32  // 0 = None, 1-14 = built-in, 15 = Custom
	customPath string // path when Custom is selected
}

// Status options for sound rules
var statusOptions = []string{"Any Status", "info", "success", "warning", "error"}

// Sound options (No Sound + built-in + Custom)
var soundOptions = append(append([]string{"No Sound"}, BuiltinSounds...), "Custom...")

// ShowSettingsWindow displays a configuration window and blocks until the user
// saves or cancels. Returns the configuration values entered.
// The isConnected function is called with server URL to check connection status.
func ShowSettingsWindow(initial *Config, isConnected func(url string) bool) SettingsResult {
	var result SettingsResult
	done := false

	// Initialize state from config
	state := &settingsState{}
	if initial != nil {
		state.name = initial.Name
		for _, s := range initial.Servers {
			connected := false
			if isConnected != nil {
				connected = isConnected(s.URL)
			}
			state.servers = append(state.servers, serverEntry{
				url:       s.URL,
				secret:    s.Secret,
				label:     s.Label,
				connected: connected,
			})
		}
		// Initialize sound config
		state.soundEnabled = initial.Sound.Enabled
		for _, rule := range initial.Sound.Rules {
			state.soundRules = append(state.soundRules, soundRuleToEntry(rule, initial.Servers))
		}
	}

	// Calculate window height based on number of servers and sound rules
	baseHeight := 300 // increased for sound section
	serverRowHeight := 35
	soundRuleHeight := 75 // taller cards with two rows
	windowHeight := baseHeight + len(state.servers)*serverRowHeight + len(state.soundRules)*soundRuleHeight
	if windowHeight < 350 {
		windowHeight = 350
	}
	if windowHeight > 800 {
		windowHeight = 800
	}

	wnd := g.NewMasterWindow("Cross-Notifier Settings", 700, windowHeight, 0)
	wnd.SetBgColor(color.RGBA{R: 30, G: 30, B: 35, A: 255})

	// Load custom font with better Unicode support
	g.Context.FontAtlas.SetDefaultFontFromBytes(hackFont, 14)

	wnd.Run(func() {
		// Force redraw on mouse release to fix combo box display lag
		if g.IsMouseReleased(g.MouseButtonLeft) {
			g.Update()
		}

		g.SingleWindow().Layout(
			g.Label("Configure notification client:"),
			g.Spacing(),

			g.Row(
				g.Label("Your Name:"),
				g.InputText(&state.name).Size(350).Hint("e.g. Bart"),
			),
			g.Spacing(),
			g.Separator(),
			g.Spacing(),

			g.Label("Notification Servers:"),
			g.Spacing(),

			// Server list
			g.Custom(func() {
				toDelete := -1
				for i := range state.servers {
					renderServerRow(state, i, &toDelete)
				}
				if toDelete >= 0 {
					state.servers = append(state.servers[:toDelete], state.servers[toDelete+1:]...)
				}
			}),

			// Add server button
			g.Row(
				g.Button("+ Add Server").Size(100, 25).OnClick(func() {
					state.servers = append(state.servers, serverEntry{})
				}),
			),
			g.Spacing(),
			g.Separator(),
			g.Spacing(),

			// Sound configuration section
			g.Label("Notification Sounds:"),
			g.Spacing(),
			g.Checkbox("Enable notification sounds", &state.soundEnabled),
			g.Spacing(),

			// Sound rules (only show if enabled)
			g.Custom(func() {
				if !state.soundEnabled {
					return
				}
				g.Label("Sound Rules (first match wins):").Build()
				g.Spacing().Build()

				toDelete := -1
				toMoveUp := -1
				toMoveDown := -1
				for i := range state.soundRules {
					renderSoundRuleRow(state, i, &toDelete, &toMoveUp, &toMoveDown)
				}
				if toDelete >= 0 {
					state.soundRules = append(state.soundRules[:toDelete], state.soundRules[toDelete+1:]...)
				}
				if toMoveUp > 0 {
					state.soundRules[toMoveUp], state.soundRules[toMoveUp-1] = state.soundRules[toMoveUp-1], state.soundRules[toMoveUp]
				}
				if toMoveDown >= 0 && toMoveDown < len(state.soundRules)-1 {
					state.soundRules[toMoveDown], state.soundRules[toMoveDown+1] = state.soundRules[toMoveDown+1], state.soundRules[toMoveDown]
				}

				g.Row(
					g.Button("+ Add Rule").Size(100, 25).OnClick(func() {
						state.soundRules = append(state.soundRules, soundRuleEntry{soundIdx: 1}) // default to first built-in sound
					}),
				).Build()
			}),
			g.Spacing(),
			g.Separator(),
			g.Spacing(),

			// Action buttons
			g.Row(
				g.Button("Save").Size(100, 30).OnClick(func() {
					result.Config = stateToConfig(state)
					result.Cancelled = false
					done = true
				}),
				g.Button("Local Only").Size(100, 30).OnClick(func() {
					result.Config = &Config{Name: state.name}
					result.Cancelled = false
					done = true
				}),
				g.Button("Cancel").Size(100, 30).OnClick(func() {
					result.Cancelled = true
					done = true
				}),
			),
		)

		if done {
			wnd.SetShouldClose(true)
		}
	})

	return result
}

// renderServerRow renders a single server entry row.
func renderServerRow(state *settingsState, index int, toDelete *int) {
	server := &state.servers[index]
	idx := index // capture for closure

	// Show connection status indicator
	statusIndicator := "○"                                // empty circle for disconnected
	statusColor := color.RGBA{R: 255, G: 0, B: 0, A: 255} // red
	if server.connected {
		statusIndicator = "●"                                // filled circle for connected
		statusColor = color.RGBA{R: 0, G: 255, B: 0, A: 255} // green
	}

	g.Row(
		g.Style().SetColor(g.StyleColorText, statusColor).To(
			g.Label(statusIndicator),
		),
		g.Label("Label:"),
		g.InputText(&server.label).Size(100).Hint("Work"),
		g.Label("URL:"),
		g.InputText(&server.url).Size(200).Hint("ws://host:9876/ws"),
		g.Label("Secret:"),
		g.InputText(&server.secret).Size(120).Flags(g.InputTextFlagsPassword),
		g.Buttonf("X##delete%d", index).Size(25, 20).OnClick(func() {
			*toDelete = idx
		}),
	).Build()
}

// stateToConfig converts the settings state to a Config.
func stateToConfig(state *settingsState) *Config {
	cfg := &Config{
		Name: state.name,
	}
	for _, s := range state.servers {
		if s.url != "" {
			cfg.Servers = append(cfg.Servers, Server{
				URL:    s.url,
				Secret: s.secret,
				Label:  s.label,
			})
		}
	}
	// Convert sound config
	cfg.Sound.Enabled = state.soundEnabled
	for _, entry := range state.soundRules {
		rule := entryToSoundRule(entry, state.servers)
		cfg.Sound.Rules = append(cfg.Sound.Rules, rule)
	}
	return cfg
}

// soundRuleToEntry converts a SoundRule to editable entry state.
func soundRuleToEntry(rule SoundRule, servers []Server) soundRuleEntry {
	entry := soundRuleEntry{
		pattern: rule.Pattern,
	}

	// Find server index (0 = Any)
	if rule.Server == "" {
		entry.serverIdx = 0
	} else {
		for i, s := range servers {
			label := s.Label
			if label == "" {
				label = s.URL
			}
			if label == rule.Server {
				entry.serverIdx = int32(i + 1)
				break
			}
		}
	}

	// Find status index (0 = Any)
	if rule.Status == "" {
		entry.statusIdx = 0
	} else {
		for i, s := range statusOptions[1:] { // skip "Any"
			if s == rule.Status {
				entry.statusIdx = int32(i + 1)
				break
			}
		}
	}

	// Find sound index
	if rule.Sound == "none" || rule.Sound == "" {
		entry.soundIdx = 0
	} else if IsBuiltinSound(rule.Sound) {
		for i, s := range BuiltinSounds {
			if s == rule.Sound {
				entry.soundIdx = int32(i + 1) // +1 because 0 is "None"
				break
			}
		}
	} else {
		// Custom sound
		entry.soundIdx = int32(len(soundOptions) - 1) // "Custom..."
		entry.customPath = rule.Sound
	}

	return entry
}

// entryToSoundRule converts an editable entry back to a SoundRule.
func entryToSoundRule(entry soundRuleEntry, servers []serverEntry) SoundRule {
	rule := SoundRule{
		Pattern: entry.pattern,
	}

	// Server
	if entry.serverIdx > 0 && int(entry.serverIdx-1) < len(servers) {
		server := servers[entry.serverIdx-1]
		if server.label != "" {
			rule.Server = server.label
		} else {
			rule.Server = server.url
		}
	}

	// Status
	if entry.statusIdx > 0 && int(entry.statusIdx) < len(statusOptions) {
		rule.Status = statusOptions[entry.statusIdx]
	}

	// Sound
	if entry.soundIdx == 0 {
		rule.Sound = "none"
	} else if int(entry.soundIdx) < len(soundOptions)-1 {
		rule.Sound = soundOptions[entry.soundIdx]
	} else {
		// Custom
		rule.Sound = entry.customPath
	}

	return rule
}

// renderSoundRuleRow renders a single sound rule entry as a card with two rows.
func renderSoundRuleRow(state *settingsState, index int, toDelete, toMoveUp, toMoveDown *int) {
	rule := &state.soundRules[index]
	idx := index // capture for closure

	// Build server options dynamically
	serverOpts := []string{"Any Server"}
	for _, s := range state.servers {
		label := s.label
		if label == "" {
			label = s.url
		}
		if label != "" {
			serverOpts = append(serverOpts, label)
		}
	}

	// Bounds check indices
	if rule.serverIdx < 0 || int(rule.serverIdx) >= len(serverOpts) {
		rule.serverIdx = 0
	}
	if rule.statusIdx < 0 || int(rule.statusIdx) >= len(statusOptions) {
		rule.statusIdx = 0
	}
	if rule.soundIdx < 0 || int(rule.soundIdx) >= len(soundOptions) {
		rule.soundIdx = 0
	}

	// Determine sound to play for preview
	var soundToPlay string
	if rule.soundIdx == 0 {
		soundToPlay = ""
	} else if int(rule.soundIdx) < len(soundOptions)-1 {
		soundToPlay = soundOptions[rule.soundIdx]
	} else {
		soundToPlay = rule.customPath
	}

	// Card background with rounded corners
	cardBg := color.RGBA{R: 45, G: 45, B: 50, A: 255}
	g.Style().SetColor(g.StyleColorChildBg, cardBg).SetStyleFloat(g.StyleVarChildRounding, 8).To(
		g.Child().Size(660, 60).Layout(
			// Row 1: Conditions (with label)
			g.Row(
				g.Label("If:"),
				g.Combo(fmt.Sprintf("##server%d", index), serverOpts[rule.serverIdx], serverOpts, &rule.serverIdx).Size(100),
				g.Combo(fmt.Sprintf("##status%d", index), statusOptions[rule.statusIdx], statusOptions, &rule.statusIdx).Size(100),
				g.Label("matches:"),
				g.InputText(&rule.pattern).Size(150).Hint("regex pattern").Label(fmt.Sprintf("##pattern%d", index)),
				// Action buttons
				g.Buttonf("↑##up%d", index).Size(25, 20).OnClick(func() {
					*toMoveUp = idx
				}),
				g.Buttonf("↓##down%d", index).Size(25, 20).OnClick(func() {
					*toMoveDown = idx
				}),
				g.Dummy(75, 0), // spacer to push delete button right
				g.Style().SetColor(g.StyleColorButton, color.RGBA{R: 180, G: 60, B: 60, A: 255}).
					SetColor(g.StyleColorButtonHovered, color.RGBA{R: 220, G: 80, B: 80, A: 255}).
					SetColor(g.StyleColorButtonActive, color.RGBA{R: 150, G: 40, B: 40, A: 255}).To(
					g.Buttonf("✕##delrule%d", index).Size(25, 20).OnClick(func() {
						*toDelete = idx
					}),
				),
			),
			// Row 2: Sound selection
			g.Row(
				g.Label("Play:"),
				g.Combo(fmt.Sprintf("##sound%d", index), soundOptions[rule.soundIdx], soundOptions, &rule.soundIdx).Size(100),
				g.Custom(func() {
					// Show custom path input and browse button if Custom is selected
					if int(rule.soundIdx) == len(soundOptions)-1 {
						g.InputText(&rule.customPath).Size(150).Hint("/path/to/sound").Label(fmt.Sprintf("##custompath%d", idx)).Build()
						g.SameLine()
						g.Button(fmt.Sprintf("...##browse%d", idx)).OnClick(func() {
							path, err := dialog.File().Filter("Sound files", "aiff", "wav", "mp3", "m4a").Load()
							if err == nil && path != "" {
								rule.customPath = path
							}
						}).Build()
						g.SameLine()
					}
				}),
				g.Buttonf("▶##play%d", index).Size(25, 20).OnClick(func() {
					if soundToPlay != "" {
						PlaySound(soundToPlay)
					}
				}),
			),
		),
	).Build()
	g.Spacing().Build()
}
