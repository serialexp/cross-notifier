//! JSON-file persistence for scheduler state. One file, whole-map
//! replacement on every write. Fine for personal-scale calendars where
//! "lots" means a few hundred pending reminders.
//!
//! The file is written atomically (tmp + rename) so a mid-write crash
//! can't corrupt existing state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;

use crate::types::PendingFire;

pub type PendingMap = HashMap<String, PendingFire>;

#[async_trait]
pub trait PendingStore: Send + Sync {
    async fn load(&self) -> anyhow::Result<PendingMap>;
    async fn save(&self, map: &PendingMap) -> anyhow::Result<()>;
}

/// In-memory store for tests and ephemeral use.
#[derive(Default)]
pub struct MemoryStore {
    inner: tokio::sync::Mutex<PendingMap>,
}

#[async_trait]
impl PendingStore for MemoryStore {
    async fn load(&self) -> anyhow::Result<PendingMap> {
        Ok(self.inner.lock().await.clone())
    }
    async fn save(&self, map: &PendingMap) -> anyhow::Result<()> {
        *self.inner.lock().await = map.clone();
        Ok(())
    }
}

pub struct JsonStore {
    path: PathBuf,
}

impl JsonStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait]
impl PendingStore for JsonStore {
    async fn load(&self) -> anyhow::Result<PendingMap> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PendingMap::new()),
            Err(e) => Err(e.into()),
        }
    }

    async fn save(&self, map: &PendingMap) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(map)?;
        atomic_write(&self.path, text.as_bytes()).await
    }
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = tokio::fs::File::create(&tmp).await?;
        f.write_all(bytes).await?;
        f.sync_all().await?;
    }
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Occurrence;
    use chrono::{Duration, Utc};

    fn sample() -> PendingFire {
        PendingFire::new(Occurrence {
            id: "abc".into(),
            event_uid: "e".into(),
            recurrence_id: None,
            fire_at: Utc::now(),
            event_start: Utc::now() + Duration::minutes(5),
            event_end: Utc::now() + Duration::minutes(35),
            summary: "S".into(),
            location: None,
            description: None,
        })
    }

    #[tokio::test]
    async fn json_store_round_trips() {
        let dir = tempdir_str();
        let store = JsonStore::new(format!("{dir}/pending.json"));
        let mut map = PendingMap::new();
        map.insert("abc".into(), sample());
        store.save(&map).await.unwrap();
        let back = store.load().await.unwrap();
        assert_eq!(back.len(), 1);
        assert!(back.contains_key("abc"));
    }

    #[tokio::test]
    async fn missing_file_yields_empty_map() {
        let store = JsonStore::new(format!("{}/does-not-exist.json", tempdir_str()));
        let map = store.load().await.unwrap();
        assert!(map.is_empty());
    }

    fn tempdir_str() -> String {
        let p = std::env::temp_dir().join(format!("calstore-{}", uuid_like()));
        std::fs::create_dir_all(&p).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn uuid_like() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        format!(
            "{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }
}
