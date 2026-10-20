use gutenberg_core::config::Config;

fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("gutenberg {}", env!("CARGO_PKG_VERSION"));
        return std::process::ExitCode::SUCCESS;
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
    match gutenberg_ui::run(config, config_path) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gutenberg: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}
