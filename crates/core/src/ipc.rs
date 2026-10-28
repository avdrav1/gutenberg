//! The window's Unix socket: scripts and desktop hooks reach a running
//! window through it (`gutenberg --reload-theme`). One line in, one line
//! out: `ok`, `ok <detail>`, `error <why>` or `unknown`.
//!
//! The socket also keeps to one window: a second `gutenberg` finds it
//! taken, asks the open window to come forward, and exits.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What a script can ask of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Read the Omarchy theme and the desktop font again.
    ReloadTheme,
    /// Come forward: another launch found this window open.
    Focus,
    /// Refresh what's due now; the answer is the refresh's summary.
    Refresh,
}

impl Command {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReloadTheme => "reload-theme",
            Self::Focus => "focus",
            Self::Refresh => "refresh",
        }
    }

    fn parse(line: &str) -> Option<Self> {
        match line.trim() {
            "reload-theme" => Some(Self::ReloadTheme),
            "focus" => Some(Self::Focus),
            "refresh" => Some(Self::Refresh),
            _ => None,
        }
    }

    /// How long a script waits for the answer: a refresh fetches every due
    /// source first.
    fn timeout(self) -> Duration {
        match self {
            Self::Refresh => Duration::from_secs(10 * 60),
            Self::ReloadTheme | Self::Focus => Duration::from_secs(2),
        }
    }
}

/// What the window says to a command: a detail line (empty for none), or
/// why it couldn't.
pub type Answer = Result<String, String>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Gutenberg window is open")]
    NotRunning,
    #[error("a Gutenberg window already listens on {}", .0.display())]
    InUse(PathBuf),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the window answered {0:?}")]
    Refused(String),
    /// The window took the command and couldn't carry it out.
    #[error("{0}")]
    Failed(String),
}

/// `$XDG_RUNTIME_DIR/gutenberg/socket`: per user and per login, removed at
/// logout. `None` without a runtime directory.
#[cfg(target_os = "linux")]
pub fn socket_path() -> Option<PathBuf> {
    directories::BaseDirs::new()?
        .runtime_dir()
        .map(|dir| dir.join("gutenberg/socket"))
}

/// `$TMPDIR/gutenberg/socket`: macOS has no runtime directory, and `$TMPDIR`
/// is the per-user `/var/folders/…/T/`, mode 0700 and cleared at reboot.
/// It's about 50 bytes long, well under a socket path's 104. `/tmp` if
/// `TMPDIR` is unset, which launchd never leaves it.
#[cfg(target_os = "macos")]
pub fn socket_path() -> Option<PathBuf> {
    Some(std::env::temp_dir().join("gutenberg/socket"))
}

/// Keeps the socket open until dropped, then removes its file.
pub struct Listener {
    path: PathBuf,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Listens on `path`, calling `on_command` for each command received, on a
/// thread per connection so a long refresh doesn't hold up a theme reload.
/// A socket file left by a window that's gone is replaced; one a live
/// window answers on is not.
pub fn listen(
    path: &Path,
    on_command: impl Fn(Command) -> Answer + Send + Sync + 'static,
) -> Result<Listener, Error> {
    let io = |source| Error::Io {
        path: path.to_owned(),
        source,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let listener = match UnixListener::bind(path) {
        Ok(listener) => listener,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                return Err(Error::InUse(path.to_owned()));
            }
            std::fs::remove_file(path).map_err(io)?;
            UnixListener::bind(path).map_err(io)?
        }
        Err(e) => return Err(io(e)),
    };
    let on_command = std::sync::Arc::new(on_command);
    std::thread::Builder::new()
        .name("ipc".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let on_command = on_command.clone();
                let _ = std::thread::Builder::new()
                    .name("ipc-connection".into())
                    .spawn(move || serve(stream, on_command.as_ref()));
            }
        })
        .map_err(io)?;
    Ok(Listener {
        path: path.to_owned(),
    })
}

/// One connection: read one line, act on it, answer.
fn serve(stream: UnixStream, on_command: &impl Fn(Command) -> Answer) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let one_line = |s: String| s.replace(['\r', '\n'], " ");
    let answer = match Command::parse(&line).map(on_command) {
        Some(Ok(detail)) if detail.is_empty() => "ok\n".to_owned(),
        Some(Ok(detail)) => format!("ok {}\n", one_line(detail)),
        Some(Err(why)) => format!("error {}\n", one_line(why)),
        None => "unknown\n".to_owned(),
    };
    let _ = (&stream).write_all(answer.as_bytes());
}

