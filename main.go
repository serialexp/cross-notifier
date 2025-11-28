// ABOUTME: Cross-platform notification daemon that displays notifications via HTTP API.
// ABOUTME: Listens on a local port and renders notifications in top-right corner.

package main

import (
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	_ "image/jpeg"
	_ "image/png"
	"log"
	"net/http"
	"os"
	"sync"
	"time"

	"github.com/AllenDang/cimgui-go/imgui"
	g "github.com/AllenDang/giu"
	"github.com/kbinani/screenshot"
	xdraw "golang.org/x/image/draw"
)

const (
	listenPort       = ":9876"
	maxVisible       = 4
	notificationW    = 320
	notificationH    = 80
	iconSize         = 48
	padding          = 10
	defaultDurationS = 5
)

// Theme colors
type theme struct {
	windowBg  color.RGBA
	cardBg    imgui.Vec4
	titleText imgui.Vec4
	bodyText  imgui.Vec4
	moreText  imgui.Vec4
}

var (
	darkTheme = theme{
		windowBg:  color.RGBA{0, 0, 0, 0}, // transparent window, cards provide the background
		cardBg:    imgui.Vec4{X: 0.15, Y: 0.15, Z: 0.17, W: 0.85},
		titleText: imgui.Vec4{X: 1, Y: 1, Z: 1, W: 1},
		bodyText:  imgui.Vec4{X: 0.8, Y: 0.8, Z: 0.8, W: 1},
		moreText:  imgui.Vec4{X: 0.6, Y: 0.6, Z: 0.6, W: 1},
	}
	lightTheme = theme{
		windowBg:  color.RGBA{0, 0, 0, 0}, // transparent window, cards provide the background
		cardBg:    imgui.Vec4{X: 0.96, Y: 0.96, Z: 0.96, W: 0.85},
		titleText: imgui.Vec4{X: 0.1, Y: 0.1, Z: 0.1, W: 1},
		bodyText:  imgui.Vec4{X: 0.3, Y: 0.3, Z: 0.3, W: 1},
		moreText:  imgui.Vec4{X: 0.5, Y: 0.5, Z: 0.5, W: 1},
	}
	currentTheme = &darkTheme
)

type Notification struct {
	ID        int64     `json:"-"`
	Title     string    `json:"title"`
	Message   string    `json:"message"`
	Icon      string    `json:"icon,omitempty"` // base64 or URL (not yet implemented)
	Duration  int       `json:"duration,omitempty"` // seconds, 0 = default
	CreatedAt time.Time `json:"-"`
	ExpiresAt time.Time `json:"-"`
}

var (
	wnd           *g.MasterWindow
	notifications []Notification
	notifMu       sync.Mutex
	nextID        int64 = 1

	// Texture management
	textures     = make(map[int64]*g.Texture)
	pendingIcons = make(map[int64]string) // notification ID -> icon path
	textureMu    sync.Mutex
)

func addNotification(n Notification) {
	notifMu.Lock()
	defer notifMu.Unlock()

	n.ID = nextID
	nextID++
	n.CreatedAt = time.Now()

	duration := n.Duration
	if duration <= 0 {
		duration = defaultDurationS
	}
	n.ExpiresAt = n.CreatedAt.Add(time.Duration(duration) * time.Second)

	// Queue icon loading if specified
	if n.Icon != "" {
		textureMu.Lock()
		pendingIcons[n.ID] = n.Icon
		textureMu.Unlock()
	}

	notifications = append(notifications, n)
	g.Update()
}

func dismissNotification(id int64) {
	notifMu.Lock()
	defer notifMu.Unlock()

	for i, n := range notifications {
		if n.ID == id {
			notifications = append(notifications[:i], notifications[i+1:]...)
			cleanupTexture(id)
			g.Update()
			return
		}
	}
}

func cleanupTexture(id int64) {
	textureMu.Lock()
	defer textureMu.Unlock()
	delete(textures, id)
	delete(pendingIcons, id)
}

