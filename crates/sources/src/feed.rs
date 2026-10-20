//! RSS 0.9x/1.0/2.0, Atom and JSON Feed: fetch a document, parse it into
//! plain fields the store can take.

use std::time::Duration;

use feed_rs::model;

use crate::discover;

/// A parsed feed. Text fields are ready to show; `summary_html` is HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    pub title: String,
    /// The site the feed belongs to, when it says.
    pub site_url: Option<String>,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The entry's id, else its link. Stable across refreshes.
    pub guid: String,
    pub url: Option<String>,
    pub title: String,
    pub author: Option<String>,
    pub summary_html: Option<String>,
    /// Unix seconds, UTC: `published`, else `updated`.
    pub published_at: Option<i64>,
}

/// Every variant displays as one line.
#[derive(Debug, thiserror::Error)]
pub enum FeedError {
    #[error("empty response")]
    Empty,
    #[error("not a feed ({0})")]
    NotAFeed(String),
    #[error("HTTP {0}")]
    Status(reqwest::StatusCode),
    #[error("timed out")]
    Timeout,
    #[error("{0}")]
    Http(String),
    #[error(
        "no feed found ({} /feed, /rss, /atom.xml didn't parse)",
        if *.page_had_links { "the page's feed links," } else { "no <link rel=\"alternate\"> on the page;" }
    )]
    NoFeedFound { page_had_links: bool },
}

/// A feed and the URL it was actually found at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub url: String,
    pub feed: Feed,
}

const TIMEOUT: Duration = Duration::from_secs(15);
const ACCEPT: &str = "application/rss+xml, application/atom+xml, application/feed+json, \
                      application/xml;q=0.9, text/xml;q=0.9, */*;q=0.8";
/// Titles made from an untitled entry's text are cut to this many characters.
const TITLE_FROM_TEXT: usize = 80;

/// Fetches `url` as a feed. If it's a web page instead, tries the feeds it
/// links to, then `/feed`, `/rss` and `/atom.xml` on its site, and returns
/// the first that parses.
pub async fn find(url: &str) -> Result<Found, FeedError> {
    let bytes = fetch(url).await?;
    let not_a_feed = match parse(&bytes, url) {
        Ok(feed) => {
            return Ok(Found {
                url: url.to_owned(),
                feed,
            })
        }
        Err(err) => err,
    };
    if !discover::looks_like_html(&bytes) {
        return Err(not_a_feed);
    }
    let links = discover::feed_links(&String::from_utf8_lossy(&bytes), url);
    let page_had_links = !links.is_empty();
    for candidate in links.into_iter().chain(discover::fallbacks(url)) {
        if candidate == url {
            continue;
        }
        let Ok(bytes) = fetch(&candidate).await else {
            continue;
        };
        if let Ok(feed) = parse(&bytes, &candidate) {
            return Ok(Found {
                url: candidate,
                feed,
            });
        }
    }
    Err(FeedError::NoFeedFound { page_had_links })
}

/// Downloads a feed document.
pub async fn fetch(url: &str) -> Result<Vec<u8>, FeedError> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("Gutenberg/", env!("CARGO_PKG_VERSION")))
        .timeout(TIMEOUT)
        .build()
        .map_err(http_error)?;
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, ACCEPT)
        .send()
        .await
        .map_err(http_error)?;
    if !response.status().is_success() {
        return Err(FeedError::Status(response.status()));
    }
    Ok(response.bytes().await.map_err(http_error)?.to_vec())
}

fn http_error(err: reqwest::Error) -> FeedError {
    if err.is_timeout() {
        return FeedError::Timeout;
    }
    // reqwest's own message hides the cause ("error sending request"); add it.
    let mut message = err.to_string();
    let mut source = std::error::Error::source(&err);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    FeedError::Http(one_line(&message))
}

/// Parses any supported format. `feed_url` resolves relative links.
pub fn parse(bytes: &[u8], feed_url: &str) -> Result<Feed, FeedError> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Err(FeedError::Empty);
    }
    let parsed = feed_rs::parser::Builder::new()
        .base_uri(Some(feed_url))
        .build()
        .parse(bytes)
        .map_err(|e| FeedError::NotAFeed(one_line(&e.to_string())))?;

    let title = parsed
        .title
        .as_ref()
        .map(plain_text)
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| feed_url.to_owned());
    let site_url = alternate(&parsed.links)
        .filter(|href| *href != feed_url)
        .map(str::to_owned);
    // RSS descriptions are entity-encoded HTML in practice, whatever the
    // parser labels them; Atom and JSON Feed say which they are.
    let rss = !matches!(
        parsed.feed_type,
        model::FeedType::Atom | model::FeedType::JSON
    );
    let entries = parsed.entries.iter().map(|e| entry(e, rss)).collect();
    Ok(Feed {
        title,
        site_url,
        entries,
    })
}

