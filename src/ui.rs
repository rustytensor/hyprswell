use crate::{home_path, log, view::{DockView, Icon, Scene}};
use gio::prelude::*;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use hyprswell::{autohide::{self, Zone}, control, desktop, grid, ipc, motion, pins, rules};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

type Shared = Rc<RefCell<State>>;
type Signature = Vec<(String, rules::Indicator, bool)>;

struct Dock {
    output: String,
    window: gtk::ApplicationWindow,
    view: DockView,
    name_window: gtk::ApplicationWindow,
    name_label: gtk::Label,
    icons: Vec<Icon>,
    hovered: Option<String>,
    pointer: Option<(f64, f64)>,
    scales: Vec<f64>,
    velocities: Vec<f64>,
    leaving: Option<(Instant, Vec<motion::Spring>)>,
    clicks: HashMap<String, Instant>,
    launches: HashMap<String, Instant>,
    rectangles: Vec<motion::Rect>,
    ticking: bool,
    last_tick: Instant,
    grid_animation: grid::Animation,
    grid_epoch: Instant,
    autohide: autohide::Controller,
    timer: Option<(f64, glib::SourceId)>,
    zone: Option<i32>,
    rest_bar: motion::Rect,
    surface_height: f64,
}

struct State {
    docks: Vec<Dock>,
    snapshot: ipc::Snapshot,
    pending: Option<ipc::Snapshot>,
    menu_count: usize,
    items: Vec<rules::DockItem>,
    signature: Signature,
    pinned: Vec<String>,
    pins_path: PathBuf,
    config_path: PathBuf,
    size: u32,
    swell: bool,
    reflections: bool,
    reflections_allowed: bool,
    launcher: grid::Position,
    refresh_icons: bool,
    autohide: bool,
    epoch: Instant,
    acks: Vec<std::os::unix::net::UnixStream>,
    ack_watch: bool,
    endpoint: Option<control::Endpoint>,
}

impl State {
    fn clock(&self) -> f64 { self.epoch.elapsed().as_secs_f64() }
}

