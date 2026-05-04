//! Fetches remote icon URLs and bakes them into base64-encoded 48x48 PNGs
//! so clients don't need to make their own outbound requests.

use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use image::{GenericImageView, ImageFormat, imageops::FilterType};

pub const ICON_SIZE: u32 = 48;

#[derive(Debug, thiserror::Error)]
pub enum IconError {
    #[error("fetch: {0}")]
    Fetch(#[from] reqwest::Error),
    #[error("non-ok status: {0}")]
    Status(u16),
    #[error("decode: {0}")]
    Decode(#[from] image::ImageError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Downloads an image from `url`, scales it to fit ICON_SIZE×ICON_SIZE
/// preserving aspect ratio (no upscaling; centers smaller images), and
/// returns a base64 PNG suitable for stuffing into `Notification::icon_data`.
pub async fn fetch_and_encode(url: &str) -> Result<String, IconError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(IconError::Status(resp.status().as_u16()));
    }
    let bytes = resp.bytes().await?;

    let img = image::load_from_memory(&bytes)?;
    let (src_w, src_h) = img.dimensions();

    let scale = (ICON_SIZE as f32 / src_w as f32).min(ICON_SIZE as f32 / src_h as f32);
    let (scaled_w, scaled_h) = if scale >= 1.0 {
        (src_w, src_h)
    } else {
        ((src_w as f32 * scale) as u32, (src_h as f32 * scale) as u32)
    };

    let scaled = if (scaled_w, scaled_h) == (src_w, src_h) {
        img.to_rgba8()
    } else {
        image::imageops::resize(&img, scaled_w, scaled_h, FilterType::CatmullRom)
    };

    // Paste onto an ICON_SIZE transparent canvas, centered.
    let mut canvas = image::RgbaImage::from_pixel(ICON_SIZE, ICON_SIZE, image::Rgba([0, 0, 0, 0]));
    let offset_x = (ICON_SIZE.saturating_sub(scaled_w)) / 2;
    let offset_y = (ICON_SIZE.saturating_sub(scaled_h)) / 2;
    image::imageops::overlay(&mut canvas, &scaled, offset_x as i64, offset_y as i64);

    let mut out = std::io::Cursor::new(Vec::new());
    canvas.write_to(&mut out, ImageFormat::Png)?;
    Ok(STANDARD.encode(out.into_inner()))
}
