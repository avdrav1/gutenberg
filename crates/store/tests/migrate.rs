use gutenberg_store::{Error, NewArticle, NewSource, SourceKind, Store};
use rusqlite::Connection;

fn feed(url: &str) -> NewSource {
    NewSource {
        folder_id: None,
        kind: SourceKind::Feed,
        title: url.to_owned(),
        url: Some(url.to_owned()),
        config: "{}".to_owned(),
    }
}

fn article(source_id: i64, guid: &str) -> NewArticle {
    NewArticle {
        source_id,
        guid: guid.to_owned(),
        url: None,
        title: guid.to_owned(),
        author: None,
        summary_html: None,
        published_at: Some(1_700_000_000),
        fetched_at: 1_700_000_100,
    }
}

#[test]
fn fresh_file_is_created_and_migrated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gutenberg").join("gutenberg.db");
    let store = Store::open(&path).unwrap();
    assert!(path.exists());
    assert_eq!(store.schema_version().unwrap(), 1);
}

#[test]
fn reopening_an_existing_file_keeps_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gutenberg.db");
    {
        let mut store = Store::open(&path).unwrap();
        let folder = store.add_folder("Tech").unwrap();
        let source = store
            .add_source(&NewSource {
                folder_id: Some(folder.id),
                ..feed("https://lwn.net/headlines/rss")
            })
            .unwrap();
        store.insert_articles(&[article(source.id, "a")]).unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), 1);
    assert_eq!(store.folders().unwrap()[0].name, "Tech");
    assert_eq!(store.sources().unwrap().len(), 1);
    assert_eq!(store.list_articles(None, 10).unwrap().len(), 1);
}

#[test]
fn an_empty_existing_file_is_migrated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gutenberg.db");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated (x)")
        .unwrap();
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), 1);
}

#[test]
fn a_file_from_a_newer_build_is_refused_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gutenberg.db");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch("PRAGMA user_version = 99").unwrap();
    drop(raw);

    let err = Store::open(&path)
        .err()
        .expect("newer schema must be refused");
    assert!(
        matches!(
            err,
            Error::TooNew {
                found: 99,
                known: 1
            }
        ),
        "{err}"
    );

    let raw = Connection::open(&path).unwrap();
    let version: i64 = raw
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    let tables: i64 = raw
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!((version, tables), (99, 0));
}

#[test]
fn deleting_a_source_deletes_its_articles() {
    let mut store = Store::open_in_memory().unwrap();
    let keep = store.add_source(&feed("https://a.example/feed")).unwrap();
    let gone = store.add_source(&feed("https://b.example/feed")).unwrap();
    store
        .insert_articles(&[article(keep.id, "a"), article(gone.id, "b")])
        .unwrap();
    store.delete_source(gone.id).unwrap();
    let left = store.list_articles(None, 10).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].source_id, keep.id);
}

#[test]
fn deleting_a_folder_keeps_its_sources() {
    let store = Store::open_in_memory().unwrap();
    let folder = store.add_folder("News").unwrap();
    store
        .add_source(&NewSource {
            folder_id: Some(folder.id),
            ..feed("https://a.example/feed")
        })
        .unwrap();
    store.delete_folder(folder.id).unwrap();
    assert_eq!(store.sources().unwrap()[0].folder_id, None);
}

#[test]
fn the_same_feed_url_cannot_be_added_twice() {
    let store = Store::open_in_memory().unwrap();
    store.add_source(&feed("https://a.example/feed")).unwrap();
    assert!(store.add_source(&feed("https://a.example/feed")).is_err());
}

/// S3.6 relies on FTS5, which the bundled SQLite compiles in.
#[test]
fn bundled_sqlite_has_fts5() {
    Connection::open_in_memory()
        .unwrap()
        .execute_batch("CREATE VIRTUAL TABLE t USING fts5(body)")
        .unwrap();
}