pub fn start(app: &gtk::Application, outputs: &[String], swell: bool, reflections_allowed: bool,
             control_path: Option<PathBuf>) {
    if !app.windows().is_empty() { return; }
    let Some(display) = gtk::gdk::Display::default() else { log("no GTK display"); return; };
    if !gtk4_layer_shell::is_supported() { log("Wayland layer shell is unavailable"); app.quit(); return; }
    let config = home_path("XDG_CONFIG_HOME", ".config");
    let pins_path = home_path("XDG_CACHE_HOME", ".cache").join("nwg-dock-pinned");
    let config_path = config.join("nwg-dock-hyprland/hyprswell.conf");
    let preferences = std::fs::read_to_string(&config_path).unwrap_or_default();
    let size = rules::icon_size(&preferences);
    let reflections = rules::reflections_enabled(&preferences, reflections_allowed);
    let launcher = grid::position(&preferences);
    let hide = autohide::enabled(&preferences);
    let css = gtk::CssProvider::new();
    let style_path = config.join("hyprswell/style.css");
    load_css(&css, &style_path);
    gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    let mut docks = Vec::new();
    let monitors = display.monitors();
    for output in outputs {
        let monitor = (0..monitors.n_items()).filter_map(|n| monitors.item(n))
            .filter_map(|m| m.downcast::<gtk::gdk::Monitor>().ok())
            .find(|m| m.connector().as_deref() == Some(output.as_str()));
        let Some(monitor) = monitor else { log(&format!("GTK cannot find screen {output}")); continue; };
        let window = gtk::ApplicationWindow::builder().application(app).css_classes(["dock-window"]).build();
        window.set_default_size(1, 1);
        window.init_layer_shell();
        window.set_namespace(Some("hyprswell"));
        window.set_layer(Layer::Top);
        window.set_monitor(Some(&monitor));
        window.set_anchor(Edge::Bottom, true);
        // The canvas includes the existing 8px bottom gap, with no input in that tail.
        window.set_margin(Edge::Bottom, 0);
        let view = DockView::new();
        window.set_child(Some(&view));
        let name_window = gtk::ApplicationWindow::builder().application(app)
            .css_classes(["dock-name-window"]).build();
        name_window.set_default_size(1, 1);
        name_window.init_layer_shell();
        name_window.set_namespace(Some("hyprswell-name"));
        name_window.set_layer(Layer::Overlay);
        name_window.set_monitor(Some(&monitor));
        name_window.set_anchor(Edge::Bottom, true);
        name_window.set_anchor(Edge::Left, true);
        name_window.set_exclusive_zone(-1);
        let name_label = gtk::Label::new(None);
        name_label.add_css_class("dock-name");
        name_window.set_child(Some(&name_label));
        name_window.connect_map(|window| {
            if let Some(surface) = window.surface() { surface.set_input_region(&gtk::cairo::Region::create()); }
        });
        docks.push(Dock { output: output.clone(), window, view, name_window, name_label, icons: vec![], hovered: None,
            pointer: None, scales: vec![], velocities: vec![], leaving: None, clicks: HashMap::new(),
            launches: HashMap::new(), rectangles: vec![], ticking: false, last_tick: Instant::now(),
            grid_animation: grid::Animation::default(), grid_epoch: Instant::now(),
            autohide: autohide::Controller::new(hide, 0.0), timer: None, zone: None,
            rest_bar: motion::Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 }, surface_height: 0.0 });
    }
    if docks.is_empty() { log("no matching GTK screens, no dock"); app.quit(); return; }
    let state = Rc::new(RefCell::new(State { docks, snapshot: ipc::Snapshot::default(), pending: None,
        menu_count: 0, items: vec![], signature: vec![], pinned: pins::load(&pins_path).unwrap_or_default(),
        pins_path, config_path, size, swell, reflections, reflections_allowed, launcher, refresh_icons: true,
        autohide: hide, epoch: Instant::now(), acks: vec![], ack_watch: false, endpoint: None }));
    for index in 0..state.borrow().docks.len() { connect_input(&state, index); }
    refresh(&state);
    let windows: Vec<_> = state.borrow().docks.iter().map(|dock| dock.window.clone()).collect();
    for window in windows { window.present(); }
    // A sleeping screen is unplugged: its dock would move to whichever screen is left (10-28: both docks on the
    // ARZOPA). Hide a dock while its screen is gone and put it back when the screen returns.
    let weak = Rc::downgrade(&state);
    display.monitors().connect_items_changed(move |_, _, _, _| {
        for delay in [500, 2500] {   // a returning screen can get its connector name a moment later
            let weak = weak.clone();
            glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                if let Some(state) = weak.upgrade() { place(&state); }
            });
        }
    });
    let (updates, mut signal) = match ipc::updates() {
        Ok(updates) => updates,
        Err(error) => { log(&format!("event notification: {error}")); app.quit(); return; }
    };
    let weak = Rc::downgrade(&state);
    let mut last_error = String::new();
    glib::source::unix_fd_add_local(signal.as_raw_fd(), glib::IOCondition::IN | glib::IOCondition::HUP, move |_, _| {
        let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break; };
        let mut bytes = [0; 64];
        loop {
            match signal.read(&mut bytes) {
                Ok(0) => return glib::ControlFlow::Break,
                Ok(_) => {},
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => return glib::ControlFlow::Break,
            }
        }
        let mut latest = None;
        let mut attention = false;
        while let Ok(update) = updates.try_recv() {
            match update {
                ipc::Update::Attention => attention = true,
                ipc::Update::Snapshot(Ok(snapshot)) => { latest = Some(snapshot); last_error.clear(); }
                ipc::Update::Snapshot(Err(error)) if error != last_error => { log(&error); last_error = error; }
                ipc::Update::Snapshot(Err(_)) => {}
            }
        }
        // Attention reveals available docks; it never focuses or moves a window.
        if attention { show_docks(&state, None); }
        if let Some(snapshot) = latest {
            let mut s = state.borrow_mut();
            if s.menu_count > 0 { s.pending = Some(snapshot); }
            else { s.snapshot = snapshot; drop(s); refresh(&state); }
        }
        glib::ControlFlow::Continue
    });
    if let Some(path) = control_path {
        match control::Server::bind(&path).and_then(control::Server::serve) {
            Ok((requests, mut signal, endpoint)) => {
                state.borrow_mut().endpoint = Some(endpoint);
                let weak = Rc::downgrade(&state);
                glib::source::unix_fd_add_local(signal.as_raw_fd(), glib::IOCondition::IN | glib::IOCondition::HUP, move |_, _| {
                    let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break; };
                    let mut bytes = [0; 64];
                    loop {
                        match signal.read(&mut bytes) {
                            Ok(0) => return glib::ControlFlow::Break,
                            Ok(_) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(_) => return glib::ControlFlow::Break,
                        }
                    }
                    while let Ok(stream) = requests.try_recv() { show_docks(&state, Some(stream)); }
                    glib::ControlFlow::Continue
                });
            }
            Err(error) => log(&format!("control socket {}: {error}", path.display())),
        }
    }
    let mut watches = Vec::new();
    if let Ok(watch) = gio::File::for_path(&style_path).monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
        let weak = Rc::downgrade(&state);
        watch.connect_changed(move |_, _, _, _| {
            load_css(&css, &style_path);
            if let Some(state) = weak.upgrade() {
                for dock in &state.borrow().docks { dock.view.queue_draw(); }
            }
        });
        watches.push(watch);
    }
    for path in [&state.borrow().pins_path, &state.borrow().config_path] {
        if let Ok(watch) = gio::File::for_path(path).monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
            let weak = Rc::downgrade(&state);
            watch.connect_changed(move |_, _, _, _| {
                if let Some(state) = weak.upgrade() {
                    state.borrow_mut().refresh_icons = true;
                    refresh(&state);
                }
            });
            watches.push(watch);
        }
    }
    let app_monitor = gio::AppInfoMonitor::get();
    let weak = Rc::downgrade(&state);
    app_monitor.connect_changed(move |_| {
        desktop::invalidate();
        if let Some(state) = weak.upgrade() {
            state.borrow_mut().refresh_icons = true;
            refresh(&state);
        }
    });
    // Quit through GTK on SIGTERM/SIGINT so shutdown unlinks the control endpoint.
    for signal in [libc::SIGTERM, libc::SIGINT] {
        let app = app.downgrade();
        glib::source::unix_signal_add_local_once(signal, move || { if let Some(app) = app.upgrade() { app.quit(); } });
    }
    app.connect_shutdown(move |_| {
        for watch in &watches { watch.cancel(); }
        let _ = &app_monitor;
        let mut s = state.borrow_mut();
        drop(s.endpoint.take());
        for dock in &s.docks { dock.name_window.close(); }
    });
}

