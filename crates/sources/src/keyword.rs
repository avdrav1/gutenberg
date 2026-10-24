//! Keyword alerts: a search query as a source. Its results come from the
//! RSS searches of Google News and Bing News, merged so a story both carry
//! appears once.

use std::collections::HashSet;

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::feed::{Entry, Feed};

/// A news search a keyword alert asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    GoogleNews,
    BingNews,
}

impl Provider {
    /// Every provider, in the order their results are merged: Bing first,
    /// because its links already name the publisher.
    pub const ALL: [Provider; 2] = [Provider::BingNews, Provider::GoogleNews];
}

/// Where each provider lives, scheme and host with no trailing slash.
/// Tests point these at a local server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Providers {
    pub google_news: String,
    pub bing_news: String,
}

impl Default for Providers {
    fn default() -> Self {
        Self {
            google_news: "https://news.google.com".to_owned(),
            bing_news: "https://www.bing.com".to_owned(),
        }
    }
}

/// A keyword source's settings, kept as JSON in `source.config`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyword {
    /// As the user typed it, runs of whitespace made one space. Quotes,
    /// `-word`, `OR` and `site:` mean what they mean to Google News.
    pub query: String,
}

/// A keyword source whose settings can't be read.
#[derive(Debug, thiserror::Error)]
#[error("keyword alert settings: {0}")]
pub struct KeywordError(String);

impl Keyword {
    /// `None` for a query with nothing in it.
    pub fn new(query: &str) -> Option<Self> {
        let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
        (!query.is_empty()).then_some(Self { query })
    }

    /// The JSON for `source.config`.
    pub fn to_config(&self) -> String {
        serde_json::to_string(self).expect("a struct of strings is JSON")
    }

    pub fn from_config(config: &str) -> Result<Self, KeywordError> {
        let stored: Keyword =
            serde_json::from_str(config).map_err(|e| KeywordError(e.to_string()))?;
        Self::new(&stored.query).ok_or_else(|| KeywordError("the query is empty".into()))
    }

    /// The RSS search for this query, in U.S. English: Google's `hl`, `gl`
    /// and `ceid` together pick the edition.
    pub fn google_news_url(&self) -> String {
        self.search_url(Provider::GoogleNews, &Providers::default())
    }

    /// `provider`'s RSS search for this query, in U.S. English: Google's
    /// `hl`, `gl` and `ceid` together pick the edition, Bing's `mkt`.
    pub fn search_url(&self, provider: Provider, providers: &Providers) -> String {
        let (base, path, edition): (_, _, &[(&str, &str)]) = match provider {
            Provider::GoogleNews => (
                &providers.google_news,
                "/rss/search",
                &[("hl", "en-US"), ("gl", "US"), ("ceid", "US:en")],
            ),
            Provider::BingNews => (
                &providers.bing_news,
                "/news/search",
                &[("format", "rss"), ("mkt", "en-US")],
            ),
        };
        let mut url = Url::parse(&format!("{base}{path}")).expect("a valid URL");
        url.query_pairs_mut()
            .append_pair("q", &self.query)
            .extend_pairs(edition);
        url.into()
    }
}

/// Fixes up a provider's results so they read like a feed's: the headline
/// as the title, the publisher as the author, and for Bing the publisher's
/// own link. `xml` is the document `feed` was parsed from.
pub fn tidy(provider: Provider, feed: &mut Feed, xml: &[u8]) {
    match provider {
        Provider::GoogleNews => tidy_google_news(feed),
        Provider::BingNews => tidy_bing_news(feed, xml),
    }
}

/// Google News titles end in " - Publisher", and each description ends with
/// the publisher's name. Where both agree, the name moves from the title to
/// the author, so the list shows the headline and the article pane says who
/// published it.
pub fn tidy_google_news(feed: &mut Feed) {
    for entry in &mut feed.entries {
        let Some((headline, publisher)) = entry.title.rsplit_once(" - ") else {
            continue;
        };
        let signed = entry.summary_html.as_deref().is_some_and(|html| {
            let html = html.trim_end();
            [
                publisher.to_owned(),
                html_escape::encode_text(publisher).into_owned(),
            ]
            .iter()
            .any(|name| html.ends_with(&format!(">{name}</font>")))
        });
        if signed && !headline.trim().is_empty() {
            entry.author = Some(publisher.to_owned());
            entry.title = headline.trim().to_owned();
        }
    }
}

