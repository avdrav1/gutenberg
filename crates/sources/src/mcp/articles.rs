//! A tool's result as articles. S6.2 reads two shapes:
//!
//! - **A list:** a JSON array of objects that each have a title and a URL,
//!   as `structuredContent`, at any depth inside it, or in a text block
//!   that is JSON. Each becomes an article. When every list there is
//!   empty, there's nothing new.
//! - **Anything else:** the whole result is one article, its text the
//!   summary. A result that hasn't changed is the same article.
//!
//! S6.3 adds feed documents and Markdown with links.

use serde_json::Value;

use super::McpError;
use crate::feed::Entry;

/// Keys a list item's fields go by, first match wins.
const TITLE: &[&str] = &["title", "headline", "name"];
const URL: &[&str] = &["url", "link", "href", "uri"];
const SUMMARY: &[&str] = &[
    "summary",
    "description",
    "snippet",
    "abstract",
    "text",
    "content",
];
const AUTHOR: &[&str] = &["author", "publisher", "source", "site"];
const DATE: &[&str] = &[
    "published",
    "publishedDate",
    "published_at",
    "publishedAt",
    "pubDate",
    "date",
    "updated",
    "updated_at",
    "created_at",
];

/// The articles in a tool's `result`, as the server sent it. A result the
/// tool marks as an error is a failure, with the tool's own words.
pub fn articles(result: &Value, tool: &str) -> Result<Vec<Entry>, McpError> {
    let texts: Vec<&str> = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect();
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        let said = texts
            .iter()
            .flat_map(|t| t.lines())
            .map(str::trim)
            .find(|l| !l.is_empty());
        return Err(McpError(match said {
            Some(line) => super::one_line(&format!("{tool}: {line}")),
            None => format!("{tool} reported an error"),
        }));
    }
    let structured = result.get("structuredContent").filter(|v| !v.is_null());
    let parsed: Vec<Value> = texts
        .iter()
        .filter_map(|t| serde_json::from_str(t.trim()).ok())
        .collect();
    for candidate in structured.into_iter().chain(&parsed) {
        let listed = list(candidate);
        if !listed.is_empty() {
            return Ok(listed);
        }
    }
    // A list of nothing, such as a day with no papers, is no articles,
    // not one article saying so.
    if structured
        .into_iter()
        .chain(&parsed)
        .any(|c| innermost_lists_empty(c) == Some(true))
    {
        return Ok(Vec::new());
    }
    let text = if texts.iter().any(|t| !t.trim().is_empty()) {
        texts.join("\n\n")
    } else if let Some(value) = structured {
        serde_json::to_string_pretty(value).unwrap_or_default()
    } else {
        String::new()
    };
    Ok(one_article(&text, tool).into_iter().collect())
}

/// The articles in the first list, at any depth, whose items have titles
/// and links.
fn list(value: &Value) -> Vec<Entry> {
    let inner: Box<dyn Iterator<Item = &Value>> = match value {
        Value::Array(items) => {
            let found: Vec<Entry> = items.iter().filter_map(item).collect();
            if !found.is_empty() {
                return found;
            }
            Box::new(items.iter())
        }
        Value::Object(fields) => Box::new(fields.values()),
        _ => return Vec::new(),
    };
    inner
        .map(list)
        .find(|found| !found.is_empty())
        .unwrap_or_default()
}

/// Whether every innermost list (one holding no list) is empty; `None`
/// when there are no lists at all.
fn innermost_lists_empty(value: &Value) -> Option<bool> {
    let children: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(fields) => fields.values().collect(),
        _ => return None,
    };
    let below: Vec<bool> = children
        .iter()
        .filter_map(|c| innermost_lists_empty(c))
        .collect();
    match (value, below.is_empty()) {
        (_, false) => Some(below.iter().all(|&empty| empty)),
        (Value::Array(items), true) => Some(items.is_empty()),
        _ => None,
    }
}

