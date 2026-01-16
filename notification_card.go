package main

import (
	"fmt"
	"image"
	"time"
)

type NotificationCardTheme struct {
	CardBg      Color
	Title       Color
	Body        Color
	Muted       Color
	ButtonBg    Color
	ButtonHv    Color
	DismissBg   Color
	DismissHv   Color
	DismissText Color
}

type NotificationCardData struct {
	ID          int64
	Title       string
	Message     string
	Status      string
	Source      string
	SourceLine  string // Pre-computed source line (if empty, computed from Source + CreatedAt)
	Actions     []Action
	Icon        *image.RGBA
	IconTexture uint32
	CreatedAt   time.Time
	Expanded    bool
}

type NotificationCardOptions struct {
	Padding            float32
	IconSize           float32
	ActionRowH         float32
	ButtonH            float32
	ShowDismiss        bool
	ShowSource         bool
	AllowDismissOnCard bool
	MouseX             float32
	MouseY             float32
	JustClicked        bool
	BorderColor        Color
	GetActionState     func(idx int) ActionState
	OnAction           func(idx int, action Action)
	OnDismiss          func()
	OnExpand           func()
}

func drawNotificationCard(r *Renderer, n NotificationCardData, x, y, w, h float32, theme NotificationCardTheme, opts NotificationCardOptions) bool {
	isHovered := pointInRect(opts.MouseX, opts.MouseY, x, y, w, h)
	bg := theme.CardBg
	if isHovered {
		bg.A = 1
	}

	r.DrawRect(x, y, w, h, bg)
	r.DrawBorder(x, y, w, h, 1, opts.BorderColor)

	paddingX := opts.Padding
	textStartX := x + paddingX

	// Render icon if available
	if n.IconTexture != 0 {
		iconX := x + paddingX
		iconY := y + paddingX
		r.DrawTexture(iconX, iconY, opts.IconSize, opts.IconSize, n.IconTexture, Color{1, 1, 1, 1})
		textStartX = iconX + opts.IconSize + paddingX
	}

	textAreaWidth := w - (textStartX - x) - paddingX
	// Reserve space for expand button (always) and dismiss button (if has actions)
	expandButtonWidth := float32(22)
	dismissButtonWidth := float32(0)
	if len(n.Actions) > 0 {
		dismissButtonWidth = 22
	}
	textAreaWidth -= expandButtonWidth + dismissButtonWidth

	// Title
	titleY := y + paddingX
	if n.Title != "" {
		title := r.fontAtlas.Truncate(n.Title, int(textAreaWidth))
		r.DrawText(textStartX, titleY, title, theme.Title)
	}

	// Check if message needs truncation (to decide whether to show expand button)
	messageWidth := int(textAreaWidth + expandButtonWidth + dismissButtonWidth)
	needsExpand := false
	if n.Message != "" {
		wrapped := r.fontAtlas.WrapText(n.Message, messageWidth)
		needsExpand = countLines(wrapped) > 2
	}

	// Expand button (only show if message is truncated or already expanded)
	expandClicked := false
	if opts.ShowDismiss && (needsExpand || n.Expanded) {
		expandX := x + w - 30
		if len(n.Actions) > 0 {
			expandX = x + w - 55 // Left of dismiss button
		}
		expandY := y + paddingX - 2
		expandW := float32(20)
		expandH := float32(20)
		expandHovered := pointInRect(opts.MouseX, opts.MouseY, expandX, expandY, expandW, expandH)
		if expandHovered {
			r.DrawRect(expandX, expandY, expandW, expandH, theme.DismissHv)
			if opts.JustClicked && opts.OnExpand != nil {
				opts.OnExpand()
				expandClicked = true
			}
		} else {
			r.DrawRect(expandX, expandY, expandW, expandH, theme.DismissBg)
		}
		// Draw expand/collapse icon
		expandIcon := "▼"
		if n.Expanded {
			expandIcon = "▲"
		}
		iconW, _ := r.fontAtlas.MeasureText(expandIcon)
		iconX := expandX + (expandW-float32(iconW))/2
		iconY := expandY + (expandH-float32(r.fontAtlas.Size))/2
		r.DrawText(iconX, iconY, expandIcon, theme.DismissText)
	}

	// Dismiss button
	if opts.ShowDismiss && len(n.Actions) > 0 {
		dismissX := x + w - 30
		dismissY := y + paddingX - 2
		dismissW := float32(20)
		dismissH := float32(20)
		hovered := pointInRect(opts.MouseX, opts.MouseY, dismissX, dismissY, dismissW, dismissH)
		if hovered {
			r.DrawRect(dismissX, dismissY, dismissW, dismissH, theme.DismissHv)
			if opts.JustClicked && opts.OnDismiss != nil {
				opts.OnDismiss()
			}
		} else {
			r.DrawRect(dismissX, dismissY, dismissW, dismissH, theme.DismissBg)
		}
		// Center the "x" in the button
		labelW, _ := r.fontAtlas.MeasureText("x")
		textX := dismissX + (dismissW-float32(labelW))/2
		textY := dismissY + (dismissH-float32(r.fontAtlas.Size))/2
		// Use hover text color when hovered (for light theme: switches to light text on red bg)
		textColor := theme.DismissText
		if hovered && theme.DismissHv.R > 0.5 {
			// If hover bg is reddish, use white text
			textColor = Color{R: 1, G: 1, B: 1, A: 1}
		}
		r.DrawText(textX, textY, "x", textColor)
	}

	// Message
	messageY := titleY + textHeight + sectionGap
	var messageLines int
	if n.Message != "" {
		var displayMessage string
		if n.Expanded {
			displayMessage = r.fontAtlas.WrapText(n.Message, messageWidth)
		} else {
			displayMessage = r.fontAtlas.TruncateLines(n.Message, messageWidth, 2)
		}
		messageLines = countLines(displayMessage)
		drawMultilineText(r, textStartX, messageY, displayMessage, lineSpacing, theme.Body)
	}
	if messageLines < 2 {
		messageLines = 2 // Minimum 2 lines worth of space
	}

	// Source/timestamp
	messageHeight := float32(messageLines)*textHeight + float32(messageLines-1)*lineSpacing
	sourceY := messageY + messageHeight + sectionGap
	if opts.ShowSource {
		sourceText := n.SourceLine
		if sourceText == "" {
			sourceText = formatSourceLine(n.Source, n.CreatedAt)
		}
		r.DrawText(textStartX, sourceY, sourceText, theme.Muted)
	}

	// Action buttons
	if len(n.Actions) > 0 {
		actionsY := sourceY + textHeight + sectionGap
		renderCardActionButtons(r, n, textStartX, actionsY, textAreaWidth, theme, opts)
	}

	if opts.AllowDismissOnCard && len(n.Actions) == 0 && isHovered && opts.JustClicked && !expandClicked && opts.OnDismiss != nil {
		opts.OnDismiss()
	}

	// Debug layout visualization
	if debugFontMetrics {
		debugColor := Color{R: 1, G: 0, B: 1, A: 0.8}
		labelColor := Color{R: 0, G: 0.5, B: 0, A: 1}

		// Draw separator lines and labels at each section
		curY := y

		// Top padding
		r.DrawRect(x, curY, w, 1, debugColor)
		r.DrawText(x+2, curY+2, fmt.Sprintf("pad %.0f", paddingX), labelColor)
		curY += paddingX

		// Title
		r.DrawRect(x, curY, w, 1, debugColor)
		r.DrawText(x+2, curY+2, fmt.Sprintf("title %d", textHeight), labelColor)
		curY += textHeight + sectionGap

		// Message (2 lines)
		r.DrawRect(x, curY, w, 1, debugColor)
		r.DrawText(x+2, curY+2, fmt.Sprintf("msg %d (%dx2+%d)", textHeight*2+lineSpacing, textHeight, lineSpacing), labelColor)
		curY += textHeight*2 + lineSpacing + sectionGap

		// Source/timestamp
		r.DrawRect(x, curY, w, 1, debugColor)
		r.DrawText(x+2, curY+2, fmt.Sprintf("src %d", textHeight), labelColor)
		curY += textHeight + sectionGap

		// Actions (if present)
		if len(n.Actions) > 0 {
			r.DrawRect(x, curY, w, 1, debugColor)
			r.DrawText(x+2, curY+2, fmt.Sprintf("actions %.0f", opts.ActionRowH), labelColor)
			curY += opts.ActionRowH
		}

		// Bottom padding
		r.DrawRect(x, curY, w, 1, debugColor)
		r.DrawText(x+2, curY+2, fmt.Sprintf("pad %.0f", paddingX), labelColor)

		// Bottom edge
		r.DrawRect(x, y+h-1, w, 1, debugColor)
	}

	return isHovered
}