/// Bing News items have no guid, and their links go through
/// `bing.com/news/apiclick.aspx`, with a `tid` that changes on every search
/// and the publisher's URL in `url`. That URL becomes the link and the guid,
/// and the item's `<News:Source>` the author.
pub fn tidy_bing_news(feed: &mut Feed, xml: &[u8]) {
    let publishers = bing_sources(xml);
    for (i, entry) in feed.entries.iter_mut().enumerate() {
        let publisher_url = entry.url.as_deref().and_then(|link| {
            let link = Url::parse(link).ok()?;
            link.query_pairs()
                .find(|(k, _)| k == "url")
                .map(|(_, v)| v.into_owned())
                .filter(|v| v.starts_with("http"))
        });
        if let Some(url) = publisher_url {
            entry.guid = url.clone();
            entry.url = Some(url);
        }
        if let Some(Some(name)) = publishers.get(i) {
            entry.author = Some(name.clone());
        }
    }
}

/// Each `<item>`'s `<News:Source>`, in order. Bing's namespace URI is the
/// search URL itself, so this goes by the prefix.
fn bing_sources(xml: &[u8]) -> Vec<Option<String>> {
    let mut reader = Reader::from_reader(xml);
    let mut sources = Vec::new();
    // The raw text of the `<News:Source>` being read, entities and all.
    let mut source: Option<String> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == "item" => sources.push(None),
            Ok(Event::Start(e)) if e.name().as_ref() == "News:Source" => {
                source = Some(String::new());
            }
            Ok(Event::Text(text)) => source.iter_mut().for_each(|s| s.push_str(&text)),
            Ok(Event::GeneralRef(r)) => {
                source
                    .iter_mut()
                    .for_each(|s| s.push_str(&format!("&{};", &*r)));
            }
            Ok(Event::End(_)) => {
                let name = source
                    .take()
                    .map(|raw| html_escape::decode_html_entities(raw.trim()).into_owned());
                if let (Some(slot), Some(name)) = (sources.last_mut(), name) {
                    *slot = Some(name).filter(|n| !n.is_empty());
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    sources
}

/// A URL as it compares for sameness: no scheme, `www.`, fragment, trailing
/// slash or tracking parameters.
pub fn canonical_url(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let tracking = |k: &str| {
        k.starts_with("utm_")
            || ["fbclid", "gclid", "ocid", "cmpid", "smid", "ref", "taid"].contains(&k)
    };
    let query: Vec<String> = url
        .query_pairs()
        .filter(|(k, _)| !tracking(k))
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    let path = url.path().trim_end_matches('/');
    Some(if query.is_empty() {
        format!("{host}{path}")
    } else {
        format!("{host}{path}?{}", query.join("&"))
    })
}

/// A headline as it compares for sameness: lowercase letters and digits,
/// one space between words. `None` when nothing is left.
pub fn title_key(title: &str) -> Option<String> {
    let key = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    (!key.is_empty()).then_some(key)
}

/// What a keyword alert already has, to tell a new result from one it has
/// seen under another provider's link.
#[derive(Debug, Default)]
pub struct Seen {
    guids: HashSet<String>,
    urls: HashSet<String>,
    titles: HashSet<String>,
}

impl Seen {
    /// Remembers an article; `false` when it, or one with the same link or
    /// headline, was already there.
    pub fn insert(&mut self, guid: &str, url: Option<&str>, title: &str) -> bool {
        let url = url.and_then(canonical_url);
        let title = title_key(title);
        let known = self.guids.contains(guid)
            || url.as_ref().is_some_and(|u| self.urls.contains(u))
            || title.as_ref().is_some_and(|t| self.titles.contains(t));
        self.guids.insert(guid.to_owned());
        self.urls.extend(url);
        self.titles.extend(title);
        !known
    }

    /// Whether this link is already among the articles.
    pub fn has_url(&self, url: &str) -> bool {
        canonical_url(url).is_some_and(|u| self.urls.contains(&u))
    }
}

/// The results of every provider as one list: an entry whose guid, link or
/// headline came earlier, or is in `seen`, is left out. `seen` ends up with
/// everything kept.
pub fn merge(results: Vec<Vec<Entry>>, seen: &mut Seen) -> Vec<Entry> {
    results
        .into_iter()
        .flatten()
        .filter(|e| seen.insert(&e.guid, e.url.as_deref(), &e.title))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::Entry;

    #[test]
    fn queries_are_trimmed_and_round_trip_through_config() {
        assert_eq!(
            Keyword::new("  \"Cory   Booker\"\n"),
            Keyword::new("\"Cory Booker\"")
        );
        assert_eq!(Keyword::new(" \t "), None);
        let k = Keyword::new("\"Cory Booker\" -NJ").unwrap();
        assert_eq!(Keyword::from_config(&k.to_config()).unwrap(), k);
        assert!(Keyword::from_config("{}").is_err());
        assert!(Keyword::from_config(r#"{"query":"  "}"#).is_err());
    }

    #[test]
    fn the_search_url_encodes_the_query() {
        let k = Keyword::new("\"Cory Booker\" site:nj.com & more").unwrap();
        assert_eq!(
            k.google_news_url(),
            "https://news.google.com/rss/search?q=%22Cory+Booker%22+site%3Anj.com+%26+more\
             &hl=en-US&gl=US&ceid=US%3Aen"
        );
    }

    #[test]
    fn bing_searches_in_us_english_and_providers_can_move() {
        let k = Keyword::new("\"Cory Booker\"").unwrap();
        assert_eq!(
            k.search_url(Provider::BingNews, &Providers::default()),
            "https://www.bing.com/news/search?q=%22Cory+Booker%22&format=rss&mkt=en-US"
        );
        let local = Providers {
            google_news: "http://127.0.0.1:9".into(),
            bing_news: "http://127.0.0.1:9".into(),
        };
        assert!(k
            .search_url(Provider::GoogleNews, &local)
            .starts_with("http://127.0.0.1:9/rss/search?q="));
    }

    #[test]
    fn urls_compare_without_tracking_or_dressing() {
        let c = |u| canonical_url(u).unwrap();
        assert_eq!(
            c("https://www.CNN.com/2026/10/23/politics/x/?utm_source=rss&id=4#top"),
            c("http://cnn.com/2026/10/23/politics/x?id=4")
        );
        assert_ne!(c("https://cnn.com/a?id=4"), c("https://cnn.com/a?id=5"));
        assert_eq!(canonical_url("not a url"), None);
    }

    #[test]
    fn headlines_compare_by_their_words() {
        assert_eq!(
            title_key("Cory Booker: “Trump’s cuts” — what's next?"),
            title_key("cory booker trump s cuts what s next")
        );
        assert_eq!(title_key(" – "), None);
    }

    #[test]
    fn merge_keeps_the_first_of_each_story() {
        let entry = |guid: &str, url: &str, title: &str| Entry {
            guid: guid.into(),
            url: Some(url.into()),
            title: title.into(),
            author: None,
            summary_html: None,
            published_at: None,
        };
        let mut seen = Seen::default();
        seen.insert("old", Some("https://site.example/old"), "Yesterday's news");
        let merged = merge(
            vec![
                vec![
                    entry("b1", "https://site.example/a", "Story A"),
                    entry("b2", "https://site.example/b", "Story B"),
                ],
                vec![
                    entry("g1", "https://news.google.com/rss/articles/1", "Story A"),
                    entry("g2", "https://www.site.example/b/", "Story B, retitled"),
                    entry(
                        "g3",
                        "https://news.google.com/rss/articles/3",
                        "Yesterday’s news",
                    ),
                    entry("g4", "https://news.google.com/rss/articles/4", "Story C"),
                ],
            ],
            &mut seen,
        );
        let guids: Vec<_> = merged.iter().map(|e| e.guid.as_str()).collect();
        assert_eq!(guids, ["b1", "b2", "g4"]);
        assert!(seen.has_url("https://site.example/a"));
    }

    #[test]
    fn the_publisher_moves_from_title_to_author_only_when_signed() {
        let entry = |title: &str, summary: &str| Entry {
            guid: title.into(),
            url: None,
            title: title.into(),
            author: None,
            summary_html: Some(summary.into()),
            published_at: None,
        };
        let mut feed = Feed {
            title: "Google News".into(),
            site_url: None,
            entries: vec![
                entry(
                    "Booker - Kim race tightens - AT&T Daily",
                    "<a href=\"x\">…</a>&nbsp;<font color=\"#6f6f6f\">AT&amp;T Daily</font>",
                ),
                entry("Left - Right", "<p>No signature</p>"),
            ],
        };
        tidy_google_news(&mut feed);
        let first = &feed.entries[0];
        assert_eq!(first.title, "Booker - Kim race tightens");
        assert_eq!(first.author.as_deref(), Some("AT&T Daily"));
        assert_eq!(feed.entries[1].title, "Left - Right");
        assert_eq!(feed.entries[1].author, None);
    }
}
