//! S10.4: what learned interest learns from. Opening or starring an article
//! counts for it; marking it read without opening counts against; marking
//! it unread again takes the skip back.

use gutenberg_store::{Filter, NewArticle, NewSource, Signal, SourceKind, Store, View};

/// Feed `a` with a1 and a2, feed `b` with b1 and b2; a1 and b1 are one
/// story. Returns the store, the feeds and the ids by guid.
fn store() -> (Store, i64, i64, impl Fn(&str) -> i64) {
    let mut store = Store::open_in_memory().unwrap();
    let feed = |name: &str| {
        store
            .add_source(&NewSource {
                folder_id: None,
                kind: SourceKind::Feed,
                title: name.to_owned(),
                url: Some(format!("https://{name}.example/feed")),
                config: "{}".to_owned(),
            })
            .unwrap()
            .id
    };
    let (a, b) = (feed("a"), feed("b"));
    let article = |source_id, guid: &str| NewArticle {
        source_id,
        guid: guid.to_owned(),
        url: None,
        title: guid.to_owned(),
        author: None,
        summary_html: None,
        published_at: Some(100),
        fetched_at: 100,
    };
    store
        .insert_articles(&[
            article(a, "a1"),
            article(a, "a2"),
            article(b, "b1"),
            article(b, "b2"),
        ])
        .unwrap();
    let ids: Vec<(String, i64)> = store
        .list_articles(None, 10)
        .unwrap()
        .into_iter()
        .map(|r| (r.title, r.id))
        .collect();
    let id = move |guid: &str| ids.iter().find(|(g, _)| g == guid).unwrap().1;
    store
        .set_stories(&[
            (id("a1"), id("a1")),
            (id("b1"), id("a1")),
            (id("a2"), id("a2")),
            (id("b2"), id("b2")),
        ])
        .unwrap();
    (store, a, b, id)
}

fn signals(store: &Store) -> Vec<(String, bool)> {
    let mut all: Vec<_> = store
        .signals()
        .unwrap()
        .into_iter()
        .map(
            |Signal {
                 title, interested, ..
             }| (title, interested),
        )
        .collect();
    all.sort();
    all
}

fn s(title: &str, interested: bool) -> (String, bool) {
    (title.to_owned(), interested)
}

#[test]
fn nothing_done_is_nothing_learned() {
    let (store, ..) = store();
    assert!(store.signals().unwrap().is_empty());
}

#[test]
fn opening_and_starring_count_for_an_article() {
    let (store, a, _, id) = store();
    store.record_opened(id("a1"), 200).unwrap();
    store.record_opened(id("a1"), 300).unwrap();
    store.set_starred(id("b2"), true).unwrap();
    assert_eq!(signals(&store), [s("a1", true), s("b2", true)]);
    let a1 = store
        .signals()
        .unwrap()
        .into_iter()
        .find(|s| s.title == "a1");
    assert_eq!(a1.unwrap().source_id, a);
}

#[test]
fn marking_a_story_read_unopened_skips_each_unread_article_of_it() {
    let (store, _, _, id) = store();
    store.record_story_skipped(id("a1"), 200).unwrap();
    store.set_story_read(id("a1"), true).unwrap();
    assert_eq!(signals(&store), [s("a1", false), s("b1", false)]);

    // Marked unread again: not passed over after all.
    store.forget_story_skipped(id("a1")).unwrap();
    store.set_story_read(id("a1"), false).unwrap();
    assert!(signals(&store).is_empty());
}

#[test]
fn opening_wins_over_a_skip_either_way_round() {
    let (store, _, _, id) = store();
    store.record_opened(id("a2"), 200).unwrap();
    store.record_story_skipped(id("a2"), 300).unwrap();
    store.record_story_skipped(id("b2"), 300).unwrap();
    store.record_opened(id("b2"), 400).unwrap();
    assert_eq!(signals(&store), [s("a2", true), s("b2", true)]);

    // Taking a skip back leaves the opening.
    store.forget_story_skipped(id("b2")).unwrap();
    assert_eq!(signals(&store), [s("a2", true), s("b2", true)]);
}

#[test]
fn marking_a_feed_read_skips_what_it_marks_and_nothing_else() {
    let (store, _, b, id) = store();
    store.set_read(id("b2"), true).unwrap();
    store
        .record_skipped_in(Filter::Source(b), View::All, 200)
        .unwrap();
    store.mark_read_in(Filter::Source(b), View::All).unwrap();
    // b1's story takes a1 with it; b2 was read already.
    assert_eq!(signals(&store), [s("a1", false), s("b1", false)]);
}

#[test]
fn a_deleted_article_takes_its_signal_with_it() {
    let (store, a, _, id) = store();
    store.record_opened(id("a2"), 200).unwrap();
    store.delete_source(a).unwrap();
    assert!(signals(&store).is_empty());
}
