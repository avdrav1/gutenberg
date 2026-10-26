//! Is there a newer Gutenberg? GitHub's latest release, asked at most once
//! a day and remembered in `$XDG_CACHE_HOME/gutenberg/latest-release` as
//! `<unix seconds> <version>`. `gutenberg --update` installs it;
//! `[updates] check = false` stops the asking.

use std::path::{Path, PathBuf};

use gutenberg_sources::feed;
use serde::Deserialize;

/// GitHub's answer for the newest release that isn't a draft or a
/// prerelease.
pub const LATEST: &str = "https://api.github.com/repos/avdrav1/gutenberg/releases/latest";

/// How long an answer is trusted before GitHub is asked again.
pub const EVERY: i64 = 24 * 60 * 60;

/// The running build's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Plenty for the release JSON, which lists its assets.
const MAX_ANSWER: usize = 1 << 20;

/// `$XDG_CACHE_HOME/gutenberg/latest-release`.
pub fn cache_path() -> Option<PathBuf> {
    crate::cache::Cache::default_dir().map(|dir| dir.join("latest-release"))
}

/// The newest release's version, such as `0.25.0`, from `url` ([`LATEST`]).
pub async fn latest(client: &feed::Client, url: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
    }
    let body = feed::download(client, url, MAX_ANSWER)
        .await
        .map_err(|e| e.to_string())?;
    let release: Release =
        serde_json::from_slice(&body).map_err(|e| format!("GitHub's answer: {e}"))?;
    let version = release.tag_name.trim_start_matches('v').to_owned();
    match parse(&version) {
        Some(_) => Ok(version),
        None => Err(format!("GitHub's latest release is {:?}", release.tag_name)),
    }
}

/// Whether `candidate` is a later release than `current`. Either not being
/// `x.y.z` is no.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse(candidate), parse(current)), (Some(a), Some(b)) if a > b)
}

fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().split('.').map(|p| p.parse::<u64>().ok());
    let parsed = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(parsed)
}

/// What the cache says: when GitHub was last asked and what it answered.
pub fn read_cache(path: &Path) -> Option<(i64, String)> {
    let text = std::fs::read_to_string(path).ok()?;
    let (at, version) = text.trim().split_once(' ')?;
    parse(version)?;
    Some((at.parse().ok()?, version.to_owned()))
}

pub fn write_cache(path: &Path, at: i64, version: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, format!("{at} {version}\n"))
}

/// The newest release, from the cache if GitHub was asked in the last day,
/// else from `url`, then cached. `None` when GitHub can't be reached and
/// nothing is cached.
pub async fn check(client: &feed::Client, url: &str, cache: &Path, now: i64) -> Option<String> {
    let cached = read_cache(cache);
    if let Some((at, version)) = &cached {
        if (0..EVERY).contains(&(now - at)) {
            return Some(version.clone());
        }
    }
    match latest(client, url).await {
        Ok(version) => {
            let _ = write_cache(cache, now, &version);
            Some(version)
        }
        // Offline: ask again at the next refresh, and say what we knew.
        Err(_) => cached.map(|(_, version)| version),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.25.0", "0.24.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.1"));
        assert!(!is_newer("0.24.0", "0.24.0"));
        assert!(!is_newer("0.23.9", "0.24.0"));
        assert!(!is_newer("0.25", "0.24.0"));
        assert!(!is_newer("0.25.0-rc1", "0.24.0"));
        assert!(!is_newer("", "0.24.0"));
    }

    #[test]
    fn the_cache_round_trips_and_ignores_junk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gutenberg/latest-release");
        assert_eq!(read_cache(&path), None);
        write_cache(&path, 1_000, "0.25.0").unwrap();
        assert_eq!(read_cache(&path), Some((1_000, "0.25.0".into())));
        std::fs::write(&path, "soon v1").unwrap();
        assert_eq!(read_cache(&path), None);
    }
}