fn place(state: &Shared) {
    let Some(display) = gtk::gdk::Display::default() else { return; };
    let monitors = display.monitors();
    // Clone the windows first: showing one runs GTK handlers that borrow the state.
    let docks: Vec<_> = state.borrow().docks.iter()
        .map(|d| (d.output.clone(), d.window.clone(), d.name_window.clone())).collect();
    for (output, window, name_window) in docks {
        let monitor = (0..monitors.n_items()).filter_map(|n| monitors.item(n))
            .filter_map(|m| m.downcast::<gtk::gdk::Monitor>().ok())
            .find(|m| m.is_valid() && m.connector().as_deref() == Some(output.as_str()));
        match monitor {
            None if window.is_visible() => {
                log(&format!("screen {output} gone, dock hidden"));
                window.set_visible(false);
                name_window.set_visible(false);
            }
            Some(monitor) if !window.is_visible() || window.monitor().as_ref() != Some(&monitor) => {
                log(&format!("screen {output} back, dock shown"));
                window.set_visible(false);
                name_window.set_visible(false);
                window.set_monitor(Some(&monitor));
                name_window.set_monitor(Some(&monitor));
                window.present();
            }
            _ => {}
        }
    }
}

fn load_css(css: &gtk::CssProvider, path: &std::path::Path) {
    css.load_from_string(&std::fs::read_to_string(path)
        .unwrap_or_else(|_| include_str!("../style.example.css").into()));
}

fn refresh(state: &Shared) {
    let mut s = state.borrow_mut();
    if s.menu_count > 0 { return; }
    let (pinned, size, reflections, launcher, hide) = if s.refresh_icons {
        let preferences = std::fs::read_to_string(&s.config_path).unwrap_or_default();
        (pins::load(&s.pins_path).unwrap_or_else(|_| s.pinned.clone()),
         rules::icon_size(&preferences), rules::reflections_enabled(&preferences, s.reflections_allowed),
         grid::position(&preferences), autohide::enabled(&preferences))
    } else { (s.pinned.clone(), s.size, s.reflections, s.launcher, s.autohide) };
    // The Settings switch is applied live, without a restart that would unreserve and re-reserve.
    if hide != s.autohide {
        s.autohide = hide;
        let now = s.clock();
        for dock in &mut s.docks { dock.autohide.set_enabled(hide, now); }
    }
    let items = grid::place_items(rules::items(&pinned, &s.snapshot.clients), launcher);
    let signature: Signature = items.iter().map(|item| (item.id.clone(), rules::indicator(&item.windows),
        item.windows.iter().any(|c| c.address == s.snapshot.active))).collect();
    let rebuild = signature != s.signature || size != s.size || reflections != s.reflections || launcher != s.launcher || s.refresh_icons;
    let old_launcher = s.launcher;
    let old_ids: Vec<_> = s.items.iter().map(|i| i.id.clone()).collect();
    s.items = items;
    s.signature = signature;
    s.pinned = pinned;
    s.size = size;
    s.reflections = reflections;
    s.launcher = launcher;
    if rebuild {
        s.refresh_icons = false;
        let drawn = rules::drawn_size(size, grid::app_count(&s.items)).max(1);
        let lookup = rules::icon_lookup_size(drawn);
        let icons: Vec<_> = s.items.iter().map(|item| {
            let display = s.docks[0].view.display();
            let theme = gtk::IconTheme::for_display(&display);
            let scale = s.docks[0].view.scale_factor();
            let paintable = if item.id == "launcher" { None } else {
                let image = if let Some(icon) = desktop::app_info(&item.id).and_then(|a| a.icon()) {
                    theme.lookup_by_gicon(&icon, lookup, scale, gtk::TextDirection::None, gtk::IconLookupFlags::empty())
                } else {
                    let name = if theme.has_icon(&item.id) { &item.id } else { "image-missing" };
                    theme.lookup_icon(name, &[], lookup, scale, gtk::TextDirection::None, gtk::IconLookupFlags::empty())
                };
                Some(image.upcast())
            };
            Icon { paintable, grid: (item.id == "launcher").then_some(grid::Look::default()), indicator: rules::indicator(&item.windows),
                active: item.windows.iter().any(|c| c.address == s.snapshot.active), shadow: false, bounce: 0.0 }
        }).collect();
        let ids: Vec<_> = s.items.iter().map(|i| i.id.clone()).collect();
        for dock in &mut s.docks {
            let old: HashMap<_, _> = old_ids.iter().cloned().zip(dock.scales.iter().copied()
                .zip(dock.velocities.iter().copied())).collect();
            dock.scales = ids.iter().map(|id| old.get(id).map_or(1.0, |s| s.0)).collect();
            dock.velocities = ids.iter().map(|id| old.get(id).map_or(0.0, |s| s.1)).collect();
            dock.icons = icons.clone();
            if launcher != old_launcher || launcher == grid::Position::Off {
                dock.grid_animation = grid::Animation::default();
                dock.grid_epoch = Instant::now();
                if dock.hovered.as_deref() == Some("launcher") { dock.hovered = None; }
            }
            if dock.hovered.as_ref().is_some_and(|id| !ids.contains(id)) { dock.hovered = None; }
            if dock.pointer.is_none() { begin_leave(dock); }
        }
    }
    // Title/workspace text is live, but unchanged icons never get rebuilt.
    for index in 0..s.docks.len() { update_name(&s, index); }
    drop(s);
    if rebuild {
        let count = state.borrow().docks.len();
        for index in 0..count {
            frame(state, index);
            wake(state, index);
        }
    }
}

