//! `gutenberg --add <url>`: find a feed (from its own URL or a site's),
//! save it and its articles.

use std::process::ExitCode;

use gutenberg_core::rules::Rules;
use gutenberg_sources::feed::{self, Feed};
use gutenberg_store::{NewSource, SourceKind, Store};

pub fn run(url: &str, folder: Option<&str>) -> ExitCode {
    match add(url, folder) {
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

fn add(url: &str, folder: Option<&str>) -> Result<String, String> {
    let path = Store::default_path().map_err(|e| e.to_string())?;
    let mut store = Store::open(&path).map_err(|e| e.to_string())?;
    let known = store.sources().map_err(|e| e.to_string())?;
    let is_known = |u: &str| known.iter().any(|s| s.url.as_deref() == Some(u));
    if is_known(url) {
        return Err("already added".to_owned());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    // A site address works too: find() follows the page to its feed.
    let found = runtime
        .block_on(feed::find(url))
        .map_err(|e| e.to_string())?;
    let mut message = String::new();
    if found.url != url {
        if is_known(&found.url) {
            return Err(format!("already added as {}", found.url));
        }
        message = format!("Found {}\n", found.url);
    }
    message += &save(
        &mut store,
        &found.url,
        &found.feed,
        folder,
        gutenberg_core::sync::unix_now(),
        &crate::limits().rules,
    )
    .map_err(|e| e.to_string())?;
    Ok(message)
}

fn save(
    store: &mut Store,
    url: &str,
    feed: &Feed,
    folder: Option<&str>,
    fetched_at: i64,
    rules: &Rules,
) -> gutenberg_store::Result<String> {
    let folder_id = match folder {
        Some(name) => Some(store.folder_named(name)?.id),
        None => None,
    };
    let source = store.add_source(&NewSource {
        folder_id,
        kind: SourceKind::Feed,
        title: feed.title.clone(),
        url: Some(url.to_owned()),
        config: "{}".to_owned(),
    })?;
    let articles = gutenberg_core::sync::articles(feed, source.id, fetched_at);
    let sorted = rules.sort(&feed.title, folder, articles);
    let added = store.insert_arrived(&sorted.articles, &sorted.arrivals)?;
    let noun = if added == 1 { "article" } else { "articles" };
    let place = folder.map_or(String::new(), |name| format!(" to {name}"));
    let muted = match sorted.muted {
        0 => String::new(),
        n => format!(", {n} muted"),
    };
    Ok(format!(
        "Added {}{place}: {added} {noun}{muted}",
        feed.title
    ))
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
        let message = save(
            &mut store,
            "https://blog.example/feed",
            &feed,
            None,
            1,
            &Rules::default(),
        )
        .unwrap();
        assert_eq!(message, "Added Blog: 2 articles");
        assert_eq!(store.sources().unwrap()[0].title, "Blog");
        assert_eq!(store.list_articles(None, 10).unwrap().len(), 2);
    }

    #[test]
    fn a_folder_is_made_on_first_use_and_reused() {
        let mut store = Store::open_in_memory().unwrap();
        let feed = |title: &str| Feed {
            title: title.to_owned(),
            site_url: None,
            entries: vec![entry(title)],
        };
        let message = save(
            &mut store,
            "https://a.example/feed",
            &feed("A"),
            Some("Tech"),
            1,
            &Rules::default(),
        )
        .unwrap();
        assert_eq!(message, "Added A to Tech: 1 article");
        save(
            &mut store,
            "https://b.example/feed",
            &feed("B"),
            Some("Tech"),
            1,
            &Rules::default(),
        )
        .unwrap();
        let folders = store.folders().unwrap();
        assert_eq!(folders.len(), 1);
        let sources = store.sources().unwrap();
        assert!(sources.iter().all(|s| s.folder_id == Some(folders[0].id)));
    }

    #[test]
    fn rules_meet_a_new_feed_s_articles() {
        use gutenberg_core::rules::{Action, Rule};
        let mut store = Store::open_in_memory().unwrap();
        let feed = Feed {
            title: "Blog".to_owned(),
            site_url: None,
            entries: vec![entry("keep"), entry("star me"), entry("ad")],
        };
        let rule = |words: &str, action| Rule {
            words: words.into(),
            within: Some("tech".into()),
            action,
        };
        let rules = Rules::new(&[rule("star", Action::Star), rule("ad", Action::Mute)]);
        let message = save(
            &mut store,
            "https://blog.example/feed",
            &feed,
            Some("Tech"),
            1,
            &rules,
        )
        .unwrap();
        assert_eq!(message, "Added Blog to Tech: 2 articles, 1 muted");
        let mut stored: Vec<_> = store
            .list_articles(None, 10)
            .unwrap()
            .into_iter()
            .map(|a| (a.title, a.starred))
            .collect();
        stored.sort();
        assert_eq!(
            stored,
            [("keep".to_owned(), false), ("star me".to_owned(), true)]
        );
    }
}
