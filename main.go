// ABOUTME: Cross-platform notification daemon that displays notifications via HTTP API.
// ABOUTME: Can run as server (forwarding to clients) or daemon (displaying notifications).

package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"image"
	"image/color"
	"log"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"time"

	"github.com/AllenDang/cimgui-go/imgui"
	g "github.com/AllenDang/giu"
	"github.com/fsnotify/fsnotify"
	"github.com/kbinani/screenshot"
	xdraw "golang.org/x/image/draw"
)

const (
	defaultPort      = "9876"
	maxVisible       = 4
	notificationW    = 320
	notificationH    = 80 // base height without actions
	actionRowH       = 28 // height of action button row
	iconSize         = 48
	padding          = 10
	stackPeek        = 20 // pixels visible for stacked notifications
	actionBtnPadding = 4  // padding between action buttons
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
		cardBg:    imgui.Vec4{X: 0.15, Y: 0.15, Z: 0.17, W: 0.9},
		titleText: imgui.Vec4{X: 1, Y: 1, Z: 1, W: 1},
		bodyText:  imgui.Vec4{X: 0.8, Y: 0.8, Z: 0.8, W: 1},
		moreText:  imgui.Vec4{X: 0.6, Y: 0.6, Z: 0.6, W: 1},
	}
	lightTheme = theme{
		windowBg:  color.RGBA{0, 0, 0, 0}, // transparent window, cards provide the background
		cardBg:    imgui.Vec4{X: 0.96, Y: 0.96, Z: 0.96, W: 0.9},
		titleText: imgui.Vec4{X: 0.1, Y: 0.1, Z: 0.1, W: 1},
		bodyText:  imgui.Vec4{X: 0.3, Y: 0.3, Z: 0.3, W: 1},
		moreText:  imgui.Vec4{X: 0.5, Y: 0.5, Z: 0.5, W: 1},
	}
	currentTheme = &darkTheme
)

type Notification struct {
	ID        int64     `json:"-"`            // local ID for GUI tracking
	ServerID  string    `json:"id,omitempty"` // server-assigned ID for coordination
	Title     string    `json:"title"`
	Message   string    `json:"message"`
	IconData  string    `json:"iconData,omitempty"`  // base64 encoded image
	IconHref  string    `json:"iconHref,omitempty"`  // URL to fetch image from
	IconPath  string    `json:"iconPath,omitempty"`  // local file path
	Duration  int       `json:"duration,omitempty"`  // seconds: >0 auto-close, 0/omitted=persistent
	Actions   []Action  `json:"actions,omitempty"`   // clickable action buttons
	Exclusive bool      `json:"exclusive,omitempty"` // if true, resolved when any client takes action
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
	pendingIcons = make(map[int64]Notification) // notification ID -> notification with icon info

	// Hover state tracking (previous frame)
	hoveredCards = make(map[int64]bool)
	textureMu    sync.Mutex

	// Animation state for notifications
	successAnimations = make(map[int64]float32) // notification ID -> animation progress (0-1)

	// Server clients for exclusive notification actions (keyed by server URL)
	serverClients   = make(map[string]*NotificationClient)
	serverClientsMu sync.RWMutex

	// Map server IDs to local IDs for exclusive notifications
	serverIDToLocalID = make(map[string]int64)
)

