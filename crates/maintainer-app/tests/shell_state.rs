//! Tests for application shell state (pure data, no GTK required).

use maintainer_app::state::{AppIntent, AppState, FOOTER_PAGES, NAVIGATION_GROUPS, Page};

#[test]
fn default_state_starts_on_overview() {
    let state = AppState::default();
    assert_eq!(state.active_page, Page::Overview);
    assert!(state.window_visible);
}

#[test]
fn navigation_selects_requested_page() {
    let mut state = AppState::default();
    state.reduce(AppIntent::Navigate(Page::Storage));
    assert_eq!(state.active_page, Page::Storage);
}

#[test]
fn navigation_to_all_pages() {
    let mut state = AppState::default();
    for &page in Page::ALL {
        state.reduce(AppIntent::Navigate(page));
        assert_eq!(state.active_page, page);
    }
}

#[test]
fn no_activity_page_variant_exists() {
    // Verify the Page enum has exactly the v1 pages and no Activity.
    let expected = [
        Page::Overview,
        Page::System,
        Page::Storage,
        Page::Cleanup,
        Page::Updates,
        Page::Diagnostics,
        Page::Reports,
        Page::Settings,
    ];
    assert_eq!(Page::ALL, &expected);
}

#[test]
fn window_visibility_updates_state() {
    let mut state = AppState::default();
    state.reduce(AppIntent::WindowVisibilityChanged(false));
    assert!(!state.window_visible);
    state.reduce(AppIntent::WindowVisibilityChanged(true));
    assert!(state.window_visible);
}

#[test]
fn retry_capability_does_not_change_active_page() {
    let mut state = AppState::default();
    state.reduce(AppIntent::Navigate(Page::System));
    state.reduce(AppIntent::RetryCapability("temperature"));
    assert_eq!(state.active_page, Page::System);
}

#[test]
fn navigation_groups_cover_all_non_footer_pages() {
    let mut group_pages: Vec<Page> = NAVIGATION_GROUPS
        .iter()
        .flat_map(|g| g.pages.iter().copied())
        .collect();
    group_pages.extend_from_slice(FOOTER_PAGES);
    group_pages.sort_by_key(|p| *p as u8);

    let mut all_pages: Vec<Page> = Page::ALL.to_vec();
    all_pages.sort_by_key(|p| *p as u8);

    assert_eq!(group_pages, all_pages);
}

#[test]
fn page_labels_are_nonempty() {
    for &page in Page::ALL {
        assert!(!page.label().is_empty(), "{page:?} has empty label");
    }
}

#[test]
fn page_icon_names_are_nonempty() {
    for &page in Page::ALL {
        assert!(!page.icon_name().is_empty(), "{page:?} has empty icon name");
    }
}

#[test]
fn settings_is_in_footer() {
    assert!(FOOTER_PAGES.contains(&Page::Settings));
}

#[test]
fn drawer_mode_navigation_covers_all_pages() {
    // Verify that navigation groups + footer pages provide access to every page,
    // ensuring the drawer menu (which uses the same source) does not miss any page.
    let drawer_pages: Vec<Page> = NAVIGATION_GROUPS
        .iter()
        .flat_map(|g| g.pages.iter().copied())
        .chain(FOOTER_PAGES.iter().copied())
        .collect();

    for &page in Page::ALL {
        assert!(
            drawer_pages.contains(&page),
            "Page {:?} is not reachable from drawer navigation",
            page
        );
    }
}

#[test]
fn drawer_mode_navigation_allows_page_switching() {
    // Simulate drawer navigation: user can navigate to any page from any page.
    let mut state = AppState::default();
    let drawer_pages: Vec<Page> = NAVIGATION_GROUPS
        .iter()
        .flat_map(|g| g.pages.iter().copied())
        .chain(FOOTER_PAGES.iter().copied())
        .collect();

    for &target in &drawer_pages {
        state.reduce(AppIntent::Navigate(target));
        assert_eq!(state.active_page, target);
    }
}
