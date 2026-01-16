// ABOUTME: Settings window renderer using custom OpenGL widgets.
// ABOUTME: Provides configuration UI for client name, servers, rules, and preferences.

package main

import (
	"fmt"
	"log"

	"github.com/go-gl/gl/v2.1/gl"
	"github.com/go-gl/glfw/v3.3/glfw"
)

// ShowSettingsWindowNew displays the settings window using our custom renderer.
// It blocks until the user saves or cancels.
func ShowSettingsWindowNew(initial *Config, isConnected func(string) bool) SettingsResult {
	wm := NewWindowManager()
	settingsWnd, err := wm.CreateSettingsWindow()
	if err != nil {
		log.Printf("Failed to create settings window: %v", err)
		return SettingsResult{Cancelled: true}
	}

	// Center the window on screen
	monitor := glfw.GetPrimaryMonitor()
	if monitor != nil {
		_, workY, workW, workH := monitor.GetWorkarea()
		x := (workW - settingsWidth) / 2
		y := workY + (workH-settingsHeight)/2
		settingsWnd.SetSize(settingsWidth, settingsHeight)
		settingsWnd.SetPos(x, y)
	}

	mw := wm.GetManagedWindow(settingsWnd)
	if mw == nil {
		log.Printf("Failed to get managed window")
		return SettingsResult{Cancelled: true}
	}

	sr := NewSettingsRenderer(mw.Renderer, settingsWnd, initial, isConnected)
	wm.SetWindowRenderCallback(settingsWnd, sr.Render)
	wm.SetWindowCloseCallback(settingsWnd, func() {
		if !sr.Done() {
			sr.result.Cancelled = true
			sr.done = true
		}
	})

	// Render function for both main loop and resize callback
	renderFrame := func() {
		settingsWnd.MakeContextCurrent()
		width, height := settingsWnd.GetSize()
		mw.Renderer.Resize(width, height)
		mw.Renderer.BeginFrame()

		if err := sr.Render(); err != nil {
			log.Printf("Settings render error: %v", err)
		}

		mw.Renderer.EndFrame()
		settingsWnd.SwapBuffers()
	}

	// Track if we rendered during a callback to avoid double-render
	renderedInCallback := false

	// During resize, just show solid background (no wobble from trying to render full UI)
	renderResizePlaceholder := func() {
		settingsWnd.MakeContextCurrent()
		width, height := settingsWnd.GetFramebufferSize()
		gl.Viewport(0, 0, int32(width), int32(height))
		gl.ClearColor(0.12, 0.12, 0.14, 1.0) // Match background color
		gl.Clear(gl.COLOR_BUFFER_BIT)
		settingsWnd.SwapBuffers()
	}

	// Redraw during resize - just show placeholder
	settingsWnd.SetRefreshCallback(func(w *glfw.Window) {
		renderResizePlaceholder()
		renderedInCallback = true
	})

	// Also handle framebuffer size changes
	settingsWnd.SetFramebufferSizeCallback(func(w *glfw.Window, width, height int) {
		renderResizePlaceholder()
		renderedInCallback = true
	})

	// Run until done
	for !sr.Done() && !settingsWnd.ShouldClose() {
		glfw.PollEvents()

		// Skip render if we just rendered in a callback
		if renderedInCallback {
			renderedInCallback = false
		} else {
			renderFrame()
		}

		if sr.Done() {
			settingsWnd.SetShouldClose(true)
		}
	}

	// Cleanup
	settingsWnd.MakeContextCurrent()
	mw.Renderer.Destroy()
	settingsWnd.Destroy()

	return *sr.Result()
}

const (
	settingsWidth      = 700
	settingsHeight     = 700
	settingsPadding    = 16
	settingsRowHeight  = 32
	settingsInputH     = 28
	settingsLabelWidth = 80
)

// SettingsRenderer handles rendering of the settings window.
type SettingsRenderer struct {
	renderer *Renderer
	window   *glfw.Window
	widgets  *WidgetState
	theme    WidgetTheme

	// Settings state
	state   *settingsState
	initial *Config
	isConn  func(string) bool
	result  *SettingsResult
	done    bool

	// Dropdown state
	openDropdown string
}

