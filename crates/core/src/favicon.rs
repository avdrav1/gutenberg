//! The favicon cache: `favicons/` beside the store, one file per feed.
//! `<key>.<ext>` is the feed's site icon; `<key>.none` records that the
//! site has none, so it isn't asked again. The key hashes the feed URL.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use gutenberg_sources::favicon::Format;

/// The extension of a "this site has no icon" file.
const MISS: &str = "none";

#[derive(Debug, Clone)]
pub struct Icons {
    dir: PathBuf,
}

impl Icons {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `favicons/` in the store's directory.
    pub fn beside(store_path: &Path) -> Self {
        Self::new(store_path.with_file_name("favicons"))
    }

    /// A feed's file name without the extension. FNV-1a, so it never
    /// changes between builds the way `DefaultHasher` may.
    pub fn key(feed_url: &str) -> String {
        let hash = feed_url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        format!("{hash:016x}")
    }

    /// Feeds whose icon, or lack of one, is already on disk.
    pub fn settled(&self) -> HashSet<String> {
        self.entries().map(|(key, _, _)| key).collect()
    }

    /// Icon files by key, misses left out.
    pub fn found(&self) -> HashMap<String, PathBuf> {
        self.entries()
            .filter_map(|(key, format, path)| format.map(|_| (key, path)))
            .collect()
    }

    /// Writes a feed's icon, or `None` for a site without one. The file
    /// appears whole: it's written beside and renamed into place.
    pub fn save(&self, feed_url: &str, icon: Option<(&[u8], Format)>) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let key = Self::key(feed_url);
        let (bytes, ext) = icon.map_or((&[][..], MISS), |(bytes, format)| (bytes, format.ext()));
        let tmp = self.dir.join(format!(".{key}.tmp"));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, self.dir.join(format!("{key}.{ext}")))
    }

    /// `(key, format, path)` for each cache file; `None` for a miss.
    /// Anything else in the directory is skipped.
    fn entries(&self) -> impl Iterator<Item = (String, Option<Format>, PathBuf)> {
        std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                let key = path.file_stem()?.to_str()?;
                if key.len() != 16 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                let format = match path.extension()?.to_str()? {
                    MISS => None,
                    ext => Some(Format::from_ext(ext)?),
                };
                Some((key.to_owned(), format, path))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_stable_across_builds() {
        // Changing the hash would orphan every cached icon.
        assert_eq!(Icons::key(""), "cbf29ce484222325");
        assert_eq!(
            Icons::key("https://lwn.net/headlines/rss"),
            Icons::key("https://lwn.net/headlines/rss")
        );
        assert_ne!(
            Icons::key("https://a.example/feed"),
            Icons::key("https://b.example/feed")
        );
    }

    #[test]
    fn misses_settle_a_feed_but_have_no_icon() {
        let dir = tempfile::tempdir().unwrap();
        let icons = Icons::new(dir.path().join("favicons"));
        assert!(icons.settled().is_empty(), "no directory yet");

        icons
            .save("https://a.example/feed", Some((b"\x89PNG", Format::Png)))
            .unwrap();
        icons.save("https://b.example/feed", None).unwrap();
        std::fs::write(dir.path().join("favicons/notes.txt"), "").unwrap();

        let a = Icons::key("https://a.example/feed");
        let b = Icons::key("https://b.example/feed");
        assert_eq!(icons.settled(), HashSet::from([a.clone(), b]));
        let found = icons.found();
        assert_eq!(found.len(), 1);
        assert_eq!(found[&a], dir.path().join(format!("favicons/{a}.png")));
        assert_eq!(std::fs::read(&found[&a]).unwrap(), b"\x89PNG");
    }
}
