//! Editor-area tab list: preview tabs, close, reopen, cycle. Generic over the tab content
//! so the rules are testable without GPUI entities.

use gpui_kit::SharedString;

/// Where a tab's data comes from. One value drives the tab stripe, the source strip above
/// the grid or console, and the status bar, so they cannot disagree.
#[derive(Clone, Debug, PartialEq)]
pub struct Origin {
    pub connection: crate::model::ConnectionId,
    pub color: u8,
    pub production: bool,
    /// `connection › database › schema`
    pub path: SharedString,
    /// `database › schema`: the path without the connection name, so a rename never has to parse the old name.
    pub tail: SharedString,
}

impl Origin {
    /// The database the connection's settings name: where a console starts out.
    pub fn of(conn: &crate::model::Connection) -> Self {
        let tail = conn.params.database();
        Self { connection: conn.id, color: conn.color, production: conn.is_production, path: join_path(&conn.name, &tail), tail: tail.into() }
    }
}

/// `name › tail`, or just `name` when there is no tail.
pub fn join_path(name: &str, tail: &str) -> SharedString {
    if tail.is_empty() { name.to_string().into() } else { format!("{name} › {tail}").into() }
}

/// Tab tooltip: title on line 1, origin path on line 2.
pub fn tooltip_text(title: &str, origin: Option<&Origin>) -> String {
    let path = origin.map_or("Sample data · not from a connection", |o| o.path.as_ref());
    format!("{title}\n{path}")
}

/// More tabs than this and the overflow list gets a search field.
pub const SEARCH_MIN_TABS: usize = 10;

pub fn show_search(tab_count: usize) -> bool {
    tab_count > SEARCH_MIN_TABS
}

/// Indices of the tabs matching `query` in the overflow search.
pub fn match_tabs<C>(tabs: &[EditorTab<C>], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    let hit = |s: &str| s.to_lowercase().contains(&query);
    (0..tabs.len()).filter(|&ix| hit(&tabs[ix].title) || tabs[ix].origin.as_ref().is_some_and(|o| hit(&o.path))).collect()
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
    /// Tab keys, most recently used first.
    mru: Vec<SharedString>,
    /// MRU position previewed by Ctrl+Tab while Ctrl is held.
    cycling: Option<usize>,
}

const CLOSED_HISTORY: usize = 20;

impl<C> Tabs<C> {
    pub fn new(tabs: Vec<EditorTab<C>>) -> Self {
        let mru = tabs.iter().map(|t| t.key.clone()).collect();
        Self { tabs, active: 0, closed: Vec::new(), mru, cycling: None }
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
        self.touch();
    }

    /// Moves the active tab to the MRU front and ends any Ctrl+Tab preview.
    fn touch(&mut self) {
        self.cycling = None;
        if let Some(key) = self.tabs.get(self.active).map(|t| t.key.clone()) {
            // Also drops keys of tabs no longer open (a preview replaced in place).
            self.mru.retain(|k| *k != key && self.tabs.iter().any(|t| t.key == *k));
            self.mru.insert(0, key);
        }
    }

    pub fn close(&mut self, ix: usize) {
        if ix >= self.tabs.len() {
            return;
        }
        self.commit();
        let keep = (ix != self.active).then(|| self.tabs[self.active].key.clone());
        let tab = self.tabs.remove(ix);
        self.mru.retain(|k| *k != tab.key);
        if self.closed.len() == CLOSED_HISTORY {
            self.closed.remove(0);
        }
        self.closed.push(tab);
        let next = keep.or_else(|| self.mru.first().cloned());
        self.active = next.and_then(|k| self.tabs.iter().position(|t| t.key == k)).unwrap_or(0);
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
        self.touch();
    }

    /// Closes every unpinned tab whose index `drop` accepts, keeping the active one active if it
    /// stays. Pinned tabs survive bulk closes, as in VS Code; they close one at a time.
    pub fn close_where(&mut self, drop: impl Fn(usize) -> bool) {
        let doomed: Vec<usize> = (0..self.tabs.len()).rev().filter(|&ix| drop(ix) && !self.tabs[ix].pinned).collect();
        for ix in doomed {
            self.close(ix);
        }
    }

    /// Ctrl+Tab while Ctrl is held: previews the next tab in MRU order.
    pub fn cycle(&mut self, forward: bool) {
        let n = self.mru.len();
        if n < 2 {
            return;
        }
        let pos = (self.cycling.unwrap_or(0) + if forward { 1 } else { n - 1 }) % n;
        self.cycling = Some(pos);
        if let Some(ix) = self.tabs.iter().position(|t| t.key == self.mru[pos]) {
            self.active = ix;
        }
    }

