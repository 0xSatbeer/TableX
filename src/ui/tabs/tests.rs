use super::{
    CLOSED_HISTORY, EditorTab, Origin, Tabs, empty_state, first_match, match_tabs, show_search,
    tooltip_text, window_title,
};

mod list;
mod mru;
mod origin;
mod search;

fn tab(key: &'static str, preview: bool) -> EditorTab<()> {
    EditorTab {
        key: key.into(),
        title: key.into(),
        origin: None,
        tooltip: None,
        preview,
        pinned: false,
        content: (),
    }
}

fn pinned(key: &'static str) -> EditorTab<()> {
    EditorTab {
        pinned: true,
        ..tab(key, false)
    }
}

fn keys(t: &Tabs<()>) -> Vec<&str> {
    t.tabs.iter().map(|t| t.key.as_ref()).collect()
}

/// Tabs [a, b, c] in position order with the given MRU order; the MRU front is active.
fn with_mru(mru: &[&'static str]) -> Tabs<()> {
    let mut t = Tabs::new(vec![tab("a", false), tab("b", false), tab("c", false)]);
    t.tabs.truncate(mru.len());
    t.mru = mru.iter().map(|&k| k.into()).collect();
    t.active = t.tabs.iter().position(|x| x.key == mru[0]).unwrap();
    t
}

/// Runs steps: 'n' Ctrl+Tab, 'p' Ctrl+Shift+Tab, 'r' release Ctrl, a digit activates that
/// position (click, Ctrl+1..9), 'o' opens new tab d, 'e' opens existing a, 'w' closes the
/// active tab, 'u' reopens, 'm' moves the active tab to the start, 'k' pins the active tab.
fn run(t: &mut Tabs<()>, steps: &str) {
    for s in steps.chars() {
        match s {
            'n' => t.cycle(true),
            'p' => t.cycle(false),
            'r' => t.commit(),
            'o' => t.open(tab("d", false)),
            'e' => t.open(tab("a", false)),
            'w' => t.close(t.active),
            'u' => t.reopen(),
            'm' => t.move_tab(t.active, 0),
            'k' => t.toggle_pin(t.active),
            d => t.activate(d.to_digit(10).unwrap() as usize),
        }
    }
}

fn state(t: &Tabs<()>) -> (&str, Vec<&str>) {
    (
        t.tabs[t.active].key.as_ref(),
        t.mru.iter().map(|k| k.as_ref()).collect(),
    )
}

fn titled(title: &'static str) -> EditorTab<()> {
    EditorTab {
        title: title.into(),
        ..tab(title, false)
    }
}

fn matches(titles: &[&'static str], query: &str) -> Vec<&'static str> {
    let tabs: Vec<_> = titles.iter().map(|t| titled(t)).collect();
    super::match_tabs(&tabs, query)
        .into_iter()
        .map(|ix| titles[ix])
        .collect()
}
