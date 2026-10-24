//! Keyword alerts: a search query as a source. Its results come from the
//! RSS searches of Google News and Bing News, merged so a story both carry
//! appears once.

use std::collections::HashSet;

use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::feed::{self, Entry, Feed};

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
    /// `-word`, `OR` and `site:` mean what they mean to Google News;
    /// `lang:fr` and `region:CA` pick the edition searched.
    pub query: String,
}

/// A keyword source whose settings can't be read.
#[derive(Debug, thiserror::Error)]
#[error("keyword alert settings: {0}")]
pub struct KeywordError(String);

impl Keyword {
    /// `None` for a query with nothing to search, such as `lang:fr` alone.
    pub fn new(query: &str) -> Option<Self> {
        let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
        let keyword = Self { query };
        keyword
            .provider_query(Provider::GoogleNews)
            .is_some()
            .then_some(keyword)
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

    /// Google News's RSS search for this query.
    pub fn google_news_url(&self) -> String {
        self.search_url(Provider::GoogleNews, &Providers::default())
            .expect("Google News reads every query")
    }

    /// `provider`'s RSS search for this query, in its edition: Google's
    /// `hl`, `gl` and `ceid` together pick it, Bing's `mkt`. Google
    /// redirects a pair it has no edition for to its nearest one. `None`
    /// when the provider can't ask it.
    pub fn search_url(&self, provider: Provider, providers: &Providers) -> Option<String> {
        let q = self.provider_query(provider)?;
        let Edition { language, region } = self.edition();
        let locale = format!("{language}-{region}");
        let (base, path, edition) = match provider {
            Provider::GoogleNews => (
                &providers.google_news,
                "/rss/search",
                vec![
                    ("hl", locale),
                    ("gl", region.clone()),
                    ("ceid", format!("{region}:{language}")),
                ],
            ),
            Provider::BingNews => (
                &providers.bing_news,
                "/news/search",
                vec![("format", "rss".to_owned()), ("mkt", locale)],
            ),
        };
        let mut url = Url::parse(&format!("{base}{path}")).expect("a valid URL");
        url.query_pairs_mut()
            .append_pair("q", &q)
            .extend_pairs(edition);
        Some(url.into())
    }

    /// The query as `provider` is asked it: without `lang:` and `region:`,
    /// and for Bing without exclusions, which it ignores (they're applied
    /// to the results instead, by [`Keyword::excludes`]). `None` for a
    /// query Bing can't ask: it finds nothing for one with `OR`.
    pub fn provider_query(&self, provider: Provider) -> Option<String> {
        let tokens = tokens(&self.query);
        if provider == Provider::BingNews && tokens.contains(&Token::Or) {
            return None;
        }
        let kept: Vec<&str> = tokens
            .iter()
            .filter_map(|token| match *token {
                Token::Language(_) | Token::Region(_) => None,
                Token::Exclude(_) if provider == Provider::BingNews => None,
                Token::Exclude(raw) | Token::Term(raw) => Some(raw),
                Token::Or => Some("OR"),
            })
            .collect();
        (!kept.is_empty()).then(|| kept.join(" "))
    }

    /// The language and region searched: U.S. English unless the query
    /// names one. With only one named, the other is the usual pairing
    /// (`lang:fr` is France, `region:CA` is English).
    pub fn edition(&self) -> Edition {
        let (mut language, mut region) = (None, None);
        for token in tokens(&self.query) {
            match token {
                Token::Language(l) => language = Some(l.to_ascii_lowercase()),
                Token::Region(r) => region = Some(r.to_ascii_uppercase()),
                _ => {}
            }
        }
        let (language, region) = match (language, region) {
            (Some(l), Some(r)) => (l, r),
            (Some(l), None) => {
                let r = home_region(&l);
                (l, r)
            }
            (None, Some(r)) => (main_language(&r), r),
            (None, None) => ("en".to_owned(), "US".to_owned()),
        };
        Edition { language, region }
    }

    /// Whether the query's exclusions leave `entry` out: a `-word` or
    /// `-"phrase"` in its headline or summary, or a `-site:` that hosts
    /// its link. Bing ignores exclusions, and Google lets the odd one by.
    pub fn excludes(&self, entry: &Entry) -> bool {
        let text = format!(
            "{} {}",
            entry.title,
            entry
                .summary_html
                .as_deref()
                .map(feed::html_to_text)
                .unwrap_or_default()
        );
        let text = title_key(&text).map(|t| format!(" {t} "));
        let host = entry
            .url
            .as_deref()
            .and_then(|u| Url::parse(u).ok())
            .and_then(|u| u.host_str().map(str::to_ascii_lowercase));
        tokens(&self.query).iter().any(|token| {
            let Token::Exclude(raw) = token else {
                return false;
            };
            let raw = &raw[1..];
            if let Some(site) = raw.strip_prefix("site:") {
                let site = site.trim_matches('"').to_ascii_lowercase();
                let site = site.trim_start_matches("www.");
                return !site.is_empty()
                    && host.as_deref().is_some_and(|h| {
                        h == site || h.strip_suffix(site).is_some_and(|p| p.ends_with('.'))
                    });
            }
            match (title_key(raw), &text) {
                (Some(words), Some(text)) => text.contains(&format!(" {words} ")),
                _ => false,
            }
        })
    }
}

/// The language and region a keyword alert searches in: `fr` and `CA`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edition {
    pub language: String,
    pub region: String,
}

