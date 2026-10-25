mod add;
mod key;
mod keyword;
mod mcp;
mod opml;
mod refresh;

use gutenberg_core::config::Config;

fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("gutenberg {}", env!("CARGO_PKG_VERSION"));
        return std::process::ExitCode::SUCCESS;
    }
    match std::env::args().nth(1).as_deref() {
        Some("--add") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return match rest.as_slice() {
                [url] => add::run(url, None),
                [url, flag, folder] | [flag, folder, url] if flag == "--folder" => {
                    add::run(url, Some(folder))
                }
                _ => usage(),
            };
        }
        Some("--keyword") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return match rest.as_slice() {
                [query] => keyword::run(query, None),
                [query, flag, folder] | [flag, folder, query] if flag == "--folder" => {
                    keyword::run(query, Some(folder))
                }
                _ => usage(),
            };
        }
        Some("--mcp") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return match mcp::Options::parse(&rest) {
                Some(options) => mcp::run(&options),
                None => usage(),
            };
        }
        Some("--key") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return match rest.as_slice() {
                [name] => key::run(name),
                _ => usage(),
            };
        }
        Some("--import") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return match rest.as_slice() {
                [path] => opml::import(path),
                _ => usage(),
            };
        }
        Some("--export") if std::env::args().len() == 2 => return opml::export(),
        Some("--refresh") => return refresh::run(),
        Some("--reload-theme") if std::env::args().len() == 2 => return reload_theme(),
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
    eprintln!(
        "usage: gutenberg [--version | --add <url> [--folder <name>] | --keyword <query> [--folder <name>] | --mcp <command or URL> [--tool <name> [--args <json>]] [--secret <NAME>] [--folder <name>] | --key <NAME> | --import <file.opml> | --export | --refresh | --reload-theme]"
    );
    std::process::ExitCode::from(2)
}

/// `gutenberg --reload-theme`: the open window reads the Omarchy theme and
/// the desktop font again. Omarchy's `theme-set` and `font-set` hooks run
/// it. With no window open there's nothing to reload, which isn't a
/// failure.
fn reload_theme() -> std::process::ExitCode {
    use gutenberg_core::ipc;
    let Some(path) = ipc::socket_path() else {
        eprintln!("gutenberg: no runtime directory ($XDG_RUNTIME_DIR) for the socket");
        return std::process::ExitCode::FAILURE;
    };
    match ipc::send(&path, ipc::Command::ReloadTheme) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err @ ipc::Error::NotRunning) => {
            eprintln!("gutenberg: {err}");
            std::process::ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("gutenberg: reload-theme: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}
