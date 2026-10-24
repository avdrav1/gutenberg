//! Google News links (`news.google.com/rss/articles/<id>`) are pages that
//! reach the publisher by script, not a redirect. The page carries a signed
//! request for the publisher's URL; sending it to Google's `batchexecute`
//! endpoint gets the URL back. Two requests per article, so keyword alerts
//! resolve a few at a time.

use url::Url;

use crate::feed::{self, FeedError};

/// Article pages run to about 1 MB.
const MAX_PAGE: usize = 4 * 1024 * 1024;

/// The signed request an article page carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub id: String,
    pub timestamp: String,
    pub signature: String,
}

/// Whether `url` is a Google News article link under `base` (scheme and
/// host, as in `Providers::google_news`).
pub fn is_article_link(url: &str, base: &str) -> bool {
    url.strip_prefix(base)
        .is_some_and(|path| path.starts_with("/rss/articles/") || path.starts_with("/articles/"))
}

/// The `data-n-a-id`, `data-n-a-ts` and `data-n-a-sg` attributes of an
/// article page.
pub fn signature(html: &str) -> Option<Signature> {
    let attr = |name: &str| {
        let start = html.find(&format!("{name}=\""))? + name.len() + 2;
        let len = html[start..].find('"')?;
        Some(html[start..start + len].to_owned()).filter(|v| !v.is_empty())
    };
    Some(Signature {
        id: attr("data-n-a-id")?,
        timestamp: attr("data-n-a-ts")?,
        signature: attr("data-n-a-sg")?,
    })
}

/// The `f.req` form field that asks for the publisher's URL.
pub fn request(sig: &Signature) -> String {
    let inner = serde_json::json!([
        "garturlreq",
        [
            [
                "X",
                "X",
                ["X", "X"],
                null,
                null,
                1,
                1,
                "US:en",
                null,
                1,
                null,
                null,
                null,
                null,
                null,
                0,
                1
            ],
            "X",
            "X",
            1,
            [1, 1, 1],
            1,
            1,
            null,
            0,
            0,
            null,
            0
        ],
        sig.id,
        sig.timestamp.parse::<i64>().unwrap_or_default(),
        sig.signature,
    ]);
    serde_json::json!([[["Fbv4je", inner.to_string(), null, "generic"]]]).to_string()
}

/// The publisher's URL from a `batchexecute` reply: after a `)]}'` line,
/// a JSON array whose `wrb.fr` row holds, as a JSON string,
/// `["garturlres", "<url>", 1]`. `None` for a refusal, which has no string.
pub fn publisher_url(reply: &str) -> Option<String> {
    let body = reply.trim_start().strip_prefix(")]}'")?;
    let rows: Vec<serde_json::Value> = serde_json::from_str(body.trim()).ok()?;
    rows.iter().find_map(|row| {
        let row = row.as_array()?;
        if row.first()?.as_str()? != "wrb.fr" {
            return None;
        }
        let inner: serde_json::Value = serde_json::from_str(row.get(2)?.as_str()?).ok()?;
        let url = inner.get(1)?.as_str()?;
        (inner.get(0)?.as_str()? == "garturlres" && Url::parse(url).is_ok()).then(|| url.to_owned())
    })
}

/// Fetches the article page at `link` and asks `base`'s `batchexecute` for
/// the publisher's URL.
pub async fn resolve(client: &feed::Client, base: &str, link: &str) -> Result<String, FeedError> {
    let page = fetch_page(client, link).await?;
    let sig = signature(&page).ok_or_else(|| FeedError::NotAFeed("no signed request".into()))?;
    resolve_signed(client, base, &sig).await
}

/// The article page at `link`.
pub async fn fetch_page(client: &feed::Client, link: &str) -> Result<String, FeedError> {
    let bytes = feed::download(client, link, MAX_PAGE).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Sends a page's signed request to `base`'s `batchexecute`.
pub async fn resolve_signed(
    client: &feed::Client,
    base: &str,
    sig: &Signature,
) -> Result<String, FeedError> {
    let response = client
        .post(format!("{base}/_/DotsSplashUi/data/batchexecute"))
        .form(&[("f.req", request(sig))])
        .send()
        .await
        .map_err(feed::http_error)?;
    let status = response.status();
    if !status.is_success() {
        return Err(FeedError::Status {
            status,
            retry_after: None,
        });
    }
    let reply = response.text().await.map_err(feed::http_error)?;
    publisher_url(&reply).ok_or_else(|| FeedError::NotAFeed("Google named no publisher".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn article_links_are_told_from_others() {
        let base = "https://news.google.com";
        assert!(is_article_link(
            "https://news.google.com/rss/articles/CBMi?oc=5",
            base
        ));
        assert!(!is_article_link(
            "https://news.google.com/rss/search?q=x",
            base
        ));
        assert!(!is_article_link("https://www.cnn.com/rss/articles/x", base));
    }

    #[test]
    fn a_page_without_its_attributes_has_no_signature() {
        assert_eq!(signature("<div data-n-a-id=\"x\" data-n-a-ts=\"1\">"), None);
        assert_eq!(
            signature("<div data-n-a-id=\"x\" data-n-a-ts=\"1\" data-n-a-sg=\"\">"),
            None
        );
    }

    #[test]
    fn the_request_carries_the_signature() {
        let sig = Signature {
            id: "CBMi".into(),
            timestamp: "1792862196".into(),
            signature: "AbIa".into(),
        };
        let outer: serde_json::Value = serde_json::from_str(&request(&sig)).unwrap();
        assert_eq!(outer[0][0][0], "Fbv4je");
        let inner: serde_json::Value =
            serde_json::from_str(outer[0][0][1].as_str().unwrap()).unwrap();
        assert_eq!(inner[0], "garturlreq");
        assert_eq!(
            (&inner[2], &inner[3], &inner[4]),
            (&"CBMi".into(), &1792862196.into(), &"AbIa".into())
        );
    }

    #[test]
    fn junk_replies_name_no_publisher() {
        assert_eq!(publisher_url(""), None);
        assert_eq!(publisher_url("<html>"), None);
        assert_eq!(
            publisher_url(
                ")]}'\n\n[[\"wrb.fr\",\"Fbv4je\",\"[\\\"garturlres\\\",\\\"not a url\\\",1]\"]]"
            ),
            None
        );
    }
}