fn begin_leave(dock: &mut Dock) {
    dock.leaving = Some((Instant::now(), dock.scales.iter().zip(&dock.velocities)
        .map(|(&value, &velocity)| motion::Spring::new(value, velocity, 1.0)).collect()));
}

fn wake(state: &Shared, index: usize) {
    let view = {
        let mut s = state.borrow_mut();
        let dock = &mut s.docks[index];
        if dock.ticking { return; }
        dock.ticking = true;
        dock.last_tick = Instant::now() - Duration::from_millis(16);
        dock.view.clone()
    };
    let weak = Rc::downgrade(state);
    view.add_tick_callback(move |_, _| {
        let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break; };
        if frame(&state, index) { glib::ControlFlow::Continue }
        else { state.borrow_mut().docks[index].ticking = false; glib::ControlFlow::Break }
    });
}

fn frame(state: &Shared, index: usize) -> bool {
    let now = Instant::now();
    let mut s = state.borrow_mut();
    let reflections = s.reflections;
    let (config, height) = autohide::dock_layout(s.size, s.items.len(), grid::app_count(&s.items), reflections);
    let ids: Vec<_> = s.items.iter().map(|item| item.id.clone()).collect();
    let grid_index = ids.iter().position(|id| id == "launcher");
    let swell = s.swell;
    let hide = s.autohide;
    let clock = s.clock();
    let running = autohide::running(&s.items);
    let dock = &mut s.docks[index];
    let dt = now.duration_since(dock.last_tick).as_secs_f64().min(0.1);
    dock.last_tick = now;
    dock.autohide.distance = autohide::hidden_offset(height as f64, config.bar_y);
    dock.autohide.advance(clock);
    let hidden = dock.autohide.settled_hidden();
    if hidden {
        // Nothing but the lip is visible: no invisible swell, grid or launch animation.
        dock.pointer = None;
        dock.hovered = None;
        dock.leaving = None;
        dock.clicks.clear();
        dock.launches.clear();
        dock.scales.fill(1.0);
        dock.velocities.fill(0.0);
        dock.grid_animation = grid::Animation::default();
    }
    let mut active = false;
    if swell {
        if let Some((x, _)) = dock.pointer {
            dock.leaving = None;
            let rest = grid::layout(config, &vec![1.0; ids.len()], grid_index);
            let centres: Vec<_> = rest.icons.iter().map(|r| r.x + r.width / 2.0).collect();
            let targets = motion::pointer_scales(&centres, Some(x), config.rest_width);
            for (i, target) in targets.into_iter().enumerate() {
                if Some(i) == grid_index { dock.scales[i] = 1.0; dock.velocities[i] = 0.0; continue; }
                let next = target + (dock.scales[i] - target) * (-dt / 0.045).exp();
                if (next - target).abs() < 0.0001 { dock.scales[i] = target; dock.velocities[i] = 0.0; }
                else { dock.velocities[i] = (target - next) / 0.045; dock.scales[i] = next; active = true; }
            }
        } else if let Some((start, springs)) = &dock.leaving {
            for (i, spring) in springs.iter().enumerate() {
                if Some(i) == grid_index { dock.scales[i] = 1.0; dock.velocities[i] = 0.0; continue; }
                let sample = spring.sample(now.duration_since(*start).as_secs_f64());
                dock.scales[i] = sample.value;
                dock.velocities[i] = sample.velocity;
                active |= sample.active;
            }
            if !active { dock.leaving = None; }
        }
    } else {
        dock.scales.fill(1.0);
        dock.velocities.fill(0.0);
        dock.leaving = None;
    }
    let mut scales = dock.scales.clone();
    for (i, id) in ids.iter().enumerate() {
        dock.icons[i].bounce = 0.0;
        if Some(i) == grid_index { continue; }
        if let Some(start) = dock.clicks.get(id) {
            let sample = motion::click_squash(now.duration_since(*start).as_secs_f64());
            scales[i] *= sample.value;
            active |= sample.active;
        }
        if let Some(start) = dock.launches.get(id) {
            let t = now.duration_since(*start).as_secs_f64();
            if t < 2.0 {
                let phase = (t % 0.7) / 0.42;
                dock.icons[i].bounce = if phase < 1.0 { 12.0 * (std::f64::consts::PI * phase).sin() } else { 0.0 };
                active = true;
            }
        }
    }
    dock.clicks.retain(|_, start| now.duration_since(*start).as_secs_f64() < 0.44);
    dock.launches.retain(|_, start| now.duration_since(*start).as_secs_f64() < 2.0);
    let mut layout = grid::layout(config, &scales, grid_index);
    if !swell {
        if let Some(i) = ids.iter().position(|id| dock.hovered.as_ref() == Some(id)) {
            if Some(i) != grid_index { layout.icons[i].y -= 6.0; }
        }
    }
    let rest = grid::layout(config, &vec![1.0; ids.len()], grid_index);
    dock.rest_bar = rest.bar;
    dock.surface_height = height as f64;
    let mut shown_input = layout.input_region();
    // Launch bounce also draws above the normal icon rectangle.
    for (icon, rect) in dock.icons.iter().zip(&layout.icons) {
        if icon.bounce > 0.0 { shown_input.push(motion::Rect { y: rect.y - icon.bounce, ..*rect }.outward()); }
    }
    let width = config.surface_width as i32;
    let lip = autohide::lip_region(rest.bar, height as f64);
    let input = dock.autohide.input_region(clock, &shown_input, lip, width, height);
    // Drawing, hit tests and names all use the translated geometry while the dock moves.
    let offset = dock.autohide.offset(clock);
    let layout = autohide::translate(&layout, offset);
    active |= dock.autohide.animating();
    let grid_hovered = grid_index.is_some_and(|i| dock.pointer.is_some_and(|(x, y)| {
        let r = layout.icons[i];
        x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
    }));
    if grid_hovered && dock.hovered.as_deref() != Some("launcher") { dock.hovered = Some("launcher".into()); }
    else if !grid_hovered && dock.hovered.as_deref() == Some("launcher") { dock.hovered = None; }
    let time = now.duration_since(dock.grid_epoch).as_secs_f64();
    dock.grid_animation.set_hover(grid_hovered, time);
    let control = dock.grid_animation.sample(time);
    active |= control.active;
    for (i, icon) in dock.icons.iter_mut().enumerate() {
        icon.grid = (Some(i) == grid_index).then_some(control.look);
        icon.shadow = swell && Some(i) != grid_index && dock.hovered.as_ref() == Some(&ids[i]);
    }
    dock.rectangles = layout.icons.clone();
    let shelf = reflections.then(|| motion::shelf(&layout, config.rest_bottom + offset, config.side_padding));
    // Set only when the required value changes; the slide never touches the reservation.
    let zone = autohide::exclusive_zone(hide, config.bar_height);
    if dock.zone != Some(zone) {
        dock.window.set_exclusive_zone(zone);
        dock.zone = Some(zone);
    }
    let lip_dots = if hidden { autohide::lip_dots(&rest, &running, height as f64) } else { vec![] };
    dock.view.update(Scene { width, height, layout, shelf, input, hidden, lip_dots, icons: dock.icons.clone(),
        indicator_y: config.rest_bottom + offset + motion::reflection_padding(config.rest_width, reflections) + 6.0 });
    update_name(&s, index);
    let menu = s.menu_count > 0;
    let dock = &mut s.docks[index];
    let label = dock.name_window.is_visible();
    dock.autohide.set_blockers(menu, label, clock);
    active |= dock.autohide.animating();
    drop(s);
    schedule(state, index);
    if control.launch { launch_launcher(); }
    active
}

