use gio::prelude::*;
use hyprswell::{desktop, pins, rules};
use rules::{Action, Client, Indicator, Monitor};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = PathBuf::from("target/hyprswell")
            .join(format!("parity-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn windows() -> Vec<Client> {
    serde_json::from_str(r#"[
        {"address":"0xa1","class":"firefox","title":"Inbox","workspace":{"id":1,"name":"1"}},
        {"address":"0xa2","class":"firefox","title":"Docs","workspace":{"id":4,"name":"4"}},
        {"address":"0xa3","class":"firefox","title":"Hidden","workspace":{"id":-99,"name":"special:minimized"}}
    ]"#).unwrap()
}

#[test]
fn pins_replace_file_and_preserve_order() {
    let scratch = Scratch::new();
    let path = scratch.0.join("cache/pinned");
    assert_eq!(pins::load(&path).unwrap(), Vec::<String>::new());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let old_bytes = " firefox \n\norg.gnome.Ptyxis\n";
    fs::write(&path, old_bytes).unwrap();
    let mut old_file = fs::File::open(&path).unwrap();
    let mut list = pins::load(&path).unwrap();
    // Whitespace and empty lines do not become app ids; line order survives.
    assert_eq!(list, ["firefox", "org.gnome.Ptyxis"]);
    pins::pin(&mut list, "google-chrome");
    pins::pin(&mut list, "google-chrome");
    pins::unpin(&mut list, "firefox");
    pins::save(&path, &list).unwrap();
    // Removing Firefox leaves Terminal then Chrome; each has one newline.
    assert_eq!(fs::read_to_string(&path).unwrap(), "org.gnome.Ptyxis\ngoogle-chrome\n");
    let mut retained = String::new();
    old_file.read_to_string(&mut retained).unwrap();
    // A pre-existing descriptor must still reference the old inode after rename.
    assert_eq!(retained, old_bytes);
}

#[test]
fn pinned_then_workspace_and_class_order() {
    let clients: Vec<Client> = serde_json::from_str(r#"[
        {"address":"0xc1","class":"zeta","title":"Z","workspace":{"id":4,"name":"4"}},
        {"address":"0xf1","class":"firefox","title":"Inbox","workspace":{"id":1,"name":"1"}},
        {"address":"0xb1","class":"beta","title":"B","workspace":{"id":2,"name":"2"}},
        {"address":"0xa1","class":"alpha","title":"A","workspace":{"id":2,"name":"2"}},
        {"address":"0xf2","class":"firefox","title":"Docs","workspace":{"id":4,"name":"4"}},
        {"address":"0xp1","class":"org.gnome.Ptyxis","title":"Shell","workspace":{"id":3,"name":"3"}},
        {"address":"0x00","class":"","title":"Closing","workspace":{"id":1,"name":"1"}},
        {"address":"0xm1","class":"minimized-app","title":"Hidden","workspace":{"id":-99,"name":"special:minimized"}}
    ]"#).unwrap();
    let pins = ["org.gnome.Ptyxis", "firefox", "pinned-only"].map(str::to_owned);
    let items = rules::items(&pins, &clients);
    // Pins lead, -99 precedes workspace 2, alpha precedes beta; closing ghost disappears.
    assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        ["launcher", "org.gnome.Ptyxis", "firefox", "pinned-only", "minimized-app", "alpha", "beta", "zeta"]);
    assert_eq!(items.iter().map(|i| i.windows.iter().map(|c| c.address.as_str()).collect::<Vec<_>>()).collect::<Vec<_>>(),
        vec![vec![], vec!["0xp1"], vec!["0xf1", "0xf2"], vec![], vec!["0xm1"], vec!["0xa1"], vec!["0xb1"], vec!["0xc1"]]);
}

