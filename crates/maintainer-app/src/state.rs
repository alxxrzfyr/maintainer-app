/// Application page enumeration.
///
/// Each variant maps to one top-level view in the navigation sidebar.
/// There is intentionally no Activity page in v1.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    #[default]
    Overview,
    System,
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
            Page::Storage => "Storage",
            Page::Cleanup => "Cleanup",
            Page::Updates => "Updates",
            Page::Diagnostics => "Diagnostics",
            Page::Reports => "Reports",
            Page::Settings => "Settings",
        }
    }

    /// Icon name from the system icon theme.
    pub fn icon_name(self) -> &'static str {
        match self {
            Page::Overview => "go-home-symbolic",
            Page::System => "utilities-system-monitor-symbolic",
            Page::Storage => "drive-harddisk-symbolic",
            Page::Cleanup => "edit-clear-all-symbolic",
            Page::Updates => "software-update-available-symbolic",
            Page::Diagnostics => "dialog-warning-symbolic",
            Page::Reports => "document-open-symbolic",
            Page::Settings => "preferences-system-symbolic",
        }
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
            AppIntent::RetryCapability(_id) => {
                // Retry intents are handled by the service layer;
                // state remains unchanged here.
            }
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
        pages: &[Page::System],
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