/// Sends `command` to the window listening on `path` and waits for its
/// answer: the detail line, empty for none.
pub fn send(path: &Path, command: Command) -> Result<String, Error> {
    let io = |source| Error::Io {
        path: path.to_owned(),
        source,
    };
    let mut stream = match UnixStream::connect(path) {
        Ok(stream) => stream,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(Error::NotRunning)
        }
        Err(e) => return Err(io(e)),
    };
    stream
        .set_read_timeout(Some(command.timeout()))
        .map_err(io)?;
    writeln!(stream, "{}", command.as_str()).map_err(io)?;
    let mut answer = String::new();
    BufReader::new(&stream).read_line(&mut answer).map_err(io)?;
    let answer = answer.trim();
    if answer == "ok" {
        return Ok(String::new());
    }
    if let Some(detail) = answer.strip_prefix("ok ") {
        return Ok(detail.to_owned());
    }
    match answer.strip_prefix("error ") {
        Some(why) => Err(Error::Failed(why.to_owned())),
        None => Err(Error::Refused(answer.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_command_reaches_the_listener_and_is_answered() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gutenberg/socket");
        let (tx, rx) = mpsc::channel();
        let listener = listen(&path, move |c| {
            tx.send(c).unwrap();
            Ok(String::new())
        })
        .unwrap();
        assert_eq!(send(&path, Command::ReloadTheme).unwrap(), "");
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Command::ReloadTheme
        );

        // A second window can't take the socket from a live one.
        assert!(matches!(
            listen(&path, |_| Ok(String::new())),
            Err(Error::InUse(_))
        ));

        // Closing the window removes the socket; scripts then hear so.
        drop(listener);
        assert!(!path.exists());
        assert!(matches!(
            send(&path, Command::ReloadTheme),
            Err(Error::NotRunning)
        ));
    }

    #[test]
    fn a_socket_left_by_a_crashed_window_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("socket");
        // Bound, then the listener is gone: the file stays, nothing answers.
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        assert!(matches!(
            send(&path, Command::ReloadTheme),
            Err(Error::NotRunning)
        ));
        let (tx, rx) = mpsc::channel();
        let _listener = listen(&path, move |c| {
            tx.send(c).unwrap();
            Ok(String::new())
        })
        .unwrap();
        send(&path, Command::ReloadTheme).unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn an_unknown_command_is_refused_and_not_acted_on() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("socket");
        let (tx, rx) = mpsc::channel();
        let _listener = listen(&path, move |c| {
            tx.send(c).unwrap();
            Ok(String::new())
        })
        .unwrap();
        let mut stream = UnixStream::connect(&path).unwrap();
        writeln!(stream, "format-disk").unwrap();
        let mut answer = String::new();
        BufReader::new(&stream).read_line(&mut answer).unwrap();
        assert_eq!(answer, "unknown\n");
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn answers_carry_a_detail_or_why_not() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("socket");
        let _listener = listen(&path, |c| match c {
            Command::Refresh => Ok("2 sources: 1 updated\n(3 new articles)".into()),
            Command::Focus => Err("no window\nyet".into()),
            Command::ReloadTheme => Ok(String::new()),
        })
        .unwrap();
        // A detail stays on one line.
        assert_eq!(
            send(&path, Command::Refresh).unwrap(),
            "2 sources: 1 updated (3 new articles)"
        );
        assert!(matches!(
            send(&path, Command::Focus),
            Err(Error::Failed(why)) if why == "no window yet"
        ));
    }

    #[test]
    fn a_slow_command_doesnt_hold_up_another() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("socket");
        let (release, wait) = mpsc::channel::<()>();
        let wait = std::sync::Mutex::new(wait);
        let _listener = listen(&path, move |c| {
            if c == Command::Refresh {
                wait.lock().unwrap().recv().unwrap();
            }
            Ok(String::new())
        })
        .unwrap();
        let slow = {
            let path = path.clone();
            std::thread::spawn(move || send(&path, Command::Refresh))
        };
        std::thread::sleep(Duration::from_millis(50));
        // The refresh is still running; the theme reload is answered.
        send(&path, Command::ReloadTheme).unwrap();
        release.send(()).unwrap();
        slow.join().unwrap().unwrap();
    }
}
