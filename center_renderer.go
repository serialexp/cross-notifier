package main

import (
	"fmt"
	"image"
	"image/draw"
	"log"
	"math"
	"strings"
	"sync"
	"time"

	"github.com/go-gl/gl/v2.1/gl"
	"github.com/go-gl/glfw/v3.3/glfw"
)

const (
	centerHeaderHeight   = 40
	centerMinHeight      = 150
	centerScrollSpeed    = 28
	centerDialogWidth    = 220
	centerDialogHeight   = 120
	centerDialogBtnWidth = 80
	centerDialogBtnH     = 26
	centerScrollBarWidth = 6
	centerScrollBarMinH  = 24
	centerScrollBarPad   = 6
	centerAnimationDurMs = 200
)

// CenterRenderer handles rendering of the notification center window.
type CenterRenderer struct {
	renderer      *Renderer
	window        *glfw.Window
	widgets       *WidgetState
	baseURL       string
	store         *NotificationStore
	notifications []CenterNotification
	lastError     string
	lastFetch     time.Time
	refreshEvery  time.Duration
	showConfirm   bool
	scrollOffset  float32
	contentHeight float32
	mouseX        float32
	mouseY        float32
	mouseDown     bool
	prevMouseDown bool
	windowHeight  int
	fps           float64
	fpsLast       time.Time
	fpsFrames     int

	hoveredCards          map[int64]bool
	textures              map[int64]*centerIconTexture
	pendingIcons          map[int64]string
	pendingTextureDeletes []uint32

	// Cached time strings to avoid per-frame allocations
	timeStringCache     map[int64]cachedTimeString
	lastTimeStringFlush time.Time

	// Slide-in animation
	animationStart time.Time
	slideOffset    float32
}

type cachedTimeString struct {
	source    string
	createdAt time.Time
	result    string
	cachedAt  time.Time
}

type centerIconTexture struct {
	img *image.RGBA
	tex uint32
}

// NewCenterRenderer creates a renderer for the notification center.
func NewCenterRenderer(renderer *Renderer, window *glfw.Window, baseURL string, store *NotificationStore) *CenterRenderer {
	cr := &CenterRenderer{
		renderer:            renderer,
		window:              window,
		widgets:             NewWidgetState(),
		baseURL:             baseURL,
		store:               store,
		refreshEvery:        2 * time.Second,
		hoveredCards:        make(map[int64]bool),
		textures:            make(map[int64]*centerIconTexture),
		pendingIcons:        make(map[int64]string),
		timeStringCache:     make(map[int64]cachedTimeString),
		lastTimeStringFlush: time.Now(),
		windowHeight:        centerMinHeight,
		scrollOffset:        0,
		contentHeight:       0,
		animationStart:      time.Now(),
		slideOffset:         float32(centerWidth),
	}

	window.SetScrollCallback(func(w *glfw.Window, xoff, yoff float64) {
		if cr.contentHeight <= cr.listViewportHeight() {
			return
		}
		cr.scrollOffset -= float32(yoff) * centerScrollSpeed
		cr.clampScroll()
	})

	return cr
}

// Render draws the notification center window each frame.
func (cr *CenterRenderer) Render() error {
	frameStart := time.Now()

	// Delete textures that were queued for deletion in previous frames
	cr.processPendingTextureDeletes()

	t0 := time.Now()
	touchCenterPoll()
	cr.updateTheme()
	tTheme := time.Since(t0)

	t0 = time.Now()
	cr.refreshNotifications()
	tRefresh := time.Since(t0)

	t0 = time.Now()
	cr.loadPendingIcons()
	tIcons := time.Since(t0)

	width, height := cr.window.GetSize()
	cr.windowHeight = height
	cr.updateMouseState()

	// Update slide animation
	cr.updateSlideAnimation()

	t0 = time.Now()

	// Draw panel background offset by slide animation
	panelX := cr.slideOffset
	panelWidth := float32(width) - cr.slideOffset
	cr.renderer.DrawRect(panelX, 0, panelWidth, float32(height), currentCenterTheme.windowBg)

	headerBottom := cr.renderHeader(panelX, panelWidth)
	cr.renderFPS(panelX, panelWidth)
	headerBottom = cr.renderError(headerBottom, panelX, panelWidth)
	tHeader := time.Since(t0)

	t0 = time.Now()
	listTop := headerBottom + float32(centerCardPadding)
	cr.renderNotificationList(listTop, panelX, panelWidth, float32(height))
	tList := time.Since(t0)

	if cr.showConfirm {
		cr.renderConfirmOverlay(panelX, panelWidth, float32(height))
	}

	cr.prevMouseDown = cr.mouseDown
	cr.widgets.EndFrame()

	// Log timing every second
	totalFrame := time.Since(frameStart)
	if cr.fpsFrames == 0 {
		log.Printf("Frame timing: total=%v theme=%v refresh=%v icons=%v header=%v list=%v",
			totalFrame, tTheme, tRefresh, tIcons, tHeader, tList)
	}

	return nil
}

