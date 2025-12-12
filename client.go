// ABOUTME: WebSocket client for connecting to a remote notification server.
// ABOUTME: Handles authentication, reconnection with backoff, and notification receiving.

package main

import (
	"encoding/json"
	"log"
	"net/http"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

const (
	// PingInterval is how often to send WebSocket pings to keep ALB connections alive.
	PingInterval = 30 * time.Second
)

// NotificationClient connects to a notification server and receives notifications.
type NotificationClient struct {
	serverURL  string
	secret     string
	name       string
	onNotify   func(Notification)
	onResolved func(ResolvedMessage)

	// Connection callbacks
	OnConnect    func()
	OnDisconnect func()

	// Reconnection settings
	MinReconnectDelay time.Duration
	MaxReconnectDelay time.Duration

	// Grace period for quick reconnects (no notifications shown if reconnect within this time)
	ReconnectGracePeriod time.Duration

	conn      *websocket.Conn
	mu        sync.RWMutex
	closed    bool
	closeChan chan struct{}
	pingStop  chan struct{} // signal to stop ping goroutine

	// Track disconnect time for grace period
	disconnectTime  time.Time
	disconnectTimer *time.Timer
	wasConnected    bool // track if we were ever connected (to avoid disconnect notification on initial connect failure)
}

// NewNotificationClient creates a new client that connects to the given server.
func NewNotificationClient(serverURL, secret, name string, onNotify func(Notification)) *NotificationClient {
	return &NotificationClient{
		serverURL:            serverURL,
		secret:               secret,
		name:                 name,
		onNotify:             onNotify,
		MinReconnectDelay:    time.Second,
		MaxReconnectDelay:    30 * time.Second,
		ReconnectGracePeriod: 2 * time.Second,
		closeChan:            make(chan struct{}),
	}
}

// Connect establishes a connection to the server.
func (c *NotificationClient) Connect() error {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return nil
	}

	// Cancel any pending disconnect notification
	if c.disconnectTimer != nil {
		c.disconnectTimer.Stop()
		c.disconnectTimer = nil
	}

	// Check if this is a quick reconnect (within grace period)
	quickReconnect := !c.disconnectTime.IsZero() && time.Since(c.disconnectTime) < c.ReconnectGracePeriod

	c.mu.Unlock()

	header := http.Header{}
	header.Set("Authorization", "Bearer "+c.secret)
	if c.name != "" {
		header.Set("X-Client-Name", c.name)
	}

	conn, _, err := websocket.DefaultDialer.Dial(c.serverURL, header)
	if err != nil {
		return err
	}

	c.mu.Lock()
	c.conn = conn
	c.disconnectTime = time.Time{} // Clear disconnect time on successful connect
	isInitialConnect := !c.wasConnected
	c.wasConnected = true
	c.mu.Unlock()

	// Show connect notification:
	// - On initial connect (user wants to know connection succeeded)
	// - On slow reconnect (after grace period, disconnect notification was shown)
	// Skip notification only on quick reconnect (silent recovery)
	if c.OnConnect != nil && (isInitialConnect || !quickReconnect) {
		c.OnConnect()
	}

	// Start ping goroutine to keep connection alive through ALBs
	c.mu.Lock()
	c.pingStop = make(chan struct{})
	c.mu.Unlock()
	go c.pingLoop()

	go c.readLoop()
	return nil
}

