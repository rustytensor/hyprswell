//! Same-user control socket. It accepts one bounded request, `show`, and never starts a dock.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

pub const MAX_REQUEST: usize = 16;
pub const SHOWN: &str = "shown";

pub fn default_path() -> io::Result<PathBuf> { crate::ipc::instance_path("hyprswell.sock") }

fn uid() -> u32 { unsafe { libc::getuid() } }

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

fn check_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let meta = std::fs::metadata(parent)?;
    if meta.uid() != uid() || meta.mode() & 0o022 != 0 {
        return Err(io::Error::other(format!("{} is not a private directory of this user", parent.display())));
    }
    Ok(())
}

/// Exactly `show`, optionally newline-terminated.
pub fn parse_request(bytes: &[u8]) -> bool {
    matches!(bytes, b"show" | b"show\n")
}

pub struct Server {
    listener: UnixListener,
    endpoint: Endpoint,
    lock: File,
}

/// Unlinks only the endpoint this server created, never a newer or foreign one.
pub struct Endpoint {
    path: PathBuf,
    identity: (u64, u64),
}

impl Endpoint {
    pub fn path(&self) -> &Path { &self.path }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        if let Ok(meta) = std::fs::symlink_metadata(&self.path) {
            if (meta.dev(), meta.ino()) == self.identity { let _ = std::fs::remove_file(&self.path); }
        }
    }
}

impl Server {
    /// Hold the lifetime lock, replace only a stale socket of this user, and bind with mode 0600.
    pub fn bind(path: &Path) -> io::Result<Self> {
        check_parent(path)?;
        let lock = OpenOptions::new().read(true).write(true).create(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(lock_path(path))?;
        if lock.metadata()?.uid() != uid() { return Err(io::Error::other("control lock belongs to another user")); }
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::new(io::ErrorKind::AddrInUse, "another dock owns this control socket"));
        }
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_socket() && meta.uid() == uid() => std::fs::remove_file(path)?,
            Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists,
                format!("{} exists and is not this user's socket; left alone", path.display()))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let meta = std::fs::symlink_metadata(path)?;
        Ok(Self { listener, endpoint: Endpoint { path: path.to_owned(), identity: (meta.dev(), meta.ino()) }, lock })
    }

    /// A worker thread reads and validates requests; the GTK loop only receives accepted show requests.
    pub fn serve(self) -> io::Result<(Receiver<UnixStream>, UnixStream, Endpoint)> {
        let (send, receive) = mpsc::channel();
        let (mut notify, signal) = UnixStream::pair()?;
        signal.set_nonblocking(true)?;
        let Self { listener, endpoint, lock } = self;
        std::thread::spawn(move || {
            // The lifetime lock is held for as long as this endpoint can accept.
            let _lock = lock;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue; };
                if peer_uid(&stream) != Some(uid()) { continue; }
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                let mut buffer = Vec::new();
                let _ = (&mut stream).take(MAX_REQUEST as u64 + 1).read_to_end(&mut buffer);
                if !parse_request(&buffer) { let _ = stream.write_all(b"error\n"); continue; }
                if send.send(stream).is_err() || notify.write_all(b".").is_err() { break; }
            }
        });
        Ok((receive, signal, endpoint))
    }
}

fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut cred = libc::ucred { pid: 0, uid: u32::MAX, gid: 0 };
    let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED,
        (&mut cred as *mut libc::ucred).cast(), &mut size) };
    (result == 0).then_some(cred.uid)
}

/// The `--show` client: ask the running dock and wait for its completed shown frame.
pub fn request_show(path: &Path, timeout: Duration) -> io::Result<String> {
    let meta = std::fs::symlink_metadata(path).map_err(|error| io::Error::new(error.kind(),
        format!("no running dock at {}: {error}", path.display())))?;
    if !meta.file_type().is_socket() || meta.uid() != uid() {
        return Err(io::Error::other(format!("{} is not this user's dock socket", path.display())));
    }
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(b"show\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut reply = String::new();
    stream.take(64).read_to_string(&mut reply)?;
    Ok(reply.trim().to_owned())
}
