use crate::{home_path, log};
use gio::prelude::*;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use hyprswell::{desktop, ipc, pins, rules};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

struct Dock {
    window: gtk::ApplicationWindow,
    bar: gtk::Box,
    name_window: gtk::ApplicationWindow,
    name_label: gtk::Label,
}

struct State {
    docks: Vec<Dock>,
    snapshot: ipc::Snapshot,
    pinned: Vec<String>,
    pins_path: PathBuf,
    config_path: PathBuf,
    size: u32,
}

pub fn start(app: &gtk::Application, outputs: &[String]) {
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
        window.init_layer_shell();
        window.set_namespace(Some("hyprswell"));
        window.set_layer(Layer::Top);
        window.set_monitor(Some(&monitor));
        window.set_anchor(Edge::Bottom, true);
        window.set_margin(Edge::Bottom, 8);
        window.auto_exclusive_zone_enable();
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.add_css_class("dock-bar");
        window.set_child(Some(&bar));
        let name_window = gtk::ApplicationWindow::builder().application(app)
            .css_classes(["dock-name-window"]).build();
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
        // A hover label must not steal input from the button beneath it.
        name_window.connect_map(|window| {
            if let Some(surface) = window.surface() {
                surface.set_input_region(&gtk::cairo::Region::create());
            }
        });
        docks.push(Dock { window, bar, name_window, name_label });
    }
    if docks.is_empty() { log("no matching GTK screens, no dock"); app.quit(); return; }
    let state = Rc::new(RefCell::new(State {
        docks, snapshot: ipc::Snapshot::default(),
        pinned: pins::load(&pins_path).unwrap_or_default(), pins_path, config_path, size,
    }));
    rebuild(&state);
    for dock in &state.borrow().docks { dock.window.present(); }
    let updates = ipc::updates();
    let weak = Rc::downgrade(&state);
    let mut last_error = String::new();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break; };
        let mut changed = false;
        while let Ok(update) = updates.try_recv() {
            match update {
                Ok(snapshot) => { state.borrow_mut().snapshot = snapshot; changed = true; last_error.clear(); }
                Err(error) if error != last_error => { log(&error); last_error = error; }
                Err(_) => {}
            }
        }
        if changed { rebuild(&state); }
        glib::ControlFlow::Continue
    });
    let mut watches = Vec::new();
    if let Ok(watch) = gio::File::for_path(&style_path).monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
        let path = style_path.clone();
        watch.connect_changed(move |_, _, _, _| load_css(&css, &path));
        watches.push(watch);
    }
    for path in [&state.borrow().pins_path, &state.borrow().config_path] {
        if let Ok(watch) = gio::File::for_path(path).monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
            let weak = Rc::downgrade(&state);
            watch.connect_changed(move |_, _, _, _| {
                if let Some(state) = weak.upgrade() {
                    {
                        let mut s = state.borrow_mut();
                        s.pinned = pins::load(&s.pins_path).unwrap_or_default();
                        s.size = rules::icon_size(&std::fs::read_to_string(&s.config_path).unwrap_or_default());
                    }
                    rebuild(&state);
                }
            });
            watches.push(watch);
        }
    }
    // Keep GTK objects and file watches alive until application shutdown.
    app.connect_shutdown(move |_| {
        for watch in &watches { watch.cancel(); }
        for dock in &state.borrow().docks { dock.name_window.close(); }
    });
}

fn load_css(css: &gtk::CssProvider, path: &std::path::Path) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| include_str!("../style.example.css").into());
    css.load_from_string(&text);
}

