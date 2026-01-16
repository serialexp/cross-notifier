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
	"runtime"
	"sync"
	"time"

	"github.com/fsnotify/fsnotify"
	xdraw "golang.org/x/image/draw"
)

// Version is set at build time via ldflags.
var Version = "dev"

const (
	defaultPort      = "9876"
	maxVisible       = 4
	notificationW    = 320
	iconSize         = 48
	padding          = 10
	stackPeek        = 20 // pixels visible for stacked notifications
	actionBtnPadding = 4  // padding between action buttons
	cardPadding      = padding
	cardButtonHeight = 22
	actionRowH       = cardButtonHeight

	// Layout constants - must match font metrics
	textHeight  = 18 // Hack font at 16px
	lineSpacing = 2  // gap between lines within a section
	sectionGap  = 8  // gap between sections

	// Calculated: title + sectionGap + message(2 lines) + sectionGap + source
	notificationH = textHeight + sectionGap + (textHeight*2 + lineSpacing) + sectionGap + textHeight
)

// Theme colors
type theme struct {
	windowBg  color.RGBA
	cardBg    Color
	titleText Color
	bodyText  Color
	moreText  Color
}

var (
	darkTheme = theme{
		windowBg:  color.RGBA{0, 0, 0, 0}, // transparent window, cards provide the background
		cardBg:    Color{R: 0.15, G: 0.15, B: 0.17, A: 0.9},
		titleText: Color{R: 1, G: 1, B: 1, A: 1},
		bodyText:  Color{R: 0.8, G: 0.8, B: 0.8, A: 1},
		moreText:  Color{R: 0.6, G: 0.6, B: 0.6, A: 1},
	}
	lightTheme = theme{
		windowBg:  color.RGBA{0, 0, 0, 0}, // transparent window, cards provide the background
		cardBg:    Color{R: 0.96, G: 0.96, B: 0.96, A: 0.9},
		titleText: Color{R: 0.1, G: 0.1, B: 0.1, A: 1},
		bodyText:  Color{R: 0.3, G: 0.3, B: 0.3, A: 1},
		moreText:  Color{R: 0.5, G: 0.5, B: 0.5, A: 1},
	}
	currentTheme = &darkTheme
)

type Notification struct {
	ID            int64     `json:"-"`            // local ID for GUI tracking
	ServerID      string    `json:"id,omitempty"` // server-assigned ID for coordination
	ServerLabel   string    `json:"-"`            // label of server that sent this notification
	Source        string    `json:"source"`       // identifier of the service that sent this notification
	Title         string    `json:"title"`
	Message       string    `json:"message"`
	Status        string    `json:"status,omitempty"`    // info, success, warning, error
	IconData      string    `json:"iconData,omitempty"`  // base64 encoded image
	IconHref      string    `json:"iconHref,omitempty"`  // URL to fetch image from
	IconPath      string    `json:"iconPath,omitempty"`  // local file path
	Duration      int       `json:"duration,omitempty"`  // seconds: >0 auto-close, 0/omitted=persistent
	Actions       []Action  `json:"actions,omitempty"`   // clickable action buttons
	Exclusive     bool      `json:"exclusive,omitempty"` // if true, resolved when any client takes action
	CreatedAt     time.Time `json:"-"`
	ExpiresAt     time.Time `json:"-"`
	StoreOnExpire bool      `json:"-"`
	Expanded      bool      `json:"-"` // UI state: card is expanded to show full message
}

var (
	windowManager  *WindowManager
	notifyRenderer *NotificationRenderer
	notifications  []Notification
	notifMu        sync.Mutex
	nextID         int64 = 1

	textureMu sync.Mutex

	// Server clients for exclusive notification actions (keyed by server URL)
	serverClients   = make(map[string]*NotificationClient)
	serverClientsMu sync.RWMutex

	// Map server IDs to local IDs for exclusive notifications
	serverIDToLocalID = make(map[string]int64)

	// Rules configuration (updated when config loads/reloads)
	currentRulesConfig RulesConfig
	rulesConfigMu      sync.RWMutex

	// Notification center store
	notificationStore *NotificationStore

	// Track when center window last polled (to suppress popups when open)
	lastCenterPoll   time.Time
	lastCenterPollMu sync.RWMutex

	centerOpenCh chan string
)

