//! `gutenberg --refresh`: refresh every due feed now and say how it went.
//! Runs in this process until the app's socket (S7.3) can take it.

use std::process::ExitCode;

use gutenberg_core::sync::{self, Limits};
use gutenberg_store::Store;

pub fn run() -> ExitCode {
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
        ))
        .map_err(|e| e.to_string())
}
