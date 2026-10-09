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
pub fn resolve_drag(size: Pixels, min: Pixels, max: Pixels, _start: Pixels) -> Drag {
    Drag::Size(size.clamp(min, max))
}

/// One panel's size and its limits. `max` takes the window extent along the split's axis.
#[derive(Clone, Copy)]
pub struct Split {
    pub size: Pixels,
    pub default: Pixels,
    pub min: Pixels,
    pub max: fn(Pixels) -> Pixels,
}

pub const EXPLORER: Split = Split { size: px(260.), default: px(260.), min: px(180.), max: |_| px(600.) };

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
}
