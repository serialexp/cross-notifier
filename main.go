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
	"sync"
	"time"

	"github.com/AllenDang/cimgui-go/imgui"
	g "github.com/AllenDang/giu"
	"github.com/kbinani/screenshot"
	xdraw "golang.org/x/image/draw"
)

const (
	defaultPort   = "9876"
	maxVisible    = 4
	notificationW = 320
	notificationH = 80
	iconSize      = 48
	padding       = 10
	stackPeek     = 20 // pixels visible for stacked notifications
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
	ID        int64     `json:"-"`
	Title     string    `json:"title"`
	Message   string    `json:"message"`
	IconData  string    `json:"iconData,omitempty"` // base64 encoded image
	IconHref  string    `json:"iconHref,omitempty"` // URL to fetch image from
	IconPath  string    `json:"iconPath,omitempty"` // local file path
	Duration  int       `json:"duration,omitempty"` // seconds: >0 auto-close, 0/omitted=persistent
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
)

func addNotification(n Notification) {
	notifMu.Lock()
	defer notifMu.Unlock()

	n.ID = nextID
	nextID++
	n.CreatedAt = time.Now()

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

func renderStackedNotification(n Notification, index int, total int) {
	id := n.ID

	// Scale down cards behind the first one for depth effect
	scale := float32(1.0) - float32(index)*0.03
	cardWidth := (notificationW - 2*padding) * scale
	cardHeight := (notificationH - padding) * scale

	// Position this card in the stack, centered horizontally
	xOffset := ((notificationW - 2*padding) - cardWidth) / 2
	yOffset := float32(index * stackPeek)
	imgui.SetCursorPos(imgui.Vec2{X: xOffset, Y: yOffset})

	// Card styling - full opacity when hovered
	cardBg := currentTheme.cardBg
	if hoveredCards[id] {
		cardBg.W = 1.0
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

		// Track hover state for next frame
		isHovered := imgui.IsWindowHovered()
		hoveredCards[id] = isHovered

		if isHovered {
			imgui.SetMouseCursor(imgui.MouseCursorHand)
		}

		// Click to dismiss
		if isHovered && imgui.IsMouseClickedBool(imgui.MouseButtonLeft) {
			go dismissNotification(id)
		}
	}
	imgui.EndChild()
	imgui.PopStyleVar() // ChildBorderSize
	imgui.PopStyleVar() // ChildRounding
	imgui.PopStyleColor() // Border
	imgui.PopStyleColor() // ChildBg
}

func updateWindowSize() {
	notifMu.Lock()
	count := len(notifications)
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

	// Stack layout: first card full height + peek for each additional card
	height := notificationH + (count-1)*stackPeek + padding

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

func runDaemon(port, serverURL, secret string) {
	wnd = g.NewMasterWindow(
		"Notifications",
		notificationW, notificationH,
		g.MasterWindowFlagsFloating|
			g.MasterWindowFlagsFrameless|
			g.MasterWindowFlagsTransparent,
	)

	wnd.SetBgColor(color.RGBA{0, 0, 0, 0})

	// Start local HTTP server in background
	go startHTTPServer(port)

	// Connect to remote server if configured
	if serverURL != "" {
		client := NewNotificationClient(serverURL, secret, func(n Notification) {
			addNotification(n)
		})

		client.OnConnect = func() {
			addNotification(Notification{
				Title:    "Connected",
				Message:  "Connected to notification server",
				Duration: 3,
			})
		}

		client.OnDisconnect = func() {
			addNotification(Notification{
				Title:    "Disconnected",
				Message:  "Lost connection to notification server",
				Duration: 5,
			})
		}

		go func() {
			if err := client.Connect(); err != nil {
				log.Printf("Failed to connect to server: %v", err)
				addNotification(Notification{
					Title:    "Connection Failed",
					Message:  fmt.Sprintf("Could not connect to %s", serverURL),
					Duration: 5,
				})
			}
		}()
	}

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

	// Determine if we need to show settings
	showSettings := *setup || !configExists

	// CLI flags override config
	serverURL := *connect
	secretKey := *secret
	if cfg != nil && serverURL == "" {
		serverURL = cfg.ServerURL
	}
	if cfg != nil && secretKey == "" {
		secretKey = cfg.Secret
	}

	if showSettings {
		initial := &Config{ServerURL: serverURL, Secret: secretKey}
		result := ShowSettingsWindow(initial)

		if result.Cancelled {
			log.Println("Setup cancelled")
			return
		}

		serverURL = result.ServerURL
		secretKey = result.Secret

		// Save config
		newCfg := &Config{ServerURL: serverURL, Secret: secretKey}
		if err := newCfg.Save(configPath); err != nil {
			log.Printf("Warning: failed to save config: %v", err)
		} else {
			log.Printf("Config saved to %s", configPath)
		}
	}

	if serverURL != "" && secretKey == "" {
		log.Fatal("Connecting to server requires a secret")
	}

	runDaemon(*port, serverURL, secretKey)
}
