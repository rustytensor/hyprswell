use crate::{home_path, log, view::{DockView, Icon, Scene}};
use gio::prelude::*;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use hyprswell::{desktop, ipc, motion, pins, rules};
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
    refresh_icons: bool,
}

pub fn start(app: &gtk::Application, outputs: &[String], swell: bool) {
    if !app.windows().is_empty() { return; }
    let Some(display) = gtk::gdk::Display::default() else { log("no GTK display"); return; };
    if !gtk4_layer_shell::is_supported() { log("Wayland layer shell is unavailable"); app.quit(); return; }
    let config = home_path("XDG_CONFIG_HOME", ".config");
    let pins_path = home_path("XDG_CACHE_HOME", ".cache").join("nwg-dock-pinned");
    let config_path = config.join("nwg-dock-hyprland/hyprswell.conf");
    let size = rules::icon_size(&std::fs::read_to_string(&config_path).unwrap_or_default());
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
        window.set_margin(Edge::Bottom, 8);
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
        docks.push(Dock { window, view, name_window, name_label, icons: vec![], hovered: None,
            pointer: None, scales: vec![], velocities: vec![], leaving: None, clicks: HashMap::new(),
            launches: HashMap::new(), rectangles: vec![], ticking: false, last_tick: Instant::now() });
    }
    if docks.is_empty() { log("no matching GTK screens, no dock"); app.quit(); return; }
    let state = Rc::new(RefCell::new(State { docks, snapshot: ipc::Snapshot::default(), pending: None,
        menu_count: 0, items: vec![], signature: vec![], pinned: pins::load(&pins_path).unwrap_or_default(),
        pins_path, config_path, size, swell, refresh_icons: true }));
    for index in 0..state.borrow().docks.len() { connect_input(&state, index); }
    refresh(&state);
    let windows: Vec<_> = state.borrow().docks.iter().map(|dock| dock.window.clone()).collect();
    for window in windows { window.present(); }
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
        while let Ok(update) = updates.try_recv() {
            match update {
                Ok(snapshot) => { latest = Some(snapshot); last_error.clear(); }
                Err(error) if error != last_error => { log(&error); last_error = error; }
                Err(_) => {}
            }
        }
        if let Some(snapshot) = latest {
            let mut s = state.borrow_mut();
            if s.menu_count > 0 { s.pending = Some(snapshot); }
            else { s.snapshot = snapshot; drop(s); refresh(&state); }
        }
        glib::ControlFlow::Continue
    });
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
    app.connect_shutdown(move |_| {
        for watch in &watches { watch.cancel(); }
        let _ = &app_monitor;
        for dock in &state.borrow().docks { dock.name_window.close(); }
    });
}

fn load_css(css: &gtk::CssProvider, path: &std::path::Path) {
    css.load_from_string(&std::fs::read_to_string(path)
        .unwrap_or_else(|_| include_str!("../style.example.css").into()));
}

