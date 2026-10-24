use criterion::{criterion_group, criterion_main, Criterion};
use gutenberg_store::{Filter, NewArticle, NewSource, SourceKind, Store, View};

/// Budget: 10k articles list in under 20 ms (S1.3).
fn list_10k(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("gutenberg.db")).unwrap();
    let sources: Vec<i64> = (0..50)
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
        .collect();
    let articles: Vec<NewArticle> = (0..10_000)
        .map(|i| NewArticle {
            source_id: sources[i % sources.len()],
            guid: format!("urn:article:{i}"),
            url: Some(format!("https://example.com/{i}")),
            title: format!("Headline number {i} about the news of the day"),
            author: Some("A. Writer".to_owned()),
            summary_html: Some("<p>Summary paragraph.</p>".repeat(8)),
            published_at: Some(1_700_000_000 + i as i64 * 60),
            fetched_at: 1_700_000_000 + i as i64 * 60 + 30,
        })
        .collect();
    store.insert_articles(&articles).unwrap();

    c.bench_function("list_10k", |b| {
        b.iter(|| store.list_articles(None, 10_000).unwrap())
    });

    // S5.4: the list the window shows, with every article grouped and
    // every fifth story five articles long.
    let ids: Vec<i64> = store
        .list_articles(None, 10_000)
        .unwrap()
        .iter()
        .map(|r| r.id)
        .collect();
    let stories: Vec<(i64, i64)> = ids
        .iter()
        .enumerate()
        .map(|(i, &id)| {
            let story = if (i / 5) % 5 == 0 { ids[i - i % 5] } else { id };
            (id, story)
        })
        .collect();
    store.set_stories(&stories).unwrap();
    c.bench_function("list_in_10k_stories", |b| {
        b.iter(|| {
            store
                .list_articles_in(Filter::All, View::All, 10_000)
                .unwrap()
        })
    });
}

criterion_group!(benches, list_10k);
criterion_main!(benches);