/// One one-shot timer per dock for its next dwell or hide deadline; nothing polls while waiting.
fn schedule(state: &Shared, index: usize) {
    let mut s = state.borrow_mut();
    let now = s.clock();
    let dock = &mut s.docks[index];
    let deadline = dock.autohide.next_deadline();
    if dock.timer.as_ref().map(|(at, _)| *at) == deadline { return; }
    if let Some((_, source)) = dock.timer.take() { source.remove(); }
    let Some(at) = deadline else { return; };
    let delay = Duration::from_micros(((at - now).max(0.0) * 1e6).ceil() as u64);
    let weak = Rc::downgrade(state);
    let source = glib::timeout_add_local_once(delay, move || {
        let Some(state) = weak.upgrade() else { return; };
        let moving = {
            let mut s = state.borrow_mut();
            let now = s.clock();
            let dock = &mut s.docks[index];
            dock.timer = None; // This source is finished; it must not be removed again.
            dock.autohide.advance(now.max(at));
            dock.autohide.animating()
        };
        schedule(&state, index);
        if moving { wake(&state, index); }
    });
    dock.timer = Some((at, source));
}

/// The activation band counts only while it takes input, and only under the rest bar's span.
fn zone_at(dock: &Dock, x: f64, y: f64) -> Zone {
    let bar = dock.rest_bar;
    let band = dock.autohide.band_input() && y >= dock.surface_height - autohide::LIP
        && x >= bar.x && x < bar.x + bar.width;
    if band { Zone::Band } else { Zone::Dock }
}

