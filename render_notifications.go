package main

import (
	"fmt"
	"image"
	"image/color"
	"log"
	"math"
	"time"

	"github.com/go-gl/glfw/v3.3/glfw"
)

// NotificationRenderer handles rendering of notification windows
type NotificationRenderer struct {
	renderer          *Renderer
	window            *glfw.Window
	notifications     []Notification
	hoveredCard       map[int64]bool
	successAnimations map[int64]float32
	lastFrameTime     time.Time
}

// NewNotificationRenderer creates a notification renderer
func NewNotificationRenderer(renderer *Renderer, window *glfw.Window) *NotificationRenderer {
	return &NotificationRenderer{
		renderer:          renderer,
		window:            window,
		notifications:     []Notification{},
		hoveredCard:       make(map[int64]bool),
		successAnimations: make(map[int64]float32),
		lastFrameTime:     time.Now(),
	}
}

// Render renders the notification window
func (nr *NotificationRenderer) Render() error {
	now := time.Now()
	deltaTime := float32(now.Sub(nr.lastFrameTime).Seconds())
	nr.lastFrameTime = now

	// Get visible notifications
	notifMu.Lock()
	visible := make([]Notification, len(notifications))
	copy(visible, notifications)
	if len(visible) > maxVisible {
		visible = visible[:maxVisible]
	}
	notifMu.Unlock()

	// Hide window if no notifications
	if len(visible) == 0 {
		nr.window.SetSize(1, 1)
		nr.window.SetPos(-100, -100)
		return nil
	}

	// Update animations
	nr.updateAnimations(deltaTime)

	// Calculate window size
	height := notificationH + (len(visible)-1)*stackPeek + padding
	nr.window.SetSize(notificationW, height)

	// Position window
	nr.positionWindow()

	// Render notifications back to front
	for i := len(visible) - 1; i >= 0; i-- {
		nr.renderNotificationCard(visible[i], i, len(visible))
	}

	return nil
}

func (nr *NotificationRenderer) renderNotificationCard(n Notification, index int, total int) {
	// Calculate card properties
	scale := 1.0 - float32(index)*0.03
	cardWidth := float32(notificationW-2*padding) * scale
	cardHeight := (float32(notificationHeight(n)) - float32(padding)) * scale

	xOffset := float32((notificationW-2*padding)-int(cardWidth)) / 2
	yOffset := float32(index * stackPeek)

	// Card styling
	cardBg := color.RGBA{R: 38, G: 38, Z: 43, A: 230} // Dark theme default
	if currentTheme.cardBg.W > 0.5 {
		// Light theme
		cardBg = color.RGBA{R: 245, G: 245, Z: 245, A: 230}
	}

	if nr.hoveredCard[n.ID] {
		// Increase opacity when hovered
		cardBg.A = 255
	}

	// Apply success animation tint
	if progress, ok := nr.successAnimations[n.ID]; ok && progress > 0 {
		greenTint := uint8(76 * progress) // 0x4C * progress
		cardBg.G = uint8(math.Min(float64(cardBg.G)+float64(greenTint), 255))
	}

	// Render card background (rounded rectangle as filled rect for now)
	bgColor := ColorRGBA(cardBg)
	nr.renderer.DrawRect(xOffset, yOffset, cardWidth, cardHeight, bgColor)

	// Render border
	borderColor := nr.statusBorderColor(n.Status)
	nr.renderer.DrawBorder(xOffset, yOffset, cardWidth, cardHeight, 2, borderColor)

	// Render icon if present
	textStartX := float32(padding)
	if _, ok := textures[n.ID]; ok {
		if img, ok := textures[n.ID].(*image.RGBA); ok {
			nr.renderer.DrawImage(
				xOffset+float32(padding),
				yOffset+float32(padding),
				float32(iconSize),
				float32(iconSize),
				img,
				Color{R: 1, G: 1, B: 1, A: 1},
			)
			textStartX = float32(padding + iconSize + padding)
		}
	}

	// Calculate text area
	textAreaWidth := cardWidth - textStartX - float32(padding)
	if len(n.Actions) > 0 {
		textAreaWidth -= 30 // Space for dismiss button
	}

	// Render title
	if n.Title != "" {
		titleColor := Color{R: 1, G: 1, B: 1, A: 1}
		if currentTheme.titleText.W > 0 {
			titleColor = Color{
				R: currentTheme.titleText.X,
				G: currentTheme.titleText.Y,
				B: currentTheme.titleText.Z,
				A: currentTheme.titleText.W,
			}
		}

		truncatedTitle := nr.renderer.fontAtlas.Truncate(n.Title, int(textAreaWidth))
		nr.renderer.DrawText(
			xOffset+textStartX,
			yOffset+float32(padding),
			truncatedTitle,
			titleColor,
		)
	}

	// Render message
	if n.Message != "" {
		msgColor := Color{R: 0.8, G: 0.8, B: 0.8, A: 1}
		if currentTheme.bodyText.W > 0 {
			msgColor = Color{
				R: currentTheme.bodyText.X,
				G: currentTheme.bodyText.Y,
				B: currentTheme.bodyText.Z,
				A: currentTheme.bodyText.W,
			}
		}

		truncatedMsg := nr.renderer.fontAtlas.TruncateLines(
			n.Message,
			int(textAreaWidth),
			2,
		)
		nr.renderer.DrawText(
			xOffset+textStartX,
			yOffset+float32(padding+32),
			truncatedMsg,
			msgColor,
		)
	}

	// Render action buttons
	if len(n.Actions) > 0 {
		nr.renderActionButtons(n, xOffset, yOffset, cardWidth, cardHeight)
		nr.renderDismissButton(n, xOffset, yOffset, cardWidth)
	}

	// Track hover state
	// TODO: Implement mouse position checking for hover detection
}

