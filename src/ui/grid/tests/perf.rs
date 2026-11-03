//! Frame cost of scrolling big_events, headless. `cargo test --release grid_scroll_frame_cost -- --ignored --nocapture`

#[gpui_kit::test]
#[ignore = "timing benchmark; run on demand"]
fn grid_scroll_frame_cost(cx: &mut gpui_kit::TestAppContext) {
    use gpui_kit::{AppContext as _, Bounds, WindowBounds, WindowOptions, point, px, size};
    cx.update(gpui_kit::init);
    let bounds = Bounds::new(point(px(0.), px(0.)), size(px(1600.), px(1000.)));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    let (window, grid) = cx.update(|cx| {
        gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|cx| super::Grid::new(std::rc::Rc::new(super::BigEvents::new()), cx))
        })
        .expect("open test window")
    });
    let mut cx = gpui_kit::VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    let mut ms = Vec::new();
    for _ in 0..400 {
        let t = std::time::Instant::now();
        grid.update(&mut cx, |g, cx| g.bench_step(cx));
        cx.run_until_parked();
        ms.push(t.elapsed().as_secs_f64() * 1000.);
    }
    let s = crate::bench::frame_stats(ms.split_off(20)).unwrap();
    eprintln!(
        "GRIDFRAME p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
        s.p50_ms, s.p95_ms, s.p99_ms, s.max_ms
    );
}

#[test]
fn column_lines_break_only_at_outlined_rows() {
    use gpui_kit::{Pixels, px};
    let spans = |gaps: &[f32], bottom: f32| -> Vec<(f32, f32)> {
        let gaps = gaps.iter().map(|&g| px(g)).collect();
        super::super::rows::line_spans(gaps, px(20.), px(bottom))
            .into_iter()
            .map(|(a, b): (Pixels, Pixels)| (a.into(), b.into()))
            .collect()
    };
    // (gap row tops, bottom, runs)
    let cases: [(&[f32], f32, Vec<(f32, f32)>); 5] = [
        (&[], 100., vec![(0., 100.)]),
        (&[40.], 100., vec![(0., 40.), (60., 100.)]),
        (&[-10.], 100., vec![(10., 100.)]),
        (&[90., 0.], 100., vec![(20., 90.)]),
        (&[200.], 100., vec![(0., 100.)]),
    ];
    for (gaps, bottom, want) in cases {
        assert_eq!(spans(gaps, bottom), want, "{gaps:?}");
    }
}
