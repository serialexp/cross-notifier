// System tray icon with context menu.
// Uses tray-icon + muda (re-exported via tray_icon::menu).
//
// On macOS, the tray icon must be created on the main thread after the event loop starts.
// On Linux, tray-icon requires a GTK event loop. Since winit doesn't use GTK, we spawn
// a dedicated GTK thread and communicate via channels + glib::idle_add_once.
//
// Theme handling: we ship two variants of each icon (idle and "has
// notifications") — black for light panels, white for dark panels. The
// active variant is picked from the user's manual override (`Auto` /
// `Light` / `Dark`), with `Auto` deferring to `crate::theme::detect()`.
// On macOS the choice always collapses to the black icon plus
// `with_icon_as_template(true)`; the OS inverts it as needed. On Linux
// the GTK idle handler periodically re-evaluates the variant so manual
// theme switches in the desktop propagate without a daemon restart.

use tracing::info;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;
use crate::config::TrayIconStyle;
use crate::theme;

// Icon byte arrays are embedded so the binary doesn't need to ship the
// PNGs alongside it. Path is relative to this file: bin/daemon/src/ →
// repo root is three levels up.
const TRAY_LIGHT: &[u8] = include_bytes!("../../../tray@2x.png");
const TRAY_NOTIFICATION_LIGHT: &[u8] = include_bytes!("../../../tray-notification@2x.png");
const TRAY_DARK: &[u8] = include_bytes!("../../../tray-dark@2x.png");
const TRAY_NOTIFICATION_DARK: &[u8] = include_bytes!("../../../tray-notification-dark@2x.png");

const MENU_NOTIFICATIONS: &str = "notifications";
const MENU_SETTINGS: &str = "settings";
const MENU_QUIT: &str = "quit";

fn load_icon(png_bytes: &[u8]) -> Icon {
    let img = image::load_from_memory(png_bytes)
        .expect("Failed to decode tray icon")
        .into_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).expect("Failed to create tray icon")
}

/// Resolve a manual override + detected theme into a concrete variant.
fn resolve_variant(style: TrayIconStyle) -> theme::Variant {
    match style {
        TrayIconStyle::Auto => theme::detect(),
        TrayIconStyle::Light => theme::Variant::Light,
        TrayIconStyle::Dark => theme::Variant::Dark,
    }
}

/// Pick the right embedded PNG bytes for the given (variant, has_notifications) pair.
fn icon_bytes(variant: theme::Variant, has_notifications: bool) -> &'static [u8] {
    match (variant, has_notifications) {
        (theme::Variant::Light, false) => TRAY_LIGHT,
        (theme::Variant::Light, true) => TRAY_NOTIFICATION_LIGHT,
        (theme::Variant::Dark, false) => TRAY_DARK,
        (theme::Variant::Dark, true) => TRAY_NOTIFICATION_DARK,
    }
}

fn build_menu() -> (Menu, MenuItem) {
    let menu = Menu::new();
    let notifications_item = MenuItem::with_id(MENU_NOTIFICATIONS, "Notifications", true, None);
    let separator1 = PredefinedMenuItem::separator();
    let settings_item = MenuItem::with_id(MENU_SETTINGS, "Settings...", true, None);
    let separator2 = PredefinedMenuItem::separator();
    let quit_item = MenuItem::with_id(MENU_QUIT, "Quit cross-notifier", true, None);

    menu.append(&notifications_item).unwrap();
    menu.append(&separator1).unwrap();
    menu.append(&settings_item).unwrap();
    menu.append(&separator2).unwrap();
    menu.append(&quit_item).unwrap();

    (menu, notifications_item)
}

// ---------------------------------------------------------------------------
// macOS / Windows: tray lives on the winit thread (original behaviour)
// ---------------------------------------------------------------------------
#[cfg(not(target_os = "linux"))]
mod platform {
    use super::*;
    use std::cell::Cell;
    use tray_icon::TrayIcon;

    pub struct TrayState {
        tray: TrayIcon,
        notifications_item: MenuItem,
        style: Cell<TrayIconStyle>,
        has_notifications: Cell<bool>,
        current_variant: Cell<theme::Variant>,
    }

    impl TrayState {
        pub fn new(style: TrayIconStyle) -> Self {
            let variant = resolve_variant(style);
            let icon = load_icon(icon_bytes(variant, false));
            let (menu, notifications_item) = build_menu();

            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("cross-notifier")
                .with_icon(icon)
                // macOS template inversion: lets the OS flip the black
                // icon to white when the menu bar is dark. Harmless on
                // Windows (the flag is ignored there).
                .with_icon_as_template(true)
                .build()
                .expect("Failed to create tray icon");

            info!(
                "System tray initialized (variant={:?}, style={:?})",
                variant, style
            );

            Self {
                tray,
                notifications_item,
                style: Cell::new(style),
                has_notifications: Cell::new(false),
                current_variant: Cell::new(variant),
            }
        }

        /// Re-render the icon from the current state. Idempotent.
        fn reapply(&self) {
            let variant = resolve_variant(self.style.get());
            self.current_variant.set(variant);
            let icon = load_icon(icon_bytes(variant, self.has_notifications.get()));
            let _ = self.tray.set_icon(Some(icon));
        }

        pub fn set_has_notifications(&self, has_notifications: bool) {
            if self.has_notifications.get() != has_notifications {
                self.has_notifications.set(has_notifications);
                self.reapply();
            }
        }

        pub fn set_notification_count(&self, count: usize) {
            let label = if count == 0 {
                "Notifications".to_string()
            } else {
                format!("Notifications ({})", count)
            };
            self.notifications_item.set_text(label);
        }