func addNotification(n Notification) {
	notifMu.Lock()
	defer notifMu.Unlock()

	n.ID = nextID
	nextID++
	n.CreatedAt = time.Now()

	// Track server ID -> local ID mapping for exclusive notifications
	if n.ServerID != "" {
		textureMu.Lock()
		serverIDToLocalID[n.ServerID] = n.ID
		textureMu.Unlock()
	}

	// duration > 0: auto-close after that many seconds
	// duration <= 0: never auto-close (default)
	if n.Duration > 0 {
		n.ExpiresAt = n.CreatedAt.Add(time.Duration(n.Duration) * time.Second)
	} else {
		n.ExpiresAt = time.Time{} // zero time = never expires
	}

	// Queue icon loading if any icon source is specified
	if n.IconData != "" || n.IconHref != "" || n.IconPath != "" {
		textureMu.Lock()
		pendingIcons[n.ID] = n
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
	delete(hoveredCards, id)
	delete(successAnimations, id)
	// Clean up server ID mapping
	for serverID, localID := range serverIDToLocalID {
		if localID == id {
			delete(serverIDToLocalID, serverID)
			break
		}
	}
	CleanupActionStates(id)
}

// handleResolvedMessage processes a resolved notification from the server.
func handleResolvedMessage(resolved ResolvedMessage) {
	textureMu.Lock()
	localID, exists := serverIDToLocalID[resolved.NotificationID]
	textureMu.Unlock()

	if !exists {
		log.Printf("Resolved message for unknown notification %s", resolved.NotificationID)
		return
	}

	if resolved.Success {
		// Trigger success animation then dismiss
		triggerSuccessAnimation(localID)
	} else {
		// Dismiss and show error
		dismissNotification(localID)
		addNotification(Notification{
			Title:    "Action Failed",
			Message:  resolved.Error,
			Duration: 5,
		})
	}
}

func loadPendingIcons() {
	textureMu.Lock()
	pending := make(map[int64]Notification)
	for id, n := range pendingIcons {
		pending[id] = n
	}
	textureMu.Unlock()

	for id, n := range pending {
		img, err := resolveIcon(n)
		if err != nil {
			log.Printf("Failed to load icon for notification %d: %v", id, err)
			textureMu.Lock()
			delete(pendingIcons, id)
			textureMu.Unlock()
			continue
		}
		if img == nil {
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
		// Keep if: never expires (zero time) or not yet expired
		if n.ExpiresAt.IsZero() || now.Before(n.ExpiresAt) {
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

func startHTTPServer(port string) {
	http.HandleFunc("/notify", httpHandler)
	addr := ":" + port
	log.Printf("Listening on http://localhost%s/notify", addr)
	if err := http.ListenAndServe(addr, nil); err != nil {
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
			// Render back to front (last notification first, so first is on top)
			for i := len(visible) - 1; i >= 0; i-- {
				renderStackedNotification(visible[i], i, len(visible))
			}
		}),
	)

	g.PopStyleColor()
	imgui.PopStyleVar()
	imgui.PopStyleVar()
}

// notificationHeight calculates the height needed for a notification.
func notificationHeight(n Notification) float32 {
	height := float32(notificationH)
	if len(n.Actions) > 0 {
		height += actionRowH
	}
	return height
}

func renderStackedNotification(n Notification, index int, total int) {
	id := n.ID

	// Scale down cards behind the first one for depth effect
	scale := float32(1.0) - float32(index)*0.03
	baseHeight := notificationHeight(n)
	cardWidth := (notificationW - 2*padding) * scale
	cardHeight := (baseHeight - padding) * scale

	// Position this card in the stack, centered horizontally
	xOffset := ((notificationW - 2*padding) - cardWidth) / 2
	yOffset := float32(index * stackPeek)
	imgui.SetCursorPos(imgui.Vec2{X: xOffset, Y: yOffset})

	// Card styling - full opacity when hovered
	cardBg := currentTheme.cardBg
	if hoveredCards[id] {
		cardBg.W = 1.0
	}

	// Apply success animation tint
	textureMu.Lock()
	successProgress := successAnimations[id]
	textureMu.Unlock()
	if successProgress > 0 {
		// Blend towards green
		greenTint := float32(0.3) * successProgress
		cardBg.Y += greenTint // Add green
	}

	imgui.PushStyleColorVec4(imgui.ColChildBg, cardBg)
	imgui.PushStyleColorVec4(imgui.ColBorder, imgui.Vec4{X: 0.3, Y: 0.3, Z: 0.3, W: 0.8})
	imgui.PushStyleVarFloat(imgui.StyleVarChildRounding, 6)
	imgui.PushStyleVarFloat(imgui.StyleVarChildBorderSize, 1)

	childFlags := imgui.ChildFlagsBorders
	windowFlags := imgui.WindowFlagsNone

	if imgui.BeginChildStrV(fmt.Sprintf("notif_%d", id), imgui.Vec2{X: cardWidth, Y: cardHeight}, childFlags, windowFlags) {
		// Check for icon texture
		textureMu.Lock()
		tex := textures[id]
		textureMu.Unlock()

		// Padding inside card
		const innerPadding float32 = 10

		textStartX := innerPadding

		if tex != nil {
			// Layout: icon on left (vertically centered), text on right
			contentHeight := notificationH - padding
			iconOffset := float32(contentHeight-iconSize) / 2
			imgui.SetCursorPos(imgui.Vec2{X: innerPadding, Y: iconOffset})
			imgui.Image(tex.ID(), imgui.Vec2{X: iconSize, Y: iconSize})
			imgui.SameLineV(0, innerPadding)
			textStartX = imgui.CursorPosX()
			imgui.SetCursorPosY(innerPadding)
		} else {
			imgui.SetCursorPos(imgui.Vec2{X: innerPadding, Y: innerPadding})
		}

		// Title
		if n.Title != "" {
			imgui.SetCursorPosX(textStartX)
			imgui.PushStyleColorVec4(imgui.ColText, currentTheme.titleText)
			imgui.TextWrapped(n.Title)
			imgui.PopStyleColor()
		}

		// Message
		if n.Message != "" {
			imgui.SetCursorPosX(textStartX)
			imgui.PushStyleColorVec4(imgui.ColText, currentTheme.bodyText)
			imgui.TextWrapped(n.Message)
			imgui.PopStyleColor()
		}

		// Action buttons
		if len(n.Actions) > 0 {
			renderActionButtons(n)
		}

		// Track hover state for next frame
		isHovered := imgui.IsWindowHovered()
		hoveredCards[id] = isHovered

		// Only show hand cursor if no actions (otherwise buttons handle their own cursors)
		if isHovered && len(n.Actions) == 0 {
			imgui.SetMouseCursor(imgui.MouseCursorHand)
		}

		// Click to dismiss (only if no actions, to avoid conflicts with button clicks)
		if len(n.Actions) == 0 && isHovered && imgui.IsMouseClickedBool(imgui.MouseButtonLeft) {
			go dismissNotification(id)
		}
	}
	imgui.EndChild()
	imgui.PopStyleVar()   // ChildBorderSize
	imgui.PopStyleVar()   // ChildRounding
	imgui.PopStyleColor() // Border
	imgui.PopStyleColor() // ChildBg
}

// renderActionButtons renders the action buttons for a notification.
func renderActionButtons(n Notification) {
	const innerPadding float32 = 10

	// Position at bottom of card
	imgui.SetCursorPos(imgui.Vec2{X: innerPadding, Y: float32(notificationH) - innerPadding - 20})

	for i, action := range n.Actions {
		if i > 0 {
			imgui.SameLineV(0, actionBtnPadding)
		}

		state := GetActionState(n.ID, i)
		renderActionButton(n, i, action, state)
	}
}

// renderActionButton renders a single action button with appropriate styling.
func renderActionButton(n Notification, actionIdx int, action Action, state *ActionStateInfo) {
	label := action.Label
	notifID := n.ID

	// Style button based on state
	switch state.State {
	case ActionLoading:
		// Show loading indicator
		label = "..."
		imgui.PushStyleColorVec4(imgui.ColButton, imgui.Vec4{X: 0.3, Y: 0.3, Z: 0.3, W: 1})
		imgui.PushStyleColorVec4(imgui.ColButtonHovered, imgui.Vec4{X: 0.3, Y: 0.3, Z: 0.3, W: 1})
		imgui.PushStyleColorVec4(imgui.ColButtonActive, imgui.Vec4{X: 0.3, Y: 0.3, Z: 0.3, W: 1})
	case ActionSuccess:
		imgui.PushStyleColorVec4(imgui.ColButton, imgui.Vec4{X: 0.2, Y: 0.6, Z: 0.2, W: 1})
		imgui.PushStyleColorVec4(imgui.ColButtonHovered, imgui.Vec4{X: 0.2, Y: 0.6, Z: 0.2, W: 1})
		imgui.PushStyleColorVec4(imgui.ColButtonActive, imgui.Vec4{X: 0.2, Y: 0.6, Z: 0.2, W: 1})
	case ActionError:
		imgui.PushStyleColorVec4(imgui.ColButton, imgui.Vec4{X: 0.6, Y: 0.2, Z: 0.2, W: 1})
		imgui.PushStyleColorVec4(imgui.ColButtonHovered, imgui.Vec4{X: 0.6, Y: 0.2, Z: 0.2, W: 1})
		imgui.PushStyleColorVec4(imgui.ColButtonActive, imgui.Vec4{X: 0.6, Y: 0.2, Z: 0.2, W: 1})
	default:
		imgui.PushStyleColorVec4(imgui.ColButton, imgui.Vec4{X: 0.25, Y: 0.25, Z: 0.28, W: 1})
		imgui.PushStyleColorVec4(imgui.ColButtonHovered, imgui.Vec4{X: 0.35, Y: 0.35, Z: 0.38, W: 1})
		imgui.PushStyleColorVec4(imgui.ColButtonActive, imgui.Vec4{X: 0.2, Y: 0.2, Z: 0.23, W: 1})
	}

	imgui.PushStyleVarFloat(imgui.StyleVarFrameRounding, 4)

	buttonID := fmt.Sprintf("%s##action_%d_%d", label, notifID, actionIdx)
	if imgui.Button(buttonID) && state.State == ActionIdle {
		// For exclusive notifications connected to server, send action to server
		connectedClient := getConnectedClient()
		if n.Exclusive && n.ServerID != "" && connectedClient != nil {
			SetActionState(notifID, actionIdx, ActionLoading, nil)
			go func() {
				if err := connectedClient.SendAction(n.ServerID, actionIdx); err != nil {
					log.Printf("Failed to send action to server: %v", err)
					// Fall back to local execution
					SetActionState(notifID, actionIdx, ActionIdle, nil)
				}
				g.Update()
			}()
		} else {
			// Execute action locally
			ExecuteActionAsync(notifID, actionIdx, action,
				func() {
					// On success: trigger success animation then dismiss
					triggerSuccessAnimation(notifID)
				},
				func(err error) {
					// On error: dismiss and show error notification
					dismissNotification(notifID)
					addNotification(Notification{
						Title:    "Action Failed",
						Message:  err.Error(),
						Duration: 5,
					})
				},
			)
		}
		g.Update()
	}

	imgui.PopStyleVar()
	imgui.PopStyleColor()
	imgui.PopStyleColor()
	imgui.PopStyleColor()
}

// triggerSuccessAnimation starts the success animation for a notification.
func triggerSuccessAnimation(notifID int64) {
	textureMu.Lock()
	successAnimations[notifID] = 1.0
	textureMu.Unlock()

	go func() {
		// Animate over 500ms then dismiss
		start := time.Now()
		duration := 500 * time.Millisecond
		for time.Since(start) < duration {
			progress := 1.0 - float32(time.Since(start))/float32(duration)
			textureMu.Lock()
			successAnimations[notifID] = progress
			textureMu.Unlock()
			g.Update()
			time.Sleep(16 * time.Millisecond) // ~60fps
		}
		dismissNotification(notifID)
	}()
}

func updateWindowSize() {
	notifMu.Lock()
	count := len(notifications)
	if count > maxVisible {
		count = maxVisible
	}
	var firstNotif Notification
	if count > 0 {
		firstNotif = notifications[0]
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

	// Stack layout: first card full height + peek for each additional card
	// Use actual height of first notification (may have actions)
	firstHeight := int(notificationHeight(firstNotif))
	height := firstHeight + (count-1)*stackPeek + padding

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

func runServer(port, secret string) {
	server := NewNotificationServer(secret)

	mux := http.NewServeMux()
	mux.HandleFunc("/notify", server.HandleNotify)
	mux.HandleFunc("/ws", server.HandleWebSocket)

	addr := ":" + port
	log.Printf("Server listening on %s", addr)
	log.Printf("  POST /notify - send notifications (requires auth)")
	log.Printf("  GET  /ws     - WebSocket connection for clients (requires auth)")

	if err := http.ListenAndServe(addr, mux); err != nil {
		log.Fatalf("Server failed: %v", err)
	}
}

// launchSettingsProcess starts a separate settings window process.
func launchSettingsProcess() {
	execPath, err := os.Executable()
	if err != nil {
		log.Printf("Failed to get executable path: %v", err)
		return
	}

	cmd := exec.Command(execPath, "-setup")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Start(); err != nil {
		log.Printf("Failed to start settings: %v", err)
	}
	// Don't wait - let it run independently
}

// getConnectedClient returns any connected server client for sending actions.
func getConnectedClient() *NotificationClient {
	serverClientsMu.RLock()
	defer serverClientsMu.RUnlock()
	for _, client := range serverClients {
		if client.IsConnected() {
			return client
		}
	}
	return nil
}

// connectToServer establishes a WebSocket connection to a notification server.
func connectToServer(server Server, clientName string) {
	client := NewNotificationClient(server.URL, server.Secret, clientName, func(n Notification) {
		addNotification(n)
	})
	client.SetOnResolved(func(resolved ResolvedMessage) {
		handleResolvedMessage(resolved)
		g.Update()
	})

	serverLabel := server.Label
	if serverLabel == "" {
		serverLabel = server.URL
	}

	client.OnConnect = func() {
		addNotification(Notification{
			Title:    "Connected",
			Message:  fmt.Sprintf("Connected to %s", serverLabel),
			Duration: 3,
		})
	}
	client.OnDisconnect = func() {
		addNotification(Notification{
			Title:    "Disconnected",
			Message:  fmt.Sprintf("Lost connection to %s", serverLabel),
			Duration: 5,
		})
	}

	serverClientsMu.Lock()
	serverClients[server.URL] = client
	serverClientsMu.Unlock()

	go func() {
		if err := client.Connect(); err != nil {
			log.Printf("Failed to connect to server %s: %v", server.URL, err)
			addNotification(Notification{
				Title:    "Connection Failed",
				Message:  fmt.Sprintf("Could not connect to %s", serverLabel),
				Duration: 5,
			})
		}
	}()
}

// disconnectServer closes connection to a specific server.
func disconnectServer(serverURL string) {
	serverClientsMu.Lock()
	client, exists := serverClients[serverURL]
	if exists {
		delete(serverClients, serverURL)
	}
	serverClientsMu.Unlock()

	if client != nil {
		client.Close()
	}
}

// connectAllServers connects to all configured servers.
func connectAllServers(servers []Server, clientName string) {
	for _, server := range servers {
		if server.URL != "" && server.Secret != "" {
			connectToServer(server, clientName)
		}
	}
}

// disconnectAllServers closes all server connections.
func disconnectAllServers() {
	serverClientsMu.Lock()
	clients := make([]*NotificationClient, 0, len(serverClients))
	for _, client := range serverClients {
		clients = append(clients, client)
	}
	serverClients = make(map[string]*NotificationClient)
	serverClientsMu.Unlock()

	for _, client := range clients {
		client.Close()
	}
}

// watchConfig monitors the config file for changes and reconnects if needed.
func watchConfig(currentCfg *Config) {
	configPath := ConfigPath()

	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		log.Printf("Failed to create config watcher: %v", err)
		return
	}

	// Watch the directory containing the config file (more reliable than watching the file directly)
	configDir := filepath.Dir(configPath)
	configFile := filepath.Base(configPath)

	if err := watcher.Add(configDir); err != nil {
		log.Printf("Failed to watch config directory: %v", err)
		watcher.Close()
		return
	}

	// Build a map of current servers for comparison
	currentServers := make(map[string]Server)
	for _, s := range currentCfg.Servers {
		currentServers[s.URL] = s
	}
	currentName := currentCfg.Name

	go func() {
		defer watcher.Close()

		for {
			select {
			case event, ok := <-watcher.Events:
				if !ok {
					return
				}
				// Check if it's our config file
				if filepath.Base(event.Name) != configFile {
					continue
				}
				if event.Op&(fsnotify.Write|fsnotify.Create) != 0 {
					log.Println("Config file changed, reloading...")
					cfg, err := LoadConfig(configPath)
					if err != nil {
						log.Printf("Failed to reload config: %v", err)
						continue
					}

					// Build map of new servers
					newServers := make(map[string]Server)
					for _, s := range cfg.Servers {
						newServers[s.URL] = s
					}

					// Find servers to disconnect (in current but not in new, or secret changed)
					for url, oldServer := range currentServers {
						newServer, exists := newServers[url]
						if !exists || newServer.Secret != oldServer.Secret {
							log.Printf("Disconnecting from %s", url)
							disconnectServer(url)
						}
					}

					// Find servers to connect (in new but not in current, or secret changed)
					for url, newServer := range newServers {
						oldServer, exists := currentServers[url]
						if !exists || newServer.Secret != oldServer.Secret {
							if newServer.URL != "" && newServer.Secret != "" {
								log.Printf("Connecting to %s", url)
								connectToServer(newServer, cfg.Name)
							}
						}
					}

					// Update client name if changed (requires reconnecting all)
					if cfg.Name != currentName {
						log.Printf("Client name changed to %s, reconnecting all servers", cfg.Name)
						disconnectAllServers()
						connectAllServers(cfg.Servers, cfg.Name)
					}

					// Update current state
					currentServers = newServers
					currentName = cfg.Name
				}
			case err, ok := <-watcher.Errors:
				if !ok {
					return
				}
				log.Printf("Config watcher error: %v", err)
			}
		}
	}()
}

func runDaemon(port string, cfg *Config) {
	// Start system tray
	StartTray(func() {
		launchSettingsProcess()
	})

	// Start local HTTP server in background
	go startHTTPServer(port)

	// Connect to all configured servers
	connectAllServers(cfg.Servers, cfg.Name)

	// Watch config file for changes
	watchConfig(cfg)

	// Create notification window
	wnd = g.NewMasterWindow(
		"Notifications",
		notificationW, notificationH,
		g.MasterWindowFlagsFloating|
			g.MasterWindowFlagsFrameless|
			g.MasterWindowFlagsTransparent,
	)
	wnd.SetBgColor(color.RGBA{0, 0, 0, 0})

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

func main() {
	serverMode := flag.Bool("server", false, "Run as notification server")
	port := flag.String("port", defaultPort, "Port to listen on (or CROSS_NOTIFIER_PORT env)")
	connect := flag.String("connect", "", "WebSocket URL of server to connect to (or CROSS_NOTIFIER_SERVER env)")
	secret := flag.String("secret", "", "Shared secret for authentication (or CROSS_NOTIFIER_SECRET env)")
	name := flag.String("name", "", "Client display name for identification (or CROSS_NOTIFIER_NAME env)")
	setup := flag.Bool("setup", false, "Open settings window")

	flag.Parse()

	// Environment variables as fallbacks
	if *port == defaultPort {
		if envPort := os.Getenv("CROSS_NOTIFIER_PORT"); envPort != "" {
			*port = envPort
		}
	}
	if *secret == "" {
		*secret = os.Getenv("CROSS_NOTIFIER_SECRET")
	}
	if *connect == "" {
		*connect = os.Getenv("CROSS_NOTIFIER_SERVER")
	}
	if *name == "" {
		*name = os.Getenv("CROSS_NOTIFIER_NAME")
	}

	if *serverMode {
		if *secret == "" {
			log.Fatal("Server mode requires -secret flag or CROSS_NOTIFIER_SECRET env")
		}
		runServer(*port, *secret)
		return
	}

	// Daemon mode - check for config file
	configPath := ConfigPath()
	cfg, err := LoadConfig(configPath)
	configExists := err == nil

	// Initialize config if not loaded
	if cfg == nil {
		cfg = &Config{}
	}

	// CLI flags override config
	if *name != "" {
		cfg.Name = *name
	}
	// Add CLI-specified server if both connect and secret are provided
	if *connect != "" && *secret != "" {
		// Check if this server is already in the list
		found := false
		for i, s := range cfg.Servers {
			if s.URL == *connect {
				cfg.Servers[i].Secret = *secret
				found = true
				break
			}
		}
		if !found {
			cfg.Servers = append(cfg.Servers, Server{URL: *connect, Secret: *secret, Label: "CLI"})
		}
	} else if *connect != "" && *secret == "" {
		log.Fatal("Connecting to server requires a secret (-secret flag)")
	}

	// Determine if we need to show settings
	showSettings := *setup || !configExists

	if showSettings {
		result := ShowSettingsWindow(cfg)

		if result.Cancelled {
			log.Println("Setup cancelled")
			return
		}

		cfg = result.Config

		// Save config
		if err := cfg.Save(configPath); err != nil {
			log.Printf("Warning: failed to save config: %v", err)
		} else {
			log.Printf("Config saved to %s", configPath)
		}

		// If -setup was explicitly passed, just save and exit (daemon is already running)
		if *setup {
			return
		}
	}

	runDaemon(*port, cfg)
}
