//! Auto-hide gate: every expected value below was worked out by hand in report 8 before implementation.
use hyprswell::{autohide::{self, Controller, Pose, Zone}, control, grid::{self, Position}, ipc,
    motion::{self, InputRect, LayoutConfig, Rect}, rules::{self, Client, Workspace}};

const TOLERANCE: f64 = 1e-6;
fn near(actual: f64, expected: f64) { assert!((actual - expected).abs() < TOLERANCE, "{actual} != {expected}"); }

/// The 200 px fixture: R40, gap8, padding10, accepted 45% reflection band: bar (22,64,156,74), surface 146.
fn fixture() -> LayoutConfig {
    motion::reflection_layout(LayoutConfig { surface_width: 200.0, rest_width: 40.0, gap: 8.0,
        side_padding: 10.0, bar_y: 64.0, bar_height: 56.0, rest_bottom: 104.0 }, true)
}
const SURFACE: f64 = 146.0;

fn client(address: &str, class: &str, workspace: &str) -> Client {
    Client { address: address.into(), class: class.into(), title: String::new(),
        workspace: Workspace { id: 1, name: workspace.into() } }
}

/// A runs one window, B is minimized, C has two windows: three running apps, LAUNCHER=off.
fn apps() -> Vec<Client> {
    vec![client("0xa", "A", "1"), client("0xb", "B", "special:minimized"), client("0xc1", "C", "1"), client("0xc2", "C", "1")]
}

#[test]
fn a_hidden_lip_is_exactly_six_pixels() {
    let config = fixture();
    // Exact hidden height first: the planted fault (lip height 0) must fail here, before any dock starts.
    let offset = autohide::hidden_offset(SURFACE, config.bar_y);
    assert_eq!(SURFACE - (config.bar_y + offset), 6.0, "hidden height must be exactly 6 px");
    assert_eq!(offset, 76.0);
    assert_eq!(config.bar_y + config.bar_height + 8.0, SURFACE);
    // Screen bottom S: surface top S-146 in both poses; visible bar top S-82 shown, S-6 hidden.
    assert_eq!(SURFACE - config.bar_y, 82.0);
    assert_eq!(SURFACE - (config.bar_y + offset), 6.0);
}

#[test]
fn geometry_dots_and_zones() {
    let items = grid::place_items(rules::items(&[], &apps()), Position::Off);
    assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["A", "B", "C"]);
    let rest = grid::layout(fixture(), &[1.0; 3], None);
    assert_eq!(rest.bar, Rect { x: 22.0, y: 64.0, width: 156.0, height: 74.0 });
    assert_eq!(autohide::lip_dots(&rest, &autohide::running(&items), SURFACE), [(52.0, 143.0), (100.0, 143.0), (148.0, 143.0)]);
    assert_eq!(autohide::DOT_RADIUS, 1.0);
    // Grid and pinned-but-not-running D give no dot: G,D,A,B,C on a 296 px surface, bar x22, centres 52..244.
    let mixed = grid::place_items(rules::items(&["D".into()], &apps()), Position::Left);
    assert_eq!(mixed.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["launcher", "D", "A", "B", "C"]);
    let wide = grid::layout(LayoutConfig { surface_width: 296.0, ..fixture() }, &[1.0; 5], Some(0));
    assert_eq!(autohide::lip_dots(&wide, &autohide::running(&mixed), SURFACE), [(148.0, 143.0), (196.0, 143.0), (244.0, 143.0)]);
    // Exclusive zone: 6 with auto-hide in every pose, 82 (bar74 + gap8) without.
    assert_eq!(autohide::exclusive_zone(true, 74.0), 6);
    assert_eq!(autohide::exclusive_zone(false, 74.0), 82);
    // Reflections off: bar56, surface128, hidden offset58; reservations 64 / 6.
    let plain = LayoutConfig { bar_height: 56.0, ..fixture() };
    assert_eq!(autohide::hidden_offset(128.0, plain.bar_y), 58.0);
    assert_eq!(autohide::exclusive_zone(false, plain.bar_height), 64);
    assert_eq!(autohide::exclusive_zone(true, plain.bar_height), 6);
    // Unshrunk ICON_SIZE=44: hidden offset 89, reservation 95 / 6; reflections off 69, 75 / 6.
    for (reflections, offset, normal) in [(true, 89.0, 95), (false, 69.0, 75)] {
        let (config, height) = autohide::dock_layout(44, 3, 3, reflections);
        assert_eq!(autohide::hidden_offset(height as f64, config.bar_y), offset);
        assert_eq!(autohide::exclusive_zone(false, config.bar_height), normal);
        assert_eq!(autohide::exclusive_zone(true, config.bar_height), 6);
    }
}

