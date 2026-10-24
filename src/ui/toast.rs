//! Toasts: every notification the app shows goes through here, so they look and read alike.

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::*;

/// Shows `message` as an error toast in `window`.
pub fn error(message: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
    show(Notification::error(message), window, cx);
}

/// Shows `note` in `window`.
pub fn show(note: Notification, window: &mut Window, cx: &mut App) {
    window.push_notification(note, cx);
}
