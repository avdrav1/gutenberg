//! S5.1: a Google News search, captured for the query "Cory Booker", reads
//! as articles with the publisher as author. S5.2: so does the same search
//! on Bing, and the two merge with the stories both carry once. Expected
//! values were read from the raw files with a separate reader. Fixtures:
//! `tests/fixtures/feeds/{google,bing}-news-cory-booker.xml`, and the
//! Google article page and replies in `tests/fixtures/pages/`.

use gutenberg_sources::feed::{self, Feed};
use gutenberg_sources::google_news;
use gutenberg_sources::keyword::{
    merge, tidy, tidy_google_news, Keyword, Provider, Providers, Seen,
};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

fn search(provider: Provider, file: &str) -> Feed {
    let url = Keyword::new("\"Cory Booker\"")
        .unwrap()
        .search_url(provider, &Providers::default());
    let xml = std::fs::read(format!("{FIXTURES}/feeds/{file}")).unwrap();
    let mut feed = feed::parse(&xml, &url).unwrap();
    tidy(provider, &mut feed, &xml);
    feed
}

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

#[test]
fn bing_news_cory_booker() {
    let feed = search(Provider::BingNews, "bing-news-cory-booker.xml");
    assert_eq!(feed.entries.len(), 10);
    let first = &feed.entries[0];
    assert_eq!(
        first.title,
        "Could Cory Booker lose his NJ Senate seat? What forecasts show"
    );
    let publisher = "https://www.msn.com/en-us/news/other/\
                     could-cory-booker-lose-his-nj-senate-seat-what-forecasts-show/ar-AA2fVael";
    assert_eq!(first.url.as_deref(), Some(publisher));
    // The link Bing sent changes with every search; the publisher's doesn't.
    assert_eq!(first.guid, publisher);
    assert_eq!(first.author.as_deref(), Some("NorthJersey.com on MSN"));
    assert_eq!(first.published_at, Some(1792734998));
    assert_eq!(first.summary_html.as_deref().map(str::len), Some(134));
    assert!(feed
        .entries
        .iter()
        .all(|e| e.author.is_some() && e.url.as_deref().is_some_and(|u| !u.contains("bing.com"))));
}

#[test]
fn the_same_story_from_both_providers_appears_once() {
    let bing = search(Provider::BingNews, "bing-news-cory-booker.xml");
    let google = search(Provider::GoogleNews, "google-news-cory-booker.xml");
    let mut seen = Seen::default();
    let merged = merge(vec![bing.entries, google.entries], &mut seen);
    // 10 + 100, less the 4 headlines both carry and one Google carries
    // twice, from two papers that ran the same piece.
    assert_eq!(merged.len(), 105);
    let forecasts: Vec<_> = merged
        .iter()
        .filter(|e| e.title == "Could Cory Booker lose his NJ Senate seat? What forecasts show")
        .collect();
    assert_eq!(forecasts.len(), 1);
    // Bing's comes first, with the publisher's own link.
    assert!(forecasts[0]
        .url
        .as_deref()
        .unwrap()
        .starts_with("https://www.msn.com/"));
    // A second search that brings the same results adds nothing.
    let again = merge(
        vec![
            search(Provider::BingNews, "bing-news-cory-booker.xml").entries,
            search(Provider::GoogleNews, "google-news-cory-booker.xml").entries,
        ],
        &mut seen,
    );
    assert!(again.is_empty());
}

#[test]
fn a_google_news_page_signs_the_request_for_its_publisher() {
    let page =
        std::fs::read_to_string(format!("{FIXTURES}/pages/google-news-article.html")).unwrap();
    let sig = google_news::signature(&page).unwrap();
    assert!(sig.id.starts_with("CBMimgFBVV95cUxPSHBRZ0prS1Ax"));
    assert_eq!(sig.timestamp, "1792862196");
    assert_eq!(sig.signature, "AbIaSL836_nOxRwo391KH9t5Lj7p");
    // The same article as the Google fixture's first entry.
    let google = search(Provider::GoogleNews, "google-news-cory-booker.xml");
    assert!(google.entries[0].url.as_deref().unwrap().contains(&sig.id));

    let reply = |file| std::fs::read_to_string(format!("{FIXTURES}/pages/{file}")).unwrap();
    assert_eq!(
        google_news::publisher_url(&reply("google-news-batchexecute.txt")).as_deref(),
        Some("https://www.cnn.com/2026/10/23/politics/video/the-arena-sen-cory-booker-doj-alaska-senate-race")
    );
    assert_eq!(
        google_news::publisher_url(&reply("google-news-batchexecute-refused.txt")),
        None
    );
}