/// Keyboard, control socket and attention requests reveal every available dock; asleep outputs stay hidden.
fn show_docks(state: &Shared, ack: Option<std::os::unix::net::UnixStream>) {
    let visible: Vec<usize> = {
        let mut s = state.borrow_mut();
        let now = s.clock();
        let visible: Vec<_> = (0..s.docks.len()).filter(|&i| s.docks[i].window.is_visible()).collect();
        for &i in &visible { s.docks[i].autohide.show(now); }
        if let Some(stream) = ack { s.acks.push(stream); }
        visible
    };
    for &i in &visible { schedule(state, i); wake(state, i); }
    let Some(&first) = visible.first() else { reply(state, "none"); return; };
    if state.borrow().acks.is_empty() { return; }
    if std::mem::replace(&mut state.borrow_mut().ack_watch, true) { return; }
    // Acknowledge after a frame has been painted with every available dock settled shown.
    let view = state.borrow().docks[first].view.clone();
    let weak = Rc::downgrade(state);
    let started = Instant::now();
    let settled = std::cell::Cell::new(0);
    view.add_tick_callback(move |_, _| {
        let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break; };
        let all = {
            let s = state.borrow();
            s.docks.iter().filter(|d| d.window.is_visible()).all(|d| d.autohide.settled_shown())
        };
        settled.set(if all { settled.get() + 1 } else { 0 });
        let timeout = started.elapsed() > Duration::from_secs(3);
        if settled.get() < 2 && !timeout { return glib::ControlFlow::Continue; }
        state.borrow_mut().ack_watch = false;
        reply(&state, if timeout { "timeout" } else { control::SHOWN });
        glib::ControlFlow::Break
    });
}

fn reply(state: &Shared, text: &str) {
    let acks = std::mem::take(&mut state.borrow_mut().acks);
    for mut stream in acks {
        use std::io::Write;
        let _ = stream.set_nonblocking(true);
        let _ = stream.write_all(format!("{text}\n").as_bytes());
    }
}

fn hit(dock: &Dock, x: f64, y: f64) -> Option<usize> {
    dock.rectangles.iter().enumerate().find_map(|(i, r)| {
        let top = r.y - dock.icons[i].bounce;
        (x >= r.x && x < r.x + r.width && y >= top && y < top + r.height).then_some(i)
    })
}

fn connect_input(state: &Shared, index: usize) {
    let view = state.borrow().docks[index].view.clone();
    let controller = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(state);
    controller.connect_motion(move |_, x, y| {
        if let Some(state) = weak.upgrade() { pointer(&state, index, x, y); }
    });
    let weak = Rc::downgrade(state);
    controller.connect_enter(move |_, x, y| {
        if let Some(state) = weak.upgrade() { pointer(&state, index, x, y); }
    });
    let weak = Rc::downgrade(state);
    controller.connect_leave(move |_| {
        if let Some(state) = weak.upgrade() {
            let quiet = {
                let mut s = state.borrow_mut();
                let now = s.clock();
                let menu = s.menu_count > 0;
                let dock = &mut s.docks[index];
                dock.pointer = None;
                dock.hovered = None;
                dock.grid_animation.set_hover(false, dock.grid_epoch.elapsed().as_secs_f64());
                dock.name_window.set_visible(false);
                begin_leave(dock);
                dock.autohide.set_blockers(menu, false, now);
                dock.autohide.pointer(Zone::Outside, now);
                dock.autohide.settled_hidden()
            };
            schedule(&state, index);
            if !quiet { wake(&state, index); }
        }
    });
    view.add_controller(controller);
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let weak = Rc::downgrade(state);
    click.connect_pressed(move |gesture, _, x, y| {
        if let Some(state) = weak.upgrade() {
            let mut s = state.borrow_mut();
            // The activation band only reveals: it never launches, focuses or minimizes.
            if zone_at(&s.docks[index], x, y) == Zone::Band || s.docks[index].autohide.settled_hidden() { return; }
            if let Some(i) = hit(&s.docks[index], x, y) {
                let id = s.items[i].id.clone();
                if id == "launcher" {
                    if matches!(gesture.current_button(), 1 | 2) {
                        let dock = &mut s.docks[index];
                        dock.grid_animation.press(dock.grid_epoch.elapsed().as_secs_f64());
                    }
                } else { s.docks[index].clicks.insert(id, Instant::now()); }
                drop(s);
                wake(&state, index);
            }
        }
    });
    let weak = Rc::downgrade(state);
    click.connect_released(move |click, _, x, y| {
        if let Some(state) = weak.upgrade() {
            let mut s = state.borrow_mut();
            let band = zone_at(&s.docks[index], x, y) == Zone::Band || s.docks[index].autohide.settled_hidden();
            let hit_index = if band { None } else { hit(&s.docks[index], x, y) };
            if s.docks[index].grid_animation.is_pressed() {
                let activate = hit_index.is_some_and(|i| s.items[i].id == "launcher")
                    && matches!(click.current_button(), 1 | 2);
                let dock = &mut s.docks[index];
                dock.grid_animation.release(dock.grid_epoch.elapsed().as_secs_f64(), activate);
                drop(s);
                wake(&state, index);
                return;
            }
            let Some(i) = hit_index else { return; };
            let item = s.items[i].clone();
            let active = s.snapshot.active.clone();
            drop(s);
            match click.current_button() {
                1 | 2 if item.id == "launcher" => {},
                1 => {
                    let action = rules::left_click(&item.id, &item.windows, &active);
                    if matches!(action, rules::Action::Menu(_)) { window_menu(&state, index, i, &item.windows); }
                    else { perform(&action, &state, index); }
                }
                2 => perform(&rules::Action::Launch(item.id), &state, index),
                3 if item.id != "launcher" => context_menu(&state, index, i, &item),
                _ => {}
            }
        }
    });
    let weak = Rc::downgrade(state);
    click.connect_cancel(move |_, _| { if let Some(state) = weak.upgrade() { cancel_grid(&state, index); } });
    view.add_controller(click);
    let weak = Rc::downgrade(state);
    view.connect_map(move |_| { if let Some(state) = weak.upgrade() { wake(&state, index); } });
}