/// A piece of a query, each borrowed as typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    /// `lang:fr`; holds `fr`.
    Language(&'a str),
    /// `region:CA`; holds `CA`.
    Region(&'a str),
    /// `-word`, `-"a phrase"` or `-site:example.com`, dash included.
    Exclude(&'a str),
    Or,
    /// A word, a quoted phrase, `site:` or anything else.
    Term(&'a str),
}

/// The query split at spaces outside quotes.
fn tokens(query: &str) -> Vec<Token<'_>> {
    let mut pieces = Vec::new();
    let mut start = None;
    let mut quoted = false;
    for (i, c) in query.char_indices() {
        match (c, start) {
            ('"', _) => {
                quoted = !quoted;
                start = start.or(Some(i));
            }
            (c, Some(s)) if c.is_whitespace() && !quoted => {
                pieces.push(&query[s..i]);
                start = None;
            }
            (c, None) if !c.is_whitespace() => start = Some(i),
            _ => {}
        }
    }
    pieces.extend(start.map(|s| &query[s..]));
    let letters = |s: &str, lens: &[usize]| {
        lens.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphabetic())
    };
    pieces
        .into_iter()
        .map(|piece| match piece.split_once(':') {
            Some(("lang", l)) if letters(l, &[2, 3]) => Token::Language(l),
            Some(("region", r)) if letters(r, &[2]) => Token::Region(r),
            _ if piece == "OR" => Token::Or,
            _ if piece.len() > 1 && piece.starts_with('-') => Token::Exclude(piece),
            _ => Token::Term(piece),
        })
        .collect()
}

/// Where a language is mostly read, for `lang:` alone. Most codes name
/// their country too (`fr`, `de`, `it`); these don't.
fn home_region(language: &str) -> String {
    const ELSEWHERE: &[(&str, &str)] = &[
        ("ar", "EG"),
        ("cs", "CZ"),
        ("da", "DK"),
        ("el", "GR"),
        ("en", "US"),
        ("he", "IL"),
        ("hi", "IN"),
        ("ja", "JP"),
        ("ko", "KR"),
        ("pt", "BR"),
        ("sv", "SE"),
        ("uk", "UA"),
        ("vi", "VN"),
        ("zh", "CN"),
    ];
    ELSEWHERE
        .iter()
        .find(|(l, _)| *l == language)
        .map_or_else(|| language.to_ascii_uppercase(), |(_, r)| (*r).to_owned())
}

/// A region's main language, for `region:` alone. Most countries' codes
/// name it too (`FR`, `DE`, `IT`); these don't.
fn main_language(region: &str) -> String {
    const OTHERWISE: &[(&str, &[&str])] = &[
        (
            "en",
            &[
                "AU", "CA", "GB", "GH", "IE", "IN", "KE", "NG", "NZ", "PH", "PK", "SG", "US", "ZA",
            ],
        ),
        ("es", &["AR", "CL", "CO", "CU", "MX", "PE", "VE"]),
        ("pt", &["AO", "BR", "MZ"]),
        ("de", &["AT", "CH"]),
        ("fr", &["BE", "CI", "SN"]),
        ("ar", &["AE", "EG", "LB", "SA"]),
        ("zh", &["CN", "HK", "TW"]),
        ("cs", &["CZ"]),
        ("da", &["DK"]),
        ("el", &["GR"]),
        ("he", &["IL"]),
        ("ja", &["JP"]),
        ("ko", &["KR"]),
        ("sv", &["SE"]),
        ("uk", &["UA"]),
        ("vi", &["VN"]),
    ];
    OTHERWISE
        .iter()
        .find(|(_, regions)| regions.contains(&region))
        .map_or_else(|| region.to_ascii_lowercase(), |(l, _)| (*l).to_owned())
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
            k.search_url(Provider::BingNews, &Providers::default())
                .unwrap(),
            "https://www.bing.com/news/search?q=%22Cory+Booker%22&format=rss&mkt=en-US"
        );
        let local = Providers {
            google_news: "http://127.0.0.1:9".into(),
            bing_news: "http://127.0.0.1:9".into(),
        };
        assert!(k
            .search_url(Provider::GoogleNews, &local)
            .unwrap()
            .starts_with("http://127.0.0.1:9/rss/search?q="));
    }

    #[test]
    fn queries_split_at_spaces_outside_quotes() {
        assert_eq!(
            tokens(r#""night trains" -"sleeper car" OR site:nj.com lang:fr region:ca -x"#),
            [
                Token::Term("\"night trains\""),
                Token::Exclude("-\"sleeper car\""),
                Token::Or,
                Token::Term("site:nj.com"),
                Token::Language("fr"),
                Token::Region("ca"),
                Token::Exclude("-x"),
            ]
        );
        // Lowercase `or`, a lone dash and a quoted `lang:` are words.
        assert_eq!(
            tokens(r#"a or - "lang:fr" lang:french"#),
            [
                Token::Term("a"),
                Token::Term("or"),
                Token::Term("-"),
                Token::Term("\"lang:fr\""),
                Token::Term("lang:french"),
            ]
        );
    }

    #[test]
    fn each_provider_is_asked_what_it_reads() {
        let k = Keyword::new(r#""Cory Booker" -Senate -site:nj.com lang:en"#).unwrap();
        assert_eq!(
            k.provider_query(Provider::GoogleNews).unwrap(),
            r#""Cory Booker" -Senate -site:nj.com"#
        );
        assert_eq!(
            k.provider_query(Provider::BingNews).unwrap(),
            r#""Cory Booker""#
        );
        let either = Keyword::new("Booker OR Kim").unwrap();
        assert_eq!(
            either.provider_query(Provider::GoogleNews).unwrap(),
            "Booker OR Kim"
        );
        assert_eq!(
            either.search_url(Provider::BingNews, &Providers::default()),
            None
        );
        assert!(Keyword::new("lang:fr region:CA").is_none());
        assert!(Keyword::new("\"lang:fr\"").is_some());
    }

    #[test]
    fn language_and_region_pick_the_edition() {
        let edition = |q: &str| {
            let Edition { language, region } = Keyword::new(q).unwrap().edition();
            format!("{language}-{region}")
        };
        assert_eq!(edition("Macron"), "en-US");
        assert_eq!(edition("Macron lang:FR"), "fr-FR");
        assert_eq!(edition("Lula lang:pt"), "pt-BR");
        assert_eq!(edition("Carney region:ca"), "en-CA");
        assert_eq!(edition("Carney region:CA lang:fr"), "fr-CA");
        assert_eq!(edition("Meloni region:IT"), "it-IT");
        assert_eq!(edition("Macron lang:de lang:fr"), "fr-FR");

        let k = Keyword::new("Carney lang:fr region:CA").unwrap();
        assert_eq!(
            k.search_url(Provider::GoogleNews, &Providers::default())
                .unwrap(),
            "https://news.google.com/rss/search?q=Carney&hl=fr-CA&gl=CA&ceid=CA%3Afr"
        );
        assert_eq!(
            k.search_url(Provider::BingNews, &Providers::default())
                .unwrap(),
            "https://www.bing.com/news/search?q=Carney&format=rss&mkt=fr-CA"
        );
    }

    #[test]
    fn exclusions_leave_out_matching_results() {
        let entry = |url: &str, title: &str, summary: &str| Entry {
            guid: url.into(),
            url: Some(url.into()),
            title: title.into(),
            author: None,
            summary_html: Some(summary.into()),
            published_at: None,
        };
        let k = Keyword::new(r#"Booker -senate -"New Jersey" -site:nj.com"#).unwrap();
        let excludes = |url, title, summary| k.excludes(&entry(url, title, summary));
        assert!(excludes("https://a.example/1", "Booker's Senate seat", ""));
        assert!(excludes(
            "https://a.example/2",
            "Booker on tour",
            "<p>Back in <b>New</b>&nbsp;Jersey</p>"
        ));
        assert!(excludes("https://www.nj.com/3", "Booker on tour", ""));
        assert!(excludes("https://politics.nj.com/4", "Booker on tour", ""));
        assert!(!excludes("https://notnj.com/5", "Booker on tour", ""));
        assert!(!excludes(
            "https://a.example/6",
            "Booker and the senators",
            ""
        ));
        assert!(!excludes(
            "https://a.example/7",
            "Booker in New York",
            "Jersey City"
        ));
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
