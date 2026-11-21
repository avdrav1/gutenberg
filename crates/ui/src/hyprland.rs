//! Asking Hyprland for windows and focus with `hyprctl`. Every call here
//! runs a child process, so callers run them on a thread of their own,
//! never on the UI thread.

use std::process::{Command, Stdio};

use serde::Deserialize;

/// Whether the window runs under Hyprland.
pub fn running() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

/// What `program` printed, when it ran and succeeded.
pub fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A window, as `hyprctl clients -j` lists it.
#[derive(Debug, Deserialize)]
pub struct Client {
    pub address: String,
    pub class: String,
    #[serde(default)]
    pub pid: i64,
    #[serde(rename = "focusHistoryID")]
    pub focus_history_id: i64,
}

/// Every window, or none if `hyprctl` can't say.
pub fn clients() -> Vec<Client> {
    output("hyprctl", &["clients", "-j"])
        .and_then(|json| parse_clients(&json))
        .unwrap_or_default()
}

pub fn parse_clients(json: &str) -> Option<Vec<Client>> {
    serde_json::from_str(json).ok()
}

/// Focuses the window at `address`, which brings its workspace along.
pub fn focus(address: &str) {
    // Hyprland 0.55 and later take Lua; older ones the classic dispatcher.
    let lua = format!("hl.dsp.focus({{ window = \"address:{address}\" }})");
    let focused = output("hyprctl", &["dispatch", &lua]).is_some_and(|reply| reply.trim() == "ok");
    if !focused {
        let _ = output(
            "hyprctl",
            &["dispatch", "focuswindow", &format!("address:{address}")],
        );
    }
}

/// Focuses this process's window. Wayland grants a window that asks for
/// itself only with an activation token, which a socket command (a second
/// launch, Open on a notification) doesn't bring; Hyprland does it anyway.
pub fn focus_own_window() {
    if let Some(address) = own_window(&clients(), std::process::id()) {
        focus(&address);
    }
}

/// The window of process `pid`.
fn own_window(clients: &[Client], pid: u32) -> Option<String> {
    clients
        .iter()
        .find(|c| c.pid == i64::from(pid))
        .map(|c| c.address.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_window_is_found_by_its_pid() {
        let clients = parse_clients(
            r#"[{"address": "0x1", "class": "chromium", "pid": 10, "focusHistoryID": 0},
                {"address": "0x2", "class": "gutenberg", "pid": 42, "focusHistoryID": 3}]"#,
        )
        .unwrap();
        assert_eq!(own_window(&clients, 42).as_deref(), Some("0x2"));
        assert_eq!(own_window(&clients, 7), None);
    }
}
