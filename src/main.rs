mod ui;
mod view;

use gtk::prelude::*;
use hyprswell::{ipc, rules};
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
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output" => {
                output = args.next();
                if output.is_none() { eprintln!("--output needs a screen name"); std::process::exit(2); }
            }
            "--no-swell" => swell = false,
            "--help" | "-h" => { println!("hyprswell [--output NAME] [--no-swell]"); return; }
            _ => { eprintln!("unknown argument: {arg}"); std::process::exit(2); }
        }
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let selected = loop {
        match ipc::monitors() {
            Ok(monitors) => {
                let selected = rules::select_monitors(&monitors, output.as_deref());
                if !selected.is_empty() { break selected; }
            }
            Err(error) => {
                if Instant::now() >= deadline { log(&format!("cannot list screens: {error}")); return; }
            }
        }
        if Instant::now() >= deadline {
            log(if output.is_some() { "requested screen not found, no dock" } else { "no ultrawide found, no dock" });
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let app = gtk::Application::builder().application_id("org.hyprswell.Dock")
        .flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_activate(move |app| ui::start(app, &selected, swell));
    app.run_with_args::<&str>(&[]);
}
