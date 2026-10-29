use gutenberg_store::{Error, NewArticle, NewSource, SourceKind, Store, SCHEMA_VERSION};
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
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
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
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
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
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
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
        matches!(err, Error::TooNew { found: 99, known } if known == SCHEMA_VERSION),
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

/// S2.3: a file written by 0.1.0 (schema 1) gains the fetch-state columns
/// and keeps its rows.
#[test]
fn a_version_1_file_upgrades_and_keeps_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gutenberg.db");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../migrations/0001_init.sql"))
        .unwrap();
    raw.execute_batch(
        "PRAGMA user_version = 1;
         INSERT INTO source (kind, title, url, position, created_at)
           VALUES ('feed', 'LWN', 'https://lwn.net/headlines/rss', 0, 1);
         INSERT INTO article (source_id, guid, title, fetched_at, sort_at, read)
           VALUES (1, 'a', 'A', 1, 1, 1);",
    )
    .unwrap();
    drop(raw);

    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    let rows = store.list_articles(None, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].read);
    let state = store.fetch_state(1).unwrap();
    assert_eq!(
        (state.failures, state.checked_at, state.error),
        (0, None, None)
    );
    assert_eq!(store.sources_due(0).unwrap().len(), 1);
}

#[test]
fn a_version_3_file_upgrades_with_its_articles_ungrouped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gutenberg.db");
    {
        let mut store = Store::open(&path).unwrap();
        let source = store.add_source(&feed("https://a.example/feed")).unwrap();
        store
            .insert_articles(&[article(source.id, "a"), article(source.id, "b")])
            .unwrap();
    }
    {
        // Back to the file 0.15.0 left: schema 3, no story column and no
        // setting table.
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch(
            "DROP TABLE setting;
             DROP INDEX article_by_story;
             DROP INDEX article_ungrouped;
             ALTER TABLE article DROP COLUMN story_id;
             PRAGMA user_version = 3;",
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    let rows = store
        .list_articles_in(gutenberg_store::Filter::All, gutenberg_store::View::All, 10)
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.story_size == 1 && r.story_id == r.id));
    assert_eq!(store.story_candidates(0).unwrap().len(), 2);
}

/// S11.1: a file from 0.26.0 (schema 4) gains the setting table and keeps
/// its articles and stories.
#[test]
fn a_version_4_file_upgrades_and_can_hold_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gutenberg.db");
    {
        let mut store = Store::open(&path).unwrap();
        let source = store.add_source(&feed("https://a.example/feed")).unwrap();
        store.insert_articles(&[article(source.id, "a")]).unwrap();
    }
    {
        // Back to the file 0.26.0 left: schema 4, no setting table.
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch("DROP TABLE setting; PRAGMA user_version = 4;")
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(store.list_articles(None, 10).unwrap().len(), 1);
    assert_eq!(store.setting("layout.widths").unwrap(), None);
    store.set_setting("layout.widths", "0.2,0.3,0.5").unwrap();
    assert!(store.setting("layout.widths").unwrap().is_some());
}