// NewSettingsRenderer creates a renderer for the settings window.
func NewSettingsRenderer(renderer *Renderer, window *glfw.Window, initial *Config, isConnected func(string) bool) *SettingsRenderer {
	sr := &SettingsRenderer{
		renderer: renderer,
		window:   window,
		widgets:  NewWidgetState(),
		initial:  initial,
		isConn:   isConnected,
		result:   &SettingsResult{},
		theme:    settingsTheme(),
	}

	// Initialize state from config
	sr.state = &settingsState{
		autostartEnabled: IsAutostartInstalled(),
		autostartInitial: IsAutostartInstalled(),
	}
	if initial != nil {
		sr.state.name = initial.Name
		sr.state.debugFontMetrics = initial.DebugFontMetrics
		for _, s := range initial.Servers {
			connected := false
			if isConnected != nil {
				connected = isConnected(s.URL)
			}
			sr.state.servers = append(sr.state.servers, serverEntry{
				url:       s.URL,
				secret:    s.Secret,
				label:     s.Label,
				connected: connected,
			})
		}
		sr.state.rulesEnabled = initial.Rules.Enabled
		for _, rule := range initial.Rules.Rules {
			sr.state.rules = append(sr.state.rules, ruleToEntry(rule, initial.Servers))
		}
	}

	// Setup keyboard callbacks
	window.SetCharCallback(func(w *glfw.Window, r rune) {
		sr.widgets.AddChar(r)
	})
	window.SetKeyCallback(func(w *glfw.Window, key glfw.Key, scancode int, action glfw.Action, mods glfw.ModifierKey) {
		if action == glfw.Press || action == glfw.Repeat {
			sr.widgets.AddKey(key, mods)
		}
	})

	return sr
}

func settingsTheme() WidgetTheme {
	return WidgetTheme{
		Background:    Color{R: 0.18, G: 0.18, B: 0.20, A: 1},
		BackgroundHov: Color{R: 0.25, G: 0.25, B: 0.28, A: 1},
		Border:        Color{R: 0.3, G: 0.3, B: 0.32, A: 1},
		Text:          Color{R: 0.95, G: 0.95, B: 0.95, A: 1},
		TextMuted:     Color{R: 0.5, G: 0.5, B: 0.5, A: 1},
		Accent:        Color{R: 0.3, G: 0.5, B: 0.9, A: 1},
		InputBg:       Color{R: 0.12, G: 0.12, B: 0.14, A: 1},
		InputBorder:   Color{R: 0.3, G: 0.3, B: 0.32, A: 1},
	}
}

// Result returns the settings result after the window closes.
func (sr *SettingsRenderer) Result() *SettingsResult {
	return sr.result
}

// Done returns true if the settings window should close.
func (sr *SettingsRenderer) Done() bool {
	return sr.done
}

