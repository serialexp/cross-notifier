// ABOUTME: Notification center window for viewing and managing stored notifications.
// ABOUTME: Frameless overlay matching the notification popup style.

package main

import (
	"encoding/json"
	"fmt"
	"image/color"
	"log"
	"net/http"
	"time"

	"github.com/AllenDang/cimgui-go/imgui"
	g "github.com/AllenDang/giu"
	"github.com/go-gl/glfw/v3.3/glfw"
)

const (
	centerWidth       = 340 // Slightly wider than notification popup
	centerMaxHeight   = 500
	centerCardHeight  = 90
	centerCardPadding = 10
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

// Center-specific state
var (
	centerHoveredCards = make(map[int64]bool)
	centerTextures     = make(map[int64]*g.Texture)
	centerPendingIcons = make(map[int64]string) // ID -> base64 iconData
)

// centerTheme holds colors for the notification center based on system theme.
type centerTheme struct {
	windowBg  color.RGBA
	cardBg    imgui.Vec4
	titleText imgui.Vec4
	bodyText  imgui.Vec4
	mutedText imgui.Vec4
	buttonBg  imgui.Vec4
	buttonHov imgui.Vec4
}

var (
	centerDarkTheme = centerTheme{
		windowBg:  color.RGBA{R: 20, G: 20, B: 25, A: 220},
		cardBg:    imgui.Vec4{X: 0.15, Y: 0.15, Z: 0.17, W: 0.6}, // Lower opacity, goes to 1.0 on hover
		titleText: imgui.Vec4{X: 1, Y: 1, Z: 1, W: 1},
		bodyText:  imgui.Vec4{X: 0.8, Y: 0.8, Z: 0.8, W: 1},
		mutedText: imgui.Vec4{X: 0.5, Y: 0.5, Z: 0.5, W: 1},
		buttonBg:  imgui.Vec4{X: 0.25, Y: 0.25, Z: 0.28, W: 1},
		buttonHov: imgui.Vec4{X: 0.35, Y: 0.35, Z: 0.38, W: 1},
	}
	centerLightTheme = centerTheme{
		windowBg:  color.RGBA{R: 240, G: 240, B: 242, A: 240},
		cardBg:    imgui.Vec4{X: 0.98, Y: 0.98, Z: 0.98, W: 0.6}, // Lower opacity, goes to 1.0 on hover
		titleText: imgui.Vec4{X: 0.1, Y: 0.1, Z: 0.1, W: 1},
		bodyText:  imgui.Vec4{X: 0.3, Y: 0.3, Z: 0.3, W: 1},
		mutedText: imgui.Vec4{X: 0.5, Y: 0.5, Z: 0.5, W: 1},
		buttonBg:  imgui.Vec4{X: 0.85, Y: 0.85, Z: 0.87, W: 1},
		buttonHov: imgui.Vec4{X: 0.75, Y: 0.75, Z: 0.78, W: 1},
	}
	currentCenterTheme = &centerDarkTheme
)

// ShowCenterWindow displays the notification center as a frameless overlay.
func ShowCenterWindow(daemonPort string) {
	baseURL := fmt.Sprintf("http://localhost:%s", daemonPort)

	var notifications []CenterNotification
	var lastError string
	var showDismissAllConfirm bool
	var centerWnd *g.MasterWindow

	// Initial fetch
	notifications, lastError = fetchNotifications(baseURL)

	// Set theme based on system
	if isDarkMode() {
		currentCenterTheme = &centerDarkTheme
	} else {
		currentCenterTheme = &centerLightTheme
	}

	// Calculate initial height based on notifications
	calcHeight := func() int {
		h := 60 // Header + padding
		h += len(notifications) * (centerCardHeight + centerCardPadding)
		if h < 150 {
			h = 150
		}
		if h > centerMaxHeight {
			h = centerMaxHeight
		}
		return h
	}

	centerWnd = g.NewMasterWindow(
		"Notification Center",
		centerWidth, calcHeight(),
		g.MasterWindowFlagsFloating|
			g.MasterWindowFlagsFrameless|
			g.MasterWindowFlagsTransparent,
	)
	centerWnd.SetBgColor(color.RGBA{0, 0, 0, 0})

	// Load custom font
	g.Context.FontAtlas.SetDefaultFontFromBytes(hackFont, 14)

	// Position in top-right corner
	positionCenterWindow(centerWnd)

	// Auto-refresh ticker
	var needsResize bool
	refreshTicker := time.NewTicker(2 * time.Second)
	go func() {
		for range refreshTicker.C {
			newNotifs, err := fetchNotifications(baseURL)
			if err == "" {
				// Only trigger resize if count changed
				if len(newNotifs) != len(notifications) {
					needsResize = true
				}
				notifications = newNotifs
				lastError = ""
			}
			// Update theme in case system changed
			if isDarkMode() {
				currentCenterTheme = &centerDarkTheme
			} else {
				currentCenterTheme = &centerLightTheme
			}
			g.Update()
		}
	}()

	centerWnd.Run(func() {
		// Handle resize from refresh (must be done in main loop)
		if needsResize {
			centerWnd.SetSize(centerWidth, calcHeight())
			positionCenterWindow(centerWnd) // Re-anchor to top-right
			needsResize = false
		}

		// Load any pending icons
		loadCenterPendingIcons()

		// Queue icons for notifications that need them
		for i := range notifications {
			n := &notifications[i]
			if n.IconData != "" {
				if _, hasTexture := centerTextures[n.ID]; !hasTexture {
					if _, pending := centerPendingIcons[n.ID]; !pending {
						centerPendingIcons[n.ID] = n.IconData
					}
				}
			}
		}

		// Semi-transparent backdrop
		imgui.PushStyleVarFloat(imgui.StyleVarWindowBorderSize, 0)
		imgui.PushStyleVarFloat(imgui.StyleVarWindowRounding, 8)
		g.PushColorWindowBg(currentCenterTheme.windowBg)

		g.SingleWindow().Layout(
			// Fixed header row with padding
			g.Custom(func() {
				// Add top padding
				imgui.SetCursorPosY(imgui.CursorPosY() + 4)

				imgui.PushStyleColorVec4(imgui.ColText, currentCenterTheme.titleText)
				imgui.Text("Notifications")
				imgui.PopStyleColor()

				// Button padding style
				imgui.PushStyleVarVec2(imgui.StyleVarFramePadding, imgui.Vec2{X: 8, Y: 4})

				// Right-align buttons
				if len(notifications) > 0 {
					imgui.SameLine()
					imgui.SetCursorPosX(float32(centerWidth) - 120)
					imgui.PushStyleColorVec4(imgui.ColButton, currentCenterTheme.buttonBg)
					imgui.PushStyleColorVec4(imgui.ColButtonHovered, currentCenterTheme.buttonHov)
					imgui.PushStyleColorVec4(imgui.ColText, currentCenterTheme.titleText)
					if imgui.SmallButton("Clear All") {
						showDismissAllConfirm = true
					}
					imgui.PopStyleColor()
					imgui.PopStyleColor()
					imgui.PopStyleColor()
				}

				// Close button (top right, theme-aware)
				imgui.SameLine()
				imgui.SetCursorPosX(float32(centerWidth) - 35)
				imgui.PushStyleColorVec4(imgui.ColButton, imgui.Vec4{X: 0, Y: 0, Z: 0, W: 0})
				imgui.PushStyleColorVec4(imgui.ColButtonHovered, currentCenterTheme.buttonHov)
				imgui.PushStyleColorVec4(imgui.ColText, currentCenterTheme.mutedText)
				if imgui.SmallButton("X") {
					centerWnd.SetShouldClose(true)
				}
				imgui.PopStyleColor()
				imgui.PopStyleColor()
				imgui.PopStyleColor()

				imgui.PopStyleVar() // FramePadding
			}),
			g.Spacing(),

			// Error message if any (also fixed)
			g.Custom(func() {
				if lastError != "" {
					imgui.PushStyleColorVec4(imgui.ColText, imgui.Vec4{X: 1, Y: 0.4, Z: 0.4, W: 1})
					imgui.TextWrapped(lastError)
					imgui.PopStyleColor()
				}
			}),

			// Scrollable notification list
			g.Custom(func() {
				// Calculate available height for scroll area
				_, availHeight := g.GetAvailableRegion()

				// Style scrollbar to be subtle
				imgui.PushStyleColorVec4(imgui.ColChildBg, imgui.Vec4{X: 0, Y: 0, Z: 0, W: 0})     // Transparent bg
				imgui.PushStyleColorVec4(imgui.ColScrollbarBg, imgui.Vec4{X: 0, Y: 0, Z: 0, W: 0}) // Transparent scrollbar bg
				imgui.PushStyleColorVec4(imgui.ColScrollbarGrab, imgui.Vec4{X: 0.5, Y: 0.5, Z: 0.5, W: 0.3})
				imgui.PushStyleColorVec4(imgui.ColScrollbarGrabHovered, imgui.Vec4{X: 0.6, Y: 0.6, Z: 0.6, W: 0.5})
				imgui.PushStyleColorVec4(imgui.ColScrollbarGrabActive, imgui.Vec4{X: 0.7, Y: 0.7, Z: 0.7, W: 0.7})
				imgui.PushStyleVarFloat(imgui.StyleVarScrollbarSize, 6) // Thin scrollbar
				imgui.PushStyleVarFloat(imgui.StyleVarScrollbarRounding, 3)
				if imgui.BeginChildStrV("notif_scroll", imgui.Vec2{X: -1, Y: availHeight - 10}, imgui.ChildFlagsNone, imgui.WindowFlagsNone) {
					if len(notifications) == 0 {
						imgui.PushStyleColorVec4(imgui.ColText, currentCenterTheme.mutedText)
						imgui.Text("No notifications")
						imgui.PopStyleColor()
					} else {
						// Render newest first (reverse order)
						for i := len(notifications) - 1; i >= 0; i-- {
							renderCenterCard(&notifications[i], baseURL, &notifications, &lastError)
						}
					}
				}
				imgui.EndChild()
				imgui.PopStyleVar()   // ScrollbarRounding
				imgui.PopStyleVar()   // ScrollbarSize
				imgui.PopStyleColor() // ScrollbarGrabActive
				imgui.PopStyleColor() // ScrollbarGrabHovered
				imgui.PopStyleColor() // ScrollbarGrab
				imgui.PopStyleColor() // ScrollbarBg
				imgui.PopStyleColor() // ChildBg
			}),

			// Dismiss all confirmation
			g.Custom(func() {
				if showDismissAllConfirm {
					g.PopupModal("Clear All?").Layout(
						g.Label("Clear all notifications?"),
						g.Spacing(),
						g.Row(
							g.Button("Cancel").Size(80, 25).OnClick(func() {
								showDismissAllConfirm = false
								g.CloseCurrentPopup()
							}),
							g.Button("Clear").Size(80, 25).OnClick(func() {
								if err := dismissAllNotifications(baseURL); err != "" {
									lastError = err
								} else {
									notifications = nil
									lastError = ""
									centerWnd.SetSize(centerWidth, calcHeight())
								}
								showDismissAllConfirm = false
								g.CloseCurrentPopup()
							}),
						),
					).Build()
					g.OpenPopup("Clear All?")
				}
			}),
		)

		g.PopStyleColor()
		imgui.PopStyleVar()
		imgui.PopStyleVar()
	})

	refreshTicker.Stop()

	// Notify daemon that center is closing
	signalCenterClosed(baseURL)
}

// positionCenterWindow positions the window in the top-right corner.
func positionCenterWindow(wnd *g.MasterWindow) {
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

	margin := 20
	x := videoMode.Width - centerWidth - margin
	y := margin

	wnd.SetPos(x, y)
}

// loadCenterPendingIcons loads any pending icon textures.
func loadCenterPendingIcons() {
	for id, iconData := range centerPendingIcons {
		img, err := loadIconFromBase64(iconData)
		if err != nil {
			log.Printf("Failed to decode icon for notification %d: %v", id, err)
			delete(centerPendingIcons, id)
			continue
		}

		notifID := id
		g.EnqueueNewTextureFromRgba(img, func(tex *g.Texture) {
			centerTextures[notifID] = tex
			delete(centerPendingIcons, notifID)
		})
	}
}

// renderCenterCard renders a single notification card matching popup style.
func renderCenterCard(n *CenterNotification, baseURL string, notifications *[]CenterNotification, lastError *string) {
	id := n.ID

	// Card styling - uses current theme, full opacity when hovered
	cardBg := currentCenterTheme.cardBg
	if centerHoveredCards[id] {
		cardBg.W = 1.0
	}
	borderColor := statusBorderVec4(n.Status)

	imgui.PushStyleColorVec4(imgui.ColChildBg, cardBg)
	imgui.PushStyleColorVec4(imgui.ColBorder, borderColor)
	imgui.PushStyleVarFloat(imgui.StyleVarChildRounding, 6)
	imgui.PushStyleVarFloat(imgui.StyleVarChildBorderSize, 1)

	cardH := float32(centerCardHeight)
	if len(n.Actions) > 0 {
		cardH += 30
	}

	if imgui.BeginChildStrV(fmt.Sprintf("card_%d", id), imgui.Vec2{X: -1, Y: cardH}, imgui.ChildFlagsBorders, imgui.WindowFlagsNone) {
		const padding float32 = 10
		const iconSize float32 = 48

		// Check for icon texture
		tex := centerTextures[id]
		textStartX := padding

		if tex != nil {
			// Icon on left
			contentHeight := cardH - padding
			iconOffset := (contentHeight - iconSize) / 2
			imgui.SetCursorPos(imgui.Vec2{X: padding, Y: iconOffset})
			imgui.Image(tex.ID(), imgui.Vec2{X: iconSize, Y: iconSize})
			textStartX = padding + iconSize + padding
		}

		// Title row
		imgui.SetCursorPos(imgui.Vec2{X: textStartX, Y: padding})
		if n.Title != "" {
			imgui.PushStyleColorVec4(imgui.ColText, currentCenterTheme.titleText)
			imgui.Text(n.Title)
			imgui.PopStyleColor()
		}

		// Dismiss button (only for notifications with actions)
		if len(n.Actions) > 0 {
			imgui.SameLine()
			imgui.SetCursorPosX(centerWidth - 50)
			imgui.PushStyleColorVec4(imgui.ColButton, imgui.Vec4{X: 0.5, Y: 0.3, Z: 0.3, W: 0.5})
			imgui.PushStyleColorVec4(imgui.ColButtonHovered, imgui.Vec4{X: 0.7, Y: 0.3, Z: 0.3, W: 0.8})
			if imgui.SmallButton(fmt.Sprintf("x##%d", id)) {
				if err := dismissNotificationAPI(baseURL, id); err != "" {
					*lastError = err
				} else {
					for i, notif := range *notifications {
						if notif.ID == id {
							*notifications = append((*notifications)[:i], (*notifications)[i+1:]...)
							delete(centerTextures, id)
							delete(centerHoveredCards, id)
							break
						}
					}
				}
			}
			imgui.PopStyleColor()
			imgui.PopStyleColor()
		}

		// Message
		if n.Message != "" {
			imgui.SetCursorPosX(textStartX)
			imgui.PushStyleColorVec4(imgui.ColText, currentCenterTheme.bodyText)
			imgui.TextWrapped(n.Message)
			imgui.PopStyleColor()
		}

		// Source and time (smaller, muted)
		imgui.SetCursorPosX(textStartX)
		timeAgo := formatTimeAgo(n.CreatedAt)
		sourceText := timeAgo
		if n.Source != "" {
			sourceText = fmt.Sprintf("%s - %s", n.Source, timeAgo)
		}
		imgui.PushStyleColorVec4(imgui.ColText, currentCenterTheme.mutedText)
		imgui.Text(sourceText)
		imgui.PopStyleColor()

		// Actions
		if len(n.Actions) > 0 {
			imgui.SetCursorPosX(textStartX)
			for i, action := range n.Actions {
				if i > 0 {
					imgui.SameLine()
				}
				actionCopy := action
				notifID := n.ID
				imgui.PushStyleColorVec4(imgui.ColButton, currentCenterTheme.buttonBg)
				imgui.PushStyleColorVec4(imgui.ColButtonHovered, currentCenterTheme.buttonHov)
				if imgui.Button(fmt.Sprintf("%s##%d_%d", action.Label, id, i)) {
					if err := ExecuteAction(actionCopy); err != nil {
						*lastError = fmt.Sprintf("Action failed: %v", err)
					} else {
						dismissNotificationAPI(baseURL, notifID)
						for j, notif := range *notifications {
							if notif.ID == notifID {
								*notifications = append((*notifications)[:j], (*notifications)[j+1:]...)
								delete(centerTextures, notifID)
								delete(centerHoveredCards, notifID)
								break
							}
						}
					}
				}
				imgui.PopStyleColor()
				imgui.PopStyleColor()
			}
		}

		// Track hover state
		isHovered := imgui.IsWindowHovered()
		centerHoveredCards[id] = isHovered

		// Show hand cursor and handle click to dismiss (if no actions)
		if isHovered && len(n.Actions) == 0 {
			imgui.SetMouseCursor(imgui.MouseCursorHand)
		}
		if len(n.Actions) == 0 && isHovered && imgui.IsMouseClickedBool(imgui.MouseButtonLeft) {
			if err := dismissNotificationAPI(baseURL, id); err != "" {
				*lastError = err
			} else {
				for i, notif := range *notifications {
					if notif.ID == id {
						*notifications = append((*notifications)[:i], (*notifications)[i+1:]...)
						delete(centerTextures, id)
						delete(centerHoveredCards, id)
						break
					}
				}
			}
		}
	}
	imgui.EndChild()

	imgui.PopStyleVar()
	imgui.PopStyleVar()
	imgui.PopStyleColor()
	imgui.PopStyleColor()

	imgui.Spacing()
}

// statusBorderVec4 returns the border color for a notification status.
func statusBorderVec4(status string) imgui.Vec4 {
	switch status {
	case "success":
		return imgui.Vec4{X: 0.2, Y: 0.7, Z: 0.3, W: 0.9}
	case "warning":
		return imgui.Vec4{X: 0.9, Y: 0.6, Z: 0.2, W: 0.9}
	case "error":
		return imgui.Vec4{X: 0.8, Y: 0.2, Z: 0.2, W: 0.9}
	default:
		return imgui.Vec4{X: 0.3, Y: 0.3, Z: 0.3, W: 0.8}
	}
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
