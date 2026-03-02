// Persistent notification store for the notification center.
// Stores notifications as JSON, same format/location as the Go daemon.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::notification::NotificationPayload;

pub type SharedStore = Arc<std::sync::RwLock<NotificationStore>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredNotification {
    pub id: i64,
    pub payload: NotificationPayload,
    pub server_label: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
struct StoreFile {
    notifications: Vec<StoredNotification>,
}

pub struct NotificationStore {
    notifications: Vec<StoredNotification>,
    path: PathBuf,
    next_id: i64,
}

impl NotificationStore {
    /// Load store from disk, or create empty if file doesn't exist.
    pub fn load(path: PathBuf) -> Self {
        let notifications = match std::fs::read_to_string(&path) {
            Ok(data) => match serde_json::from_str::<StoreFile>(&data) {
                Ok(file) => {
                    info!("Loaded {} stored notifications", file.notifications.len());
                    file.notifications
                }
                Err(e) => {
                    warn!("Failed to parse notification store: {}", e);
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };

        let next_id = notifications.iter().map(|n| n.id).max().unwrap_or(0) + 1;

        Self {
            notifications,
            path,
            next_id,
        }
    }

    /// Default store path (same location as Go daemon).
    pub fn default_path() -> PathBuf {
        let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        config_dir
            .join("cross-notifier")
            .join("notifications.json")
    }

    fn save(&self) {
        let file = StoreFile {
            notifications: self.notifications.clone(),
        };
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(&file) {
            Ok(data) => {
                if let Err(e) = std::fs::write(&self.path, data) {
                    warn!("Failed to save notification store: {}", e);
                }
            }
            Err(e) => warn!("Failed to serialize notification store: {}", e),
        }
    }

    /// Add a notification to the store. Returns the assigned store ID.
    pub fn add(&mut self, payload: NotificationPayload, server_label: String) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.notifications.push(StoredNotification {
            id,
            payload,
            server_label,
            created_at: Utc::now(),
        });
        self.save();
        id
    }

    /// Remove a notification by ID. Returns true if found.
    pub fn remove(&mut self, id: i64) -> bool {
        if let Some(pos) = self.notifications.iter().position(|n| n.id == id) {
            self.notifications.remove(pos);
            self.save();
            true
        } else {
            false
        }
    }

    /// Remove a notification by server-assigned ID. Returns true if found.
    #[allow(dead_code)]
    pub fn remove_by_server_id(&mut self, server_id: &str) -> bool {
        if let Some(pos) = self
            .notifications
            .iter()
            .position(|n| n.payload.id == server_id)
        {
            self.notifications.remove(pos);
            self.save();
            true
        } else {
            false
        }
    }

    /// Clear all notifications.
    pub fn clear(&mut self) {
        self.notifications.clear();
        self.save();
    }

    pub fn list(&self) -> &[StoredNotification] {
        &self.notifications
    }

    pub fn count(&self) -> usize {
        self.notifications.len()
    }

    pub fn get(&self, id: i64) -> Option<&StoredNotification> {
        self.notifications.iter().find(|n| n.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_payload(title: &str, message: &str) -> NotificationPayload {
        NotificationPayload {
            id: String::new(),
            source: String::new(),
            title: title.to_string(),
            message: message.to_string(),
            status: "info".to_string(),
            icon_data: String::new(),
            icon_href: String::new(),
            icon_path: String::new(),
            duration: 0,
            actions: Vec::new(),
            exclusive: false,
            store_on_expire: false,
        }
    }

    fn make_test_store(dir: &std::path::Path) -> NotificationStore {
        NotificationStore::load(dir.join("notifications.json"))
    }

    #[test]
    fn test_add_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = make_test_store(dir.path());

        let id1 = store.add(make_test_payload("Title 1", "Msg 1"), "server".into());
        let id2 = store.add(make_test_payload("Title 2", "Msg 2"), "server".into());

        assert_eq!(store.count(), 2);
        assert_eq!(store.list()[0].id, id1);
        assert_eq!(store.list()[1].id, id2);
        assert_eq!(store.list()[0].payload.title, "Title 1");
        assert_eq!(store.list()[1].payload.title, "Title 2");
    }

    #[test]
    fn test_remove() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = make_test_store(dir.path());

        let id1 = store.add(make_test_payload("A", "a"), "s".into());
        let id2 = store.add(make_test_payload("B", "b"), "s".into());
        let id3 = store.add(make_test_payload("C", "c"), "s".into());

        assert!(store.remove(id2));
        assert_eq!(store.count(), 2);
        assert_eq!(store.list()[0].id, id1);
        assert_eq!(store.list()[1].id, id3);
    }

    #[test]
    fn test_remove_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = make_test_store(dir.path());
        store.add(make_test_payload("A", "a"), "s".into());

        assert!(!store.remove(999));
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn test_clear() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = make_test_store(dir.path());

        store.add(make_test_payload("A", "a"), "s".into());
        store.add(make_test_payload("B", "b"), "s".into());
        store.add(make_test_payload("C", "c"), "s".into());

        store.clear();
        assert_eq!(store.count(), 0);
        assert!(store.list().is_empty());
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");

        {
            let mut store = NotificationStore::load(path.clone());
            store.add(make_test_payload("Title 1", "Msg 1"), "server-a".into());
            store.add(make_test_payload("Title 2", "Msg 2"), "server-b".into());
        }

        // Reload from disk
        let store = NotificationStore::load(path);
        assert_eq!(store.count(), 2);
        assert_eq!(store.list()[0].payload.title, "Title 1");
        assert_eq!(store.list()[0].server_label, "server-a");
        assert_eq!(store.list()[1].payload.title, "Title 2");
        assert_eq!(store.list()[1].server_label, "server-b");
    }

    #[test]
    fn test_load_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = NotificationStore::load(dir.path().join("does-not-exist.json"));
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_next_id_after_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");

        {
            let mut store = NotificationStore::load(path.clone());
            store.add(make_test_payload("A", "a"), "s".into()); // id 1
            store.add(make_test_payload("B", "b"), "s".into()); // id 2
            store.remove(1); // remove first, max id is still 2
        }

        let mut store = NotificationStore::load(path);
        let new_id = store.add(make_test_payload("C", "c"), "s".into());
        assert_eq!(new_id, 3); // next after max(2) = 3
    }

    #[test]
    fn test_get_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = make_test_store(dir.path());

        let id1 = store.add(make_test_payload("First", "msg1"), "s".into());
        let _id2 = store.add(make_test_payload("Second", "msg2"), "s".into());

        let found = store.get(id1).unwrap();
        assert_eq!(found.payload.title, "First");
        assert_eq!(found.payload.message, "msg1");

        assert!(store.get(999).is_none());
    }

    #[test]
    fn test_remove_by_server_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = make_test_store(dir.path());

        let mut p1 = make_test_payload("A", "a");
        p1.id = "server-123".to_string();
        store.add(p1, "s".into());

        let mut p2 = make_test_payload("B", "b");
        p2.id = "server-456".to_string();
        store.add(p2, "s".into());

        assert!(store.remove_by_server_id("server-123"));
        assert_eq!(store.count(), 1);
        assert_eq!(store.list()[0].payload.id, "server-456");

        assert!(!store.remove_by_server_id("nonexistent"));
    }
}
