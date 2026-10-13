use super::EditorTab;

/// More tabs than this and the overflow list gets a search field.
pub const SEARCH_MIN_TABS: usize = 10;

pub fn show_search(tab_count: usize) -> bool {
    tab_count > SEARCH_MIN_TABS
}

/// Indices of the tabs matching `query` in the overflow search.
pub fn match_tabs<C>(tabs: &[EditorTab<C>], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    let hit = |s: &str| s.to_lowercase().contains(&query);
    (0..tabs.len())
        .filter(|&ix| {
            hit(&tabs[ix].title) || tabs[ix].origin.as_ref().is_some_and(|o| hit(&o.path))
        })
        .collect()
}

/// Shown in the overflow list when `match_tabs` finds nothing.
pub const NO_TABS_MATCH: &str = "No tabs match";

/// The overflow list's empty state: `NO_TABS_MATCH` when `query` matches no tab.
pub fn empty_state<C>(tabs: &[EditorTab<C>], query: &str) -> Option<&'static str> {
    match_tabs(tabs, query).is_empty().then_some(NO_TABS_MATCH)
}

/// The tab Enter activates in the overflow search.
pub fn first_match<C>(tabs: &[EditorTab<C>], query: &str) -> Option<usize> {
    match_tabs(tabs, query).first().copied()
}
