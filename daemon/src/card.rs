// Notification card rendering.
// Draws a single notification card with status bar, icon, title, message, and source text.

use crate::font::FontAtlas;
use crate::icon;
use crate::renderer::{Renderer2D, Vertex};

pub const CARD_W: f32 = 300.0;
pub const PADDING: f32 = 10.0;
pub const ICON_SIZE: f32 = icon::ICON_SIZE as f32;
const ICON_GAP: f32 = 8.0;
pub const STATUS_BAR_W: f32 = 3.0;
pub const SECTION_GAP: f32 = 4.0;
pub const MAX_VISIBLE: usize = 4;
pub const STACK_PEEK: f32 = 20.0;
const STACK_SHRINK: f32 = 0.02; // each stacked card shrinks by 2%

const TEXT_COLOR: [f32; 4] = [0.95, 0.95, 0.95, 1.0];
const MUTED_COLOR: [f32; 4] = [0.5, 0.5, 0.5, 1.0];
const CARD_BG: [f32; 4] = [0.15, 0.15, 0.18, 0.95];
const CARD_BG_HOVER: [f32; 4] = [0.22, 0.22, 0.26, 1.0];
const CARD_BG_STACKED: [f32; 4] = [0.15, 0.15, 0.18, 0.85];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

pub fn status_color(status: &str) -> [f32; 4] {
    match status {
        "success" => [0.2, 0.8, 0.3, 1.0],
        "warning" => [1.0, 0.8, 0.0, 1.0],
        "error" => [0.9, 0.2, 0.2, 1.0],
        _ => [0.3, 0.5, 0.9, 1.0], // info (default)
    }
}

/// Width available for text given whether an icon is present.
fn text_max_width(has_icon: bool) -> f32 {
    let icon_space = if has_icon { ICON_SIZE + ICON_GAP } else { 0.0 };
    CARD_W - PADDING * 2.0 - STATUS_BAR_W - icon_space
}

/// Returns true if the message wraps to more than 2 lines (i.e. needs an expand button).
pub fn message_overflows(font: &mut FontAtlas, message: &str, has_icon: bool) -> bool {
    let max_text_w = text_max_width(has_icon);
    font.wrap_text(message, max_text_w).len() > 2
}

/// Calculate card height based on content.
/// `expand_t` is 0.0 (collapsed, max 2 lines) to 1.0 (expanded, all lines).
/// Values in between produce smooth height interpolation.
pub fn card_height(font: &mut FontAtlas, message: &str, has_icon: bool, expand_t: f32) -> f32 {
    let max_text_w = text_max_width(has_icon);
    let line_h = font.line_height();
    let title_h = line_h; // 1 line
    let lines = font.wrap_text(message, max_text_w);

    let collapsed_lines = lines.len().min(2) as f32;
    let expanded_lines = lines.len() as f32;
    let msg_lines = collapsed_lines + (expanded_lines - collapsed_lines) * expand_t;
    let msg_h = msg_lines * line_h;
    let source_h = line_h;

    let text_h = PADDING + title_h + SECTION_GAP + msg_h + SECTION_GAP + source_h + PADDING;

    // Card must be at least tall enough for icon + padding
    if has_icon {
        text_h.max(PADDING + ICON_SIZE + PADDING)
    } else {
        text_h
    }
}

/// Scale factor for a card at the given stack index. Index 0 = topmost = 1.0.
pub fn stack_scale(index: usize) -> f32 {
    1.0 - index as f32 * STACK_SHRINK
}

/// Compute the total window height needed for all visible notifications.
/// Cards are stacked at y = index * STACK_PEEK, scaled down progressively.
/// `topmost_expand_t` is the expand animation progress for the topmost card.
pub fn total_height(
    font: &mut FontAtlas,
    notifications: &[&crate::notification::Notification],
    topmost_expand_t: f32,
) -> f32 {
    if notifications.is_empty() {
        return 0.0;
    }

    let count = notifications.len().min(MAX_VISIBLE);
    let mut max_bottom: f32 = 0.0;
    for (i, notif) in notifications.iter().enumerate().take(count) {
        let scale = stack_scale(i);
        let has_icon = notif.icon_bind_group.is_some();
        let et = if i == 0 { topmost_expand_t } else { 0.0 };
        let h = card_height(font, notif.message(), has_icon, et) * scale;
        let bottom = i as f32 * STACK_PEEK + h;
        max_bottom = max_bottom.max(bottom);
    }

    max_bottom
}

