//! One Untethered per profile, as on Discord: launching it again while it
//! runs (in the tray, say) shows the running window instead of starting a
//! second copy that would find the profile locked.
//!
//! The first process holds `instance.lock` in the profile directory and
//! listens on a loopback port, which `instance` beside it names together
//! with a secret. A later launch finds the lock taken, connects, sends the
//! secret and exits once the running one has shown its window. One that is
//! quitting says so, and the new launch waits for it to exit, then starts as
//! usual. A loopback socket rather than a Unix socket or a named pipe, so the
//! same code runs on Linux and Windows.
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// How long a launch waits for a running Untethered that is quitting: Quit
/// gives up on a call that won't end after 10 s (tray.rs).
const PATIENCE: Duration = Duration::from_secs(15);
/// How long the running one may take to show its window.
const ANSWER: Duration = Duration::from_secs(5);
const LOCK: &str = "instance.lock";
const ADDRESS: &str = "instance";

pub enum Start {
    /// This is the profile's one Untethered, and later launches arrive here.
    /// Keep it until the app exits.
    First(Instance),
    /// The running Untethered showed its window, or this start at sign-in
    /// found it running already: nothing left to do.
    Handed,
    /// No profile directory, or no way to listen or reach the running one:
    /// run without the guard, as before (a second copy then reports the
    /// profile as already open).
    Unguarded,
}

pub struct Instance {
    _lock: fs::File,
    pub launches: async_channel::Receiver<Launch>,
}

/// Untethered was launched again: answer whether the window is showing.
#[derive(Clone)]
pub struct Launch(mpsc::SyncSender<bool>);
impl Launch {
    pub fn answer(&self, shown: bool) {
        let _ = self.0.try_send(shown);
    }
}

/// Become the profile's one Untethered, or hand this launch to the running
/// one. `hidden` is a start at sign-in, which leaves a running one alone.
pub fn start(hidden: bool) -> Start {
    match crate::engine::profile_root() {
        Ok(root) => start_in(&root, hidden, PATIENCE),
        Err(_) => Start::Unguarded,
    }
}

fn start_in(root: &Path, hidden: bool, patience: Duration) -> Start {
    let Ok(lock) = open_lock(root) else {
        return Start::Unguarded;
    };
    let give_up = Instant::now() + patience;
    let mut quitting = false;
    loop {
        match lock.try_lock() {
            Ok(()) => return listen(root, lock).map_or(Start::Unguarded, Start::First),
            Err(fs::TryLockError::WouldBlock) => {}
            Err(fs::TryLockError::Error(_)) => return Start::Unguarded,
        }
        if hidden {
            return Start::Handed;
        }
        if !quitting {
            match ask(root) {
                Ok(true) => return Start::Handed,
                Ok(false) => quitting = true,
                // Still starting up, and not listening yet.
                Err(_) => {}
            }
        }
        if Instant::now() >= give_up {
            return Start::Unguarded;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn open_lock(root: &Path) -> io::Result<fs::File> {
    let mut dir = fs::DirBuilder::new();
    dir.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut dir, 0o700);
    dir.create(root)?;
    fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(LOCK))
}

fn listen(root: &Path, lock: fs::File) -> io::Result<Instance> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let secret = uuid::Uuid::new_v4().simple().to_string();
    // Mode 0600, and renamed into place: a launch never reads half of it.
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    writeln!(file, "{} {secret}", listener.local_addr()?.port())?;
    file.persist(root.join(ADDRESS)).map_err(|e| e.error)?;
    let (app, launches) = async_channel::unbounded();
    thread::Builder::new()
        .name("instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = serve(stream, &secret, &app);
            }
        })?;
    Ok(Instance {
        _lock: lock,
        launches,
    })
}