func loadPendingIcons() {
	textureMu.Lock()
	pending := make(map[int64]string)
	for id, path := range pendingIcons {
		pending[id] = path
	}
	textureMu.Unlock()

	for id, path := range pending {
		img, err := loadImage(path)
		if err != nil {
			log.Printf("Failed to load icon %s: %v", path, err)
			textureMu.Lock()
			delete(pendingIcons, id)
			textureMu.Unlock()
			continue
		}

		notifID := id // capture for closure
		g.EnqueueNewTextureFromRgba(img, func(tex *g.Texture) {
			textureMu.Lock()
			textures[notifID] = tex
			delete(pendingIcons, notifID)
			textureMu.Unlock()
		})
	}
}

func loadImage(path string) (image.Image, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	img, _, err := image.Decode(f)
	if err != nil {
		return nil, err
	}

	// Scale down to iconSize for better quality
	return scaleImage(img, iconSize, iconSize), nil
}

func scaleImage(src image.Image, width, height int) image.Image {
	srcBounds := src.Bounds()
	if srcBounds.Dx() <= width && srcBounds.Dy() <= height {
		return src // no scaling needed
	}

	dst := image.NewRGBA(image.Rect(0, 0, width, height))
	xdraw.CatmullRom.Scale(dst, dst.Bounds(), src, srcBounds, xdraw.Over, nil)
	return dst
}

func pruneExpired() {
	notifMu.Lock()
	defer notifMu.Unlock()

	now := time.Now()
	changed := false
	filtered := notifications[:0]
	for _, n := range notifications {
		if now.Before(n.ExpiresAt) {
			filtered = append(filtered, n)
		} else {
			cleanupTexture(n.ID)
			changed = true
		}
	}
	notifications = filtered

	if changed {
		g.Update()
	}
}

func httpHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST required", http.StatusMethodNotAllowed)
		return
	}

	var n Notification
	if err := json.NewDecoder(r.Body).Decode(&n); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	if n.Title == "" && n.Message == "" {
		http.Error(w, "title or message required", http.StatusBadRequest)
		return
	}

	addNotification(n)
	w.WriteHeader(http.StatusAccepted)
}

func startHTTPServer() {
	http.HandleFunc("/notify", httpHandler)
	log.Printf("Listening on http://localhost%s/notify", listenPort)
	if err := http.ListenAndServe(listenPort, nil); err != nil {
		log.Fatalf("HTTP server failed: %v", err)
	}
}

func updateTheme() {
	if isDarkMode() {
		currentTheme = &darkTheme
	} else {
		currentTheme = &lightTheme
	}
}

func loop() {
	// Update theme based on OS setting
	updateTheme()

	// Prune expired notifications each frame
	pruneExpired()

	// Load any pending icons (must happen on main thread)
	loadPendingIcons()

	// Update window size on main thread
	updateWindowSize()

	notifMu.Lock()
	visible := notifications
	if len(visible) > maxVisible {
		visible = visible[:maxVisible]
	}
	notifMu.Unlock()

	if len(visible) == 0 {
		// Nothing to show - render empty
		return
	}

	imgui.PushStyleVarFloat(imgui.StyleVarWindowBorderSize, 0)
	imgui.PushStyleVarFloat(imgui.StyleVarWindowRounding, 8)
	g.PushColorWindowBg(currentTheme.windowBg)

	g.SingleWindow().Layout(
		g.Custom(func() {
			for i, n := range visible {
				renderNotification(n, i)
			}

			// Show count of hidden notifications
			notifMu.Lock()
			hiddenCount := len(notifications) - len(visible)
			notifMu.Unlock()

			if hiddenCount > 0 {
				imgui.Spacing()
				imgui.TextColored(currentTheme.moreText,
					fmt.Sprintf("+ %d more notification(s)...", hiddenCount))
			}
		}),
	)

	g.PopStyleColor()
	imgui.PopStyleVar()
	imgui.PopStyleVar()
}