func (cr *CenterRenderer) updateSlideAnimation() {
	elapsed := time.Since(cr.animationStart).Milliseconds()
	if elapsed >= centerAnimationDurMs {
		cr.slideOffset = 0
		return
	}

	// Ease-out cubic: 1 - (1-t)^3
	t := float64(elapsed) / float64(centerAnimationDurMs)
	eased := 1 - math.Pow(1-t, 3)
	cr.slideOffset = float32(centerWidth) * float32(1-eased)
}

func (cr *CenterRenderer) renderFPS(panelX, panelWidth float32) {
	cr.updateFPS()
	if cr.fps <= 0 {
		return
	}
	label := fmt.Sprintf("FPS: %.0f", cr.fps)
	labelWidth, _ := cr.renderer.fontAtlas.MeasureText(label)
	x := panelX + panelWidth - float32(centerCardPadding) - float32(labelWidth)
	cr.renderer.DrawText(x, 24, label, currentCenterTheme.mutedText)
}

func (cr *CenterRenderer) updateFPS() {
	now := time.Now()
	if cr.fpsLast.IsZero() {
		cr.fpsLast = now
		cr.fpsFrames = 0
		return
	}
	cr.fpsFrames++
	elapsed := now.Sub(cr.fpsLast)
	if elapsed >= time.Second {
		cr.fps = float64(cr.fpsFrames) / elapsed.Seconds()
		cr.fpsFrames = 0
		cr.fpsLast = now
	}
}

func (cr *CenterRenderer) refreshNotifications() {
	if cr.store != nil {
		if !cr.lastFetch.IsZero() && !consumeCenterDirty() {
			return
		}
		cr.notifications = listCenterNotificationsFromStore(cr.store)
		cr.lastError = ""
		cr.lastFetch = time.Now()
		cr.queuePendingIcons()
		return
	}

	if time.Since(cr.lastFetch) < cr.refreshEvery {
		return
	}
	notifications, errMsg := fetchNotifications(cr.baseURL)
	if errMsg != "" {
		cr.lastError = errMsg
		cr.lastFetch = time.Now()
		return
	}

	cr.notifications = notifications
	cr.lastError = ""
	cr.lastFetch = time.Now()
	cr.queuePendingIcons()
}

func (cr *CenterRenderer) queuePendingIcons() {
	for i := range cr.notifications {
		n := &cr.notifications[i]
		if n.IconData != "" {
			if _, ok := cr.textures[n.ID]; ok {
				continue
			}
			if _, ok := cr.pendingIcons[n.ID]; ok {
				continue
			}
			cr.pendingIcons[n.ID] = n.IconData
		}
	}
}

func (cr *CenterRenderer) updateTheme() {
	if isDarkMode() {
		currentCenterTheme = &centerDarkTheme
	} else {
		currentCenterTheme = &centerLightTheme
	}
}

func (cr *CenterRenderer) updateMouseState() {
	x, y := cr.window.GetCursorPos()
	cr.mouseX = float32(x)
	cr.mouseY = float32(y)
	cr.mouseDown = cr.window.GetMouseButton(glfw.MouseButtonLeft) == glfw.Press
	cr.widgets.UpdateMouse(cr.window)
}

func (cr *CenterRenderer) renderHeader(panelX, panelWidth float32) float32 {
	paddingX := panelX + float32(centerCardPadding)
	titleY := float32(8)

	cr.renderer.DrawText(paddingX, titleY, "Notifications", currentCenterTheme.titleText)

	buttonY := float32(6)
	btnPad := float32(12)
	btnGap := float32(8)
	theme := cr.buttonTheme()

	// Calculate button widths
	closeW := ButtonWidth(cr.renderer, "X", btnPad)
	clearW := ButtonWidth(cr.renderer, "Clear All", btnPad)

	// Position from right edge
	closeBtnX := panelX + panelWidth - float32(centerCardPadding) - closeW

	// Close button (transparent background)
	closeTheme := theme
	closeTheme.Background = Color{R: 0, G: 0, B: 0, A: 0}
	if Button(cr.renderer, cr.widgets, "center_close", closeBtnX, buttonY, 0, cardButtonHeight, "X", closeTheme) {
		cr.window.SetShouldClose(true)
	}

	if len(cr.notifications) > 0 {
		clearBtnX := closeBtnX - btnGap - clearW
		if Button(cr.renderer, cr.widgets, "center_clear", clearBtnX, buttonY, 0, cardButtonHeight, "Clear All", theme) {
			cr.showConfirm = true
		}
	}

	return float32(centerHeaderHeight)
}

