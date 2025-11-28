// ABOUTME: Cross-platform notification daemon that displays notifications via HTTP API.
// ABOUTME: Listens on a local port and renders notifications in top-right corner.

package main

import (
	"encoding/json"
	"fmt"
	"image/color"
	"log"
	"net/http"
	"sync"
	"time"

	"github.com/AllenDang/cimgui-go/imgui"
	g "github.com/AllenDang/giu"
	"github.com/kbinani/screenshot"
)

const (
	listenPort       = ":9876"
	maxVisible       = 4
	notificationW    = 320
	notificationH    = 80
	padding          = 10
	defaultDurationS = 5
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

	notifications = append(notifications, n)
	g.Update()
}

func dismissNotification(id int64) {
	notifMu.Lock()
	defer notifMu.Unlock()

	for i, n := range notifications {
		if n.ID == id {
			notifications = append(notifications[:i], notifications[i+1:]...)
			g.Update()
			return
		}
	}
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

func loop() {
	// Prune expired notifications each frame
	pruneExpired()

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
	g.PushColorWindowBg(color.RGBA{40, 40, 40, 230})

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
				imgui.TextColored(imgui.Vec4{X: 0.6, Y: 0.6, Z: 0.6, W: 1},
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
	imgui.PushStyleColorVec4(imgui.ColChildBg, imgui.Vec4{X: 0.2, Y: 0.2, Z: 0.25, W: 0.9})

	childFlags := imgui.ChildFlagsNone
	windowFlags := imgui.WindowFlagsNone

	if imgui.BeginChildStrV(fmt.Sprintf("notif_%d", id), imgui.Vec2{X: notificationW - 2*padding, Y: notificationH - padding}, childFlags, windowFlags) {
		// Title
		if n.Title != "" {
			imgui.PushStyleColorVec4(imgui.ColText, imgui.Vec4{X: 1, Y: 1, Z: 1, W: 1})
			imgui.TextWrapped(n.Title)
			imgui.PopStyleColor()
		}

		// Message
		if n.Message != "" {
			imgui.PushStyleColorVec4(imgui.ColText, imgui.Vec4{X: 0.8, Y: 0.8, Z: 0.8, W: 1})
			imgui.TextWrapped(n.Message)
			imgui.PopStyleColor()
		}

		// Click to dismiss
		if imgui.IsWindowHovered() && imgui.IsMouseClickedBool(imgui.MouseButtonLeft) {
			go dismissNotification(id)
		}
	}
	imgui.EndChild()
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
