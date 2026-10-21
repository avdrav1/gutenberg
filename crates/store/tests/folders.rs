//! S2.4: folders, source order and unread counts; order survives reopening.

use gutenberg_store::{Filter, NewArticle, NewSource, SourceKind, SourceTarget, Store};

fn feed(store: &Store, name: &str, folder: Option<i64>) -> i64 {
    store
        .add_source(&NewSource {
            folder_id: folder,
            kind: SourceKind::Feed,
            title: name.to_owned(),
            url: Some(format!("https://{name}.example/feed")),
            config: "{}".to_owned(),
        })
        .unwrap()
        .id
}

fn order(store: &Store) -> Vec<(String, Option<i64>)> {
    store
        .sources()
        .unwrap()
        .into_iter()
        .map(|s| (s.title, s.folder_id))
        .collect()
}

#[test]
fn folder_named_finds_or_makes_one() {
    let store = Store::open_in_memory().unwrap();
    let tech = store.folder_named("Tech").unwrap();
    assert_eq!(store.folder_named("Tech").unwrap(), tech);
    assert_eq!(store.folder_named("News").unwrap().position, 1);
    assert_eq!(store.folders().unwrap().len(), 2);
}

#[test]
fn moves_persist_across_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.db");
    {
        let mut store = Store::open(&path).unwrap();
        let tech = store.folder_named("Tech").unwrap().id;
        let news = store.folder_named("News").unwrap().id;
        let a = feed(&store, "a", Some(tech));
        let b = feed(&store, "b", Some(tech));
        let c = feed(&store, "c", None);
        let d = feed(&store, "d", Some(news));

        // c before a: into Tech, first.
        store.move_source(c, SourceTarget::Before(a)).unwrap();
        // b to the end of News.
        store
            .move_source(b, SourceTarget::EndOf(Some(news)))
            .unwrap();
        // d out of any folder.
        store.move_source(d, SourceTarget::EndOf(None)).unwrap();
        // News before Tech.
        store.move_folder(news, Some(tech)).unwrap();
    }
    let store = Store::open(&path).unwrap();
    let tech = store.folder_named("Tech").unwrap().id;
    let news = store.folder_named("News").unwrap().id;
    assert_eq!(
        order(&store),
        [
            ("c".to_owned(), Some(tech)),
            ("a".to_owned(), Some(tech)),
            ("b".to_owned(), Some(news)),
            ("d".to_owned(), None),
        ]
    );
    let folders: Vec<_> = store
        .folders()
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(folders, ["News", "Tech"]);
}

#[test]
fn unread_counts_follow_reading_and_folders_filter() {
    let mut store = Store::open_in_memory().unwrap();
    let tech = store.folder_named("Tech").unwrap().id;
    let a = feed(&store, "a", Some(tech));
    let b = feed(&store, "b", None);
    let article = |source_id, guid: &str| NewArticle {
        source_id,
        guid: guid.to_owned(),
        url: None,
        title: guid.to_owned(),
        author: None,
        summary_html: None,
        published_at: Some(1),
        fetched_at: 1,
    };
    store
        .insert_articles(&[article(a, "1"), article(a, "2"), article(b, "3")])
        .unwrap();
    let counts = store.unread_counts().unwrap();
    assert_eq!((counts[&a], counts[&b]), (2, 1));

    let first = store.list_articles_in(Filter::Source(a), 10).unwrap()[0].id;
    store.set_read(first, true).unwrap();
    assert_eq!(store.unread_counts().unwrap()[&a], 1);

    assert_eq!(
        store
            .list_articles_in(Filter::Folder(tech), 10)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(store.list_articles_in(Filter::All, 10).unwrap().len(), 3);
}
