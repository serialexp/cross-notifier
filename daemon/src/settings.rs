// Settings window using egui for form rendering.
// Separate egui rendering pipeline from the custom wgpu Renderer2D used by popup/center.

use std::collections::HashMap;
use std::sync::Arc;

use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

use crate::autostart;
use crate::config::{CenterPanelConfig, Config, NotificationRule, RuleAction, RulesConfig, Server};
use crate::gpu::{GpuContext, WindowSurface};
use crate::sound;

// ── Dropdown option lists ──────────────────────────────────────────────

const STATUS_OPTIONS: &[&str] = &["Any Status", "info", "success", "warning", "error"];
const ACTION_OPTIONS: &[&str] = &["Normal", "Silent", "Dismiss"];

fn sound_options() -> Vec<String> {
    let mut opts = vec!["No Sound".to_string()];
    opts.extend(sound::builtin_sounds().iter().map(|s| s.to_string()));
    opts
}

// ── Editable form state ────────────────────────────────────────────────

struct ServerEntry {
    label: String,
    url: String,
    secret: String,
    connected: bool,
}

struct RuleEntry {
    server_idx: usize, // 0 = "Any Server"
    source: String,
    status_idx: usize, // 0 = "Any Status"
    pattern: String,
    sound_idx: usize,  // 0 = "No Sound", 1..N = builtins, N+1 = custom file
    custom_sound: String, // file path when sound_idx == custom marker
    action_idx: usize, // 0 = Normal, 1 = Silent, 2 = Dismiss
}

/// Index in the sound_options list that represents "Custom File..."
fn custom_sound_idx() -> usize {
    sound_options().len()
}

#[derive(Default)]
struct SettingsState {
    name: String,
    servers: Vec<ServerEntry>,
    rules_enabled: bool,
    rules: Vec<RuleEntry>,
    autostart_enabled: bool,
    debug_font_metrics: bool,
}

impl SettingsState {
    fn from_config(config: &Config, connection_status: &HashMap<String, bool>) -> Self {
        let servers: Vec<ServerEntry> = config
            .servers
            .iter()
            .map(|s| ServerEntry {
                label: s.label.clone(),
                url: s.url.clone(),
                secret: s.secret.clone(),
                connected: connection_status
                    .get(&s.url)
                    .copied()
                    .unwrap_or(false),
            })
            .collect();

        let sound_opts = sound_options();

        let rules: Vec<RuleEntry> = config
            .rules
            .rules
            .iter()
            .map(|r| {
                // Map server name to index (0 = Any)
                let server_idx = if r.server.is_empty() {
                    0
                } else {
                    servers
                        .iter()
                        .position(|s| s.label == r.server)
                        .map(|i| i + 1)
                        .unwrap_or(0)
                };

                let status_idx = STATUS_OPTIONS
                    .iter()
                    .position(|s| *s == r.status)
                    .unwrap_or(0);

                let (sound_idx, custom_sound) = if r.sound.is_empty() {
                    (0, String::new())
                } else if let Some(idx) = sound_opts.iter().position(|s| s == &r.sound) {
                    (idx, String::new())
                } else {
                    // Not a builtin — treat as custom file path
                    (custom_sound_idx(), r.sound.clone())
                };

                let action_idx = match r.effective_action() {
                    RuleAction::Normal => 0,
                    RuleAction::Silent => 1,
                    RuleAction::Dismiss => 2,
                };

                RuleEntry {
                    server_idx,
                    source: r.source.clone(),
                    status_idx,
                    pattern: r.pattern.clone(),
                    sound_idx,
                    custom_sound,
                    action_idx,
                }
            })
            .collect();

        Self {
            name: config.name.clone(),
            servers,
            rules_enabled: config.rules.enabled,
            rules,
            autostart_enabled: autostart::is_autostart_installed(),
            debug_font_metrics: config.debug_font_metrics,
        }
    }

