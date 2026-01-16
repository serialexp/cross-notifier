package main

import (
	"log"
	"math"
	"sync"
	"time"

	"github.com/go-gl/gl/v2.1/gl"
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
	mouseX            float32
	mouseY            float32
	mouseDown         bool
	prevMouseDown     bool

	// Icon texture cache
	iconTextures map[int64]uint32

	// Pending texture deletions (must be processed on main GL thread)
	pendingDeletes   []int64
	pendingDeletesMu sync.Mutex
}

// NewNotificationRenderer creates a notification renderer
func NewNotificationRenderer(renderer *Renderer, window *glfw.Window) *NotificationRenderer {
	return &NotificationRenderer{
		renderer:          renderer,
		window:            window,
		notifications:     []Notification{},
		hoveredCard:       make(map[int64]bool),
		successAnimations: make(map[int64]float32),
		iconTextures:      make(map[int64]uint32),
		lastFrameTime:     time.Now(),
	}
}

// expandedCardWidth returns the width for an expanded card (2x normal)
func expandedCardWidth() float32 {
	return float32(notificationW-2*padding) * 2
}

// expandedCardHeight calculates the height for an expanded notification
func (nr *NotificationRenderer) expandedCardHeight(n Notification, cardWidth float32) float32 {
	// Calculate text area width (card - padding - icon - padding - buttons - padding)
	textAreaWidth := cardWidth - float32(padding) - float32(iconSize) - float32(padding)
	textAreaWidth -= 22 // expand button
	if len(n.Actions) > 0 {
		textAreaWidth -= 22 // dismiss button
	}
	textAreaWidth -= float32(padding)

	// Wrap the message to get line count
	messageWidth := int(textAreaWidth + 22 + 22) // Add button widths back for message area
	wrapped := nr.renderer.fontAtlas.WrapText(n.Message, messageWidth)
	lineCount := countLines(wrapped)
	if lineCount < 2 {
		lineCount = 2 // Minimum 2 lines
	}

	// Calculate height: padding + title + sectionGap + message + sectionGap + source + padding
	messageHeight := float32(lineCount)*textHeight + float32(lineCount-1)*lineSpacing
	height := float32(padding) + textHeight + sectionGap + messageHeight + sectionGap + textHeight + float32(padding)

	if len(n.Actions) > 0 {
		height += sectionGap + actionRowH
	}

	return height
}

// Render renders the notification window
func (nr *NotificationRenderer) Render() error {
	// Process pending texture deletions on the GL thread
	nr.processPendingDeletes()

	updateTheme()
	pruneExpired()
	now := time.Now()
	deltaTime := float32(now.Sub(nr.lastFrameTime).Seconds())
	nr.lastFrameTime = now
	nr.updateMouseState()

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

	// Calculate window size based on stacked card heights.
	maxHeight := float32(0)
	maxWidth := float32(notificationW)
	for i := range visible {
		n := visible[i]
		var cardHeight, cardWidth float32
		if n.Expanded {
			cardWidth = expandedCardWidth() + float32(padding*2)
			cardHeight = nr.expandedCardHeight(n, expandedCardWidth())
		} else {
			scale := 1.0 - float32(i)*0.03
			cardWidth = float32(notificationW-2*padding)*scale + float32(padding*2)
			cardHeight = float32(notificationHeight(n)) * scale
		}
		bottom := float32(i*stackPeek) + cardHeight
		if bottom > maxHeight {
			maxHeight = bottom
		}
		if cardWidth > maxWidth {
			maxWidth = cardWidth
		}
	}
	height := int(math.Ceil(float64(maxHeight))) + padding
	width := int(math.Ceil(float64(maxWidth)))
	nr.window.SetSize(width, height)

	// Position window
	nr.positionWindow()

	// Render notifications back to front
	for i := len(visible) - 1; i >= 0; i-- {
		nr.renderNotificationCard(visible[i], i, len(visible))
	}

	nr.prevMouseDown = nr.mouseDown
	return nil
}

