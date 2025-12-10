// ABOUTME: Tests for notification store persistence.
// ABOUTME: Verifies add, remove, list, and disk persistence.

package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestStoreAddAndList(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "notifications.json")
	store := NewNotificationStore(path)

	n := Notification{Title: "Test", Message: "Hello"}
	rawJSON := []byte(`{"title":"Test","message":"Hello"}`)

	id := store.Add(n, rawJSON)
	if id != 1 {
		t.Errorf("expected id 1, got %d", id)
	}

	list := store.List()
	if len(list) != 1 {
		t.Fatalf("expected 1 notification, got %d", len(list))
	}

	if list[0].ID != 1 {
		t.Errorf("expected ID 1, got %d", list[0].ID)
	}
}

func TestStoreRemove(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "notifications.json")
	store := NewNotificationStore(path)

	rawJSON := []byte(`{"title":"Test"}`)
	id1 := store.Add(Notification{Title: "Test1"}, rawJSON)
	id2 := store.Add(Notification{Title: "Test2"}, rawJSON)

	if store.Count() != 2 {
		t.Fatalf("expected 2 notifications, got %d", store.Count())
	}

	removed := store.Remove(id1)
	if !removed {
		t.Error("expected Remove to return true")
	}

	if store.Count() != 1 {
		t.Fatalf("expected 1 notification after remove, got %d", store.Count())
	}

	list := store.List()
	if list[0].ID != id2 {
		t.Errorf("expected remaining notification to have ID %d, got %d", id2, list[0].ID)
	}
}

func TestStorePersistence(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "notifications.json")

	// Create store and add notification
	store1 := NewNotificationStore(path)
	rawJSON := []byte(`{"title":"Persistent","message":"Data"}`)
	store1.Add(Notification{Title: "Persistent", Message: "Data"}, rawJSON)

	// Create new store and load from disk
	store2 := NewNotificationStore(path)
	if err := store2.Load(); err != nil {
		t.Fatalf("failed to load: %v", err)
	}

	if store2.Count() != 1 {
		t.Fatalf("expected 1 notification after load, got %d", store2.Count())
	}

	// Verify nextID is correct after load
	id := store2.Add(Notification{Title: "New"}, []byte(`{}`))
	if id != 2 {
		t.Errorf("expected nextID to be 2 after load, got %d", id)
	}
}

func TestStoreClear(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "notifications.json")
	store := NewNotificationStore(path)

	rawJSON := []byte(`{}`)
	store.Add(Notification{}, rawJSON)
	store.Add(Notification{}, rawJSON)

	if store.Count() != 2 {
		t.Fatalf("expected 2 notifications, got %d", store.Count())
	}

	store.Clear()

	if store.Count() != 0 {
		t.Errorf("expected 0 notifications after clear, got %d", store.Count())
	}

	// Verify persisted
	store2 := NewNotificationStore(path)
	if err := store2.Load(); err != nil {
		t.Fatalf("failed to load: %v", err)
	}
	if store2.Count() != 0 {
		t.Errorf("expected 0 notifications after reload, got %d", store2.Count())
	}
}

func TestStoreLoadNonexistent(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "nonexistent.json")
	store := NewNotificationStore(path)

	err := store.Load()
	if err != nil {
		t.Errorf("expected no error for nonexistent file, got %v", err)
	}

	if store.Count() != 0 {
		t.Errorf("expected empty store, got %d", store.Count())
	}
}

func TestStoreParseNotification(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "notifications.json")
	store := NewNotificationStore(path)

	rawJSON := []byte(`{"title":"Test Title","message":"Test Message","status":"success"}`)
	id := store.Add(Notification{}, rawJSON)

	stored := store.Get(id)
	if stored == nil {
		t.Fatal("expected to find notification")
	}

	n, err := stored.ParseNotification()
	if err != nil {
		t.Fatalf("failed to parse: %v", err)
	}

	if n.Title != "Test Title" {
		t.Errorf("expected title 'Test Title', got %q", n.Title)
	}
	if n.Message != "Test Message" {
		t.Errorf("expected message 'Test Message', got %q", n.Message)
	}
	if n.Status != "success" {
		t.Errorf("expected status 'success', got %q", n.Status)
	}
	if n.ID != id {
		t.Errorf("expected ID %d, got %d", id, n.ID)
	}
}

func TestStoreGet(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "notifications.json")
	store := NewNotificationStore(path)

	rawJSON := []byte(`{"title":"Find Me"}`)
	id := store.Add(Notification{Title: "Find Me"}, rawJSON)

	found := store.Get(id)
	if found == nil {
		t.Fatal("expected to find notification")
	}

	notFound := store.Get(999)
	if notFound != nil {
		t.Error("expected nil for nonexistent ID")
	}
}

func TestStoreRemoveNonexistent(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "notifications.json")
	store := NewNotificationStore(path)

	removed := store.Remove(999)
	if removed {
		t.Error("expected Remove to return false for nonexistent ID")
	}
}

// Ensure the file is actually created
func TestStoreCreatesFile(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "subdir", "notifications.json")
	store := NewNotificationStore(path)

	store.Add(Notification{}, []byte(`{}`))

	if _, err := os.Stat(path); os.IsNotExist(err) {
		t.Error("expected file to be created")
	}
}