#[test]
fn clicks_restore_before_considering_focus() {
    let w = windows();
    // The active visible singleton minimizes; a hidden one restores even if last active.
    assert_eq!(rules::left_click("firefox", &[], "0xa1"), Action::Launch("firefox".into()));
    assert_eq!(rules::left_click("firefox", &w[0..1], "0xa1"), Action::Minimize("0xa1".into()));
    assert_eq!(rules::left_click("firefox", &w[1..2], "0xa1"), Action::Focus("0xa2".into()));
    assert_eq!(rules::left_click("firefox", &w[2..3], "0xa1"), Action::Restore("0xa3".into()));
    assert_eq!(rules::left_click("firefox", &w[2..3], "0xa3"), Action::Restore("0xa3".into()));
    assert_eq!(rules::left_click("firefox", &w[..2], "0xa1"), Action::Menu(w[..2].to_vec()));
    // Menu choices always bring a window forward, never minimize it.
    assert_eq!(rules::menu_pick(&w[0]), Action::Focus("0xa1".into()));
    assert_eq!(rules::menu_pick(&w[2]), Action::Restore("0xa3".into()));
}

#[test]
fn indicators_and_hover_lines() {
    let w = windows();
    // Hollow means every window is minimized, not merely one of them.
    assert_eq!(rules::indicator(&[]), Indicator::None);
    assert_eq!(rules::indicator(&w[..1]), Indicator::One);
    assert_eq!(rules::indicator(&w[..2]), Indicator::Two);
    assert_eq!(rules::indicator(&w[2..]), Indicator::Hollow);
    assert_eq!(rules::indicator(&[w[2].clone(), w[2].clone()]), Indicator::Hollow);
    assert_eq!(rules::indicator(&[w[0].clone(), w[2].clone()]), Indicator::Two);
    assert_eq!(rules::hover_text("Firefox", &w), "Firefox\nInbox  · 1\nDocs  · 4\nHidden  · minimized");
    assert_eq!(rules::hover_text("Firefox", &[]), "Firefox");
    assert_eq!(rules::window_line(&w[1]), "Docs  · 4");
    let mut long = w[2].clone();
    // 48 Unicode scalar characters fit; 49 become 47 plus one ellipsis, before the suffix.
    long.title = "é".repeat(48);
    assert_eq!(rules::window_line(&long), format!("{}  · minimized", "é".repeat(48)));
    long.title.push('é');
    assert_eq!(rules::window_line(&long), format!("{}…  · minimized", "é".repeat(47)));
}

#[test]
fn icon_configuration_and_integer_shrink() {
    assert_eq!(rules::icon_size(""), 44);
    assert_eq!(rules::icon_size("ICON_SIZE=nope\n"), 44);
    assert_eq!(rules::icon_size("# comment\nICON_SIZE=48\n"), 48);
    // 44*5/4=55. App counts 7 and 8 still have floor((n-6)/3)=0.
    assert_eq!(rules::drawn_size(44, 6), 55);
    assert_eq!(rules::drawn_size(44, 7), 55);
    assert_eq!(rules::drawn_size(44, 8), 55);
    // 330/7 and 330/8 truncate to 47 and 41, not a smooth reduction.
    assert_eq!(rules::drawn_size(44, 9), 47);
    assert_eq!(rules::drawn_size(44, 12), 41);
}

#[test]
fn screens_match_model_not_port() {
    let monitors: Vec<Monitor> = serde_json::from_str(r#"[
        {"name":"DP-2","description":"GWD ARZOPA 0x37330206"},
        {"name":"HDMI-A-1","description":"Dell Inc. DELL U2515H YKFWP4CC09VL"},
        {"name":"HDMI-A-2","description":"GWD ARZOPA 0000001504150"},
        {"name":"DP-1","description":"Samsung Electric Company S34C65xU HCNYB00237"},
        {"name":"DP-3","description":"Samsung Electric Company S34C65xU HCNYB00019"}
    ]"#).unwrap();
    // Only the two Samsung descriptions contain the model; explicit testing overrides that filter.
    assert_eq!(rules::select_monitors(&monitors, None), ["DP-1", "DP-3"]);
    assert_eq!(rules::select_monitors(&monitors, Some("DP-2")), ["DP-2"]);
    assert!(rules::select_monitors(&monitors[..3], None).is_empty());
    assert!(rules::select_monitors(&monitors, Some("unknown")).is_empty());
    let moved = vec![Monitor { name: "DP-9".into(), description: monitors[3].description.clone() }];
    assert_eq!(rules::select_monitors(&moved, None), ["DP-9"]);
}

