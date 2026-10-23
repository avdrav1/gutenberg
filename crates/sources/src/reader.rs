//! Reader view: a web page cut down to its article with `dom_smoothie`, a
//! port of Firefox's Readability.

use crate::discover;
use crate::feed::{self, FeedError};

/// Bigger than any article page worth reading.
pub const MAX_BYTES: usize = 5 * 1024 * 1024;

/// An article as the reader view shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// The article's own markup, links and images made absolute.
    pub html: String,
    /// The same as plain text.
    pub text: String,
}

/// Why a page has no reader view. Every variant displays as one line.
#[derive(Debug, thiserror::Error)]
pub enum ReaderError {
    #[error("{0}")]
    Fetch(#[from] FeedError),
    #[error("not a web page")]
    NotHtml,
    #[error("no article found on the page")]
    NothingReadable,
}

/// The article in `html`, a page found at `url`.
pub fn extract(html: &str, url: &str) -> Result<Extracted, ReaderError> {
    let cfg = dom_smoothie::Config {
        // Readability.js's own pick keeps only the first part of an article
        // split by ad slots (Ars Technica); this one follows the parts.
        candidate_select_mode: dom_smoothie::CandidateSelectMode::DomSmoothie,
        // Paragraphs separated by blank lines rather than run together.
        text_mode: dom_smoothie::TextMode::Formatted,
        ..Default::default()
    };
    let mut readability = dom_smoothie::Readability::new(html, Some(url), Some(cfg))
        .map_err(|_| ReaderError::NothingReadable)?;
    let article = readability
        .parse()
        .map_err(|_| ReaderError::NothingReadable)?;
    let text = article.text_content.trim().to_owned();
    if text.is_empty() {
        return Err(ReaderError::NothingReadable);
    }
    Ok(Extracted {
        html: article.content.to_string(),
        text,
    })
}

/// Downloads the page at `url` and extracts its article.
pub async fn fetch(client: &reqwest::Client, url: &str) -> Result<Extracted, ReaderError> {
    let bytes = feed::download(client, url, MAX_BYTES).await?;
    if !discover::looks_like_html(&bytes) {
        return Err(ReaderError::NotHtml);
    }
    extract(&String::from_utf8_lossy(&bytes), url)
}
