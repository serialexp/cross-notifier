// ABOUTME: Standalone notification server binary for Docker/headless deployment.
// ABOUTME: Broadcasts notifications to connected WebSocket clients with exclusive action coordination.

package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"flag"
	"fmt"
	"image"
	_ "image/jpeg"
	"image/png"
	"io"
	"log"
	"net/http"
	"os"
	"os/exec"
	"runtime"
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"
	"github.com/gorilla/websocket"
	xdraw "golang.org/x/image/draw"
)

const (
	defaultPort = "9876"
	iconSize    = 48
)

// MessageType identifies the type of WebSocket message.
type MessageType string

const (
	MessageTypeNotification MessageType = "notification"
	MessageTypeAction       MessageType = "action"
	MessageTypeResolved     MessageType = "resolved"
)

// Message is the envelope for all WebSocket communication.
type Message struct {
	Type MessageType     `json:"type"`
	Data json.RawMessage `json:"data"`
}

// ActionMessage is sent by clients when they click an action button.
type ActionMessage struct {
	NotificationID string `json:"id"`
	ActionIndex    int    `json:"actionIndex"`
}

// ResolvedMessage is broadcast by the server when an exclusive notification is resolved.
type ResolvedMessage struct {
	NotificationID string `json:"id"`
	ResolvedBy     string `json:"resolvedBy"`
	ActionLabel    string `json:"actionLabel"`
	Success        bool   `json:"success"`
	Error          string `json:"error,omitempty"`
}

// Action represents a clickable action button on a notification.
type Action struct {
	Label   string            `json:"label"`
	URL     string            `json:"url"`
	Method  string            `json:"method,omitempty"`
	Headers map[string]string `json:"headers,omitempty"`
	Body    string            `json:"body,omitempty"`
	Open    bool              `json:"open,omitempty"`
}

// Notification represents a notification message.
type Notification struct {
	ID        string   `json:"id,omitempty"`
	Source    string   `json:"source"`
	Title     string   `json:"title"`
	Message   string   `json:"message"`
	Status    string   `json:"status,omitempty"`
	IconData  string   `json:"iconData,omitempty"`
	IconHref  string   `json:"iconHref,omitempty"`
	Duration  int      `json:"duration,omitempty"`
	Actions   []Action `json:"actions,omitempty"`
	Exclusive bool     `json:"exclusive,omitempty"`
}

// ClientInfo holds information about a connected client.
type ClientInfo struct {
	Name string
}

var upgrader = websocket.Upgrader{
	CheckOrigin: func(r *http.Request) bool {
		return true
	},
}

// Server manages WebSocket connections and broadcasts notifications.
type Server struct {
	secret    string
	clients   map[*websocket.Conn]*ClientInfo
	pending   map[string]Notification // ID -> Notification for exclusive notifications
	mu        sync.RWMutex
	pendingMu sync.RWMutex
}

func NewServer(secret string) *Server {
	return &Server{
		secret:  secret,
		clients: make(map[*websocket.Conn]*ClientInfo),
		pending: make(map[string]Notification),
	}
}

func (s *Server) checkAuth(r *http.Request) bool {
	auth := r.Header.Get("Authorization")
	if !strings.HasPrefix(auth, "Bearer ") {
		return false
	}
	return strings.TrimPrefix(auth, "Bearer ") == s.secret
}

func (s *Server) HandleWebSocket(w http.ResponseWriter, r *http.Request) {
	if !s.checkAuth(r) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}

	conn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		log.Printf("WebSocket upgrade failed: %v", err)
		return
	}

	clientName := r.Header.Get("X-Client-Name")
	clientInfo := &ClientInfo{Name: clientName}

	s.mu.Lock()
	s.clients[conn] = clientInfo
	s.mu.Unlock()

	if clientName != "" {
		log.Printf("Client '%s' connected (%d total)", clientName, s.ClientCount())
	} else {
		log.Printf("Client connected (%d total)", s.ClientCount())
	}

	go s.handleClient(conn)
}

func (s *Server) handleClient(conn *websocket.Conn) {
	defer func() {
		s.mu.Lock()
		clientInfo := s.clients[conn]
		delete(s.clients, conn)
		s.mu.Unlock()
		conn.Close()
		if clientInfo != nil && clientInfo.Name != "" {
			log.Printf("Client '%s' disconnected (%d remaining)", clientInfo.Name, s.ClientCount())
		} else {
			log.Printf("Client disconnected (%d remaining)", s.ClientCount())
		}
	}()

	s.mu.RLock()
	clientInfo := s.clients[conn]
	s.mu.RUnlock()

	clientName := ""
	if clientInfo != nil {
		clientName = clientInfo.Name
	}

	for {
		_, raw, err := conn.ReadMessage()
		if err != nil {
			return
		}

		msg, err := decodeMessage(raw)
		if err != nil {
			log.Printf("Failed to decode message: %v", err)
			continue
		}

		switch msg.Type {
		case MessageTypeAction:
			var actionMsg ActionMessage
			if err := json.Unmarshal(msg.Data, &actionMsg); err != nil {
				log.Printf("Failed to decode action message: %v", err)
				continue
			}
			s.handleActionMessage(clientName, actionMsg)
		}
	}
}