#[test]
fn desktop_matching_isolated() {
    if std::env::var_os("HYPRSWELL_DESKTOP_FIXTURE").is_some() {
        // Real GIO reads only these child-process XDG directories, before any GIO caches exist.
        let firefox = desktop::app_info("firefox").unwrap();
        assert_eq!(firefox.id().as_deref(), Some("firefox.desktop"));
        assert_eq!(firefox.name(), "Firefox");
        assert_eq!(firefox.icon().unwrap().to_string().as_deref(), Some("firefox"));
        assert_eq!(desktop::app_info("Firefox").unwrap().id(), firefox.id());
        // No google-chrome.desktop exists: this result requires StartupWMClass matching.
        let chrome = desktop::app_info("google-chrome").unwrap();
        assert_eq!(chrome.id().as_deref(), Some("google-chrome-stable.desktop"));
        assert_eq!(chrome.name(), "Google Chrome");
        assert_eq!(desktop::app_info("GOOGLE-CHROME").unwrap().id(), chrome.id());
        let terminal = desktop::app_info("org.gnome.Ptyxis").unwrap();
        assert_eq!(terminal.name(), "Terminal");
        assert_eq!(terminal.icon().unwrap().to_string().as_deref(), Some("org.gnome.Ptyxis"));
        assert!(desktop::app_info("no-such-fixture-app").is_none());
        // A changed desktop file stays cached until the application monitor invalidates it.
        fs::write(firefox.filename().unwrap(), "[Desktop Entry]\nType=Application\nName=Firefox Updated\nIcon=firefox\nExec=firefox %u\n").unwrap();
        assert_eq!(desktop::app_info("firefox").unwrap().name(), "Firefox");
        desktop::invalidate();
        assert_eq!(desktop::app_info("firefox").unwrap().name(), "Firefox Updated");
        return;
    }
    let scratch = Scratch::new();
    let data = scratch.0.join("data");
    let apps = data.join("applications");
    let bin = scratch.0.join("bin");
    fs::create_dir_all(&apps).unwrap();
    fs::create_dir_all(&bin).unwrap();
    // Executable stubs make desktop validity independent of installed applications. Never launched.
    for exe in ["firefox", "google-chrome-stable", "ptyxis"] {
        std::os::unix::fs::symlink("/bin/true", bin.join(exe)).unwrap();
    }
    for (id, body) in [
        ("firefox", "Name=Firefox\nIcon=firefox\nExec=firefox %u\n"),
        ("google-chrome-stable", "Name=Google Chrome\nIcon=google-chrome\nExec=google-chrome-stable %U\nStartupWMClass=google-chrome\n"),
        ("org.gnome.Ptyxis", "Name=Terminal\nIcon=org.gnome.Ptyxis\nExec=ptyxis\n"),
    ] {
        fs::write(apps.join(format!("{id}.desktop")), format!("[Desktop Entry]\nType=Application\n{body}")).unwrap();
    }
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "desktop_matching_isolated", "--nocapture"])
        .env("HYPRSWELL_DESKTOP_FIXTURE", "1")
        .env("HOME", &scratch.0)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_DATA_DIRS", scratch.0.join("empty-data"))
        .env("XDG_CONFIG_HOME", scratch.0.join("config"))
        .env("XDG_CONFIG_DIRS", scratch.0.join("empty-config"))
        .env("XDG_CACHE_HOME", scratch.0.join("cache"))
        .env("LC_ALL", "C")
        .env_remove("LANGUAGE")
        .env("PATH", &bin)
        .status().unwrap();
    assert!(status.success(), "isolated desktop matching failed");
}
