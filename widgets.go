// ABOUTME: Reusable UI widgets built on top of the OpenGL renderer.
// ABOUTME: Provides buttons, checkboxes, text inputs, and dropdowns.

package main

import (
	"time"
	"unicode"

	"github.com/go-gl/glfw/v3.3/glfw"
)

// WidgetTheme holds colors for widget rendering.
type WidgetTheme struct {
	Background    Color
	BackgroundHov Color
	Border        Color
	Text          Color
	TextMuted     Color
	Accent        Color
	InputBg       Color
	InputBorder   Color
}

// DeferredDropdown holds info for drawing dropdown popups in a later pass.
type DeferredDropdown struct {
	ID          string
	X, Y, W, H  float32
	Options     []string
	SelectedIdx int
	Theme       WidgetTheme
	Renderer    *Renderer
}

// WidgetState tracks interactive state for widgets.
type WidgetState struct {
	MouseX, MouseY float32
	MouseDown      bool
	PrevMouseDown  bool

	// Text input state
	FocusedID    string
	CursorPos    int
	CursorBlink  time.Time
	CharBuffer   []rune
	KeyBuffer    []glfw.Key
	ModBuffer    []glfw.ModifierKey
	ScrollOffset map[string]float32 // horizontal scroll offset per input ID

	// Deferred dropdown popups
	DeferredDropdowns []DeferredDropdown
}

// NewWidgetState creates a new widget state.
func NewWidgetState() *WidgetState {
	return &WidgetState{
		CursorBlink:  time.Now(),
		ScrollOffset: make(map[string]float32),
	}
}

// JustClicked returns true if mouse was just pressed this frame.
func (ws *WidgetState) JustClicked() bool {
	return ws.MouseDown && !ws.PrevMouseDown
}

// JustReleased returns true if mouse was just released this frame.
func (ws *WidgetState) JustReleased() bool {
	return !ws.MouseDown && ws.PrevMouseDown
}

// UpdateMouse updates mouse state from window.
func (ws *WidgetState) UpdateMouse(wnd *glfw.Window) {
	ws.PrevMouseDown = ws.MouseDown
	x, y := wnd.GetCursorPos()
	ws.MouseX = float32(x)
	ws.MouseY = float32(y)
	ws.MouseDown = wnd.GetMouseButton(glfw.MouseButtonLeft) == glfw.Press
}

// EndFrame clears per-frame buffers.
func (ws *WidgetState) EndFrame() {
	ws.CharBuffer = ws.CharBuffer[:0]
	ws.KeyBuffer = ws.KeyBuffer[:0]
	ws.ModBuffer = ws.ModBuffer[:0]
	ws.DeferredDropdowns = ws.DeferredDropdowns[:0]
}

// AddChar adds a character to the input buffer.
func (ws *WidgetState) AddChar(r rune) {
	ws.CharBuffer = append(ws.CharBuffer, r)
}

// AddKey adds a key press to the input buffer.
func (ws *WidgetState) AddKey(key glfw.Key, mods glfw.ModifierKey) {
	ws.KeyBuffer = append(ws.KeyBuffer, key)
	ws.ModBuffer = append(ws.ModBuffer, mods)
}

// ButtonWidth calculates the width needed for a button with the given label.
func ButtonWidth(r *Renderer, label string, padding float32) float32 {
	labelW, _ := r.fontAtlas.MeasureText(label)
	return float32(labelW) + padding*2
}

// Button draws a clickable button and returns true if clicked.
// Width is auto-calculated from text; pass minW to enforce a minimum width.
func Button(r *Renderer, ws *WidgetState, id string, x, y, minW, h float32, label string, theme WidgetTheme) bool {
	// Calculate width from text with padding
	padding := float32(12)
	labelW, _ := r.fontAtlas.MeasureText(label)
	w := float32(labelW) + padding*2
	if w < minW {
		w = minW
	}

	hovered := pointInRect(ws.MouseX, ws.MouseY, x, y, w, h)

	bg := theme.Background
	if hovered {
		bg = theme.BackgroundHov
	}

	r.DrawRect(x, y, w, h, bg)

	// Center text horizontally and vertically
	textX := x + (w-float32(labelW))/2
	// Use font Size for visual centering (Height includes descent which throws off centering)
	textY := y + (h-float32(r.fontAtlas.Size))/2
	r.DrawText(textX, textY, label, theme.Text)

	return hovered && ws.JustClicked()
}