fn cancel_grid(state: &Shared, index: usize) {
    let mut s = state.borrow_mut();
    let dock = &mut s.docks[index];
    if !dock.grid_animation.is_pressed() { return; }
    dock.grid_animation.release(dock.grid_epoch.elapsed().as_secs_f64(), false);
    drop(s);
    wake(state, index);
}

fn pointer(state: &Shared, index: usize, x: f64, y: f64) {
    let mut s = state.borrow_mut();
    let now = s.clock();
    let zone = zone_at(&s.docks[index], x, y);
    // A held activation band feeds no position into icon or grid hover.
    let hovered = if zone == Zone::Band { None } else { hit(&s.docks[index], x, y).map(|i| s.items[i].id.clone()) };
    let dock = &mut s.docks[index];
    let was_hidden = dock.autohide.settled_hidden();
    dock.autohide.pointer(zone, now);
    let quiet = was_hidden && dock.autohide.settled_hidden();
    if zone == Zone::Band {
        if dock.pointer.is_some() { begin_leave(dock); }
        dock.pointer = None;
    } else { dock.pointer = Some((x, y)); }
    dock.grid_animation.set_hover(hovered.as_deref() == Some("launcher"), dock.grid_epoch.elapsed().as_secs_f64());
    dock.hovered = hovered;
    drop(s);
    schedule(state, index);
    // No frame work during the dwell itself: the hidden dock looks the same until its timer fires.
    if !quiet { wake(state, index); }
}

fn update_name(s: &State, index: usize) {
    let dock = &s.docks[index];
    if s.menu_count > 0 { dock.name_window.set_visible(false); return; }
    let Some(i) = s.items.iter().position(|item| Some(&item.id) == dock.hovered.as_ref()) else {
        dock.name_window.set_visible(false); return;
    };
    let Some(rect) = dock.rectangles.get(i) else { return; };
    let item = &s.items[i];
    let name = if item.id == "launcher" { "Applications".into() }
        else { desktop::app_info(&item.id).map(|a| a.display_name().to_string()).unwrap_or_else(|| item.id.clone()) };
    let text = rules::hover_text(&name, &item.windows);
    if dock.name_label.text() != text { dock.name_label.set_text(&text); }
    let (_, natural, _, _) = dock.name_label.measure(gtk::Orientation::Horizontal, -1);
    if let Some(monitor) = dock.window.monitor() {
        let x = (monitor.geometry().width() - dock.view.width()) / 2
            + (rect.x + rect.width / 2.0) as i32 - natural / 2;
        dock.name_window.set_margin(Edge::Left, x.max(0).min((monitor.geometry().width() - natural).max(0)));
        // The lower grid must not put its label over neighbouring application icons.
        let top = if item.id == "launcher" { dock.rectangles.iter().fold(rect.y, |top, r| top.min(r.y)) } else { rect.y };
        dock.name_window.set_margin(Edge::Bottom, dock.view.height() - top.floor() as i32 + 6);
        if !dock.name_window.is_visible() { dock.name_window.present(); }
    }
}

fn popover(state: &Shared, index: usize, item: usize) -> (gtk::Popover, gtk::Box) {
    let menu = gtk::Popover::new();
    menu.set_position(gtk::PositionType::Top);
    {
        let mut s = state.borrow_mut();
        s.menu_count += 1;
        let now = s.clock();
        for dock in &mut s.docks { dock.autohide.set_blockers(true, false, now); }
        let dock = &s.docks[index];
        let rect = dock.rectangles[item].outward();
        menu.set_parent(&dock.view);
        menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(rect.x, rect.y, rect.width, rect.height)));
        dock.name_window.set_visible(false);
    }
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.add_css_class("dock-menu");
    menu.set_child(Some(&content));
    let weak = Rc::downgrade(state);
    menu.connect_closed(move |menu| {
        menu.unparent();
        if let Some(state) = weak.upgrade() {
            {
                let mut s = state.borrow_mut();
                s.menu_count = s.menu_count.saturating_sub(1);
                if s.menu_count == 0 {
                    if let Some(snapshot) = s.pending.take() { s.snapshot = snapshot; }
                }
                let (now, menu) = (s.clock(), s.menu_count > 0);
                for dock in &mut s.docks {
                    let label = dock.name_window.is_visible();
                    dock.autohide.set_blockers(menu, label, now);
                }
            }
            let count = state.borrow().docks.len();
            for i in 0..count { schedule(&state, i); }
            refresh(&state);
        }
    });
    (menu, content)
}

