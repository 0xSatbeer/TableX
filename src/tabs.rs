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

    pub fn activate(&mut self, ix: usize) {
        if ix < self.tabs.len() {
            self.active = ix;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EditorTab, Tabs};

    fn tab(key: &'static str, preview: bool) -> EditorTab<()> {
        EditorTab { key: key.into(), title: key.into(), origin: None, tooltip: None, preview, pinned: false, content: () }
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
}
