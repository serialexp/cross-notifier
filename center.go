// ABOUTME: Notification center window for viewing and managing stored notifications.
// ABOUTME: Frameless overlay matching the notification popup style.

package main

import (
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"time"

	"github.com/go-gl/glfw/v3.3/glfw"
)

const (
	centerWidth       = 340 // Slightly wider than notification popup
	centerMaxHeight   = 500
	centerCardPadding = cardPadding
)

var (
	centerCardHeight  = int(notificationHeight(Notification{}))
	centerPanelConfig CenterPanelConfig
)

// CenterNotification represents a notification from the daemon's center API.
type CenterNotification struct {
	ID        int64     `json:"id"`
	Title     string    `json:"title"`
	Message   string    `json:"message"`
	Status    string    `json:"status"`
	Source    string    `json:"source"`
	IconData  string    `json:"iconData"`
	Actions   []Action  `json:"actions"`
	CreatedAt time.Time `json:"createdAt"`
}

// centerTheme holds colors for the notification center based on system theme.
type centerTheme struct {
	windowBg    Color
	cardBg      Color
	titleText   Color
	bodyText    Color
	mutedText   Color
	buttonBg    Color
	buttonHov   Color
	dismissBg   Color
	dismissHov  Color
	dismissText Color
}

var (
	centerDarkTheme = centerTheme{
		windowBg:    Color{R: 0.078, G: 0.078, B: 0.098, A: 0.863},
		cardBg:      Color{R: 0.15, G: 0.15, B: 0.17, A: 0.863},
		titleText:   Color{R: 1, G: 1, B: 1, A: 1},
		bodyText:    Color{R: 0.8, G: 0.8, B: 0.8, A: 1},
		mutedText:   Color{R: 0.5, G: 0.5, B: 0.5, A: 1},
		buttonBg:    Color{R: 0.25, G: 0.25, B: 0.28, A: 1},
		buttonHov:   Color{R: 0.35, G: 0.35, B: 0.38, A: 1},
		dismissBg:   Color{R: 0.5, G: 0.3, B: 0.3, A: 0.863},
		dismissHov:  Color{R: 0.7, G: 0.3, B: 0.3, A: 0.863},
		dismissText: Color{R: 1, G: 1, B: 1, A: 1},
	}
	centerLightTheme = centerTheme{
		windowBg:    Color{R: 0.941, G: 0.941, B: 0.949, A: 0.941},
		cardBg:      Color{R: 0.98, G: 0.98, B: 0.98, A: 0.941},
		titleText:   Color{R: 0.1, G: 0.1, B: 0.1, A: 1},
		bodyText:    Color{R: 0.3, G: 0.3, B: 0.3, A: 1},
		mutedText:   Color{R: 0.5, G: 0.5, B: 0.5, A: 1},
		buttonBg:    Color{R: 0.85, G: 0.85, B: 0.87, A: 1},
		buttonHov:   Color{R: 0.75, G: 0.75, B: 0.78, A: 1},
		dismissBg:   Color{R: 0.9, G: 0.9, B: 0.9, A: 0.941},
		dismissHov:  Color{R: 0.8, G: 0.2, B: 0.2, A: 0.941},
		dismissText: Color{R: 0.8, G: 0.2, B: 0.2, A: 1},
	}
	currentCenterTheme = &centerDarkTheme
)

// ShowCenterWindow displays the notification center as a frameless overlay.
func ShowCenterWindow(daemonPort string) error {
	if !markCenterOpen() {
		return fmt.Errorf("notification center already open")
	}
	defer markCenterClosed()

	// Refresh theme when opening center
	forceRefreshTheme()

	var baseURL string
	store := notificationStore
	if store == nil {
		baseURL = fmt.Sprintf("http://localhost:%s", daemonPort)
	}

	wm := NewWindowManager()
	centerWnd, err := wm.CreateCenterWindow()
	if err != nil {
		return err
	}
	positionCenterWindow(centerWnd)

	mw := wm.GetManagedWindow(centerWnd)
	if mw == nil {
		return fmt.Errorf("failed to create managed center window")
	}

	centerRenderer := NewCenterRenderer(mw.Renderer, centerWnd, baseURL, store)
	wm.SetWindowRenderCallback(centerWnd, centerRenderer.Render)
	wm.SetWindowCloseCallback(centerWnd, func() {
		clearCenterPoll()
		if baseURL != "" {
			signalCenterClosed(baseURL)
		}
	})

	return wm.Run(func() error { return nil })
}

// OpenCenterWindow launches the notification center window in-process.
func OpenCenterWindow(daemonPort string) {
	if centerOpenCh == nil {
		go func() {
			if err := ShowCenterWindow(daemonPort); err != nil {
				log.Printf("Notification center error: %v", err)
			}
		}()
		return
	}

	select {
	case centerOpenCh <- daemonPort:
	default:
	}
}

