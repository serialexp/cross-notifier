// Glyph atlas for GPU text rendering.
// Rasterizes font glyphs into a texture atlas, provides text measurement and drawing.

use std::collections::HashMap;

use fontdue::{Font, FontSettings};

use crate::gpu::GpuContext;
use crate::renderer::{Renderer2D, Vertex};

const ATLAS_SIZE: u32 = 1024;

static FONT_DATA: &[u8] = include_bytes!("../fonts/Hack-Regular.ttf");

#[derive(Debug, Clone, Copy)]
pub struct GlyphInfo {
    pub uv_x: f32,
    pub uv_y: f32,
    pub uv_w: f32,
    pub uv_h: f32,
    pub width: u32,
    pub height: u32,
    pub advance: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

pub struct FontAtlas {
    font: Font,
    font_size: f32,
    ascent: f32,
    line_height: f32,
    glyphs: HashMap<char, GlyphInfo>,
    atlas_data: Vec<u8>, // RGBA
    atlas_width: u32,
    atlas_height: u32,
    next_x: u32,
    next_y: u32,
    row_height: u32,
    dirty: bool, // atlas data changed, needs GPU re-upload

    // GPU resources
    texture: Option<wgpu::Texture>,
    bind_group: Option<wgpu::BindGroup>,
}

impl FontAtlas {
    pub fn new(font_size: f32) -> Self {
        let font = Font::from_bytes(FONT_DATA, FontSettings::default())
            .expect("Failed to load embedded font");

        let line_metrics =
            font.horizontal_line_metrics(font_size)
                .unwrap_or(fontdue::LineMetrics {
                    ascent: font_size * 0.8,
                    descent: font_size * -0.2,
                    line_gap: 0.0,
                    new_line_size: font_size,
                });

        let ascent = line_metrics.ascent;
        let line_height = line_metrics.ascent - line_metrics.descent + line_metrics.line_gap;

        let atlas_data = vec![0u8; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize];

        let mut atlas = Self {
            font,
            font_size,
            ascent,
            line_height,
            glyphs: HashMap::new(),
            atlas_data,
            atlas_width: ATLAS_SIZE,
            atlas_height: ATLAS_SIZE,
            next_x: 0,
            next_y: 0,
            row_height: 0,
            dirty: false,
            texture: None,
            bind_group: None,
        };

        // Pre-rasterize common ASCII range
        for c in ' '..='~' {
            atlas.rasterize_glyph(c);
        }
        atlas.dirty = true;

        atlas
    }

    pub fn line_height(&self) -> f32 {
        self.line_height
    }

    fn rasterize_glyph(&mut self, ch: char) -> Option<&GlyphInfo> {
        if self.glyphs.contains_key(&ch) {
            return self.glyphs.get(&ch);
        }

        let (metrics, bitmap) = self.font.rasterize(ch, self.font_size);

        if metrics.width == 0 || metrics.height == 0 {
            // Whitespace or empty glyph - store with zero dimensions
            let info = GlyphInfo {
                uv_x: 0.0,
                uv_y: 0.0,
                uv_w: 0.0,
                uv_h: 0.0,
                width: 0,
                height: 0,
                advance: metrics.advance_width,
                offset_x: metrics.xmin as f32,
                offset_y: metrics.ymin as f32,
            };
            self.glyphs.insert(ch, info);
            return self.glyphs.get(&ch);
        }

        let w = metrics.width as u32;
        let h = metrics.height as u32;

        // Check if we need to move to next row
        if self.next_x + w + 1 > self.atlas_width {
            self.next_x = 0;
            self.next_y += self.row_height + 1;
            self.row_height = 0;
        }

        // Check if atlas is full
        if self.next_y + h > self.atlas_height {
            tracing::warn!("Font atlas is full, cannot add glyph '{}'", ch);
            return None;
        }

        // Copy glyph bitmap to atlas (convert grayscale to RGBA)
        let ax = self.next_x;
        let ay = self.next_y;
        for gy in 0..h {
            for gx in 0..w {
                let alpha = bitmap[(gy * w + gx) as usize];
                let idx = ((ay + gy) * self.atlas_width + (ax + gx)) as usize * 4;
                self.atlas_data[idx] = 255; // R
                self.atlas_data[idx + 1] = 255; // G
                self.atlas_data[idx + 2] = 255; // B
                self.atlas_data[idx + 3] = alpha; // A
            }
        }

        let info = GlyphInfo {
            uv_x: ax as f32 / self.atlas_width as f32,
            uv_y: ay as f32 / self.atlas_height as f32,
            uv_w: w as f32 / self.atlas_width as f32,
            uv_h: h as f32 / self.atlas_height as f32,
            width: w,
            height: h,
            advance: metrics.advance_width,
            offset_x: metrics.xmin as f32,
            offset_y: metrics.ymin as f32,
        };

        self.glyphs.insert(ch, info);
        self.next_x += w + 1;
        self.row_height = self.row_height.max(h);
        self.dirty = true;

        self.glyphs.get(&ch)
    }

