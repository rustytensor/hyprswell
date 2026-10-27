use crate::rules::{Client, Monitor};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub clients: Vec<Client>,
    pub active: String,
}

fn socket(name: &str) -> io::Result<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| io::Error::other("XDG_RUNTIME_DIR is not set"))?;
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .map_err(|_| io::Error::other("HYPRLAND_INSTANCE_SIGNATURE is not set"))?;
    if signature.contains('/') || signature == ".." {
        return Err(io::Error::other("invalid compositor instance"));
    }
    Ok(PathBuf::from(runtime).join("hypr").join(signature).join(name))
}

pub fn request(command: &str) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket(".socket.sock")?)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(command.as_bytes())?;
    let mut reply = String::new();
    stream.take(8 * 1024 * 1024).read_to_string(&mut reply)?;
    Ok(reply)
}

pub fn monitors() -> io::Result<Vec<Monitor>> {
    serde_json::from_str(&request("j/monitors")?).map_err(io::Error::other)
}

pub fn snapshot() -> io::Result<Snapshot> {
    let clients = serde_json::from_str(&request("j/clients")?).map_err(io::Error::other)?;
    let active: serde_json::Value = serde_json::from_str(&request("j/activewindow")?)
        .map_err(io::Error::other)?;
    Ok(Snapshot {
        clients,
        active: active["address"].as_str().unwrap_or_default().to_owned(),
    })
}

fn refresh_event(line: &str) -> bool {
    matches!(line.split_once(">>").map(|x| x.0),
        Some("openwindow" | "closewindow" | "activewindowv2" | "movewindowv2"
        | "workspace" | "workspacev2" | "focusedmon" | "activespecial"
        | "activespecialv2" | "windowtitle" | "windowtitlev2"))
}

// The worker owns all socket reads; GTK only drains completed snapshots.
pub fn updates() -> io::Result<(Receiver<Result<Snapshot, String>>, UnixStream)> {
    let (send, receive) = mpsc::sync_channel(1);
    let (mut notify, signal) = UnixStream::pair()?;
    signal.set_nonblocking(true)?;
    std::thread::spawn(move || {
        let mut deliver = |value| send.send(value).is_ok() && notify.write_all(b".").is_ok();
        loop {
            let connection = socket(".socket2.sock").and_then(UnixStream::connect);
            let stream = match connection {
                Ok(stream) => stream,
                Err(error) => {
                    if !deliver(Err(format!("event socket: {error}"))) { return; }
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            let (events, pending) = mpsc::sync_channel(1);
            let reader = std::thread::spawn(move || {
                for line in BufReader::new(stream).lines() {
                    match line {
                        Ok(line) if refresh_event(&line) => {
                            match events.try_send(()) {
                                Err(mpsc::TrySendError::Disconnected(_)) => break,
                                _ => {}
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            });
            let mut last = Instant::now() - Duration::from_millis(50);
            loop {
                if let Some(wait) = Duration::from_millis(50).checked_sub(last.elapsed()) {
                    std::thread::sleep(wait);
                }
                while pending.try_recv().is_ok() {}
                let result = snapshot().map_err(|error| format!("refresh: {error}"));
                if !deliver(result) { return; }
                last = Instant::now();
                if pending.recv().is_err() { break; }
            }
            let _ = reader.join();
            if !deliver(Err("event socket disconnected; reconnecting".into())) { return; }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
    Ok((receive, signal))
}

#[derive(Clone, Copy, Debug)]
pub enum WindowAction { Focus, Close, Float, Fullscreen, Move(u8) }

// Addresses are compositor identifiers, never executable Lua supplied by a title.
pub fn commands(address: &str, action: WindowAction) -> io::Result<Vec<String>> {
    let hex = address.strip_prefix("0x").unwrap_or("");
    if hex.is_empty() || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(io::Error::other("invalid window address"));
    }
    let focus = format!("dispatch hl.dsp.focus({{ window = \"address:{address}\" }})");
    Ok(match action {
        WindowAction::Focus => vec![focus],
        WindowAction::Close => vec![format!("dispatch hl.dsp.window.close({{ window = \"address:{address}\" }})")],
        WindowAction::Float => vec![format!("dispatch hl.dsp.window.float({{ action = \"toggle\", window = \"address:{address}\" }})")],
        WindowAction::Fullscreen => vec![focus, "dispatch hl.dsp.window.fullscreen({ mode = \"fullscreen\" })".into()],
        WindowAction::Move(n @ 1..=9) => vec![format!("dispatch hl.dsp.window.move({{ workspace = \"{n}\", follow = false, window = \"address:{address}\" }})")],
        WindowAction::Move(_) => return Err(io::Error::other("workspace must be 1–9")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lua_actions() {
        // The 0.56.2 brief specifies these literal commands; fullscreen acts on focus.
        assert_eq!(commands("0xa1", WindowAction::Focus).unwrap(), ["dispatch hl.dsp.focus({ window = \"address:0xa1\" })"]);
        assert_eq!(commands("0xa1", WindowAction::Close).unwrap(), ["dispatch hl.dsp.window.close({ window = \"address:0xa1\" })"]);
        assert_eq!(commands("0xa1", WindowAction::Float).unwrap(), ["dispatch hl.dsp.window.float({ action = \"toggle\", window = \"address:0xa1\" })"]);
        assert_eq!(commands("0xa1", WindowAction::Fullscreen).unwrap(), ["dispatch hl.dsp.focus({ window = \"address:0xa1\" })", "dispatch hl.dsp.window.fullscreen({ mode = \"fullscreen\" })"]);
        assert_eq!(commands("0xa1", WindowAction::Move(3)).unwrap(), ["dispatch hl.dsp.window.move({ workspace = \"3\", follow = false, window = \"address:0xa1\" })"]);
        assert!(commands("0xa1\" })", WindowAction::Close).is_err());
    }
}
