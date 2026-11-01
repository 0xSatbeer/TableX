//! Editor-area tab list: preview tabs, close, reopen, cycle. Generic over the tab content
//! so the rules are testable without GPUI entities.

use gpui_kit::SharedString;

mod origin;
mod search;

pub use origin::{Origin, join_path, tooltip_text};
pub use search::{empty_state, first_match, match_tabs, show_search};

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
    /// Recently closed tabs, newest last, for reopening.
    pub closed: Vec<EditorTab<C>>,
    /// Tab keys, most recently used first.
    mru: Vec<SharedString>,
    /// MRU position previewed by Ctrl+Tab while Ctrl is held.
    cycling: Option<usize>,
}

const CLOSED_HISTORY: usize = 20;

impl<C> Tabs<C> {
    pub fn new(tabs: Vec<EditorTab<C>>) -> Self {
        let mru = tabs.iter().map(|t| t.key.clone()).collect();
        Self {
            tabs,
            active: 0,
            closed: Vec::new(),
            mru,
            cycling: None,
        }
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
        } else if let Some(ix) = self
            .tabs
            .iter()
            .position(|t| t.preview)
            .filter(|_| tab.preview)
        {
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
            self.mru
                .retain(|k| *k != key && self.tabs.iter().any(|t| t.key == *k));
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
        self.active = next
            .and_then(|k| self.tabs.iter().position(|t| t.key == k))
            .unwrap_or(0);
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

    /// The unpinned tabs whose index `drop` accepts, in order: what a bulk close closes. Pinned
    /// tabs survive bulk closes, as in VS Code; they close one at a time.
    pub fn closable(&self, drop: impl Fn(usize) -> bool) -> Vec<usize> {
        (0..self.tabs.len())
            .filter(|&ix| drop(ix) && !self.tabs[ix].pinned)
            .collect()
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
        let ix = if tab.pinned {
            ix.min(pinned)
        } else {
            ix.max(pinned)
        };
        self.tabs.insert(ix, tab);
        self.active = self.tabs.iter().position(|t| t.key == active).unwrap_or(0);
    }

    /// Gives open and recently closed tabs their connection's current name, color and production flag.
    pub fn refresh_origins(&mut self, ws: &crate::model::Workspace) {
        for origin in self
            .tabs
            .iter_mut()
            .chain(&mut self.closed)
            .filter_map(|t| t.origin.as_mut())
        {
            let Some(conn) = ws.connections.iter().find(|c| c.id == origin.connection) else {
                continue;
            };
            origin.path = join_path(&conn.name, &origin.tail);
            (origin.color, origin.production) = (conn.color, conn.is_production);
        }
    }

    /// Closes every tab from connection `id`, pinned ones too, and drops them from the reopen history.
    /// Returns the tabs it closed.
    pub fn drop_connection(&mut self, id: crate::model::ConnectionId) -> Vec<EditorTab<C>> {
        let from = |t: &EditorTab<C>| t.origin.as_ref().is_some_and(|o| o.connection == id);
        // Set the history aside so these closes neither enter it nor evict other connections' tabs.
        let mut kept = std::mem::take(&mut self.closed);
        kept.retain(|t| !from(t));
        while let Some(ix) = self.tabs.iter().position(from) {
            self.close(ix);
        }
        std::mem::replace(&mut self.closed, kept)
    }

    /// The tab's title, plus `[connection]` when another open tab from a different connection
    /// has the same title, so `users` from two databases can be told apart.
    pub fn label(&self, ix: usize) -> SharedString {
        let t = &self.tabs[ix];
        let conn = |t: &EditorTab<C>| t.origin.as_ref().map(|o| o.connection);
        let clash = self
            .tabs
            .iter()
            .any(|o| o.title == t.title && conn(o) != conn(t));
        match t.origin.as_ref().filter(|_| clash) {
            Some(o) => format!("{} [{}]", t.title, o.name()).into(),
            None => t.title.clone(),
        }
    }

    pub fn activate(&mut self, ix: usize) {
        if ix < self.tabs.len() {
            self.active = ix;
            self.touch();
        }
    }
}

#[cfg(test)]
mod tests;
