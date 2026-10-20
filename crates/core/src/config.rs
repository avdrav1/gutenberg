//! `config.toml`: loaded at launch, reloaded when it changes on disk.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use serde::Deserialize;

/// Every key is optional; a missing file is the same as an empty one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub layout: Layout,
}

/// Relative pane widths. 0 hides a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layout {
    pub sources: u16,
    pub list: u16,
    pub article: u16,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            sources: 2,
            list: 4,
            article: 6,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: {}", path.display(), source.message().trim())]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{}: [layout] needs at least one pane wider than 0", path.display())]
    NoPanes { path: PathBuf },
    #[error("watch {}: {source}", path.display())]
    Watch {
        path: PathBuf,
        source: notify::Error,
    },
}

impl Config {
    /// `$XDG_CONFIG_HOME/gutenberg/config.toml`.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "gutenberg")
            .map(|dirs| dirs.config_dir().join("config.toml"))
    }

    pub fn load(path: &Path) -> Result<Self, Error> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(path, &text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::Read {
                path: path.to_owned(),
                source,
            }),
        }
    }

    fn parse(path: &Path, text: &str) -> Result<Self, Error> {
        let config: Self = toml::from_str(text).map_err(|source| Error::Parse {
            path: path.to_owned(),
            source,
        })?;
        let l = config.layout;
        if l.sources == 0 && l.list == 0 && l.article == 0 {
            return Err(Error::NoPanes {
                path: path.to_owned(),
            });
        }
        Ok(config)
    }
}

/// Keeps the file watched until dropped.
pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
}

/// Editors fire several events per save; wait this long for them to settle.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// Calls `on_change` with the re-read config after each change to `path`,
/// on a background thread. Watches the directory, not the file, because
/// editors save by replacing the file. Creates the directory if needed.
pub fn watch(
    path: &Path,
    on_change: impl Fn(Result<Config, Error>) + Send + 'static,
) -> Result<WatchHandle, Error> {
    let watch_err = |source| Error::Watch {
        path: path.to_owned(),
        source,
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|source| Error::Read {
        path: dir.to_owned(),
        source,
    })?;

    let (tx, rx) = mpsc::channel();
    let name = path.file_name().map(ToOwned::to_owned);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event {
            if event.paths.iter().any(|p| p.file_name() == name.as_deref()) {
                let _ = tx.send(());
            }
        }
    })
    .map_err(watch_err)?;
    watcher
        .watch(dir, RecursiveMode::NonRecursive)
        .map_err(watch_err)?;

    let path = path.to_owned();
    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            // Ends when the watcher, and so `tx`, is dropped.
            while rx.recv().is_ok() {
                while rx.recv_timeout(DEBOUNCE).is_ok() {}
                on_change(Config::load(&path));
            }
        })
        .map_err(|source| Error::Read {
            path: dir.to_owned(),
            source,
        })?;

    Ok(WatchHandle { _watcher: watcher })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Config, Error> {
        Config::parse(Path::new("config.toml"), text)
    }

    #[test]
    fn empty_file_is_the_defaults() {
        assert_eq!(parse("").unwrap(), Config::default());
    }

    #[test]
    fn missing_keys_keep_their_defaults() {
        let config = parse("[layout]\nlist = 9").unwrap();
        assert_eq!(
            config.layout,
            Layout {
                list: 9,
                ..Layout::default()
            }
        );
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(matches!(
            parse("[layout]\nlsit = 9"),
            Err(Error::Parse { .. })
        ));
        assert!(matches!(parse("[theme]"), Err(Error::Parse { .. })));
    }

    #[test]
    fn hiding_every_pane_is_rejected() {
        let err = parse("[layout]\nsources = 0\nlist = 0\narticle = 0").unwrap_err();
        assert!(matches!(err, Error::NoPanes { .. }));
    }

    #[test]
    fn bad_toml_is_one_line_of_error() {
        let err = parse("[layout\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("config.toml: "), "{msg}");
    }

    #[test]
    fn missing_file_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join("config.toml")).unwrap();
        assert_eq!(config, Config::default());
    }
}
