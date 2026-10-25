//! `gutenberg --key <NAME>`: save a key for MCP servers in the keyring. The
//! key comes from stdin, typed without showing at a terminal or piped in
//! from a password manager; it never goes on the command line, where
//! `ps` and shell history would keep it.

use std::io::{BufRead, IsTerminal};
use std::process::ExitCode;

use gutenberg_core::secrets;
use nix::sys::termios::{self, LocalFlags, SetArg};

pub fn run(name: &str) -> ExitCode {
    match save(name) {
        Ok(()) => {
            println!(
                "Saved {name} in the keyring; servers that need it use it from the next refresh"
            );
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("gutenberg: {name}: {message}");
            ExitCode::FAILURE
        }
    }
}

fn save(name: &str) -> Result<(), String> {
    if !secrets::valid_name(name) {
        return Err("a key's name is letters, digits and _, like JINA_API_KEY".into());
    }
    let value = read_hidden(&format!("{name}: "))?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(secrets::save(name, &value))
}

/// One line from stdin; at a terminal, after `prompt` and without echo.
fn read_hidden(prompt: &str) -> Result<String, String> {
    let stdin = std::io::stdin();
    let terminal = stdin.is_terminal();
    let saved = if terminal {
        eprint!("{prompt}");
        let saved = termios::tcgetattr(&stdin).map_err(|e| e.to_string())?;
        let mut quiet = saved.clone();
        quiet.local_flags.remove(LocalFlags::ECHO);
        termios::tcsetattr(&stdin, SetArg::TCSANOW, &quiet).map_err(|e| e.to_string())?;
        Some(saved)
    } else {
        None
    };
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    if let Some(saved) = saved {
        let _ = termios::tcsetattr(&stdin, SetArg::TCSANOW, &saved);
        eprintln!();
    }
    read.map_err(|e| e.to_string())?;
    Ok(line.trim().to_owned())
}