// Checkbox draws a checkbox and returns the new value.
func Checkbox(r *Renderer, ws *WidgetState, id string, x, y float32, label string, value bool, theme WidgetTheme) bool {
	boxSize := float32(18)
	fontSize := float32(r.fontAtlas.Size)

	// Use the taller of box or font as the row height
	rowHeight := boxSize
	if fontSize > rowHeight {
		rowHeight = fontSize
	}

	// Center box and text vertically within row
	boxY := y + (rowHeight-boxSize)/2
	textY := y + (rowHeight-fontSize)/2

	hovered := pointInRect(ws.MouseX, ws.MouseY, x, y, boxSize+float32(len(label)*8)+8, rowHeight)

	// Draw box
	r.DrawRect(x, boxY, boxSize, boxSize, theme.InputBg)
	r.DrawBorder(x, boxY, boxSize, boxSize, 1, theme.InputBorder)

	// Draw inner filled square if checked
	if value {
		inset := float32(4)
		innerSize := boxSize - inset*2
		r.DrawRect(x+inset, boxY+inset, innerSize, innerSize, theme.Accent)
	}

	// Draw label
	r.DrawText(x+boxSize+8, textY, label, theme.Text)

	// Toggle on click
	if hovered && ws.JustClicked() {
		return !value
	}
	return value
}