/// Draw a notification card and its text. Returns the scaled card height.
/// `stack_index` is 0 for topmost, 1+ for stacked cards behind it.
/// `anim_t` is the dismiss animation progress (0.0 = just dismissed, 1.0 = settled).
/// `expand_t` is the hover-expand animation progress (0.0 = collapsed, 1.0 = fully expanded).
/// During animation, cards interpolate scale from their old index to their new index.
#[allow(clippy::too_many_arguments)]
pub fn draw_card(
    renderer: &mut Renderer2D,
    font: &mut FontAtlas,
    x: f32,
    y: f32,
    card_w: f32,
    notification: &crate::notification::Notification,
    stack_index: usize,
    anim_t: f32,
    hovered: bool,
    expand_t: f32,
    text_verts: &mut Vec<Vertex>,
    text_indices: &mut Vec<u32>,
) -> f32 {
    let is_topmost = stack_index == 0;
    // Interpolate scale: from old position (index+1) to new (index)
    let old_scale = stack_scale(stack_index + 1);
    let new_scale = stack_scale(stack_index);
    let scale = old_scale + (new_scale - old_scale) * anim_t;
    let has_icon = notification.icon_bind_group.is_some();
    let max_text_w = text_max_width(has_icon);
    let h = card_height(font, notification.message(), has_icon, expand_t) * scale;
    let w = card_w * scale;
    let line_h = font.line_height();

    // Center scaled card horizontally within the full card width
    let x_offset = x + (card_w - w) * 0.5;

    // Card background — brighten on hover
    let bg = if hovered {
        CARD_BG_HOVER
    } else if is_topmost {
        CARD_BG
    } else {
        CARD_BG_STACKED
    };
    renderer.draw_rect(x_offset, y, w, h, bg);

    // Status color bar
    renderer.draw_rect(x_offset, y, STATUS_BAR_W, h, status_color(notification.status()));

    // Icon (if present)
    let content_x = x_offset + STATUS_BAR_W + PADDING;
    if let Some(bind_group) = &notification.icon_bind_group {
        renderer.draw_textured(
            content_x,
            y + PADDING,
            ICON_SIZE,
            ICON_SIZE,
            WHITE,
            bind_group,
        );
    }

    let tx = content_x + if has_icon { ICON_SIZE + ICON_GAP } else { 0.0 };
    let mut ty = y + PADDING;

    // Title (1 line, truncated)
    let title = font.truncate(notification.title(), max_text_w);
    font.draw_text(tx, ty, &title, TEXT_COLOR, text_verts, text_indices);
    ty += line_h + SECTION_GAP;

    // Message lines — reveal progressively as expand_t grows
    let lines = font.wrap_text(notification.message(), max_text_w);
    let collapsed_lines = lines.len().min(2);
    let extra = ((lines.len() - collapsed_lines) as f32 * expand_t).floor() as usize;
    let visible_lines = (collapsed_lines + extra).min(lines.len());
    let is_truncated = visible_lines < lines.len();

    for (li, line) in lines.iter().take(visible_lines).enumerate() {
        let display = if is_truncated && li == visible_lines - 1 {
            // Last visible line when more exist: add ellipsis
            font.truncate(line, max_text_w)
        } else {
            line.clone()
        };
        font.draw_text(tx, ty, &display, TEXT_COLOR, text_verts, text_indices);
        ty += line_h;
    }
    ty += SECTION_GAP;

    // Source (muted)
    let source = if notification.source().is_empty() {
        notification.server_label.clone()
    } else {
        format!("{} · {}", notification.source(), notification.server_label)
    };
    let source_text = font.truncate(&source, max_text_w);
    font.draw_text(tx, ty, &source_text, MUTED_COLOR, text_verts, text_indices);

    h
}
