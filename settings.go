// ABOUTME: Settings window for configuring the notification daemon.
// ABOUTME: Allows users to configure client name and multiple notification servers.

package main

import (
	_ "embed"
	"fmt"
	"image/color"

	"github.com/AllenDang/cimgui-go/imgui"
	g "github.com/AllenDang/giu"
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
	rulesEnabled bool
	rules        []notificationRuleEntry
}

// serverEntry holds editable fields for a single server.
type serverEntry struct {
	url       string
	secret    string
	label     string
	connected bool // connection status
}

// notificationRuleEntry holds editable fields for a single notification rule.
type notificationRuleEntry struct {
	serverIdx int32  // 0 = Any, 1+ = server index + 1
	source    string // source filter (empty = any)
	statusIdx int32  // 0 = Any, 1+ = status index + 1
	pattern   string // regex pattern
	soundIdx  int32  // 0 = None, 1+ = built-in sounds
	actionIdx int32  // 0 = Normal, 1 = Silent, 2 = Dismiss
}

// Status options for notification rules
var statusOptions = []string{"Any Status", "info", "success", "warning", "error"}

// Sound options (No Sound + built-in sounds)
var soundOptions = append([]string{"No Sound"}, BuiltinSounds...)

// Action options for notification rules
var actionOptions = []string{"Normal", "Silent", "Dismiss"}