// TextInput draws a text input field and returns the updated value.
func TextInput(r *Renderer, ws *WidgetState, id string, x, y, w, h float32, value string, hint string, password bool, theme WidgetTheme) string {
	hovered := pointInRect(ws.MouseX, ws.MouseY, x, y, w, h)
	focused := ws.FocusedID == id
	padding := float32(6)
	visibleWidth := w - padding*2

	// Click to focus
	if hovered && ws.JustClicked() {
		ws.FocusedID = id
		ws.CursorPos = len([]rune(value))
		ws.CursorBlink = time.Now()
		focused = true
	}

	// Draw background
	bg := theme.InputBg
	r.DrawRect(x, y, w, h, bg)

	// Draw border (highlighted if focused)
	borderColor := theme.InputBorder
	if focused {
		borderColor = theme.Accent
	}
	r.DrawBorder(x, y, w, h, 1, borderColor)

	// Process input if focused
	runes := []rune(value)
	if focused {
		// Clamp cursor
		if ws.CursorPos > len(runes) {
			ws.CursorPos = len(runes)
		}
		if ws.CursorPos < 0 {
			ws.CursorPos = 0
		}

		// Handle character input
		for _, ch := range ws.CharBuffer {
			if unicode.IsPrint(ch) {
				// Insert character at cursor
				newRunes := make([]rune, 0, len(runes)+1)
				newRunes = append(newRunes, runes[:ws.CursorPos]...)
				newRunes = append(newRunes, ch)
				newRunes = append(newRunes, runes[ws.CursorPos:]...)
				runes = newRunes
				ws.CursorPos++
				ws.CursorBlink = time.Now()
			}
		}

		// Handle special keys
		for _, key := range ws.KeyBuffer {
			switch key {
			case glfw.KeyBackspace:
				if ws.CursorPos > 0 {
					runes = append(runes[:ws.CursorPos-1], runes[ws.CursorPos:]...)
					ws.CursorPos--
					ws.CursorBlink = time.Now()
				}
			case glfw.KeyDelete:
				if ws.CursorPos < len(runes) {
					runes = append(runes[:ws.CursorPos], runes[ws.CursorPos+1:]...)
					ws.CursorBlink = time.Now()
				}
			case glfw.KeyLeft:
				if ws.CursorPos > 0 {
					ws.CursorPos--
					ws.CursorBlink = time.Now()
				}
			case glfw.KeyRight:
				if ws.CursorPos < len(runes) {
					ws.CursorPos++
					ws.CursorBlink = time.Now()
				}
			case glfw.KeyHome:
				ws.CursorPos = 0
				ws.CursorBlink = time.Now()
			case glfw.KeyEnd:
				ws.CursorPos = len(runes)
				ws.CursorBlink = time.Now()
			}
		}
	}

	// Build display text (password masking)
	displayRunes := runes
	if password && len(runes) > 0 {
		displayRunes = make([]rune, len(runes))
		for i := range runes {
			displayRunes[i] = '*'
		}
	}
	displayText := string(displayRunes)

	// Calculate cursor position in pixels
	cursorTextWidth := float32(0)
	if ws.CursorPos > 0 && ws.CursorPos <= len(displayRunes) {
		w, _ := r.fontAtlas.MeasureText(string(displayRunes[:ws.CursorPos]))
		cursorTextWidth = float32(w)
	}

	// Get/update scroll offset for this input
	scrollOffset := ws.ScrollOffset[id]

	// Adjust scroll to keep cursor visible
	cursorScreenX := cursorTextWidth - scrollOffset
	if cursorScreenX < 0 {
		// Cursor is off the left edge, scroll left
		scrollOffset = cursorTextWidth
	} else if cursorScreenX > visibleWidth {
		// Cursor is off the right edge, scroll right
		scrollOffset = cursorTextWidth - visibleWidth
	}
	// Clamp scroll offset
	if scrollOffset < 0 {
		scrollOffset = 0
	}
	ws.ScrollOffset[id] = scrollOffset

	// Draw text or hint with clipping
	textY := y + (h-float32(r.fontAtlas.Size))/2
	textX := x + padding

	if len(runes) == 0 && !focused && hint != "" {
		r.DrawText(textX, textY, hint, theme.TextMuted)
	} else if len(displayText) > 0 {
		// Find the visible portion of text
		// Start from scrollOffset and render what fits in visibleWidth
		startChar, endChar := findVisibleRange(r, displayRunes, scrollOffset, visibleWidth)
		if endChar > startChar {
			visibleText := string(displayRunes[startChar:endChar])
			// Calculate X offset for partial first character
			startWidth := float32(0)
			if startChar > 0 {
				w, _ := r.fontAtlas.MeasureText(string(displayRunes[:startChar]))
				startWidth = float32(w)
			}
			drawX := textX + startWidth - scrollOffset
			r.DrawText(drawX, textY, visibleText, theme.Text)
		}
	}

	// Draw cursor if focused
	if focused {
		// Blink cursor
		elapsed := time.Since(ws.CursorBlink).Milliseconds()
		if (elapsed/500)%2 == 0 {
			cursorX := textX + cursorTextWidth - scrollOffset
			// Only draw cursor if it's within visible area
			if cursorX >= textX && cursorX <= textX+visibleWidth {
				cursorH := float32(r.fontAtlas.Size)
				r.DrawRect(cursorX, textY, 1, cursorH, theme.Text)
			}
		}
	}

	return string(runes)
}

// findVisibleRange finds the range of characters visible within the given width starting from scrollOffset.
func findVisibleRange(r *Renderer, runes []rune, scrollOffset, visibleWidth float32) (start, end int) {
	if len(runes) == 0 {
		return 0, 0
	}

	// Find first visible character
	currentX := float32(0)
	for i, ch := range runes {
		glyph, ok := r.fontAtlas.Glyphs[ch]
		if !ok {
			continue
		}
		charEnd := currentX + float32(glyph.Advance)
		if charEnd > scrollOffset {
			start = i
			break
		}
		currentX = charEnd
		if i == len(runes)-1 {
			start = len(runes)
		}
	}

	// Find last visible character
	end = start
	visibleEnd := scrollOffset + visibleWidth
	currentX = float32(0)
	for i, ch := range runes {
		glyph, ok := r.fontAtlas.Glyphs[ch]
		if !ok {
			continue
		}
		currentX += float32(glyph.Advance)
		if i >= start {
			end = i + 1
			if currentX > visibleEnd {
				break
			}
		}
	}

	return start, end
}

