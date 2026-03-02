// Notification center panel.
// A persistent side panel displaying stored notifications with scroll, dismiss, and actions.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};

use crate::card;
use crate::font::FontAtlas;
use crate::gpu::{GpuContext, WindowSurface};
use crate::renderer::Renderer2D;
use crate::store::{SharedStore, StoredNotification};

// Layout constants (matching Go daemon)
pub const CENTER_W: f32 = 340.0;
const HEADER_H: f32 = 40.0;
const CARD_GAP: f32 = 8.0;
const CARD_PADDING: f32 = 10.0;
const SCROLL_SPEED: f32 = 28.0;
const SCROLL_BAR_W: f32 = 6.0;
const SCROLL_BAR_MIN_H: f32 = 24.0;
const SLIDE_DURATION_MS: f32 = 200.0;

// Colors
const PANEL_BG: [f32; 4] = [0.078, 0.078, 0.098, 0.92];
const HEADER_TEXT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const MUTED_TEXT: [f32; 4] = [0.5, 0.5, 0.5, 1.0];
const CARD_BG: [f32; 4] = [0.15, 0.15, 0.17, 0.92];
const CARD_BG_HOVER: [f32; 4] = [0.20, 0.20, 0.24, 0.92];
const TEXT_COLOR: [f32; 4] = [0.95, 0.95, 0.95, 1.0];
const CLEAR_BTN_BG: [f32; 4] = [0.25, 0.25, 0.28, 1.0];
const CLEAR_BTN_HOVER: [f32; 4] = [0.5, 0.3, 0.3, 1.0];
const CLOSE_BTN_HOVER: [f32; 4] = [0.35, 0.35, 0.38, 1.0];
const SCROLL_BAR_COLOR: [f32; 4] = [0.4, 0.4, 0.4, 0.5];
const EMPTY_TEXT: [f32; 4] = [0.4, 0.4, 0.4, 1.0];
const ACTION_BTN_BG: [f32; 4] = [0.25, 0.25, 0.28, 1.0];
const ACTION_BTN_HOVER: [f32; 4] = [0.35, 0.35, 0.38, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

const ACTION_BTN_H: f32 = 24.0;
const ACTION_BTN_GAP: f32 = 6.0;
const ACTION_BTN_PAD_X: f32 = 10.0;

pub struct CenterState {
    pub window: Arc<winit::window::Window>,
    pub surface: WindowSurface,

    // Scroll
    scroll_offset: f32,
    content_height: f32,

    // Slide animation
    pub slide_t: f32, // 0.0 = off-screen, 1.0 = fully visible
    pub closing: bool,
    slide_start: Option<Instant>,

    // Guard: ignore focus-loss shortly after opening (tray menu steals focus on macOS)
    created_at: Instant,

    // Input
    cursor_pos: (f64, f64),
    hovered_card_id: Option<i64>,
    hovered_clear_all: bool,
    hovered_close: bool,
    hovered_action: Option<(i64, usize)>,

    // Hover-expand per card
    expand_states: HashMap<i64, f32>, // card_id -> expand_t (0.0..1.0)

    // Icon textures for stored notifications
    icon_bind_groups: HashMap<i64, Arc<wgpu::BindGroup>>,
}

impl CenterState {
    pub fn new(window: Arc<winit::window::Window>, surface: WindowSurface) -> Self {
        Self {
            window,
            surface,
            scroll_offset: 0.0,
            content_height: 0.0,
            slide_t: 0.0,
            closing: false,
            slide_start: Some(Instant::now()),
            created_at: Instant::now(),
            cursor_pos: (-1.0, -1.0),
            hovered_card_id: None,
            hovered_clear_all: false,
            hovered_close: false,
            hovered_action: None,
            expand_states: HashMap::new(),
            icon_bind_groups: HashMap::new(),
        }
    }

    pub fn start_closing(&mut self) {
        if !self.closing {
            self.closing = true;
            self.slide_start = Some(Instant::now());
        }
    }

    pub fn is_fully_closed(&self) -> bool {
        self.closing && self.slide_t <= 0.0
    }

    pub fn is_animating(&self) -> bool {
        self.slide_start.is_some()
    }

    /// Whether focus-loss should trigger a close.
    /// Returns false during the first 500ms to avoid tray-menu focus steal on macOS.
    pub fn should_close_on_focus_loss(&self) -> bool {
        self.created_at.elapsed() > std::time::Duration::from_millis(500)
    }

    pub fn on_cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_pos = (x, y);
    }

    pub fn on_cursor_left(&mut self) {
        self.cursor_pos = (-1.0, -1.0);
    }

    pub fn on_scroll(&mut self, delta_y: f32) {
        self.scroll_offset -= delta_y * SCROLL_SPEED;
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let visible_h = self.surface.size.1 as f32 - HEADER_H;
        let max = (self.content_height - visible_h).max(0.0);
        self.scroll_offset = self.scroll_offset.clamp(0.0, max);
    }

    /// Returns the clicked element, if any.
    pub fn on_click(&self, _store: &SharedStore) -> Option<CenterClick> {
        let (cx, cy) = self.cursor_pos;
        let slide_offset = (1.0 - self.slide_t) * CENTER_W;
        let cx = cx as f32 - slide_offset;

        // Close button (top-right)
        if self.hovered_close {
            return Some(CenterClick::Close);
        }

        // Clear All button
        if self.hovered_clear_all {
            return Some(CenterClick::ClearAll);
        }

        // Action button
        if let Some((id, idx)) = self.hovered_action {
            return Some(CenterClick::Action(id, idx));
        }

        // Click on a card body (expand/collapse or no-op)
        if (0.0..=CENTER_W).contains(&cx) && cy as f32 > HEADER_H
            && let Some(id) = self.hovered_card_id
        {
            return Some(CenterClick::CardBody(id));
        }

        None
    }

    /// Load icons for notifications that don't have textures yet.
    pub fn ensure_icons(&mut self, store: &SharedStore, gpu: &GpuContext, renderer: &Renderer2D) {
        let store = store.read().unwrap();
        for notif in store.list() {
            if self.icon_bind_groups.contains_key(&notif.id) {
                continue;
            }
            if !notif.payload.icon_data.is_empty()
                && let Ok(img) = crate::icon::load_from_base64_pub(&notif.payload.icon_data)
            {
                let bind_group = renderer.upload_texture(gpu, &img);
                self.icon_bind_groups.insert(notif.id, bind_group);
            }
        }
    }

    /// Remove icon textures for notifications that no longer exist in the store.
    pub fn prune_icons(&mut self, store: &SharedStore) {
        let store = store.read().unwrap();
        let ids: std::collections::HashSet<i64> = store.list().iter().map(|n| n.id).collect();
        self.icon_bind_groups.retain(|id, _| ids.contains(id));
        self.expand_states.retain(|id, _| ids.contains(id));
    }

    pub fn render(
        &mut self,
        gpu: &GpuContext,
        renderer: &mut Renderer2D,
        font: &mut FontAtlas,
        store: &SharedStore,
        dt: f32,
    ) {
        // Update slide animation
        if let Some(start) = self.slide_start {
            let elapsed = start.elapsed().as_secs_f32() * 1000.0;
            let t = (elapsed / SLIDE_DURATION_MS).clamp(0.0, 1.0);

            if self.closing {
                // Ease-in cubic: t^3
                let eased = t * t * t;
                self.slide_t = 1.0 - eased;
            } else {
                // Ease-out cubic: 1 - (1-t)^3
                let inv = 1.0 - t;
                let eased = 1.0 - inv * inv * inv;
                self.slide_t = eased;
            }

            if t >= 1.0 {
                self.slide_start = None;
            }
        }

        let slide_offset = (1.0 - self.slide_t) * CENTER_W;
        let panel_w = CENTER_W;
        let panel_h = self.surface.size.1 as f32;

        // Read store
        let store_guard = store.read().unwrap();
        let notifications = store_guard.list();

        // Update hover states
        let (cx, cy) = self.cursor_pos;
        let cx_adj = cx as f32 - slide_offset;
        self.update_hover(notifications, font, cx_adj, cy as f32);

        // Update expand animations
        self.update_expand(notifications, font, dt);

        // Begin rendering
        renderer.begin_frame();

        // Panel background
        renderer.draw_rect(slide_offset, 0.0, panel_w, panel_h, PANEL_BG);

        // Header
        self.draw_header(renderer, font, slide_offset, panel_w, notifications.is_empty());

        // Content area
        let content_y = HEADER_H;
        let content_h = panel_h - HEADER_H;

        if notifications.is_empty() {
            // Empty state
            let mut verts = Vec::new();
            let mut indices = Vec::new();
            let text = "No notifications";
            let tw = font.measure_text(text);
            let tx = slide_offset + (panel_w - tw) * 0.5;
            let ty = content_y + content_h * 0.4;
            font.draw_text(tx, ty, text, EMPTY_TEXT, &mut verts, &mut indices);
            if let Some(bg) = font.bind_group() {
                renderer.draw_text_batch(&verts, &indices, bg);
            }
        } else {
            // Calculate content height and draw cards
            self.content_height = self.calculate_content_height(notifications, font);
            self.clamp_scroll();

            let mut y = content_y - self.scroll_offset;
            for notif in notifications {
                let card_h = self.draw_center_card(
                    renderer,
                    font,
                    slide_offset + CARD_PADDING,
                    y,
                    panel_w - CARD_PADDING * 2.0,
                    notif,
                );
                y += card_h + CARD_GAP;
            }

            // Scroll bar
            if self.content_height > content_h {
                self.draw_scroll_bar(renderer, slide_offset, content_y, content_h);
            }
        }

        // Ensure font atlas is up-to-date
        font.ensure_gpu_texture(gpu, renderer);

        // Submit
        let output = match self.surface.surface.get_current_texture() {
            Ok(t) => t,
            Err(_) => return,
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        renderer.render(gpu, &view, font.bind_group());
        output.present();
    }

    fn draw_header(
        &self,
        renderer: &mut Renderer2D,
        font: &mut FontAtlas,
        x_offset: f32,
        panel_w: f32,
        is_empty: bool,
    ) {
        let mut text_verts = Vec::new();
        let mut text_indices = Vec::new();

        // Title
        let title = "Notifications";
        let tx = x_offset + CARD_PADDING;
        let ty = (HEADER_H - font.line_height()) * 0.5;
        font.draw_text(tx, ty, title, HEADER_TEXT, &mut text_verts, &mut text_indices);

        // Close button (X) — top right
        let close_x = x_offset + panel_w - 30.0;
        let close_y = 8.0;
        let close_w = 22.0;
        let close_h = 24.0;
        if self.hovered_close {
            renderer.draw_rect(close_x, close_y, close_w, close_h, CLOSE_BTN_HOVER);
        }
        let x_text_x = close_x + (close_w - font.measure_text("X")) * 0.5;
        let x_text_y = close_y + (close_h - font.line_height()) * 0.5;
        font.draw_text(
            x_text_x,
            x_text_y,
            "X",
            HEADER_TEXT,
            &mut text_verts,
            &mut text_indices,
        );

        // Clear All button — before close button
        if !is_empty {
            let clear_text = "Clear All";
            let clear_tw = font.measure_text(clear_text);
            let clear_w = clear_tw + 16.0;
            let clear_h = 24.0;
            let clear_x = close_x - clear_w - 8.0;
            let clear_y = (HEADER_H - clear_h) * 0.5;
            let bg = if self.hovered_clear_all {
                CLEAR_BTN_HOVER
            } else {
                CLEAR_BTN_BG
            };
            renderer.draw_rect(clear_x, clear_y, clear_w, clear_h, bg);
            let ctx = clear_x + (clear_w - clear_tw) * 0.5;
            let cty = clear_y + (clear_h - font.line_height()) * 0.5;
            font.draw_text(
                ctx,
                cty,
                clear_text,
                HEADER_TEXT,
                &mut text_verts,
                &mut text_indices,
            );
        }

        if let Some(bg) = font.bind_group() {
            renderer.draw_text_batch(&text_verts, &text_indices, bg);
        }
    }

    fn update_hover(
        &mut self,
        notifications: &[StoredNotification],
        font: &mut FontAtlas,
        cx: f32,
        cy: f32,
    ) {
        self.hovered_card_id = None;
        self.hovered_clear_all = false;
        self.hovered_close = false;
        self.hovered_action = None;

        // Close button
        let close_x = CENTER_W - 30.0;
        if (close_x..=close_x + 22.0).contains(&cx) && (8.0..=32.0).contains(&cy) {
            self.hovered_close = true;
            return;
        }

        // Clear All button
        if !notifications.is_empty() {
            let clear_text = "Clear All";
            let clear_tw = font.measure_text(clear_text);
            let clear_w = clear_tw + 16.0;
            let clear_x = close_x - clear_w - 8.0;
            let clear_y = (HEADER_H - 24.0) * 0.5;
            if cx >= clear_x && cx <= clear_x + clear_w && cy >= clear_y && cy <= clear_y + 24.0 {
                self.hovered_clear_all = true;
                return;
            }
        }

        // Cards
        if cy < HEADER_H {
            return;
        }

        let mut y = HEADER_H - self.scroll_offset;
        for notif in notifications {
            let has_icon = self.icon_bind_groups.contains_key(&notif.id);
            let expand_t = self.expand_states.get(&notif.id).copied().unwrap_or(0.0);
            let eased = expand_t * expand_t * (3.0 - 2.0 * expand_t);
            let card_h = self.center_card_height(font, notif, has_icon, eased);
            let card_w = CENTER_W - CARD_PADDING * 2.0;

            if cy >= y && cy < y + card_h && cx >= CARD_PADDING && cx < CARD_PADDING + card_w {
                self.hovered_card_id = Some(notif.id);

                // Action buttons
                if !notif.payload.actions.is_empty() {
                    let actions_y = y + card_h
                        - CARD_PADDING
                        - ACTION_BTN_H;
                    let mut ax = CARD_PADDING + card::STATUS_BAR_W + CARD_PADDING;
                    for (i, action) in notif.payload.actions.iter().enumerate() {
                        let btn_w = font.measure_text(&action.label) + ACTION_BTN_PAD_X * 2.0;
                        if cx >= ax && cx <= ax + btn_w && cy >= actions_y && cy <= actions_y + ACTION_BTN_H
                        {
                            self.hovered_action = Some((notif.id, i));
                            return;
                        }
                        ax += btn_w + ACTION_BTN_GAP;
                    }
                }

                return;
            }
            y += card_h + CARD_GAP;
        }
    }

    fn update_expand(
        &mut self,
        notifications: &[StoredNotification],
        font: &mut FontAtlas,
        dt: f32,
    ) {
        const EXPAND_SPEED: f32 = 6.0;

        for notif in notifications {
            let has_icon = self.icon_bind_groups.contains_key(&notif.id);
            let overflows = card::message_overflows(font, &notif.payload.message, has_icon);
            let is_hovered = self.hovered_card_id == Some(notif.id);
            let target = if is_hovered && overflows { 1.0 } else { 0.0 };

            let t = self.expand_states.entry(notif.id).or_insert(0.0);
            if *t < target {
                *t = (*t + EXPAND_SPEED * dt).min(1.0);
            } else if *t > target {
                *t = (*t - EXPAND_SPEED * dt).max(0.0);
            }
        }
    }

    fn center_card_height(
        &self,
        font: &mut FontAtlas,
        notif: &StoredNotification,
        has_icon: bool,
        expand_t: f32,
    ) -> f32 {
        let base = card::card_height(font, &notif.payload.message, has_icon, expand_t);
        // Add space for action buttons if present
        if notif.payload.actions.is_empty() {
            base
        } else {
            base + ACTION_BTN_H + card::SECTION_GAP
        }
    }

    fn calculate_content_height(
        &self,
        notifications: &[StoredNotification],
        font: &mut FontAtlas,
    ) -> f32 {
        let mut h = 0.0;
        for notif in notifications {
            let has_icon = self.icon_bind_groups.contains_key(&notif.id);
            let expand_t = self.expand_states.get(&notif.id).copied().unwrap_or(0.0);
            let eased = expand_t * expand_t * (3.0 - 2.0 * expand_t);
            h += self.center_card_height(font, notif, has_icon, eased) + CARD_GAP;
        }
        h
    }

    fn draw_center_card(
        &self,
        renderer: &mut Renderer2D,
        font: &mut FontAtlas,
        x: f32,
        y: f32,
        card_w: f32,
        notif: &StoredNotification,
    ) -> f32 {
        let has_icon = self.icon_bind_groups.contains_key(&notif.id);
        let expand_t = self.expand_states.get(&notif.id).copied().unwrap_or(0.0);
        let eased = expand_t * expand_t * (3.0 - 2.0 * expand_t);
        let h = self.center_card_height(font, notif, has_icon, eased);
        let is_hovered = self.hovered_card_id == Some(notif.id);

        let mut text_verts = Vec::new();
        let mut text_indices = Vec::new();

        // Card background
        let bg = if is_hovered { CARD_BG_HOVER } else { CARD_BG };
        renderer.draw_rect(x, y, card_w, h, bg);

        // Status bar
        renderer.draw_rect(
            x,
            y,
            card::STATUS_BAR_W,
            h,
            card::status_color(&notif.payload.status),
        );

        // Icon
        let content_x = x + card::STATUS_BAR_W + CARD_PADDING;
        let icon_space = if has_icon {
            if let Some(bind_group) = self.icon_bind_groups.get(&notif.id) {
                renderer.draw_textured(
                    content_x,
                    y + CARD_PADDING,
                    card::ICON_SIZE,
                    card::ICON_SIZE,
                    WHITE,
                    bind_group,
                );
            }
            card::ICON_SIZE + 8.0
        } else {
            0.0
        };

        let tx = content_x + icon_space;
        let max_text_w = card_w - CARD_PADDING * 2.0 - card::STATUS_BAR_W - icon_space;
        let mut ty = y + CARD_PADDING;
        let line_h = font.line_height();

        // Title
        let title = font.truncate(&notif.payload.title, max_text_w);
        font.draw_text(tx, ty, &title, TEXT_COLOR, &mut text_verts, &mut text_indices);
        ty += line_h + card::SECTION_GAP;

        // Message with expand
        let lines = font.wrap_text(&notif.payload.message, max_text_w);
        let collapsed_lines = lines.len().min(2);
        let extra = ((lines.len() - collapsed_lines) as f32 * eased).floor() as usize;
        let visible_lines = (collapsed_lines + extra).min(lines.len());
        let is_truncated = visible_lines < lines.len();

        for (li, line) in lines.iter().take(visible_lines).enumerate() {
            let display = if is_truncated && li == visible_lines - 1 {
                font.truncate(line, max_text_w)
            } else {
                line.clone()
            };
            font.draw_text(tx, ty, &display, TEXT_COLOR, &mut text_verts, &mut text_indices);
            ty += line_h;
        }
        ty += card::SECTION_GAP;

        // Source + time ago
        let source_line = format_source_line(notif);
        let source_text = font.truncate(&source_line, max_text_w);
        font.draw_text(
            tx,
            ty,
            &source_text,
            MUTED_TEXT,
            &mut text_verts,
            &mut text_indices,
        );

        // Action buttons
        if !notif.payload.actions.is_empty() {
            let actions_y = y + h - CARD_PADDING - ACTION_BTN_H;
            let mut ax = tx;
            for (i, action) in notif.payload.actions.iter().enumerate() {
                let btn_w = font.measure_text(&action.label) + ACTION_BTN_PAD_X * 2.0;
                let btn_bg = if self.hovered_action == Some((notif.id, i)) {
                    ACTION_BTN_HOVER
                } else {
                    ACTION_BTN_BG
                };
                renderer.draw_rect(ax, actions_y, btn_w, ACTION_BTN_H, btn_bg);
                font.draw_text(
                    ax + ACTION_BTN_PAD_X,
                    actions_y + (ACTION_BTN_H - line_h) * 0.5,
                    &action.label,
                    TEXT_COLOR,
                    &mut text_verts,
                    &mut text_indices,
                );
                ax += btn_w + ACTION_BTN_GAP;
            }
        }

        // Submit text
        if let Some(bg) = font.bind_group() {
            renderer.draw_text_batch(&text_verts, &text_indices, bg);
        }

        h
    }

    fn draw_scroll_bar(
        &self,
        renderer: &mut Renderer2D,
        x_offset: f32,
        content_y: f32,
        content_h: f32,
    ) {
        let total = self.content_height;
        if total <= 0.0 {
            return;
        }

        let thumb_ratio = (content_h / total).min(1.0);
        let thumb_h = (content_h * thumb_ratio).max(SCROLL_BAR_MIN_H);
        let scrollable = total - content_h;
        let thumb_y = if scrollable > 0.0 {
            content_y + (self.scroll_offset / scrollable) * (content_h - thumb_h)
        } else {
            content_y
        };

        let bar_x = x_offset + CENTER_W - SCROLL_BAR_W - 2.0;
        renderer.draw_rect(bar_x, thumb_y, SCROLL_BAR_W, thumb_h, SCROLL_BAR_COLOR);
    }
}

#[derive(Debug)]
pub enum CenterClick {
    Close,
    ClearAll,
    Action(i64, usize),
    CardBody(i64),
}

fn format_source_line(notif: &StoredNotification) -> String {
    let time_ago = format_time_ago(notif.created_at);
    if notif.payload.source.is_empty() {
        format!("{} - {}", notif.server_label, time_ago)
    } else {
        format!("{} - {}", notif.payload.source, time_ago)
    }
}

pub fn format_time_ago(created_at: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(created_at);

    let secs = duration.num_seconds();
    if secs < 60 {
        return "just now".to_string();
    }

    let mins = duration.num_minutes();
    if mins == 1 {
        return "1 min ago".to_string();
    }
    if mins < 60 {
        return format!("{} min ago", mins);
    }

    let hours = duration.num_hours();
    if hours == 1 {
        return "1 hour ago".to_string();
    }
    if hours < 24 {
        return format!("{} hours ago", hours);
    }

    let days = duration.num_days();
    if days == 1 {
        return "yesterday".to_string();
    }
    format!("{} days ago", days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn test_format_time_ago_just_now() {
        let now = Utc::now();
        assert_eq!(format_time_ago(now), "just now");
        assert_eq!(format_time_ago(now - Duration::seconds(30)), "just now");
        assert_eq!(format_time_ago(now - Duration::seconds(59)), "just now");
    }

    #[test]
    fn test_format_time_ago_minutes() {
        let now = Utc::now();
        assert_eq!(format_time_ago(now - Duration::minutes(1)), "1 min ago");
        assert_eq!(format_time_ago(now - Duration::minutes(5)), "5 min ago");
        assert_eq!(format_time_ago(now - Duration::minutes(59)), "59 min ago");
    }

    #[test]
    fn test_format_time_ago_hours() {
        let now = Utc::now();
        assert_eq!(format_time_ago(now - Duration::hours(1)), "1 hour ago");
        assert_eq!(format_time_ago(now - Duration::hours(23)), "23 hours ago");
    }

    #[test]
    fn test_format_time_ago_days() {
        let now = Utc::now();
        assert_eq!(format_time_ago(now - Duration::days(1)), "yesterday");
        assert_eq!(format_time_ago(now - Duration::days(2)), "2 days ago");
        assert_eq!(format_time_ago(now - Duration::days(30)), "30 days ago");
    }
}
