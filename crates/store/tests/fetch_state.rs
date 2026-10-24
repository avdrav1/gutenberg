//! S2.3: what a refresh leaves on a source, and which feeds are due.

use gutenberg_store::{NewArticle, NewSource, SourceKind, Store, Validators};

fn store_with_feed() -> (Store, i64) {
    let store = Store::open_in_memory().unwrap();
    let id = store
        .add_source(&NewSource {
            folder_id: None,
            kind: SourceKind::Feed,
            title: "Feed".to_owned(),
            url: Some("https://site.example/feed".to_owned()),
            config: "{}".to_owned(),
        })
        .unwrap()
        .id;
    (store, id)
}

fn article(source_id: i64, guid: &str) -> NewArticle {
    NewArticle {
        source_id,
        guid: guid.to_owned(),
        url: None,
        title: guid.to_owned(),
        author: None,
        summary_html: None,
        published_at: Some(1),
        fetched_at: 1,
    }
}

#[test]
fn a_fetch_stores_articles_and_validators_together() {
    let (mut store, id) = store_with_feed();
    let validators = Validators {
        etag: Some("\"v1\"".to_owned()),
        last_modified: Some("Mon, 19 Oct 2026 10:00:00 GMT".to_owned()),
    };
    let new = store
        .record_fetched(id, &[article(id, "a"), article(id, "b")], &validators, 100)
        .unwrap();
    assert_eq!(new, 2);
    let state = store.fetch_state(id).unwrap();
    assert_eq!(state.validators, validators);
    assert_eq!(state.checked_at, Some(100));
    assert_eq!(store.sources_due(100).unwrap()[0].validators, validators);

    // The same items again: nothing new.
    assert_eq!(
        store
            .record_fetched(id, &[article(id, "a")], &validators, 200)
            .unwrap(),
        0
    );
}

#[test]
fn a_failure_backs_off_and_a_success_clears_it() {
    let (store, id) = store_with_feed();
    store
        .record_failure(id, "HTTP 503 Service Unavailable", 220, 100)
        .unwrap();
    let state = store.fetch_state(id).unwrap();
    assert_eq!(state.failures, 1);
    assert_eq!(state.retry_at, Some(220));
    assert_eq!(state.error.as_deref(), Some("HTTP 503 Service Unavailable"));

    assert!(store.sources_due(219).unwrap().is_empty());
    assert_eq!(store.sources_due(220).unwrap()[0].failures, 1);

    store.record_failure(id, "timed out", 900, 300).unwrap();
    assert_eq!(store.fetch_state(id).unwrap().failures, 2);

    store.record_not_modified(id, 1000).unwrap();
    let state = store.fetch_state(id).unwrap();
    assert_eq!(
        (state.failures, state.retry_at, state.error),
        (0, None, None)
    );
    assert_eq!(state.checked_at, Some(1000));
}

#[test]
fn feeds_keyword_alerts_and_mcp_servers_are_refreshed() {
    let (store, feed) = store_with_feed();
    let source = |kind, title: &str| NewSource {
        folder_id: None,
        kind,
        title: title.to_owned(),
        url: None,
        config: r#"{"query":"Cory Booker"}"#.to_owned(),
    };
    let keyword = store
        .add_source(&source(SourceKind::Keyword, "Cory Booker"))
        .unwrap()
        .id;
    let mcp = store
        .add_source(&source(SourceKind::Mcp, "Cronkite"))
        .unwrap()
        .id;
    let due = store.sources_due(0).unwrap();
    assert_eq!(
        due.iter()
            .map(|d| (d.source_id, d.kind, d.url.is_some()))
            .collect::<Vec<_>>(),
        [
            (feed, SourceKind::Feed, true),
            (keyword, SourceKind::Keyword, false),
            (mcp, SourceKind::Mcp, false)
        ]
    );
    assert_eq!(due[1].config, r#"{"query":"Cory Booker"}"#);
}

#[test]
fn a_new_feed_is_unchecked_until_its_first_refresh() {
    let (mut store, id) = store_with_feed();
    assert!(store.has_unchecked_sources().unwrap());
    store
        .record_fetched(id, &[article(id, "a")], &Validators::default(), 100)
        .unwrap();
    assert!(!store.has_unchecked_sources().unwrap());

    let second = store
        .add_source(&NewSource {
            folder_id: None,
            kind: SourceKind::Feed,
            title: "Second".to_owned(),
            url: Some("https://other.example/feed".to_owned()),
            config: "{}".to_owned(),
        })
        .unwrap()
        .id;
    assert!(store.has_unchecked_sources().unwrap());
    // A failure counts as checked, so a broken feed doesn't refresh nonstop.
    store.record_failure(second, "timed out", 400, 100).unwrap();
    assert!(!store.has_unchecked_sources().unwrap());

    // A new keyword alert is fetched right away too.
    store
        .add_source(&NewSource {
            folder_id: None,
            kind: SourceKind::Keyword,
            title: "Cory Booker".to_owned(),
            url: None,
            config: r#"{"query":"Cory Booker"}"#.to_owned(),
        })
        .unwrap();
    assert!(store.has_unchecked_sources().unwrap());
}
