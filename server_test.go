// ABOUTME: Tests for the notification server WebSocket functionality.
// ABOUTME: Covers client connections, authentication, and notification broadcasting.

package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/gorilla/websocket"
)

func TestServerAcceptsAuthenticatedConnection(t *testing.T) {
	secret := "test-secret-key"
	server := NewNotificationServer(secret)

	ts := httptest.NewServer(http.HandlerFunc(server.HandleWebSocket))
	defer ts.Close()

	// Convert http:// to ws://
	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http")

	// Connect with correct auth
	header := http.Header{}
	header.Set("Authorization", "Bearer "+secret)
	conn, resp, err := websocket.DefaultDialer.Dial(wsURL, header)
	if err != nil {
		t.Fatalf("failed to connect: %v", err)
	}
	defer conn.Close()

	if resp.StatusCode != http.StatusSwitchingProtocols {
		t.Errorf("expected 101 Switching Protocols, got %d", resp.StatusCode)
	}
}

func TestServerRejectsUnauthenticatedConnection(t *testing.T) {
	secret := "test-secret-key"
	server := NewNotificationServer(secret)

	ts := httptest.NewServer(http.HandlerFunc(server.HandleWebSocket))
	defer ts.Close()

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http")

	// Connect without auth
	_, resp, err := websocket.DefaultDialer.Dial(wsURL, nil)
	if err == nil {
		t.Fatal("expected connection to fail")
	}
	if resp != nil && resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("expected 401 Unauthorized, got %d", resp.StatusCode)
	}
}

func TestServerRejectsWrongSecret(t *testing.T) {
	secret := "test-secret-key"
	server := NewNotificationServer(secret)

	ts := httptest.NewServer(http.HandlerFunc(server.HandleWebSocket))
	defer ts.Close()

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http")

	// Connect with wrong secret
	header := http.Header{}
	header.Set("Authorization", "Bearer wrong-secret")
	_, resp, err := websocket.DefaultDialer.Dial(wsURL, header)
	if err == nil {
		t.Fatal("expected connection to fail")
	}
	if resp != nil && resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("expected 401 Unauthorized, got %d", resp.StatusCode)
	}
}

func TestServerBroadcastsNotifications(t *testing.T) {
	secret := "test-secret-key"
	server := NewNotificationServer(secret)

	ts := httptest.NewServer(http.HandlerFunc(server.HandleWebSocket))
	defer ts.Close()

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http")

	// Connect two clients
	header := http.Header{}
	header.Set("Authorization", "Bearer "+secret)

	conn1, _, err := websocket.DefaultDialer.Dial(wsURL, header)
	if err != nil {
		t.Fatalf("client 1 failed to connect: %v", err)
	}
	defer conn1.Close()

	conn2, _, err := websocket.DefaultDialer.Dial(wsURL, header)
	if err != nil {
		t.Fatalf("client 2 failed to connect: %v", err)
	}
	defer conn2.Close()

	// Give server time to register clients
	time.Sleep(50 * time.Millisecond)

	// Broadcast a notification
	notif := Notification{
		Title:   "Test",
		Message: "Hello",
	}
	server.Broadcast(notif)

	// Both clients should receive it
	for i, conn := range []*websocket.Conn{conn1, conn2} {
		_ = conn.SetReadDeadline(time.Now().Add(time.Second))
		_, raw, err := conn.ReadMessage()
		if err != nil {
			t.Errorf("client %d failed to read: %v", i+1, err)
			continue
		}

		msg, err := DecodeMessage(raw)
		if err != nil {
			t.Errorf("client %d failed to decode message: %v", i+1, err)
			continue
		}

		if msg.Type != MessageTypeNotification {
			t.Errorf("client %d got wrong message type: %s", i+1, msg.Type)
			continue
		}

		var received Notification
		if err := json.Unmarshal(msg.Data, &received); err != nil {
			t.Errorf("client %d failed to unmarshal notification: %v", i+1, err)
			continue
		}

		if received.Title != notif.Title || received.Message != notif.Message {
			t.Errorf("client %d received wrong notification: %+v", i+1, received)
		}
	}
}

