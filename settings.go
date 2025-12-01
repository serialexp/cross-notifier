// ABOUTME: Settings window for configuring the notification daemon.
// ABOUTME: Allows users to set server URL and authentication secret.

package main

import (
	"image/color"

	g "github.com/AllenDang/giu"
)

// SettingsResult holds the outcome of the settings window.
type SettingsResult struct {
	ServerURL string
	Secret    string
	Cancelled bool
}

// ShowSettingsWindow displays a configuration window and blocks until the user
// saves or cancels. Returns the configuration values entered.
func ShowSettingsWindow(initial *Config) SettingsResult {
	var result SettingsResult
	var serverURL, secret string
	done := false

	if initial != nil {
		serverURL = initial.ServerURL
		secret = initial.Secret
	}

	wnd := g.NewMasterWindow("Cross-Notifier Settings", 400, 200, 0)
	wnd.SetBgColor(color.RGBA{R: 30, G: 30, B: 35, A: 255})

	wnd.Run(func() {
		g.SingleWindow().Layout(
			g.Label("Configure notification server connection:"),
			g.Spacing(),

			g.Row(
				g.Label("Server URL:"),
				g.InputText(&serverURL).Size(250).Hint("ws://host:9876/ws"),
			),
			g.Spacing(),

			g.Row(
				g.Label("Secret:"),
				g.InputText(&secret).Size(250).Flags(g.InputTextFlagsPassword),
			),
			g.Spacing(),
			g.Spacing(),

			g.Row(
				g.Button("Save & Start").Size(120, 30).OnClick(func() {
					result.ServerURL = serverURL
					result.Secret = secret
					result.Cancelled = false
					done = true
				}),
				g.Button("Local Only").Size(120, 30).OnClick(func() {
					result.ServerURL = ""
					result.Secret = ""
					result.Cancelled = false
					done = true
				}),
				g.Button("Cancel").Size(80, 30).OnClick(func() {
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
