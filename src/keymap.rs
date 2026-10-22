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
#[allow(dead_code)] // read by the palette and cheat sheet
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
