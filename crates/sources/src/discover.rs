//! Find a site's feed from a page: `<link rel="alternate">` first, then
//! the usual paths on the site root.

use url::Url;

/// `type`s that mark a feed link. Daring Fireball, among others, labels
/// its JSON Feed plain `application/json`.
const FEED_TYPES: &[&str] = &[
    "application/rss+xml",
    "application/atom+xml",
    "application/feed+json",
    "application/json",
];

/// Tried on the site root, in order, when the page names no feed that works.
pub const FALLBACK_PATHS: &[&str] = &["/feed", "/rss", "/atom.xml"];

/// Feed URLs a page advertises, absolute, in page order, without repeats.
pub fn feed_links(html: &str, page_url: &str) -> Vec<String> {
    let Ok(page) = Url::parse(page_url) else {
        return Vec::new();
    };
    let mut base = page.clone();
    let mut links = Vec::new();
    for tag in tags(html) {
        match tag.name.as_str() {
            "base" => {
                if let Some(href) = tag.attr("href") {
                    if let Ok(url) = page.join(&href) {
                        base = url;
                    }
                }
            }
            "link" => {
                let alternate = tag.attr("rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|t| t.eq_ignore_ascii_case("alternate"))
                });
                let is_feed = tag.attr("type").is_some_and(|t| {
                    let t = t
                        .split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase();
                    FEED_TYPES.contains(&t.as_str())
                });
                if let (true, true, Some(href)) = (alternate, is_feed, tag.attr("href")) {
                    if let Ok(url) = base.join(href.trim()) {
                        let url = url.to_string();
                        if !links.contains(&url) {
                            links.push(url);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    links
}

/// `/feed`, `/rss`, `/atom.xml` on the root of the page's site.
pub fn fallbacks(page_url: &str) -> Vec<String> {
    let Ok(page) = Url::parse(page_url) else {
        return Vec::new();
    };
    FALLBACK_PATHS
        .iter()
        .filter_map(|path| page.join(path).ok())
        .map(String::from)
        .collect()
}

/// Whether a response is a web page rather than a broken feed.
pub fn looks_like_html(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<!doctype html") || head.starts_with("<html") || head.contains("<head")
}

struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
}

impl Tag {
    /// The attribute's value with HTML entities decoded.
    fn attr(&self, name: &str) -> Option<String> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| html_escape::decode_html_entities(v).into_owned())
    }
}

/// Start tags in document order, names and attribute names lowercased.
/// Enough HTML for `<link>` and `<base>`: comments are skipped, quoted and
/// unquoted values are read, and nothing inside a tag is interpreted.
fn tags(html: &str) -> impl Iterator<Item = Tag> + '_ {
    let mut rest = html;
    std::iter::from_fn(move || loop {
        let open = rest.find('<')?;
        rest = &rest[open + 1..];
        if let Some(after) = rest.strip_prefix("!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        let name_len = rest
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(rest.len());
        if name_len == 0 {
            continue; // `</x>`, `<!doctype>`, a stray `<`
        }
        let name = rest[..name_len].to_ascii_lowercase();
        rest = &rest[name_len..];
        let mut attrs = Vec::new();
        loop {
            rest = rest.trim_start();
            match rest.chars().next() {
                None => return Some(Tag { name, attrs }),
                Some('>') => {
                    rest = &rest[1..];
                    return Some(Tag { name, attrs });
                }
                Some('/') => rest = &rest[1..],
                Some(c) => {
                    let n = rest
                        .find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/'))
                        .unwrap_or(rest.len());
                    if n == 0 {
                        // A stray `=` or quote: step over it.
                        rest = &rest[c.len_utf8()..];
                        continue;
                    }
                    let attr = rest[..n].to_ascii_lowercase();
                    rest = rest[n..].trim_start();
                    let value = if let Some(after) = rest.strip_prefix('=') {
                        let after = after.trim_start();
                        let (value, tail) = match after.chars().next() {
                            Some(q @ ('"' | '\'')) => {
                                let body = &after[1..];
                                let end = body.find(q).unwrap_or(body.len());
                                (&body[..end], body.get(end + 1..).unwrap_or(""))
                            }
                            _ => {
                                let end = after
                                    .find(|c: char| c.is_whitespace() || c == '>')
                                    .unwrap_or(after.len());
                                (&after[..end], &after[end..])
                            }
                        };
                        rest = tail;
                        value.to_owned()
                    } else {
                        String::new()
                    };
                    attrs.push((attr, value));
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_quoted_unquoted_and_odd_attributes() {
        let html = r#"<LINK Rel=ALTERNATE type='application/rss+xml; charset=utf-8' href=/a.xml>
            <!-- <link rel="alternate" type="application/rss+xml" href="/commented-out"> -->
            <link rel="stylesheet alternate" type="text/css" href="/style.css">
            <link rel="alternate" type="application/atom+xml" href="/a.xml">
            <link rel="alternate" type="application/feed+json" href="feed.json?a=1&amp;b=2" />"#;
        assert_eq!(
            feed_links(html, "https://site.example/blog/"),
            [
                "https://site.example/a.xml",
                "https://site.example/blog/feed.json?a=1&b=2"
            ]
        );
    }

    #[test]
    fn base_href_changes_how_links_resolve() {
        let html = r#"<base href="https://cdn.example/x/"><link rel="alternate" type="application/rss+xml" href="rss">"#;
        assert_eq!(
            feed_links(html, "https://site.example/"),
            ["https://cdn.example/x/rss"]
        );
    }

    #[test]
    fn fallbacks_are_on_the_site_root() {
        assert_eq!(
            fallbacks("https://site.example/blog/post?x=1"),
            [
                "https://site.example/feed",
                "https://site.example/rss",
                "https://site.example/atom.xml"
            ]
        );
    }

    #[test]
    fn html_is_told_apart_from_a_feed() {
        assert!(looks_like_html(b"\n<!DOCTYPE html><html>"));
        assert!(looks_like_html(b"<html lang=en>"));
        assert!(!looks_like_html(b"<?xml version=\"1.0\"?><rss>"));
        assert!(!looks_like_html(
            b"{\"version\": \"https://jsonfeed.org/version/1.1\"}"
        ));
    }
}
