// ABOUTME: Persistent storage for notifications in the notification center.
// ABOUTME: Stores notifications as JSON on disk, survives daemon restarts.

package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
	"time"
)

// StoredNotification wraps a notification with metadata for persistence.
type StoredNotification struct {
	ID        int64           `json:"id"`
	Payload   json.RawMessage `json:"payload"` // Original JSON for reprocessing
	CreatedAt time.Time       `json:"createdAt"`
}

// NotificationStore manages persistent notification storage.
type NotificationStore struct {
	Notifications []StoredNotification `json:"notifications"`
	path          string
	mu            sync.RWMutex
	nextID        int64
}

// NewNotificationStore creates a store that persists to the given path.
func NewNotificationStore(path string) *NotificationStore {
	return &NotificationStore{
		path:   path,
		nextID: 1,
	}
}

// DefaultStorePath returns the platform-appropriate path for the notification store.
func DefaultStorePath() string {
	configDir, err := os.UserConfigDir()
	if err != nil {
		configDir = "."
	}
	return filepath.Join(configDir, "cross-notifier", "notifications.json")
}

// Load reads notifications from disk.
func (s *NotificationStore) Load() error {
	s.mu.Lock()
	defer s.mu.Unlock()

	data, err := os.ReadFile(s.path)
	if err != nil {
		if os.IsNotExist(err) {
			// No file yet, start empty
			s.Notifications = nil
			return nil
		}
		return err
	}

	if err := json.Unmarshal(data, s); err != nil {
		return err
	}

	// Find highest ID for nextID
	for _, n := range s.Notifications {
		if n.ID >= s.nextID {
			s.nextID = n.ID + 1
		}
	}

	return nil
}

// Save writes notifications to disk.
func (s *NotificationStore) Save() error {
	dir := filepath.Dir(s.path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}

	data, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return err
	}

	return os.WriteFile(s.path, data, 0600)
}

// Add stores a notification and persists to disk.
// Returns the assigned ID.
func (s *NotificationStore) Add(n Notification, rawJSON []byte) int64 {
	s.mu.Lock()
	defer s.mu.Unlock()

	id := s.nextID
	s.nextID++

	createdAt := time.Now()
	if !n.CreatedAt.IsZero() {
		createdAt = n.CreatedAt
	}
	stored := StoredNotification{
		ID:        id,
		Payload:   rawJSON,
		CreatedAt: createdAt,
	}

	s.Notifications = append(s.Notifications, stored)
	_ = s.Save() // Best effort persist

	return id
}

// Remove deletes a notification by ID and persists to disk.
func (s *NotificationStore) Remove(id int64) bool {
	s.mu.Lock()
	defer s.mu.Unlock()

	for i, n := range s.Notifications {
		if n.ID == id {
			s.Notifications = append(s.Notifications[:i], s.Notifications[i+1:]...)
			_ = s.Save() // Best effort persist
			return true
		}
	}
	return false
}

// Clear removes all notifications and persists to disk.
func (s *NotificationStore) Clear() {
	s.mu.Lock()
	defer s.mu.Unlock()

	s.Notifications = nil
	_ = s.Save()
}

// List returns all stored notifications.
func (s *NotificationStore) List() []StoredNotification {
	s.mu.RLock()
	defer s.mu.RUnlock()

	// Return a copy to avoid race conditions
	result := make([]StoredNotification, len(s.Notifications))
	copy(result, s.Notifications)
	return result
}

// Count returns the number of stored notifications.
func (s *NotificationStore) Count() int {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return len(s.Notifications)
}

// Get returns a single notification by ID, or nil if not found.
func (s *NotificationStore) Get(id int64) *StoredNotification {
	s.mu.RLock()
	defer s.mu.RUnlock()

	for _, n := range s.Notifications {
		if n.ID == id {
			return &n
		}
	}
	return nil
}

// ParseNotification decodes the stored payload into a Notification.
func (sn *StoredNotification) ParseNotification() (*Notification, error) {
	var n Notification
	if err := json.Unmarshal(sn.Payload, &n); err != nil {
		return nil, err
	}
	n.ID = sn.ID
	n.CreatedAt = sn.CreatedAt
	return &n, nil
}