// ruleCardWidth is the width of a notification rule card
const ruleCardWidth = 680

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
		// Initialize rules config
		state.rulesEnabled = initial.Rules.Enabled
		for _, rule := range initial.Rules.Rules {
			state.rules = append(state.rules, ruleToEntry(rule, initial.Servers))
		}
	}

	// Calculate window height based on number of servers and rules
	baseHeight := 300
	serverRowHeight := 35
	ruleHeight := 100 // taller cards with more fields
	windowHeight := baseHeight + len(state.servers)*serverRowHeight + len(state.rules)*ruleHeight
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

			// Notification rules section
			g.Label("Notification Rules:"),
			g.Spacing(),
			g.Checkbox("Enable notification rules", &state.rulesEnabled),
			g.Spacing(),

			// Rules (only show if enabled)
			g.Custom(func() {
				if !state.rulesEnabled {
					return
				}
				g.Label("Rules (first match wins):").Build()
				g.Spacing().Build()

				toDelete := -1
				toMoveUp := -1
				toMoveDown := -1
				for i := range state.rules {
					renderRuleRow(state, i, &toDelete, &toMoveUp, &toMoveDown)
				}
				if toDelete >= 0 {
					state.rules = append(state.rules[:toDelete], state.rules[toDelete+1:]...)
				}
				if toMoveUp > 0 {
					state.rules[toMoveUp], state.rules[toMoveUp-1] = state.rules[toMoveUp-1], state.rules[toMoveUp]
				}
				if toMoveDown >= 0 && toMoveDown < len(state.rules)-1 {
					state.rules[toMoveDown], state.rules[toMoveDown+1] = state.rules[toMoveDown+1], state.rules[toMoveDown]
				}

				g.Row(
					g.Button("+ Add Rule").Size(100, 25).OnClick(func() {
						state.rules = append(state.rules, notificationRuleEntry{soundIdx: 1}) // default to first built-in sound
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

		// Draw version in bottom right corner
		windowWidth, windowHeight := g.GetAvailableRegion()
		versionText := "v" + Version
		textWidth := float32(len(versionText) * 7) // approximate width
		imgui.SetCursorPos(imgui.Vec2{X: windowWidth - textWidth - 10, Y: windowHeight - 20})
		g.Style().SetColor(g.StyleColorText, color.RGBA{R: 128, G: 128, B: 128, A: 255}).To(
			g.Label(versionText),
		).Build()

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
	// Convert rules config
	cfg.Rules.Enabled = state.rulesEnabled
	for _, entry := range state.rules {
		rule := entryToRule(entry, state.servers)
		cfg.Rules.Rules = append(cfg.Rules.Rules, rule)
	}
	return cfg
}

// ruleToEntry converts a NotificationRule to editable entry state.
func ruleToEntry(rule NotificationRule, servers []Server) notificationRuleEntry {
	entry := notificationRuleEntry{
		pattern: rule.Pattern,
		source:  rule.Source,
	}

	// Convert action to index
	switch rule.EffectiveAction() {
	case RuleActionNormal:
		entry.actionIdx = 0
	case RuleActionSilent:
		entry.actionIdx = 1
	case RuleActionDismiss:
		entry.actionIdx = 2
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
	}

	return entry
}

// entryToRule converts an editable entry back to a NotificationRule.
func entryToRule(entry notificationRuleEntry, servers []serverEntry) NotificationRule {
	rule := NotificationRule{
		Pattern: entry.pattern,
		Source:  entry.source,
	}

	// Convert action index to RuleAction
	switch entry.actionIdx {
	case 0:
		rule.Action = RuleActionNormal
	case 1:
		rule.Action = RuleActionSilent
	case 2:
		rule.Action = RuleActionDismiss
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
		rule.Sound = ""
	} else if int(entry.soundIdx) < len(soundOptions) {
		rule.Sound = soundOptions[entry.soundIdx]
	}

	return rule
}

// renderRuleRow renders a single notification rule entry as a card.
func renderRuleRow(state *settingsState, index int, toDelete, toMoveUp, toMoveDown *int) {
	rule := &state.rules[index]
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
	if rule.soundIdx > 0 && int(rule.soundIdx) < len(soundOptions) {
		soundToPlay = soundOptions[rule.soundIdx]
	}

	// Card background with rounded corners
	cardBg := color.RGBA{R: 45, G: 45, B: 50, A: 255}
	g.Style().SetColor(g.StyleColorChildBg, cardBg).SetStyleFloat(g.StyleVarChildRounding, 8).To(
		g.Child().Size(ruleCardWidth, 85).Layout(
			// Row 1: Filters
			g.Row(
				g.Label("If:"),
				g.Combo(fmt.Sprintf("##server%d", index), serverOpts[rule.serverIdx], serverOpts, &rule.serverIdx).Size(100),
				g.Label("source:"),
				g.InputText(&rule.source).Size(80).Hint("any").Label(fmt.Sprintf("##source%d", index)),
				g.Combo(fmt.Sprintf("##status%d", index), statusOptions[rule.statusIdx], statusOptions, &rule.statusIdx).Size(100),
				g.Label("pattern:"),
				g.InputText(&rule.pattern).Size(100).Hint("regex").Label(fmt.Sprintf("##pattern%d", index)),
				// Action buttons
				g.Buttonf("↑##up%d", index).Size(25, 20).OnClick(func() {
					*toMoveUp = idx
				}),
				g.Buttonf("↓##down%d", index).Size(25, 20).OnClick(func() {
					*toMoveDown = idx
				}),
				g.Style().SetColor(g.StyleColorButton, color.RGBA{R: 180, G: 60, B: 60, A: 255}).
					SetColor(g.StyleColorButtonHovered, color.RGBA{R: 220, G: 80, B: 80, A: 255}).
					SetColor(g.StyleColorButtonActive, color.RGBA{R: 150, G: 40, B: 40, A: 255}).To(
					g.Buttonf("✕##delrule%d", index).Size(25, 20).OnClick(func() {
						*toDelete = idx
					}),
				),
			),
			// Row 2: Actions
			g.Row(
				g.Label("Then:"),
				g.Combo(fmt.Sprintf("##action%d", index), actionOptions[rule.actionIdx], actionOptions, &rule.actionIdx).Size(80),
				g.Custom(func() {
					// Only show sound options for Normal action
					if rule.actionIdx != 0 {
						return
					}
					g.Label("Sound:").Build()
					g.SameLine()
					g.Combo(fmt.Sprintf("##sound%d", idx), soundOptions[rule.soundIdx], soundOptions, &rule.soundIdx).Size(100).Build()
					g.SameLine()
					g.Buttonf("▶##play%d", idx).Size(25, 20).OnClick(func() {
						if soundToPlay != "" {
							PlaySound(soundToPlay)
						}
					}).Build()
				}),
			),
		),
	).Build()
	g.Spacing().Build()
}
