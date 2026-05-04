// Cross-Notifier Daemon
// Displays desktop notifications received from remote servers or local HTTP endpoint.
// Uses winit + wgpu for rendering, tokio for async networking.

mod app;
mod autostart;
mod card;
mod center;
mod client;
mod config;
mod font;
mod gpu;
mod icon;
mod notification;
mod protocol;
mod renderer;
mod rules;
mod server;
mod settings;
mod sound;
mod store;
mod theme;
mod tray;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use notify::{RecommendedWatcher, Watcher};
use tracing::info;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowAttributes, WindowId, WindowLevel};

use app::AppEvent;
use card::CARD_W;
use config::Config;
use cross_notifier_calendar::{CalendarHandleSlot, CalendarService};
use cross_notifier_core::CoreState;
use font::FontAtlas;
use gpu::{GpuContext, WindowSurface};
use notification::{NotificationPayload, NotificationQueue};
use renderer::Renderer2D;
use server::{ConnectionMap, ConnectionState};
use store::SharedStore;
use tray::TrayState;

const NOTIFICATION_W: u32 = (CARD_W + card::PADDING * 2.0) as u32;
const DEFAULT_PORT: u16 = 9876;
const FONT_SIZE: f32 = 14.0;

struct App {
    event_proxy: EventLoopProxy<AppEvent>,
    config: Config,
    port: u16,

    // Windowing & rendering
    window: Option<Arc<Window>>,
    gpu: Option<GpuContext>,
    popup_surface: Option<WindowSurface>,
    renderer: Option<Renderer2D>,
    font: Option<FontAtlas>,

    // System tray
    tray: Option<TrayState>,

    // Notification state
    queue: NotificationQueue,
    store: SharedStore,

    // Notification center
    center: Option<center::CenterState>,

    // Settings window
    settings: Option<settings::SettingsWindow>,

    // Connection tracking
    connections: ConnectionMap,
    client_handles: Vec<client::ClientHandle>,

    // Embedded notification core — shared with the HTTP server (for
    // /notify, /ws, etc.) and the calendar service (for reminder
    // delivery). Held at the App level so we can spawn/restart the
    // calendar without taking the HTTP server down.
    core: CoreState,
    calendar_slot: CalendarHandleSlot,
    calendar: Option<CalendarService>,

    // Tokio runtime (owned, kept alive)
    _runtime: tokio::runtime::Runtime,

    // Config file watcher (owned, kept alive)
    _config_watcher: Option<RecommendedWatcher>,

    // Input state
    cursor_pos: (f64, f64),

    // Dismiss animation: remaining cards slide up to new positions
    dismiss_anim_start: Option<Instant>,

    // Hover-expand animation (0.0 = collapsed, 1.0 = fully expanded)
    expand_t: f32,
    last_frame_time: Instant,

    // Track whether we need to redraw
    needs_redraw: bool,
}

impl App {
    fn new(event_loop: &EventLoop<AppEvent>, port: u16) -> Self {
        let event_proxy = event_loop.create_proxy();

        // Create tokio runtime
        let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

        // Load config
        let config_path = Config::path();
        let config = Config::load(&config_path).unwrap_or_default();
        info!("Config loaded from {:?}", config_path);

        let connections: ConnectionMap = Arc::new(tokio::sync::RwLock::new(HashMap::new()));

        // Load notification store
        let store: SharedStore = Arc::new(std::sync::RwLock::new(store::NotificationStore::load(
            store::NotificationStore::default_path(),
        )));

        // Core with auth disabled (localhost only) — gives us the same
        // /notify, /notify/{id}/wait, /ws, /health, /openapi.* endpoints
        // the remote server exposes, with identical wait/maxWait behavior.
        // Owned at the App level so the calendar service can share it
        // across reloads without the HTTP server task also holding one.
        let core = CoreState::new("");
        let calendar_slot = CalendarHandleSlot::new();

        // Spawn the initial calendar service if the user already has one
        // configured. Later config changes reuse the same slot so routes
        // keep working while we swap services underneath.
        let calendar = if let Some(cal_cfg) = config.calendar.clone() {
            runtime.block_on(async {
                let svc = server::spawn_local_calendar(cal_cfg, core.clone(), port).await;
                if let Some(svc) = svc.as_ref() {
                    calendar_slot.set(Some(svc.handle()));
                }
                svc
            })
        } else {
            None
        };

        // Start HTTP server (uses EventLoopProxy to wake main thread).
        // The HTTP server mounts the calendar action router against the
        // shared slot, so reloads can swap handles without re-binding.
        let server_proxy = event_proxy.clone();
        let server_connections = connections.clone();
        let server_store = store.clone();
        runtime.spawn(server::run_server(
            port,
            server_proxy,
            server_connections,
            server_store,
            core.clone(),
            calendar_slot.clone(),
        ));

        // Start WebSocket clients for each configured server
        let mut client_handles = Vec::new();
        for srv in &config.servers {
            let handle = runtime.block_on(async {
                client::spawn_client(
                    srv.url.clone(),
                    srv.secret.clone(),
                    config.name.clone(),
                    srv.label.clone(),
                    event_proxy.clone(),
                )
            });
            client_handles.push(handle);
        }

        // Watch config file for external changes
        let config_watcher = Self::start_config_watcher(&event_proxy);

        Self {
            event_proxy,
            config,
            port: DEFAULT_PORT,
            window: None,
            gpu: None,
            popup_surface: None,
            renderer: None,
            font: None,
            tray: None,
            queue: NotificationQueue::new(),
            store,
            center: None,
            settings: None,
            connections,
            client_handles,
            core,
            calendar_slot,
            calendar,
            _runtime: runtime,
            _config_watcher: config_watcher,
            cursor_pos: (0.0, 0.0),
            dismiss_anim_start: None,
            expand_t: 0.0,
            last_frame_time: Instant::now(),
            needs_redraw: false,
        }
    }

