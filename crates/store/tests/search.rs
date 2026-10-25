//! S3.6: full-text search over title, author and summary text.

use std::time::{Duration, Instant};

use gutenberg_store::{NewArticle, NewSource, SourceKind, Store, SCHEMA_VERSION};
use rusqlite::Connection;

fn feed(store: &Store, name: &str) -> i64 {
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
}

fn article(source_id: i64, guid: &str, title: &str, summary: &str, at: i64) -> NewArticle {
    NewArticle {
        source_id,
        guid: guid.to_owned(),
        url: None,
        title: title.to_owned(),
        author: Some("Jane Doe".to_owned()),
        summary_html: Some(summary.to_owned()),
        published_at: Some(at),
        fetched_at: at,
    }
}

fn titles(store: &Store, typed: &str) -> Vec<String> {
    store
        .search(typed, 100)
        .unwrap()
        .into_iter()
        .map(|r| r.title)
        .collect()
}

#[test]
fn words_in_title_author_and_text_match_but_markup_does_not() {
    let mut store = Store::open_in_memory().unwrap();
    let a = feed(&store, "a");
    store
        .insert_articles(&[
            article(a, "1", "Harrisburg budget passes", "<p>Lawmakers voted.</p>", 1),
            article(
                a,
                "2",
                "Council meets",
                r#"<p>In <a href="https://pennlive.example/harrisburg">the capital</a>, a vote.</p>"#,
                2,
            ),
            article(a, "3", "Snow day", "<p>Schools in <b>Harrisburg</b> closed.</p>", 3),
            article(a, "4", "Weather", "<div class=\"council\">Rain</div>", 4),
        ])
        .unwrap();
    // Newest first; the href's "harrisburg" doesn't count.
    assert_eq!(
        titles(&store, "Harrisburg"),
        ["Snow day", "Harrisburg budget passes"]
    );
    assert_eq!(
        titles(&store, "harris"),
        ["Snow day", "Harrisburg budget passes"]
    );
    assert_eq!(
        titles(&store, "jane doe").len(),
        4,
        "the author is searched"
    );
    assert_eq!(
        titles(&store, "council"),
        ["Council meets"],
        "not the class name"
    );
    assert_eq!(titles(&store, "harrisburg closed"), ["Snow day"]);
}

#[test]
fn anything_typed_is_safe() {
    let mut store = Store::open_in_memory().unwrap();
    let a = feed(&store, "a");
    store
        .insert_articles(&[article(a, "1", "O'Neal says AND or NOT", "", 1)])
        .unwrap();
    for typed in [
        "\"", "-", "*", "AND", "NOT", "o'neal", "(", "a:b", "NEAR(", "", "   ",
    ] {
        store
            .search(typed, 10)
            .unwrap_or_else(|e| panic!("{typed:?}: {e}"));
    }
    assert_eq!(titles(&store, "o'neal").len(), 1);
    assert_eq!(titles(&store, "AND").len(), 1, "a word, not an operator");
    assert!(titles(&store, " - ").is_empty());
}

#[test]
fn a_deleted_feed_leaves_search() {
    let mut store = Store::open_in_memory().unwrap();
    let a = feed(&store, "a");
    let b = feed(&store, "b");
    store
        .insert_articles(&[
            article(a, "1", "Harrisburg one", "", 1),
            article(b, "2", "Harrisburg two", "", 2),
        ])
        .unwrap();
    store.delete_source(a).unwrap();
    assert_eq!(titles(&store, "harrisburg"), ["Harrisburg two"]);
}

