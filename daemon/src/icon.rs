// Icon loading from various sources: base64 data, local file path, URL.
// Handles decoding, scaling to a standard size, and preparing for GPU upload.

use base64::Engine;
use tracing::warn;

use crate::notification::NotificationPayload;

pub const ICON_SIZE: u32 = 48;

/// Try to load an icon synchronously from the payload (base64 or file path).
/// Returns None if no sync source is available or loading fails.
pub fn resolve_sync(payload: &NotificationPayload) -> Option<image::RgbaImage> {
    if !payload.icon_data.is_empty() {
        match load_from_base64(&payload.icon_data) {
            Ok(img) => return Some(img),
            Err(e) => warn!("Failed to decode icon base64: {}", e),
        }
    }
    if !payload.icon_path.is_empty() {
        match load_from_file(&payload.icon_path) {
            Ok(img) => return Some(img),
            Err(e) => warn!("Failed to load icon from {}: {}", payload.icon_path, e),
        }
    }
    None
}

/// Returns true if the payload has a URL icon source that requires async fetch.
/// Only relevant when base64 and file path are both empty.
pub fn needs_async_fetch(payload: &NotificationPayload) -> bool {
    payload.icon_data.is_empty()
        && payload.icon_path.is_empty()
        && !payload.icon_href.is_empty()
}

/// Fetch an icon from a URL. Call from a tokio task.
pub async fn load_from_url(url: &str) -> anyhow::Result<image::RgbaImage> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let bytes = client.get(url).send().await?.bytes().await?;
    let img = image::load_from_memory(&bytes)?;
    Ok(scale_to_icon(img))
}

/// Public entry for center to load stored icon_data.
pub fn load_from_base64_pub(data: &str) -> anyhow::Result<image::RgbaImage> {
    load_from_base64(data)
}

fn load_from_base64(data: &str) -> anyhow::Result<image::RgbaImage> {
    // Strip data URI prefix if present (e.g. "data:image/png;base64,...")
    let b64 = if let Some(pos) = data.find(";base64,") {
        &data[pos + 8..]
    } else {
        data
    };
    let decoded = base64::engine::general_purpose::STANDARD.decode(b64)?;
    let img = image::load_from_memory(&decoded)?;
    Ok(scale_to_icon(img))
}

fn load_from_file(path: &str) -> anyhow::Result<image::RgbaImage> {
    let img = image::open(path)?;
    Ok(scale_to_icon(img))
}

fn scale_to_icon(img: image::DynamicImage) -> image::RgbaImage {
    // Scale down preserving aspect ratio, then center on a transparent canvas
    let scaled = img.resize(ICON_SIZE, ICON_SIZE, image::imageops::FilterType::Lanczos3);
    let sw = scaled.width();
    let sh = scaled.height();

    if sw == ICON_SIZE && sh == ICON_SIZE {
        return scaled.to_rgba8();
    }

    let mut canvas = image::RgbaImage::new(ICON_SIZE, ICON_SIZE);
    let ox = (ICON_SIZE - sw) / 2;
    let oy = (ICON_SIZE - sh) / 2;
    image::imageops::overlay(&mut canvas, &scaled.to_rgba8(), ox as i64, oy as i64);
    canvas
}