func (nr *NotificationRenderer) renderActionButtons(n Notification, x, y, w, h float32) {
	// Button rendering logic
	buttonY := y + h - float32(actionRowH) + float32(padding)
	buttonX := x + float32(padding)

	for i, action := range n.Actions {
		if i > 0 {
			buttonX += float32(actionBtnPadding) + 80 // Approximate button width
		}

		// Render button background
		btnBg := Color{R: 0.25, G: 0.25, B: 0.28, A: 1}
		nr.renderer.DrawRect(buttonX, buttonY, 80, 24, btnBg)

		// Render button text
		btnText := Color{R: 1, G: 1, B: 1, A: 1}
		nr.renderer.DrawText(buttonX+4, buttonY+4, action.Label, btnText)
	}
}

func (nr *NotificationRenderer) renderDismissButton(n Notification, x, y, w float32) {
	dismissX := x + w - 25
	dismissY := y + float32(padding)

	// Render X button
	btnBg := Color{R: 0.5, G: 0.3, B: 0.3, A: 0.5}
	nr.renderer.DrawRect(dismissX, dismissY, 20, 20, btnBg)

	btnText := Color{R: 1, G: 1, B: 1, A: 1}
	nr.renderer.DrawText(dismissX+6, dismissY+3, "×", btnText)
}

func (nr *NotificationRenderer) statusBorderColor(status string) Color {
	switch status {
	case "success":
		return Color{R: 0.2, G: 0.7, B: 0.3, A: 0.9}
	case "warning":
		return Color{R: 0.9, G: 0.6, B: 0.2, A: 0.9}
	case "error":
		return Color{R: 0.8, G: 0.2, B: 0.2, A: 0.9}
	default:
		return Color{R: 0.3, G: 0.3, B: 0.3, A: 0.8}
	}
}

func (nr *NotificationRenderer) positionWindow() {
	monitor := glfw.GetPrimaryMonitor()
	if monitor == nil {
		nr.window.SetPos(100, 100)
		return
	}

	videoMode := monitor.GetVideoMode()
	if videoMode == nil {
		nr.window.SetPos(100, 100)
		return
	}

	margin := 20
	x := videoMode.Width - notificationW - margin
	y := margin

	nr.window.SetPos(x, y)
}

func (nr *NotificationRenderer) updateAnimations(deltaTime float32) {
	for id, progress := range nr.successAnimations {
		// Fade out success animation
		newProgress := progress - deltaTime
		if newProgress <= 0 {
			delete(nr.successAnimations, id)
		} else {
			nr.successAnimations[id] = newProgress
		}
	}
}

// StartSuccessAnimation starts a success animation for a notification
func (nr *NotificationRenderer) StartSuccessAnimation(id int64) {
	nr.successAnimations[id] = 0.5 // 0.5 second animation
}
