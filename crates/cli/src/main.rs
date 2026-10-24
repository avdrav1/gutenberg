mod add;
mod keyword;
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
        Some("--import") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return match rest.as_slice() {
                [path] => opml::import(path),
                _ => usage(),
            };
        }
        Some("--export") if std::env::args().len() == 2 => return opml::export(),
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
    eprintln!(
        "usage: gutenberg [--version | --add <url> [--folder <name>] | --keyword <query> [--folder <name>] | --import <file.opml> | --export | --refresh]"
    );
    std::process::ExitCode::from(2)
}
