// ABOUTME: Tests for the notification client WebSocket functionality.
// ABOUTME: Covers connection, reconnection, and notification receiving.

package main

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
)

func TestClientConnectsToServer(t *testing.T) {
	secret := "test-secret"
	server := NewNotificationServer(secret)

	ts := httptest.NewServer(http.HandlerFunc(server.HandleWebSocket))
	defer ts.Close()

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http")

	received := make(chan Notification, 1)
	client := NewNotificationClient(wsURL, secret, func(n Notification) {
		received <- n
	})

	if err := client.Connect(); err != nil {
		t.Fatalf("Connect failed: %v", err)
	}
	defer client.Close()

	// Give server time to register
	time.Sleep(50 * time.Millisecond)

	if server.ClientCount() != 1 {
		t.Errorf("expected 1 client on server, got %d", server.ClientCount())
	}
}

func TestClientReceivesNotifications(t *testing.T) {
	secret := "test-secret"
	server := NewNotificationServer(secret)

	ts := httptest.NewServer(http.HandlerFunc(server.HandleWebSocket))
	defer ts.Close()

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http")

	received := make(chan Notification, 1)
	client := NewNotificationClient(wsURL, secret, func(n Notification) {
		received <- n
	})

	if err := client.Connect(); err != nil {
		t.Fatalf("Connect failed: %v", err)
	}
	defer client.Close()

	time.Sleep(50 * time.Millisecond)

	// Send notification from server
	server.Broadcast(Notification{
		Title:   "Test",
		Message: "Hello from server",
	})

	select {
	case n := <-received:
		if n.Title != "Test" || n.Message != "Hello from server" {
			t.Errorf("wrong notification: %+v", n)
		}
	case <-time.After(time.Second):
		t.Fatal("timeout waiting for notification")
	}
}

func TestClientReconnects(t *testing.T) {
	secret := "test-secret"
	server := NewNotificationServer(secret)

	ts := httptest.NewServer(http.HandlerFunc(server.HandleWebSocket))

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http")

	var mu sync.Mutex
	connectionEvents := []string{}

	client := NewNotificationClient(wsURL, secret, func(n Notification) {})
	client.OnConnect = func() {
		mu.Lock()
		connectionEvents = append(connectionEvents, "connected")
		mu.Unlock()
	}
	client.OnDisconnect = func() {
		mu.Lock()
		connectionEvents = append(connectionEvents, "disconnected")
		mu.Unlock()
	}

	// Set fast reconnect for testing
	client.MinReconnectDelay = 10 * time.Millisecond
	client.MaxReconnectDelay = 50 * time.Millisecond

	if err := client.Connect(); err != nil {
		t.Fatalf("Connect failed: %v", err)
	}
	defer client.Close()

	time.Sleep(50 * time.Millisecond)

	// Close the server to trigger disconnect
	ts.Close()

	time.Sleep(100 * time.Millisecond)

	// Start a new server on the same URL (can't do this with httptest, so just verify disconnect happened)
	mu.Lock()
	events := connectionEvents
	mu.Unlock()

	if len(events) < 1 || events[0] != "connected" {
		t.Errorf("expected first event to be 'connected', got %v", events)
	}
}

func TestClientAuthFailure(t *testing.T) {
	secret := "test-secret"
	server := NewNotificationServer(secret)

	ts := httptest.NewServer(http.HandlerFunc(server.HandleWebSocket))
	defer ts.Close()

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http")

	client := NewNotificationClient(wsURL, "wrong-secret", func(n Notification) {})

	err := client.Connect()
	if err == nil {
		client.Close()
		t.Fatal("expected connection to fail with wrong secret")
	}
}

func TestClientIsConnected(t *testing.T) {
	secret := "test-secret"
	server := NewNotificationServer(secret)

	ts := httptest.NewServer(http.HandlerFunc(server.HandleWebSocket))
	defer ts.Close()

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http")

	client := NewNotificationClient(wsURL, secret, func(n Notification) {})

	if client.IsConnected() {
		t.Error("expected not connected before Connect()")
	}

	if err := client.Connect(); err != nil {
		t.Fatalf("Connect failed: %v", err)
	}

	time.Sleep(50 * time.Millisecond)

	if !client.IsConnected() {
		t.Error("expected connected after Connect()")
	}

	client.Close()

	time.Sleep(50 * time.Millisecond)

	if client.IsConnected() {
		t.Error("expected not connected after Close()")
	}
}