    /// Ctrl released: the previewed tab becomes the most recently used.
    pub fn commit(&mut self) {
        if self.cycling.is_some() {
            self.touch();
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

    /// Gives open and recently closed tabs their connection's current name, color and production flag.
    pub fn refresh_origins(&mut self, ws: &crate::model::Workspace) {
        for origin in self.tabs.iter_mut().chain(&mut self.closed).filter_map(|t| t.origin.as_mut()) {
            let Some(conn) = ws.connections.iter().find(|c| c.id == origin.connection) else { continue };
            origin.path = join_path(&conn.name, &origin.tail);
            (origin.color, origin.production) = (conn.color, conn.is_production);
        }
    }

    /// Closes every tab from connection `id`, pinned ones too, and drops them from the reopen history.
    pub fn drop_connection(&mut self, id: crate::model::ConnectionId) {
        let from = |t: &EditorTab<C>| t.origin.as_ref().is_some_and(|o| o.connection == id);
        // Set the history aside so these closes neither enter it nor evict other connections' tabs.
        let mut kept = std::mem::take(&mut self.closed);
        kept.retain(|t| !from(t));
        while let Some(ix) = self.tabs.iter().position(from) {
            self.close(ix);
        }
        self.closed = kept;
    }

    pub fn activate(&mut self, ix: usize) {
        if ix < self.tabs.len() {
            self.active = ix;
            self.touch();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{tooltip_text, EditorTab, Origin, Tabs};

    #[test]
    fn a_console_origin_uses_the_dialog_database_or_the_sqlite_file_name() {
        use crate::model::{Connection, ConnectionId, ConnectionParams, ConnectionStatus, Database, Schema, SslMode};
        let pg = |database: &str| ConnectionParams::Postgres { host: "h".into(), port: 5432, user: "u".into(), password: None, database: database.into(), ssl: SslMode::Prefer };
        let sqlite = |path: &str| ConnectionParams::Sqlite { path: path.into() };
        let listed = |names: &[&str]| Some(names.iter().map(|n| Database { name: n.to_string(), schemas: Some(vec![Schema { name: "public".into(), objects: None }]) }).collect());
        // (params, databases listed, origin database)
        let cases = [
            (pg("app"), listed(&["analytics", "app"]), "app"),
            (pg("app"), None, "app"),
            (pg("app"), listed(&[]), "app"),
            (pg("app"), listed(&["analytics"]), "app"),
            (sqlite("/x/local.db"), listed(&["local.db"]), "local.db"),
            (sqlite("/x/local.db"), listed(&["main"]), "local.db"),
            (sqlite("/x/local.db"), None, "local.db"),
            (sqlite("notes"), None, "notes"),
        ];
        for (params, databases, want) in cases {
            let label = format!("{params:?} {databases:?}");
            let conn = Connection { id: ConnectionId(1), name: "c".into(), color: 0, is_production: false, status: ConnectionStatus::Connected, params, databases };
            let origin = Origin::of(&conn);
            assert_eq!((origin.tail.as_ref(), origin.path.as_ref()), (want, format!("c › {want}").as_str()), "{label}");
        }
    }

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
        assert_eq!(t.active, 0, "closing the active tab activates the next MRU tab");
        t.close(0);
        assert_eq!((keys(&t), t.active), (vec!["b"], 0));
        t.reopen();
        assert_eq!((keys(&t), t.active), (vec!["b", "a"], 1));
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
        (t.tabs[t.active].key.as_ref(), t.mru.iter().map(|k| k.as_ref()).collect())
    }

    #[test]
    fn ctrl_tab_release_commits_previous_mru_tab() {
        // (MRU before, steps, active after, MRU after)
        let cases: [(&[&'static str], &str, &str, &[&str]); 2] = [
            (&["c", "b", "a"], "nr", "b", &["b", "c", "a"]),
            (&["b", "a"], "nr", "a", &["a", "b"]),
        ];
        for (mru, steps, active, want) in cases {
            let mut t = with_mru(mru);
            run(&mut t, steps);
            assert_eq!(state(&t), (active, want.to_vec()), "MRU {mru:?}, steps {steps}");
        }
    }

    #[test]
    fn ctrl_tab_held_walks_further_down_the_mru() {
        // (MRU before, steps, active after, MRU after)
        let cases: [(&[&'static str], &str, &str, &[&str]); 3] = [
            (&["c", "b", "a"], "nnr", "a", &["a", "c", "b"]),
            (&["c", "b", "a"], "nn", "a", &["c", "b", "a"]),
            (&["c", "b", "a"], "nrnr", "c", &["c", "b", "a"]),
        ];
        for (mru, steps, active, want) in cases {
            let mut t = with_mru(mru);
            run(&mut t, steps);
            assert_eq!(state(&t), (active, want.to_vec()), "MRU {mru:?}, steps {steps}");
        }
    }

    #[test]
    fn ctrl_tab_wraps_and_shift_walks_backward() {
        // (MRU before, steps, active after, MRU after)
        let cases: [(&[&'static str], &str, &str, &[&str]); 6] = [
            (&["c", "b", "a"], "pr", "a", &["a", "c", "b"]),
            (&["c", "b", "a"], "nnnr", "c", &["c", "b", "a"]),
            (&["c", "b", "a"], "ppr", "b", &["b", "c", "a"]),
            (&["c", "b", "a"], "npr", "c", &["c", "b", "a"]),
            (&["c", "b", "a"], "nnnnr", "b", &["b", "c", "a"]),
            (&["b", "a"], "nnr", "b", &["b", "a"]),
        ];
        for (mru, steps, active, want) in cases {
            let mut t = with_mru(mru);
            run(&mut t, steps);
            assert_eq!(state(&t), (active, want.to_vec()), "MRU {mru:?}, steps {steps}");
        }
    }

    #[test]
    fn ctrl_tab_with_one_or_no_tab_changes_nothing() {
        for steps in ["nr", "pr", "nnr", "n", "r"] {
            let mut t = with_mru(&["a"]);
            run(&mut t, steps);
            assert_eq!(state(&t), ("a", vec!["a"]), "one tab, steps {steps}");
            let mut t: Tabs<()> = Tabs::new(Vec::new());
            run(&mut t, steps);
            assert_eq!((t.tabs.len(), t.active, t.mru.len()), (0, 0, 0), "no tabs, steps {steps}");
        }
    }

    #[test]
    fn close_activates_next_mru_tab() {
        // (MRU before, steps before the close, closed key, active after, MRU after); tabs [a, b, c]
        let cases: [(&[&'static str], &str, &str, &str, &[&str]); 7] = [
            (&["c", "b", "a"], "", "c", "b", &["b", "a"]),
            (&["a", "c", "b"], "", "a", "c", &["c", "b"]),
            (&["c", "b", "a"], "", "a", "c", &["c", "b"]),
            (&["c", "b", "a"], "", "b", "c", &["c", "a"]),
            // Mid-cycle: the previewed tab is committed first, as if Ctrl were released.
            (&["c", "b", "a"], "n", "b", "c", &["c", "a"]),
            (&["c", "b", "a"], "n", "a", "b", &["b", "c"]),
            (&["c", "b", "a"], "nn", "c", "a", &["a", "b"]),
        ];
        for (mru, steps, closed, active, want) in cases {
            let mut t = with_mru(mru);
            run(&mut t, steps);
            let ix = t.tabs.iter().position(|x| x.key == closed).unwrap();
            t.close(ix);
            assert_eq!(state(&t), (active, want.to_vec()), "MRU {mru:?}, steps {steps}, close {closed}");
            t.commit();
            assert_eq!(state(&t), (active, want.to_vec()), "release after the close changes nothing");
        }
        let mut t = with_mru(&["a"]);
        t.close(0);
        assert_eq!((t.tabs.len(), t.active, t.mru.len()), (0, 0, 0), "closing the last tab");
    }

    #[test]
    fn activating_moves_tab_to_mru_front() {
        // (MRU before, steps, tab order after, active after, MRU after); tabs [a, b, c]
        let cases: [(&[&'static str], &str, &[&str], &str, &[&str]); 11] = [
            (&["c", "b", "a"], "0", &["a", "b", "c"], "a", &["a", "c", "b"]),
            (&["c", "b", "a"], "1", &["a", "b", "c"], "b", &["b", "c", "a"]),
            (&["b", "a", "c"], "2", &["a", "b", "c"], "c", &["c", "b", "a"]),
            (&["c", "b", "a"], "n0r", &["a", "b", "c"], "a", &["a", "c", "b"]),
            (&["c", "b", "a"], "o", &["a", "b", "c", "d"], "d", &["d", "c", "b", "a"]),
            (&["c", "b", "a"], "nor", &["a", "b", "c", "d"], "d", &["d", "c", "b", "a"]),
            (&["c", "b", "a"], "e", &["a", "b", "c"], "a", &["a", "c", "b"]),
            (&["c", "b", "a"], "wu", &["a", "b", "c"], "c", &["c", "b", "a"]),
            (&["c", "b", "a"], "nnmr", &["a", "b", "c"], "a", &["a", "c", "b"]),
            (&["c", "b", "a"], "nmr", &["b", "a", "c"], "b", &["b", "c", "a"]),
            (&["c", "b", "a"], "1k", &["b", "a", "c"], "b", &["b", "c", "a"]),
        ];
        for (mru, steps, order, active, want) in cases {
            let mut t = with_mru(mru);
            run(&mut t, steps);
            assert_eq!((keys(&t), state(&t)), (order.to_vec(), (active, want.to_vec())), "MRU {mru:?}, steps {steps}");
        }
        let mut t = with_mru(&["a"]);
        run(&mut t, "wu");
        assert_eq!(state(&t), ("a", vec!["a"]), "reopen after closing the last tab");
        let mut t = with_mru(&["c", "b", "a"]);
        run(&mut t, "nwu");
        assert_eq!(state(&t), ("b", vec!["b", "c", "a"]), "reopen the tab closed mid-cycle");

        let mut t = Tabs::new(vec![tab("a", false), tab("p", true)]);
        t.open(tab("q", true));
        assert_eq!((keys(&t), state(&t)), (vec!["a", "q"], ("q", vec!["q", "a"])), "a replaced preview leaves the MRU");
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
        let origin = |path: &str| Origin { connection: crate::model::ConnectionId(0), color: 0, production: false, path: path.to_string().into(), tail: "".into() };
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

    #[test]
    fn overflow_search_field_only_above_ten_tabs() {
        for (count, shown) in [(0, false), (1, false), (10, false), (11, true), (12, true)] {
            assert_eq!(super::show_search(count), shown, "{count} tabs");
        }
    }

    fn titled(title: &'static str) -> EditorTab<()> {
        EditorTab { title: title.into(), ..tab(title, false) }
    }

    fn matches(titles: &[&'static str], query: &str) -> Vec<&'static str> {
        let tabs: Vec<_> = titles.iter().map(|t| titled(t)).collect();
        super::match_tabs(&tabs, query).into_iter().map(|ix| titles[ix]).collect()
    }

    #[test]
    fn match_tabs_filters_by_title() {
        let all = ["orders", "users", "ord_items"];
        for (query, want) in [
            ("ord", vec!["orders", "ord_items"]),
            ("ORD", vec!["orders", "ord_items"]),
            ("items", vec!["ord_items"]),
            ("", vec!["orders", "users", "ord_items"]),
            ("zzz", vec![]),
        ] {
            assert_eq!(matches(&all, query), want, "query {query:?}");
        }
        assert!(matches(&[], "ord").is_empty());
    }

    #[test]
    fn match_tabs_filters_by_path() {
        let origin = |path: &'static str| Some(Origin { connection: crate::model::ConnectionId(0), color: 0, production: false, path: path.into(), tail: "".into() });
        let tabs = [
            EditorTab { origin: origin("prod-pg › app_db › public"), ..titled("users") },
            EditorTab { origin: origin("prod-pg › app_db › audit"), ..titled("log") },
            titled("big_events"),
        ];
        for (query, want) in [("PUBLIC", vec![0]), ("public", vec![0]), ("app_db", vec![0, 1]), ("prod-pg › app_db › audit", vec![1]), ("sample", vec![])] {
            assert_eq!(super::match_tabs(&tabs, query), want, "query {query:?}");
        }
    }

    #[test]
    fn overflow_no_match_is_labelled() {
        let tabs = [titled("orders"), titled("users"), titled("ord_items")];
        for (query, want) in [("zzz", Some("No tabs match")), ("ord", None), ("", None)] {
            assert_eq!(super::empty_state(&tabs, query), want, "query {query:?}");
        }
        assert_eq!(super::empty_state::<()>(&[], ""), Some("No tabs match"));
    }

    #[test]
    fn overflow_enter_picks_first_match() {
        let tabs = [titled("orders"), titled("users"), titled("ord_items")];
        for (query, want) in [("ord", Some(0)), ("ite", Some(2)), ("", Some(0)), ("zzz", None)] {
            assert_eq!(super::first_match(&tabs, query), want, "query {query:?}");
        }
        assert_eq!(super::first_match::<()>(&[], ""), None);
    }

    #[test]
    fn refresh_origins_cases() {
        use crate::model::{Connection, Workspace};
        let ws = crate::mock::workspaces().remove(0);
        let prod = Origin::of(&ws.connections[0]);
        let local = Origin::of(&ws.connections[2]);
        let tail = prod.path.strip_prefix("prod-pg").expect("path starts with the connection name").to_string();
        assert!(tail.starts_with(" › "), "path has segments after the name: {:?}", prod.path);
        let edit = |f: fn(&mut Connection)| {
            let mut ws = ws.clone();
            f(&mut ws.connections[0]);
            ws
        };
        let with = |color: u8, production: bool, path: &str| Origin { color, production, path: path.to_string().into(), ..prod.clone() };
        let gone = Workspace { connections: ws.connections[1..].to_vec(), ..ws.clone() };
        // (case, workspace after the change, tab origin before, origin expected after)
        let cases: Vec<(&str, Workspace, Option<Origin>, Option<Origin>)> = vec![
            ("rename", edit(|c| c.name = "live".into()), Some(prod.clone()), Some(with(0, true, &format!("live{tail}")))),
            ("recolor", edit(|c| c.color = 3), Some(prod.clone()), Some(with(3, true, &prod.path))),
            ("production off", edit(|c| c.is_production = false), Some(prod.clone()), Some(with(0, false, &prod.path))),
            ("rename, recolor and production off", edit(|c| (c.name, c.color, c.is_production) = ("live".into(), 7, false)), Some(prod.clone()), Some(with(7, false, &format!("live{tail}")))),
            ("path is only the connection name", edit(|c| c.name = "live".into()), Some(Origin { path: "prod-pg".into(), tail: "".into(), ..prod.clone() }), Some(Origin { tail: "".into(), ..with(0, true, "live") })),
            ("old name contains the separator", edit(|c| c.name = "c".into()), Some(Origin::of(&Connection { name: "a › b".into(), ..ws.connections[0].clone() })), Some(with(0, true, &format!("c{tail}")))),
            ("new name contains the old one", edit(|c| c.name = "prod-pg-2".into()), Some(prod.clone()), Some(with(0, true, &format!("prod-pg-2{tail}")))),
            ("another connection's tab", edit(|c| (c.name, c.color) = ("live".into(), 7)), Some(local.clone()), Some(local.clone())),
            ("sample tab without an origin", edit(|c| c.name = "live".into()), None, None),
            ("connection no longer in the workspace", gone, Some(prod.clone()), Some(prod.clone())),
            ("nothing changed", ws.clone(), Some(prod.clone()), Some(prod.clone())),
        ];
        for (case, ws, before, expected) in cases {
            let mut tabs = Tabs::new(vec![EditorTab { origin: before.clone(), ..tab("open", false) }, EditorTab { origin: before, ..tab("closed", false) }]);
            tabs.close(1);
            tabs.refresh_origins(&ws);
            assert_eq!(tabs.tabs[0].origin, expected, "{case}: open tab");
            tabs.reopen();
            assert_eq!(tabs.tabs[1].origin, expected, "{case}: closed tab, once reopened");
        }
    }

    #[test]
    fn dropping_a_connection_keeps_a_full_reopen_history_of_other_connections() {
        use crate::model::ConnectionId;
        let on = |key: String, conn: u32| {
            let origin = Origin { connection: ConnectionId(conn), color: 0, production: false, path: "".into(), tail: "".into() };
            EditorTab { key: key.clone().into(), title: key.into(), origin: Some(origin), tooltip: None, preview: false, pinned: false, content: () }
        };
        let others: Vec<String> = (0..super::CLOSED_HISTORY).map(|i| format!("other-{i}")).collect();
        let mut t = Tabs::new(vec![on("gone-a".into(), 2), on("keep".into(), 1), on("gone-b".into(), 2)]);
        for key in &others {
            t.open(on(key.clone(), 1));
            t.close(t.tabs.iter().position(|x| x.key == key.as_str()).unwrap());
        }
        t.drop_connection(ConnectionId(2));
        assert_eq!(keys(&t), ["keep"]);
        let mut reopened = Vec::new();
        while t.tabs.len() < 1 + super::CLOSED_HISTORY + 1 {
            let before = t.tabs.len();
            t.reopen();
            if t.tabs.len() == before {
                break;
            }
            reopened.push(t.tabs[t.active].key.to_string());
        }
        reopened.reverse();
        assert_eq!(reopened, others, "every other connection's closed tab is still reopenable");
    }
}
