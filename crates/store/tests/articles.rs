use std::time::{Duration, Instant};

use gutenberg_store::{NewArticle, NewSource, SourceKind, Store};

fn add_feeds(store: &Store, n: usize) -> Vec<i64> {
    (0..n)
        .map(|i| {
            let url = format!("https://site{i}.example/feed");
            store
                .add_source(&NewSource {
                    folder_id: None,
                    kind: SourceKind::Feed,
                    title: url.clone(),
                    url: Some(url),
                    config: "{}".to_owned(),
                })
                .unwrap()
                .id
        })
        .collect()
}

fn articles(sources: &[i64], n: usize) -> Vec<NewArticle> {
    (0..n)
        .map(|i| NewArticle {
            source_id: sources[i % sources.len()],
            guid: format!("urn:article:{i}"),
            url: Some(format!("https://example.com/{i}")),
            title: format!("Headline number {i} about the news of the day"),
            author: Some("A. Writer".to_owned()),
            summary_html: Some("<p>Summary paragraph.</p>".repeat(8)),
            // Every tenth item has no date and sorts by fetch time.
            published_at: (i % 10 != 0).then_some(1_700_000_000 + i as i64 * 60),
            fetched_at: 1_700_000_000 + i as i64 * 60 + 30,
        })
        .collect()
}

#[test]
fn a_refresh_ignores_known_articles_and_keeps_their_flags() {
    let mut store = Store::open_in_memory().unwrap();
    let sources = add_feeds(&store, 1);
    let batch = articles(&sources, 3);
    assert_eq!(store.insert_articles(&batch).unwrap(), 3);

    let first = store.list_articles(None, 10).unwrap()[0].id;
    store.set_read(first, true).unwrap();

    assert_eq!(store.insert_articles(&batch).unwrap(), 0);
    let rows = store.list_articles(None, 10).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().find(|r| r.id == first).unwrap().read);
}

#[test]
fn articles_list_newest_first_and_filter_by_source() {
    let mut store = Store::open_in_memory().unwrap();
    let sources = add_feeds(&store, 2);
    store.insert_articles(&articles(&sources, 20)).unwrap();

    let all = store.list_articles(None, 100).unwrap();
    assert!(all.windows(2).all(|w| w[0].sort_at >= w[1].sort_at));

    let one = store.list_articles(Some(sources[1]), 100).unwrap();
    assert_eq!(one.len(), 10);
    assert!(one.iter().all(|r| r.source_id == sources[1]));
}

#[test]
fn summary_loads_on_demand() {
    let mut store = Store::open_in_memory().unwrap();
    let sources = add_feeds(&store, 1);
    store.insert_articles(&articles(&sources, 1)).unwrap();
    let id = store.list_articles(None, 1).unwrap()[0].id;
    assert!(store.summary(id).unwrap().unwrap().starts_with("<p>"));
    assert_eq!(store.summary(id + 1).unwrap(), None);
}

/// Acceptance for S1.3: 10k rows list in under 20 ms. Best of five, so a
/// noisy CI neighbour doesn't fail the build; `cargo bench` has the full picture.
#[test]
fn ten_thousand_articles_list_in_under_20_ms() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("gutenberg.db")).unwrap();
    let sources = add_feeds(&store, 50);
    assert_eq!(
        store.insert_articles(&articles(&sources, 10_000)).unwrap(),
        10_000
    );

    let mut best = Duration::MAX;
    for _ in 0..5 {
        let start = Instant::now();
        let rows = store.list_articles(None, 10_000).unwrap();
        best = best.min(start.elapsed());
        assert_eq!(rows.len(), 10_000);
    }
    println!("list 10k: {best:?}");
    assert!(best < Duration::from_millis(20), "took {best:?}");
}

#[test]
fn a_star_lasts_and_comes_off() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.db");
    let id = {
        let mut store = Store::open(&path).unwrap();
        let sources = add_feeds(&store, 1);
        store.insert_articles(&articles(&sources, 2)).unwrap();
        let id = store.list_articles(None, 10).unwrap()[1].id;
        store.set_starred(id, true).unwrap();
        id
    };
    let store = Store::open(&path).unwrap();
    let starred: Vec<i64> = store
        .list_articles(None, 10)
        .unwrap()
        .into_iter()
        .filter(|r| r.starred)
        .map(|r| r.id)
        .collect();
    assert_eq!(starred, [id]);
    store.set_starred(id, false).unwrap();
    assert!(store
        .list_articles(None, 10)
        .unwrap()
        .iter()
        .all(|r| !r.starred));
}

#[test]
fn a_sources_keys_come_newest_first_and_a_link_can_move() {
    let mut store = Store::open_in_memory().unwrap();
    let sources = add_feeds(&store, 2);
    store.insert_articles(&articles(&sources, 6)).unwrap();
    let keys = store.article_keys(sources[1]).unwrap();
    // Articles 1, 3 and 5 are the second feed's.
    let guids: Vec<_> = keys.iter().map(|k| k.guid.as_str()).collect();
    assert_eq!(guids, ["urn:article:5", "urn:article:3", "urn:article:1"]);
    assert_eq!(keys[0].url.as_deref(), Some("https://example.com/5"));

    store
        .set_article_url(keys[0].id, "https://publisher.example/5")
        .unwrap();
    let row = store
        .list_articles(Some(sources[1]), 10)
        .unwrap()
        .into_iter()
        .find(|r| r.id == keys[0].id)
        .unwrap();
    assert_eq!(row.url.as_deref(), Some("https://publisher.example/5"));
}