    fn to_config(&self) -> Config {
        let servers: Vec<Server> = self
            .servers
            .iter()
            .filter(|s| !s.url.is_empty()) // Skip empty entries
            .map(|s| Server {
                label: s.label.clone(),
                url: s.url.clone(),
                secret: s.secret.clone(),
            })
            .collect();

        let sound_opts = sound_options();

        let rules: Vec<NotificationRule> = self
            .rules
            .iter()
            .map(|r| {
                let server = if r.server_idx == 0 {
                    String::new()
                } else {
                    self.servers
                        .get(r.server_idx - 1)
                        .map(|s| s.label.clone())
                        .unwrap_or_default()
                };

                let status = if r.status_idx == 0 {
                    String::new()
                } else {
                    STATUS_OPTIONS
                        .get(r.status_idx)
                        .unwrap_or(&"")
                        .to_string()
                };

                let sound = if r.sound_idx == custom_sound_idx() {
                    r.custom_sound.clone()
                } else {
                    let sound_name = sound_opts.get(r.sound_idx).cloned().unwrap_or_default();
                    if sound_name == "No Sound" {
                        String::new()
                    } else {
                        sound_name
                    }
                };

                let action = match r.action_idx {
                    1 => RuleAction::Silent,
                    2 => RuleAction::Dismiss,
                    _ => RuleAction::Normal,
                };

                NotificationRule {
                    server,
                    source: r.source.clone(),
                    status,
                    pattern: r.pattern.clone(),
                    sound,
                    action,
                    suppress: false,
                }
            })
            .collect();

        Config {
            name: self.name.clone(),
            servers,
            rules: RulesConfig {
                enabled: self.rules_enabled,
                rules,
            },
            center_panel: CenterPanelConfig::default(),
            debug_font_metrics: self.debug_font_metrics,
        }
    }
}

// ── Settings result ────────────────────────────────────────────────────

pub enum SettingsResult {
    Save(Config),
    Cancel,
}

// ── Settings window ────────────────────────────────────────────────────

pub struct SettingsWindow {
    pub window: Arc<Window>,
    pub surface: WindowSurface,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    state: SettingsState,
    result: Option<SettingsResult>,
}

impl SettingsWindow {
    pub fn new(
        event_loop: &ActiveEventLoop,
        gpu: &GpuContext,
        config: &Config,
        connection_status: &HashMap<String, bool>,
    ) -> Self {
        let attrs = WindowAttributes::default()
            .with_title("Cross-Notifier Settings")
            .with_inner_size(LogicalSize::new(700u32, 700u32))
            .with_decorations(true)
            .with_resizable(true);

        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("Failed to create settings window"),
        );

        let surface = gpu
            .create_surface(window.clone())
            .expect("Failed to create settings surface");

        let egui_ctx = egui::Context::default();