        pub fn set_theme_override(&self, style: TrayIconStyle) {
            if self.style.get() != style {
                self.style.set(style);
                theme::invalidate_cache();
                self.reapply();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Linux: tray lives on a dedicated GTK thread
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    enum TrayCommand {
        HasNotifications(bool),
        NotificationCount(usize),
        ThemeOverride(TrayIconStyle),
    }

    pub struct TrayState {
        tx: Arc<Mutex<mpsc::Sender<TrayCommand>>>,
    }

    impl TrayState {
        pub fn new(style: TrayIconStyle) -> Self {
            let (tx, rx) = mpsc::channel::<TrayCommand>();
            let tx = Arc::new(Mutex::new(tx));

            // std channel to wait for the GTK thread to finish init.
            let (ready_tx, ready_rx) = mpsc::channel();

            std::thread::Builder::new()
                .name("gtk-tray".into())
                .spawn(move || {
                    gtk::init().expect("Failed to initialize GTK for tray icon");

                    let initial_variant = resolve_variant(style);
                    let icon = load_icon(icon_bytes(initial_variant, false));
                    let (menu, notifications_item) = build_menu();

                    let tray = TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_tooltip("cross-notifier")
                        .with_icon(icon)
                        .build()
                        .expect("Failed to create tray icon");

                    info!(
                        "System tray initialized (GTK thread, variant={:?}, style={:?})",
                        initial_variant, style
                    );
                    let _ = ready_tx.send(());

                    // All mutable state lives in this single FnMut closure
                    // (single-threaded GTK main loop), so primitive Cells
                    // would be sufficient — but keeping plain locals is
                    // simpler since the closure already captures `mut`.
                    let mut current_style = style;
                    let mut has_notifications = false;
                    let mut current_variant = initial_variant;
                    // Re-probe the desktop theme at most every CACHE_TTL +
                    // a small safety margin. theme::detect() caches
                    // internally too, so calling it here is cheap; we
                    // additionally throttle so we don't even bother
                    // calling it more than ~1Hz.
                    let mut last_auto_check = Instant::now();
                    const AUTO_RECHECK: Duration = Duration::from_millis(1500);

                    let reapply =
                        |variant: theme::Variant, has: bool, tray: &tray_icon::TrayIcon| {
                            let icon = load_icon(icon_bytes(variant, has));
                            let _ = tray.set_icon(Some(icon));
                        };

                    glib::idle_add_local(move || {
                        // Drain pending commands first so a settings-save
                        // takes effect on this tick.
                        while let Ok(cmd) = rx.try_recv() {
                            match cmd {
                                TrayCommand::HasNotifications(has) => {
                                    if has != has_notifications {
                                        has_notifications = has;
                                        reapply(current_variant, has_notifications, &tray);
                                    }
                                }
                                TrayCommand::NotificationCount(count) => {
                                    let label = if count == 0 {
                                        "Notifications".to_string()
                                    } else {
                                        format!("Notifications ({})", count)
                                    };
                                    notifications_item.set_text(label);
                                }
                                TrayCommand::ThemeOverride(style) => {
                                    if style != current_style {
                                        current_style = style;
                                        theme::invalidate_cache();
                                        let new_variant = resolve_variant(current_style);
                                        if new_variant != current_variant {
                                            current_variant = new_variant;
                                            reapply(current_variant, has_notifications, &tray);
                                        }
                                    }
                                }
                            }
                        }

                        // Periodic re-detection: only meaningful when the
                        // user hasn't pinned a manual variant. We pick up
                        // a runtime theme switch within ~1.5s.
                        if matches!(current_style, TrayIconStyle::Auto)
                            && last_auto_check.elapsed() >= AUTO_RECHECK
                        {
                            last_auto_check = Instant::now();
                            let detected = theme::detect();
                            if detected != current_variant {
                                current_variant = detected;
                                reapply(current_variant, has_notifications, &tray);
                            }
                        }

                        glib::ControlFlow::Continue
                    });

                    gtk::main();
                })
                .expect("Failed to spawn GTK tray thread");

            // Wait for GTK init to complete before returning
            ready_rx
                .recv()
                .expect("GTK tray thread failed to initialize");

            Self { tx }
        }

        pub fn set_has_notifications(&self, has_notifications: bool) {
            let _ = self
                .tx
                .lock()
                .unwrap()
                .send(TrayCommand::HasNotifications(has_notifications));
        }

        pub fn set_notification_count(&self, count: usize) {
            let _ = self
                .tx
                .lock()
                .unwrap()
                .send(TrayCommand::NotificationCount(count));
        }

        pub fn set_theme_override(&self, style: TrayIconStyle) {
            let _ = self
                .tx
                .lock()
                .unwrap()
                .send(TrayCommand::ThemeOverride(style));
        }
    }
}

pub use platform::TrayState;

/// Set up global menu event handler that dispatches to AppEvent via EventLoopProxy.
/// Must be called before the event loop starts.
pub fn setup_event_handlers(proxy: &EventLoopProxy<AppEvent>) {
    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.0.as_str() {
        MENU_NOTIFICATIONS => {
            let _ = menu_proxy.send_event(AppEvent::ToggleCenter);
        }
        MENU_SETTINGS => {
            let _ = menu_proxy.send_event(AppEvent::OpenSettings);
        }
        MENU_QUIT => {
            let _ = menu_proxy.send_event(AppEvent::Quit);
        }
        _ => {}
    }));
}
