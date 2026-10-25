//! The disk cache for what the article pane fetches: article images in
//! `images/` and reader pages in `pages/`, under `$XDG_CACHE_HOME/gutenberg`.
//! A file is named for its URL's key ([`key`]) and its kind (`<key>.png`,
//! `<key>.html`). Both directories share one size cap; past it, the files
//! read longest ago go first. Reading a file marks it used.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use gutenberg_sources::image::Format;

const IMAGES: &str = "images";
const PAGES: &str = "pages";
const PAGE_EXT: &str = "html";

/// A URL's file name without the extension: FNV-1a in hex, so it never
/// changes between builds the way `DefaultHasher` may.
pub fn key(url: &str) -> String {
    let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

#[derive(Debug)]
pub struct Cache {
    dir: PathBuf,
    cap: u64,
    /// Bytes in both directories, kept as files are written and removed.
    used: u64,
}

impl Cache {
    /// `$XDG_CACHE_HOME/gutenberg`.
    pub fn default_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "gutenberg").map(|dirs| dirs.cache_dir().to_owned())
    }

    /// The cache in `dir`, holding at most `cap` bytes; already over it
    /// (a lower cap in `config.toml`), it's pruned now.
    pub fn open(dir: impl Into<PathBuf>, cap: u64) -> Self {
        let mut cache = Self {
            dir: dir.into(),
            cap,
            used: 0,
        };
        cache.used = cache.files().iter().map(|f| f.size).sum();
        cache.prune();
        cache
    }

    /// Bytes on disk now.
    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn image(&self, url: &str) -> Option<(Vec<u8>, Format)> {
        let key = key(url);
        Format::ALL.into_iter().find_map(|format| {
            let bytes = read(
                &self
                    .dir
                    .join(IMAGES)
                    .join(format!("{key}.{}", format.ext())),
            )?;
            Some((bytes, format))
        })
    }

    pub fn page(&self, url: &str) -> Option<String> {
        let path = self
            .dir
            .join(PAGES)
            .join(format!("{}.{PAGE_EXT}", key(url)));
        String::from_utf8(read(&path)?).ok()
    }

    pub fn save_image(&mut self, url: &str, bytes: &[u8], format: Format) -> io::Result<()> {
        self.save(IMAGES, &format!("{}.{}", key(url), format.ext()), bytes)
    }

    pub fn save_page(&mut self, url: &str, html: &str) -> io::Result<()> {
        self.save(PAGES, &format!("{}.{PAGE_EXT}", key(url)), html.as_bytes())
    }

    /// Writes beside and renames into place, so a file appears whole.
    fn save(&mut self, kind: &str, name: &str, bytes: &[u8]) -> io::Result<()> {
        let dir = self.dir.join(kind);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(name);
        let replaced = std::fs::metadata(&path).map_or(0, |m| m.len());
        let tmp = dir.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        self.used = self.used.saturating_sub(replaced) + bytes.len() as u64;
        self.prune();
        Ok(())
    }

    /// Past the cap, removes the files used longest ago until a tenth of
    /// the cap is free, so it isn't pruned again at the next write.
    fn prune(&mut self) {
        if self.used <= self.cap {
            return;
        }
        let mut files = self.files();
        files.sort_by_key(|f| f.used);
        let goal = self.cap - self.cap / 10;
        for file in files {
            if self.used <= goal {
                break;
            }
            if std::fs::remove_file(&file.path).is_ok() {
                self.used = self.used.saturating_sub(file.size);
            }
        }
    }

    fn files(&self) -> Vec<File> {
        [IMAGES, PAGES]
            .into_iter()
            .flat_map(|kind| std::fs::read_dir(self.dir.join(kind)).into_iter().flatten())
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let meta = entry.metadata().ok()?;
                meta.is_file().then(|| File {
                    path: entry.path(),
                    size: meta.len(),
                    used: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                })
            })
            .collect()
    }
}

struct File {
    path: PathBuf,
    size: u64,
    used: SystemTime,
}

/// A cache file's bytes, marking it used.
fn read(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    if let Ok(file) = std::fs::File::options().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn age(cache: &Cache, kind: &str, name: &str, secs: u64) {
        let path = cache.dir.join(kind).join(name);
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(secs))
            .unwrap();
    }

    #[test]
    fn keys_are_stable_across_builds() {
        // Changing the hash would orphan every cached file and favicon.
        assert_eq!(key(""), "cbf29ce484222325");
        assert_ne!(
            key("https://a.example/1.png"),
            key("https://a.example/2.png")
        );
    }

    #[test]
    fn images_and_pages_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = Cache::open(dir.path(), 1 << 20);
        assert!(cache.image("https://a.example/1.png").is_none());
        cache
            .save_image("https://a.example/1.png", b"\x89PNG", Format::Png)
            .unwrap();
        cache
            .save_page("https://a.example/story", "<p>Text</p>")
            .unwrap();
        assert_eq!(
            cache.image("https://a.example/1.png"),
            Some((b"\x89PNG".to_vec(), Format::Png))
        );
        assert_eq!(
            cache.page("https://a.example/story").as_deref(),
            Some("<p>Text</p>")
        );
        assert_eq!(cache.used(), 4 + 11);
        // Opened again, it counts what's there.
        assert_eq!(Cache::open(dir.path(), 1 << 20).used(), 15);
    }

    #[test]
    fn past_the_cap_the_file_read_longest_ago_goes() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = Cache::open(dir.path(), 100);
        let (a, b, c) = (
            "https://x.example/a",
            "https://x.example/b",
            "https://x.example/c",
        );
        cache.save_page(a, &"a".repeat(40)).unwrap();
        cache.save_page(b, &"b".repeat(40)).unwrap();
        age(&cache, PAGES, &format!("{}.html", key(a)), 300);
        age(&cache, PAGES, &format!("{}.html", key(b)), 200);
        // Reading a marks it used; b is now the oldest.
        assert!(cache.page(a).is_some());
        cache.save_page(c, &"c".repeat(40)).unwrap();
        assert!(cache.page(b).is_none());
        assert!(cache.page(a).is_some() && cache.page(c).is_some());
        assert_eq!(cache.used(), 80);
    }

    #[test]
    fn a_lower_cap_prunes_at_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = Cache::open(dir.path(), 1000);
        for i in 0..10 {
            cache
                .save_image(&format!("https://x.example/{i}.gif"), &[0; 50], Format::Gif)
                .unwrap();
        }
        assert_eq!(cache.used(), 500);
        let smaller = Cache::open(dir.path(), 200);
        assert!(smaller.used() <= 180, "{}", smaller.used());
    }
}