// updateRulesConfig updates the global rules configuration.
func updateRulesConfig(cfg RulesConfig) {
	rulesConfigMu.Lock()
	currentRulesConfig = cfg
	rulesConfigMu.Unlock()
}

// isCenterOpen returns true if the notification center was recently active.
func isCenterOpen() bool {
	lastCenterPollMu.RLock()
	last := lastCenterPoll
	lastCenterPollMu.RUnlock()
	return time.Since(last) < 5*time.Second
}

// touchCenterPoll updates the last center poll time.
func touchCenterPoll() {
	lastCenterPollMu.Lock()
	lastCenterPoll = time.Now()
	lastCenterPollMu.Unlock()
}

// clearCenterPoll marks the center as closed.
func clearCenterPoll() {
	lastCenterPollMu.Lock()
	lastCenterPoll = time.Time{}
	lastCenterPollMu.Unlock()
}

func addNotification(n Notification) {
	// Refresh theme if it's been a while since last check
	refreshThemeIfStale()

	// Convert iconPath/iconHref to iconData for persistence
	if n.IconData == "" && (n.IconPath != "" || n.IconHref != "") {
		var img image.Image
		var err error
		if n.IconPath != "" {
			img, err = loadIconFromPath(n.IconPath)
		} else {
			img, err = loadIconFromURL(n.IconHref)
		}
		if err != nil {
			log.Printf("Failed to load icon: %v", err)
		} else if img != nil {
			if encoded, err := encodeImageToBase64(img); err != nil {
				log.Printf("Failed to encode icon: %v", err)
			} else {
				n.IconData = encoded
				n.IconPath = "" // Clear path since we have data
				n.IconHref = "" // Clear href since we have data
			}
		}
	}

	// Check rules for action and sound
	rulesConfigMu.RLock()
	rulesCfg := currentRulesConfig
	rulesConfigMu.RUnlock()

	action := RuleActionNormal
	var soundToPlay string

	rule := MatchRule(n, rulesCfg)
	if rule != nil {
		action = rule.EffectiveAction()
		soundToPlay = rule.Sound
	}

	// Handle dismiss action - don't store, don't show
	if action == RuleActionDismiss {
		return
	}

	// Decide whether to store now or only after popup expires.
	storeNow := action == RuleActionSilent
	if action == RuleActionNormal && isCenterOpen() {
		storeNow = true
	}

	// Silent action: stored to center but no popup or sound
	if action == RuleActionSilent {
		n.CreatedAt = time.Now()
		storeNotification(n)
		return
	}

	// Skip popup if notification center is open (user is already looking at notifications)
	if isCenterOpen() {
		n.CreatedAt = time.Now()
		storeNotification(n)
		return
	}

	// Normal action: play sound and show popup
	if soundToPlay != "" {
		PlaySound(soundToPlay)
	}

	notifMu.Lock()
	defer notifMu.Unlock()

	// Use local ID for popup notifications; store after expiration if applicable.
	n.ID = nextID
	nextID++
	n.CreatedAt = time.Now()
	n.StoreOnExpire = !storeNow

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

	notifications = append(notifications, n)
}

func dismissNotification(id int64) {
	// Remove from notification center store
	if notificationStore != nil {
		notificationStore.Remove(id)
		markCenterDirty()
	}

	notifMu.Lock()
	defer notifMu.Unlock()

	for i, n := range notifications {
		if n.ID == id {
			notifications = append(notifications[:i], notifications[i+1:]...)
			cleanupTexture(id)
			return
		}
	}
}

func toggleNotificationExpanded(id int64) {
	notifMu.Lock()
	defer notifMu.Unlock()

	for i, n := range notifications {
		if n.ID == id {
			notifications[i].Expanded = !notifications[i].Expanded
			return
		}
	}
}