/// One later launch: if it knows the secret, show the window and say so.
fn serve(
    mut stream: TcpStream,
    secret: &str,
    app: &async_channel::Sender<Launch>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut line = String::new();
    BufReader::new((&stream).take(128)).read_line(&mut line)?;
    if line.trim_end().strip_prefix("show ") != Some(secret) {
        return Ok(());
    }
    let (answer, answered) = mpsc::sync_channel(1);
    let shown = app.send_blocking(Launch(answer)).is_ok()
        && matches!(answered.recv_timeout(ANSWER), Ok(true));
    stream.write_all(if shown { b"shown\n" } else { b"quitting\n" })
}

/// Ask the running Untethered to show its window: true when it did, false
/// when it is quitting.
fn ask(root: &Path) -> io::Result<bool> {
    let address = fs::read_to_string(root.join(ADDRESS))?;
    let (port, secret) = address
        .trim_end()
        .split_once(' ')
        .ok_or(io::ErrorKind::InvalidData)?;
    let port: u16 = port.parse().map_err(|_| io::ErrorKind::InvalidData)?;
    // Windows lets only the foreground process bring a window to the front;
    // this launch is that process, so it passes the right on.
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
        AllowSetForegroundWindow(ASFW_ANY);
    }
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
    stream.set_read_timeout(Some(ANSWER + Duration::from_secs(2)))?;
    writeln!(stream, "show {secret}")?;
    let mut reply = String::new();
    BufReader::new(stream.take(32)).read_line(&mut reply)?;
    match reply.trim_end() {
        "shown" => Ok(true),
        "quitting" => Ok(false),
        _ => Err(io::ErrorKind::InvalidData.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(root: &Path) -> Instance {
        match start_in(root, false, Duration::ZERO) {
            Start::First(instance) => instance,
            _ => panic!("the first launch must become the instance"),
        }
    }

    #[test]
    fn a_second_launch_shows_the_running_window() {
        let dir = tempfile::tempdir().unwrap();
        let running = first(dir.path());
        let launches = running.launches.clone();
        let app = thread::spawn(move || launches.recv_blocking().unwrap().answer(true));
        assert!(matches!(
            start_in(dir.path(), false, Duration::from_secs(5)),
            Start::Handed
        ));
        app.join().unwrap();
    }

    #[test]
    fn a_start_at_sign_in_leaves_the_running_one_alone() {
        let dir = tempfile::tempdir().unwrap();
        let running = first(dir.path());
        assert!(matches!(
            start_in(dir.path(), true, Duration::from_secs(5)),
            Start::Handed
        ));
        assert!(running.launches.is_empty());
    }

    #[test]
    fn a_launch_waits_for_one_that_is_quitting_then_starts() {
        let dir = tempfile::tempdir().unwrap();
        let running = first(dir.path());
        let app = thread::spawn(move || {
            running.launches.recv_blocking().unwrap().answer(false);
            thread::sleep(Duration::from_millis(300));
            drop(running);
        });
        let next = start_in(dir.path(), false, Duration::from_secs(5));
        app.join().unwrap();
        let Start::First(next) = next else {
            panic!("the launch must start once the old one exits");
        };
        // And it answers launches in turn.
        let launches = next.launches.clone();
        let app = thread::spawn(move || launches.recv_blocking().unwrap().answer(true));
        assert!(matches!(ask(dir.path()), Ok(true)));
        app.join().unwrap();
    }

    #[test]
    fn a_wrong_secret_shows_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let running = first(dir.path());
        let address = fs::read_to_string(dir.path().join(ADDRESS)).unwrap();
        let port: u16 = address.split_once(' ').unwrap().0.parse().unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        writeln!(stream, "show not-the-secret").unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        assert_eq!(reply, "");
        assert!(running.launches.is_empty());
    }

    #[test]
    fn an_unreachable_instance_leaves_the_launch_unguarded() {
        let dir = tempfile::tempdir().unwrap();
        // Locked, but not listening where the address says.
        let held = open_lock(dir.path()).unwrap();
        held.try_lock().unwrap();
        fs::write(dir.path().join(ADDRESS), "1 secret\n").unwrap();
        assert!(matches!(
            start_in(dir.path(), false, Duration::from_millis(300)),
            Start::Unguarded
        ));
    }
}