fn entry(e: &model::Entry, rss: bool) -> Entry {
    let url = alternate(&e.links).map(str::to_owned);
    let summary_html = summary_html(e, rss);
    let title = e
        .title
        .as_ref()
        .map(plain_text)
        .filter(|t| !t.is_empty())
        .or_else(|| summary_html.as_deref().and_then(title_from_html))
        .unwrap_or_else(|| "Untitled".to_owned());
    let guid = if e.id.is_empty() {
        url.clone().unwrap_or_else(|| title.clone())
    } else {
        e.id.clone()
    };
    Entry {
        guid,
        url,
        title,
        author: e
            .authors
            .iter()
            .find_map(|p| p.name.as_deref().map(str::trim).filter(|n| !n.is_empty()))
            .map(str::to_owned),
        summary_html,
        published_at: e.published.or(e.updated).map(|t| t.timestamp()),
    }
}

/// The `alternate` link (or one with no rel), which is the page itself.
fn alternate(links: &[model::Link]) -> Option<&str> {
    links
        .iter()
        .find(|l| matches!(l.rel.as_deref(), None | Some("alternate")))
        .map(|l| l.href.as_str())
}

/// Content, else summary, else a media description (YouTube). Plain text
/// is escaped so the field is always HTML.
fn summary_html(e: &model::Entry, rss: bool) -> Option<String> {
    let is_html = |media_type: &str| rss || is_html(media_type);
    let content = e
        .content
        .as_ref()
        .and_then(|c| Some((c.body.as_deref()?, is_html(c.content_type.as_str()))));
    let summary = e
        .summary
        .as_ref()
        .map(|t| (t.content.as_str(), is_html(t.content_type.as_str())));
    let media = e
        .media
        .iter()
        .find_map(|m| m.description.as_ref())
        .map(|t| (t.content.as_str(), is_html(t.content_type.as_str())));
    let (body, html) = [content, summary, media]
        .into_iter()
        .flatten()
        .find(|(body, _)| !body.trim().is_empty())?;
    Some(if html {
        body.to_owned()
    } else {
        format!("<p>{}</p>", html_escape::encode_text(body.trim()))
    })
}

fn is_html(media_type: &str) -> bool {
    media_type.contains("html")
}

/// A title as text: HTML titles lose their tags and entities; all lose
/// runs of whitespace.
fn plain_text(text: &model::Text) -> String {
    if is_html(text.content_type.as_str()) {
        html_to_text(&text.content)
    } else {
        collapse_whitespace(&text.content)
    }
}

fn title_from_html(html: &str) -> Option<String> {
    let text = html_to_text(html);
    if text.is_empty() {
        return None;
    }
    let mut chars = text.chars();
    let mut title: String = chars.by_ref().take(TITLE_FROM_TEXT).collect();
    if chars.next().is_some() {
        title = format!("{}…", title.trim_end());
    }
    Some(title)
}

fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        text.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('>') else {
            rest = "";
            break;
        };
        // Block tags separate words; inline ones (<b>, <a>) don't.
        let name: String = rest[open + 1..open + close]
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase();
        if BLOCK_TAGS.contains(&name.as_str()) {
            text.push(' ');
        }
        rest = &rest[open + close + 1..];
    }
    text.push_str(rest);
    collapse_whitespace(&html_escape::decode_html_entities(&text))
}

const BLOCK_TAGS: &[&str] = &[
    "p",
    "br",
    "div",
    "li",
    "ul",
    "ol",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "tr",
    "td",
    "th",
    "blockquote",
    "pre",
    "hr",
    "img",
    "figure",
    "figcaption",
];

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn one_line(message: &str) -> String {
    collapse_whitespace(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_titles_lose_tags_and_entities() {
        assert_eq!(
            html_to_text("<b>Rust</b>aceans &amp; <i>you</i>"),
            "Rustaceans & you"
        );
        assert_eq!(
            html_to_text("<p>One</p><p>Two<br/>three</p>"),
            "One Two three"
        );
        assert_eq!(html_to_text("a &lt;b&gt; tag"), "a <b> tag");
        assert_eq!(html_to_text("cut <a href"), "cut");
    }

    #[test]
    fn untitled_entries_take_a_title_from_their_text() {
        let long = format!("<p>{}</p>", "word ".repeat(40));
        let title = title_from_html(&long).unwrap();
        assert!(title.ends_with('…'));
        assert!(title.chars().count() <= TITLE_FROM_TEXT + 1);
        assert_eq!(title_from_html("<p>Short toot</p>").unwrap(), "Short toot");
        assert_eq!(title_from_html("<img src=x>"), None);
    }
}
