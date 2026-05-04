//! Detect the user's desktop color-scheme preference.
//!
//! Used to pick between the black "for-light-panels" and white
//! "for-dark-panels" tray icon variants. On macOS we never need to detect
//! — the OS template-inverts a single black icon based on the menu bar's
//! current colour, so we always ship the black variant and let the system
//! flip it. On Linux/Windows we walk a chain of probes:
//!
//!   1. XDG portal `org.freedesktop.appearance/color-scheme` (works on
//!      most modern desktops, including KDE Plasma 6, GNOME 42+, and
//!      anything else honouring the `xdg-desktop-portal` standard).
//!   2. KDE: `kdeglobals` `[General] ColorScheme` via `kreadconfig6` /
//!      `kreadconfig5` — older Plasma installs or systems where the
//!      portal isn't running.
//!   3. GNOME: `gsettings` `org.gnome.desktop.interface color-scheme`,
//!      then the `gtk-theme` name as a final fallback for pre-42 GNOME.
//!   4. Default to `Dark` — most desktops ship a dark panel and the white
//!      icon is the more legible default. KDE users with a dark global
//!      theme but a deliberately light panel are exactly why the manual
//!      override exists.
//!
//! Results are cached for `CACHE_TTL` so the tray's idle tick can call
//! [`detect`] freely without spawning subprocesses on every iteration.
//! A manual theme switch propagates within roughly one cache window.
//!
//! Probes that produce no output (binary missing, command failed, empty
//! result) silently fall through to the next probe. We never panic — a
//! broken DE setup just lands on the default.

use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Which icon pair the tray should display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Black icon — drawn on a light panel.
    Light,
    /// White icon — drawn on a dark panel.
    Dark,
}

/// How long a detection result is reused before we re-probe. Two seconds
/// is short enough that a manual desktop theme switch propagates within a
/// tray tick, long enough to not spam subprocesses.
const CACHE_TTL: Duration = Duration::from_secs(2);

static CACHE: Mutex<Option<(Variant, Instant)>> = Mutex::new(None);

/// Detect the appropriate tray icon variant for the current desktop.
///
/// On macOS this always returns [`Variant::Light`] — the menu bar applies
/// its own template inversion to the black icon, and shipping a
/// pre-inverted dark icon would double-flip it.
pub fn detect() -> Variant {
    if cfg!(target_os = "macos") {
        return Variant::Light;
    }

    if let Ok(mut cache) = CACHE.lock() {
        if let Some((variant, when)) = *cache
            && when.elapsed() < CACHE_TTL
        {
            return variant;
        }
        let detected = detect_uncached();
        *cache = Some((detected, Instant::now()));
        detected
    } else {
        // Lock poisoned — fall back to a fresh probe rather than panic.
        detect_uncached()
    }
}

/// Force-clear the cached value so the next [`detect`] call re-probes.
/// Used when the user changes the manual override at runtime — the
/// override doesn't depend on the cache, but the picker calls `detect()`
/// regardless, and a fresh result is cheap insurance.
pub fn invalidate_cache() {
    if let Ok(mut cache) = CACHE.lock() {
        *cache = None;
    }
}

fn detect_uncached() -> Variant {
    if let Some(v) = detect_xdg_portal() {
        return v;
    }
    if let Some(v) = detect_kde() {
        return v;
    }
    if let Some(v) = detect_gnome() {
        return v;
    }
    Variant::Dark
}

/// Run a command, capture stdout, return `None` on any failure.
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Query the XDG desktop portal for the user's color-scheme preference.
///
/// `dbus-send --print-reply` returns multi-line text that ends with the
/// variant value:
///
/// ```text
/// method return time=...
///    variant       variant          uint32 1
/// ```
///
/// The trailing token is the integer we want: 0 = no preference,
/// 1 = prefer dark, 2 = prefer light.
fn detect_xdg_portal() -> Option<Variant> {
    let raw = run(
        "dbus-send",
        &[
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings.Read",
            "string:org.freedesktop.appearance",
            "string:color-scheme",
        ],
    )?;
    let token = raw.split_whitespace().next_back()?;
    match token {
        "1" => Some(Variant::Dark),
        "2" => Some(Variant::Light),
        // 0 = "no preference" — the portal can't help us, fall through.
        _ => None,
    }
}

/// Read KDE's chosen ColorScheme directly from `kdeglobals`. The convention
/// is that any scheme containing "dark" in its name is a dark theme;
/// everything else (Breeze, BreezeLight, custom palettes, ...) is light.
fn detect_kde() -> Option<Variant> {
    for bin in ["kreadconfig6", "kreadconfig5"] {
        if let Some(out) = run(
            bin,
            &[
                "--file",
                "kdeglobals",
                "--group",
                "General",
                "--key",
                "ColorScheme",
            ],
        ) {
            let lower = out.trim().to_lowercase();
            if lower.is_empty() {
                continue;
            }
            return Some(if lower.contains("dark") {
                Variant::Dark
            } else {
                Variant::Light
            });
        }
    }
    None
}

/// Query GNOME's interface settings. Modern GNOME (42+) exposes an
/// explicit `color-scheme` key (`prefer-dark`, `prefer-light`, `default`).
/// Older GNOME only has `gtk-theme` whose name typically encodes light/
/// dark — we pattern-match on "dark" as a last resort.
fn detect_gnome() -> Option<Variant> {
    if let Some(out) = run(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "color-scheme"],
    ) {
        let lower = out.trim().trim_matches('\'').to_lowercase();
        if lower.contains("dark") {
            return Some(Variant::Dark);
        }
        if lower.contains("light") {
            return Some(Variant::Light);
        }
        // "default" or unrecognised → don't conclude here, let the
        // gtk-theme fallback try.
    }

    let raw = run(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "gtk-theme"],
    )?;
    let lower = raw.trim().trim_matches('\'').to_lowercase();
    if lower.is_empty() {
        return None;
    }
    Some(if lower.contains("dark") {
        Variant::Dark
    } else {
        Variant::Light
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On macOS the function must short-circuit to Light regardless of
    /// what's in the cache or environment, because the OS handles
    /// inversion via the template flag.
    #[test]
    #[cfg(target_os = "macos")]
    fn macos_always_light() {
        assert_eq!(detect(), Variant::Light);
    }

    /// `detect()` must produce *some* result on every platform — even on
    /// a stripped-down container with no dbus/gsettings, the default
    /// fallback kicks in.
    #[test]
    fn detect_returns_a_variant() {
        let _ = detect();
    }

    #[test]
    fn cache_invalidation_does_not_panic() {
        invalidate_cache();
        let _ = detect();
        invalidate_cache();
    }
}
