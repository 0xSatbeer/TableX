//! Splitter sizes for the shell's two panels, in pixels so a window resize leaves them alone.

use gpui_kit::{Pixels, px};

/// What releasing a drag does to a panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    Size(Pixels),
    /// Released below the minimum: the panel hides and keeps this size for when it comes back.
    Hidden(Pixels),
}

/// Where a drag that asked for `size` lands, given the size the panel had when it started.
pub fn resolve_drag(size: Pixels, min: Pixels, max: Pixels, start: Pixels) -> Drag {
    if size < min { Drag::Hidden(start) } else { Drag::Size(size.min(max)) }
}

/// One panel's size and its limits. `max` takes the window extent along the split's axis.
#[derive(Clone, Copy)]
pub struct Split {
    pub size: Pixels,
    pub default: Pixels,
    pub min: Pixels,
    pub max: fn(Pixels) -> Pixels,
}

impl Split {
    pub fn reset(&mut self) {}
}

pub const EXPLORER: Split = Split { size: px(260.), default: px(260.), min: px(180.), max: |_| px(600.) };
pub const BOTTOM: Split = Split { size: px(240.), default: px(240.), min: px(80.), max: |h| h };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_drag_clamps_to_range() {
        let e = EXPLORER;
        for (to, expected) in [
            (300., Drag::Size(px(300.))),
            (180., Drag::Size(px(180.))),
            (600., Drag::Size(px(600.))),
            (700., Drag::Size(px(600.))),
        ] {
            assert_eq!(resolve_drag(px(to), e.min, (e.max)(px(1200.)), e.size), expected, "explorer drag to {to}");
        }
    }

    #[test]
    fn release_below_min_hides_and_keeps_pre_drag_size() {
        let e = EXPLORER;
        for (to, start, expected) in [
            (179., 260., Drag::Hidden(px(260.))),
            (179., 412., Drag::Hidden(px(412.))),
            (0., 300., Drag::Hidden(px(300.))),
            (-50., 300., Drag::Hidden(px(300.))),
            (180., 300., Drag::Size(px(180.))),
        ] {
            assert_eq!(resolve_drag(px(to), e.min, (e.max)(px(1200.)), px(start)), expected, "explorer drag {start} -> {to}");
        }
    }

    #[test]
    fn double_click_resets_to_default() {
        for (name, split, at, expected) in [
            ("explorer", EXPLORER, 412., 260.),
            ("explorer", EXPLORER, 180., 260.),
            ("bottom", BOTTOM, 500., 240.),
            ("bottom", BOTTOM, 240., 240.),
        ] {
            let mut split = Split { size: px(at), ..split };
            split.reset();
            assert_eq!(split.size, px(expected), "{name} double-click at {at}");
        }
    }
}
