//! `gutenberg --update`: install the newest release over this binary. The
//! installer is `site/install.sh`, built into the binary, so nothing but the
//! release tarball is downloaded, and it's checked against the release's
//! SHA256SUMS before it replaces anything.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use gutenberg_core::{ipc, sync, update};
use gutenberg_sources::feed;

const INSTALLER: &str = include_str!("../../../site/install.sh");

pub fn run() -> ExitCode {
    match update_to_latest() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("gutenberg: update: {message}");
            ExitCode::FAILURE
        }
    }
}

fn update_to_latest() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let client = feed::client().map_err(|e| e.to_string())?;
    let latest = runtime.block_on(update::latest(&client, update::LATEST))?;
    if !update::is_newer(&latest, update::CURRENT) {
        println!("Gutenberg {} is the latest release.", update::CURRENT);
        return Ok(());
    }
    println!("Gutenberg {} → {latest}", update::CURRENT);
    let dir = install_dir()?;
    let mut installer = installer(&latest, &dir)
        .spawn()
        .map_err(|e| format!("couldn't run sh: {e}"))?;
    if let Some(mut stdin) = installer.stdin.take() {
        stdin
            .write_all(INSTALLER.as_bytes())
            .map_err(|e| format!("couldn't start the installer: {e}"))?;
    }
    let status = installer.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("the installer stopped; nothing was replaced unless it said so".to_owned());
    }
    // The window's notice goes at its next check.
    if let Some(cache) = update::cache_path() {
        let _ = update::write_cache(&cache, sync::unix_now(), &latest);
    }
    if window_is_open() {
        println!("Quit the open window (q) and open it again to use {latest}.");
    }
    Ok(())
}

/// Where this binary is, so the new one replaces it.
fn install_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().and_then(std::fs::canonicalize);
    exe.ok()
        .and_then(|exe| exe.parent().map(Path::to_owned))
        .ok_or_else(|| "couldn't tell where gutenberg is installed".to_owned())
}

/// `sh` reading the installer on stdin, for `version`, into `dir`.
fn installer(version: &str, dir: &Path) -> Command {
    let mut command = Command::new("sh");
    command
        .arg("-s")
        .env("GUTENBERG_VERSION", version)
        .env("GUTENBERG_INSTALL_DIR", dir)
        .env_remove("GUTENBERG_BASE_URL")
        .stdin(Stdio::piped());
    command
}

fn window_is_open() -> bool {
    ipc::socket_path().is_some_and(|path| std::os::unix::net::UnixStream::connect(path).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installer_is_the_site_s_for_this_version_and_place() {
        assert!(INSTALLER.starts_with("#!/bin/sh"));
        assert!(INSTALLER.contains("SHA256SUMS"));
        let command = installer("0.25.0", Path::new("/home/me/.local/bin"));
        assert_eq!(command.get_program(), "sh");
        let env: Vec<_> = command.get_envs().collect();
        assert!(env.contains(&("GUTENBERG_VERSION".as_ref(), Some("0.25.0".as_ref()))));
        assert!(env.contains(&(
            "GUTENBERG_INSTALL_DIR".as_ref(),
            Some("/home/me/.local/bin".as_ref())
        )));
        // A test build's tarball location never leaks into an update.
        assert!(env.contains(&("GUTENBERG_BASE_URL".as_ref(), None)));
    }
}