    fn start_config_watcher(proxy: &EventLoopProxy<AppEvent>) -> Option<RecommendedWatcher> {
        let config_path = Config::path();
        let watch_dir = match config_path.parent() {
            Some(d) => d.to_path_buf(),
            None => return None,
        };
        let config_filename = config_path.file_name().map(|f| f.to_os_string());

        let proxy = proxy.clone();
        let watcher_result =
            notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    // Only react to data modifications
                    if !matches!(
                        event.kind,
                        notify::EventKind::Modify(notify::event::ModifyKind::Data(_))
                    ) {
                        return;
                    }
                    // Check if any of the affected paths match our config file
                    let is_config = event
                        .paths
                        .iter()
                        .any(|p| p.file_name().map(|f| f.to_os_string()) == config_filename);
                    if is_config {
                        let _ = proxy.send_event(AppEvent::ConfigChanged);
                    }
                }
            });

        match watcher_result {
            Ok(mut watcher) => {
                if let Err(e) = watcher.watch(&watch_dir, notify::RecursiveMode::NonRecursive) {
                    tracing::warn!("Failed to watch config directory: {}", e);
                    return None;
                }
                info!("Watching config at {:?}", watch_dir);
                Some(watcher)
            }
            Err(e) => {
                tracing::warn!("Failed to create config watcher: {}", e);
                None
            }
        }
    }

    fn create_notification_window(&self, event_loop: &ActiveEventLoop) -> Arc<Window> {
        // Get primary monitor work area for positioning
        let monitor = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next());

        let (pos_x, pos_y) = if let Some(mon) = &monitor {
            let size = mon.size();
            let pos = mon.position();
            let scale = mon.scale_factor();
            let phys_w = (NOTIFICATION_W as f64 * scale) as i32;
            // Top-right corner with padding
            (
                pos.x + size.width as i32 - phys_w - (16.0 * scale) as i32,
                pos.y + (40.0 * scale) as i32, // Below menu bar on macOS
            )
        } else {
            (100, 100)
        };

        let attrs = WindowAttributes::default()
            .with_title("cross-notifier")
            .with_inner_size(LogicalSize::new(NOTIFICATION_W, 120))
            .with_position(PhysicalPosition::new(pos_x, pos_y))
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_resizable(false);

        let window = event_loop
            .create_window(attrs)
            .expect("Failed to create notification window");

        // Start hidden, show when we have notifications
        window.set_visible(false);

        Arc::new(window)
    }

    fn update_tray_icon(&self) {
        if let Some(tray) = &self.tray {
            // The dot signals "there's something in the center worth
            // opening." Live popups are their own on-screen indicator —
            // they don't need to also light the dot, because once a
            // popup expires/dismisses without store_on_expire there
            // would be nothing in the center for the user to find.
            let store_count = self.store.read().unwrap().count();
            tray.set_has_notifications(store_count > 0);
            tray.set_notification_count(store_count);
        }
    }

    fn handle_notification(&mut self, server_label: String, payload: NotificationPayload) {
        info!(
            "Notification from {}: {} - {}",
            server_label, payload.title, payload.message
        );

        // Rule matching
        let (sound_name, action) =
            match rules::match_rule(&payload, &server_label, &self.config.rules) {
                Some(m) => (m.sound.to_string(), m.action.clone()),
                None => (String::new(), config::RuleAction::Normal),
            };

        // Dismiss: discard entirely
        if action == config::RuleAction::Dismiss {
            info!(
                "Rule dismissed notification: {} - {}",
                payload.title, payload.message
            );
            return;
        }

        // Silent: store to center only, no popup or sound
        if action == config::RuleAction::Silent {
            info!(
                "Rule silenced notification: {} - {}",
                payload.title, payload.message
            );
            self.store.write().unwrap().add(payload, server_label);
            self.update_tray_icon();
            self.needs_redraw = true;
            return;
        }

        // Normal action — play sound if specified
        if !sound_name.is_empty() && sound_name != "none" {
            sound::play_sound(&sound_name);
        }

        // If center is open, send directly to store (no popup)
        if self.center.is_some() {
            self.store.write().unwrap().add(payload, server_label);
            self.update_tray_icon();
            self.needs_redraw = true;
            return;
        }

        // Check if we need an async icon fetch before adding (borrows payload)
        let needs_async = icon::needs_async_fetch(&payload);
        let icon_href = if needs_async {
            Some(payload.icon_href.clone())
        } else {
            None
        };

        // Try synchronous icon loading (base64 or file path)
        let icon_image = icon::resolve_sync(&payload);

        let id = self.queue.add(server_label, payload);

        // Set the sync-loaded icon on the notification
        if let Some(img) = icon_image {
            self.upload_icon(id, img);
        }

        // Spawn async URL fetch if needed
        if let Some(url) = icon_href {
            let proxy = self.event_proxy.clone();
            self._runtime.spawn(async move {
                match icon::load_from_url(&url).await {
                    Ok(img) => {
                        let _ = proxy.send_event(AppEvent::IconLoaded {
                            notification_id: id,
                            image: img,
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Failed to fetch icon from {}: {}", url, e);
                    }
                }
            });
        }

        self.needs_redraw = true;
        self.update_tray_icon();

        // Show window if hidden
        if let Some(window) = &self.window
            && !self.queue.is_empty()
        {
            window.set_visible(true);
        }
    }

    fn handle_click(&mut self) {
        if self.queue.is_empty() {
            return;
        }

        let font = match &mut self.font {
            Some(f) => f,
            None => return,
        };

        // Only the topmost card is clickable
        let notifications = self.queue.visible();
        let first = &notifications[0];
        let has_icon = first.icon_bind_group.is_some();
        let h = card::card_height(font, first.message(), has_icon, self.expand_t);

        let (cx, cy) = self.cursor_pos;
        let card_x = card::PADDING as f64;
        let card_y = 0.0; // topmost card starts at y=0
        let card_w = card::CARD_W as f64;

        if cx >= card_x && cx <= card_x + card_w && cy >= card_y && cy <= card_y + h as f64 {
            let id = first.id;
            info!("Click-dismiss notification {}", id);
            let had_more = self.queue.visible().len() > 1;
            self.queue.dismiss(id);
            self.expand_t = 0.0; // reset expand for new topmost
            if had_more && !self.queue.is_empty() {
                self.dismiss_anim_start = Some(Instant::now());
            }
            self.needs_redraw = true;
            self.update_tray_icon();
        }
    }

    fn upload_icon(&mut self, notification_id: i64, image: image::RgbaImage) {
        let (gpu, renderer) = match (&self.gpu, &self.renderer) {
            (Some(g), Some(r)) => (g, r),
            _ => return,
        };

        let bind_group = renderer.upload_texture(gpu, &image);

        if let Some(notification) = self.queue.get_mut(notification_id) {
            notification.icon = Some(image);
            notification.icon_bind_group = Some(bind_group);
        }
    }

    fn open_center(&mut self, event_loop: &ActiveEventLoop) {
        if self.center.is_some() {
            return; // Already open
        }
        let gpu = match &self.gpu {
            Some(g) => g,
            None => return,
        };

        // Position at right edge of screen
        let monitor = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next());

        let center_w = center::CENTER_W as u32;
        let (pos_x, pos_y, height) = if let Some(mon) = &monitor {
            let size = mon.size();
            let pos = mon.position();
            let scale = mon.scale_factor();
            let phys_w = (center_w as f64 * scale) as i32;
            (
                pos.x + size.width as i32 - phys_w,
                pos.y + (40.0 * scale) as i32, // Below menu bar on macOS
                ((size.height as f64 / scale) as u32)
                    .saturating_sub(80)
                    .min(700), // Leave room for dock (logical)
            )
        } else {
            (100, 100, 500)
        };

        let attrs = WindowAttributes::default()
            .with_title("Notifications")
            .with_inner_size(LogicalSize::new(center_w, height))
            .with_position(PhysicalPosition::new(pos_x, pos_y))
            .with_decorations(false)
            .with_transparent(true)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_resizable(false);

        let window = event_loop
            .create_window(attrs)
            .expect("Failed to create center window");
        let window = Arc::new(window);

        let surface = gpu
            .create_surface(window.clone())
            .expect("Failed to create center surface");

        // Set projection for center window
        if let Some(renderer) = &self.renderer {
            renderer.resize(gpu, surface.size.0, surface.size.1);
        }

        let mut center = center::CenterState::new(window.clone(), surface);

        // Load existing icons
        if let (Some(gpu), Some(renderer)) = (&self.gpu, &self.renderer) {
            center.ensure_icons(&self.store, gpu, renderer);
        }

        // Focus the window so Focused(false) fires on click-away
        window.focus_window();

        self.center = Some(center);
        info!("Notification center opened");
    }

    fn close_center(&mut self) {
        if let Some(center) = &mut self.center {
            center.start_closing();
        }
    }

    fn toggle_center(&mut self, event_loop: &ActiveEventLoop) {
        if self.center.is_some() {
            self.close_center();
        } else {
            self.open_center(event_loop);
        }
    }

    fn render_center(&mut self) {
        // Must take center out to avoid borrow conflicts with gpu/renderer/font/store
        let mut center = match self.center.take() {
            Some(c) => c,
            None => return,
        };

        let gpu = match &self.gpu {
            Some(g) => g,
            None => {
                self.center = Some(center);
                return;
            }
        };
        let renderer = match &mut self.renderer {
            Some(r) => r,
            None => {
                self.center = Some(center);
                return;
            }
        };
        let font = match &mut self.font {
            Some(f) => f,
            None => {
                self.center = Some(center);
                return;
            }
        };

        // Set projection for center window dimensions
        renderer.resize(gpu, center.surface.size.0, center.surface.size.1);

        // Delta time
        let now = Instant::now();
        let dt = (now - self.last_frame_time).as_secs_f32().min(0.1);

        // Ensure icons are loaded
        center.ensure_icons(&self.store, gpu, renderer);
        center.prune_icons(&self.store);

        center.render(gpu, renderer, font, &self.store, dt);

        // Check if fully closed
        if center.is_fully_closed() {
            info!("Notification center closed");
            // Don't put it back — drop it
        } else {
            self.center = Some(center);
        }
    }

    fn handle_center_click(&mut self) {
        let click = {
            let center = match &self.center {
                Some(c) => c,
                None => return,
            };
            center.on_click(&self.store)
        };

        if let Some(click) = click {
            match click {
                center::CenterClick::Close => {
                    self.close_center();
                }
                center::CenterClick::ClearAll => {
                    self.store.write().unwrap().clear();
                    self.update_tray_icon();
                    self.needs_redraw = true;
                }
                center::CenterClick::Action(id, idx) => {
                    self.execute_action(id, idx);
                }
                center::CenterClick::CardBody(id) => {
                    self.store.write().unwrap().remove(id);
                    self.update_tray_icon();
                    self.needs_redraw = true;
                }
            }
        }
    }

    fn execute_action(&mut self, store_id: i64, action_idx: usize) {
        let action = {
            let store = self.store.read().unwrap();
            let notif = match store.get(store_id) {
                Some(n) => n,
                None => return,
            };
            match notif.payload.actions.get(action_idx) {
                Some(a) => a.clone(),
                None => return,
            }
        };

        info!(
            "Executing action '{}' on notification {}",
            action.label, store_id
        );

        if action.open {
            // Open URL in browser
            let _ = open::that(&action.url);
            // Dismiss after opening
            self.store.write().unwrap().remove(store_id);
            self.update_tray_icon();
        } else if !action.url.is_empty() {
            // HTTP request
            let url = action.url.clone();
            let method = if action.method.is_empty() {
                "GET".to_string()
            } else {
                action.method.clone()
            };
            let headers = action.headers.clone();
            let body = action.body.clone();
            let store = self.store.clone();
            let proxy = self.event_proxy.clone();

            self._runtime.spawn(async move {
                let client = reqwest::Client::new();
                let mut req = match method.to_uppercase().as_str() {
                    "POST" => client.post(&url),
                    "PUT" => client.put(&url),
                    "DELETE" => client.delete(&url),
                    "PATCH" => client.patch(&url),
                    _ => client.get(&url),
                };
                for (k, v) in &headers {
                    req = req.header(k.as_str(), v.as_str());
                }
                if !body.is_empty() {
                    req = req.body(body);
                }

                match req.send().await {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            // Remove on success
                            store.write().unwrap().remove(store_id);
                            let _ = proxy.send_event(AppEvent::CenterDirty);
                        } else {
                            tracing::warn!(
                                "Action failed with status {} for notification {}",
                                resp.status(),
                                store_id
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Action request failed for notification {}: {}",
                            store_id,
                            e
                        );
                    }
                }
            });
        }
    }

    fn handle_popup_window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let (Some(gpu), Some(surface)) = (&self.gpu, &mut self.popup_surface) {
                    surface.resize(gpu, size.width, size.height);
                    if let Some(renderer) = &self.renderer {
                        renderer.resize(gpu, size.width, size.height);
                    }
                }
                self.needs_redraw = true;
            }
            WindowEvent::RedrawRequested => {
                self.render_popup();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = (position.x, position.y);
                if !self.queue.is_empty() {
                    self.needs_redraw = true;
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_pos = (-1.0, -1.0);
                if !self.queue.is_empty() {
                    self.needs_redraw = true;
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.handle_click();
            }
            _ => {}
        }
    }

    fn handle_center_window_event(&mut self, _event_loop: &ActiveEventLoop, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.close_center();
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &self.gpu
                    && let Some(center) = &mut self.center
                {
                    center.surface.resize(gpu, size.width, size.height);
                }
                self.needs_redraw = true;
            }
            WindowEvent::RedrawRequested => {
                self.render_center();
                // Restore popup projection after center render
                if let (Some(gpu), Some(renderer), Some(surface)) =
                    (&self.gpu, &self.renderer, &self.popup_surface)
                {
                    renderer.resize(gpu, surface.size.0, surface.size.1);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(center) = &mut self.center {
                    center.on_cursor_moved(position.x, position.y);
                }
                self.needs_redraw = true;
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(center) = &mut self.center {
                    center.on_cursor_left();
                }
                self.needs_redraw = true;
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.handle_center_click();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 28.0,
                };
                if let Some(center) = &mut self.center {
                    center.on_scroll(dy);
                }
                self.needs_redraw = true;
            }
            WindowEvent::Focused(false)
                if self
                    .center
                    .as_ref()
                    .is_some_and(|c| c.should_close_on_focus_loss()) =>
            {
                self.close_center();
            }
            _ => {}
        }
    }

    // ── Settings window ────────────────────────────────────────────────

    fn open_settings(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(settings) = &self.settings {
            settings.window.focus_window();
            return;
        }

        let gpu = match &self.gpu {
            Some(g) => g,
            None => return,
        };

        // Snapshot connection status for settings display
        let connection_status: HashMap<String, ConnectionState> = {
            let connections = self.connections.clone();
            self._runtime.block_on(async {
                let map = connections.read().await;
                map.clone()
            })
        };

        let settings = settings::SettingsWindow::new(
            event_loop,
            gpu,
            &self.config,
            &connection_status,
            self.connections.clone(),
            self.event_proxy.clone(),
        );

        self.settings = Some(settings);
        info!("Settings window opened");
    }

    fn handle_settings_window_event(&mut self, event: WindowEvent) {
        // Let egui handle the event first
        let consumed = if let Some(settings) = &mut self.settings {
            settings.on_window_event(&event)
        } else {
            return;
        };

        match &event {
            WindowEvent::CloseRequested => {
                self.settings = None;
                info!("Settings window closed");
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &self.gpu
                    && let Some(settings) = &mut self.settings
                {
                    settings.surface.resize(gpu, size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(gpu) = &self.gpu
                    && let Some(settings) = &mut self.settings
                {
                    settings.render(gpu);
                }
                // Check if settings window produced a result
                let result = self.settings.as_mut().and_then(|s| s.take_result());
                if let Some(result) = result {
                    match result {
                        settings::SettingsResult::Save(new_config) => {
                            self.save_and_apply_config(new_config);
                        }
                        settings::SettingsResult::Cancel => {}
                    }
                    self.settings = None;
                    info!("Settings window closed");
                }
            }
            _ => {
                // egui already handled it above
                if consumed {
                    self.needs_redraw = true;
                }
            }
        }
    }

    fn save_and_apply_config(&mut self, new_config: Config) {
        let config_path = Config::path();
        if let Err(e) = new_config.save(&config_path) {
            tracing::error!("Failed to save config: {}", e);
        }
        info!("Configuration saved to {:?}", config_path);
        self.apply_config(new_config);
    }

    fn apply_config(&mut self, new_config: Config) {
        // Check if servers changed
        let servers_changed =
            self.config.servers != new_config.servers || self.config.name != new_config.name;
        let calendar_changed = self.config.calendar != new_config.calendar;
        let tray_style_changed = self.config.tray_icon_style != new_config.tray_icon_style;

        self.config = new_config;

        if tray_style_changed && let Some(tray) = &self.tray {
            tray.set_theme_override(self.config.tray_icon_style);
        }

        if calendar_changed {
            self.reload_calendar();
        }

        if servers_changed {
            // Drop old clients
            self.client_handles.clear();

            // Spawn new clients
            for srv in &self.config.servers {
                let handle = self._runtime.block_on(async {
                    client::spawn_client(
                        srv.url.clone(),
                        srv.secret.clone(),
                        self.config.name.clone(),
                        srv.label.clone(),
                        self.event_proxy.clone(),
                    )
                });
                self.client_handles.push(handle);
            }

            info!("Reconnected {} servers", self.config.servers.len());
        }
    }

    /// Replace the running calendar service with one built from the
    /// current config. Called after the user saves new calendar settings
    /// (or the on-disk config file changes). Shuts the old service down
    /// cleanly before starting the replacement so they don't race on the
    /// shared state file.
    fn reload_calendar(&mut self) {
        let cfg = self.config.calendar.clone();
        let core = self.core.clone();
        let slot = self.calendar_slot.clone();
        let port = self.port;

        // Take the existing service out before spawning the new one so we
        // don't hold two services for the same state file at once.
        let old = self.calendar.take();
        slot.set(None);

        let new_svc = self._runtime.block_on(async move {
            if let Some(svc) = old {
                svc.shutdown().await;
            }
            match cfg {
                Some(cal_cfg) => server::spawn_local_calendar(cal_cfg, core, port).await,
                None => None,
            }
        });

        if let Some(svc) = new_svc.as_ref() {
            slot.set(Some(svc.handle()));
            info!("Calendar service reloaded");
        } else {
            info!("Calendar service stopped");
        }
        self.calendar = new_svc;
    }

    /// Manually reconnect a single server by dropping its client handle
    /// and spawning a fresh one. The handles vector is parallel to
    /// `self.config.servers`, so we find the index by URL.
    fn reconnect_server(&mut self, url: &str) {
        let Some(idx) = self.config.servers.iter().position(|s| s.url == url) else {
            tracing::warn!("Reconnect requested for unknown server: {}", url);
            return;
        };
        let srv = self.config.servers[idx].clone();
        info!("Manual reconnect requested for {}", srv.label);

        // Mark disconnected immediately so the indicator reflects the drop.
        // Clear any stale error — user is retrying, so don't show old failures
        // until the new attempt produces its own.
        let connections = self.connections.clone();
        let url_owned = srv.url.clone();
        self._runtime.block_on(async {
            let mut map = connections.write().await;
            // Manual reconnect — drop stale server-advertised calendars
            // so the UI doesn't claim the server is still pushing them
            // until the new ServerInfo arrives.
            map.insert(
                url_owned,
                ConnectionState {
                    connected: false,
                    last_error: None,
                    server_calendars: Vec::new(),
                },
            );
        });

        // Drop old handle (triggers shutdown oneshot, cancelling the WS task)
        // before spawning the new one, so both aren't racing to report status.
        let new_handle = self._runtime.block_on(async {
            client::spawn_client(
                srv.url.clone(),
                srv.secret.clone(),
                self.config.name.clone(),
                srv.label.clone(),
                self.event_proxy.clone(),
            )
        });
        self.client_handles[idx] = new_handle;

        if let Some(settings) = &self.settings {
            settings.window.request_redraw();
        }
    }

    fn render_popup(&mut self) {
        let (gpu, surface) = match (&self.gpu, &self.popup_surface) {
            (Some(g), Some(s)) => (g, s),
            _ => return,
        };
        let renderer = match &mut self.renderer {
            Some(r) => r,
            None => return,
        };
        let font = match &mut self.font {
            Some(f) => f,
            None => return,
        };

        // Delta time for animations
        let now = Instant::now();
        let dt = (now - self.last_frame_time).as_secs_f32().min(0.1);
        self.last_frame_time = now;

        // Hover-expand animation: compute hover, animate expand_t, sync expanded flag
        let topmost_hovered;
        if !self.queue.is_empty() {
            // Extract data from topmost to avoid holding borrow on queue
            let (first_id, message, has_icon) = {
                let first = &self.queue.visible()[0];
                (
                    first.id,
                    first.message().to_string(),
                    first.icon_bind_group.is_some(),
                )
            };

            // Smoothstep easing for expand animation
            let eased_t = self.expand_t * self.expand_t * (3.0 - 2.0 * self.expand_t);
            let h = card::card_height(font, &message, has_icon, eased_t);
            let (cx, cy) = self.cursor_pos;
            topmost_hovered = cx >= card::PADDING as f64
                && cx <= (card::PADDING + card::CARD_W) as f64
                && cy >= 0.0
                && cy <= h as f64;

            // Animate expand_t towards target
            let overflows = card::message_overflows(font, &message, has_icon);
            let expand_target = if topmost_hovered && overflows {
                1.0
            } else {
                0.0
            };
            const EXPAND_SPEED: f32 = 6.0; // ~170ms full transition
            if self.expand_t < expand_target {
                self.expand_t = (self.expand_t + EXPAND_SPEED * dt).min(1.0);
                self.needs_redraw = true;
            } else if self.expand_t > expand_target {
                self.expand_t = (self.expand_t - EXPAND_SPEED * dt).max(0.0);
                self.needs_redraw = true;
            }

            // Sync expanded flag so is_expired() respects hover-expand
            if let Some(n) = self.queue.get_mut(first_id) {
                n.expanded = self.expand_t > 0.0;
            }
        } else {
            topmost_hovered = false;
            self.expand_t = 0.0;
        }

        // Prune expired notifications (respects expanded flag set above)
        let had_notifications = !self.queue.is_empty();
        let expired = self.queue.prune_expired();
        // Move store-on-expire entries into the center *before* refreshing
        // the tray, so the dot reflects the post-prune store count.
        for expired_notif in &expired {
            if expired_notif.payload.store_on_expire {
                self.store.write().unwrap().add(
                    expired_notif.payload.clone(),
                    expired_notif.server_label.clone(),
                );
            }
        }
        if !expired.is_empty() {
            if had_notifications && !self.queue.is_empty() {
                self.dismiss_anim_start = Some(Instant::now());
            }
            // Inlined update_tray_icon — `self.update_tray_icon()` would
            // conflict with the &mut self.renderer borrow held by the
            // surrounding render path.
            if let Some(tray) = &self.tray {
                let store_count = self.store.read().unwrap().count();
                tray.set_has_notifications(store_count > 0);
                tray.set_notification_count(store_count);
            }
        }

        // Hide window if no notifications
        if self.queue.is_empty() {
            if let Some(window) = &self.window {
                window.set_visible(false);
            }
            return;
        }

        // Eased expand_t for rendering
        let eased_expand = self.expand_t * self.expand_t * (3.0 - 2.0 * self.expand_t);

        // Collect notification refs for sizing
        let notification_refs: Vec<&notification::Notification> = self
            .queue
            .visible()
            .iter()
            .take(card::MAX_VISIBLE)
            .collect();

        // Resize window to fit content
        let target_h = card::total_height(font, &notification_refs, eased_expand);
        if let Some(window) = &self.window {
            let current = window.inner_size();
            let target_h_u32 = target_h.ceil() as u32;
            if current.height != target_h_u32 || current.width != NOTIFICATION_W {
                let _ = window.request_inner_size(LogicalSize::new(NOTIFICATION_W, target_h_u32));
            }
        }

        // Ensure font atlas texture is up-to-date
        font.ensure_gpu_texture(gpu, renderer);

        let output = match surface.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                // Reconfigure on next resize; about_to_wait will schedule a redraw
                return;
            }
            Err(e) => {
                tracing::error!("Surface error: {}", e);
                return;
            }
        };

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        renderer.begin_frame();

        let notifications = self.queue.visible();
        let count = notifications.len().min(card::MAX_VISIBLE);
        let atlas_bind_group = font.bind_group().cloned();

        // Dismiss animation progress (0.0 = just dismissed, 1.0 = settled)
        const DISMISS_DURATION: f32 = 0.2; // seconds
        let anim_t = if let Some(start) = self.dismiss_anim_start {
            let elapsed = start.elapsed().as_secs_f32();
            if elapsed >= DISMISS_DURATION {
                self.dismiss_anim_start = None;
                1.0
            } else {
                let t = elapsed / DISMISS_DURATION;
                // Ease-out: 1 - (1-t)^2
                1.0 - (1.0 - t) * (1.0 - t)
            }
        } else {
            1.0
        };

        // During animation, keep redrawing
        if anim_t < 1.0 {
            self.needs_redraw = true;
        }

        // Draw cards back-to-front so front cards cover back cards
        for ri in 0..count {
            let i = count - 1 - ri;
            let notification = &notifications[i];
            let hovered = i == 0 && topmost_hovered;

            // During animation, cards slide from old position (index+1) to new (index)
            let anim_index = i as f32 + (1.0 - anim_t);
            let y = anim_index * card::STACK_PEEK;

            // Only topmost card expands
            let card_expand_t = if i == 0 { eased_expand } else { 0.0 };

            let mut text_verts = Vec::new();
            let mut text_indices = Vec::new();

            card::draw_card(
                renderer,
                font,
                card::PADDING,
                y,
                card::CARD_W,
                notification,
                i,
                anim_t,
                hovered,
                card_expand_t,
                &mut text_verts,
                &mut text_indices,
            );

            // Submit this card's text right after its background
            if !text_verts.is_empty() {
                renderer.draw_text_batch(
                    &text_verts,
                    &text_indices,
                    atlas_bind_group.as_ref().unwrap(),
                );
            }
        }

        renderer.render(gpu, &view, atlas_bind_group.as_ref());
        output.present();
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init && self.tray.is_none() {
            self.tray = Some(TrayState::new(self.config.tray_icon_style));
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = self.create_notification_window(event_loop);
        let (gpu, popup_surface) =
            pollster::block_on(GpuContext::new(window.clone())).expect("Failed to initialize GPU");
        let renderer = Renderer2D::new(&gpu);
        let mut font = FontAtlas::new(FONT_SIZE);
        font.ensure_gpu_texture(&gpu, &renderer);

        // Set initial projection for popup window
        renderer.resize(&gpu, popup_surface.size.0, popup_surface.size.1);

        self.window = Some(window);
        self.gpu = Some(gpu);
        self.popup_surface = Some(popup_surface);
        self.renderer = Some(renderer);
        self.font = Some(font);

        info!("Renderer initialized, listening on port {}", self.port);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let is_popup = self.window.as_ref().map(|w| w.id()) == Some(window_id);
        let is_center = self.center.as_ref().map(|c| c.window.id()) == Some(window_id);
        let is_settings = self.settings.as_ref().map(|s| s.window.id()) == Some(window_id);

        if is_popup {
            self.handle_popup_window_event(event_loop, event);
        } else if is_center {
            self.handle_center_window_event(event_loop, event);
        } else if is_settings {
            self.handle_settings_window_event(event);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::IncomingNotification {
                server_label,
                payload,
            } => {
                self.handle_notification(server_label, payload);
            }
            AppEvent::ConnectionStatus {
                server_url,
                connected,
                error,
            } => {
                let connections = self.connections.clone();
                let url_for_map = server_url.clone();
                let error_for_map = error.clone();
                self._runtime.block_on(async {
                    let mut map = connections.write().await;
                    // Update in place so server-advertised calendars (set
                    // by ServerInfoReceived) survive transient status
                    // flips, but clear them on a hard disconnect — they
                    // could be stale by the time we reconnect.
                    let entry = map.entry(url_for_map).or_default();
                    entry.connected = connected;
                    entry.last_error = if connected { None } else { error_for_map };
                    if !connected {
                        entry.server_calendars.clear();
                    }
                });
                let status = if connected {
                    "connected"
                } else {
                    "disconnected"
                };
                info!("{} {}", server_url, status);
                // Nudge the settings window to repaint so its indicator reflects the new status.
                if let Some(settings) = &self.settings {
                    settings.window.request_redraw();
                }
            }
            AppEvent::ServerInfoReceived { server_url, info } => {
                info!(
                    "Server {} advertised {} calendar(s)",
                    server_url,
                    info.calendars.len()
                );
                let connections = self.connections.clone();
                self._runtime.block_on(async {
                    let mut map = connections.write().await;
                    let entry = map.entry(server_url).or_default();
                    entry.server_calendars = info.calendars;
                });
                if let Some(settings) = &self.settings {
                    settings.window.request_redraw();
                }
            }
            AppEvent::NotificationResolved(resolved) => {
                info!(
                    "Notification {} resolved by {}",
                    resolved.notification_id, resolved.resolved_by
                );
                if let Some(local_id) = self.queue.find_by_server_id(&resolved.notification_id) {
                    let had_more = self.queue.visible().len() > 1;
                    self.queue.dismiss(local_id);
                    if had_more && !self.queue.is_empty() {
                        self.dismiss_anim_start = Some(Instant::now());
                    }
                    self.needs_redraw = true;
                    self.update_tray_icon();
                }
            }
            AppEvent::NotificationExpired(expired) => {
                info!("Notification {} expired", expired.notification_id);
                // TODO: keep card visible and swap action buttons for a
                // disabled "Timed out" pill. For now we dismiss so stale
                // buttons don't remain live. See TODO.md.
                if let Some(local_id) = self.queue.find_by_server_id(&expired.notification_id) {
                    let had_more = self.queue.visible().len() > 1;
                    self.queue.dismiss(local_id);
                    if had_more && !self.queue.is_empty() {
                        self.dismiss_anim_start = Some(Instant::now());
                    }
                    self.needs_redraw = true;
                    self.update_tray_icon();
                }
            }
            AppEvent::IconLoaded {
                notification_id,
                image,
            } => {
                self.upload_icon(notification_id, image);
                self.needs_redraw = true;
            }
            AppEvent::ToggleCenter => {
                self.toggle_center(event_loop);
            }
            AppEvent::CenterDirty => {
                // Store changed via HTTP — center will pick up on next render
                self.update_tray_icon();
            }
            AppEvent::OpenSettings => {
                self.open_settings(event_loop);
            }
            AppEvent::ConfigChanged => {
                // Don't reload while settings window is open (user is editing)
                if self.settings.is_some() {
                    return;
                }
                let config_path = Config::path();
                match Config::load(&config_path) {
                    Ok(new_config) => {
                        info!("Config file changed, reloading");
                        self.apply_config(new_config);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to reload config: {}", e);
                    }
                }
            }
            AppEvent::ReconnectServer { url } => {
                self.reconnect_server(&url);
            }
            AppEvent::Quit => {
                event_loop.exit();
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        if let Some(center) = &self.center {
            center.window.request_redraw();
        }
        if let Some(settings) = &self.settings {
            settings.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if !self.queue.is_empty() {
            self.needs_redraw = true;
        }

        // Center needs continuous redraw during slide animation
        if let Some(center) = &self.center
            && center.is_animating()
        {
            self.needs_redraw = true;
        }

        if self.needs_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            if let Some(center) = &self.center {
                center.window.request_redraw();
            }
            self.needs_redraw = false;
        }

        // Settings window needs continuous redraws for egui cursor blink, etc.
        if let Some(settings) = &self.settings {
            settings.window.request_redraw();
        }
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Parse CLI flags
    let mut port = DEFAULT_PORT;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-install-autostart" | "--install-autostart" => {
                autostart::install_autostart()?;
                println!("Autostart installed.");
                return Ok(());
            }
            "-uninstall-autostart" | "--uninstall-autostart" => {
                autostart::uninstall_autostart()?;
                println!("Autostart uninstalled.");
                return Ok(());
            }
            "-port" | "--port" => {
                i += 1;
                port = args
                    .get(i)
                    .ok_or_else(|| anyhow::anyhow!("-port requires a value"))?
                    .parse::<u16>()
                    .map_err(|_| anyhow::anyhow!("-port value must be a valid port number"))?;
            }
            "-help" | "--help" | "-h" => {
                println!(
                    "Usage: cross-notifier [-port N] [-install-autostart] [-uninstall-autostart]"
                );
                return Ok(());
            }
            arg => {
                eprintln!("Unknown flag: {}", arg);
                eprintln!(
                    "Usage: cross-notifier [-port N] [-install-autostart] [-uninstall-autostart]"
                );
                std::process::exit(1);
            }
        }
        i += 1;
    }

    info!("cross-notifier daemon starting on port {}", port);

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;

    // Set up tray menu event handlers (must be before event loop starts)
    let proxy = event_loop.create_proxy();
    tray::setup_event_handlers(&proxy);

    let mut app = App::new(&event_loop, port);

    event_loop.run_app(&mut app)?;

    Ok(())
}