func renderNotification(n Notification, index int) {
	id := n.ID

	// Card background
	imgui.PushStyleColorVec4(imgui.ColChildBg, currentTheme.cardBg)

	childFlags := imgui.ChildFlagsNone
	windowFlags := imgui.WindowFlagsNone

	// Card styling
	imgui.PushStyleVarFloat(imgui.StyleVarChildRounding, 6)

	if imgui.BeginChildStrV(fmt.Sprintf("notif_%d", id), imgui.Vec2{X: notificationW - 2*padding, Y: notificationH - padding}, childFlags, windowFlags) {
		// Manual padding inside card
		imgui.SetCursorPos(imgui.Vec2{X: 10, Y: 8})

		// Check for icon texture
		textureMu.Lock()
		tex := textures[id]
		textureMu.Unlock()

		if tex != nil {
			// Layout: icon on left (vertically centered), text on right
			contentHeight := notificationH - padding
			iconOffset := float32(contentHeight-iconSize) / 2
			if iconOffset > 0 {
				imgui.SetCursorPosY(imgui.CursorPosY() + iconOffset)
			}
			imgui.Image(tex.ID(), imgui.Vec2{X: iconSize, Y: iconSize})
			imgui.SameLineV(0, 10)
			imgui.SetCursorPosY(imgui.CursorPosY() - iconOffset) // reset for text
			imgui.BeginGroup()
		}

		// Title
		if n.Title != "" {
			imgui.PushStyleColorVec4(imgui.ColText, currentTheme.titleText)
			imgui.TextWrapped(n.Title)
			imgui.PopStyleColor()
		}

		// Message
		if n.Message != "" {
			imgui.PushStyleColorVec4(imgui.ColText, currentTheme.bodyText)
			imgui.TextWrapped(n.Message)
			imgui.PopStyleColor()
		}

		if tex != nil {
			imgui.EndGroup()
		}

		// Click to dismiss
		if imgui.IsWindowHovered() && imgui.IsMouseClickedBool(imgui.MouseButtonLeft) {
			go dismissNotification(id)
		}
	}
	imgui.EndChild()
	imgui.PopStyleVar() // ChildRounding
	imgui.PopStyleColor()

	// Spacing between notifications
	if index < maxVisible-1 {
		imgui.Spacing()
	}
}

func updateWindowSize() {
	notifMu.Lock()
	count := len(notifications)
	hasMore := count > maxVisible
	if count > maxVisible {
		count = maxVisible
	}
	notifMu.Unlock()

	// Hide window when no notifications, show when there are some
	if count == 0 {
		wnd.SetSize(1, 1)
		wnd.SetPos(-100, -100)
		return
	}

	// Reposition when showing notifications
	positionWindow()

	height := count*notificationH + padding
	if hasMore {
		height += 30
	}

	wnd.SetSize(notificationW, height)
}

func positionWindow() {
	// Get primary display bounds
	bounds := screenshot.GetDisplayBounds(0)

	// Position in top-right corner with some margin
	margin := 20
	x := bounds.Max.X - notificationW - margin
	y := bounds.Min.Y + margin

	wnd.SetPos(x, y)
}

func main() {
	wnd = g.NewMasterWindow(
		"Notifications",
		notificationW, notificationH,
		g.MasterWindowFlagsFloating|
			g.MasterWindowFlagsFrameless|
			g.MasterWindowFlagsTransparent,
	)

	wnd.SetBgColor(color.RGBA{0, 0, 0, 0})

	// Start HTTP server in background
	go startHTTPServer()

	// Periodically trigger redraws for expiration checks
	go func() {
		ticker := time.NewTicker(500 * time.Millisecond)
		for range ticker.C {
			g.Update()
		}
	}()

	positionWindow()
	wnd.Run(loop)
}
