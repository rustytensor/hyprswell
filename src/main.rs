mod ui;
mod view;

use gtk::prelude::*;
use hyprswell::{control, ipc, rules};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub fn home_path(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback)
    })
}

pub fn log(message: &str) {
    let path = home_path("XDG_STATE_HOME", ".local/state").join("hyprswell/dock.log");
    if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{message}");
    }
}

fn main() {
    let mut output = None;
    let mut swell = true;
    let mut reflections_allowed = true;
    let mut show = false;
    let mut control_socket = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" => {
                output = args.next();
                if output.is_none() { eprintln!("--output needs a screen name"); std::process::exit(2); }
            }
            "--no-swell" => swell = false,
            "--no-reflections" => reflections_allowed = false,
            "--show" => show = true,
            "--control-socket" => {
                control_socket = args.next().map(PathBuf::from);
                if control_socket.is_none() { eprintln!("--control-socket needs a path"); std::process::exit(2); }
            }
            "--help" | "-h" => {
                println!("hyprswell [--output NAME] [--no-swell] [--no-reflections] [--control-socket PATH]\n\
                          hyprswell --show [--control-socket PATH]   reveal the running dock; never starts one");
                return;
            }
            _ => { eprintln!("unknown argument: {arg}"); std::process::exit(2); }
        }
    }
    let control_path = match control_socket.map_or_else(control::default_path, Ok) {
        Ok(path) => Some(path),
        Err(error) if show => { eprintln!("hyprswell --show: {error}"); std::process::exit(1); }
        Err(error) => { log(&format!("no control socket: {error}")); None }
    };
    if show {
        let path = control_path.expect("checked above");
        match control::request_show(&path, Duration::from_secs(5)) {
            Ok(reply) if reply == control::SHOWN => return,
            Ok(reply) => { eprintln!("hyprswell --show: dock replied {reply:?}"); std::process::exit(1); }
            Err(error) => { eprintln!("hyprswell --show: {error}"); std::process::exit(1); }
        }
    }
    let config = home_path("XDG_CONFIG_HOME", ".config").join("nwg-dock-hyprland/hyprswell.conf");
    let filters = rules::screen_filters(&std::fs::read_to_string(config).unwrap_or_default());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut pending = None;
    let selected = loop {
        let (current, error) = match ipc::monitors() {
            Ok(monitors) => (Some(rules::select_monitors(&monitors, output.as_deref(), &filters)), None),
            Err(error) => (None, Some(error)),
        };
        // One deadline decision per response: expiry must not discard a pending fallback.
        let now = Instant::now();
        let expired = now >= deadline;
        if let Some(selected) = rules::startup_selection(current, &mut pending, !filters.is_empty(), expired) {
            if selected.fallback {
                let reason = if filters.is_empty() { "SCREENS unset or empty" } else { "no SCREENS match" };
                log(&format!("{reason}; falling back to {}", selected.names[0]));
            }
            break selected.names;
        }
        if expired {
            if let Some(error) = error { log(&format!("cannot list screens: {error}")); }
            else { log(if output.is_some() { "requested screen not found, no dock" } else { "no screens found, no dock" }); }
            return;
        }
        std::thread::sleep(Duration::from_millis(250).min(deadline.saturating_duration_since(now)));
    };
    let app = gtk::Application::builder().application_id("org.hyprswell.Dock")
        .flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_activate(move |app| ui::start(app, &selected, swell, reflections_allowed, control_path.clone()));
    app.run_with_args::<&str>(&[]);
}
