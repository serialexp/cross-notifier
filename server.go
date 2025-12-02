// ABOUTME: WebSocket server for broadcasting notifications to connected clients.
// ABOUTME: Accepts HTTP POST notifications and forwards them to all clients.

package main

import (
	"encoding/json"
	"log"
	"net/http"
	"strings"
	"sync"

	"github.com/google/uuid"
	"github.com/gorilla/websocket"
)

var upgrader = websocket.Upgrader{
	CheckOrigin: func(r *http.Request) bool {
		return true // Allow connections from any origin
	},
}

// ClientInfo holds information about a connected client.
type ClientInfo struct {
	Name string
}

// NotificationServer manages WebSocket connections and broadcasts notifications.
type NotificationServer struct {
	secret    string
	clients   map[*websocket.Conn]*ClientInfo
	pending   map[string]Notification // ServerID -> Notification for exclusive notifications
	mu        sync.RWMutex
	pendingMu sync.RWMutex
}

// NewNotificationServer creates a new server with the given authentication secret.
func NewNotificationServer(secret string) *NotificationServer {
	return &NotificationServer{
		secret:  secret,
		clients: make(map[*websocket.Conn]*ClientInfo),
		pending: make(map[string]Notification),
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

	// Handle client connection lifecycle
	go s.handleClient(conn)
}

// handleClient reads messages from the client and handles disconnection.
func (s *NotificationServer) handleClient(conn *websocket.Conn) {
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

	// Get client info for this connection
	s.mu.RLock()
	clientInfo := s.clients[conn]
	s.mu.RUnlock()

	clientName := ""
	if clientInfo != nil {
		clientName = clientInfo.Name
	}

	// Read loop for messages
	for {
		_, raw, err := conn.ReadMessage()
		if err != nil {
			return
		}

		msg, err := DecodeMessage(raw)
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

// handleActionMessage processes an action request from a client.
func (s *NotificationServer) handleActionMessage(clientName string, msg ActionMessage) {
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

	// Execute the action
	var execErr error
	if action.Open {
		// For open actions, we can't open a browser on the server
		// Just mark as success - client should handle opening URLs locally
		log.Printf("Open action requested - client should handle locally")
	} else {
		execErr = ExecuteAction(action)
	}

	// Broadcast resolved message
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

// HandleNotify handles HTTP POST requests to send notifications.
func (s *NotificationServer) HandleNotify(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		log.Printf("HandleNotify: rejected non-POST request")
		http.Error(w, "POST required", http.StatusMethodNotAllowed)
		return
	}

	if !s.checkAuth(r) {
		log.Printf("HandleNotify: rejected unauthorized request")
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}

	var n Notification
	if err := json.NewDecoder(r.Body).Decode(&n); err != nil {
		log.Printf("HandleNotify: failed to decode JSON: %v", err)
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	if n.Title == "" && n.Message == "" {
		log.Printf("HandleNotify: rejected notification with empty title and message")
		http.Error(w, "title or message required", http.StatusBadRequest)
		return
	}

	log.Printf("HandleNotify: received notification - Title: %q, Message: %q", n.Title, n.Message)

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

	// Assign server ID for exclusive notifications
	if n.Exclusive && n.ServerID == "" {
		n.ServerID = uuid.New().String()
	}

	// Store exclusive notifications for later action handling
	if n.Exclusive {
		s.pendingMu.Lock()
		s.pending[n.ServerID] = n
		s.pendingMu.Unlock()
		log.Printf("Stored exclusive notification %s", n.ServerID)
	}

	s.Broadcast(n)
	w.WriteHeader(http.StatusAccepted)
}

// Broadcast sends a notification to all connected clients.
func (s *NotificationServer) Broadcast(n Notification) {
	data, err := EncodeMessage(MessageTypeNotification, n)
	if err != nil {
		log.Printf("Broadcast: failed to encode notification message: %v", err)
		return
	}

	s.mu.RLock()
	clients := make([]*websocket.Conn, 0, len(s.clients))
	for conn := range s.clients {
		clients = append(clients, conn)
	}
	clientCount := len(clients)
	s.mu.RUnlock()

	log.Printf("Broadcast: sending notification to %d connected client(s)", clientCount)

	successCount := 0
	for _, conn := range clients {
		if err := conn.WriteMessage(websocket.TextMessage, data); err != nil {
			log.Printf("Broadcast: failed to send to client: %v", err)
			// Connection will be cleaned up by read loop
		} else {
			successCount++
		}
	}

	log.Printf("Broadcast: successfully sent to %d/%d client(s)", successCount, clientCount)
}

// BroadcastResolved sends a resolved message to all connected clients.
func (s *NotificationServer) BroadcastResolved(resolved ResolvedMessage) {
	data, err := EncodeMessage(MessageTypeResolved, resolved)
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

// ClientCount returns the number of connected clients.
func (s *NotificationServer) ClientCount() int {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return len(s.clients)
}