func openCenterWindowInProcess(wm *WindowManager, daemonPort string) error {
	if !markCenterOpen() {
		return nil
	}

	// Refresh theme when opening center
	forceRefreshTheme()

	var baseURL string
	store := notificationStore
	if store == nil {
		baseURL = fmt.Sprintf("http://localhost:%s", daemonPort)
	}

	centerWnd, err := wm.CreateCenterWindow()
	if err != nil {
		markCenterClosed()
		return err
	}
	positionCenterWindow(centerWnd)

	mw := wm.GetManagedWindow(centerWnd)
	if mw == nil {
		markCenterClosed()
		return fmt.Errorf("failed to create managed center window")
	}

	centerRenderer := NewCenterRenderer(mw.Renderer, centerWnd, baseURL, store)
	wm.SetWindowRenderCallback(centerWnd, centerRenderer.Render)
	wm.SetWindowCloseCallback(centerWnd, func() {
		clearCenterPoll()
		markCenterClosed()
		if baseURL != "" {
			signalCenterClosed(baseURL)
		}
	})

	return nil
}

// positionCenterWindow positions the window at the right edge spanning the configured area.
func positionCenterWindow(wnd *glfw.Window) {
	monitor := glfw.GetPrimaryMonitor()
	if monitor == nil {
		wnd.SetPos(100, 100)
		return
	}

	videoMode := monitor.GetVideoMode()
	if videoMode == nil {
		wnd.SetPos(100, 100)
		return
	}

	// GetWorkarea returns usable area excluding menu bar and dock
	workX, workY, workW, workH := monitor.GetWorkarea()

	var x, y, height int

	if centerPanelConfig.RespectWorkAreaTop {
		y = workY
	} else {
		y = 0
	}

	if centerPanelConfig.RespectWorkAreaBottom {
		// Use work area height (accounts for dock)
		height = workY + workH - y
		x = workX + workW - centerWidth
	} else {
		// Extend to full screen height
		height = videoMode.Height - y
		x = videoMode.Width - centerWidth
	}

	wnd.SetSize(centerWidth, height)
	wnd.SetPos(x, y)
}

// formatTimeAgo returns a human-readable relative time string.
func formatTimeAgo(t time.Time) string {
	d := time.Since(t)
	switch {
	case d < time.Minute:
		return "just now"
	case d < time.Hour:
		mins := int(d.Minutes())
		if mins == 1 {
			return "1 minute ago"
		}
		return fmt.Sprintf("%d minutes ago", mins)
	case d < 24*time.Hour:
		hours := int(d.Hours())
		if hours == 1 {
			return "1 hour ago"
		}
		return fmt.Sprintf("%d hours ago", hours)
	default:
		days := int(d.Hours() / 24)
		if days == 1 {
			return "1 day ago"
		}
		return fmt.Sprintf("%d days ago", days)
	}
}

// fetchNotifications retrieves notifications from the daemon.
func fetchNotifications(baseURL string) ([]CenterNotification, string) {
	resp, err := http.Get(baseURL + "/center")
	if err != nil {
		return nil, fmt.Sprintf("Failed to connect: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Sprintf("Server error: %d", resp.StatusCode)
	}

	var notifications []CenterNotification
	if err := json.NewDecoder(resp.Body).Decode(&notifications); err != nil {
		return nil, fmt.Sprintf("Failed to parse response: %v", err)
	}

	return notifications, ""
}

// dismissNotificationAPI calls the daemon to dismiss a single notification.
func dismissNotificationAPI(baseURL string, id int64) string {
	req, err := http.NewRequest(http.MethodDelete, fmt.Sprintf("%s/center/%d", baseURL, id), nil)
	if err != nil {
		return fmt.Sprintf("Failed to create request: %v", err)
	}

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return fmt.Sprintf("Failed to dismiss: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Sprintf("Failed to dismiss: %d", resp.StatusCode)
	}

	return ""
}

// dismissAllNotifications calls the daemon to dismiss all notifications.
func dismissAllNotifications(baseURL string) string {
	req, err := http.NewRequest(http.MethodDelete, baseURL+"/center?confirm=true", nil)
	if err != nil {
		return fmt.Sprintf("Failed to create request: %v", err)
	}

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return fmt.Sprintf("Failed to dismiss all: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Sprintf("Failed to dismiss all: %d", resp.StatusCode)
	}

	log.Println("All notifications dismissed")
	return ""
}

// signalCenterClosed notifies the daemon that the center window has closed.
func signalCenterClosed(baseURL string) {
	req, err := http.NewRequest(http.MethodPost, baseURL+"/center/close", nil)
	if err != nil {
		return
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return
	}
	resp.Body.Close()
}
