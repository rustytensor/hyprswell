// Marc's approved design change (2026-10-27), calculated before changing code:
// alpha .55 -> 0, midpoint .275; visible height = .45*h, E = ceil(.45*R).
// R40/B104: rest band (80,104,40,18), midpoint113, end122.
// At1.6: h64,r12; band (68,116,64,28.8), midpoint130.4, end144.8.
// Flip origins remain144/180. Fixture E18: bar74, bottom138, surface146,
// reservation82. (90,140) is reflection-only and must pass input through.
// R44: E20, bar87, reservation95, surface161. Off remains bar67/reservation75.
// These replace the prior design expectations, not a failing implementation's output.
use hyprswell::{motion::*, rules};

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

fn fixture() -> LayoutConfig {
    LayoutConfig { surface_width: 200.0, rest_width: 40.0, gap: 8.0, side_padding: 10.0,
        bar_y: 64.0, bar_height: 56.0, rest_bottom: 104.0 }
}

#[test]
fn reflection_maths_rectangles_and_gradients() {
    let config = reflection_layout(fixture(), true);
    let rest = config.layout(&[1.0; 3]);
    let grown = config.layout(&[1.0, 1.6, 1.0]);
    let a = reflection(rest.icons[1], config.rest_bottom);
    let b = reflection(grown.icons[1], config.rest_bottom);
    // Mirror about B=104. Rest bottom=104; swollen bottom=92, hence rise=12.
    assert_eq!(a.image, Rect { x: 80.0, y: 104.0, width: 40.0, height: 40.0 });
    assert_eq!(a.visible, Rect { x: 80.0, y: 104.0, width: 40.0, height: 18.0 });
    assert_eq!(b.image, Rect { x: 68.0, y: 116.0, width: 64.0, height: 64.0 }, "reflection follows rise downward");
    assert_eq!(b.visible, Rect { x: 68.0, y: 116.0, width: 64.0, height: 28.8 });
    // y' = flip_y - y: the original bottom reaches the reflected top.
    near(a.flip_y, 144.0);
    near(b.flip_y, 180.0);
    near(a.flip_y - 40.0, 104.0);
    near(b.flip_y - 64.0, 116.0);
    for r in [a, b] { assert_eq!(r.stops, [(0.0, 0.55), (1.0, 0.0)]); }
    // Half of .55 is .275. Half-band positions: 104+9=113, 116+14.4=130.4.
    near(a.visible.y + a.visible.height / 2.0, 113.0);
    near(b.visible.y + b.visible.height / 2.0, 130.4);
    near((a.stops[0].1 + a.stops[1].1) / 2.0, 0.275);
    near((b.stops[0].1 + b.stops[1].1) / 2.0, 0.275);
    near(a.visible.y + a.visible.height, 122.0);
    near(b.visible.y + b.visible.height, 144.8);
}

#[test]
fn reflection_maths_lane_and_input() {
    let config = reflection_layout(fixture(), true);
    // ceil(40*.45)=18 extra pixels. Bar56+18=74; +gap8 reserves82.
    near(config.bar_height, 74.0);
    near(config.bar_height.ceil() + 8.0, 82.0);
    near(config.bar_y + config.bar_height + 8.0, 146.0);
    let rest = config.layout(&[1.0; 3]);
    let grown = config.layout(&[1.0, 1.6, 1.0]);
    assert_eq!(shelf(&rest, 104.0, 10.0), Rect { x: 32.0, y: 104.0, width: 136.0, height: 1.0 });
    assert_eq!(shelf(&grown, 104.0, 10.0), Rect { x: 20.0, y: 104.0, width: 160.0, height: 1.0 });
    assert_eq!(rest.input_region(), [InputRect { x: 22, y: 64, width: 156, height: 74 }]);
    let region = grown.input_region();
    assert_eq!(region, [InputRect { x: 10, y: 64, width: 180, height: 74 },
                       InputRect { x: 68, y: 28, width: 64, height: 36 }]);
    // (90,140) lies inside the reflection but below the bar's bottom138.
    assert!(!region.iter().any(|r| 90 >= r.x && 90 < r.x + r.width && 140 >= r.y && 140 < r.y + r.height));
    let unshrunk = LayoutConfig { rest_width: 44.0, bar_height: 67.0, bar_y: 66.0,
        rest_bottom: 116.0, ..fixture() };
    // Default unshrunk icons: ceil(44*.45)=20, bar67+20=87; reservation95.
    let enabled = reflection_layout(unshrunk, true);
    near(enabled.bar_height, 87.0);
    near(enabled.bar_height.ceil() + 8.0, 95.0);
    near(enabled.bar_y + enabled.bar_height + 8.0, 161.0);
    let disabled = reflection_layout(unshrunk, false);
    near(disabled.bar_height, 67.0);
    near(disabled.bar_height.ceil() + 8.0, 75.0);
}

#[test]
fn reflection_preferences() {
    assert!(rules::reflections_enabled("", true));
    assert!(!rules::reflections_enabled("REFLECTIONS=0\n", true));
    assert!(!rules::reflections_enabled("REFLECTIONS=1\n", false));
    // Invalid values use the documented default; last occurrence wins.
    assert!(rules::reflections_enabled("REFLECTIONS=0\n REFLECTIONS = invalid\n", true));
}

#[test]
fn startup_keeps_pending_fallback_at_expiry() {
    let fallback = rules::ScreenSelection { names: vec!["Virtual-1".into()], fallback: true };
    let mut pending = None;
    // Configured target absent before the deadline: wait with Virtual-1 retained.
    assert!(rules::startup_selection(Some(fallback.clone()), &mut pending, true, false).is_none());
    // The deadline expires even if the final IPC read failed: Virtual-1 still wins over no dock.
    assert_eq!(rules::startup_selection(None, &mut pending, true, true), Some(fallback.clone()));
    assert!(pending.is_none());
    // With no requested descriptions, default selection is immediate, not delayed ten seconds.
    assert_eq!(rules::startup_selection(Some(fallback), &mut pending, false, false).unwrap().names, ["Virtual-1"]);
    let matched = rules::ScreenSelection { names: vec!["DP-1".into(), "DP-3".into()], fallback: false };
    assert_eq!(rules::startup_selection(Some(matched.clone()), &mut pending, true, false), Some(matched));
}
