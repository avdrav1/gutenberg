//! Images: telling one from a page served in its place, and downloading
//! one without holding more than a cap in memory.

use crate::discover;

/// Larger than any image an article pane needs.
pub const MAX_BYTES: usize = 10 * 1024 * 1024;

/// An image format, told from the first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Ico,
    Gif,
    Jpeg,
    Webp,
    Svg,
}

impl Format {
    pub const ALL: [Format; 6] = [
        Format::Png,
        Format::Ico,
        Format::Gif,
        Format::Jpeg,
        Format::Webp,
        Format::Svg,
    ];

    /// The file extension, which is how an image decoder picks the format.
    pub const fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Ico => "ico",
            Format::Gif => "gif",
            Format::Jpeg => "jpg",
            Format::Webp => "webp",
            Format::Svg => "svg",
        }
    }

    pub fn from_ext(ext: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.ext() == ext)
    }
}

/// The format of a download, or `None` when it isn't an image: an HTML
/// error page served with a 200, or an empty body.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        Some(Format::Ico)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(Format::Gif)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(Format::Jpeg)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(Format::Webp)
    } else if is_svg(bytes) {
        Some(Format::Svg)
    } else {
        None
    }
}

fn is_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    head.contains("<svg") && !discover::looks_like_html(bytes)
}

/// Downloads an image. `None` when the server fails, the body is bigger
/// than [`MAX_BYTES`], or it isn't an image.
pub async fn fetch(client: &reqwest::Client, url: &str) -> Option<(Vec<u8>, Format)> {
    let bytes = crate::feed::download(client, url, MAX_BYTES).await.ok()?;
    let format = sniff(&bytes)?;
    Some((bytes, format))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn fixture(path: &str) -> Vec<u8> {
        std::fs::read(format!("{FIXTURES}/{path}")).unwrap()
    }

    #[test]
    fn real_images_are_told_apart_from_pages() {
        assert_eq!(sniff(&fixture("icons/archlinux.png")), Some(Format::Png));
        assert_eq!(
            sniff(&fixture("icons/daringfireball.ico")),
            Some(Format::Ico)
        );
        assert_eq!(sniff(&fixture("icons/rust-lang.svg")), Some(Format::Svg));
        // A soft 404: a page with an inline <svg>, served with 200.
        assert_eq!(
            sniff(b"<!DOCTYPE html><html><body><svg></svg>Not found</body></html>"),
            None
        );
        assert_eq!(sniff(b""), None);
    }
}