// Render draws the settings window.
func (sr *SettingsRenderer) Render() error {
	width, height := sr.window.GetSize()
	sr.widgets.UpdateMouse(sr.window)

	// Draw background
	bgColor := Color{R: 0.12, G: 0.12, B: 0.14, A: 1}
	sr.renderer.DrawRect(0, 0, float32(width), float32(height), bgColor)

	y := float32(settingsPadding)
	x := float32(settingsPadding)
	contentW := float32(width) - settingsPadding*2

	// Title
	Label(sr.renderer, x, y, "Cross-Notifier Settings", sr.theme.Text)
	y += settingsRowHeight

	// Separator
	sr.renderer.DrawRect(x, y, contentW, 1, sr.theme.Border)
	y += settingsPadding

	// Client name
	Label(sr.renderer, x, y+6, "Name:", sr.theme.Text)
	sr.state.name = TextInput(sr.renderer, sr.widgets, "name", x+settingsLabelWidth, y, 200, settingsInputH,
		sr.state.name, "Your name", false, sr.theme)
	y += settingsRowHeight + 8

	// Servers section
	Label(sr.renderer, x, y, "Notification Servers:", sr.theme.Text)
	y += settingsRowHeight

	// Server list
	toDelete := -1
	for i := range sr.state.servers {
		y = sr.renderServerRow(x, y, contentW, i, &toDelete)
	}
	if toDelete >= 0 {
		sr.state.servers = append(sr.state.servers[:toDelete], sr.state.servers[toDelete+1:]...)
	}

	// Add server button
	if Button(sr.renderer, sr.widgets, "add_server", x, y, 0, settingsInputH, "+ Add Server", sr.theme) {
		sr.state.servers = append(sr.state.servers, serverEntry{})
	}
	y += settingsRowHeight + 8

	// Separator
	sr.renderer.DrawRect(x, y, contentW, 1, sr.theme.Border)
	y += settingsPadding

	// Rules section
	Label(sr.renderer, x, y, "Notification Rules:", sr.theme.Text)
	y += settingsRowHeight

	sr.state.rulesEnabled = Checkbox(sr.renderer, sr.widgets, "rules_enabled", x, y,
		"Enable notification rules", sr.state.rulesEnabled, sr.theme)
	y += settingsRowHeight

	if sr.state.rulesEnabled {
		Label(sr.renderer, x, y, "Rules (first match wins):", sr.theme.TextMuted)
		y += settingsRowHeight

		ruleToDelete := -1
		for i := range sr.state.rules {
			y = sr.renderRuleRow(x, y, contentW, i, &ruleToDelete)
		}
		if ruleToDelete >= 0 {
			sr.state.rules = append(sr.state.rules[:ruleToDelete], sr.state.rules[ruleToDelete+1:]...)
		}

		if Button(sr.renderer, sr.widgets, "add_rule", x, y, 0, settingsInputH, "+ Add Rule", sr.theme) {
			sr.state.rules = append(sr.state.rules, notificationRuleEntry{soundIdx: 1})
		}
		y += settingsRowHeight + 8
	}

	// Separator
	sr.renderer.DrawRect(x, y, contentW, 1, sr.theme.Border)
	y += settingsPadding

	// Startup section
	Label(sr.renderer, x, y, "Startup:", sr.theme.Text)
	y += settingsRowHeight

	sr.state.autostartEnabled = Checkbox(sr.renderer, sr.widgets, "autostart", x, y,
		"Start automatically on login", sr.state.autostartEnabled, sr.theme)
	y += settingsRowHeight + 8

	// Separator
	sr.renderer.DrawRect(x, y, contentW, 1, sr.theme.Border)
	y += settingsPadding

	// Debug section
	Label(sr.renderer, x, y, "Debug:", sr.theme.Text)
	y += settingsRowHeight

	sr.state.debugFontMetrics = Checkbox(sr.renderer, sr.widgets, "debug_font", x, y,
		"Show font metrics overlay", sr.state.debugFontMetrics, sr.theme)
	y += settingsRowHeight + 8

	// Separator
	sr.renderer.DrawRect(x, y, contentW, 1, sr.theme.Border)

	// Action buttons at bottom
	btnY := float32(height) - settingsPadding - settingsInputH - 10
	btnH := float32(30)
	btnPad := float32(12)
	btnGap := float32(10)

	// Calculate button widths for positioning
	saveW := ButtonWidth(sr.renderer, "Save", btnPad)
	localW := ButtonWidth(sr.renderer, "Local Only", btnPad)
	cancelW := ButtonWidth(sr.renderer, "Cancel", btnPad)

	btnX := x
	if Button(sr.renderer, sr.widgets, "save", btnX, btnY, 0, btnH, "Save", sr.theme) {
		sr.result.Config = stateToConfig(sr.state)
		sr.result.Cancelled = false
		sr.applyAutostartChange()
		sr.done = true
	}
	btnX += saveW + btnGap

	if Button(sr.renderer, sr.widgets, "local_only", btnX, btnY, 0, btnH, "Local Only", sr.theme) {
		sr.result.Config = &Config{Name: sr.state.name}
		sr.result.Cancelled = false
		sr.applyAutostartChange()
		sr.done = true
	}
	btnX += localW + btnGap

	if Button(sr.renderer, sr.widgets, "cancel", btnX, btnY, 0, btnH, "Cancel", sr.theme) {
		sr.result.Cancelled = true
		sr.done = true
	}
	_ = cancelW // used for positioning if we add more buttons

	// Version in bottom right
	versionText := "v" + Version
	vw, _ := sr.renderer.fontAtlas.MeasureText(versionText)
	sr.renderer.DrawText(float32(width)-float32(vw)-settingsPadding, btnY+8, versionText, sr.theme.TextMuted)

	// Draw dropdown popups last so they appear on top of other widgets
	DrawDeferredDropdowns(sr.widgets)

	sr.widgets.EndFrame()
	return nil
}

