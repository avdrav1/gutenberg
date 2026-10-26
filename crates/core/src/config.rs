//! `config.toml`: loaded at launch, reloaded when it changes on disk.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use serde::Deserialize;

use crate::rules::Rule;

/// Every key is optional; a missing file is the same as an empty one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub layout: Layout,
    pub refresh: Refresh,
    pub font: Font,
    pub keys: Keys,
    pub cache: Cache,
    /// `[[rules]]`: see [`crate::rules`].
    pub rules: Vec<Rule>,
    pub updates: Updates,
}

/// Telling you about new releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Updates {
    /// Ask GitHub once a day whether a newer Gutenberg is out, and say so
    /// in the key bar.
    pub check: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Self { check: true }
    }
}

/// Article images and reader pages kept on disk, in
/// `$XDG_CACHE_HOME/gutenberg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Cache {
    /// Most megabytes kept; past it, what was read longest ago goes.
    pub megabytes: u32,
}

impl Default for Cache {
    fn default() -> Self {
        Self { megabytes: 200 }
    }
}

impl Cache {
    pub fn bytes(self) -> u64 {
        u64::from(self.megabytes) << 20
    }
}

/// Help with the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keys {
    /// The bar along the bottom with the keys for what's on screen.
    pub hints: bool,
}

impl Default for Keys {
    fn default() -> Self {
        Self { hints: true }
    }
}

/// Text. The face is the desktop's (fontconfig's `monospace`); only the
/// size is set here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Font {
    /// Pixels for list and sidebar text; the rest of the window scales with
    /// it. Between [`Font::MIN_SIZE`] and [`Font::MAX_SIZE`].
    pub size: u16,
}

impl Font {
    pub const DEFAULT_SIZE: u16 = 14;
    pub const MIN_SIZE: u16 = 8;
    pub const MAX_SIZE: u16 = 32;
}

impl Default for Font {
    fn default() -> Self {
        Self {
            size: Self::DEFAULT_SIZE,
        }
    }
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

/// Background refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Refresh {
    /// Minutes between refreshes; 0 turns background refresh off.
    pub minutes: u32,
}

impl Default for Refresh {
    fn default() -> Self {
        Self { minutes: 30 }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: {}", path.display(), one_line(source.message()))]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{}: [layout] needs at least one pane wider than 0", path.display())]
    NoPanes { path: PathBuf },
    #[error(
        "{}: [font] size must be from {} to {}",
        path.display(),
        Font::MIN_SIZE,
        Font::MAX_SIZE
    )]
    FontSize { path: PathBuf },
    #[error("{}: a rule's match is empty", path.display())]
    EmptyRule { path: PathBuf },
    #[error("watch {}: {source}", path.display())]
    Watch {
        path: PathBuf,
        source: notify::Error,
    },
}

/// toml's messages span lines ("invalid table header\nexpected ..."); the
/// error is shown as one.
fn one_line(message: &str) -> String {
    message
        .trim()
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("; ")
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
        if !(Font::MIN_SIZE..=Font::MAX_SIZE).contains(&config.font.size) {
            return Err(Error::FontSize {
                path: path.to_owned(),
            });
        }
        if config.rules.iter().any(|rule| rule.words.trim().is_empty()) {
            return Err(Error::EmptyRule {
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
    fn the_cache_defaults_to_200_megabytes() {
        assert_eq!(parse("").unwrap().cache.bytes(), 200 << 20);
        assert_eq!(
            parse("[cache]\nmegabytes = 50").unwrap().cache.megabytes,
            50
        );
    }

    #[test]
    fn refresh_interval_defaults_to_30_minutes() {
        assert_eq!(parse("").unwrap().refresh.minutes, 30);
        assert_eq!(parse("[refresh]\nminutes = 0").unwrap().refresh.minutes, 0);
    }

    #[test]
    fn key_hints_show_unless_turned_off() {
        assert!(parse("").unwrap().keys.hints);
        assert!(!parse("[keys]\nhints = false").unwrap().keys.hints);
    }

    #[test]
    fn release_checks_are_on_unless_turned_off() {
        assert!(parse("").unwrap().updates.check);
        assert!(!parse("[updates]\ncheck = false").unwrap().updates.check);
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
    fn font_size_is_14_and_bounded() {
        assert_eq!(parse("").unwrap().font.size, 14);
        assert_eq!(parse("[font]\nsize = 18").unwrap().font.size, 18);
        assert_eq!(parse("[font]\nsize = 8").unwrap().font.size, 8);
        assert_eq!(parse("[font]\nsize = 32").unwrap().font.size, 32);
        for size in [0, 7, 33] {
            let err = parse(&format!("[font]\nsize = {size}")).unwrap_err();
            assert!(matches!(err, Error::FontSize { .. }), "{size}");
        }
    }

    #[test]
    fn rules_are_read_in_order() {
        use crate::rules::{Action, Rule};
        let config = parse(
            "[[rules]]\nmatch = \"sponsored\"\naction = \"mute\"\n\n\
             [[rules]]\nmatch = \"Linux 7\"\nin = \"Phoronix\"\naction = \"star\"",
        )
        .unwrap();
        assert_eq!(
            config.rules,
            [
                Rule {
                    words: "sponsored".into(),
                    within: None,
                    action: Action::Mute,
                },
                Rule {
                    words: "Linux 7".into(),
                    within: Some("Phoronix".into()),
                    action: Action::Star,
                },
            ]
        );
        assert!(parse("").unwrap().rules.is_empty());
    }

    #[test]
    fn a_rule_needs_words_and_a_known_action() {
        let err = parse("[[rules]]\nmatch = \" \"\naction = \"star\"").unwrap_err();
        assert!(matches!(err, Error::EmptyRule { .. }));
        for bad in [
            "[[rules]]\nmatch = \"x\"\naction = \"delete\"",
            "[[rules]]\nmatch = \"x\"",
            "[[rules]]\nmatch = \"x\"\naction = \"star\"\nwhere = \"y\"",
        ] {
            assert!(matches!(parse(bad), Err(Error::Parse { .. })), "{bad}");
        }
    }

    #[test]
    fn bad_toml_is_one_line_of_error() {
        let err = parse("[layout\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with("config.toml: "), "{msg}");
        assert!(!msg.contains('\n'), "{msg}");
    }

    #[test]
    fn missing_file_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join("config.toml")).unwrap();
        assert_eq!(config, Config::default());
    }
}
