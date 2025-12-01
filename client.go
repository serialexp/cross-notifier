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

// NotificationClient connects to a notification server and receives notifications.
type NotificationClient struct {
	serverURL string
	secret    string
	onNotify  func(Notification)

	// Connection callbacks
	OnConnect    func()
	OnDisconnect func()

	// Reconnection settings
	MinReconnectDelay time.Duration
	MaxReconnectDelay time.Duration

	conn      *websocket.Conn
	mu        sync.RWMutex
	closed    bool
	closeChan chan struct{}
}

// NewNotificationClient creates a new client that connects to the given server.
func NewNotificationClient(serverURL, secret string, onNotify func(Notification)) *NotificationClient {
	return &NotificationClient{
		serverURL:         serverURL,
		secret:            secret,
		onNotify:          onNotify,
		MinReconnectDelay: time.Second,
		MaxReconnectDelay: 30 * time.Second,
		closeChan:         make(chan struct{}),
	}
}

// Connect establishes a connection to the server.
func (c *NotificationClient) Connect() error {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return nil
	}
	c.mu.Unlock()

	header := http.Header{}
	header.Set("Authorization", "Bearer "+c.secret)

	conn, _, err := websocket.DefaultDialer.Dial(c.serverURL, header)
	if err != nil {
		return err
	}

	c.mu.Lock()
	c.conn = conn
	c.mu.Unlock()

	if c.OnConnect != nil {
		c.OnConnect()
	}

	go c.readLoop()
	return nil
}

// readLoop reads messages from the server and handles disconnection.
func (c *NotificationClient) readLoop() {
	defer func() {
		c.mu.Lock()
		if c.conn != nil {
			c.conn.Close()
			c.conn = nil
		}
		closed := c.closed
		c.mu.Unlock()

		if c.OnDisconnect != nil {
			c.OnDisconnect()
		}

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

		_, msg, err := conn.ReadMessage()
		if err != nil {
			return
		}

		var n Notification
		if err := json.Unmarshal(msg, &n); err != nil {
			log.Printf("Failed to unmarshal notification: %v", err)
			continue
		}

		if c.onNotify != nil {
			c.onNotify(n)
		}
	}
}

// reconnectLoop attempts to reconnect with exponential backoff.
func (c *NotificationClient) reconnectLoop() {
	delay := c.MinReconnectDelay

	for {
		c.mu.RLock()
		closed := c.closed
		c.mu.RUnlock()

		if closed {
			return
		}

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

		log.Printf("Attempting to reconnect to %s...", c.serverURL)

		if err := c.Connect(); err != nil {
			log.Printf("Reconnection failed: %v", err)
			// Exponential backoff
			delay *= 2
			if delay > c.MaxReconnectDelay {
				delay = c.MaxReconnectDelay
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
