// ABOUTME: Standalone notification server binary for Docker/headless deployment.
// ABOUTME: Does not include GUI dependencies.

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
	"log"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
	xdraw "golang.org/x/image/draw"
)

const (
	defaultPort = "9876"
	iconSize    = 48
)

// Notification represents a notification message.
type Notification struct {
	Title    string `json:"title"`
	Message  string `json:"message"`
	IconData string `json:"iconData,omitempty"`
	IconHref string `json:"iconHref,omitempty"`
	Duration int    `json:"duration,omitempty"`
}

var upgrader = websocket.Upgrader{
	CheckOrigin: func(r *http.Request) bool {
		return true
	},
}

// Server manages WebSocket connections and broadcasts notifications.
type Server struct {
	secret  string
	clients map[*websocket.Conn]bool
	mu      sync.RWMutex
}

func NewServer(secret string) *Server {
	return &Server{
		secret:  secret,
		clients: make(map[*websocket.Conn]bool),
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

	s.mu.Lock()
	s.clients[conn] = true
	s.mu.Unlock()

	log.Printf("Client connected (%d total)", s.ClientCount())

	go s.handleClient(conn)
}

func (s *Server) handleClient(conn *websocket.Conn) {
	defer func() {
		s.mu.Lock()
		delete(s.clients, conn)
		s.mu.Unlock()
		conn.Close()
		log.Printf("Client disconnected (%d remaining)", s.ClientCount())
	}()

	for {
		_, _, err := conn.ReadMessage()
		if err != nil {
			return
		}
	}
}

func (s *Server) HandleNotify(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST required", http.StatusMethodNotAllowed)
		return
	}

	if !s.checkAuth(r) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
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

	s.Broadcast(n)
	w.WriteHeader(http.StatusAccepted)
}

func (s *Server) Broadcast(n Notification) {
	data, err := json.Marshal(n)
	if err != nil {
		log.Printf("Failed to marshal notification: %v", err)
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
			log.Printf("Failed to send to client: %v", err)
		}
	}
}

func (s *Server) ClientCount() int {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return len(s.clients)
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

	// Scale down
	scaled := scaleImage(img, iconSize, iconSize)

	// Encode to PNG then base64
	var buf bytes.Buffer
	if err := png.Encode(&buf, scaled); err != nil {
		return "", fmt.Errorf("encode: %w", err)
	}

	return base64.StdEncoding.EncodeToString(buf.Bytes()), nil
}

func scaleImage(src image.Image, width, height int) image.Image {
	bounds := src.Bounds()
	if bounds.Dx() <= width && bounds.Dy() <= height {
		return src
	}
	dst := image.NewRGBA(image.Rect(0, 0, width, height))
	xdraw.CatmullRom.Scale(dst, dst.Bounds(), src, bounds, xdraw.Over, nil)
	return dst
}

func main() {
	port := flag.String("port", defaultPort, "Port to listen on")
	secret := flag.String("secret", "", "Shared secret for authentication")
	flag.Parse()

	// Environment variable fallbacks
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

	addr := ":" + *port
	log.Printf("Notification server listening on %s", addr)
	log.Printf("  POST /notify - send notifications (requires auth)")
	log.Printf("  GET  /ws     - WebSocket for clients (requires auth)")

	if err := http.ListenAndServe(addr, nil); err != nil {
		log.Fatalf("Server failed: %v", err)
	}
}
