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
        Self {
            connection: conn.id,
            color: conn.color,
            production: conn.is_production,
            path: join_path(&conn.name, &tail),
            tail: tail.into(),
        }
    }
}

/// `name › tail`, or just `name` when there is no tail.
pub fn join_path(name: &str, tail: &str) -> SharedString {
    if tail.is_empty() {
        name.to_string().into()
    } else {
        format!("{name} › {tail}").into()
    }
}

/// Tab tooltip: title on line 1, origin path on line 2.
pub fn tooltip_text(title: &str, origin: Option<&Origin>) -> String {
    let path = origin.map_or("Sample data · not from a connection", |o| o.path.as_ref());
    format!("{title}\n{path}")
}