func (s *Server) handleActionMessage(clientName string, msg ActionMessage) {
	s.pendingMu.Lock()
	notif, exists := s.pending[msg.NotificationID]
	if exists {
		delete(s.pending, msg.NotificationID)
	}
	s.pendingMu.Unlock()

	if !exists {
		log.Printf("Action for unknown notification %s", msg.NotificationID)
		return
	}

	if msg.ActionIndex < 0 || msg.ActionIndex >= len(notif.Actions) {
		log.Printf("Invalid action index %d for notification %s", msg.ActionIndex, msg.NotificationID)
		return
	}

	action := notif.Actions[msg.ActionIndex]
	displayName := clientName
	if displayName == "" {
		displayName = "anonymous"
	}

	log.Printf("Client '%s' executing action '%s' on notification %s", displayName, action.Label, msg.NotificationID)

	var execErr error
	if action.Open {
		log.Printf("Open action requested - client should handle locally")
	} else {
		execErr = executeAction(action)
	}

	resolved := ResolvedMessage{
		NotificationID: msg.NotificationID,
		ResolvedBy:     clientName,
		ActionLabel:    action.Label,
		Success:        execErr == nil,
	}
	if execErr != nil {
		resolved.Error = execErr.Error()
		log.Printf("Action failed: %v", execErr)
	}

	s.BroadcastResolved(resolved)
}

func (s *Server) HandleNotify(w http.ResponseWriter, r *http.Request) {
	log.Printf("Received /notify request from %s", r.RemoteAddr)

	if r.Method != http.MethodPost {
		http.Error(w, `{"error":"POST required"}`, http.StatusMethodNotAllowed)
		return
	}

	if !s.checkAuth(r) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}

	// Read body into buffer so we can log it if decode fails
	var buf bytes.Buffer
	bodyReader := io.TeeReader(r.Body, &buf)

	var n Notification
	if err := json.NewDecoder(bodyReader).Decode(&n); err != nil {
		log.Printf("Failed to decode notification JSON: %v", err)
		log.Printf("Raw payload: %s", buf.String())
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	if n.Source == "" {
		log.Printf("Validation failed: missing source field. Payload: %s", buf.String())
		http.Error(w, "source is required", http.StatusBadRequest)
		return
	}

	if n.Title == "" && n.Message == "" {
		log.Printf("Validation failed: missing title and message. Payload: %s", buf.String())
		http.Error(w, "title or message required", http.StatusBadRequest)
		return
	}

	// Fetch icon URL and convert to base64
	if n.IconHref != "" {
		iconData, err := fetchAndEncodeIcon(n.IconHref)
		if err != nil {
			log.Printf("Failed to fetch icon from %s: %v", n.IconHref, err)
		} else {
			n.IconData = iconData
			n.IconHref = ""
		}
	}

	// Assign server ID for exclusive notifications
	if n.Exclusive && n.ID == "" {
		n.ID = uuid.New().String()
	}

	// Store exclusive notifications for later action handling
	if n.Exclusive {
		s.pendingMu.Lock()
		s.pending[n.ID] = n
		s.pendingMu.Unlock()
		log.Printf("Stored exclusive notification %s", n.ID)
	}

	s.Broadcast(n)
	w.WriteHeader(http.StatusAccepted)
}

func (s *Server) Broadcast(n Notification) {
	data, err := encodeMessage(MessageTypeNotification, n)
	if err != nil {
		log.Printf("Failed to marshal notification: %v", err)
		return
	}

	s.mu.RLock()
	clients := make([]*websocket.Conn, 0, len(s.clients))
	for conn := range s.clients {
		clients = append(clients, conn)
	}
	clientCount := len(clients)
	s.mu.RUnlock()

	log.Printf("Broadcasting notification to %d client(s)", clientCount)

	for _, conn := range clients {
		if err := conn.WriteMessage(websocket.TextMessage, data); err != nil {
			log.Printf("Failed to send to client: %v", err)
		}
	}
}

func (s *Server) BroadcastResolved(resolved ResolvedMessage) {
	data, err := encodeMessage(MessageTypeResolved, resolved)
	if err != nil {
		log.Printf("Failed to encode resolved message: %v", err)
		return
	}

	s.mu.RLock()
	clients := make([]*websocket.Conn, 0, len(s.clients))
	for conn := range s.clients {
		clients = append(clients, conn)
	}
	s.mu.RUnlock()

	for _, conn := range clients {
		if err := conn.WriteMessage(websocket.TextMessage, data); err != nil {
			log.Printf("Failed to send resolved to client: %v", err)
		}
	}
}

func (s *Server) ClientCount() int {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return len(s.clients)
}

