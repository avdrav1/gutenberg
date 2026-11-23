//! One refresh at a time (S10.3). The window's refresh thread and a
//! `gutenberg --refresh` from the systemd timer both write the store; each
//! takes an exclusive lock on `refresh.lock` beside the window's socket
//! first. The window waits for it; the timer's run, finding it taken, has
//! nothing to do and exits. The kernel drops the lock with the process, so
//! a run that crashed never leaves it held.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// `refresh.lock` in the socket's directory: `$XDG_RUNTIME_DIR/gutenberg`
/// (`$TMPDIR/gutenberg` on macOS). `None` without one.
pub fn path() -> Option<PathBuf> {
    crate::ipc::socket_path().map(|socket| socket.with_file_name("refresh.lock"))
}

/// Held until dropped.
#[derive(Debug)]
pub struct RefreshLock(#[allow(dead_code)] File);

impl RefreshLock {
    /// The lock if it's free; `Ok(None)` if another process (or another
    /// handle in this one) holds it.
    pub fn try_take(path: &Path) -> io::Result<Option<Self>> {
        let file = open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self(file))),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(err)) => Err(err),
        }
    }

    /// The lock, waiting for whoever holds it.
    pub fn take(path: &Path) -> io::Result<Self> {
        let file = open(path)?;
        file.lock()?;
        Ok(Self(file))
    }
}

fn open(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_refresh_waits_its_turn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gutenberg/refresh.lock");
        let first = RefreshLock::try_take(&path)
            .unwrap()
            .expect("free at first");
        assert!(RefreshLock::try_take(&path).unwrap().is_none());
        // The window's thread waits, and gets it once the first is done.
        let waiting = {
            let path = path.clone();
            std::thread::spawn(move || RefreshLock::take(&path).unwrap())
        };
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!waiting.is_finished());
        drop(first);
        let second = waiting.join().unwrap();
        assert!(RefreshLock::try_take(&path).unwrap().is_none());
        drop(second);
        assert!(RefreshLock::try_take(&path).unwrap().is_some());
    }

    #[test]
    fn the_lock_sits_beside_the_socket() {
        if let (Some(lock), Some(socket)) = (path(), crate::ipc::socket_path()) {
            assert_eq!(lock.parent(), socket.parent());
        }
    }
}
