// ABOUTME: WebSocket server for broadcasting notifications to connected clients.
// ABOUTME: Accepts HTTP POST notifications and forwards them to all clients.

package main

import (
	"encoding/json"
	"log"
	"net/http"
	"strings"
	"sync"

	"github.com/gorilla/websocket"
)

var upgrader = websocket.Upgrader{
	CheckOrigin: func(r *http.Request) bool {
		return true // Allow connections from any origin
	},
}

// NotificationServer manages WebSocket connections and broadcasts notifications.
type NotificationServer struct {
	secret  string
	clients map[*websocket.Conn]bool
	mu      sync.RWMutex
}

// NewNotificationServer creates a new server with the given authentication secret.
func NewNotificationServer(secret string) *NotificationServer {
	return &NotificationServer{
		secret:  secret,
		clients: make(map[*websocket.Conn]bool),
	}
}

// checkAuth validates the Authorization header against the server secret.
func (s *NotificationServer) checkAuth(r *http.Request) bool {
	auth := r.Header.Get("Authorization")
	if !strings.HasPrefix(auth, "Bearer ") {
		return false
	}
	token := strings.TrimPrefix(auth, "Bearer ")
	return token == s.secret
}

// HandleWebSocket handles WebSocket connection upgrades.
func (s *NotificationServer) HandleWebSocket(w http.ResponseWriter, r *http.Request) {
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

	// Handle client connection lifecycle
	go s.handleClient(conn)
}

// handleClient reads from the client connection to detect disconnection.
func (s *NotificationServer) handleClient(conn *websocket.Conn) {
	defer func() {
		s.mu.Lock()
		delete(s.clients, conn)
		s.mu.Unlock()
		conn.Close()
		log.Printf("Client disconnected (%d remaining)", s.ClientCount())
	}()

	// Read loop to detect disconnection
	for {
		_, _, err := conn.ReadMessage()
		if err != nil {
			return
		}
	}
}

// HandleNotify handles HTTP POST requests to send notifications.
func (s *NotificationServer) HandleNotify(w http.ResponseWriter, r *http.Request) {
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

	// Process icon: fetch URL if provided, convert to base64
	if n.IconHref != "" {
		iconData, err := fetchAndEncodeIcon(n.IconHref)
		if err != nil {
			log.Printf("Failed to fetch icon from %s: %v", n.IconHref, err)
		} else {
			n.IconData = iconData
			n.IconHref = "" // Clear href since we've converted it
		}
	}

	s.Broadcast(n)
	w.WriteHeader(http.StatusAccepted)
}

// Broadcast sends a notification to all connected clients.
func (s *NotificationServer) Broadcast(n Notification) {
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
			// Connection will be cleaned up by read loop
		}
	}
}

// ClientCount returns the number of connected clients.
func (s *NotificationServer) ClientCount() int {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return len(s.clients)
}