func (sr *SettingsRenderer) renderServerRow(x, y, contentW float32, index int, toDelete *int) float32 {
	server := &sr.state.servers[index]
	labelGap := float32(8) // Gap between label and input

	// Status indicator
	statusColor := sr.theme.TextMuted
	statusText := "○"
	if server.connected {
		statusColor = Color{R: 0.2, G: 0.8, B: 0.3, A: 1}
		statusText = "●"
	}
	sr.renderer.DrawText(x, y+6, statusText, statusColor)

	// Label input
	labelX := x + 20
	labelLabelW, _ := sr.renderer.fontAtlas.MeasureText("Label:")
	Label(sr.renderer, labelX, y+6, "Label:", sr.theme.TextMuted)
	server.label = TextInput(sr.renderer, sr.widgets, fmt.Sprintf("server_label_%d", index),
		labelX+float32(labelLabelW)+labelGap, y, 80, settingsInputH, server.label, "Work", false, sr.theme)

	// URL input
	urlX := labelX + float32(labelLabelW) + labelGap + 80 + 10
	urlLabelW, _ := sr.renderer.fontAtlas.MeasureText("URL:")
	Label(sr.renderer, urlX, y+6, "URL:", sr.theme.TextMuted)
	server.url = TextInput(sr.renderer, sr.widgets, fmt.Sprintf("server_url_%d", index),
		urlX+float32(urlLabelW)+labelGap, y, 180, settingsInputH, server.url, "ws://host:9876/ws", false, sr.theme)

	// Secret input
	secretX := urlX + float32(urlLabelW) + labelGap + 180 + 10
	secretLabelW, _ := sr.renderer.fontAtlas.MeasureText("Secret:")
	Label(sr.renderer, secretX, y+6, "Secret:", sr.theme.TextMuted)
	server.secret = TextInput(sr.renderer, sr.widgets, fmt.Sprintf("server_secret_%d", index),
		secretX+float32(secretLabelW)+labelGap, y, 100, settingsInputH, server.secret, "", true, sr.theme)

	// Delete button
	deleteX := secretX + float32(secretLabelW) + labelGap + 100 + 10
	idx := index
	if Button(sr.renderer, sr.widgets, fmt.Sprintf("delete_server_%d", index), deleteX, y, 25, settingsInputH, "X", sr.theme) {
		*toDelete = idx
	}

	return y + settingsRowHeight + 4
}