    /// Measure text width in pixels.
    pub fn measure_text(&mut self, text: &str) -> f32 {
        let mut width = 0.0;
        for ch in text.chars() {
            if let Some(glyph) = self.rasterize_glyph(ch) {
                width += glyph.advance;
            }
        }
        width
    }

    /// Truncate text to fit within max_width, appending "..." if truncated.
    pub fn truncate(&mut self, text: &str, max_width: f32) -> String {
        let ellipsis_width = self.measure_text("...");
        let mut width = 0.0;
        let mut result = String::new();

        for ch in text.chars() {
            let advance = self.rasterize_glyph(ch).map(|g| g.advance).unwrap_or(0.0);

            if width + advance > max_width - ellipsis_width {
                result.push_str("...");
                return result;
            }
            width += advance;
            result.push(ch);
        }
        result
    }

    /// Wrap text to fit within max_width, returning lines.
    pub fn wrap_text(&mut self, text: &str, max_width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut current_line = String::new();
        let mut current_width = 0.0;

        for word in text.split_whitespace() {
            let word_width = self.measure_text(word);
            let space_width = if current_line.is_empty() {
                0.0
            } else {
                self.measure_text(" ")
            };

            if current_width + space_width + word_width > max_width && !current_line.is_empty() {
                lines.push(std::mem::take(&mut current_line));
                current_width = 0.0;
            }

            if !current_line.is_empty() {
                current_line.push(' ');
                current_width += space_width;
            }
            current_line.push_str(word);
            current_width += word_width;
        }

        if !current_line.is_empty() {
            lines.push(current_line);
        }

        lines
    }

    /// Ensure the GPU texture is up-to-date. Call once per frame before drawing text.
    pub fn ensure_gpu_texture(&mut self, gpu: &GpuContext, renderer: &Renderer2D) {
        if self.texture.is_none() {
            self.create_gpu_texture(gpu, renderer);
            self.dirty = false;
        } else if self.dirty {
            self.update_gpu_texture(gpu);
            self.dirty = false;
        }
    }

    fn create_gpu_texture(&mut self, gpu: &GpuContext, renderer: &Renderer2D) {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Font Atlas"),
            size: wgpu::Extent3d {
                width: self.atlas_width,
                height: self.atlas_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.atlas_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * self.atlas_width),
                rows_per_image: Some(self.atlas_height),
            },
            wgpu::Extent3d {
                width: self.atlas_width,
                height: self.atlas_height,
                depth_or_array_layers: 1,
            },
        );

        let bind_group = renderer.create_texture_bind_group(gpu, &texture);
        self.bind_group = Some(bind_group);
        self.texture = Some(texture);
    }

    fn update_gpu_texture(&self, gpu: &GpuContext) {
        if let Some(texture) = &self.texture {
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.atlas_data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * self.atlas_width),
                    rows_per_image: Some(self.atlas_height),
                },
                wgpu::Extent3d {
                    width: self.atlas_width,
                    height: self.atlas_height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Generate vertices for drawing text. Returns (vertices, indices) for textured quads.
    /// The color is applied to each vertex for tinting.
    pub fn draw_text(
        &mut self,
        x: f32,
        y: f32,
        text: &str,
        color: [f32; 4],
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
    ) {
        let mut cursor_x = x;
        let baseline_y = y + self.ascent;

        for ch in text.chars() {
            let glyph = match self.rasterize_glyph(ch) {
                Some(g) => *g,
                None => continue,
            };

            if glyph.width == 0 || glyph.height == 0 {
                cursor_x += glyph.advance;
                continue;
            }

            // Position glyph relative to baseline
            let gx = cursor_x + glyph.offset_x;
            let gy = baseline_y - glyph.offset_y - glyph.height as f32;

            let gw = glyph.width as f32;
            let gh = glyph.height as f32;

            let base_idx = vertices.len() as u32;

            vertices.extend_from_slice(&[
                Vertex {
                    position: [gx, gy],
                    tex_coords: [glyph.uv_x, glyph.uv_y],
                    color,
                },
                Vertex {
                    position: [gx + gw, gy],
                    tex_coords: [glyph.uv_x + glyph.uv_w, glyph.uv_y],
                    color,
                },
                Vertex {
                    position: [gx + gw, gy + gh],
                    tex_coords: [glyph.uv_x + glyph.uv_w, glyph.uv_y + glyph.uv_h],
                    color,
                },
                Vertex {
                    position: [gx, gy + gh],
                    tex_coords: [glyph.uv_x, glyph.uv_y + glyph.uv_h],
                    color,
                },
            ]);

            indices.extend_from_slice(&[
                base_idx,
                base_idx + 1,
                base_idx + 2,
                base_idx,
                base_idx + 2,
                base_idx + 3,
            ]);

            cursor_x += glyph.advance;
        }
    }

    /// Get the bind group for the atlas texture (for use in draw calls).
    pub fn bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.bind_group.as_ref()
    }
}
