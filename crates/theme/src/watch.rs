//! The inotify half of live reload: calls back when Omarchy swaps the
//! current theme or when fontconfig's settings change. The `theme-set` and
//! `font-set` hooks (installed by `install.sh`) are the other half; this
//! covers installs without them.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

/// `omarchy-theme-set` removes the theme directory, moves the next one in
/// and writes `theme.name`, all within a few milliseconds; one reload
/// follows them all.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// A watched directory and the entries in it that matter; empty means any.
pub struct Watched {
    pub dir: PathBuf,
    pub names: &'static [&'static str],
}

/// Keeps the watches until dropped.
pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
}

/// What a running window watches:
/// - `$XDG_STATE_HOME/omarchy/current`, where the theme is swapped (Omarchy
///   3 and later), and `$XDG_CONFIG_HOME/omarchy/current` (older);
/// - `$XDG_CONFIG_HOME/fontconfig`, where `omarchy-font-set` writes
///   `fonts.conf`.
///
/// Directories that don't exist are skipped: one created later, such as
/// the first `fonts.conf`, is only seen through the hooks.
#[cfg(target_os = "linux")]
pub fn watched() -> Vec<Watched> {
    let Some(dirs) = directories::BaseDirs::new() else {
        return Vec::new();
    };
    let state = dirs
        .state_dir()
        .map_or_else(|| dirs.home_dir().join(".local/state"), Path::to_path_buf);
    vec![
        Watched {
            dir: state.join("omarchy/current"),
            names: &["theme", "theme.name"],
        },
        Watched {
            dir: dirs.config_dir().join("omarchy/current"),
            names: &["theme"],
        },
        Watched {
            dir: dirs.config_dir().join("fontconfig"),
            names: &[],
        },
    ]
}

/// Nothing: macOS has neither Omarchy nor fontconfig, so there is no
/// directory whose change would mean a new theme or font. The system's
/// light or dark switch reaches the window through iced instead.
#[cfg(target_os = "macos")]
pub fn watched() -> Vec<Watched> {
    Vec::new()
}

/// Calls `on_change` on a background thread after each settled change in
/// any of `watched` that exists.
pub fn watch(
    watched: Vec<Watched>,
    on_change: impl Fn() + Send + 'static,
) -> notify::Result<WatchHandle> {
    let watched: Vec<Watched> = watched.into_iter().filter(|w| w.dir.is_dir()).collect();
    let (tx, rx) = mpsc::channel();
    let dirs: Vec<(PathBuf, &'static [&'static str])> =
        watched.iter().map(|w| (w.dir.clone(), w.names)).collect();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event {
            if event.paths.iter().any(|p| matters(&dirs, p)) {
                let _ = tx.send(());
            }
        }
    })?;
    for w in &watched {
        watcher.watch(&w.dir, RecursiveMode::NonRecursive)?;
    }
    std::thread::Builder::new()
        .name("theme-watch".into())
        .spawn(move || {
            // Ends when the watcher, and so `tx`, is dropped.
            while rx.recv().is_ok() {
                while rx.recv_timeout(DEBOUNCE).is_ok() {}
                on_change();
            }
        })
        .map_err(notify::Error::io)?;
    Ok(WatchHandle { _watcher: watcher })
}

/// Whether `path` is one of the entries watched in its directory.
fn matters(dirs: &[(PathBuf, &'static [&'static str])], path: &Path) -> bool {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    dirs.iter()
        .any(|(dir, names)| parent == dir && (names.is_empty() || names.iter().any(|n| *n == name)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    fn counter() -> (Arc<Mutex<usize>>, impl Fn() + Send + 'static) {
        let count = Arc::new(Mutex::new(0));
        let seen = count.clone();
        (count, move || *seen.lock().unwrap() += 1)
    }

    fn wait_for(count: &Mutex<usize>, n: usize) -> Option<Duration> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            if *count.lock().unwrap() >= n {
                return Some(start.elapsed());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn a_theme_swap_calls_back_once_within_a_second() {
        let tmp = tempfile::tempdir().unwrap();
        // Events name the real path: on macOS the temp dir is under
        // `/var`, a symlink to `/private/var`, and FSEvents reports the
        // latter. On Linux `/tmp` is no symlink and this changes nothing.
        let current = tmp.path().canonicalize().unwrap().join("omarchy/current");
        std::fs::create_dir_all(current.join("theme")).unwrap();
        let (count, on_change) = counter();
        let names: &'static [&'static str] = &["theme", "theme.name"];
        let _handle = watch(
            vec![Watched {
                dir: current.clone(),
                names,
            }],
            on_change,
        )
        .unwrap();
        // FSEvents hands a new stream events from just before it started,
        // here the creation of `theme`, which counts as a reload; inotify
        // reports nothing from before the watch. Let those pass first.
        #[cfg(target_os = "macos")]
        {
            std::thread::sleep(DEBOUNCE * 3);
            *count.lock().unwrap() = 0;
        }

        // An unrelated file in the same directory is ignored.
        std::fs::write(current.join("theme.generation"), "1").unwrap();
        std::thread::sleep(DEBOUNCE * 3);
        assert_eq!(*count.lock().unwrap(), 0);

        // What omarchy-theme-set does: build next-theme, replace theme, name it.
        let next = current.join("next-theme");
        std::fs::create_dir_all(&next).unwrap();
        std::fs::write(next.join("colors.toml"), "background = \"#000000\"").unwrap();
        std::fs::remove_dir_all(current.join("theme")).unwrap();
        std::fs::rename(&next, current.join("theme")).unwrap();
        std::fs::write(current.join("theme.name"), "vantablack").unwrap();
        let took = wait_for(&count, 1).expect("a reload");
        assert!(took < Duration::from_secs(1), "{took:?}");
        std::thread::sleep(DEBOUNCE * 3);
        assert_eq!(*count.lock().unwrap(), 1, "one reload for the whole swap");
    }

    #[test]
    fn missing_directories_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, on_change) = counter();
        let gone = Watched {
            dir: tmp.path().join("nope"),
            names: &[],
        };
        assert!(watch(vec![gone], on_change).is_ok());
    }
}
