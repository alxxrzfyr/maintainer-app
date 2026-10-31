/// Application page enumeration.
///
/// Each variant maps to one top-level view in the navigation sidebar.
/// There is intentionally no Activity page in v1.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    #[default]
    Overview,
    System,
    Network,
    Processes,
    Storage,
    Cleanup,
    Updates,
    Diagnostics,
    Reports,
    Settings,
}

impl Page {
    /// All pages in navigation order.
    pub const ALL: &'static [Page] = &[
        Page::Overview,
        Page::System,
        Page::Network,
        Page::Processes,
        Page::Storage,
        Page::Cleanup,
        Page::Updates,
        Page::Diagnostics,
        Page::Reports,
        Page::Settings,
    ];

    /// Human-readable label for the page.
    pub fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::System => "System",
            Page::Network => "Network",
            Page::Processes => "Processes",
            Page::Storage => "Storage",
            Page::Cleanup => "Cleanup",
            Page::Updates => "Updates",
            Page::Diagnostics => "Diagnostics",
            Page::Reports => "Reports",
            Page::Settings => "Settings",
        }
    }

    /// Fallback icon chain for this page.
    ///
    /// Returns a chain of icon names for fallback resolution. The last entry
    /// is guaranteed to exist in all GTK4 installations.
    pub fn icon_chain(self) -> &'static [&'static str] {
        match self {
            Page::Overview => &[
                "go-home-symbolic",
                "user-home-symbolic",
                "emblem-default-symbolic",
            ],
            Page::System => &[
                "utilities-system-monitor-symbolic",
                "preferences-system-symbolic",
            ],
            Page::Network => &[
                "network-wired-symbolic",
                "network-idle-symbolic",
                "preferences-system-network-symbolic",
            ],
            Page::Processes => &[
                "system-run-symbolic",
                "system-processes-symbolic",
                "utilities-system-monitor-symbolic",
            ],
            Page::Storage => &["drive-harddisk-symbolic", "drive-multidisk-symbolic"],
            Page::Cleanup => &[
                "edit-clear-all-symbolic",
                "edit-clear-symbolic",
                "emblem-default-symbolic",
            ],
            Page::Updates => &[
                "software-update-available-symbolic",
                "emblem-synchronizing-symbolic",
            ],
            Page::Diagnostics => &[
                "preferences-system-details-symbolic",
                "dialog-warning-symbolic",
                "emblem-important-symbolic",
            ],
            Page::Reports => &[
                "x-office-document-symbolic",
                "document-open-symbolic",
                "emblem-documents-symbolic",
            ],
            Page::Settings => &["emblem-system-symbolic", "preferences-system-symbolic"],
        }
    }

    /// Resolved icon name using the fallback chain.
    ///
    /// Delegates to `icons::resolve_icon` which queries the active GTK icon
    /// theme. In headless / test contexts, returns the last chain entry.
    pub fn icon_name(self) -> &'static str {
        crate::ui::icons::resolve_icon(self.icon_chain())
    }
}

/// Typed user intents that drive state transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppIntent {
    /// Navigate to a specific page.
    Navigate(Page),
    /// Window visibility changed (e.g., hidden/shown).
    WindowVisibilityChanged(bool),
    /// Retry a capability by its identifier.
    RetryCapability(&'static str),
}

/// Presentation state of the application shell.
///
/// Pure data — no GTK types. Testable without a display server.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Currently active page.
    pub active_page: Page,
    /// Whether the window is currently visible.
    pub window_visible: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            active_page: Page::Overview,
            window_visible: true,
        }
    }
}

impl AppState {
    /// Apply an intent and update state.
    pub fn reduce(&mut self, intent: AppIntent) {
        match intent {
            AppIntent::Navigate(page) => {
                self.active_page = page;
            }
            AppIntent::WindowVisibilityChanged(visible) => {
                self.window_visible = visible;
            }
            AppIntent::RetryCapability(_id) => {}
        }
    }
}

/// Navigation group definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationGroup {
    /// Group label.
    pub label: &'static str,
    /// Pages in this group.
    pub pages: &'static [Page],
}

/// Navigation groups in sidebar order.
pub const NAVIGATION_GROUPS: &[NavigationGroup] = &[
    NavigationGroup {
        label: "General",
        pages: &[Page::Overview],
    },
    NavigationGroup {
        label: "Monitor",
        pages: &[Page::System, Page::Network, Page::Processes],
    },
    NavigationGroup {
        label: "Storage",
        pages: &[Page::Storage],
    },
    NavigationGroup {
        label: "Maintenance",
        pages: &[Page::Cleanup, Page::Updates],
    },
    NavigationGroup {
        label: "Diagnostics",
        pages: &[Page::Diagnostics, Page::Reports],
    },
];

/// Footer pages (displayed separately at bottom of sidebar).
pub const FOOTER_PAGES: &[Page] = &[Page::Settings];
