//! The window's Unix socket: scripts and Omarchy hooks reach a running
//! window through it (`gutenberg --reload-theme`). One line in, one line
//! out. S7.3 adds single-instance and `--refresh` here.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What a script can ask of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Read the Omarchy theme and the desktop font again.
    ReloadTheme,
}

impl Command {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReloadTheme => "reload-theme",
        }
    }

    fn parse(line: &str) -> Option<Self> {
        match line.trim() {
            "reload-theme" => Some(Self::ReloadTheme),
            _ => None,
        }
    }
}

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
}

/// `$XDG_RUNTIME_DIR/gutenberg/socket`: per user and per login, removed at
/// logout. `None` without a runtime directory.
pub fn socket_path() -> Option<PathBuf> {
    directories::BaseDirs::new()?
        .runtime_dir()
        .map(|dir| dir.join("gutenberg/socket"))
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

/// Listens on `path`, calling `on_command` on a background thread for each
/// command received. A socket file left by a window that's gone is
/// replaced; one a live window answers on is not.
pub fn listen(
    path: &Path,
    on_command: impl Fn(Command) + Send + 'static,
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
    std::thread::Builder::new()
        .name("ipc".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, &on_command);
            }
        })
        .map_err(io)?;
    Ok(Listener {
        path: path.to_owned(),
    })
}

/// One connection: read one line, act on it, answer.
fn serve(stream: UnixStream, on_command: &impl Fn(Command)) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let answer = match Command::parse(&line) {
        Some(command) => {
            on_command(command);
            "ok\n"
        }
        None => "unknown\n",
    };
    let _ = (&stream).write_all(answer.as_bytes());
}

/// Sends `command` to the window listening on `path` and waits for its
/// answer.
pub fn send(path: &Path, command: Command) -> Result<(), Error> {
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
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(io)?;
    writeln!(stream, "{}", command.as_str()).map_err(io)?;
    let mut answer = String::new();
    BufReader::new(&stream).read_line(&mut answer).map_err(io)?;
    match answer.trim() {
        "ok" => Ok(()),
        other => Err(Error::Refused(other.to_owned())),
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
        let listener = listen(&path, move |c| tx.send(c).unwrap()).unwrap();
        send(&path, Command::ReloadTheme).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Command::ReloadTheme
        );

        // A second window can't take the socket from a live one.
        assert!(matches!(listen(&path, |_| {}), Err(Error::InUse(_))));

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
        let _listener = listen(&path, move |c| tx.send(c).unwrap()).unwrap();
        send(&path, Command::ReloadTheme).unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn an_unknown_command_is_refused_and_not_acted_on() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("socket");
        let (tx, rx) = mpsc::channel();
        let _listener = listen(&path, move |c| tx.send(c).unwrap()).unwrap();
        let mut stream = UnixStream::connect(&path).unwrap();
        writeln!(stream, "format-disk").unwrap();
        let mut answer = String::new();
        BufReader::new(&stream).read_line(&mut answer).unwrap();
        assert_eq!(answer, "unknown\n");
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
