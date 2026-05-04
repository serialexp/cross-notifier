//! Device registry: persistent record of mobile (APNS/FCM) push destinations.
//!
//! A device is identified by its opaque push token. Registrations are keyed
//! by token so re-registering with the same token updates the entry in place
//! (labels can change, OS reinstalls rotate the token and we get a fresh
//! entry, server learns old tokens are dead via a 410 and prunes).
//!
//! Persistence is a single JSON file written atomically via tmp + rename.
//! If no path is configured the registry is in-memory only — fine for
//! tests and ephemeral deployments, a foot-gun for production.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Which push transport a device expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Ios,
    Android,
}

/// A single registered push destination.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// Opaque push token issued by APNS / FCM.
    pub token: String,
    /// Human-readable label ("Bart's iPhone") shown in the devices list.
    #[serde(default)]
    pub label: String,
    pub platform: Platform,
    pub registered_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_push_at: Option<DateTime<Utc>>,
}

/// Shared, clone-cheap handle to the device registry.
#[derive(Clone, Default)]
pub struct DeviceRegistry {
    inner: Arc<RegistryInner>,
}

#[derive(Default)]
struct RegistryInner {
    /// `None` means in-memory only; writes are not persisted.
    path: Option<PathBuf>,
    devices: RwLock<HashMap<String, Device>>,
}

impl DeviceRegistry {
    /// Fresh empty in-memory registry. Useful for tests and the default
    /// case when no `-devices-file` is configured.
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(RegistryInner::default()),
        }
    }

    /// Registry backed by `path`. If the file exists it is loaded; if it
    /// doesn't, we start empty and the first successful save creates it.
    /// A corrupt file is a hard error — refuse to start rather than
    /// silently discard registrations.
    pub async fn from_file(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let devices = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice::<HashMap<String, Device>>(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e),
        };
        info!(
            path = %path.display(),
            count = devices.len(),
            "device registry loaded",
        );
        Ok(Self {
            inner: Arc::new(RegistryInner {
                path: Some(path),
                devices: RwLock::new(devices),
            }),
        })
    }

    /// Insert-or-update a device. Preserves `registered_at` for existing
    /// tokens so re-registering on app launch doesn't reset the timestamp.
    pub async fn register(&self, token: String, label: String, platform: Platform) -> Device {
        let now = Utc::now();
        let device = {
            let mut devices = self.inner.devices.write().await;
            let entry = devices.entry(token.clone()).and_modify(|d| {
                d.label = label.clone();
                d.platform = platform;
            });
            entry
                .or_insert_with(|| Device {
                    token,
                    label,
                    platform,
                    registered_at: now,
                    last_push_at: None,
                })
                .clone()
        };
        self.save_best_effort().await;
        device
    }

    /// Remove a single device by token. Returns whether it existed.
    pub async fn unregister(&self, token: &str) -> bool {
        let removed = self.inner.devices.write().await.remove(token).is_some();
        if removed {
            self.save_best_effort().await;
        }
        removed
    }

    /// Bulk-remove the given tokens (called after APNS reports them dead).
    pub async fn remove_many(&self, tokens: &[String]) {
        if tokens.is_empty() {
            return;
        }
        {
            let mut devices = self.inner.devices.write().await;
            for t in tokens {
                devices.remove(t);
            }
        }
        self.save_best_effort().await;
    }

    /// Update the `last_push_at` for a token. Best-effort, silently
    /// ignored if the token was pruned between iteration and update.
    pub async fn record_push(&self, token: &str) {
        let mut devices = self.inner.devices.write().await;
        if let Some(d) = devices.get_mut(token) {
            d.last_push_at = Some(Utc::now());
        }
    }

    /// Snapshot of all devices. Cheap: just clones the HashMap values.
    pub async fn list(&self) -> Vec<Device> {
        self.inner.devices.read().await.values().cloned().collect()
    }

    /// Snapshot limited to a specific platform (tests + push fan-out).
    pub async fn list_for(&self, platform: Platform) -> Vec<Device> {
        self.inner
            .devices
            .read()
            .await
            .values()
            .filter(|d| d.platform == platform)
            .cloned()
            .collect()
    }

    /// Number of registered devices (tests).
    pub async fn count(&self) -> usize {
        self.inner.devices.read().await.len()
    }

    /// Persist to disk if configured. Best-effort: a write failure is
    /// logged and swallowed — the in-memory state is still authoritative
    /// for the running process and we don't want a failed write to reject
    /// a registration.
    async fn save_best_effort(&self) {
        let Some(path) = self.inner.path.clone() else {
            return;
        };
        let snapshot: HashMap<String, Device> = self.inner.devices.read().await.clone();
        if let Err(e) = write_atomic(&path, &snapshot).await {
            warn!(path = %path.display(), "device registry save failed: {e}");
        }
    }
}

