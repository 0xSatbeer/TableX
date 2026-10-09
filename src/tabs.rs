//! Editor-area tab list: preview tabs, close, reopen, cycle. Generic over the tab content
//! so the rules are testable without GPUI entities.

use gpui_kit::SharedString;

/// Where a tab's data comes from. One value drives the tab stripe, the source strip above
/// the grid or console, and the status bar, so they cannot disagree.
#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    pub color: u8,
    pub production: bool,
    /// `connection › database › schema`
    pub path: SharedString,
}

impl Origin {
    /// The connection's first database and schema: where a console starts out.
    pub fn of(conn: &crate::model::Connection) -> Self {
        let db = conn.databases.first();
        let schema = db.and_then(|d| d.schemas.first());
        let path = [Some(conn.name.as_str()), db.map(|d| d.name.as_str()), schema.map(|s| s.name.as_str())];
        Self {
            color: conn.color,
            production: conn.is_production,
            path: path.into_iter().flatten().collect::<Vec<_>>().join(" › ").into(),
        }
    }
}

/// Tab tooltip: title on line 1, origin path on line 2.
pub fn tooltip_text(title: &str, origin: Option<&Origin>) -> String {
    let path = origin.map_or("Sample data · not from a connection", |o| o.path.as_ref());
    format!("{title}\n{path}")
}

pub struct EditorTab<C> {
    /// Identity: opening the same key again activates the existing tab.
    pub key: SharedString,
    pub title: SharedString,
    /// `None` for sample data that belongs to no connection.
    pub origin: Option<Origin>,
    pub tooltip: Option<SharedString>,
    pub preview: bool,
    /// Pinned tabs sit at the left, icon only.
    pub pinned: bool,
    pub content: C,
}

pub struct Tabs<C> {
    pub tabs: Vec<EditorTab<C>>,
    pub active: usize,
    closed: Vec<EditorTab<C>>,
}

const CLOSED_HISTORY: usize = 20;

impl<C> Tabs<C> {
    pub fn new(tabs: Vec<EditorTab<C>>) -> Self {
        Self { tabs, active: 0, closed: Vec::new() }
    }

    pub fn active(&self) -> Option<&EditorTab<C>> {
        self.tabs.get(self.active)
    }

    /// Activates the tab with this key if open (pinning it when `preview` is false), else
    /// opens it. A new preview replaces the existing preview tab in place.
    pub fn open(&mut self, tab: EditorTab<C>) {
        if let Some(ix) = self.tabs.iter().position(|t| t.key == tab.key) {
            self.tabs[ix].preview &= tab.preview;
            self.active = ix;
        } else if let Some(ix) = self.tabs.iter().position(|t| t.preview).filter(|_| tab.preview) {
            self.tabs[ix] = tab;
            self.active = ix;
        } else {
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
        }
    }

    pub fn close(&mut self, ix: usize) {
        if ix >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(ix);
        if self.closed.len() == CLOSED_HISTORY {
            self.closed.remove(0);
        }
        self.closed.push(tab);
        if self.active > ix || self.active == self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
    }

    pub fn reopen(&mut self) {
        if let Some(mut tab) = self.closed.pop() {
            tab.preview = false;
            self.open(tab);
        }
    }

    pub fn toggle_pin(&mut self, ix: usize) {
        if ix >= self.tabs.len() {
            return;
        }
        let mut tab = self.tabs.remove(ix);
        (tab.pinned, tab.preview) = (!tab.pinned, false);
        let at = self.tabs.iter().take_while(|t| t.pinned).count();
        self.tabs.insert(at, tab);
        self.active = at;
    }

    /// Closes every unpinned tab whose index `drop` accepts, keeping the active one active if it
    /// stays. Pinned tabs survive bulk closes, as in VS Code; they close one at a time.
    pub fn close_where(&mut self, drop: impl Fn(usize) -> bool) {
        let doomed: Vec<usize> = (0..self.tabs.len()).rev().filter(|&ix| drop(ix) && !self.tabs[ix].pinned).collect();
        for ix in doomed {
            self.close(ix);
        }
    }

