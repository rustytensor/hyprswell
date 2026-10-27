#[path = "../src/motion.rs"]
mod motion;

use motion::*;

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

fn fixture() -> LayoutConfig {
    LayoutConfig {
        surface_width: 200.0, rest_width: 40.0, gap: 8.0,
        side_padding: 10.0, bar_y: 64.0, bar_height: 56.0, rest_bottom: 104.0,
    }
}

#[test]
fn cosine_at_hand_worked_distances() {
    // cos(2pi/5)=(sqrt(5)-1)/4; cos(4pi/5)=-(sqrt(5)+1)/4.
    // Substitute in 1+0.3*(1+cos); beyond 2.5 widths the scale is exactly 1.
    for (distance, expected) in [
        (0.0, 1.6), (1.0, 1.3927050983124842),
        (2.0, 1.0572949016875157), (2.5, 1.0), (3.0, 1.0),
    ] {
        near(swell_scale(distance), expected);
        near(swell_scale(-distance), expected);
    }
    // Pointer at 60, fixed centres 20/60/100, width 40: distances 1/0/1.
    let scales = pointer_scales(&[20.0, 60.0, 100.0], Some(60.0), 40.0);
    for (actual, expected) in scales.into_iter().zip([1.3927050983124842, 1.6, 1.3927050983124842]) {
        near(actual, expected);
    }
    // No pointer gives three rest scales, regardless of centre positions.
    assert_eq!(pointer_scales(&[20.0, 60.0, 100.0], None, 40.0), vec![1.0; 3]);
}

#[test]
fn packed_icons_and_exact_click_through_region() {
    let layout = fixture().layout(&[1.0, 1.6, 1.0]);
    // 40+64+40+2*8=160 content; +20 padding=180, centred at x=10.
    assert_eq!(layout.bar, Rect { x: 10.0, y: 64.0, width: 180.0, height: 56.0 });
    // Centre rise=20*0.6=12; bottom=104-12=92; top=92-64=28.
    assert_eq!(layout.icons, vec![
        Rect { x: 20.0, y: 64.0, width: 40.0, height: 40.0 },
        Rect { x: 68.0, y: 28.0, width: 64.0, height: 64.0 },
        Rect { x: 140.0, y: 64.0, width: 40.0, height: 40.0 },
    ]);
    // Only y=28..64 of the centre icon protrudes: height 36.
    let region = layout.input_region();
    assert_eq!(region, vec![
        InputRect { x: 10, y: 64, width: 180, height: 56 },
        InputRect { x: 68, y: 28, width: 64, height: 36 },
    ]);
    let contains = |x, y| region.iter().any(|r| {
        x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
    });
    assert!(contains(70, 30));
    assert!(contains(15, 70));
    assert!(!contains(15, 30));
    assert!(!contains(150, 30));
    // Rest width=120+16+20=156; (200-156)/2=22. No icon protrudes.
    assert_eq!(fixture().layout(&[1.0; 3]).input_region(), vec![
        InputRect { x: 22, y: 64, width: 156, height: 56 },
    ]);
    // Legacy rise of 4 leaves widths unchanged; middle top becomes 60.
    let plain = fixture().no_swell(3, Some(1), 4.0);
    assert_eq!(plain.icons[1], Rect { x: 80.0, y: 60.0, width: 40.0, height: 40.0 });
    assert_eq!(plain.bar.height, 56.0); // Reservation stays the bar height.
    // Bounds 1.25..3.75 and 2.5..5.25 round outward to 1..4 and 2..6.
    assert_eq!(Rect { x: 1.25, y: 2.5, width: 2.5, height: 2.75 }.outward(),
        InputRect { x: 1, y: 2, width: 3, height: 4 });
}

#[test]
fn leave_spring_and_idle_deadline() {
    let spring = Spring::new(1.6, 0.0, 1.0);
    // a=ln(7.5)/.14, w=pi/.14. At .1 the phase is 5pi/7.
    // At .14: sin=0, cos=-1, decay=1/7.5 => 1-.6/7.5=.92.
    // At .35: phase=5pi/2, cos=0, sin=1 => residual .0024980571343722.
    for (seconds, expected) in [
        (0.0, 1.6), (0.100, 0.9826359544035586),
        (0.140, 0.92), (0.350, 1.0024980571343722),
    ] {
        near(spring.raw(seconds).value, expected);
    }
    near(spring.raw(0.0).velocity, 0.0);
    assert!(spring.sample(0.140).active);
    // The animation, unlike raw maths, snaps at the agreed 350 ms deadline.
    assert_eq!(spring.sample(0.350), Sample { value: 1.0, velocity: 0.0, active: false });
    assert_eq!(spring.sample(1.0), Sample { value: 1.0, velocity: 0.0, active: false });
    // New initial value/velocity are preserved: no reset to 1.6 on retarget.
    let retargeted = Spring::new(1.2, -0.4, 1.4).sample(0.0);
    near(retargeted.value, 1.2);
    near(retargeted.velocity, -0.4);
}

#[test]
fn click_multiplier_and_return() {
    // Half cosine: cos(0)=1, cos(pi/2)=0, cos(pi)=-1 => 1, .95, .9.
    near(click_squash(0.0).value, 1.0);
    near(click_squash(0.045).value, 0.95);
    near(click_squash(0.090).value, 0.9);
    near(click_squash(0.0).velocity, 0.0);
    near(click_squash(0.090).velocity, 0.0);
    // Swell 1.6 times .9 is 1.44, not absolute .9.
    near(1.6 * click_squash(0.090).value, 1.44);
    // 140 ms into return: initial offset -.1 changes to +.1/7.5.
    near(click_squash(0.230).value, 1.0133333333333334);
    // 90 ms squash + 350 ms return = 440 ms; no idle tick survives.
    assert_eq!(click_squash(0.440), Sample { value: 1.0, velocity: 0.0, active: false });
}