func (cr *CenterRenderer) buttonTheme() WidgetTheme {
	return WidgetTheme{
		Background:    currentCenterTheme.buttonBg,
		BackgroundHov: currentCenterTheme.buttonHov,
		Text:          currentCenterTheme.titleText,
		TextMuted:     currentCenterTheme.mutedText,
	}
}

func (cr *CenterRenderer) renderError(startY, panelX, panelWidth float32) float32 {
	if cr.lastError == "" {
		return startY
	}

	lines := cr.wrapLines(cr.lastError, int(panelWidth-float32(centerCardPadding*2)))
	lineHeight := float32(cr.renderer.fontAtlas.Height + 2)
	y := startY

	for _, line := range lines {
		cr.renderer.DrawText(panelX+float32(centerCardPadding), y, line, Color{R: 1, G: 0.4, B: 0.4, A: 1})
		y += lineHeight
	}

	return y + float32(centerCardPadding)
}

func (cr *CenterRenderer) renderNotificationList(startY, panelX, panelWidth, height float32) {
	listHeight := height - startY - float32(centerCardPadding)
	cr.contentHeight = 0

	if len(cr.notifications) == 0 {
		cr.renderer.DrawText(panelX+float32(centerCardPadding), startY, "No notifications", currentCenterTheme.mutedText)
		return
	}

	cardWidth := panelWidth - float32(centerCardPadding*2)
	y := startY - cr.scrollOffset

	for i := len(cr.notifications) - 1; i >= 0; i-- {
		n := cr.notifications[i]
		cardHeight := float32(centerCardHeight)
		if len(n.Actions) > 0 {
			cardHeight += sectionGap + actionRowH
		}

		cr.contentHeight += cardHeight + float32(centerCardPadding)

		if y+cardHeight >= startY && y <= startY+listHeight {
			cr.renderCard(n, panelX+float32(centerCardPadding), y, cardWidth, cardHeight)
		}

		y += cardHeight + float32(centerCardPadding)
	}

	cr.clampScroll()
	if cr.contentHeight > listHeight {
		cr.renderScrollIndicator(startY, listHeight, panelX, panelWidth)
	}
}

func (cr *CenterRenderer) renderScrollIndicator(startY, height, panelX, panelWidth float32) {
	trackX := panelX + panelWidth - float32(centerCardPadding) - float32(centerScrollBarWidth)
	trackY := startY
	trackW := float32(centerScrollBarWidth)
	trackH := height

	maxScroll := float32(math.Max(0, float64(cr.contentHeight-cr.listViewportHeight())))
	if maxScroll <= 0 || trackH <= 0 {
		return
	}

	visibleRatio := trackH / cr.contentHeight
	thumbH := trackH * visibleRatio
	if thumbH < centerScrollBarMinH {
		thumbH = centerScrollBarMinH
	}
	if thumbH > trackH {
		thumbH = trackH
	}

	thumbTravel := trackH - thumbH
	thumbY := trackY
	if thumbTravel > 0 {
		thumbY = trackY + (cr.scrollOffset/maxScroll)*thumbTravel
	}

	trackColor := currentCenterTheme.mutedText
	trackColor.A = 0.15
	thumbColor := currentCenterTheme.mutedText
	thumbColor.A = 0.6

	cr.renderer.DrawRect(trackX, trackY+centerScrollBarPad, trackW, trackH-2*centerScrollBarPad, trackColor)
	cr.renderer.DrawRect(trackX, thumbY, trackW, thumbH, thumbColor)
}