        // Dark theme matching our notification style
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = egui::Color32::from_rgb(30, 30, 36);
        visuals.panel_fill = egui::Color32::from_rgb(30, 30, 36);
        visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(38, 38, 44);
        egui_ctx.set_visuals(visuals);

        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &*window,
            Some(window.scale_factor() as f32),
            None,
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );

        let egui_renderer = egui_wgpu::Renderer::new(
            &gpu.device,
            gpu.surface_format(),
            None,  // no depth format
            1,     // no MSAA
            true,  // dithering
        );

        let state = SettingsState::from_config(config, connection_status);

        Self {
            window,
            surface,
            egui_ctx,
            egui_state,
            egui_renderer,
            state,
            result: None,
        }
    }

    /// Forward a winit event to egui. Returns true if egui consumed it.
    pub fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        let response = self.egui_state.on_window_event(&self.window, event);
        if response.repaint {
            self.window.request_redraw();
        }
        response.consumed
    }

    /// Full egui render cycle.
    pub fn render(&mut self, gpu: &GpuContext) {
        let raw_input = self.egui_state.take_egui_input(&self.window);

        // Take state out to avoid borrow conflict with egui_ctx.run()
        let mut state = std::mem::take(&mut self.state);
        let mut result = self.result.take();

        let full_output = self.egui_ctx.run(raw_input, |ctx| {
            draw_settings_ui(ctx, &mut state, &mut result);
        });

        self.state = state;
        self.result = result;

        self.egui_state
            .handle_platform_output(&self.window, full_output.platform_output);

        let paint_jobs = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        // Update textures
        for (id, delta) in &full_output.textures_delta.set {
            self.egui_renderer
                .update_texture(&gpu.device, &gpu.queue, *id, delta);
        }

        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.surface.size.0, self.surface.size.1],
            pixels_per_point: self.window.scale_factor() as f32,
        };

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("egui settings encoder"),
            });

        let user_cmd_bufs = self.egui_renderer.update_buffers(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &paint_jobs,
            &screen,
        );

        let output = match self.surface.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.resize(gpu, self.surface.size.0, self.surface.size.1);
                return;
            }
            Err(e) => {
                tracing::error!("Settings surface error: {}", e);
                return;
            }
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        {
            let mut rpass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui settings"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.12,
                                g: 0.12,
                                b: 0.14,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();

            self.egui_renderer.render(&mut rpass, &paint_jobs, &screen);
        }

        let mut cmd_bufs: Vec<wgpu::CommandBuffer> = user_cmd_bufs;
        cmd_bufs.push(encoder.finish());
        gpu.queue.submit(cmd_bufs);
        output.present();

        // Free textures
        for id in &full_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }

    pub fn take_result(&mut self) -> Option<SettingsResult> {
        self.result.take()
    }

}

// ── UI drawing (free functions to avoid borrow conflicts) ──────────────

fn draw_settings_ui(
    ctx: &egui::Context,
    state: &mut SettingsState,
    result: &mut Option<SettingsResult>,
) {
    egui::CentralPanel::default().show(ctx, |ui| {
        // Fixed bottom buttons
        egui::TopBottomPanel::bottom("actions").show_inside(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    apply_autostart_from_state(state);
                    *result = Some(SettingsResult::Save(state.to_config()));
                }
                if ui.button("Cancel").clicked() {
                    *result = Some(SettingsResult::Cancel);
                }
            });
            ui.add_space(4.0);
        });

        // Scrollable content
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Cross-Notifier Settings");
            ui.add_space(8.0);

            // Client name
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut state.name);
            });

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            draw_servers(ui, state);

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            draw_rules(ui, state);

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label("Startup:");
            ui.checkbox(&mut state.autostart_enabled, "Start automatically on login");

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label("Debug:");
            ui.checkbox(&mut state.debug_font_metrics, "Show font metrics overlay");

            ui.add_space(16.0);
        });
    });
}

fn draw_servers(ui: &mut egui::Ui, state: &mut SettingsState) {
    ui.label("Notification Servers:");
    ui.add_space(4.0);

    let mut to_remove: Option<usize> = None;

    for i in 0..state.servers.len() {
        ui.horizontal(|ui| {
            // Connection indicator
            let color = if state.servers[i].connected {
                egui::Color32::from_rgb(51, 204, 76)
            } else {
                egui::Color32::from_rgb(128, 128, 128)
            };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 5.0, color);

            ui.label("Label:");
            let label_edit = egui::TextEdit::singleline(&mut state.servers[i].label)
                .desired_width(70.0)
                .hint_text("Work");
            ui.add(label_edit);

            ui.label("URL:");
            let url_edit = egui::TextEdit::singleline(&mut state.servers[i].url)
                .desired_width(180.0)
                .hint_text("ws://host:9876/ws");
            ui.add(url_edit);

            ui.label("Secret:");
            let secret_edit = egui::TextEdit::singleline(&mut state.servers[i].secret)
                .desired_width(100.0)
                .password(true);
            ui.add(secret_edit);

            if ui.button("X").clicked() {
                to_remove = Some(i);
            }
        });
    }

    if let Some(idx) = to_remove {
        state.servers.remove(idx);
    }

    ui.add_space(4.0);
    if ui.button("+ Add Server").clicked() {
        state.servers.push(ServerEntry {
            label: String::new(),
            url: String::new(),
            secret: String::new(),
            connected: false,
        });
    }
}

