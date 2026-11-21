//! Opening a link in the browser. `xdg-open` (`open` on macOS) runs on a
//! thread of its own, so the window never waits on it.
//!
//! Under Hyprland a Chromium-family browser hands a link to the window it
//! was last active in only when that window is on the current workspace;
//! otherwise it opens a new window here. So before the link goes out, the
//! browser's most recently focused window is focused, which brings its
//! workspace along and the link lands there as a tab.

use std::process::{Command, Stdio};

#[cfg(target_os = "linux")]
use crate::hyprland::{self, Client};

/// Opens `url` in the default browser without blocking the caller.
pub fn open(url: &str) {
    let url = url.to_owned();
    std::thread::spawn(move || {
        #[cfg(target_os = "linux")]
        if hyprland::running() {
            focus_browser();
        }
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let spawned = Command::new(opener)
            .arg(&url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                let _ = child.wait();
            }
            Err(err) => eprintln!("gutenberg: opening {url}: {err}"),
        }
    });
}

/// Focuses the default browser's last-used window, if it has one open.
/// Any failure leaves focus where it was; the link still opens.
#[cfg(target_os = "linux")]
fn focus_browser() {
    let Some(entry) = default_browser() else {
        return;
    };
    if let Some(address) = browser_window(&hyprland::clients(), &entry) {
        hyprland::focus(&address);
    }
}

/// The desktop entry of the default browser, as Omarchy's launcher picks it,
/// or as `xdg-settings` does elsewhere. Omarchy's routing entry names no
/// browser, so it counts as none.
#[cfg(target_os = "linux")]
fn default_browser() -> Option<String> {
    let asks: [(&str, &[&str]); 2] = [
        ("omarchy-cmd-default-browser", &[]),
        ("xdg-settings", &["get", "default-web-browser"]),
    ];
    asks.into_iter().find_map(|(program, args)| {
        let entry = hyprland::output(program, args)?.trim().to_owned();
        (!entry.is_empty() && entry != "omarchy-browser.desktop").then_some(entry)
    })
}

/// The address of the most recently focused window that belongs to the
/// browser in desktop entry `entry`, from `hyprctl clients -j`. A browser's
/// window class is its entry's name: `chromium.desktop` is `chromium`,
/// `google-chrome.desktop` is `Google-chrome`.
#[cfg(target_os = "linux")]
fn browser_window(clients: &[Client], entry: &str) -> Option<String> {
    let class = entry.strip_suffix(".desktop").unwrap_or(entry);
    clients
        .iter()
        .filter(|client| client.class.eq_ignore_ascii_case(class))
        .min_by_key(|client| client.focus_history_id)
        .map(|client| client.address.clone())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    const CLIENTS: &str = r#"[
        {"address": "0x1", "class": "com.mitchellh.ghostty", "focusHistoryID": 0},
        {"address": "0x2", "class": "chromium", "focusHistoryID": 5},
        {"address": "0x3", "class": "chromium", "focusHistoryID": 2},
        {"address": "0x4", "class": "Google-chrome", "focusHistoryID": 1}
    ]"#;

    fn clients() -> Vec<Client> {
        hyprland::parse_clients(CLIENTS).unwrap()
    }

    #[test]
    fn the_browsers_last_focused_window_is_picked() {
        assert_eq!(
            browser_window(&clients(), "chromium.desktop").as_deref(),
            Some("0x3")
        );
    }

    #[test]
    fn the_class_matches_the_entry_in_any_case() {
        assert_eq!(
            browser_window(&clients(), "google-chrome.desktop").as_deref(),
            Some("0x4")
        );
    }

    #[test]
    fn a_browser_with_no_window_open_picks_nothing() {
        assert_eq!(browser_window(&clients(), "firefox.desktop"), None);
        assert!(hyprland::parse_clients("not json").is_none());
    }
}