func (cr *CenterRenderer) renderCard(n CenterNotification, x, y, w, h float32) {
	id := n.ID
	theme := NotificationCardTheme{
		CardBg:      currentCenterTheme.cardBg,
		Title:       currentCenterTheme.titleText,
		Body:        currentCenterTheme.bodyText,
		Muted:       currentCenterTheme.mutedText,
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
		SourceLine:  cr.getCachedSourceLine(id, n.Source, n.CreatedAt),
		Actions:     n.Actions,
		IconTexture: cr.iconTexture(id),
		CreatedAt:   n.CreatedAt,
	}
	opts := NotificationCardOptions{
		Padding:            float32(cardPadding),
		IconSize:           iconSize,
		ActionRowH:         actionRowH,
		ButtonH:            cardButtonHeight,
		ShowDismiss:        true,
		ShowSource:         true,
		AllowDismissOnCard: true,
		MouseX:             cr.mouseX,
		MouseY:             cr.mouseY,
		JustClicked:        cr.justClicked(),
		BorderColor:        centerStatusBorderColor(n.Status),
		OnDismiss: func() {
			cr.dismissNotification(id)
		},
		OnAction: func(_ int, action Action) {
			if err := ExecuteAction(action); err != nil {
				cr.lastError = fmt.Sprintf("Action failed: %v", err)
				return
			}
			cr.dismissNotification(id)
		},
	}
	cr.hoveredCards[id] = drawNotificationCard(cr.renderer, data, x, y, w, h, theme, opts)
}

func (cr *CenterRenderer) renderConfirmOverlay(panelX, panelWidth, height float32) {
	// Semi-transparent overlay over the panel
	cr.renderer.DrawRect(panelX, 0, panelWidth, height, Color{R: 0, G: 0, B: 0, A: 0.4})

	// Dialog centered within the panel
	dialogX := panelX + (panelWidth-centerDialogWidth)/2
	dialogY := (height - centerDialogHeight) / 2
	cr.renderer.DrawRect(dialogX, dialogY, centerDialogWidth, centerDialogHeight, currentCenterTheme.cardBg)
	cr.renderer.DrawBorder(dialogX, dialogY, centerDialogWidth, centerDialogHeight, 1, currentCenterTheme.buttonHov)

	titleY := dialogY + 16
	cr.renderer.DrawText(dialogX+12, titleY, "Clear all notifications?", currentCenterTheme.titleText)

	theme := cr.buttonTheme()
	btnPad := float32(12)
	buttonY := dialogY + centerDialogHeight - 40

	// Calculate button widths
	cancelW := ButtonWidth(cr.renderer, "Cancel", btnPad)
	clearW := ButtonWidth(cr.renderer, "Clear", btnPad)

	cancelX := dialogX + 16
	clearX := dialogX + centerDialogWidth - clearW - 16

	if Button(cr.renderer, cr.widgets, "confirm_cancel", cancelX, buttonY, 0, centerDialogBtnH, "Cancel", theme) {
		cr.showConfirm = false
	}

	if Button(cr.renderer, cr.widgets, "confirm_clear", clearX, buttonY, 0, centerDialogBtnH, "Clear", theme) {
		if cr.store != nil {
			ids := listStoredIDs(cr.store)
			for _, id := range ids {
				dismissNotification(id)
			}
			cr.notifications = nil
			cr.lastError = ""
		} else {
			if err := dismissAllNotifications(cr.baseURL); err != "" {
				cr.lastError = err
			} else {
				cr.notifications = nil
				cr.lastError = ""
			}
		}
		cr.showConfirm = false
	}
	_ = cancelW // used for positioning if we add more buttons
}

func (cr *CenterRenderer) wrapLines(text string, maxWidth int) []string {
	if maxWidth <= 0 {
		return nil
	}

	words := strings.Fields(text)
	if len(words) == 0 {
		return nil
	}

	lines := []string{}
	current := words[0]
	for _, word := range words[1:] {
		test := current + " " + word
		width, _ := cr.renderer.fontAtlas.MeasureText(test)
		if width > maxWidth {
			lines = append(lines, current)
			current = word
		} else {
			current = test
		}
	}
	lines = append(lines, current)
	return lines
}

func (cr *CenterRenderer) dismissNotification(id int64) {
	if cr.store != nil {
		dismissNotification(id)
	} else {
		if err := dismissNotificationAPI(cr.baseURL, id); err != "" {
			cr.lastError = err
			return
		}
	}

	for i, notif := range cr.notifications {
		if notif.ID == id {
			cr.notifications = append(cr.notifications[:i], cr.notifications[i+1:]...)
			break
		}
	}
	// Queue texture for deletion next frame to avoid deleting while still in use
	if icon := cr.textures[id]; icon != nil && icon.tex != 0 {
		cr.pendingTextureDeletes = append(cr.pendingTextureDeletes, icon.tex)
	}
	delete(cr.textures, id)
	delete(cr.hoveredCards, id)
}

func (cr *CenterRenderer) processPendingTextureDeletes() {
	if len(cr.pendingTextureDeletes) == 0 {
		return
	}
	for _, tex := range cr.pendingTextureDeletes {
		gl.DeleteTextures(1, &tex)
	}
	cr.pendingTextureDeletes = cr.pendingTextureDeletes[:0]
}

