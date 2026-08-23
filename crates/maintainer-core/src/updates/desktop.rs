//! Desktop environment detection and package categorization.
//!
//! Detects active desktop environments (XFCE, Linux Mint / Cinnamon, MATE, GNOME, KDE)
//! and categorizes packages so updates are clearly tagged for the user's specific
//! system environment.

/// Supported or recognized desktop environments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DesktopEnvironment {
    Xfce,
    Mint,
    Mate,
    Gnome,
    Kde,
    Generic,
}

impl DesktopEnvironment {
    /// Detect the active desktop environment from session environment variables
    /// and distribution files.
    pub fn detect() -> Self {
        let xdg_current = std::env::var("XDG_CURRENT_DESKTOP").ok();
        let desktop_session = std::env::var("DESKTOP_SESSION").ok();
        let os_release = std::fs::read_to_string("/etc/os-release").ok();

        Self::from_env_and_os(
            xdg_current.as_deref(),
            desktop_session.as_deref(),
            os_release.as_deref(),
        )
    }

    /// Pure parser for desktop environment from strings.
    pub fn from_env_and_os(
        xdg_current: Option<&str>,
        desktop_session: Option<&str>,
        os_release: Option<&str>,
    ) -> Self {
        let xdg = xdg_current.unwrap_or("").to_ascii_lowercase();
        let session = desktop_session.unwrap_or("").to_ascii_lowercase();
        let os = os_release.unwrap_or("").to_ascii_lowercase();

        if xdg.contains("xfce") || session.contains("xfce") || session.contains("xubuntu") {
            Self::Xfce
        } else if xdg.contains("cinnamon")
            || session.contains("cinnamon")
            || os.contains("linuxmint")
            || os.contains("mint")
        {
            Self::Mint
        } else if xdg.contains("mate") || session.contains("mate") {
            Self::Mate
        } else if xdg.contains("gnome") || session.contains("gnome") || session.contains("ubuntu") {
            Self::Gnome
        } else if xdg.contains("kde") || session.contains("plasma") || session.contains("kde") {
            Self::Kde
        } else {
            Self::Generic
        }
    }

    /// User-friendly label for this desktop environment.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Xfce => "XFCE Desktop",
            Self::Mint => "Linux Mint",
            Self::Mate => "MATE Desktop",
            Self::Gnome => "GNOME Desktop",
            Self::Kde => "KDE Plasma",
            Self::Generic => "Desktop",
        }
    }
}

/// Category of an update package.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PackageCategory {
    /// Associated with the user's desktop environment (e.g. XFCE, Mint).
    Desktop(String),
    /// Linux kernel image, headers, or modules.
    Kernel,
    /// Hardware drivers, microcode, or firmware.
    Driver,
    /// End-user desktop application.
    Application,
    /// Core system library or service.
    System,
}

impl PackageCategory {
    /// User-facing display label for the category badge.
    pub fn badge_label(&self) -> &str {
        match self {
            Self::Desktop(label) => label.as_str(),
            Self::Kernel => "Kernel",
            Self::Driver => "Hardware / Driver",
            Self::Application => "Application",
            Self::System => "System Core",
        }
    }

    /// CSS class suffix for badge styling.
    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Desktop(_) => "maintainer_badge_desktop",
            Self::Kernel => "maintainer_badge_kernel",
            Self::Driver => "maintainer_badge_driver",
            Self::Application => "maintainer_badge_app",
            Self::System => "maintainer_badge_system",
        }
    }
}

/// Classify an update package into a category based on its name and active desktop environment.
pub fn classify_package(name: &str, desktop_env: DesktopEnvironment) -> PackageCategory {
    let lower = name.to_ascii_lowercase();

    // 1. Kernel packages
    if lower.starts_with("linux-image-")
        || lower.starts_with("linux-headers-")
        || lower.starts_with("linux-modules-")
        || lower == "linux-generic"
        || lower == "linux-image-generic"
        || lower == "linux-headers-generic"
        || lower == "linux-modules-generic"
        || lower.starts_with("linux-hwe-")
    {
        return PackageCategory::Kernel;
    }

    // 2. Hardware drivers and firmware
    if lower.starts_with("linux-firmware")
        || lower.contains("microcode")
        || lower.starts_with("nvidia-")
        || lower.starts_with("mesa")
        || lower.starts_with("xserver-xorg-video-")
        || lower.starts_with("firmware-")
        || lower.starts_with("alsa-")
        || lower.starts_with("pulseaudio")
        || lower.starts_with("pipewire")
    {
        return PackageCategory::Driver;
    }

    // 3. Desktop environment specific packages
    if is_xfce_package(&lower) {
        return PackageCategory::Desktop("XFCE Desktop".to_string());
    }
    if is_mint_package(&lower) {
        return PackageCategory::Desktop("Linux Mint".to_string());
    }
    if is_mate_package(&lower) {
        return PackageCategory::Desktop("MATE Desktop".to_string());
    }
    if is_gnome_package(&lower) {
        return PackageCategory::Desktop("GNOME Desktop".to_string());
    }
    if is_kde_package(&lower) {
        return PackageCategory::Desktop("KDE Plasma".to_string());
    }

    // 4. User applications
    if is_application_package(&lower) {
        return PackageCategory::Application;
    }

    // Default to system core
    match desktop_env {
        DesktopEnvironment::Generic => PackageCategory::System,
        _ => PackageCategory::System,
    }
}