func storeNotification(n Notification) {
	if notificationStore == nil {
		return
	}
	rawJSON, err := json.Marshal(n)
	if err != nil {
		log.Printf("Failed to serialize notification: %v", err)
		return
	}
	_ = notificationStore.Add(n, rawJSON)
	markCenterDirty()
}

func cleanupTexture(id int64) {
	textureMu.Lock()
	// Clean up server ID mapping
	for serverID, localID := range serverIDToLocalID {
		if localID == id {
			delete(serverIDToLocalID, serverID)
			break
		}
	}
	textureMu.Unlock()

	CleanupActionStates(id)

	// Clean up OpenGL texture from popup renderer
	if notifyRenderer != nil {
		notifyRenderer.cleanupIconTexture(id)
	}
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

func scaleImage(src image.Image, width, height int) image.Image {
	srcBounds := src.Bounds()
	srcW := srcBounds.Dx()
	srcH := srcBounds.Dy()

	// Calculate scale factor to fit within bounds while preserving aspect ratio
	scaleX := float64(width) / float64(srcW)
	scaleY := float64(height) / float64(srcH)
	scale := scaleX
	if scaleY < scaleX {
		scale = scaleY
	}

	// Don't upscale
	if scale >= 1.0 {
		// Center the original image in the target area
		dst := image.NewRGBA(image.Rect(0, 0, width, height))
		offsetX := (width - srcW) / 2
		offsetY := (height - srcH) / 2
		xdraw.Copy(dst, image.Point{X: offsetX, Y: offsetY}, src, srcBounds, xdraw.Over, nil)
		return dst
	}

	// Calculate scaled dimensions
	scaledW := int(float64(srcW) * scale)
	scaledH := int(float64(srcH) * scale)

	// Create destination with target size, scale image centered
	dst := image.NewRGBA(image.Rect(0, 0, width, height))
	offsetX := (width - scaledW) / 2
	offsetY := (height - scaledH) / 2
	dstRect := image.Rect(offsetX, offsetY, offsetX+scaledW, offsetY+scaledH)
	xdraw.CatmullRom.Scale(dst, dstRect, src, srcBounds, xdraw.Over, nil)
	return dst
}

func pruneExpired() {
	notifMu.Lock()
	defer notifMu.Unlock()

	now := time.Now()
	filtered := notifications[:0]
	for _, n := range notifications {
		// Keep if: expanded, never expires (zero time), or not yet expired
		if n.Expanded || n.ExpiresAt.IsZero() || now.Before(n.ExpiresAt) {
			filtered = append(filtered, n)
		} else {
			if n.StoreOnExpire {
				storeNotification(n)
			}
			cleanupTexture(n.ID)
		}
	}
	notifications = filtered
}

// apiError writes a JSON error response with usage information.
func apiError(w http.ResponseWriter, status int, message string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(map[string]any{
		"error": message,
		"usage": map[string]any{
			"method": "POST",
			"headers": map[string]string{
				"Content-Type": "application/json",
			},
			"body": map[string]any{
				"title":     "(string) notification title",
				"message":   "(string) notification body",
				"status":    "(string, optional) info|success|warning|error",
				"iconPath":  "(string, optional) local file path to icon",
				"iconHref":  "(string, optional) URL to fetch icon from",
				"iconData":  "(string, optional) base64-encoded icon",
				"duration":  "(int, optional) seconds before auto-dismiss, 0=persistent",
				"actions":   "(array, optional) action buttons",
				"exclusive": "(bool, optional) coordinate actions across clients",
			},
			"example": map[string]any{
				"title":    "Hello",
				"message":  "World",
				"status":   "success",
				"duration": 5,
			},
		},
	})
}

func httpHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		apiError(w, http.StatusMethodNotAllowed, "POST required")
		return
	}

	var n Notification
	if err := json.NewDecoder(r.Body).Decode(&n); err != nil {
		apiError(w, http.StatusBadRequest, "invalid JSON: "+err.Error())
		return
	}

	if n.Title == "" && n.Message == "" {
		apiError(w, http.StatusBadRequest, "title or message required")
		return
	}

	addNotification(n)
	w.WriteHeader(http.StatusAccepted)
}

func statusHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "GET required", http.StatusMethodNotAllowed)
		return
	}

	serverClientsMu.RLock()
	status := make(map[string]bool)
	for url, client := range serverClients {
		status[url] = client.IsConnected()
	}
	serverClientsMu.RUnlock()

	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(status)
}

// centerListHandler returns all notifications in the center.
func centerListHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "GET required", http.StatusMethodNotAllowed)
		return
	}

	if notificationStore == nil {
		http.Error(w, "store not initialized", http.StatusServiceUnavailable)
		return
	}

	// Track that center is actively polling
	touchCenterPoll()

	stored := notificationStore.List()

	// Parse notifications for JSON response
	result := make([]map[string]any, 0, len(stored))
	for _, s := range stored {
		n, err := s.ParseNotification()
		if err != nil {
			continue
		}
		result = append(result, map[string]any{
			"id":        s.ID,
			"title":     n.Title,
			"message":   n.Message,
			"status":    n.Status,
			"source":    n.Source,
			"actions":   n.Actions,
			"iconData":  n.IconData,
			"createdAt": s.CreatedAt,
		})
	}

	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(result)
}

// centerDismissHandler handles dismissing notifications from the center.
// DELETE /center/{id} - dismiss single notification
// DELETE /center?confirm=true - dismiss all
func centerDismissHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodDelete {
		http.Error(w, "DELETE required", http.StatusMethodNotAllowed)
		return
	}

	if notificationStore == nil {
		http.Error(w, "store not initialized", http.StatusServiceUnavailable)
		return
	}

	// Check for specific ID in path: /center/123
	path := r.URL.Path
	if len(path) > len("/center/") {
		idStr := path[len("/center/"):]
		var id int64
		if _, err := fmt.Sscanf(idStr, "%d", &id); err != nil {
			http.Error(w, "invalid ID", http.StatusBadRequest)
			return
		}

		if notificationStore.Remove(id) {
			// Also dismiss from popup if visible
			dismissNotification(id)
			w.WriteHeader(http.StatusOK)
		} else {
			http.Error(w, "not found", http.StatusNotFound)
		}
		return
	}

	// Dismiss all requires confirmation
	if r.URL.Query().Get("confirm") != "true" {
		http.Error(w, "confirm=true required to dismiss all", http.StatusBadRequest)
		return
	}

	// Dismiss all from popup display
	for _, s := range notificationStore.List() {
		dismissNotification(s.ID)
	}

	notificationStore.Clear()
	markCenterDirty()
	w.WriteHeader(http.StatusOK)
}

// centerCountHandler returns just the count of notifications.
func centerCountHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "GET required", http.StatusMethodNotAllowed)
		return
	}

	if notificationStore == nil {
		http.Error(w, "store not initialized", http.StatusServiceUnavailable)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]int{"count": notificationStore.Count()})
}

// centerCloseHandler signals that the notification center window has closed.
func centerCloseHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST required", http.StatusMethodNotAllowed)
		return
	}
	clearCenterPoll()
	w.WriteHeader(http.StatusOK)
}

func startHTTPServer(port string) {
	http.HandleFunc("/notify", httpHandler)
	http.HandleFunc("/status", statusHandler)
	http.HandleFunc("/center", centerListHandler)
	http.HandleFunc("/center/", centerDismissHandler)
	http.HandleFunc("/center/count", centerCountHandler)
	http.HandleFunc("/center/close", centerCloseHandler)
	addr := ":" + port
	log.Printf("Listening on http://localhost%s/notify", addr)
	if err := http.ListenAndServe(addr, nil); err != nil {
		log.Fatalf("HTTP server failed: %v", err)
	}
}

func updateTheme() {
	if isDarkMode() {
		currentTheme = &darkTheme
		currentCenterTheme = &centerDarkTheme
	} else {
		currentTheme = &lightTheme
		currentCenterTheme = &centerLightTheme
	}
}

// notificationHeight calculates the height needed for a notification.
func notificationHeight(n Notification) float32 {
	height := float32(notificationH + padding*2)
	if len(n.Actions) > 0 {
		height += sectionGap + actionRowH
	}
	return height
}

