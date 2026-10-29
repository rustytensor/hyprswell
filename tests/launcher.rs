use hyprswell::{grid::{self, Animation, Look, Position}, motion::{self, InputRect, LayoutConfig, Rect}, rules};

fn near(actual: f64, expected: f64) { assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}"); }
fn fixture() -> LayoutConfig {
    motion::reflection_layout(LayoutConfig { surface_width: 200.0, rest_width: 40.0, gap: 8.0,
        side_padding: 10.0, bar_y: 64.0, bar_height: 56.0, rest_bottom: 104.0 }, true)
}

#[test]
fn grid_positions_and_fixed_frame() {
    let pins = ["A".into(), "B".into()];
    for (position, order, x) in [
        (Position::Left, vec!["launcher", "A", "B"], Some(32.0)),
        (Position::Right, vec!["A", "B", "launcher"], Some(128.0)),
        (Position::Center, vec!["A", "launcher", "B"], Some(80.0)),
        (Position::Off, vec!["A", "B"], None),
    ] {
        let items = grid::place_items(rules::items(&pins, &[]), position);
        assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), order);
        assert_eq!(grid::app_count(&items), 2); // Hiding a control must not change app crowding.
        let index = items.iter().position(|i| i.id == "launcher");
        let layout = grid::layout(fixture(), &vec![1.0; items.len()], index);
        if let Some(x) = x {
            // Bar74 centred at y101 => fixed grid y81; three slots give bar156 at x22.
            assert_eq!(layout.icons[index.unwrap()], Rect { x, y: 81.0, width: 40.0, height: 40.0 });
            assert_eq!(layout.bar, Rect { x: 22.0, y: 64.0, width: 156.0, height: 74.0 });
        } else {
            assert_eq!(index, None);
            // Two apps: 80+8+20=108, centred at x46; no grid slot/input remains.
            assert_eq!(layout.input_region(), [InputRect { x: 46, y: 64, width: 108, height: 74 }]);
        }
    }
    let hover = grid::layout(fixture(), &[1.6, 1.0, 1.0], Some(0));
    assert_eq!(hover.icons[0].width, 40.0, "grid must not inherit the swell");
    assert_eq!(hover.icons[0], Rect { x: 32.0, y: 81.0, width: 40.0, height: 40.0 });
    // Neighbour grows40->64: centred bar widens24, so grid x shifts left12. y/w/h stay fixed.
    let neighbour = grid::layout(fixture(), &[1.0, 1.6, 1.0], Some(0));
    assert_eq!(neighbour.icons[0], Rect { x: 20.0, y: 81.0, width: 40.0, height: 40.0 });
    let reflected = grid::reflection(hover.icons[0]);
    assert_eq!(reflected.visible, Rect { x: 32.0, y: 121.0, width: 40.0, height: 18.0 });
    assert_eq!(reflected.stops, [(0.0, 0.55), (1.0, 0.0)]);
    let plain = LayoutConfig { bar_height: 56.0, ..fixture() };
    near(grid::layout(plain, &[1.6, 1.0, 1.0], Some(0)).icons[0].y, 72.0);
    let odd = grid::place_items(rules::items(&["A".into(), "B".into(), "C".into()], &[]), Position::Center);
    assert_eq!(odd.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["A", "B", "launcher", "C"]);
}

#[test]
fn grid_dots_and_animation() {
    let rect = Rect { x: 32.0, y: 81.0, width: 40.0, height: 40.0 };
    let (dots, radius) = grid::dots(rect, Look::default());
    // Centre52,101; rest step10, radius2.5. Nine literal points, no scaled button image.
    assert_eq!(dots, [(42.0,91.0),(52.0,91.0),(62.0,91.0),
        (42.0,101.0),(52.0,101.0),(62.0,101.0),(42.0,111.0),(52.0,111.0),(62.0,111.0)]);
    near(radius, 2.5);
    let (hover, r) = grid::dots(rect, Look { spacing: 1.2, accent: 1.0 });
    assert_eq!(hover, [(40.0,89.0),(52.0,89.0),(64.0,89.0),
        (40.0,101.0),(52.0,101.0),(64.0,101.0),(40.0,113.0),(52.0,113.0),(64.0,113.0)]);
    near(r, 2.5);
    let (pressed, _) = grid::dots(rect, Look { spacing: 0.6, accent: 1.0 });
    assert_eq!(pressed, [(46.0,95.0),(52.0,95.0),(58.0,95.0),
        (46.0,101.0),(52.0,101.0),(58.0,101.0),(46.0,107.0),(52.0,107.0),(58.0,107.0)]);
    let mut animation = Animation::default();
    assert!(!animation.sample(0.0).active);
    animation.set_hover(true, 0.0);
    let halfway = animation.sample(0.075);
    // Cubic ease-out at half duration: 1-(1/2)^3=.875 => step11.75.
    near(halfway.look.spacing * 10.0, 11.75);
    near(halfway.look.accent, 0.875);
    assert!(!animation.sample(0.15).active);
    animation.press(0.15);
    near(animation.sample(0.15).look.spacing * 10.0, 6.0);
    animation.release(0.15, true);
    // Existing spring at140ms: target12 + (12-6)/7.5 =12.8.
    near(animation.sample(0.29).look.spacing * 10.0, 12.8);
    let done = animation.sample(0.5);
    near(done.look.spacing * 10.0, 12.0);
    assert!(!done.active && done.launch);
    assert!(!animation.sample(0.6).launch); // One launch decision per accepted click.
    animation.set_hover(false, 0.6);
    near(animation.sample(0.675).look.spacing * 10.0, 10.25);
    assert!(!animation.sample(0.75).active);
}

#[test]
fn grid_preferences() {
    assert_eq!(grid::position(""), Position::Left);
    assert_eq!(grid::position("LAUNCHER=unknown"), Position::Left);
    assert_eq!(grid::position("LAUNCHER=left\n LAUNCHER = center \n"), Position::Center);
    assert_eq!(grid::position("LAUNCHER=right"), Position::Right);
    assert_eq!(grid::position("LAUNCHER=off"), Position::Off);
}
