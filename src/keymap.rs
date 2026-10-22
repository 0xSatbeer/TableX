//! Every default key binding, and the `keymap.toml` (C10) overrides applied on top.

use std::collections::BTreeMap;
use std::path::Path;

use gpui_kit::*;

use crate::shell::{OVERFLOW_CONTEXT, PANEL_MAXIMIZED_CONTEXT};
use crate::ui::console;

/// Matched at the console editor's own depth, or its ctrl-enter (Enter { secondary }) wins.
const CONSOLE_EDITOR: &str = "Console > Input";

/// (action name, keystrokes, key context) for every built-in binding.
pub const DEFAULTS: &[(&str, &str, Option<&str>)] = &[
    // `Popover` binds space to Confirm (closing it), which would swallow spaces typed in the
    // overflow search field; a no-op binding in its own, deeper context lets the field type them.
    ("zed::NoAction", "space", Some(OVERFLOW_CONTEXT)),
    (
        "queryx::RestorePanel",
        "escape",
        Some(PANEL_MAXIMIZED_CONTEXT),
    ),
    // With neither a grid nor an editor focused nothing handles these.
    ("theme::ZoomIn", "ctrl-=", None),
    ("theme::ZoomOut", "ctrl--", None),
    ("theme::ZoomReset", "ctrl-0", None),
    ("console::RunStatement", "ctrl-enter", Some(CONSOLE_EDITOR)),
    (
        "console::RunScript",
        "ctrl-shift-enter",
        Some(CONSOLE_EDITOR),
    ),
    ("console::Cancel", "escape", Some(CONSOLE_EDITOR)),
    ("console::Rerun", "f5", Some(console::CONTEXT)),
    ("queryx::Quit", "ctrl-q", None),
    ("queryx::ToggleExplorer", "ctrl-b", None),
    ("queryx::ToggleBottomPanel", "ctrl-j", None),
    ("queryx::MaximizeEditor", "ctrl-shift-f", None),
    ("queryx::ToggleGallery", "ctrl-alt-shift-d", None),
    ("queryx::FocusExplorer", "ctrl-alt-f", None),
    ("queryx::CloseTab", "ctrl-w", None),
    ("queryx::ReopenTab", "ctrl-shift-t", None),
    ("queryx::NextTab", "ctrl-tab", None),
    ("queryx::PrevTab", "ctrl-shift-tab", None),
    ("queryx::CycleTheme", "ctrl-k ctrl-t", None),
    ("queryx::CommandPalette", "ctrl-shift-p", None),
    ("queryx::CheatSheet", "ctrl-k ctrl-s", None),
    ("queryx::QuickOpen", "ctrl-p", None),
];

/// (action name, human name, area) for every action the palette lists.
pub const NAMES: &[(&str, &str, &str)] = &[
    ("queryx::CommandPalette", "Command palette", "General"),
    ("queryx::CheatSheet", "Keyboard shortcuts", "General"),
    ("queryx::QuickOpen", "Go to object", "General"),
    ("queryx::Quit", "Quit", "General"),
    ("queryx::CycleTheme", "Cycle theme", "Appearance"),
    ("theme::ZoomIn", "Zoom in", "Appearance"),
    ("theme::ZoomOut", "Zoom out", "Appearance"),
    ("theme::ZoomReset", "Reset zoom", "Appearance"),
    (
        "queryx::ToggleGallery",
        "Toggle component gallery",
        "Appearance",
    ),
    ("queryx::ToggleExplorer", "Toggle explorer", "Layout"),
    ("queryx::ToggleBottomPanel", "Toggle bottom panel", "Layout"),
    ("queryx::MaximizeEditor", "Maximize editor", "Layout"),
    ("queryx::MaximizePanel", "Maximize bottom panel", "Layout"),
    ("queryx::RestorePanel", "Restore bottom panel", "Layout"),
    ("queryx::FocusExplorer", "Focus explorer", "Layout"),
    ("queryx::CloseTab", "Close tab", "Tabs"),
    ("queryx::ReopenTab", "Reopen closed tab", "Tabs"),
    ("queryx::NextTab", "Next tab", "Tabs"),
    ("queryx::PrevTab", "Previous tab", "Tabs"),
    ("queryx::CloseOtherTabs", "Close other tabs", "Tabs"),
    (
        "queryx::CloseTabsToRight",
        "Close tabs to the right",
        "Tabs",
    ),
    ("queryx::CloseAllTabs", "Close all tabs", "Tabs"),
    ("queryx::TogglePinTab", "Pin or unpin tab", "Tabs"),
    ("queryx::CopyTabName", "Copy tab name", "Tabs"),
    ("queryx::RevealTab", "Reveal tab in explorer", "Tabs"),
    ("queryx::AddConnection", "Add connection", "Connections"),
    ("queryx::EditConnection", "Edit connection", "Connections"),
    (
        "queryx::DeleteConnection",
        "Delete connection",
        "Connections",
    ),
    ("queryx::RefreshNode", "Refresh", "Explorer"),
    ("queryx::ModifyObject", "Modify object", "Explorer"),
    ("queryx::NewTable", "New table", "Explorer"),
    ("queryx::NewDatabase", "New database", "Explorer"),
    ("queryx::NewRole", "New role", "Explorer"),
    ("queryx::NewUser", "New user", "Explorer"),
    ("queryx::OpenConsole", "Open console", "Explorer"),
    ("console::RunStatement", "Run statement", "Console"),
    ("console::RunScript", "Run script", "Console"),
    ("console::Rerun", "Rerun last query", "Console"),
    ("console::Cancel", "Cancel query", "Console"),
    ("grid::CopyCsv", "Copy as CSV", "Grid"),
    ("grid::CopyJson", "Copy as JSON", "Grid"),
    ("grid::CopyMarkdown", "Copy as Markdown", "Grid"),
    ("grid::CopySqlInsert", "Copy as SQL INSERT", "Grid"),
    ("grid::CopySqlInList", "Copy as SQL IN list", "Grid"),
    ("grid::HideColumn", "Hide column", "Grid"),
    ("grid::ShowAllColumns", "Show all columns", "Grid"),
    ("grid::Paste", "Paste", "Grid"),
    ("grid::SetValue", "Set value", "Grid"),
];