func (nr *NotificationRenderer) renderNotificationCard(n Notification, index int, total int) {
	// Calculate card properties
	scale := 1.0 - float32(index)*0.03
	var cardWidth, cardHeight float32
	if n.Expanded {
		cardWidth = expandedCardWidth()
		cardHeight = nr.expandedCardHeight(n, cardWidth)
	} else {
		cardWidth = float32(notificationW-2*padding) * scale
		cardHeight = float32(notificationHeight(n)) * scale
	}

	// Right-align cards within the window
	windowW, _ := nr.window.GetSize()
	xOffset := float32(windowW) - cardWidth - float32(padding)
	yOffset := float32(index * stackPeek)

	// Card styling
	cardBg := currentTheme.cardBg

	if nr.hoveredCard[n.ID] {
		// Increase opacity when hovered
		cardBg.A = 1
	}

	// Apply success animation tint
	if progress, ok := nr.successAnimations[n.ID]; ok && progress > 0 {
		cardBg.G = float32(math.Min(float64(cardBg.G+0.3*progress), 1))
	}
	theme := NotificationCardTheme{
		CardBg:      cardBg,
		Title:       currentTheme.titleText,
		Body:        currentTheme.bodyText,
		Muted:       currentTheme.moreText,
		ButtonBg:    currentCenterTheme.buttonBg,
		ButtonHv:    currentCenterTheme.buttonHov,
		DismissBg:   currentCenterTheme.dismissBg,
		DismissHv:   currentCenterTheme.dismissHov,
		DismissText: currentCenterTheme.dismissText,
	}
	data := NotificationCardData{
		ID:          n.ID,
		Title:       n.Title,
		Message:     n.Message,
		Status:      n.Status,
		Source:      n.Source,
		Actions:     n.Actions,
		IconTexture: nr.getIconTexture(n),
		CreatedAt:   n.CreatedAt,
		Expanded:    n.Expanded,
	}
	opts := NotificationCardOptions{
		Padding:            float32(cardPadding),
		IconSize:           iconSize,
		ActionRowH:         actionRowH,
		ButtonH:            cardButtonHeight,
		ShowDismiss:        true,
		ShowSource:         true,
		AllowDismissOnCard: true,
		MouseX:             nr.mouseX,
		MouseY:             nr.mouseY,
		JustClicked:        nr.justClicked(),
		BorderColor:        nr.statusBorderColor(n.Status),
		GetActionState: func(idx int) ActionState {
			state := GetActionState(n.ID, idx)
			return state.State
		},
		OnDismiss: func() {
			dismissNotification(n.ID)
		},
		OnAction: func(idx int, action Action) {
			nr.handleActionClick(n, idx, action)
		},
		OnExpand: func() {
			toggleNotificationExpanded(n.ID)
		},
	}
	nr.hoveredCard[n.ID] = drawNotificationCard(nr.renderer, data, xOffset, yOffset, cardWidth, cardHeight, theme, opts)
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

	windowW, _ := nr.window.GetSize()
	margin := 20
	x := videoMode.Width - windowW - margin
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

func (nr *NotificationRenderer) updateMouseState() {
	x, y := nr.window.GetCursorPos()
	nr.mouseX = float32(x)
	nr.mouseY = float32(y)
	nr.mouseDown = nr.window.GetMouseButton(glfw.MouseButtonLeft) == glfw.Press
}

func (nr *NotificationRenderer) justClicked() bool {
	return nr.mouseDown && !nr.prevMouseDown
}

// getIconTexture returns the OpenGL texture ID for a notification's icon, loading it if needed.
func (nr *NotificationRenderer) getIconTexture(n Notification) uint32 {
	// Already cached?
	if tex, ok := nr.iconTextures[n.ID]; ok {
		return tex
	}

	// No icon data?
	if n.IconData == "" {
		return 0
	}

	// Load and cache
	img, err := loadIconFromBase64(n.IconData)
	if err != nil {
		log.Printf("Failed to load icon for notification %d: %v", n.ID, err)
		return 0
	}

	rgba := imageToRGBA(img)
	tex := uploadTexture(rgba)
	nr.iconTextures[n.ID] = tex
	return tex
}

// cleanupIconTexture queues a texture for deletion (safe to call from any goroutine).
func (nr *NotificationRenderer) cleanupIconTexture(id int64) {
	nr.pendingDeletesMu.Lock()
	nr.pendingDeletes = append(nr.pendingDeletes, id)
	nr.pendingDeletesMu.Unlock()
}

// processPendingDeletes deletes queued textures (must be called on GL thread).
func (nr *NotificationRenderer) processPendingDeletes() {
	nr.pendingDeletesMu.Lock()
	toDelete := nr.pendingDeletes
	nr.pendingDeletes = nil
	nr.pendingDeletesMu.Unlock()

	for _, id := range toDelete {
		if tex, ok := nr.iconTextures[id]; ok && tex != 0 {
			gl.DeleteTextures(1, &tex)
			delete(nr.iconTextures, id)
		}
	}
}

func (nr *NotificationRenderer) handleActionClick(n Notification, actionIdx int, action Action) {
	connectedClient := getConnectedClient()
	if n.Exclusive && n.ServerID != "" && connectedClient != nil {
		SetActionState(n.ID, actionIdx, ActionLoading, nil)
		go func() {
			if err := connectedClient.SendAction(n.ServerID, actionIdx); err != nil {
				log.Printf("Failed to send action to server: %v", err)
				SetActionState(n.ID, actionIdx, ActionIdle, nil)
			}
		}()
		return
	}

	ExecuteActionAsync(n.ID, actionIdx, action,
		func() {
			triggerSuccessAnimation(n.ID)
		},
		func(err error) {
			dismissNotification(n.ID)
			addNotification(Notification{
				Title:    "Action Failed",
				Message:  err.Error(),
				Duration: 5,
			})
		},
	)
}

// StartSuccessAnimation starts a success animation for a notification
func (nr *NotificationRenderer) StartSuccessAnimation(id int64) {
	nr.successAnimations[id] = 0.5 // 0.5 second animation
}