#[test]
fn input_regions() {
    let rest = grid::layout(fixture(), &[1.0; 3], None);
    let lip = autohide::lip_region(rest.bar, SURFACE);
    assert_eq!(lip, InputRect { x: 22, y: 140, width: 156, height: 6 });
    let strip = autohide::trigger_strip(rest.bar, SURFACE);
    assert_eq!(strip, InputRect { x: 22, y: 144, width: 156, height: 2 });
    assert!(strip.x >= lip.x && strip.y >= lip.y && strip.x + strip.width <= lip.x + lip.width
        && strip.y + strip.height <= lip.y + lip.height, "trigger strip must lie inside the lip");
    let shown = rest.input_region();
    assert_eq!(shown, [InputRect { x: 22, y: 64, width: 156, height: 74 }]);
    let swollen = grid::layout(fixture(), &[1.0, 1.6, 1.0], None).input_region();
    assert_eq!(swollen, [InputRect { x: 10, y: 64, width: 180, height: 74 }, InputRect { x: 68, y: 28, width: 64, height: 36 }]);
    // The fully translated bar clips to the lip.
    assert_eq!(autohide::clip(InputRect { x: 22, y: 140, width: 156, height: 74 }, 200, 146), Some(lip));

    let mut dock = Controller::new(true, 76.0);
    assert_eq!(dock.input_region(0.0, &shown, lip, 200, 146), [lip]);
    // R2 B: revealed from the band, the band stays while the pointer stays in it, then goes.
    dock.pointer(Zone::Band, 0.0);
    dock.advance(0.5);
    assert!(dock.settled_shown());
    assert_eq!(dock.input_region(0.5, &shown, lip, 200, 146), [shown[0], lip]);
    dock.pointer(Zone::Dock, 0.6);
    assert_eq!(dock.input_region(0.6, &shown, lip, 200, 146), shown);
}

#[test]
fn motion_oracle() {
    for (t, expected) in [(0.0, 0.0), (0.1, 19.0), (0.2, 76.0)] { near(autohide::hide_sample(0.0, 76.0, t).0, expected); }
    // R1 A: a = ln(12.5)/0.14, w = pi/0.14, from 76 with velocity 0.
    near(autohide::reveal_damping(), 12.5_f64.ln() / 0.14);
    for (t, expected) in [(0.0, 76.0), (0.1, 0.063494), (0.14, -6.08), (0.35, 0.110605)] {
        near(autohide::reveal_raw(76.0, 0.0, t).0, expected);
    }
    let mut dock = Controller::new(true, 76.0);
    dock.show(0.0);
    near(dock.offset(0.14), -6.08);
    dock.advance(0.35);
    assert_eq!((dock.offset(0.35), dock.animating()), (0.0, false), "350 ms: 0, inactive");
    // Hiding interrupted at 199 ms: no jump (76*(0.199/0.2)^2 = 75.2419), and the clamp keeps the lip.
    let mut dock = Controller::new(false, 76.0);
    dock.set_enabled(true, 0.0);
    dock.advance(0.7);
    near(dock.offset(0.899), 75.2419);
    dock.pointer(Zone::Dock, 0.899);
    near(dock.offset(0.899), 75.2419);
    let highest = (0..350).map(|ms| dock.offset(0.899 + ms as f64 / 1000.0)).fold(f64::MIN, f64::max);
    assert_eq!(highest, 76.0, "unclamped this reversal would reach 78.75");
}