func (sr *SettingsRenderer) renderRuleRow(x, y, contentW float32, index int, toDelete *int) float32 {
	rule := &sr.state.rules[index]
	labelGap := float32(8)  // Gap between label and input
	fieldGap := float32(10) // Gap between fields

	// Card background
	cardH := float32(70)
	cardBg := Color{R: 0.15, G: 0.15, B: 0.17, A: 1}
	sr.renderer.DrawRect(x, y, contentW, cardH, cardBg)
	sr.renderer.DrawBorder(x, y, contentW, cardH, 1, sr.theme.Border)

	innerX := x + 10
	innerY := y + 8

	// Row 1: Filters
	ifLabelW, _ := sr.renderer.fontAtlas.MeasureText("If:")
	Label(sr.renderer, innerX, innerY+6, "If:", sr.theme.TextMuted)

	// Server dropdown
	serverOpts := []string{"Any Server"}
	for _, s := range sr.state.servers {
		label := s.label
		if label == "" {
			label = s.url
		}
		if label != "" {
			serverOpts = append(serverOpts, label)
		}
	}
	if rule.serverIdx < 0 || int(rule.serverIdx) >= len(serverOpts) {
		rule.serverIdx = 0
	}
	serverDropX := innerX + float32(ifLabelW) + labelGap
	dropdownID := fmt.Sprintf("rule_server_%d", index)
	isOpen := sr.openDropdown == dropdownID
	newIdx, newOpen := Dropdown(sr.renderer, sr.widgets, dropdownID, serverDropX, innerY, 100, settingsInputH,
		serverOpts, int(rule.serverIdx), isOpen, sr.theme)
	rule.serverIdx = int32(newIdx)
	if newOpen && !isOpen {
		sr.openDropdown = dropdownID
	} else if !newOpen && isOpen {
		sr.openDropdown = ""
	}

	// Source input
	sourceLabelX := serverDropX + 100 + fieldGap
	sourceLabelW, _ := sr.renderer.fontAtlas.MeasureText("source:")
	Label(sr.renderer, sourceLabelX, innerY+6, "source:", sr.theme.TextMuted)
	rule.source = TextInput(sr.renderer, sr.widgets, fmt.Sprintf("rule_source_%d", index),
		sourceLabelX+float32(sourceLabelW)+labelGap, innerY, 80, settingsInputH, rule.source, "any", false, sr.theme)

	// Status dropdown
	statusLabelX := sourceLabelX + float32(sourceLabelW) + labelGap + 80 + fieldGap
	statusDropID := fmt.Sprintf("rule_status_%d", index)
	statusOpen := sr.openDropdown == statusDropID
	if rule.statusIdx < 0 || int(rule.statusIdx) >= len(statusOptions) {
		rule.statusIdx = 0
	}
	statusIdx, statusNewOpen := Dropdown(sr.renderer, sr.widgets, statusDropID, statusLabelX, innerY, 100, settingsInputH,
		statusOptions, int(rule.statusIdx), statusOpen, sr.theme)
	rule.statusIdx = int32(statusIdx)
	if statusNewOpen && !statusOpen {
		sr.openDropdown = statusDropID
	} else if !statusNewOpen && statusOpen {
		sr.openDropdown = ""
	}

	// Pattern input
	patternLabelX := statusLabelX + 100 + fieldGap
	patternLabelW, _ := sr.renderer.fontAtlas.MeasureText("pattern:")
	Label(sr.renderer, patternLabelX, innerY+6, "pattern:", sr.theme.TextMuted)
	rule.pattern = TextInput(sr.renderer, sr.widgets, fmt.Sprintf("rule_pattern_%d", index),
		patternLabelX+float32(patternLabelW)+labelGap, innerY, 80, settingsInputH, rule.pattern, "regex", false, sr.theme)

	// Delete button
	idx := index
	if Button(sr.renderer, sr.widgets, fmt.Sprintf("delete_rule_%d", index), contentW-35, innerY, 25, settingsInputH, "X", sr.theme) {
		*toDelete = idx
	}

	// Row 2: Actions
	innerY += settingsRowHeight
	thenLabelW, _ := sr.renderer.fontAtlas.MeasureText("Then:")
	Label(sr.renderer, innerX, innerY+6, "Then:", sr.theme.TextMuted)

	// Action dropdown
	actionDropX := innerX + float32(thenLabelW) + labelGap
	actionDropID := fmt.Sprintf("rule_action_%d", index)
	actionOpen := sr.openDropdown == actionDropID
	if rule.actionIdx < 0 || int(rule.actionIdx) >= len(actionOptions) {
		rule.actionIdx = 0
	}
	actionIdx, actionNewOpen := Dropdown(sr.renderer, sr.widgets, actionDropID, actionDropX, innerY, 80, settingsInputH,
		actionOptions, int(rule.actionIdx), actionOpen, sr.theme)
	rule.actionIdx = int32(actionIdx)
	if actionNewOpen && !actionOpen {
		sr.openDropdown = actionDropID
	} else if !actionNewOpen && actionOpen {
		sr.openDropdown = ""
	}

	// Sound dropdown (only for Normal action)
	if rule.actionIdx == 0 {
		soundLabelX := actionDropX + 80 + fieldGap
		soundLabelW, _ := sr.renderer.fontAtlas.MeasureText("Sound:")
		Label(sr.renderer, soundLabelX, innerY+6, "Sound:", sr.theme.TextMuted)
		soundDropID := fmt.Sprintf("rule_sound_%d", index)
		soundOpen := sr.openDropdown == soundDropID
		if rule.soundIdx < 0 || int(rule.soundIdx) >= len(soundOptions) {
			rule.soundIdx = 0
		}
		soundDropX := soundLabelX + float32(soundLabelW) + labelGap
		soundIdx, soundNewOpen := Dropdown(sr.renderer, sr.widgets, soundDropID, soundDropX, innerY, 100, settingsInputH,
			soundOptions, int(rule.soundIdx), soundOpen, sr.theme)
		rule.soundIdx = int32(soundIdx)
		if soundNewOpen && !soundOpen {
			sr.openDropdown = soundDropID
		} else if !soundNewOpen && soundOpen {
			sr.openDropdown = ""
		}

		// Play button
		if Button(sr.renderer, sr.widgets, fmt.Sprintf("play_sound_%d", index), soundDropX+100+fieldGap, innerY, 25, settingsInputH, "▶", sr.theme) {
			if rule.soundIdx > 0 && int(rule.soundIdx) < len(soundOptions) {
				PlaySound(soundOptions[rule.soundIdx])
			}
		}
	}

	return y + cardH + 8
}

func (sr *SettingsRenderer) applyAutostartChange() {
	if sr.state.autostartEnabled != sr.state.autostartInitial {
		if sr.state.autostartEnabled {
			_ = InstallAutostart()
		} else {
			_ = UninstallAutostart()
		}
	}
}