fn draw_rules(ui: &mut egui::Ui, state: &mut SettingsState) {
    ui.label("Notification Rules:");
    ui.checkbox(&mut state.rules_enabled, "Enable notification rules");

    if !state.rules_enabled {
        return;
    }

    ui.add_space(4.0);

    // Build server options for dropdown: "Any Server" + server labels
    let server_options: Vec<String> = std::iter::once("Any Server".to_string())
        .chain(state.servers.iter().map(|s| {
            if s.label.is_empty() {
                s.url.clone()
            } else {
                s.label.clone()
            }
        }))
        .collect();

    let sound_opts = sound_options();

    let mut to_remove: Option<usize> = None;
    let mut play_sound_name: Option<String> = None;

    for i in 0..state.rules.len() {
        egui::Frame::default()
            .inner_margin(8.0)
            .fill(egui::Color32::from_rgb(38, 38, 44))
            .corner_radius(4.0)
            .show(ui, |ui| {
                // Row 1: Filters
                ui.horizontal(|ui| {
                    ui.label("If:");

                    let server_label = server_options
                        .get(state.rules[i].server_idx)
                        .cloned()
                        .unwrap_or_else(|| "Any Server".to_string());
                    egui::ComboBox::from_id_salt(format!("rule_server_{}", i))
                        .selected_text(&server_label)
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            for (idx, opt) in server_options.iter().enumerate() {
                                ui.selectable_value(
                                    &mut state.rules[i].server_idx,
                                    idx,
                                    opt,
                                );
                            }
                        });

                    ui.label("Source:");
                    let source_edit = egui::TextEdit::singleline(&mut state.rules[i].source)
                        .desired_width(70.0)
                        .hint_text("any");
                    ui.add(source_edit);

                    let status_label = STATUS_OPTIONS
                        .get(state.rules[i].status_idx)
                        .unwrap_or(&"Any Status");
                    egui::ComboBox::from_id_salt(format!("rule_status_{}", i))
                        .selected_text(*status_label)
                        .width(90.0)
                        .show_ui(ui, |ui| {
                            for (idx, opt) in STATUS_OPTIONS.iter().enumerate() {
                                ui.selectable_value(
                                    &mut state.rules[i].status_idx,
                                    idx,
                                    *opt,
                                );
                            }
                        });

                    ui.label("Pattern:");
                    let pattern_edit = egui::TextEdit::singleline(&mut state.rules[i].pattern)
                        .desired_width(70.0)
                        .hint_text("regex");
                    ui.add(pattern_edit);

                    if ui.button("X").clicked() {
                        to_remove = Some(i);
                    }
                });

                // Row 2: Actions
                ui.horizontal(|ui| {
                    ui.label("Then:");

                    let action_label = ACTION_OPTIONS
                        .get(state.rules[i].action_idx)
                        .unwrap_or(&"Normal");
                    egui::ComboBox::from_id_salt(format!("rule_action_{}", i))
                        .selected_text(*action_label)
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for (idx, opt) in ACTION_OPTIONS.iter().enumerate() {
                                ui.selectable_value(
                                    &mut state.rules[i].action_idx,
                                    idx,
                                    *opt,
                                );
                            }
                        });

                    // Sound dropdown + file browser (only for Normal action)
                    if state.rules[i].action_idx == 0 {
                        let custom_idx = custom_sound_idx();
                        let sound_label = if state.rules[i].sound_idx == custom_idx {
                            // Show filename only for custom paths
                            std::path::Path::new(&state.rules[i].custom_sound)
                                .file_name()
                                .map(|f| f.to_string_lossy().to_string())
                                .unwrap_or_else(|| "Custom File".to_string())
                        } else {
                            sound_opts
                                .get(state.rules[i].sound_idx)
                                .cloned()
                                .unwrap_or_else(|| "No Sound".to_string())
                        };
                        egui::ComboBox::from_id_salt(format!("rule_sound_{}", i))
                            .selected_text(&sound_label)
                            .width(100.0)
                            .show_ui(ui, |ui| {
                                for (idx, opt) in sound_opts.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut state.rules[i].sound_idx,
                                        idx,
                                        opt.as_str(),
                                    );
                                }
                            });

                        // Browse button for custom file
                        if ui.button("...").clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .add_filter("Audio", &["wav", "mp3", "ogg", "flac"])
                                .pick_file()
                        {
                            state.rules[i].custom_sound = path.to_string_lossy().to_string();
                            state.rules[i].sound_idx = custom_idx;
                        }

                        // Play button
                        let has_sound = if state.rules[i].sound_idx == custom_idx {
                            !state.rules[i].custom_sound.is_empty()
                        } else {
                            state.rules[i].sound_idx > 0
                        };
                        if has_sound
                            && ui.button("\u{25B6}").clicked()
                        {
                            let name = if state.rules[i].sound_idx == custom_idx {
                                state.rules[i].custom_sound.clone()
                            } else {
                                sound_opts.get(state.rules[i].sound_idx)
                                    .cloned()
                                    .unwrap_or_default()
                            };
                            play_sound_name = Some(name);
                        }
                    }
                });
            });

        ui.add_space(4.0);
    }

    if let Some(idx) = to_remove {
        state.rules.remove(idx);
    }

    if let Some(name) = play_sound_name {
        sound::play_sound(&name);
    }

    if ui.button("+ Add Rule").clicked() {
        state.rules.push(RuleEntry {
            server_idx: 0,
            source: String::new(),
            status_idx: 0,
            pattern: String::new(),
            sound_idx: 0,
            custom_sound: String::new(),
            action_idx: 0,
        });
    }
}

