//! S3.7: views narrow a feed or folder's articles to the unread, starred or
//! today's ones; the counts match the lists.

use gutenberg_store::{Filter, NewArticle, NewSource, SourceKind, Store, View, ViewCounts};

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

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Tech holds `a`; `b` is unfiled. "new" articles are published now, so
/// always today; "old" ones three days ago, never today.
fn store() -> (Store, i64) {
    let mut store = Store::open_in_memory().unwrap();
    let tech = store.folder_named("Tech").unwrap().id;
    let a = feed(&store, "a", Some(tech));
    let b = feed(&store, "b", None);
    let article = |source_id, guid: &str, at| NewArticle {
        source_id,
        guid: guid.to_owned(),
        url: None,
        title: guid.to_owned(),
        author: None,
        summary_html: None,
        published_at: Some(at),
        fetched_at: at,
    };
    let (today, old) = (now(), now() - 3 * 86_400);
    store
        .insert_articles(&[
            article(a, "a-new", today),
            article(a, "a-old", old),
            article(b, "b-new", today),
            article(b, "b-old", old - 1),
        ])
        .unwrap();
    // a-old read; a-old and b-new starred.
    let id = |store: &Store, title: &str| {
        store
            .list_articles(None, 10)
            .unwrap()
            .into_iter()
            .find(|r| r.title == title)
            .unwrap()
            .id
    };
    store.set_read(id(&store, "a-old"), true).unwrap();
    store.set_starred(id(&store, "a-old"), true).unwrap();
    store.set_starred(id(&store, "b-new"), true).unwrap();
    (store, tech)
}

fn titles(store: &Store, filter: Filter, view: View) -> Vec<String> {
    store
        .list_articles_in(filter, view, 100)
        .unwrap()
        .into_iter()
        .map(|r| r.title)
        .collect()
}

#[test]
fn each_view_lists_its_articles_everywhere_and_in_a_folder() {
    let (store, tech) = store();
    let all = Filter::All;
    assert_eq!(
        titles(&store, all, View::All),
        ["b-new", "a-new", "a-old", "b-old"]
    );
    assert_eq!(
        titles(&store, all, View::Unread),
        ["b-new", "a-new", "b-old"]
    );
    assert_eq!(titles(&store, all, View::Starred), ["b-new", "a-old"]);
    assert_eq!(titles(&store, all, View::Today), ["b-new", "a-new"]);

    let tech = Filter::Folder(tech);
    assert_eq!(titles(&store, tech, View::Unread), ["a-new"]);
    assert_eq!(titles(&store, tech, View::Starred), ["a-old"]);
    assert_eq!(titles(&store, tech, View::Today), ["a-new"]);
}

#[test]
fn counts_match_the_lists() {
    let (store, _) = store();
    assert_eq!(
        store.view_counts().unwrap(),
        ViewCounts {
            unread: 3,
            starred: 2,
            today: 2
        }
    );
}

#[test]
fn marking_a_view_read_leaves_the_rest_unread() {
    let (store, _) = store();
    // Starred everywhere: only b-new is starred and unread.
    assert_eq!(store.mark_read_in(Filter::All, View::Starred).unwrap(), 1);
    assert_eq!(
        titles(&store, Filter::All, View::Unread),
        ["a-new", "b-old"]
    );
    // Today: a-new is left.
    assert_eq!(store.mark_read_in(Filter::All, View::Today).unwrap(), 1);
    assert_eq!(titles(&store, Filter::All, View::Unread), ["b-old"]);
}
