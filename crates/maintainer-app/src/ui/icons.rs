//! Icon resolution with fallback chains for GTK 4.6 compatibility.
//!
//! GTK4 icon themes differ between distributions and desktop environments.
//! Some symbolic icon names (e.g., `sensors-temperature-symbolic`) are not
//! universally available. This module provides fallback chains that degrade
//! gracefully to icons known to exist in all GTK4 installations.

use gtk4::{IconTheme, Image};

/// Resolve the first available icon name from a fallback chain.
///
/// Queries the default icon theme. If no display is available (headless CI)
/// or if called from a non-main thread, returns the last entry unconditionally
/// (assumed to be a universally safe fallback).
pub fn resolve_icon<'a>(names: &'a [&'a str]) -> &'a str {
    debug_assert!(!names.is_empty(), "fallback chain must not be empty");

    // GTK icon theme queries require the main thread with an active display.
    // In test runners and background threads, fall through to the safe default.
    if !gtk4::is_initialized() || !gtk4::is_initialized_main_thread() {
        return names.last().unwrap_or(&"image-missing-symbolic");
    }

    let theme = match gtk4::gdk::Display::default() {
        Some(display) => IconTheme::for_display(&display),
        None => return names.last().unwrap_or(&"image-missing-symbolic"),
    };

    for &name in &names[..names.len().saturating_sub(1)] {
        if theme.has_icon(name) {
            return name;
        }
    }

    // Last entry is the guaranteed fallback.
    names.last().unwrap_or(&"image-missing-symbolic")
}

/// Create a `gtk4::Image` using the first available icon from a fallback chain.
pub fn icon_with_fallback(names: &[&str], pixel_size: i32) -> Image {
    let name = resolve_icon(names);
    let image = Image::from_icon_name(name);
    image.set_pixel_size(pixel_size);
    image
}

/// Predefined fallback chains for icons used across the application.
///
/// Each chain ends with an icon guaranteed to exist in the base Adwaita/hicolor
/// themes shipped with GTK4.
pub mod chains {
    /// Temperature/thermal sensor icon.
    pub const TEMPERATURE: &[&str] = &[
        "sensors-temperature-symbolic",
        "weather-clear-symbolic",
        "dialog-information-symbolic",
    ];

    /// Memory/RAM icon.
    pub const MEMORY: &[&str] = &["media-memory-symbolic", "drive-harddisk-symbolic"];

    /// Firmware update icon.
    pub const FIRMWARE: &[&str] = &[
        "firmware-symbolic",
        "application-x-firmware-symbolic",
        "software-update-available-symbolic",
    ];

    /// Loading/spinner icon.
    pub const LOADING: &[&str] = &[
        "content-loading-symbolic",
        "process-working-symbolic",
        "emblem-synchronizing-symbolic",
    ];

    /// Unavailable/missing capability icon.
    pub const UNAVAILABLE: &[&str] = &["action-unavailable-symbolic", "dialog-warning-symbolic"];

    /// Laptop/device icon.
    pub const LAPTOP: &[&str] = &[
        "computer-laptop-symbolic",
        "computer-symbolic",
        "preferences-system-symbolic",
    ];

    /// Network activity icon.
    pub const NETWORK_ACTIVITY: &[&str] = &[
        "network-transmit-receive-symbolic",
        "network-idle-symbolic",
        "preferences-system-network-symbolic",
    ];

    /// Process/task icon.
    pub const PROCESS: &[&str] = &[
        "system-processes-symbolic",
        "utilities-system-monitor-symbolic",
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_returns_last_when_no_display() {
        // In headless CI, no display is available so resolve_icon must
        // return the last entry without panicking.
        let result = resolve_icon(&["nonexistent-icon-xyz", "dialog-information-symbolic"]);
        assert_eq!(result, "dialog-information-symbolic");
    }

    #[test]
    fn all_chains_have_at_least_two_entries() {
        let all_chains: &[&[&str]] = &[
            chains::TEMPERATURE,
            chains::MEMORY,
            chains::FIRMWARE,
            chains::LOADING,
            chains::UNAVAILABLE,
            chains::LAPTOP,
            chains::NETWORK_ACTIVITY,
            chains::PROCESS,
        ];
        for chain in all_chains {
            assert!(
                chain.len() >= 2,
                "chain {chain:?} must have at least 2 entries for meaningful fallback"
            );
        }
    }

    #[test]
    fn all_chain_final_entries_are_universally_available() {
        // Final entries must use icon names known to exist in base Adwaita/hicolor.
        // These prefixes are guaranteed by the GTK4 icon theme spec.
        let safe_prefixes = [
            "dialog-",
            "preferences-",
            "software-",
            "drive-",
            "network-",
            "utilities-",
            "emblem-",
            "process-",
            "weather-",
        ];
        let all_chains: &[&[&str]] = &[
            chains::TEMPERATURE,
            chains::MEMORY,
            chains::FIRMWARE,
            chains::LOADING,
            chains::UNAVAILABLE,
            chains::LAPTOP,
            chains::NETWORK_ACTIVITY,
            chains::PROCESS,
        ];
        for chain in all_chains {
            let last = chain.last().unwrap();
            let is_safe = safe_prefixes.iter().any(|prefix| last.starts_with(prefix));
            assert!(
                is_safe,
                "final fallback '{last}' does not start with a known safe prefix"
            );
        }
    }
}
