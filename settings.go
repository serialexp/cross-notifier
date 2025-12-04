// ABOUTME: Settings window for configuring the notification daemon.
// ABOUTME: Allows users to configure client name and multiple notification servers.

package main

import (
	"image/color"

	g "github.com/AllenDang/giu"
)

// SettingsResult holds the outcome of the settings window.
type SettingsResult struct {
	Config    *Config
	Cancelled bool
}

// settingsState holds the editable state for the settings window.
type settingsState struct {
	name    string
	servers []serverEntry
}

// serverEntry holds editable fields for a single server.
type serverEntry struct {
	url       string
	secret    string
	label     string
	connected bool // connection status
}

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
	}

	// Calculate window height based on number of servers
	baseHeight := 200
	serverRowHeight := 35
	windowHeight := baseHeight + len(state.servers)*serverRowHeight
	if windowHeight < 250 {
		windowHeight = 250
	}
	if windowHeight > 500 {
		windowHeight = 500
	}

	wnd := g.NewMasterWindow("Cross-Notifier Settings", 700, windowHeight, 0)
	wnd.SetBgColor(color.RGBA{R: 30, G: 30, B: 35, A: 255})

	wnd.Run(func() {
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
	return cfg
}
