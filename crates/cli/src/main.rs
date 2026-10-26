mod add;
mod crash;
mod key;
mod keyword;
mod mcp;
mod opml;
mod refresh;
mod unread;
mod update;

use std::sync::{Arc, OnceLock};

use gutenberg_core::config::Config;
use gutenberg_core::ipc;
use gutenberg_core::sync::{Limits, Refresher};

/// How a refresh from the command line runs: with the rules in
/// `config.toml`, or none if it can't be read.
fn limits() -> Limits {
    let rules = match Config::default_path().as_deref().map(Config::load) {
        Some(Ok(config)) => config.rules,
        Some(Err(err)) => {
            eprintln!("gutenberg: {err}; no rules applied");
            Vec::new()
        }
        None => Vec::new(),
    };
    Limits {
        rules: gutenberg_core::rules::Rules::new(&rules),
        ..Limits::default()
    }
}

fn main() -> std::process::ExitCode {
    crash::install();
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
        Some("--update") if std::env::args().len() == 2 => return update::run(),
        Some("--unread") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return match rest.as_slice() {
                [] => unread::run(false),
                [flag] if flag == "--json" => unread::run(true),
                _ => usage(),
            };
        }
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
    let (outbox, inbox) = gutenberg_ui::inbox();
    let refresher = Arc::new(OnceLock::new());
    // One window: the socket is taken before anything else starts, and a
    // second launch asks the open window to come forward instead.
    let _listening = match listen(outbox, refresher.clone()) {
        Listening::Yes(listener) => Some(listener),
        Listening::Elsewhere => return focus_open_window(),
        Listening::No => None,
    };
    // Refresh runs on its own thread with its own store connection.
    if let Ok(store_path) = gutenberg_store::Store::default_path() {
        let _ = refresher.set(gutenberg_core::sync::spawn(
            store_path,
            config_path.clone(),
            &config,
        ));
    }
    let store_path = gutenberg_store::Store::default_path().ok();
    let status = refresher.get().map(Refresher::status);
    match gutenberg_ui::run(config, config_path, store_path, inbox, status) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gutenberg: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

enum Listening {
    Yes(ipc::Listener),
    /// Another window has the socket.
    Elsewhere,
    /// No runtime directory, or the socket couldn't be made: the window
    /// opens without it.
    No,
}

/// Opens the window's socket: theme reloads and coming forward go to the
/// window, refreshes to the refresh thread once it has started.
fn listen(outbox: gutenberg_ui::Outbox, refresher: Arc<OnceLock<Refresher>>) -> Listening {
    let Some(path) = ipc::socket_path() else {
        return Listening::No;
    };
    let on_command = move |command| match command {
        ipc::Command::ReloadTheme | ipc::Command::Focus => match outbox.send(command) {
            true => Ok(String::new()),
            false => Err("the window is closing".to_owned()),
        },
        ipc::Command::Refresh => match refresher.get() {
            Some(refresher) => refresher.now().map(|summary| summary.to_string()),
            None => Err("the window has no store to refresh".to_owned()),
        },
    };
    match ipc::listen(&path, on_command) {
        Ok(listener) => Listening::Yes(listener),
        Err(ipc::Error::InUse(_)) => Listening::Elsewhere,
        Err(err) => {
            eprintln!("gutenberg: {err}");
            Listening::No
        }
    }
}

/// A second launch: the open window comes forward, and this one exits.
fn focus_open_window() -> std::process::ExitCode {
    let sent = ipc::socket_path()
        .ok_or(ipc::Error::NotRunning)
        .and_then(|path| ipc::send(&path, ipc::Command::Focus));
    match sent {
        Ok(_) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gutenberg: a window is open but didn't come forward: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn usage() -> std::process::ExitCode {
    eprintln!(
        "usage: gutenberg [--version | --add <url> [--folder <name>] | --keyword <query> [--folder <name>] | --mcp <command or URL> [--tool <name> [--args <json>]] [--secret <NAME>] [--folder <name>] | --key <NAME> | --import <file.opml> | --export | --unread [--json] | --refresh | --reload-theme | --update]"
    );
    std::process::ExitCode::from(2)
}

/// `gutenberg --reload-theme`: the open window reads the Omarchy theme and
/// the desktop font again. Omarchy's `theme-set` and `font-set` hooks run
/// it. With no window open there's nothing to reload, which isn't a
/// failure.
fn reload_theme() -> std::process::ExitCode {
    let Some(path) = ipc::socket_path() else {
        eprintln!("gutenberg: no runtime directory ($XDG_RUNTIME_DIR) for the socket");
        return std::process::ExitCode::FAILURE;
    };
    match ipc::send(&path, ipc::Command::ReloadTheme) {
        Ok(_) => std::process::ExitCode::SUCCESS,
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