fn refresh(state: &Shared) {
    let mut s = state.borrow_mut();
    if s.menu_count > 0 { return; }
    let (pinned, size) = if s.refresh_icons {
        (pins::load(&s.pins_path).unwrap_or_else(|_| s.pinned.clone()),
         rules::icon_size(&std::fs::read_to_string(&s.config_path).unwrap_or_default()))
    } else { (s.pinned.clone(), s.size) };
    let items = rules::items(&pinned, &s.snapshot.clients);
    let signature: Signature = items.iter().map(|item| (item.id.clone(), rules::indicator(&item.windows),
        item.windows.iter().any(|c| c.address == s.snapshot.active))).collect();
    let rebuild = signature != s.signature || size != s.size || s.refresh_icons;
    let old_ids: Vec<_> = s.items.iter().map(|i| i.id.clone()).collect();
    s.items = items;
    s.signature = signature;
    s.pinned = pinned;
    s.size = size;
    if rebuild {
        s.refresh_icons = false;
        let drawn = rules::drawn_size(size, s.items.len().saturating_sub(1)).max(1);
        let lookup = rules::icon_lookup_size(drawn);
        let icons: Vec<_> = s.items.iter().map(|item| {
            let display = s.docks[0].view.display();
            let theme = gtk::IconTheme::for_display(&display);
            let scale = s.docks[0].view.scale_factor();
            let info = if item.id == "launcher" { None } else { desktop::app_info(&item.id) };
            let paintable = if let Some(icon) = info.and_then(|a| a.icon()) {
                theme.lookup_by_gicon(&icon, lookup, scale, gtk::TextDirection::None, gtk::IconLookupFlags::empty())
            } else {
                let name = if item.id == "launcher" { "view-app-grid-symbolic" }
                    else if theme.has_icon(&item.id) { &item.id } else { "image-missing" };
                theme.lookup_icon(name, &[], lookup, scale, gtk::TextDirection::None, gtk::IconLookupFlags::empty())
            };
            Icon { paintable: paintable.upcast(), indicator: rules::indicator(&item.windows),
                active: item.windows.iter().any(|c| c.address == s.snapshot.active), shadow: false, bounce: 0.0 }
        }).collect();
        let ids: Vec<_> = s.items.iter().map(|i| i.id.clone()).collect();
        for dock in &mut s.docks {
            let old: HashMap<_, _> = old_ids.iter().cloned().zip(dock.scales.iter().copied()
                .zip(dock.velocities.iter().copied())).collect();
            dock.scales = ids.iter().map(|id| old.get(id).map_or(1.0, |s| s.0)).collect();
            dock.velocities = ids.iter().map(|id| old.get(id).map_or(0.0, |s| s.1)).collect();
            dock.icons = icons.clone();
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

fn layout_config(size: u32, count: usize) -> (motion::LayoutConfig, i32) {
    let rest = rules::drawn_size(size, count.saturating_sub(1)).max(1) as f64 * 0.8;
    let bar_height = rest + 23.0;
    let height = (rest * 2.2 + 36.0).ceil() as i32;
    // Keep a fixed transparent surface; only the painted bar changes width.
    let width = (count as f64 * rest * 1.8 + count.saturating_sub(1) as f64 * 8.0 + 20.0).ceil();
    (motion::LayoutConfig { surface_width: width, rest_width: rest, gap: 8.0, side_padding: 10.0,
        bar_y: height as f64 - bar_height, bar_height, rest_bottom: height as f64 - 17.0 }, height)
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
    let (config, height) = layout_config(s.size, s.items.len());
    let ids: Vec<_> = s.items.iter().map(|item| item.id.clone()).collect();
    let swell = s.swell;
    let dock = &mut s.docks[index];
    let dt = now.duration_since(dock.last_tick).as_secs_f64().min(0.1);
    dock.last_tick = now;
    let mut active = false;
    if swell {
        if let Some((x, _)) = dock.pointer {
            dock.leaving = None;
            let rest = config.layout(&vec![1.0; ids.len()]);
            let centres: Vec<_> = rest.icons.iter().map(|r| r.x + r.width / 2.0).collect();
            let targets = motion::pointer_scales(&centres, Some(x), config.rest_width);
            for (i, target) in targets.into_iter().enumerate() {
                let next = target + (dock.scales[i] - target) * (-dt / 0.045).exp();
                if (next - target).abs() < 0.0001 { dock.scales[i] = target; dock.velocities[i] = 0.0; }
                else { dock.velocities[i] = (target - next) / 0.045; dock.scales[i] = next; active = true; }
            }
        } else if let Some((start, springs)) = &dock.leaving {
            for (i, spring) in springs.iter().enumerate() {
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
        if let Some(start) = dock.clicks.get(id) {
            let sample = motion::click_squash(now.duration_since(*start).as_secs_f64());
            scales[i] *= sample.value;
            active |= sample.active;
        }
        dock.icons[i].shadow = swell && dock.hovered.as_ref() == Some(id);
        dock.icons[i].bounce = 0.0;
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
    let mut layout = config.layout(&scales);
    if !swell {
        if let Some(i) = ids.iter().position(|id| dock.hovered.as_ref() == Some(id)) { layout.icons[i].y -= 6.0; }
    }
    dock.rectangles = layout.icons.clone();
    dock.window.set_exclusive_zone(config.bar_height.ceil() as i32);
    dock.view.update(Scene { width: config.surface_width as i32, height, layout,
        icons: dock.icons.clone(), indicator_y: config.rest_bottom + 6.0 });
    update_name(&s, index);
    active
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
            {
                let mut s = state.borrow_mut();
                let dock = &mut s.docks[index];
                dock.pointer = None;
                dock.hovered = None;
                dock.name_window.set_visible(false);
                begin_leave(dock);
            }
            wake(&state, index);
        }
    });
    view.add_controller(controller);
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let weak = Rc::downgrade(state);
    click.connect_pressed(move |_, _, x, y| {
        if let Some(state) = weak.upgrade() {
            let mut s = state.borrow_mut();
            if let Some(i) = hit(&s.docks[index], x, y) {
                let id = s.items[i].id.clone();
                s.docks[index].clicks.insert(id, Instant::now());
                drop(s);
                wake(&state, index);
            }
        }
    });
    let weak = Rc::downgrade(state);
    click.connect_released(move |click, _, x, y| {
        if let Some(state) = weak.upgrade() {
            let s = state.borrow();
            let Some(i) = hit(&s.docks[index], x, y) else { return; };
            let item = s.items[i].clone();
            let active = s.snapshot.active.clone();
            drop(s);
            match click.current_button() {
                1 | 2 if item.id == "launcher" => launch_launcher(),
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
    view.add_controller(click);
    let weak = Rc::downgrade(state);
    view.connect_map(move |_| { if let Some(state) = weak.upgrade() { wake(&state, index); } });
}

fn pointer(state: &Shared, index: usize, x: f64, y: f64) {
    let mut s = state.borrow_mut();
    let hovered = hit(&s.docks[index], x, y).map(|i| s.items[i].id.clone());
    s.docks[index].pointer = Some((x, y));
    s.docks[index].hovered = hovered;
    drop(s);
    wake(state, index);
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
        dock.name_window.set_margin(Edge::Bottom, 8 + dock.view.height() - rect.y.floor() as i32 + 6);
        if !dock.name_window.is_visible() { dock.name_window.present(); }
    }
}

fn popover(state: &Shared, index: usize, item: usize) -> (gtk::Popover, gtk::Box) {
    let menu = gtk::Popover::new();
    menu.set_position(gtk::PositionType::Top);
    {
        let mut s = state.borrow_mut();
        s.menu_count += 1;
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
            }
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