// triggerSuccessAnimation starts the success animation for a notification.
func triggerSuccessAnimation(notifID int64) {
	if notifyRenderer != nil {
		notifyRenderer.StartSuccessAnimation(notifID)
	}

	go func() {
		// Wait for animation to complete then dismiss
		time.Sleep(500 * time.Millisecond)
		dismissNotification(notifID)
	}()
}

// makeConnectionStatusChecker returns a function that queries the daemon's
// /status endpoint to check if a server URL is connected.
func makeConnectionStatusChecker(daemonPort string) func(string) bool {
	return func(url string) bool {
		resp, err := http.Get(fmt.Sprintf("http://localhost:%s/status", daemonPort))
		if err != nil {
			return false
		}
		defer resp.Body.Close()

		var status map[string]bool
		if err := json.NewDecoder(resp.Body).Decode(&status); err != nil {
			return false
		}

		return status[url]
	}
}

// launchSettingsProcess starts a separate settings window process.
// It passes the daemon's port via environment variable so the settings
// window can query connection status via HTTP.
func launchSettingsProcess(port string) {
	execPath, err := os.Executable()
	if err != nil {
		log.Printf("Failed to get executable path: %v", err)
		return
	}

	cmd := exec.Command(execPath, "-setup")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	cmd.Env = append(os.Environ(), fmt.Sprintf("CROSS_NOTIFIER_DAEMON_PORT=%s", port))
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
	serverLabel := server.Label
	if serverLabel == "" {
		serverLabel = server.URL
	}

	client := NewNotificationClient(server.URL, server.Secret, clientName, func(n Notification) {
		n.ServerLabel = serverLabel
		addNotification(n)
	})
	client.SetOnResolved(func(resolved ResolvedMessage) {
		handleResolvedMessage(resolved)
	})

	client.OnConnect = func() {
		addNotification(Notification{
			Title:    "Connected",
			Message:  fmt.Sprintf("Connected to %s", serverLabel),
			IconPath: "tray.png",
			Duration: 20,
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

		// Debounce timer to prevent multiple reloads from rapid file events
		var debounceTimer *time.Timer
		debounceDelay := 200 * time.Millisecond

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
					// Reset debounce timer
					if debounceTimer != nil {
						debounceTimer.Stop()
					}
					debounceTimer = time.AfterFunc(debounceDelay, func() {
						log.Println("Config file changed, reloading...")
						cfg, err := LoadConfig(configPath)
						if err != nil {
							log.Printf("Failed to reload config: %v", err)
							return
						}

						// Update rules configuration
						updateRulesConfig(cfg.Rules)
						debugFontMetrics = cfg.DebugFontMetrics

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
					})
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
	StartTray(
		func() {
			launchSettingsProcess(port)
		},
		func() {
			OpenCenterWindow(port)
		},
		func() int {
			// Return number of connected servers
			serverClientsMu.RLock()
			defer serverClientsMu.RUnlock()

			count := 0
			for _, client := range serverClients {
				if client.IsConnected() {
					count++
				}
			}
			return count
		},
		func() int {
			// Return number of notifications in center
			if notificationStore == nil {
				return 0
			}
			return notificationStore.Count()
		},
	)

	// Initialize rules configuration
	updateRulesConfig(cfg.Rules)
	debugFontMetrics = cfg.DebugFontMetrics

	// Initialize notification store
	notificationStore = NewNotificationStore(DefaultStorePath())
	if err := notificationStore.Load(); err != nil {
		log.Printf("Warning: failed to load notification store: %v", err)
	}

	// Start local HTTP server in background
	go startHTTPServer(port)

	// Connect to all configured servers
	connectAllServers(cfg.Servers, cfg.Name)

	// Watch config file for changes
	watchConfig(cfg)

	centerOpenCh = make(chan string, 1)

	windowManager = NewWindowManager()
	notifyWnd, err := windowManager.CreateNotificationWindow()
	if err != nil {
		log.Fatalf("Failed to create notification window: %v", err)
	}

	mw := windowManager.GetManagedWindow(notifyWnd)
	if mw == nil {
		log.Fatalf("Failed to manage notification window")
	}

	notifyRenderer = NewNotificationRenderer(mw.Renderer, notifyWnd)
	windowManager.SetWindowRenderCallback(notifyWnd, notifyRenderer.Render)

	if err := windowManager.Run(func() error {
		for {
			select {
			case requestPort := <-centerOpenCh:
				if err := openCenterWindowInProcess(windowManager, requestPort); err != nil {
					log.Printf("Failed to open notification center: %v", err)
				}
			default:
				return nil
			}
		}
	}); err != nil {
		log.Fatalf("Window manager failed: %v", err)
	}
}

func main() {
	runtime.LockOSThread()
	port := flag.String("port", defaultPort, "Port to listen on (or CROSS_NOTIFIER_PORT env)")
	connect := flag.String("connect", "", "WebSocket URL of server to connect to (or CROSS_NOTIFIER_SERVER env)")
	secret := flag.String("secret", "", "Shared secret for authentication (or CROSS_NOTIFIER_SECRET env)")
	name := flag.String("name", "", "Client display name for identification (or CROSS_NOTIFIER_NAME env)")
	setup := flag.Bool("setup", false, "Open settings window")
	center := flag.Bool("center", false, "Open notification center window")
	installAutostart := flag.Bool("install-autostart", false, "Enable auto-start on login")
	uninstallAutostart := flag.Bool("uninstall-autostart", false, "Disable auto-start on login")

	flag.Parse()

	// Handle autostart commands
	if *installAutostart {
		if err := InstallAutostart(); err != nil {
			log.Fatalf("Failed to enable auto-start: %v", err)
		}
		fmt.Println("Auto-start enabled. CrossNotifier will start automatically on login.")
		return
	}
	if *uninstallAutostart {
		if err := UninstallAutostart(); err != nil {
			log.Fatalf("Failed to disable auto-start: %v", err)
		}
		fmt.Println("Auto-start disabled. CrossNotifier will no longer start automatically.")
		return
	}

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

	// Daemon mode - load config file if it exists
	configPath := ConfigPath()
	cfg, _ := LoadConfig(configPath)

	// Initialize config if not loaded
	if cfg == nil {
		cfg = &Config{}
	}

	// Apply center panel config (default to respecting top work area for macOS menu bar)
	if cfg.CenterPanel == (CenterPanelConfig{}) {
		cfg.CenterPanel.RespectWorkAreaTop = true
	}
	centerPanelConfig = cfg.CenterPanel

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

	// Only show settings window if explicitly requested with -setup flag
	// The tray icon provides access to settings, and defaults work fine for local use
	if *setup {
		// Check if daemon is running and get connection status
		daemonPort := os.Getenv("CROSS_NOTIFIER_DAEMON_PORT")
		if daemonPort == "" {
			daemonPort = *port
		}

		isConnected := makeConnectionStatusChecker(daemonPort)
		log.Printf("Opening settings window with config: Name=%q, Servers=%d", cfg.Name, len(cfg.Servers))
		result := ShowSettingsWindowNew(cfg, isConnected)

		if result.Cancelled || result.Config == nil {
			log.Println("Setup cancelled or closed without saving")
			return
		}

		cfg = result.Config
		log.Printf("Settings window returned config: Name=%q, Servers=%d", cfg.Name, len(cfg.Servers))

		// Save config
		if err := cfg.Save(configPath); err != nil {
			log.Printf("Warning: failed to save config: %v", err)
		} else {
			log.Printf("Config saved to %s", configPath)
		}

		// -setup flag just saves and exits (daemon is already running or will be started separately)
		return
	}

	// Open notification center window if requested
	if *center {
		daemonPort := os.Getenv("CROSS_NOTIFIER_DAEMON_PORT")
		if daemonPort == "" {
			daemonPort = *port
		}

		if err := ShowCenterWindow(daemonPort); err != nil {
			log.Printf("Notification center error: %v", err)
		}
		return
	}

	runDaemon(*port, cfg)
}
