use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub fn load(path: &Path) -> io::Result<Vec<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text.lines().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e),
    }
}

pub fn save(path: &Path, pins: &[String]) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    // A unique sibling keeps concurrent saves separate and rename on one filesystem.
    let (temporary, mut file) = loop {
        let temporary = parent.join(format!(".dock-pins-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match OpenOptions::new().write(true).create_new(true).open(&temporary) {
            Ok(file) => break (temporary, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = (|| {
        for pin in pins { writeln!(file, "{pin}")?; }
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}

pub fn pin(pins: &mut Vec<String>, id: &str) {
    if !pins.iter().any(|p| p == id) { pins.push(id.into()); }
}

pub fn unpin(pins: &mut Vec<String>, id: &str) {
    pins.retain(|p| p != id);
}