/// An article from a list item with a title and a URL.
fn item(value: &Value) -> Option<Entry> {
    let fields = value.as_object()?;
    let text = |keys: &[&str]| {
        keys.iter()
            .filter_map(|k| fields.get(*k)?.as_str())
            .map(str::trim)
            .find(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let title = text(TITLE)?;
    let url = text(URL).filter(|u| url::Url::parse(u).is_ok())?;
    Some(Entry {
        guid: url.clone(),
        url: Some(url),
        title,
        author: text(AUTHOR),
        summary_html: text(SUMMARY).map(|s| paragraphs(&s)),
        published_at: text(DATE).and_then(|d| date(&d)),
    })
}

/// The whole result as one article: its first line the title, all of it
/// the summary. `None` for an empty result.
fn one_article(text: &str, tool: &str) -> Option<Entry> {
    let first = text
        .lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())?;
    let mut chars = first.chars();
    let mut title: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() {
        title = format!("{}…", title.trim_end());
    }
    Some(Entry {
        // The same result is the same article; a new one is new.
        guid: format!("mcp:{tool}:{:016x}", fnv1a(text.as_bytes())),
        url: None,
        title,
        author: None,
        summary_html: Some(paragraphs(text)),
        published_at: None,
    })
}

/// Text as HTML paragraphs: blank lines part them, line breaks stay.
fn paragraphs(text: &str) -> String {
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let lines: Vec<String> = p
                .lines()
                .map(|l| html_escape::encode_text(l.trim_end()).into_owned())
                .collect();
            format!("<p>{}</p>", lines.join("<br>"))
        })
        .collect()
}

/// Unix seconds from an RFC 3339, RFC 2822 or `YYYY-MM-DD` date.
fn date(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(text))
        .map(|d| d.timestamp())
        .ok()
        .or_else(|| {
            let day = chrono::NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d").ok()?;
            Some(day.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
        })
}

/// FNV-1a, 64-bit: stable across builds, unlike `std`'s hasher.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_list_of_titles_and_links_is_articles() {
        let result = json!({"content": [{"type": "text", "text": json!([
            {"title": "Paper one", "url": "https://arxiv.org/abs/1", "published": "2026-10-23", "authors": ["A"]},
            {"title": "No link here"},
            {"name": "Paper two", "link": "https://arxiv.org/abs/2", "summary": "Line one\nline <two>"}
        ]).to_string()}]});
        let found = articles(&result, "search").unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].guid, "https://arxiv.org/abs/1");
        assert_eq!(found[0].published_at, Some(1_792_713_600));
        assert_eq!(found[1].title, "Paper two");
        assert_eq!(
            found[1].summary_html.as_deref(),
            Some("<p>Line one<br>line &lt;two&gt;</p>")
        );
    }

    #[test]
    fn a_list_inside_structured_content_is_found() {
        let result = json!({
            "content": [{"type": "text", "text": "2 results"}],
            "structuredContent": {"query": "x", "results": [
                {"title": "A", "url": "https://a.example/", "publishedDate": "2026-10-24T08:00:00Z", "author": "Ann"}
            ]}
        });
        let found = articles(&result, "web_search").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].author.as_deref(), Some("Ann"));
        assert_eq!(found[0].published_at, Some(1_792_828_800));
    }

    #[test]
    fn anything_else_is_one_article_the_same_while_unchanged() {
        let text = "## Daily papers\n\nTwo new papers today.\nSee below.";
        let result = json!({"content": [{"type": "text", "text": text}]});
        let one = articles(&result, "hf_fs").unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].title, "Daily papers");
        assert_eq!(
            one[0].summary_html.as_deref(),
            Some("<p>## Daily papers</p><p>Two new papers today.<br>See below.</p>")
        );
        assert_eq!(articles(&result, "hf_fs").unwrap()[0].guid, one[0].guid);
        let other = json!({"content": [{"type": "text", "text": "Three papers"}]});
        assert_ne!(articles(&other, "hf_fs").unwrap()[0].guid, one[0].guid);
        assert!(articles(&json!({"content": []}), "hf_fs")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn an_error_result_is_a_failure_in_the_tools_words() {
        let result = json!({"isError": true, "content": [{"type": "text", "text": "\nInvalid API key\nmore"}]});
        assert_eq!(
            articles(&result, "brave_news_search")
                .unwrap_err()
                .to_string(),
            "brave_news_search: Invalid API key"
        );
    }
}