func TestServerHTTPNotifyEndpoint(t *testing.T) {
	secret := "test-secret-key"
	server := NewNotificationServer(secret)

	// Set up WebSocket endpoint
	wsts := httptest.NewServer(http.HandlerFunc(server.HandleWebSocket))
	defer wsts.Close()

	// Connect a client
	wsURL := "ws" + strings.TrimPrefix(wsts.URL, "http")
	header := http.Header{}
	header.Set("Authorization", "Bearer "+secret)
	conn, _, err := websocket.DefaultDialer.Dial(wsURL, header)
	if err != nil {
		t.Fatalf("failed to connect: %v", err)
	}
	defer conn.Close()

	// Give server time to register client
	time.Sleep(50 * time.Millisecond)

	// Set up HTTP notify endpoint
	httpts := httptest.NewServer(http.HandlerFunc(server.HandleNotify))
	defer httpts.Close()

	// Send notification via HTTP
	body := `{"title": "HTTP Test", "message": "Via HTTP"}`
	req, _ := http.NewRequest("POST", httpts.URL, strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+secret)

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("HTTP request failed: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusAccepted {
		t.Errorf("expected 202 Accepted, got %d", resp.StatusCode)
	}

	// Client should receive the notification
	_ = conn.SetReadDeadline(time.Now().Add(time.Second))
	_, raw, err := conn.ReadMessage()
	if err != nil {
		t.Fatalf("failed to read notification: %v", err)
	}

	msg, err := DecodeMessage(raw)
	if err != nil {
		t.Fatalf("failed to decode message: %v", err)
	}

	if msg.Type != MessageTypeNotification {
		t.Fatalf("wrong message type: %s", msg.Type)
	}

	var received Notification
	if err := json.Unmarshal(msg.Data, &received); err != nil {
		t.Fatalf("failed to unmarshal notification: %v", err)
	}

	if received.Title != "HTTP Test" {
		t.Errorf("wrong title: %s", received.Title)
	}
}

func TestServerHTTPNotifyRequiresAuth(t *testing.T) {
	secret := "test-secret-key"
	server := NewNotificationServer(secret)

	ts := httptest.NewServer(http.HandlerFunc(server.HandleNotify))
	defer ts.Close()

	// Send without auth
	body := `{"title": "Test"}`
	resp, err := http.Post(ts.URL, "application/json", strings.NewReader(body))
	if err != nil {
		t.Fatalf("HTTP request failed: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("expected 401 Unauthorized, got %d", resp.StatusCode)
	}
}

func TestServerClientCount(t *testing.T) {
	secret := "test-secret-key"
	server := NewNotificationServer(secret)

	ts := httptest.NewServer(http.HandlerFunc(server.HandleWebSocket))
	defer ts.Close()

	if server.ClientCount() != 0 {
		t.Errorf("expected 0 clients, got %d", server.ClientCount())
	}

	wsURL := "ws" + strings.TrimPrefix(ts.URL, "http")
	header := http.Header{}
	header.Set("Authorization", "Bearer "+secret)

	conn1, _, err := websocket.DefaultDialer.Dial(wsURL, header)
	if err != nil {
		t.Fatalf("failed to connect: %v", err)
	}

	// Give server time to register
	time.Sleep(50 * time.Millisecond)

	if server.ClientCount() != 1 {
		t.Errorf("expected 1 client, got %d", server.ClientCount())
	}

	conn2, _, err := websocket.DefaultDialer.Dial(wsURL, header)
	if err != nil {
		t.Fatalf("failed to connect: %v", err)
	}

	time.Sleep(50 * time.Millisecond)

	if server.ClientCount() != 2 {
		t.Errorf("expected 2 clients, got %d", server.ClientCount())
	}

	conn1.Close()
	time.Sleep(50 * time.Millisecond)

	if server.ClientCount() != 1 {
		t.Errorf("expected 1 client after disconnect, got %d", server.ClientCount())
	}

	conn2.Close()
}
