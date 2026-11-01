use super::*;

fn rgb(hex: &str) -> [f64; 3] {
    [0, 2, 4].map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap() as f64 / 255.)
}

/// `#RRGGBB` value of `key` in a theme JSON.
fn color(theme: &str, key: &str) -> [f64; 3] {
    let pat = format!("\"{key}\": \"#");
    let at = theme.find(&pat).unwrap_or_else(|| panic!("{key} missing")) + pat.len();
    rgb(&theme[at..at + 6])
}

/// WCAG 2 contrast ratio.
fn ratio(a: [f64; 3], b: [f64; 3]) -> f64 {
    let lum = |c: [f64; 3]| {
        let [r, g, b] = c.map(|v| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    };
    let (a, b) = (lum(a), lum(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn text_tokens_meet_contrast_targets() {
    let text = [
        ("foreground", "background"),
        ("muted.foreground", "background"),
        ("muted.foreground", "sidebar.background"),
        ("muted.foreground", "muted.background"),
        ("muted.foreground", "list.hover.background"),
        ("muted.foreground", "table.hover.background"),
        ("muted.foreground", "status_bar.background"),
        ("muted.foreground", "tab_bar.background"),
        ("sidebar.foreground", "sidebar.background"),
        ("tab.foreground", "tab_bar.background"),
        ("table.head.foreground", "table.head.background"),
        ("popover.foreground", "popover.background"),
        ("primary.foreground", "primary.background"),
        // gpui-kit draws danger.foreground on a danger fill (titlebar close hover), not on the page.
        ("danger.foreground", "danger.background"),
        ("background", "danger.background"),
        ("foreground", "muted.background"),
        ("foreground", "list.hover.background"),
        ("tab.active.foreground", "tab.active.background"),
        ("muted.foreground", "sidebar.accent.background"),
    ];
    // Status text on the surfaces it is drawn on, hover rows included.
    let status = ["success", "warning", "info"];
    let surfaces = [
        "background",
        "sidebar.background",
        "muted.background",
        "list.hover.background",
        "table.hover.background",
    ];
    let text: Vec<(String, &str)> = text
        .iter()
        .map(|&(f, b)| (f.to_string(), b))
        .chain(status.iter().flat_map(|s| {
            surfaces
                .iter()
                .map(move |b| (format!("{s}.foreground"), *b))
        }))
        .collect();
    for (name, theme) in [("dark", DARK), ("light", LIGHT), ("darcula", DARCULA)] {
        for (fg, bg) in &text {
            let r = ratio(color(theme, fg), color(theme, bg));
            assert!(r >= 4.5, "{name}: {fg} on {bg} is {r:.2}:1, needs 4.5:1");
        }
        for bg in [
            "table.background",
            "table.even.background",
            "table.hover.background",
        ] {
            let r = ratio(color(theme, "foreground"), color(theme, bg));
            assert!(r >= 7., "{name}: grid text on {bg} is {r:.2}:1, needs 7:1");
        }
    }
}

/// Error text is drawn as `theme.danger` (= danger.background).
#[test]
fn drawn_status_and_null_text_meet_contrast() {
    let pairs = [
        ("danger.background", "background"),
        ("danger.background", "sidebar.background"),
        ("danger.background", "muted.background"),
    ];
    for (name, theme) in [("dark", DARK), ("light", LIGHT), ("darcula", DARCULA)] {
        for (fg, bg) in pairs {
            let r = ratio(color(theme, fg), color(theme, bg));
            assert!(r >= 4.5, "{name}: {fg} on {bg} is {r:.2}:1, needs 4.5:1");
        }
    }
}

#[test]
fn grid_null_text_meets_contrast() {
    for (name, theme) in [("dark", DARK), ("light", LIGHT), ("darcula", DARCULA)] {
        for bg in ["table.background", "table.even.background"] {
            let r = ratio(color(theme, "muted.foreground"), color(theme, bg));
            assert!(
                r >= 4.5,
                "{name}: grid NULL on {bg} is {r:.2}:1, needs 4.5:1"
            );
        }
    }
}

/// Dots and tab stripes are UI components: 3:1 against every surface they sit on.
#[test]
fn connection_colors_read_on_their_theme() {
    let rgba = |c: Rgba| [c.r as f64, c.g as f64, c.b as f64];
    for (name, theme, set) in [
        ("dark", DARK, CONNECTION_COLORS),
        ("light", LIGHT, CONNECTION_COLORS_LIGHT),
        ("darcula", DARCULA, CONNECTION_COLORS),
    ] {
        for bg in [
            "background",
            "sidebar.background",
            "tab_bar.background",
            "status_bar.background",
            "table.head.background",
        ] {
            for (i, c) in set.iter().enumerate() {
                let r = ratio(rgba(*c), color(theme, bg));
                assert!(
                    r >= 3.,
                    "{name}: connection color {i} on {bg} is {r:.2}:1, needs 3:1"
                );
            }
        }
    }
}

#[test]
fn themes_define_every_syntax_token() {
    for theme in [DARK, LIGHT, DARCULA] {
        let syntax = theme
            .split("\"syntax\"")
            .nth(1)
            .expect("theme has highlight.syntax");
        for key in super::SYNTAX {
            assert!(
                syntax.contains(&format!("\"{key}\": {{")),
                "syntax.{key} missing from theme JSON"
            );
        }
    }
}

#[test]
fn light_theme_has_the_dark_key_set() {
    let keys = |t: &'static str| {
        let parts: Vec<&str> = t.split('"').collect();
        parts
            .windows(2)
            .filter(|w| w[1].trim_start().starts_with(':'))
            .map(|w| w[0])
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(DARK), keys(LIGHT));
    assert_eq!(keys(DARK), keys(DARCULA));
}

/// Library `text_sm` must land on UI_SM (see the type scale note); mono on MONO.
#[test]
fn theme_font_sizes_match_the_type_scale() {
    let num = |theme: &str, key: &str| -> f32 {
        let pat = format!("\"{key}\": ");
        let at = theme.find(&pat).unwrap_or_else(|| panic!("{key} missing")) + pat.len();
        theme[at..]
            .split(',')
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    };
    for theme in [DARK, LIGHT, DARCULA] {
        assert!(
            (num(theme, "font.size") * 0.875 - f32::from(super::UI_SM)).abs() < 0.01,
            "font.size * 0.875 != UI_SM"
        );
        assert_eq!(num(theme, "mono_font.size"), f32::from(super::MONO));
    }
}
