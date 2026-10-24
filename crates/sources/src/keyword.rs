//! Keyword alerts: a search query as a source. Its results come from Google
//! News's RSS search, which answers with an ordinary RSS feed.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::feed::Feed;

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
        let mut url = Url::parse("https://news.google.com/rss/search").expect("a valid URL");
        url.query_pairs_mut()
            .append_pair("q", &self.query)
            .append_pair("hl", "en-US")
            .append_pair("gl", "US")
            .append_pair("ceid", "US:en");
        url.into()
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