func renderCardActionButtons(r *Renderer, n NotificationCardData, x, y, maxWidth float32, theme NotificationCardTheme, opts NotificationCardOptions) {
	for i, action := range n.Actions {
		if i > 0 {
			x += float32(actionBtnPadding)
		}

		state := ActionIdle
		if opts.GetActionState != nil {
			state = opts.GetActionState(i)
		}

		label := action.Label
		if state == ActionLoading {
			label = "..."
		}

		labelWidth, _ := r.fontAtlas.MeasureText(label)
		btnPadding := float32(12)
		btnWidth := float32(labelWidth) + btnPadding
		if btnWidth > maxWidth {
			btnWidth = maxWidth
		}

		btnBg := theme.ButtonBg
		if pointInRect(opts.MouseX, opts.MouseY, x, y, btnWidth, opts.ButtonH) {
			btnBg = theme.ButtonHv
			if opts.JustClicked && opts.OnAction != nil && state == ActionIdle {
				opts.OnAction(i, action)
			}
		}

		switch state {
		case ActionLoading:
			btnBg = Color{R: 0.3, G: 0.3, B: 0.3, A: 1}
		case ActionSuccess:
			btnBg = Color{R: 0.2, G: 0.6, B: 0.2, A: 1}
		case ActionError:
			btnBg = Color{R: 0.6, G: 0.2, B: 0.2, A: 1}
		}

		r.DrawRect(x, y, btnWidth, opts.ButtonH, btnBg)
		// Center text horizontally and vertically
		textX := x + (btnWidth-float32(labelWidth))/2
		textY := y + (opts.ButtonH-float32(r.fontAtlas.Size))/2
		r.DrawText(textX, textY, label, theme.Title)

		x += btnWidth
		maxWidth -= btnWidth
		if maxWidth <= 0 {
			break
		}
	}
}

func drawMultilineText(r *Renderer, x, y float32, text string, lineSpacing float32, c Color) {
	textHeight := float32(r.fontAtlas.Height)
	curY := y
	start := 0
	for i := 0; i <= len(text); i++ {
		if i == len(text) || text[i] == '\n' {
			if i > start {
				r.DrawText(x, curY, text[start:i], c)
			}
			curY += textHeight + lineSpacing
			start = i + 1
		}
	}
}

func formatSourceLine(source string, createdAt time.Time) string {
	timeAgo := formatTimeAgo(createdAt)
	if source == "" {
		return timeAgo
	}
	return source + " - " + timeAgo
}

func countLines(text string) int {
	if text == "" {
		return 0
	}
	count := 1
	for _, ch := range text {
		if ch == '\n' {
			count++
		}
	}
	return count
}
