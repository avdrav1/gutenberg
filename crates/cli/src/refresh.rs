//! `gutenberg --refresh`: refresh every due source now and say how it
//! went. With a window open, its refresh thread does it, over the socket;
//! otherwise this process does.

use std::process::ExitCode;

use gutenberg_core::favicon::Icons;
use gutenberg_core::ipc;
use gutenberg_core::sync::{self, Limits};
use gutenberg_store::Store;

pub fn run() -> ExitCode {
    if let Some(path) = ipc::socket_path() {
        match ipc::send(&path, ipc::Command::Refresh) {
            // The window shows what failed in its sources pane.
            Ok(summary) => {
                println!("{summary}");
                return ExitCode::SUCCESS;
            }
            Err(ipc::Error::NotRunning) => {}
            Err(err) => {
                eprintln!("gutenberg: refresh: {err}");
                return ExitCode::FAILURE;
            }
        }
    }
    match refresh() {
        Ok(summary) => {
            for (url, error) in &summary.failed {
                eprintln!("gutenberg: {url}: {error}");
            }
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("gutenberg: refresh: {message}");
            ExitCode::FAILURE
        }
    }
}

fn refresh() -> Result<sync::Summary, String> {
    let path = Store::default_path().map_err(|e| e.to_string())?;
    let mut store = Store::open(&path).map_err(|e| e.to_string())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime
        .block_on(sync::refresh(
            &mut store,
            sync::unix_now(),
            &Limits::default(),
            Some(&Icons::beside(&path)),
        ))
        .map_err(|e| e.to_string())
}