fn window_menu(state: &Shared, index: usize, item: usize, windows: &[rules::Client]) {
    let (menu, content) = popover(state, index, item);
    for window in windows {
        let row = gtk::Button::with_label(&rules::window_line(window));
        let action = rules::menu_pick(window);
        let menu_weak = menu.downgrade();
        let weak = Rc::downgrade(state);
        row.connect_clicked(move |_| {
            if let Some(menu) = menu_weak.upgrade() { menu.popdown(); }
            if let Some(state) = weak.upgrade() { perform(&action, &state, index); }
        });
        content.append(&row);
    }
    menu.popup();
}

fn context_menu(state: &Shared, index: usize, item_index: usize, item: &rules::DockItem) {
    let (menu, content) = popover(state, index, item_index);
    for window in &item.windows {
        let submenu = gtk::MenuButton::builder().label(rules::window_line(window)).build();
        let choices = gtk::Popover::new();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        choices.set_child(Some(&list));
        submenu.set_popover(Some(&choices));
        let mut actions = vec![("Close".to_string(), ipc::WindowAction::Close),
            ("Float/tile".into(), ipc::WindowAction::Float), ("Full screen".into(), ipc::WindowAction::Fullscreen)];
        for n in 1..=9 { actions.push((format!("Move to workspace {n}"), ipc::WindowAction::Move(n))); }
        for (label, action) in actions {
            let row = gtk::Button::with_label(&label);
            let address = window.address.clone();
            let menu_weak = menu.downgrade();
            row.connect_clicked(move |_| {
                if let Some(menu) = menu_weak.upgrade() { menu.popdown(); }
                dispatch(&address, action);
            });
            list.append(&row);
        }
        content.append(&submenu);
    }
    let pinned = state.borrow().pinned.contains(&item.id);
    let row = gtk::Button::with_label(if pinned { "Unpin" } else { "Pin" });
    let weak = Rc::downgrade(state);
    let id = item.id.clone();
    let menu_weak = menu.downgrade();
    row.connect_clicked(move |_| {
        if let Some(menu) = menu_weak.upgrade() { menu.popdown(); }
        if let Some(state) = weak.upgrade() {
            {
                let s = state.borrow();
                let mut next = pins::load(&s.pins_path).unwrap_or_else(|_| s.pinned.clone());
                if pinned { pins::unpin(&mut next, &id); } else { pins::pin(&mut next, &id); }
                if let Err(error) = pins::save(&s.pins_path, &next) { log(&format!("save pins: {error}")); return; }
            }
            state.borrow_mut().refresh_icons = true;
            refresh(&state);
        }
    });
    content.append(&row);
    menu.popup();
}

fn launch_launcher() {
    std::thread::spawn(|| {
        match std::process::Command::new("fuzzel").spawn() {
            Ok(mut child) => { let _ = child.wait(); }
            Err(error) => log(&format!("launcher: {error}")),
        }
    });
}

fn perform(action: &rules::Action, state: &Shared, index: usize) {
    match action {
        rules::Action::Launch(id) => {
            if let Some(info) = desktop::app_info(id) {
                if let Err(error) = info.launch(&[], gio::AppLaunchContext::NONE) { log(&format!("launch failed: {error}")); return; }
                state.borrow_mut().docks[index].launches.insert(id.clone(), Instant::now());
                wake(state, index);
            } else { log("cannot launch app: no desktop entry"); }
        }
        rules::Action::Focus(address) => dispatch(address, ipc::WindowAction::Focus),
        rules::Action::Minimize(address) | rules::Action::Restore(address) => {
            let arg = if matches!(action, rules::Action::Minimize(_)) { "--minimize" } else { "--restore" };
            let address = address.clone();
            std::thread::spawn(move || {
                match std::process::Command::new("hypr-minimize").args([arg, &address]).status() {
                    Ok(status) if status.success() => {}
                    Ok(_) => log("hypr-minimize failed"),
                    Err(error) => log(&format!("hypr-minimize: {error}")),
                }
            });
        }
        rules::Action::Menu(_) => {}
    }
}

fn dispatch(address: &str, action: ipc::WindowAction) {
    let commands = match ipc::commands(address, action) {
        Ok(commands) => commands,
        Err(error) => { log(&error.to_string()); return; }
    };
    std::thread::spawn(move || {
        for command in commands {
            match ipc::request(&command) {
                Ok(reply) if reply.trim() == "ok" => {}
                Ok(_) => { log("compositor rejected window action"); break; }
                Err(error) => { log(&format!("window action: {error}")); break; }
            }
        }
    });
}