// Dropdown draws a dropdown selector and returns the selected index.
// Returns (newIndex, open) - open indicates if dropdown should show options.
// The popup is drawn in a deferred pass via DrawDeferredDropdowns.
func Dropdown(r *Renderer, ws *WidgetState, id string, x, y, w, h float32, options []string, selectedIdx int, isOpen bool, theme WidgetTheme) (int, bool) {
	hovered := pointInRect(ws.MouseX, ws.MouseY, x, y, w, h)

	// Draw button background
	bg := theme.InputBg
	if hovered {
		bg = theme.BackgroundHov
	}
	r.DrawRect(x, y, w, h, bg)
	r.DrawBorder(x, y, w, h, 1, theme.InputBorder)

	// Draw selected text
	padding := float32(6)
	textY := y + (h-float32(r.fontAtlas.Size))/2
	selectedText := ""
	if selectedIdx >= 0 && selectedIdx < len(options) {
		selectedText = options[selectedIdx]
	}
	r.DrawText(x+padding, textY, selectedText, theme.Text)

	// Draw dropdown arrow
	arrowX := x + w - 16
	r.DrawText(arrowX, textY, "v", theme.TextMuted)

	// Toggle open on click of the button
	newOpen := isOpen
	if hovered && ws.JustClicked() {
		newOpen = !isOpen
	}

	// Handle option selection if open (check clicks before deferring draw)
	newIdx := selectedIdx
	if isOpen {
		optY := y + h
		for i := range options {
			optHovered := pointInRect(ws.MouseX, ws.MouseY, x, optY, w, h)
			if optHovered && ws.JustClicked() {
				newIdx = i
				newOpen = false
			}
			optY += h
		}

		// Close dropdown if clicked outside
		totalH := h + float32(len(options))*h
		clickedOutside := ws.JustClicked() && !pointInRect(ws.MouseX, ws.MouseY, x, y, w, totalH)
		if clickedOutside {
			newOpen = false
		}

		// Defer the popup drawing to a later pass (so it renders on top)
		ws.DeferredDropdowns = append(ws.DeferredDropdowns, DeferredDropdown{
			ID:          id,
			X:           x,
			Y:           y + h, // Start below the button
			W:           w,
			H:           h,
			Options:     options,
			SelectedIdx: selectedIdx,
			Theme:       theme,
			Renderer:    r,
		})
	}

	return newIdx, newOpen
}

// DrawDeferredDropdowns draws all deferred dropdown popups.
// Call this at the end of your render function so popups appear on top.
func DrawDeferredDropdowns(ws *WidgetState) {
	for _, dd := range ws.DeferredDropdowns {
		r := dd.Renderer
		padding := float32(6)
		totalH := float32(len(dd.Options)) * dd.H

		// Draw solid background for entire popup
		r.DrawRect(dd.X, dd.Y, dd.W, totalH, dd.Theme.InputBg)

		// Draw individual options
		optY := dd.Y
		for i, opt := range dd.Options {
			optHovered := pointInRect(ws.MouseX, ws.MouseY, dd.X, optY, dd.W, dd.H)
			if optHovered {
				r.DrawRect(dd.X, optY, dd.W, dd.H, dd.Theme.BackgroundHov)
			} else if i == dd.SelectedIdx {
				// Highlight current selection
				selBg := dd.Theme.Accent
				selBg.A = 0.3
				r.DrawRect(dd.X, optY, dd.W, dd.H, selBg)
			}
			r.DrawText(dd.X+padding, optY+(dd.H-float32(r.fontAtlas.Size))/2, opt, dd.Theme.Text)
			optY += dd.H
		}

		// Draw border around dropdown options
		r.DrawBorder(dd.X, dd.Y, dd.W, totalH, 1, dd.Theme.InputBorder)
	}
}

// Label draws a text label.
func Label(r *Renderer, x, y float32, text string, color Color) {
	r.DrawText(x, y, text, color)
}
