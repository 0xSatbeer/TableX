use gpui_kit::SharedString;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::tooltip::Tooltip;

/// Outlines `pane` with a muted 1px `ring` while it holds keyboard focus. The ring is an overlay drawn
/// after the content, so it neither shifts the layout nor hides under a child's background.
pub fn focus_ring(
    name: &'static str,
    pane: gpui_kit::Div,
    focused: bool,
    theme: &Theme,
) -> gpui_kit::Div {
    use gpui_kit::{
        InteractiveElement as _, ParentElement as _, Styled as _, prelude::FluentBuilder as _,
    };
    pane.relative()
        .debug_selector(|| name.into())
        .when(focused, |d| {
            d.child(
                gpui_kit::div()
                    .debug_selector(|| "focus-ring".into())
                    .absolute()
                    .inset_0()
                    .border_1()
                    .border_color(theme.ring.opacity(0.3)),
            )
        })
}

/// The tooltip for text drawn as `shown`: its `full` text, only when the box cut it.
pub fn truncation_tip(full: &SharedString, shown: &str) -> Option<SharedString> {
    (shown != full.as_ref()).then(|| full.clone())
}

/// `text` (built from `full`) on one line. Inside a `.truncate()` box it ends in "…" when it
/// doesn't fit, and only then does hovering show `full` in a tooltip.
pub fn truncating(
    id: impl Into<gpui_kit::ElementId>,
    text: gpui_kit::StyledText,
    full: SharedString,
) -> gpui_kit::InteractiveText {
    use gpui_kit::{InteractiveElement as _, ParentElement as _};
    let layout = text.layout().clone();
    gpui_kit::InteractiveText::new(id, text).tooltip(move |_, window, cx| {
        let tip = truncation_tip(&full, &layout.text())?;
        Some(
            Tooltip::element(move |_, _| {
                gpui_kit::div()
                    .debug_selector(|| "truncation-tip".into())
                    .child(tip.clone())
            })
            .build(window, cx),
        )
    })
}