func encodeMessage(msgType MessageType, data interface{}) ([]byte, error) {
	dataBytes, err := json.Marshal(data)
	if err != nil {
		return nil, err
	}
	msg := Message{
		Type: msgType,
		Data: dataBytes,
	}
	return json.Marshal(msg)
}

func decodeMessage(raw []byte) (*Message, error) {
	var msg Message
	if err := json.Unmarshal(raw, &msg); err != nil {
		return nil, err
	}
	return &msg, nil
}

func executeAction(a Action) error {
	if a.Open {
		return openURL(a.URL)
	}
	return executeHTTPAction(a)
}

func openURL(url string) error {
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "darwin":
		cmd = exec.Command("open", url)
	case "linux":
		cmd = exec.Command("xdg-open", url)
	case "windows":
		cmd = exec.Command("rundll32", "url.dll,FileProtocolHandler", url)
	default:
		return fmt.Errorf("unsupported platform: %s", runtime.GOOS)
	}
	return cmd.Start()
}

func executeHTTPAction(a Action) error {
	method := a.Method
	if method == "" {
		method = "GET"
	}
	method = strings.ToUpper(method)

	var bodyReader *strings.Reader
	if a.Body != "" {
		bodyReader = strings.NewReader(a.Body)
	}

	var req *http.Request
	var err error
	if bodyReader != nil {
		req, err = http.NewRequest(method, a.URL, bodyReader)
	} else {
		req, err = http.NewRequest(method, a.URL, nil)
	}
	if err != nil {
		return fmt.Errorf("failed to create request: %w", err)
	}

	for k, v := range a.Headers {
		req.Header.Set(k, v)
	}

	client := &http.Client{Timeout: 30 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("request failed: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("request returned status %d", resp.StatusCode)
	}

	return nil
}

func fetchAndEncodeIcon(url string) (string, error) {
	client := &http.Client{Timeout: 10 * time.Second}
	resp, err := client.Get(url)
	if err != nil {
		return "", fmt.Errorf("fetch: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("HTTP %d", resp.StatusCode)
	}

	img, _, err := image.Decode(resp.Body)
	if err != nil {
		return "", fmt.Errorf("decode: %w", err)
	}

	scaled := scaleImage(img, iconSize, iconSize)

	var buf bytes.Buffer
	if err := png.Encode(&buf, scaled); err != nil {
		return "", fmt.Errorf("encode: %w", err)
	}

	return base64.StdEncoding.EncodeToString(buf.Bytes()), nil
}

func scaleImage(src image.Image, width, height int) image.Image {
	srcBounds := src.Bounds()
	srcW := srcBounds.Dx()
	srcH := srcBounds.Dy()

	scaleX := float64(width) / float64(srcW)
	scaleY := float64(height) / float64(srcH)
	scale := scaleX
	if scaleY < scaleX {
		scale = scaleY
	}

	if scale >= 1.0 {
		dst := image.NewRGBA(image.Rect(0, 0, width, height))
		offsetX := (width - srcW) / 2
		offsetY := (height - srcH) / 2
		xdraw.Copy(dst, image.Point{X: offsetX, Y: offsetY}, src, srcBounds, xdraw.Over, nil)
		return dst
	}

	scaledW := int(float64(srcW) * scale)
	scaledH := int(float64(srcH) * scale)

	dst := image.NewRGBA(image.Rect(0, 0, width, height))
	offsetX := (width - scaledW) / 2
	offsetY := (height - scaledH) / 2
	dstRect := image.Rect(offsetX, offsetY, offsetX+scaledW, offsetY+scaledH)
	xdraw.CatmullRom.Scale(dst, dstRect, src, srcBounds, xdraw.Over, nil)
	return dst
}

func main() {
	port := flag.String("port", defaultPort, "Port to listen on")
	secret := flag.String("secret", "", "Shared secret for authentication")
	flag.Parse()

	if *port == defaultPort {
		if envPort := os.Getenv("CROSS_NOTIFIER_PORT"); envPort != "" {
			*port = envPort
		}
	}
	if *secret == "" {
		*secret = os.Getenv("CROSS_NOTIFIER_SECRET")
	}

	if *secret == "" {
		log.Fatal("Secret required: use -secret flag or CROSS_NOTIFIER_SECRET env")
	}

	server := NewServer(*secret)

	http.HandleFunc("/notify", server.HandleNotify)
	http.HandleFunc("/ws", server.HandleWebSocket)
	http.HandleFunc("/health", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ok"))
	})

	addr := ":" + *port
	log.Printf("Notification server listening on %s", addr)
	log.Printf("  POST /notify - send notifications (requires auth)")
	log.Printf("  GET  /ws     - WebSocket for clients (requires auth)")
	log.Printf("  GET  /health - health check (no auth)")

	if err := http.ListenAndServe(addr, nil); err != nil {
		log.Fatalf("Server failed: %v", err)
	}
}
