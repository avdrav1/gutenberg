//! S5.1: a Google News search, captured for the query "Cory Booker", reads
//! as articles with the publisher as author. Expected values were read from
//! the raw file with a separate reader. Fixture:
//! `tests/fixtures/feeds/google-news-cory-booker.xml`.

use gutenberg_sources::feed;
use gutenberg_sources::keyword::{tidy_google_news, Keyword};

#[test]
fn google_news_cory_booker() {
    let url = Keyword::new("\"Cory Booker\"").unwrap().google_news_url();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/feeds/google-news-cory-booker.xml"
    );
    let mut feed = feed::parse(&std::fs::read(path).unwrap(), &url).unwrap();
    tidy_google_news(&mut feed);
    assert_eq!(feed.title, "\"\"Cory Booker\"\" - Google News");
    assert_eq!(feed.entries.len(), 100);
    let first = &feed.entries[0];
    assert_eq!(
        first.title,
        "It's 'insulting Alaskan voters': Sen. Booker on the DOJ lawsuit to block \
         Alaska Senate results if Republican incumbent loses"
    );
    assert_eq!(first.author.as_deref(), Some("CNN"));
    assert!(first
        .url
        .as_deref()
        .unwrap()
        .starts_with("https://news.google.com/rss/articles/CBMimgFBVV95cUxPSHBRZ0prS1Ax"));
    assert_eq!(first.published_at, Some(1792788740));
    // Every title carried its publisher, and every one moved to the author.
    assert!(feed
        .entries
        .iter()
        .all(|e| e.author.is_some() && !e.title.ends_with(e.author.as_deref().unwrap())));
}
