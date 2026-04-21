// System tray icon with context menu.
// Uses tray-icon + muda (re-exported via tray_icon::menu).
//
// On macOS, the tray icon must be created on the main thread after the event loop starts.
// On Linux, tray-icon requires a GTK event loop. Since winit doesn't use GTK, we spawn
// a dedicated GTK thread and communicate via channels + glib::idle_add_once.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};
use tracing::info;
use winit::event_loop::EventLoopProxy;

use crate::app::AppEvent;

const TRAY_PNG: &[u8] = include_bytes!("../../tray@2x.png");
const TRAY_NOTIFICATION_PNG: &[u8] = include_bytes!("../../tray-notification@2x.png");

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

fn build_menu() -> (Menu, MenuItem) {
    let menu = Menu::new();
    let notifications_item =
        MenuItem::with_id(MENU_NOTIFICATIONS, "Notifications", true, None);
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
    use tray_icon::TrayIcon;

    pub struct TrayState {
        tray: TrayIcon,
        notifications_item: MenuItem,
    }

    impl TrayState {
        pub fn new() -> Self {
            let icon = load_icon(TRAY_PNG);
            let (menu, notifications_item) = build_menu();

            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("cross-notifier")
                .with_icon(icon)
                .with_icon_as_template(true)
                .build()
                .expect("Failed to create tray icon");

            info!("System tray initialized");

            Self {
                tray,
                notifications_item,
            }
        }

        pub fn set_has_notifications(&self, has_notifications: bool) {
            let icon = if has_notifications {
                load_icon(TRAY_NOTIFICATION_PNG)
            } else {
                load_icon(TRAY_PNG)
            };
            let _ = self.tray.set_icon(Some(icon));
        }

        pub fn set_notification_count(&self, count: usize) {
            let label = if count == 0 {
                "Notifications".to_string()
            } else {
                format!("Notifications ({})", count)
            };
            self.notifications_item.set_text(label);
        }
    }
}

// ---------------------------------------------------------------------------
// Linux: tray lives on a dedicated GTK thread
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::sync::{mpsc, Arc, Mutex};

    enum TrayCommand {
        SetHasNotifications(bool),
        SetNotificationCount(usize),
    }

    pub struct TrayState {
        tx: Arc<Mutex<mpsc::Sender<TrayCommand>>>,
    }

    impl TrayState {
        pub fn new() -> Self {
            let (tx, rx) = mpsc::channel::<TrayCommand>();
            let tx = Arc::new(Mutex::new(tx));

            // std channel to wait for the GTK thread to finish init.
            let (ready_tx, ready_rx) = mpsc::channel();

            std::thread::Builder::new()
                .name("gtk-tray".into())
                .spawn(move || {
                    gtk::init().expect("Failed to initialize GTK for tray icon");

                    let icon = load_icon(TRAY_PNG);
                    let (menu, notifications_item) = build_menu();

                    let tray = TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_tooltip("cross-notifier")
                        .with_icon(icon)
                        .build()
                        .expect("Failed to create tray icon");

                    info!("System tray initialized (GTK thread)");
                    let _ = ready_tx.send(());

                    // Poll the channel from the GTK main loop via idle_add
                    glib::idle_add_local(move || {
                        while let Ok(cmd) = rx.try_recv() {
                            match cmd {
                                TrayCommand::SetHasNotifications(has) => {
                                    let icon = if has {
                                        load_icon(TRAY_NOTIFICATION_PNG)
                                    } else {
                                        load_icon(TRAY_PNG)
                                    };
                                    let _ = tray.set_icon(Some(icon));
                                }
                                TrayCommand::SetNotificationCount(count) => {
                                    let label = if count == 0 {
                                        "Notifications".to_string()
                                    } else {
                                        format!("Notifications ({})", count)
                                    };
                                    notifications_item.set_text(label);
                                }
                            }
                        }
                        glib::ControlFlow::Continue
                    });

                    gtk::main();
                })
                .expect("Failed to spawn GTK tray thread");

            // Wait for GTK init to complete before returning
            ready_rx.recv().expect("GTK tray thread failed to initialize");

            Self { tx }
        }

        pub fn set_has_notifications(&self, has_notifications: bool) {
            let _ = self.tx.lock().unwrap().send(TrayCommand::SetHasNotifications(has_notifications));
        }

        pub fn set_notification_count(&self, count: usize) {
            let _ = self.tx.lock().unwrap().send(TrayCommand::SetNotificationCount(count));
        }
    }
}

pub use platform::TrayState;

/// Set up global menu event handler that dispatches to AppEvent via EventLoopProxy.
/// Must be called before the event loop starts.
pub fn setup_event_handlers(proxy: &EventLoopProxy<AppEvent>) {
    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        match event.id.0.as_str() {
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
        }
    }));
}