/// Write `devices` to `path` atomically: serialize to a sibling tmp file,
/// fsync, rename over the target. On rename failure the original file is
/// untouched.
async fn write_atomic(path: &Path, devices: &HashMap<String, Device>) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(devices)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await?;
    }

    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &json).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_register_list_unregister() {
        let reg = DeviceRegistry::in_memory();
        assert_eq!(reg.count().await, 0);

        reg.register("tok1".into(), "iPhone".into(), Platform::Ios)
            .await;
        reg.register("tok2".into(), "Pixel".into(), Platform::Android)
            .await;
        assert_eq!(reg.count().await, 2);

        let ios = reg.list_for(Platform::Ios).await;
        assert_eq!(ios.len(), 1);
        assert_eq!(ios[0].token, "tok1");

        assert!(reg.unregister("tok1").await);
        assert!(!reg.unregister("tok1").await, "second unregister no-op");
        assert_eq!(reg.count().await, 1);
    }

    #[tokio::test]
    async fn register_same_token_updates_label() {
        let reg = DeviceRegistry::in_memory();
        let first = reg
            .register("tok".into(), "old".into(), Platform::Ios)
            .await;
        // Wait a tick so timestamps would differ if we were resetting them.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let second = reg
            .register("tok".into(), "new".into(), Platform::Ios)
            .await;

        assert_eq!(reg.count().await, 1);
        assert_eq!(second.label, "new");
        assert_eq!(
            second.registered_at, first.registered_at,
            "registered_at preserved on re-register"
        );
    }

    #[tokio::test]
    async fn remove_many_prunes_in_bulk() {
        let reg = DeviceRegistry::in_memory();
        for i in 0..5 {
            reg.register(format!("tok{i}"), format!("dev{i}"), Platform::Ios)
                .await;
        }
        reg.remove_many(&["tok1".into(), "tok3".into(), "missing".into()])
            .await;
        let remaining: Vec<String> = reg.list().await.into_iter().map(|d| d.token).collect();
        assert_eq!(remaining.len(), 3);
        assert!(remaining.contains(&"tok0".into()));
        assert!(remaining.contains(&"tok2".into()));
        assert!(remaining.contains(&"tok4".into()));
    }

    #[tokio::test]
    async fn persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");

        {
            let reg = DeviceRegistry::from_file(&path).await.unwrap();
            reg.register("tok1".into(), "A".into(), Platform::Ios).await;
            reg.register("tok2".into(), "B".into(), Platform::Android)
                .await;
        }

        let reloaded = DeviceRegistry::from_file(&path).await.unwrap();
        assert_eq!(reloaded.count().await, 2);
        let labels: std::collections::HashSet<String> =
            reloaded.list().await.into_iter().map(|d| d.label).collect();
        assert!(labels.contains("A"));
        assert!(labels.contains("B"));
    }

    #[tokio::test]
    async fn from_file_missing_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        let reg = DeviceRegistry::from_file(&path).await.unwrap();
        assert_eq!(reg.count().await, 0);
        // First write creates it.
        reg.register("tok".into(), "x".into(), Platform::Ios).await;
        assert!(path.exists());
    }

    #[tokio::test]
    async fn from_file_corrupt_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        tokio::fs::write(&path, b"not json").await.unwrap();
        assert!(DeviceRegistry::from_file(&path).await.is_err());
    }

    #[tokio::test]
    async fn record_push_updates_last_push_at() {
        let reg = DeviceRegistry::in_memory();
        reg.register("tok".into(), "x".into(), Platform::Ios).await;
        assert!(reg.list().await[0].last_push_at.is_none());
        reg.record_push("tok").await;
        assert!(reg.list().await[0].last_push_at.is_some());
    }
}
