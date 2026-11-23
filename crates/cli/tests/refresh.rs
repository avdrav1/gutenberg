//! `gutenberg --refresh` with no window (S10.3): two runs never refresh at
//! once. Everything lives in a temp dir: the runtime directory (the socket
//! and the refresh lock), the store and the config.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Command, Output};

use gutenberg_core::lock::RefreshLock;

fn refresh(home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gutenberg"))
        .arg("--refresh")
        .env("HOME", home)
        .env("XDG_RUNTIME_DIR", home.join("run"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        // No desktop to notify, whatever the test runs under.
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .output()
        .unwrap()
}

#[test]
fn a_refresh_that_finds_another_running_exits_without_refreshing() {
    let home = tempfile::tempdir().unwrap();
    let lock = home.path().join("run/gutenberg/refresh.lock");
    let store = home.path().join("data/gutenberg/gutenberg.db");

    // Another run (the window's thread, or an earlier timer run) holds it.
    let held = RefreshLock::try_take(&lock).unwrap().unwrap();
    let out = refresh(home.path());
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "Another refresh is running\n"
    );
    assert!(!store.exists(), "the store was opened");

    // Once it's done, this run refreshes, and lets the lock go after.
    drop(held);
    let out = refresh(home.path());
    assert!(out.status.success(), "{out:?}");
    assert!(store.exists());
    assert!(RefreshLock::try_take(&lock).unwrap().is_some());
}
