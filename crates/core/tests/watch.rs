use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use gutenberg_core::config::{self, Config, Error};

/// Acceptance for S1.4: a change applies within 1 s.
const BUDGET: Duration = Duration::from_secs(1);

fn next(rx: &mpsc::Receiver<Result<Config, Error>>) -> (Result<Config, Error>, Duration) {
    let start = Instant::now();
    let got = rx.recv_timeout(BUDGET).expect("no reload within 1 s");
    (got, start.elapsed())
}

fn list_width(rx: &mpsc::Receiver<Result<Config, Error>>) -> u16 {
    next(rx).0.expect("config should parse").layout.list
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
}

#[test]
fn edits_replacements_and_deletes_reload_within_a_second() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gutenberg").join("config.toml");
    let (tx, rx) = mpsc::channel();
    let _handle = config::watch(&path, move |c| {
        let _ = tx.send(c);
    })
    .unwrap();

    // A new file.
    write(&path, "[layout]\nlist = 5\n");
    assert_eq!(list_width(&rx), 5);

    // Saved in place.
    write(&path, "[layout]\nlist = 6\n");
    assert_eq!(list_width(&rx), 6);

    // Saved the way vim and helix do: write a temp file, rename it over.
    let tmp = path.with_extension("toml.swp");
    write(&tmp, "[layout]\nlist = 7\n");
    std::fs::rename(&tmp, &path).unwrap();
    assert_eq!(list_width(&rx), 7);

    // A typo reports an error; the caller keeps its last good config.
    write(&path, "[layout\n");
    assert!(matches!(next(&rx).0, Err(Error::Parse { .. })));

    // Deleted: back to the defaults.
    std::fs::remove_file(&path).unwrap();
    assert_eq!(list_width(&rx), Config::default().layout.list);
}

#[test]
fn other_files_in_the_directory_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (tx, rx) = mpsc::channel();
    let _handle = config::watch(&path, move |c| {
        let _ = tx.send(c);
    })
    .unwrap();

    write(&dir.path().join("notes.txt"), "hello");
    assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
}