#[test]
fn timer_timeline() {
    // 1. Enter the lip at 0, leave at 119: no reveal at 120.
    let mut dock = Controller::new(true, 76.0);
    dock.pointer(Zone::Band, 0.0);
    assert_eq!(dock.next_deadline(), Some(0.12));
    dock.pointer(Zone::Band, 0.05); // Motion inside the band never restarts the dwell.
    assert_eq!(dock.next_deadline(), Some(0.12));
    dock.pointer(Zone::Outside, 0.119);
    assert_eq!(dock.next_deadline(), None);
    dock.advance(0.12);
    assert_eq!(dock.pose(), Pose::Hidden);
    // 2. Enter at 200 and stay: hidden through 319, reveal at 320, settled at 670; no frames during the dwell.
    dock.pointer(Zone::Band, 0.2);
    assert_eq!(dock.next_deadline(), Some(0.32));
    dock.advance(0.319);
    assert!(dock.settled_hidden() && !dock.animating());
    dock.advance(0.32);
    assert!(dock.animating());
    near(dock.offset(0.32), 76.0);
    dock.advance(0.669);
    assert!(dock.animating());
    dock.advance(0.67);
    assert!(dock.settled_shown() && dock.next_deadline().is_none());
    // 3. Leave everything at 1000: shown through 1699, hide at 1700, 19 at 1800, hidden 76 at 1900, then idle.
    dock.pointer(Zone::Outside, 1.0);
    assert_eq!(dock.next_deadline(), Some(1.7));
    dock.advance(1.699);
    assert!(dock.settled_shown());
    dock.advance(1.7);
    assert!(dock.animating());
    near(dock.offset(1.8), 19.0);
    dock.advance(1.9);
    assert_eq!((dock.offset(1.9), dock.settled_hidden(), dock.animating(), dock.next_deadline()), (76.0, true, false, None));
    // 4. Shown: leave at 0, menu opens 600, closes 900 with the label open, label closes 1000: hide at 1700.
    let mut dock = Controller::new(false, 76.0);
    dock.pointer(Zone::Dock, -1.0);
    dock.set_enabled(true, -1.0);
    dock.pointer(Zone::Outside, 0.0);
    assert_eq!(dock.next_deadline(), Some(0.7));
    dock.set_blockers(true, false, 0.6);
    assert_eq!(dock.next_deadline(), None);
    dock.set_blockers(false, true, 0.9);
    assert_eq!(dock.next_deadline(), None);
    dock.set_blockers(false, false, 1.0);
    assert_eq!(dock.next_deadline(), Some(1.7));
    dock.advance(1.699);
    assert!(dock.settled_shown());
    dock.advance(1.7);
    assert!(dock.animating());
}

#[test]
fn show_enable_and_config() {
    // Keyboard/attention: no dwell; pointer outside, so the hide delay starts when the reveal completes.
    let mut dock = Controller::new(true, 76.0);
    dock.show(2.0);
    dock.show(2.1); // A repeated request does not restart the running reveal.
    assert_eq!(dock.pose(), Pose::Revealing { start: 2.0, from: 76.0, velocity: 0.0 });
    dock.advance(2.35);
    assert_eq!(dock.next_deadline(), Some(2.35 + 0.7));
    // Disabling clears timers and reveals; enabling keeps the pose and uses the normal leave delay.
    dock.set_enabled(false, 2.5);
    assert_eq!(dock.next_deadline(), None);
    assert!(dock.settled_shown());
    dock.set_enabled(true, 3.0);
    assert!(dock.settled_shown());
    assert_eq!(dock.next_deadline(), Some(3.7));
    let mut hidden = Controller::new(true, 76.0);
    hidden.set_enabled(false, 0.0);
    assert!(hidden.animating() && hidden.next_deadline().is_none());

    assert!(!autohide::enabled(""));
    assert!(!autohide::enabled("AUTOHIDE=yes"));
    assert!(autohide::enabled(" AUTOHIDE = 1 "));
    assert!(!autohide::enabled("AUTOHIDE=1\nAUTOHIDE=0"));
    assert!(autohide::enabled("ICON_SIZE=44\nAUTOHIDE=0\nAUTOHIDE=1\n"));
}

#[test]
fn urgency_records() {
    // Local records only; nothing is broadcast into the session.
    let mut batch = ipc::Pending::default();
    batch.absorb("windowtitlev2>>55aa,New title");
    batch.absorb("urgent>>55aa");
    assert!(batch.refresh);
    assert_eq!(batch.reveal_requests(), 1);
    let mut title = ipc::Pending::default();
    title.absorb("windowtitlev2>>55aa,New title");
    assert_eq!((title.refresh, title.reveal_requests()), (true, 0));
}

#[test]
fn control_socket_ownership() {
    let base = std::path::Path::new("target/hyprswell/tmp");
    let dir = base.join(format!("control-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    // A regular file at the path is not ours to unlink.
    let file = dir.join("plain");
    std::fs::write(&file, "keep").unwrap();
    assert!(control::Server::bind(&file).is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep");
    // A live endpoint is never replaced; a missing server is an error, never a new dock.
    let path = dir.join("dock.sock");
    let (requests, _signal, endpoint) = control::Server::bind(&path).unwrap().serve().unwrap();
    assert_eq!(control::Server::bind(&path).err().map(|e| e.kind()), Some(std::io::ErrorKind::AddrInUse));
    assert!(path.exists());
    let client = { let path = path.clone(); std::thread::spawn(move || control::request_show(&path, std::time::Duration::from_secs(2))) };
    let mut stream = requests.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    std::io::Write::write_all(&mut stream, b"shown\n").unwrap();
    drop(stream);
    assert_eq!(client.join().unwrap().unwrap(), "shown");
    drop(endpoint);
    assert!(!path.exists());
    assert!(control::request_show(&path, std::time::Duration::from_secs(1)).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