/// `keymap.toml`'s `[bindings]`: action name → keystrokes; `""` unbinds.
pub type Overrides = BTreeMap<String, String>;

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub action: String,
    pub keys: String,
    pub context: Option<&'static str>,
}

/// The bindings in effect (action name → keystrokes), for the palette and cheat sheet.
pub struct Bindings(pub Vec<Binding>);

impl Global for Bindings {}

#[derive(serde::Deserialize)]
struct FileToml {
    #[allow(dead_code)]
    version: Option<u32>,
    #[serde(default)]
    bindings: Overrides,
}

/// Loads `<root>/keymap.toml`; empty when absent. A corrupt file is moved to `.bak` and the
/// notice says so.
pub fn load(root: &Path) -> (Overrides, Option<String>) {
    match crate::persist::read_or_backup(&root.join("keymap.toml"), from_toml) {
        Ok(overrides) => (overrides.unwrap_or_default(), None),
        Err(notice) => (Overrides::new(), Some(notice)),
    }
}

fn from_toml(text: &str) -> Result<Overrides, toml::de::Error> {
    toml::from_str::<FileToml>(text).map(|f| f.bindings)
}

/// The defaults with each overridden action's bindings replaced by its override, in the
/// context of its first default. Overrides come last, so they win over defaults on the same keys.
pub fn effective(overrides: &Overrides) -> Vec<Binding> {
    let defaults = DEFAULTS
        .iter()
        .filter(|(action, ..)| !overrides.contains_key(*action));
    let replaced = overrides
        .iter()
        .filter(|(_, keys)| !keys.is_empty())
        .map(|(action, keys)| {
            let context = DEFAULTS
                .iter()
                .find(|(a, ..)| a == action)
                .and_then(|d| d.2);
            Binding {
                action: action.clone(),
                keys: keys.clone(),
                context,
            }
        });
    defaults
        .map(|&(action, keys, context)| Binding {
            action: action.into(),
            keys: keys.into(),
            context,
        })
        .chain(replaced)
        .collect()
}

/// Binds the defaults with `overrides` applied and stores them as `Bindings`. Returns the
/// overrides skipped: unknown actions and bad keystrokes.
pub fn bind(cx: &mut App, overrides: &Overrides) -> Vec<String> {
    // Checked before `effective`, so a skipped entry keeps its action's defaults.
    let mut skipped = Vec::new();
    let mut valid = Overrides::new();
    for (action, keys) in overrides {
        if cx.build_action(action, None).is_err() {
            skipped.push(format!("{action} (unknown action)"));
        } else if keys
            .split_whitespace()
            .any(|k| Keystroke::parse(k).is_err())
        {
            skipped.push(format!("{action} = \"{keys}\" (bad keystrokes)"));
        } else {
            valid.insert(action.clone(), keys.clone());
        }
    }
    let bound = effective(&valid);
    let key_bindings: Vec<_> = bound
        .iter()
        .map(|binding| {
            let action = cx
                .build_action(&binding.action, None)
                .expect("checked above");
            let context = binding.context.map(|c| {
                KeyBindingContextPredicate::parse(c)
                    .expect("valid key context")
                    .into()
            });
            KeyBinding::load(
                &binding.keys,
                action,
                context,
                false,
                None,
                &DummyKeyboardMapper,
            )
            .expect("checked above")
        })
        .collect();
    cx.bind_keys(key_bindings);
    cx.set_global(Bindings(bound));
    skipped
}

/// Binds the defaults `keep` selects, unchanged; tests bind just the keys they exercise.
#[cfg(test)]
pub fn bind_defaults(cx: &mut App, keep: impl Fn(&str, Option<&str>) -> bool) {
    let some: Overrides = DEFAULTS
        .iter()
        .filter(|(action, _, context)| !keep(action, *context))
        .map(|(action, ..)| (action.to_string(), String::new()))
        .collect();
    bind(cx, &some);
}

#[cfg(test)]
mod tests;