    pub fn cycle(&mut self, forward: bool) {
        let n = self.tabs.len();
        if n > 0 {
            self.active = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n };
        }
    }

    /// Moves the tab at `from` to just before index `to` (a drop slot, 0..=len).
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() {
            return;
        }
        let to = to.min(self.tabs.len());
        let active = self.tabs[self.active.min(self.tabs.len() - 1)].key.clone();
        let tab = self.tabs.remove(from);
        let ix = if to > from { to - 1 } else { to };
        let pinned = self.tabs.iter().take_while(|t| t.pinned).count();
        let ix = if tab.pinned { ix.min(pinned) } else { ix.max(pinned) };
        self.tabs.insert(ix, tab);
        self.active = self.tabs.iter().position(|t| t.key == active).unwrap_or(0);
    }

    pub fn activate(&mut self, ix: usize) {
        if ix < self.tabs.len() {
            self.active = ix;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{tooltip_text, EditorTab, Origin, Tabs};

    fn tab(key: &'static str, preview: bool) -> EditorTab<()> {
        EditorTab { key: key.into(), title: key.into(), origin: None, tooltip: None, preview, pinned: false, content: () }
    }

    fn pinned(key: &'static str) -> EditorTab<()> {
        EditorTab { pinned: true, ..tab(key, false) }
    }

    fn keys(t: &Tabs<()>) -> Vec<&str> {
        t.tabs.iter().map(|t| t.key.as_ref()).collect()
    }

    #[test]
    fn preview_replaces_preview_and_pins_on_reopen() {
        let mut t = Tabs::new(vec![tab("console", false)]);
        t.open(tab("users", true));
        t.open(tab("orders", true));
        assert_eq!(keys(&t), ["console", "orders"], "second preview replaced the first");
        t.open(tab("orders", false));
        assert!(!t.tabs[1].preview, "opening pinned pins the preview");
        t.open(tab("items", true));
        assert_eq!(keys(&t), ["console", "orders", "items"]);
        assert_eq!(t.active, 2);
    }

    #[test]
    fn close_and_reopen_keep_active_sane() {
        let mut t = Tabs::new(vec![tab("a", false), tab("b", false), tab("c", false)]);
        t.activate(2);
        t.close(2);
        assert_eq!(t.active, 1, "closing the last active tab activates its left neighbour");
        t.close(0);
        assert_eq!((keys(&t), t.active), (vec!["b"], 0));
        t.reopen();
        assert_eq!((keys(&t), t.active), (vec!["b", "a"], 1));
        t.cycle(true);
        assert_eq!(t.active, 0);
        t.cycle(false);
        assert_eq!(t.active, 1);
    }

    #[test]
    fn pin_moves_left_and_close_others_keeps_active() {
        let mut t = Tabs::new(vec![tab("a", false), tab("b", false), tab("c", true)]);
        t.toggle_pin(2);
        assert_eq!((keys(&t), t.active), (vec!["c", "a", "b"], 0));
        assert!(t.tabs[0].pinned && !t.tabs[0].preview, "pinning also ends preview");
        t.activate(2);
        let active = t.active;
        t.close_where(|ix| ix != active);
        assert_eq!((keys(&t), t.active), (vec!["c", "b"], 1), "close others keeps pinned tabs");

        let mut t = Tabs::new(vec![tab("a", false), tab("b", false), tab("c", false), tab("d", false)]);
        t.activate(1);
        t.close_where(|ix| ix > 1);
        assert_eq!((keys(&t), t.active), (vec!["a", "b"], 1), "close to the right keeps the active tab");
    }

    #[test]
    fn move_tab_unpinned() {
        // (from, drop slot, expected)
        let cases: [(usize, usize, [&str; 4]); 8] = [
            (0, 3, ["b", "c", "a", "d"]),
            (3, 0, ["d", "a", "b", "c"]),
            (0, 4, ["b", "c", "d", "a"]),
            (1, 1, ["a", "b", "c", "d"]),
            (1, 2, ["a", "b", "c", "d"]),
            (2, 99, ["a", "b", "d", "c"]),
            (99, 0, ["a", "b", "c", "d"]),
            (0, 0, ["a", "b", "c", "d"]),
        ];
        for (from, to, want) in cases {
            let mut t = Tabs::new(vec![tab("a", false), tab("b", false), tab("c", false), tab("d", false)]);
            t.move_tab(from, to);
            assert_eq!(keys(&t), want, "move {from} before {to}");
        }
        for n in 0..=1 {
            let mut t = Tabs::new((0..n).map(|_| tab("a", false)).collect());
            t.move_tab(0, 0);
            t.move_tab(0, 1);
            assert_eq!(t.tabs.len(), n, "{n} tabs: nothing to move");
        }
    }

    #[test]
    fn move_tab_keeps_pinned_block_left() {
        // (from, drop slot, expected); tabs are [pa, pb, c, d]
        let cases: [(usize, usize, [&str; 4]); 7] = [
            (3, 0, ["pa", "pb", "d", "c"]),
            (3, 1, ["pa", "pb", "d", "c"]),
            (2, 0, ["pa", "pb", "c", "d"]),
            (0, 4, ["pb", "pa", "c", "d"]),
            (0, 3, ["pb", "pa", "c", "d"]),
            (1, 0, ["pb", "pa", "c", "d"]),
            (0, 1, ["pa", "pb", "c", "d"]),
        ];
        for (from, to, want) in cases {
            let mut t = Tabs::new(vec![pinned("pa"), pinned("pb"), tab("c", false), tab("d", false)]);
            t.move_tab(from, to);
            assert_eq!(keys(&t), want, "move {from} before {to}");
        }
    }

    #[test]
    fn move_tab_active_follows_its_tab() {
        // (active, from, drop slot, expected keys, expected active)
        let cases: [(usize, usize, usize, [&str; 3], usize); 6] = [
            (2, 2, 0, ["c", "a", "b"], 0),
            (2, 2, 2, ["a", "b", "c"], 2),
            (0, 2, 0, ["c", "a", "b"], 1),
            (1, 0, 3, ["b", "c", "a"], 0),
            (1, 1, 3, ["a", "c", "b"], 2),
            (1, 0, 1, ["a", "b", "c"], 1),
        ];
        for (active, from, to, want, want_active) in cases {
            let mut t = Tabs::new(vec![tab("a", false), tab("b", false), tab("c", false)]);
            t.activate(active);
            let key = t.tabs[active].key.clone();
            t.move_tab(from, to);
            assert_eq!((keys(&t), t.active), (want.to_vec(), want_active), "active {active}, move {from} before {to}");
            assert_eq!(t.tabs[t.active].key, key, "same tab stays active");
        }
    }

    #[test]
    fn tooltip_is_title_then_origin_path() {
        let origin = |path: &str| Origin { color: 0, production: false, path: path.to_string().into() };
        let cases = [
            ("users", Some(origin("prod-pg › app_db › public")), "users\nprod-pg › app_db › public"),
            ("big_events", None, "big_events\nSample data · not from a connection"),
            ("Console", Some(origin("dev")), "Console\ndev"),
            ("", None, "\nSample data · not from a connection"),
        ];
        for (title, o, want) in cases {
            assert_eq!(tooltip_text(title, o.as_ref()), want, "{title}");
        }
    }
}