// readLoop reads messages from the server and handles disconnection.
func (c *NotificationClient) readLoop() {
	defer func() {
		c.mu.Lock()
		// Stop ping goroutine
		if c.pingStop != nil {
			close(c.pingStop)
			c.pingStop = nil
		}
		if c.conn != nil {
			c.conn.Close()
			c.conn = nil
		}
		closed := c.closed
		c.disconnectTime = time.Now()

		// Schedule disconnect notification after grace period
		// (will be cancelled if we reconnect quickly)
		if c.OnDisconnect != nil && !closed {
			c.disconnectTimer = time.AfterFunc(c.ReconnectGracePeriod, func() {
				c.mu.Lock()
				c.disconnectTimer = nil
				c.mu.Unlock()
				c.OnDisconnect()
			})
		}
		c.mu.Unlock()

		// Attempt reconnection if not intentionally closed
		if !closed {
			go c.reconnectLoop()
		}
	}()

	for {
		c.mu.RLock()
		conn := c.conn
		c.mu.RUnlock()

		if conn == nil {
			return
		}

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
		case MessageTypeNotification:
			var n Notification
			if err := json.Unmarshal(msg.Data, &n); err != nil {
				log.Printf("Failed to unmarshal notification: %v", err)
				continue
			}
			log.Printf("Received notification: %s - %s", n.Title, n.Message)
			if c.onNotify != nil {
				c.onNotify(n)
			}

		case MessageTypeResolved:
			var resolved ResolvedMessage
			if err := json.Unmarshal(msg.Data, &resolved); err != nil {
				log.Printf("Failed to unmarshal resolved message: %v", err)
				continue
			}
			if c.onResolved != nil {
				c.onResolved(resolved)
			}
		}
	}
}

// pingLoop sends periodic pings to keep the connection alive through load balancers.
func (c *NotificationClient) pingLoop() {
	ticker := time.NewTicker(PingInterval)
	defer ticker.Stop()

	for {
		select {
		case <-ticker.C:
			c.mu.RLock()
			conn := c.conn
			c.mu.RUnlock()

			if conn == nil {
				return
			}

			if err := conn.WriteControl(websocket.PingMessage, nil, time.Now().Add(10*time.Second)); err != nil {
				log.Printf("Ping failed: %v", err)
				return
			}
		case <-c.pingStop:
			return
		}
	}
}

// reconnectLoop attempts to reconnect with exponential backoff.
func (c *NotificationClient) reconnectLoop() {
	delay := time.Duration(0) // Try immediately first

	for {
		c.mu.RLock()
		closed := c.closed
		c.mu.RUnlock()

		if closed {
			return
		}

		// Wait before reconnecting (0 on first attempt)
		if delay > 0 {
			select {
			case <-c.closeChan:
				return
			case <-time.After(delay):
			}

			c.mu.RLock()
			closed = c.closed
			c.mu.RUnlock()

			if closed {
				return
			}
		}

		log.Printf("Attempting to reconnect to %s...", c.serverURL)

		if err := c.Connect(); err != nil {
			log.Printf("Reconnection failed: %v", err)
			// Exponential backoff starting from MinReconnectDelay
			if delay == 0 {
				delay = c.MinReconnectDelay
			} else {
				delay *= 2
				if delay > c.MaxReconnectDelay {
					delay = c.MaxReconnectDelay
				}
			}
			continue
		}

		// Successfully reconnected
		return
	}
}

// Close disconnects from the server and stops reconnection attempts.
func (c *NotificationClient) Close() {
	c.mu.Lock()
	c.closed = true
	if c.conn != nil {
		c.conn.Close()
		c.conn = nil
	}
	// Cancel any pending disconnect notification
	if c.disconnectTimer != nil {
		c.disconnectTimer.Stop()
		c.disconnectTimer = nil
	}
	c.mu.Unlock()

	// Signal reconnect loop to stop
	select {
	case c.closeChan <- struct{}{}:
	default:
	}
}

// IsConnected returns true if the client is currently connected.
func (c *NotificationClient) IsConnected() bool {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return c.conn != nil
}

// SetOnResolved sets the callback for resolved messages.
func (c *NotificationClient) SetOnResolved(handler func(ResolvedMessage)) {
	c.onResolved = handler
}

// SendAction sends an action message to the server for an exclusive notification.
func (c *NotificationClient) SendAction(notificationID string, actionIndex int) error {
	c.mu.RLock()
	conn := c.conn
	c.mu.RUnlock()

	if conn == nil {
		return nil // Not connected, action will be handled locally
	}

	msg := ActionMessage{
		NotificationID: notificationID,
		ActionIndex:    actionIndex,
	}

	data, err := EncodeMessage(MessageTypeAction, msg)
	if err != nil {
		return err
	}

	return conn.WriteMessage(websocket.TextMessage, data)
}