fn is_xfce_package(name: &str) -> bool {
    name.starts_with("xfce4-")
        || name.starts_with("xfwm4")
        || name.starts_with("thunar")
        || name.starts_with("tumbler")
        || name.starts_with("xfdesktop")
        || name.starts_with("mousepad")
        || name.starts_with("ristretto")
        || name.starts_with("parole")
        || name.starts_with("xarchiver")
        || name.starts_with("libxfce4")
        || name.starts_with("lightdm")
        || name.starts_with("exo-")
        || name.starts_with("garcon")
        || name == "xfconf"
        || name == "xfce4"
}

fn is_mint_package(name: &str) -> bool {
    name.starts_with("mint")
        || name.starts_with("cinnamon")
        || name.starts_with("nemo")
        || name.starts_with("muffin")
        || name.starts_with("warpinator")
        || name.starts_with("hypnotix")
        || name.starts_with("thingy")
        || name.starts_with("pix")
        || name.starts_with("xviewer")
        || name.starts_with("xreader")
        || name.starts_with("xplayer")
        || name.starts_with("xed")
}

fn is_mate_package(name: &str) -> bool {
    name.starts_with("mate-")
        || name.starts_with("caja")
        || name.starts_with("marco")
        || name.starts_with("pluma")
        || name.starts_with("atril")
        || name.starts_with("engrampa")
        || name == "mate-desktop"
}

fn is_gnome_package(name: &str) -> bool {
    name.starts_with("gnome-")
        || name.starts_with("mutter")
        || name.starts_with("nautilus")
        || name.starts_with("gdm3")
        || name.starts_with("evince")
        || name.starts_with("eog")
        || name.starts_with("totem")
}

fn is_kde_package(name: &str) -> bool {
    name.starts_with("plasma-")
        || name.starts_with("kwin")
        || name.starts_with("dolphin")
        || name.starts_with("kde-")
        || name.starts_with("konsole")
        || name.starts_with("kate")
}

fn is_application_package(name: &str) -> bool {
    const APPS: &[&str] = &[
        "firefox",
        "thunderbird",
        "chromium",
        "gimp",
        "vlc",
        "libreoffice",
        "inkscape",
        "blender",
        "obs-studio",
        "audacity",
        "transmission",
        "mpv",
        "neovim",
        "vim",
        "gedit",
        "code",
        "discord",
        "slack",
        "spotify",
        "steam",
        "telegram-desktop",
        "filezilla",
        "curl",
        "wget",
        "htop",
        "btop",
    ];

    APPS.iter().any(|app| {
        name == *app || name.starts_with(&format!("{app}-")) || name.starts_with(&format!("{app}_"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_xfce_desktop() {
        let env = DesktopEnvironment::from_env_and_os(
            Some("XFCE"),
            Some("xfce"),
            Some("NAME=\"Ubuntu\"\nVERSION=\"24.04\""),
        );
        assert_eq!(env, DesktopEnvironment::Xfce);
        assert_eq!(env.label(), "XFCE Desktop");
    }

    #[test]
    fn detect_mint_desktop() {
        let env = DesktopEnvironment::from_env_and_os(
            Some("X-Cinnamon"),
            Some("cinnamon"),
            Some("NAME=\"Linux Mint\"\nVERSION=\"22 (Wilma)\""),
        );
        assert_eq!(env, DesktopEnvironment::Mint);
        assert_eq!(env.label(), "Linux Mint");
    }

    #[test]
    fn classify_xfce_packages() {
        let cat = classify_package("thunar", DesktopEnvironment::Xfce);
        assert_eq!(cat, PackageCategory::Desktop("XFCE Desktop".to_string()));

        let cat2 = classify_package("xfce4-panel", DesktopEnvironment::Xfce);
        assert_eq!(cat2, PackageCategory::Desktop("XFCE Desktop".to_string()));

        let cat3 = classify_package("mousepad", DesktopEnvironment::Xfce);
        assert_eq!(cat3, PackageCategory::Desktop("XFCE Desktop".to_string()));
    }

    #[test]
    fn classify_mint_packages() {
        let cat = classify_package("mintupdate", DesktopEnvironment::Mint);
        assert_eq!(cat, PackageCategory::Desktop("Linux Mint".to_string()));

        let cat2 = classify_package("nemo", DesktopEnvironment::Mint);
        assert_eq!(cat2, PackageCategory::Desktop("Linux Mint".to_string()));
    }

    #[test]
    fn classify_kernel_packages() {
        let cat = classify_package("linux-image-6.8.0-40-generic", DesktopEnvironment::Xfce);
        assert_eq!(cat, PackageCategory::Kernel);
    }

    #[test]
    fn classify_driver_packages() {
        let cat = classify_package("nvidia-driver-535", DesktopEnvironment::Xfce);
        assert_eq!(cat, PackageCategory::Driver);

        let cat2 = classify_package("intel-microcode", DesktopEnvironment::Xfce);
        assert_eq!(cat2, PackageCategory::Driver);
    }

    #[test]
    fn classify_application_packages() {
        let cat = classify_package("firefox", DesktopEnvironment::Xfce);
        assert_eq!(cat, PackageCategory::Application);

        let cat2 = classify_package("vlc", DesktopEnvironment::Xfce);
        assert_eq!(cat2, PackageCategory::Application);
    }

    #[test]
    fn classify_system_packages() {
        let cat = classify_package("libc6", DesktopEnvironment::Xfce);
        assert_eq!(cat, PackageCategory::System);

        let cat2 = classify_package("systemd", DesktopEnvironment::Xfce);
        assert_eq!(cat2, PackageCategory::System);
    }
}