#[test]
fn articles_stored_before_search_existed_are_found_after_upgrading() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gutenberg.db");
    {
        // A file as 0.7.0 left it: schema 2, articles, no index.
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../migrations/0001_init.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0002_fetch_state.sql"))
            .unwrap();
        conn.execute_batch(
            "PRAGMA user_version = 2;
             INSERT INTO source (id, kind, title, url, position, created_at)
               VALUES (1, 'feed', 'PennLive', 'https://pennlive.example/feed', 0, 0);
             INSERT INTO article (source_id, guid, title, summary_html, fetched_at, sort_at)
               VALUES (1, 'g1', 'Capitol news', '<p>From <i>Harrisburg</i> today</p>', 0, 0),
                      (1, 'g2', 'Elsewhere', '<p>Pittsburgh</p>', 0, 1);",
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(titles(&store, "harrisburg"), ["Capitol news"]);
    assert_eq!(store.list_articles(None, 10).unwrap().len(), 2);
}

/// S3.6 acceptance: "Harrisburg" across 100k articles in under 50 ms.
/// Timed only in an optimized build; a debug build checks the result.
#[test]
fn harrisburg_in_100k_articles_under_50_ms() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("gutenberg.db")).unwrap();
    let corpus = corpus(&store, 100_000);
    store.insert_articles(&corpus).unwrap();

    let start = Instant::now();
    let found = store.search("Harrisburg", 100_000).unwrap();
    let took = start.elapsed();
    assert_eq!(found.len(), HARRISBURG);
    assert!(found.windows(2).all(|w| w[0].sort_at >= w[1].sort_at));
    if !cfg!(debug_assertions) {
        assert!(took < Duration::from_millis(50), "took {took:?}");
    }
}

/// Articles that mention Harrisburg in the corpus below.
pub const HARRISBURG: usize = 37;

/// `n` articles over 50 feeds; every 2,700th mentions Harrisburg, in the
/// title or deep in the summary.
pub fn corpus(store: &Store, n: usize) -> Vec<NewArticle> {
    let sources: Vec<i64> = (0..50).map(|i| feed(store, &format!("site{i}"))).collect();
    const WORDS: &[&str] = &[
        "council", "budget", "storm", "election", "school", "river", "bridge", "court", "market",
        "transit", "hospital", "senate", "harbor", "museum", "festival", "factory",
    ];
    (0..n)
        .map(|i| {
            let w = |k: usize| WORDS[(i * 7 + k * 13) % WORDS.len()];
            let mention = i % 2_700 == 0 && i / 2_700 < HARRISBURG;
            let title = if mention && i % 2 == 0 {
                format!("Harrisburg {} {} update", w(0), w(1))
            } else {
                format!("{} {} report number {i}", w(0), w(1))
            };
            let tail = if mention && i % 2 != 0 {
                " in Harrisburg."
            } else {
                "."
            };
            NewArticle {
                source_id: sources[i % sources.len()],
                guid: format!("urn:article:{i}"),
                url: Some(format!("https://example.com/{i}")),
                title,
                author: Some("A. Writer".to_owned()),
                summary_html: Some(format!(
                    "<p>The {} met over the {} and the {}{tail}</p>",
                    w(2),
                    w(3),
                    w(4)
                )),
                published_at: Some(1_700_000_000 + i as i64 * 60),
                fetched_at: 1_700_000_000 + i as i64 * 60 + 30,
            }
        })
        .collect()
}

#[test]
fn a_reader_page_once_read_is_searched_too() {
    // S7.4: the page's words join the summary's; indexing it again replaces
    // them rather than adding a second copy.
    let mut store = Store::open_in_memory().unwrap();
    let a = feed(&store, "a");
    let url = "https://a.example/budget";
    let mut first = article(a, "1", "Budget passes", "<p>Lawmakers voted.</p>", 1);
    first.url = Some(url.to_owned());
    store
        .insert_articles(&[first, article(a, "2", "Snow day", "<p>Closed.</p>", 2)])
        .unwrap();
    assert!(titles(&store, "transit").is_empty());

    let page = "<article><p>Transit gets <b>$300 million</b>.</p><script>var x</script></article>";
    assert_eq!(store.index_page(url, page).unwrap(), 1);
    assert_eq!(titles(&store, "transit"), ["Budget passes"]);
    // The summary's words still match, and markup never does.
    assert_eq!(titles(&store, "lawmakers"), ["Budget passes"]);
    assert!(titles(&store, "script").is_empty());

    assert_eq!(
        store
            .index_page(url, "<p>Schools get a formula fix.</p>")
            .unwrap(),
        1
    );
    assert!(titles(&store, "transit").is_empty());
    assert_eq!(titles(&store, "formula"), ["Budget passes"]);
    // A page no article links to changes nothing.
    assert_eq!(
        store
            .index_page("https://a.example/other", "<p>x</p>")
            .unwrap(),
        0
    );
}
