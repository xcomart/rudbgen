//! Manual CPU-frame measurements for real dialog trees, excluding GPU presentation.

use gpui::{Render, TestAppContext, VisualTestContext, WindowHandle, px, size};

pub(crate) fn measure<V: Render>(name: &str, window: WindowHandle<V>, cx: &mut TestAppContext) {
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    for (width, height) in [(1200., 900.), (900., 600.)] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.run_until_parked();
        let mut times = Vec::with_capacity(80);
        for frame in 0..85 {
            let start = std::time::Instant::now();
            cx.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
            if frame >= 5 {
                times.push(start.elapsed().as_micros());
            }
        }
        times.sort_unstable();
        eprintln!(
            "{name} CPU frame ({width}x{height}, microseconds): p50={}, p95={}, max={}",
            times[40], times[76], times[79]
        );
    }
}