func (cr *CenterRenderer) loadPendingIcons() {
	for id, iconData := range cr.pendingIcons {
		img, err := loadIconFromBase64(iconData)
		if err != nil {
			log.Printf("Failed to decode icon for notification %d: %v", id, err)
			delete(cr.pendingIcons, id)
			continue
		}
		cr.textures[id] = &centerIconTexture{img: imageToRGBA(img)}
		delete(cr.pendingIcons, id)
	}
}

func (cr *CenterRenderer) iconTexture(id int64) uint32 {
	icon := cr.textures[id]
	if icon == nil {
		return 0
	}
	if icon.tex == 0 && icon.img != nil {
		icon.tex = uploadTexture(icon.img)
	}
	return icon.tex
}

func (cr *CenterRenderer) clampScroll() {
	maxScroll := float32(math.Max(0, float64(cr.contentHeight-cr.listViewportHeight())))
	if cr.scrollOffset < 0 {
		cr.scrollOffset = 0
	}
	if cr.scrollOffset > maxScroll {
		cr.scrollOffset = maxScroll
	}
}

// getCachedSourceLine returns a cached source line string, regenerating if stale.
// Cache is invalidated every second to update "X minutes ago" strings.
func (cr *CenterRenderer) getCachedSourceLine(id int64, source string, createdAt time.Time) string {
	now := time.Now()

	// Flush all caches every second
	if now.Sub(cr.lastTimeStringFlush) > time.Second {
		cr.timeStringCache = make(map[int64]cachedTimeString)
		cr.lastTimeStringFlush = now
	}

	cached, ok := cr.timeStringCache[id]
	if ok && cached.source == source && cached.createdAt.Equal(createdAt) {
		return cached.result
	}

	result := formatSourceLine(source, createdAt)
	cr.timeStringCache[id] = cachedTimeString{
		source:    source,
		createdAt: createdAt,
		result:    result,
		cachedAt:  now,
	}
	return result
}

func (cr *CenterRenderer) listViewportHeight() float32 {
	return float32(cr.windowHeight - centerHeaderHeight - centerCardPadding*2)
}

func (cr *CenterRenderer) justClicked() bool {
	return cr.mouseDown && !cr.prevMouseDown
}

func pointInRect(x, y, rx, ry, rw, rh float32) bool {
	return x >= rx && x <= rx+rw && y >= ry && y <= ry+rh
}

func imageToRGBA(img image.Image) *image.RGBA {
	rgba, ok := img.(*image.RGBA)
	if ok {
		return rgba
	}
	bounds := img.Bounds()
	dst := image.NewRGBA(bounds)
	draw.Draw(dst, bounds, img, bounds.Min, draw.Src)
	return dst
}

func centerStatusBorderColor(status string) Color {
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

func listCenterNotificationsFromStore(store *NotificationStore) []CenterNotification {
	if store == nil {
		return nil
	}
	stored := store.List()
	result := make([]CenterNotification, 0, len(stored))
	for _, entry := range stored {
		n, err := entry.ParseNotification()
		if err != nil || n == nil {
			continue
		}
		result = append(result, CenterNotification{
			ID:        n.ID,
			Title:     n.Title,
			Message:   n.Message,
			Status:    n.Status,
			Source:    n.Source,
			IconData:  n.IconData,
			Actions:   n.Actions,
			CreatedAt: n.CreatedAt,
		})
	}
	return result
}

func listStoredIDs(store *NotificationStore) []int64 {
	if store == nil {
		return nil
	}
	stored := store.List()
	ids := make([]int64, 0, len(stored))
	for _, entry := range stored {
		ids = append(ids, entry.ID)
	}
	return ids
}

var centerWindowState struct {
	mu   sync.Mutex
	open bool
}

var centerDirtyState struct {
	mu    sync.Mutex
	dirty bool
}

func markCenterOpen() bool {
	centerWindowState.mu.Lock()
	defer centerWindowState.mu.Unlock()
	if centerWindowState.open {
		return false
	}
	centerWindowState.open = true
	return true
}

func markCenterClosed() {
	centerWindowState.mu.Lock()
	centerWindowState.open = false
	centerWindowState.mu.Unlock()
}

func markCenterDirty() {
	centerDirtyState.mu.Lock()
	centerDirtyState.dirty = true
	centerDirtyState.mu.Unlock()
}

func consumeCenterDirty() bool {
	centerDirtyState.mu.Lock()
	dirty := centerDirtyState.dirty
	centerDirtyState.dirty = false
	centerDirtyState.mu.Unlock()
	return dirty
}
