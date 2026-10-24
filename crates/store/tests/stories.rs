//! S5.4: articles filed under one story list as one row, its newest, with
//! the story's size; reading it reads them all, and counts count stories.

use gutenberg_store::{Filter, NewArticle, NewSource, SourceKind, Store, View};

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

const DAY: i64 = 86_400;

/// Feed `a` (in News) has a1 and a2, feed `b` (unfiled) b1 and b2; a1, b1
/// and b2 are one story, a2 is alone. Returns the store, the feeds and the
/// articles' ids by guid.
fn store() -> (Store, i64, i64, impl Fn(&str) -> i64) {
    let mut store = Store::open_in_memory().unwrap();
    let news = store.folder_named("News").unwrap().id;
    let a = feed(&store, "a", Some(news));
    let b = feed(&store, "b", None);
    let article = |source_id, guid: &str, at| NewArticle {
        source_id,
        guid: guid.to_owned(),
        url: Some(format!("https://site.example/{guid}")),
        title: format!("Headline {guid}"),
        author: None,
        summary_html: None,
        published_at: Some(at),
        fetched_at: at,
    };
    store
        .insert_articles(&[
            article(a, "a1", 100 * DAY),
            article(a, "a2", 100 * DAY + 50),
            article(b, "b1", 100 * DAY + 10),
            article(b, "b2", 100 * DAY + 20),
        ])
        .unwrap();
    let ids: Vec<(String, i64)> = store
        .list_articles(None, 10)
        .unwrap()
        .into_iter()
        .map(|r| (r.title.trim_start_matches("Headline ").to_owned(), r.id))
        .collect();
    let id = move |guid: &str| ids.iter().find(|(g, _)| g == guid).unwrap().1;
    let first = id("a1");
    store
        .set_stories(&[
            (first, first),
            (id("b1"), first),
            (id("b2"), first),
            (id("a2"), id("a2")),
        ])
        .unwrap();
    (store, a, b, id)
}

fn titles(rows: &[gutenberg_store::ArticleRow]) -> Vec<(&str, i64)> {
    rows.iter()
        .map(|r| (r.title.trim_start_matches("Headline "), r.story_size))
        .collect()
}

#[test]
fn a_story_lists_as_its_newest_article_where_it_shows() {
    let (store, a, b, _) = store();
    let all = store.list_articles_in(Filter::All, View::All, 10).unwrap();
    assert_eq!(titles(&all), [("a2", 1), ("b2", 3)]);
    // Within one feed, the story's newest there.
    let in_a = store
        .list_articles_in(Filter::Source(a), View::All, 10)
        .unwrap();
    assert_eq!(titles(&in_a), [("a2", 1), ("a1", 3)]);
    let in_b = store
        .list_articles_in(Filter::Source(b), View::All, 10)
        .unwrap();
    assert_eq!(titles(&in_b), [("b2", 3)]);
    // Every article on its own, for callers that want them.
    assert_eq!(store.list_articles(None, 10).unwrap().len(), 4);
}

#[test]
fn reading_a_story_reads_all_of_it_and_counts_count_stories() {
    let (store, a, b, id) = store();
    let counts = store.unread_counts().unwrap();
    assert_eq!((counts[&a], counts[&b]), (2, 1));
    assert_eq!(store.view_counts().unwrap().unread, 2);

    let story = store.story(id("a1")).unwrap();
    assert_eq!(titles(&story), [("b2", 3), ("b1", 3), ("a1", 3)]);
    store.set_story_read(id("a1"), true).unwrap();
    let counts = store.unread_counts().unwrap();
    assert_eq!((counts[&a], counts.get(&b)), (1, None));
    let unread = store
        .list_articles_in(Filter::All, View::Unread, 10)
        .unwrap();
    assert_eq!(titles(&unread), [("a2", 1)]);

    store.set_story_read(id("a1"), false).unwrap();
    assert_eq!(store.view_counts().unwrap().unread, 2);
    // An article not yet grouped is a story of one.
    store.set_story_read(id("a2"), true).unwrap();
    assert_eq!(store.view_counts().unwrap().unread, 1);
}

#[test]
fn marking_a_feed_read_reads_its_stories_elsewhere() {
    let (store, a, b, _) = store();
    store.mark_read_in(Filter::Source(a), View::All).unwrap();
    assert_eq!(store.unread_counts().unwrap().get(&b), None);
}

#[test]
fn search_shows_a_story_once() {
    let (store, ..) = store();
    let found = store.search("headline", 10).unwrap();
    assert_eq!(titles(&found), [("a2", 1), ("b2", 3)]);
    let found = store.search("b1", 10).unwrap();
    assert_eq!(titles(&found), [("b1", 3)]);
}

#[test]
fn candidates_are_the_ungrouped_and_their_neighbours() {
    let (mut store, a, ..) = store();
    assert!(store.story_candidates(3 * DAY).unwrap().is_empty());
    let article = |guid: &str, at| NewArticle {
        source_id: a,
        guid: guid.to_owned(),
        url: None,
        title: guid.to_owned(),
        author: None,
        summary_html: None,
        published_at: Some(at),
        fetched_at: at,
    };
    store
        .insert_articles(&[article("later", 102 * DAY), article("far", 110 * DAY)])
        .unwrap();
    let near: Vec<_> = store
        .story_candidates(3 * DAY)
        .unwrap()
        .into_iter()
        .map(|c| (c.title, c.story_id.is_some()))
        .collect();
    let near: Vec<_> = near.iter().map(|(t, g)| (t.as_str(), *g)).collect();
    // Oldest first: everything from 99 to 113 days.
    assert_eq!(
        near,
        [
            ("Headline a1", true),
            ("Headline b1", true),
            ("Headline b2", true),
            ("Headline a2", true),
            ("later", false),
            ("far", false),
        ]
    );
}