fn apply_autostart_from_state(state: &SettingsState) {
    let currently_installed = autostart::is_autostart_installed();
    if state.autostart_enabled && !currently_installed
        && let Err(e) = autostart::install_autostart()
    {
        tracing::error!("Failed to install autostart: {}", e);
    } else if !state.autostart_enabled && currently_installed
        && let Err(e) = autostart::uninstall_autostart()
    {
        tracing::error!("Failed to uninstall autostart: {}", e);
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_config() -> Config {
        Config {
            name: "Test".to_string(),
            servers: vec![
                Server {
                    url: "ws://host1:9876/ws".to_string(),
                    secret: "secret1".to_string(),
                    label: "Work".to_string(),
                },
                Server {
                    url: "ws://host2:9876/ws".to_string(),
                    secret: "secret2".to_string(),
                    label: "Home".to_string(),
                },
            ],
            rules: RulesConfig {
                enabled: true,
                rules: vec![
                    NotificationRule {
                        server: "Work".to_string(),
                        source: "jenkins".to_string(),
                        status: "error".to_string(),
                        pattern: "deploy".to_string(),
                        sound: "Bell".to_string(),
                        action: RuleAction::Normal,
                        suppress: false,
                    },
                    NotificationRule {
                        server: String::new(),
                        source: String::new(),
                        status: String::new(),
                        pattern: String::new(),
                        sound: String::new(),
                        action: RuleAction::Dismiss,
                        suppress: false,
                    },
                ],
            },
            center_panel: CenterPanelConfig::default(),
            debug_font_metrics: true,
        }
    }

    #[test]
    fn test_settings_state_roundtrip() {
        let config = make_test_config();
        let status = HashMap::new();
        let state = SettingsState::from_config(&config, &status);
        let result = state.to_config();

        assert_eq!(result.name, "Test");
        assert_eq!(result.servers.len(), 2);
        assert_eq!(result.servers[0].label, "Work");
        assert_eq!(result.servers[0].url, "ws://host1:9876/ws");
        assert_eq!(result.servers[1].label, "Home");
        assert!(result.rules.enabled);
        assert_eq!(result.rules.rules.len(), 2);
        assert_eq!(result.rules.rules[0].server, "Work");
        assert_eq!(result.rules.rules[0].source, "jenkins");
        assert_eq!(result.rules.rules[0].status, "error");
        assert_eq!(result.rules.rules[0].pattern, "deploy");
        assert_eq!(result.rules.rules[0].sound, "Bell");
        assert_eq!(result.rules.rules[0].action, RuleAction::Normal);
        assert_eq!(result.rules.rules[1].action, RuleAction::Dismiss);
        assert!(result.debug_font_metrics);
    }

    #[test]
    fn test_settings_state_empty_config() {
        let config = Config::default();
        let status = HashMap::new();
        let state = SettingsState::from_config(&config, &status);

        assert!(state.name.is_empty());
        assert!(state.servers.is_empty());
        assert!(!state.rules_enabled);
        assert!(state.rules.is_empty());
    }

    #[test]
    fn test_server_entry_roundtrip() {
        let server = Server {
            url: "ws://test:9876/ws".to_string(),
            secret: "s3cret".to_string(),
            label: "Test".to_string(),
        };
        let config = Config {
            servers: vec![server],
            ..Default::default()
        };
        let status = HashMap::new();
        let state = SettingsState::from_config(&config, &status);
        let result = state.to_config();

        assert_eq!(result.servers.len(), 1);
        assert_eq!(result.servers[0].url, "ws://test:9876/ws");
        assert_eq!(result.servers[0].secret, "s3cret");
        assert_eq!(result.servers[0].label, "Test");
    }

    #[test]
    fn test_rule_entry_roundtrip() {
        let config = Config {
            servers: vec![Server {
                url: "ws://host:9876/ws".to_string(),
                secret: "s".to_string(),
                label: "Prod".to_string(),
            }],
            rules: RulesConfig {
                enabled: true,
                rules: vec![NotificationRule {
                    server: "Prod".to_string(),
                    source: "ci".to_string(),
                    status: "warning".to_string(),
                    pattern: "test.*fail".to_string(),
                    sound: "tone:alert".to_string(),
                    action: RuleAction::Silent,
                    suppress: false,
                }],
            },
            ..Default::default()
        };
        let status = HashMap::new();
        let state = SettingsState::from_config(&config, &status);

        assert_eq!(state.rules[0].server_idx, 1); // "Prod" is first server
        assert_eq!(state.rules[0].status_idx, 3); // "warning" is index 3
        assert_eq!(state.rules[0].action_idx, 1); // Silent

        let result = state.to_config();
        assert_eq!(result.rules.rules[0].server, "Prod");
        assert_eq!(result.rules.rules[0].status, "warning");
        assert_eq!(result.rules.rules[0].pattern, "test.*fail");
        assert_eq!(result.rules.rules[0].sound, "tone:alert");
        assert_eq!(result.rules.rules[0].action, RuleAction::Silent);
    }

    #[test]
    fn test_empty_servers_filtered() {
        let config = Config::default();
        let status = HashMap::new();
        let mut state = SettingsState::from_config(&config, &status);

        // Add an empty server entry (user clicked + Add Server but didn't fill it)
        state.servers.push(ServerEntry {
            label: String::new(),
            url: String::new(),
            secret: String::new(),
            connected: false,
        });

        let result = state.to_config();
        assert!(result.servers.is_empty()); // Empty entries should be filtered out
    }

    #[test]
    fn test_custom_sound_file_roundtrip() {
        let config = Config {
            rules: RulesConfig {
                enabled: true,
                rules: vec![NotificationRule {
                    server: String::new(),
                    source: String::new(),
                    status: String::new(),
                    pattern: String::new(),
                    sound: "/Users/test/sounds/alert.wav".to_string(),
                    action: RuleAction::Normal,
                    suppress: false,
                }],
            },
            ..Default::default()
        };
        let status = HashMap::new();
        let state = SettingsState::from_config(&config, &status);

        // Custom file should be stored in custom_sound, with sound_idx at custom marker
        assert_eq!(state.rules[0].sound_idx, custom_sound_idx());
        assert_eq!(state.rules[0].custom_sound, "/Users/test/sounds/alert.wav");

        let result = state.to_config();
        assert_eq!(result.rules.rules[0].sound, "/Users/test/sounds/alert.wav");
    }
}