fn rebuild(state: &Rc<RefCell<State>>) {
    let s = state.borrow();
    let items = rules::items(&s.pinned, &s.snapshot.clients);
    let count = items.len().saturating_sub(1);
    let drawn = rules::drawn_size(s.size, count) as i32;
    for dock in &s.docks {
        dock.name_window.set_visible(false);
        while let Some(child) = dock.bar.first_child() { dock.bar.remove(&child); }
        for item in &items {
            let launcher = item.id == "launcher";
            let info = if launcher { None } else { desktop::app_info(&item.id) };
            let name = if launcher { "Applications".to_string() } else {
                info.as_ref().map(|a| a.display_name().to_string()).unwrap_or_else(|| item.id.clone())
            };
            let button = gtk::Button::new();
            button.add_css_class("dock-item");
            if item.windows.iter().any(|c| c.address == s.snapshot.active) { button.add_css_class("active"); }
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let image = gtk::Image::new();
            image.add_css_class("dock-icon");
            image.set_pixel_size(drawn);
            if launcher { image.set_icon_name(Some("view-app-grid-symbolic")); }
            else if let Some(icon) = info.as_ref().and_then(|a| a.icon()) { image.set_from_gicon(&icon); }
            else {
                let theme = gtk::IconTheme::for_display(&button.display());
                image.set_icon_name(Some(if theme.has_icon(&item.id) { &item.id } else { "image-missing" }));
            }
            content.append(&image);
            let marker = match rules::indicator(&item.windows) {
                rules::Indicator::None => " ", rules::Indicator::One => "•",
                rules::Indicator::Two => "••", rules::Indicator::Hollow => "◦",
            };
            let indicator = gtk::Label::new(Some(marker));
            indicator.add_css_class("dock-indicator");
            content.append(&indicator);
            button.set_child(Some(&content));
            dock.bar.append(&button);
            let action = rules::left_click(&item.id, &item.windows, &s.snapshot.active);
            let weak = Rc::downgrade(state);
            let windows = item.windows.clone();
            button.connect_clicked(move |button| {
                if launcher { launch_launcher(); }
                else if matches!(action, rules::Action::Menu(_)) { window_menu(button, &windows); }
                else { perform(&action, Some(button)); }
                if let Some(state) = weak.upgrade() {
                    for dock in &state.borrow().docks { dock.name_window.set_visible(false); }
                }
            });
            let gesture = gtk::GestureClick::new();
            gesture.set_button(0);
            let id = item.id.clone();
            let windows = item.windows.clone();
            let weak = Rc::downgrade(state);
            let button_weak = button.downgrade();
            gesture.connect_released(move |gesture, _, _, _| {
                let Some(button) = button_weak.upgrade() else { return; };
                match gesture.current_button() {
                    2 => { if launcher { launch_launcher(); } else { launch(&id, Some(&button)); } }
                    3 if !launcher => { if let Some(state) = weak.upgrade() { context_menu(&button, &id, &windows, &state); } }
                    _ => {}
                }
            });
            button.add_controller(gesture);
            let motion = gtk::EventControllerMotion::new();
            let name_window = dock.name_window.clone();
            let name_label = dock.name_label.clone();
            let window = dock.window.clone();
            let button_weak = button.downgrade();
            let text = rules::hover_text(&name, &item.windows);
            motion.connect_enter(move |_, _, _| {
                let Some(button) = button_weak.upgrade() else { return; };
                name_label.set_text(&text);
                let (_, natural, _, _) = name_label.measure(gtk::Orientation::Horizontal, -1);
                let point = button.compute_point(&window, &gtk::graphene::Point::new(0.0, 0.0));
                let monitor = window.monitor();
                if let (Some(point), Some(monitor)) = (point, monitor) {
                    let x = (monitor.geometry().width() - window.width()) / 2
                        + point.x() as i32 + button.width() / 2 - natural / 2;
                    name_window.set_margin(Edge::Left, x.max(0));
                    name_window.set_margin(Edge::Bottom, 8 + window.height() + 6);
                    name_window.present();
                }
            });
            let name_window = dock.name_window.clone();
            motion.connect_leave(move |_| name_window.set_visible(false));
            button.add_controller(motion);
        }
    }
}

fn launch_launcher() {
    std::thread::spawn(|| {
        match std::process::Command::new("fuzzel").spawn() {
            Ok(mut child) => { let _ = child.wait(); }
            Err(error) => log(&format!("launcher: {error}")),
        }
    });
}

fn launch(id: &str, button: Option<&gtk::Button>) {
    if let Some(info) = desktop::app_info(id) {
        if let Err(error) = info.launch(&[], gio::AppLaunchContext::NONE) { log(&format!("launch failed: {error}")); return; }
        if let Some(button) = button {
            button.add_css_class("launching");
            let weak = button.downgrade();
            glib::timeout_add_local_once(Duration::from_secs(2), move || {
                if let Some(button) = weak.upgrade() { button.remove_css_class("launching"); }
            });
        }
    } else { log("cannot launch app: no desktop entry"); }
}

fn perform(action: &rules::Action, button: Option<&gtk::Button>) {
    match action {
        rules::Action::Launch(id) => launch(id, button),
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

fn popover(button: &gtk::Button) -> (gtk::Popover, gtk::Box) {
    let menu = gtk::Popover::new();
    menu.set_position(gtk::PositionType::Top);
    menu.set_parent(button);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.add_css_class("dock-menu");
    menu.set_child(Some(&content));
    menu.connect_closed(|menu| menu.unparent());
    (menu, content)
}

fn window_menu(button: &gtk::Button, windows: &[rules::Client]) {
    let (menu, content) = popover(button);
    for window in windows {
        let row = gtk::Button::with_label(&rules::window_line(window));
        let action = rules::menu_pick(window);
        let menu_weak = menu.downgrade();
        row.connect_clicked(move |_| {
            perform(&action, None);
            if let Some(menu) = menu_weak.upgrade() { menu.popdown(); }
        });
        content.append(&row);
    }
    menu.popup();
}

fn context_menu(button: &gtk::Button, id: &str, windows: &[rules::Client], state: &Rc<RefCell<State>>) {
    let (menu, content) = popover(button);
    for window in windows {
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
                dispatch(&address, action);
                if let Some(menu) = menu_weak.upgrade() { menu.popdown(); }
            });
            list.append(&row);
        }
        content.append(&submenu);
    }
    let pinned = state.borrow().pinned.iter().any(|pin| pin == id);
    let row = gtk::Button::with_label(if pinned { "Unpin" } else { "Pin" });
    let weak = Rc::downgrade(state);
    let id = id.to_string();
    let menu_weak = menu.downgrade();
    row.connect_clicked(move |_| {
        if let Some(menu) = menu_weak.upgrade() { menu.popdown(); }
        if let Some(state) = weak.upgrade() {
            {
                let mut s = state.borrow_mut();
                let mut next = pins::load(&s.pins_path).unwrap_or_else(|_| s.pinned.clone());
                if pinned { pins::unpin(&mut next, &id); } else { pins::pin(&mut next, &id); }
                if let Err(error) = pins::save(&s.pins_path, &next) { log(&format!("save pins: {error}")); return; }
                s.pinned = next;
            }
            rebuild(&state);
        }
    });
    content.append(&row);
    menu.popup();
}
