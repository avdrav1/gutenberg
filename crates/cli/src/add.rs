//! `gutenberg --add <url>`: fetch a feed, save it and its articles.

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use gutenberg_sources::feed::{self, Feed};
use gutenberg_store::{NewArticle, NewSource, SourceKind, Store};

pub fn run(url: &str) -> ExitCode {
    match add(url) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("gutenberg: {url}: {message}");
            ExitCode::FAILURE
        }
    }
}

fn add(url: &str) -> Result<String, String> {
    let path = Store::default_path().map_err(|e| e.to_string())?;
    let mut store = Store::open(&path).map_err(|e| e.to_string())?;
    let known = store.sources().map_err(|e| e.to_string())?;
    if known.iter().any(|s| s.url.as_deref() == Some(url)) {
        return Err("already added".to_owned());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let bytes = runtime
        .block_on(feed::fetch(url))
        .map_err(|e| e.to_string())?;
    let parsed = feed::parse(&bytes, url).map_err(|e| e.to_string())?;
    save(&mut store, url, &parsed, now()).map_err(|e| e.to_string())
}

fn save(
    store: &mut Store,
    url: &str,
    feed: &Feed,
    fetched_at: i64,
) -> gutenberg_store::Result<String> {
    let source = store.add_source(&NewSource {
        folder_id: None,
        kind: SourceKind::Feed,
        title: feed.title.clone(),
        url: Some(url.to_owned()),
        config: "{}".to_owned(),
    })?;
    let articles: Vec<NewArticle> = feed
        .entries
        .iter()
        .map(|e| NewArticle {
            source_id: source.id,
            guid: e.guid.clone(),
            url: e.url.clone(),
            title: e.title.clone(),
            author: e.author.clone(),
            summary_html: e.summary_html.clone(),
            published_at: e.published_at,
            fetched_at,
        })
        .collect();
    let added = store.insert_articles(&articles)?;
    let noun = if added == 1 { "article" } else { "articles" };
    Ok(format!("Added {}: {added} {noun}", feed.title))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gutenberg_sources::feed::Entry;

    fn entry(guid: &str) -> Entry {
        Entry {
            guid: guid.to_owned(),
            url: Some(format!("https://blog.example/{guid}")),
            title: guid.to_owned(),
            author: None,
            summary_html: None,
            published_at: Some(1_700_000_000),
        }
    }

    #[test]
    fn saves_the_source_and_its_articles() {
        let mut store = Store::open_in_memory().unwrap();
        let feed = Feed {
            title: "Blog".to_owned(),
            site_url: None,
            // A feed that repeats an entry stores it once.
            entries: vec![entry("a"), entry("b"), entry("a")],
        };
        let message = save(&mut store, "https://blog.example/feed", &feed, 1).unwrap();
        assert_eq!(message, "Added Blog: 2 articles");
        assert_eq!(store.sources().unwrap()[0].title, "Blog");
        assert_eq!(store.list_articles(None, 10).unwrap().len(), 2);
    }
}
