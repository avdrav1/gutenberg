//! `gutenberg --open <article>`: an open window shows the article; with
//! none open, the window starts on it (S10.3). Open on a pop-up from a
//! timer's refresh runs it through [`launch`].

use std::process::{Command, ExitCode, Stdio};

use gutenberg_core::ipc;

/// The article id after `--open`.
pub fn parse(args: &[String]) -> Option<i64> {
    match args {
        [id] => id.parse().ok().filter(|id| *id > 0),
        _ => None,
    }
}

/// Shows `article` in the open window. `None` when no window answers, so
/// the caller starts one on it.
pub fn in_open_window(article: i64) -> Option<ExitCode> {
    let path = ipc::socket_path()?;
    match ipc::send(&path, ipc::Command::Open(article)) {
        Ok(_) => Some(ExitCode::SUCCESS),
        Err(ipc::Error::NotRunning) => None,
        Err(err) => {
            eprintln!("gutenberg: open: {err}");
            Some(ExitCode::FAILURE)
        }
    }
}

/// Runs `gutenberg --open <article>` apart from this process, which is
/// about to exit: on Hyprland the compositor starts it, so it isn't a child
/// of the timer's service; elsewhere it starts in a process group of its
/// own (and the service is `KillMode=process`).
pub fn launch(article: i64) {
    let Ok(exe) = std::env::current_exe() else {
        return eprintln!("gutenberg: open: can't find this program");
    };
    #[cfg(target_os = "linux")]
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        if let Some(path) = exe.to_str() {
            let line = format!("{} --open {article}", shell_quote(path));
            if hyprland_exec(&line) {
                return;
            }
        }
    }
    let mut command = Command::new(exe);
    command
        .args(["--open", &article.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    if let Err(err) = command.spawn() {
        eprintln!("gutenberg: open: {err}");
    }
}

/// Asks Hyprland to run `line` in a shell: the Lua dispatcher (0.55 on),
/// then the older one.
#[cfg(target_os = "linux")]
fn hyprland_exec(line: &str) -> bool {
    let ran = |args: &[&str]| {
        Command::new("hyprctl")
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|out| out.status.success() && out.stdout.trim_ascii() == b"ok")
    };
    ran(&[
        "dispatch",
        &format!("hl.dsp.exec_cmd({})", lua_string(line)),
    ]) || ran(&["dispatch", "exec", line])
}

/// `text` as one word for `sh`.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// `text` as a Lua string literal.
#[cfg(target_os = "linux")]
fn lua_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', r"\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn open_takes_one_article_id() {
        assert_eq!(parse(&args(&["42"])), Some(42));
        for bad in [&[][..], &["x"], &["-3"], &["0"], &["1", "2"]] {
            assert_eq!(parse(&args(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn a_path_is_quoted_for_the_shell_and_for_lua() {
        let line = format!("{} --open 7", shell_quote("/home/a b/it's/gutenberg"));
        assert_eq!(line, r#"'/home/a b/it'\''s/gutenberg' --open 7"#);
        #[cfg(target_os = "linux")]
        assert_eq!(
            lua_string(&line),
            r#""'/home/a b/it'\\''s/gutenberg' --open 7""#
        );
    }
}
