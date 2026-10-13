use super::*;

mod contrast;
mod user_themes;

/// Hovers the start of the element drawn as `selector` past the tooltip delay, and says
/// whether a truncation tooltip came up.
pub(crate) fn hover_shows_truncation_tip(
    cx: &mut gpui_kit::VisualTestContext,
    selector: String,
) -> bool {
    use gpui_kit::{Modifiers, point, px};
    let target = cx
        .debug_bounds(selector.clone().leak())
        .unwrap_or_else(|| panic!("{selector} drawn"));
    cx.simulate_mouse_move(
        point(target.left() + px(4.), target.center().y),
        None,
        Modifiers::none(),
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.debug_bounds("truncation-tip").is_some()
}

#[test]
fn truncation_tip_is_the_full_text_only_when_cut() {
    let long = "shipment_line_item_carrier_tracking_events_archive_partitioned_by_month_2026_q3x";
    // (full, shown, tooltip)
    let cases = [
        (long, "shipment_line_item_carrier…", Some(long)),
        (long, "…", Some(long)),
        (long, long, None),
        ("orders", "orders", None),
        ("", "", None),
        (
            "naïve_ünïcode_名前",
            "naïve_ünïcode…",
            Some("naïve_ünïcode_名前"),
        ),
    ];
    for (full, shown, want) in cases {
        assert_eq!(
            super::truncation_tip(&full.into(), shown),
            want.map(Into::into),
            "{full:?} shown as {shown:?}"
        );
    }
}

use super::{CONNECTION_COLORS, CONNECTION_COLORS_LIGHT, DARK, LIGHT, Rgba, Zoom, px, zoom};

#[test]
fn scale_multiplies_a_token_by_the_percent() {
    use super::{DEFAULT_UI_SCALE, UI_SCALES, scale};
    assert_eq!(UI_SCALES, [90, 100, 110, 120, 130]);
    assert_eq!(DEFAULT_UI_SCALE, 100);
    let cases = [
        (13., 130, 16.9),
        (24., 130, 31.2),
        (13., 100, 13.),
        (24., 100, 24.),
        (13., 90, 11.7),
        (24., 110, 26.4),
        (24., 120, 28.8),
        (12.5, 130, 16.25),
    ];
    for (token, percent, want) in cases {
        let got: f32 = scale(px(token), percent).into();
        assert!(
            (got - want).abs() < 0.001,
            "{token} at {percent}%: {got}, want {want}"
        );
    }
}

/// Tokens and the library's rem follow the UI scale; a percent off the menu is clamped to 90..=130.
#[gpui_kit::test]
fn ui_scale_scales_every_token_and_library_text(cx: &mut gpui_kit::TestAppContext) {
    use super::{set_ui_scale, tree_row, ui, ui_scale};
    use gpui_kit::component::ActiveTheme as _;
    cx.update(gpui_kit::init);
    assert_eq!(
        (ui_scale(), ui(), tree_row()),
        (100, px(13.), px(24.)),
        "default 100%"
    );
    let cases = [
        (130, 130, 16.9, 31.2),
        (90, 90, 11.7, 21.6),
        (100, 100, 13., 24.),
        (200, 130, 16.9, 31.2),
        (50, 90, 11.7, 21.6),
    ];
    for (set, percent, text, row) in cases {
        cx.update(|cx| set_ui_scale(set, cx));
        assert_eq!(ui_scale(), percent, "set {set}");
        let (got_text, got_row): (f32, f32) = (ui().into(), tree_row().into());
        assert!(
            (got_text - text).abs() < 0.001 && (got_row - row).abs() < 0.001,
            "set {set}: UI {got_text}, tree row {got_row}"
        );
        let rem: f32 = cx.update(|cx| cx.theme().font_size).into();
        assert!(
            (rem - 12. / 0.875 * percent as f32 / 100.).abs() < 0.001,
            "set {set}: rem {rem}"
        );
    }
}

#[test]
fn zoom_steps_1px_within_9_to_24_and_resets_to_12_5() {
    use Zoom::*;
    let cases = [
        (12.5, In, 13.5),
        (24., In, 24.),
        (23.5, In, 24.),
        (23., In, 24.),
        (9.5, Out, 9.),
        (9., Out, 9.),
        (10., Out, 9.),
        (13.5, Out, 12.5),
        (9., In, 10.),
        (24., Out, 23.),
        (24., Reset, 12.5),
        (9., Reset, 12.5),
        (12.5, Reset, 12.5),
        // Out of range (never produced by zoom itself) lands back inside it.
        (30., In, 24.),
        (30., Out, 24.),
        (5., Out, 9.),
        (5., In, 9.),
    ];
    for (size, step, want) in cases {
        assert_eq!(zoom(px(size), step), px(want), "{size} {step:?}");
    }
}

/// A fresh dir under the workspace's `target/`, never the user's config dir.
pub(crate) fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/test-themes")
        .join(name);
    _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
