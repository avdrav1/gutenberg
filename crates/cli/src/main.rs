mod add;
mod refresh;

use gutenberg_core::config::Config;

fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("gutenberg {}", env!("CARGO_PKG_VERSION"));
        return std::process::ExitCode::SUCCESS;
    }
    match std::env::args().nth(1).as_deref() {
        Some("--add") => {
            return match std::env::args().nth(2) {
                Some(url) => add::run(&url),
                None => usage(),
            }
        }
        Some("--refresh") => return refresh::run(),
        Some(_) => return usage(),
        None => {}
    }
    let config_path = Config::default_path();
    let config = match config_path.as_deref().map(Config::load) {
        Some(Ok(config)) => config,
        Some(Err(err)) => {
            eprintln!("gutenberg: {err}; using defaults");
            Config::default()
        }
        None => Config::default(),
    };
    // Refresh runs on its own thread with its own store connection.
    if let Ok(store_path) = gutenberg_store::Store::default_path() {
        gutenberg_core::sync::spawn(store_path, config_path.clone(), &config);
    }
    let store_path = gutenberg_store::Store::default_path().ok();
    match gutenberg_ui::run(config, config_path, store_path) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gutenberg: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn usage() -> std::process::ExitCode {
    eprintln!("usage: gutenberg [--version | --add <url> | --refresh]");
    std::process::ExitCode::from(2)
}
