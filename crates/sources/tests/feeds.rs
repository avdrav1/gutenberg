//! S2.1 acceptance: 20 captured feeds parse; bad input fails with one line.
//! Expected values were checked against the raw files with a separate
//! reader, not copied from this parser. Fixtures: `tests/fixtures/feeds/`.

use gutenberg_sources::feed::{self, Feed, FeedError};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../../tests/fixtures/feeds/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn parse(name: &str, url: &str) -> Feed {
    feed::parse(&fixture(name), url).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// One test per fixture, so `cargo test <slug>` runs just that one.
macro_rules! feeds {
    ($($slug:ident: $file:literal from $url:literal => $title:literal, $count:literal entries,
        first $first:literal at $link:literal, $published:expr, summary $summary:literal chars;)+) => {$(
        #[test]
        fn $slug() {
            let feed = parse($file, $url);
            assert_eq!(feed.title, $title);
            assert_eq!(feed.entries.len(), $count);
            let e = &feed.entries[0];
            assert_eq!(e.title, $first);
            assert_eq!(e.url.as_deref(), Some($link));
            assert_eq!(e.published_at, $published);
            assert_eq!(e.summary_html.as_ref().map_or(0, |s| s.chars().count()), $summary);
            assert!(feed.entries.iter().all(|e| !e.guid.is_empty() && !e.title.is_empty()));
        }
    )+};
}

feeds! {
    // RSS 2.0
    archlinux_news: "archlinux-news.xml" from "https://archlinux.org/feeds/news/"
        => "Arch Linux: Recent news updates", 10 entries,
        first "Mkinitcpio >=42 requires manual intervention for TPM2-based unlocking of LUKS devices"
        at "https://archlinux.org/news/mkinitcpio-42-requires-manual-intervention-for-tpm2-based-unlocking-of-luks-devices/",
        Some(1790068167), summary 1054 chars;
    lwn_headlines: "lwn-headlines.xml" from "https://lwn.net/headlines/rss"
        => "LWN.net", 10 entries,
        first "Linux Plumbers Conference videos now available" at "https://lwn.net/Articles/1101483/",
        Some(1792436695), summary 683 chars;
    hnrss_frontpage: "hnrss-frontpage.xml" from "https://hnrss.org/frontpage"
        => "Hacker News: Front Page", 10 entries,
        first "Show HN: Death Plan Switch, a self-hosted dead man's switch for solo devs"
        at "https://deathplanswitch.com/", Some(1792471063), summary 1177 chars;
    bbc_news: "bbc-news.xml" from "https://feeds.bbci.co.uk/news/rss.xml"
        => "BBC News", 10 entries,
        first "Chancellor rejects calls to raise Capital Gains Tax in Budget"
        at "https://www.bbc.co.uk/news/articles/c50me4w4x1vyo?at_medium=RSS&at_campaign=rss",
        Some(1792483577), summary 148 chars;
    nyt_home: "nyt-home.xml" from "https://rss.nytimes.com/services/xml/rss/nyt/HomePage.xml"
        => "NYT > Top Stories", 10 entries,
        first "Trump’s Push for Deals With Russia Powers His Frustration With Ukraine"
        at "https://www.nytimes.com/2026/10/19/us/politics/trump-russia-deals.html",
        Some(1792480754), summary 198 chars;
    npr_news: "npr-news.xml" from "https://feeds.npr.org/1001/rss.xml"
        => "NPR Topics: News", 10 entries,
        first "Rays will face Brewers in World Series to produce a first-time champion"
        at "https://www.npr.org/2026/10/20/nx-s1-6008836/rays-brewers-world-series",
        Some(1792477690), summary 727 chars;
    kernel_releases: "kernel-releases.xml" from "https://www.kernel.org/feeds/kdist.xml"
        => "Latest Linux Kernel Versions", 9 entries,
        first "7.3: mainline" at "https://www.kernel.org/",
        // "Sun, 18 Oct 2026 23:14:58 -0000"
        Some(1792365298), summary 680 chars;
    omgubuntu: "omgubuntu.xml" from "https://www.omgubuntu.co.uk/feed"
        => "OMG! Ubuntu", 10 entries,
        first "BleachBit 6.0.6 released with fixes for admin cleaning on Linux"
        at "https://www.omgubuntu.co.uk/2026/10/bleachbit-606-update-speed-boost",
        Some(1792454264), summary 1546 chars;
    mastodon_user: "mastodon-user.xml" from "https://mastodon.social/@Mastodon.rss"
        => "Mastodon", 10 entries,
        // Toots have no title; one is made from the text.
        first "We didn’t want to require you to have a GitHub profile to report issues or sugge…"
        at "https://mastodon.social/@Mastodon/117466985005297414",
        Some(1792403945), summary 513 chars;
    pennlive: "pennlive.xml" from "https://www.pennlive.com/arc/outboundfeeds/rss/?outputType=xml"
        => "pennlive.com", 10 entries,
        first "Powerball: See the winning numbers in Monday’s $607 million drawing"
        at "https://www.pennlive.com/lottery/2026/10/powerball-see-the-winning-numbers-in-mondays-607-million-drawing.html",
        Some(1792465735), summary 546 chars;
    // RSS 1.0 (RDF)
    slashdot_main: "slashdot-main.xml" from "http://rss.slashdot.org/Slashdot/slashdotMain"
        => "Slashdot", 10 entries,
        first "Japan Launches Historic MMX Sample-Return Mission to Mars' Moon Phobos"
        at "https://science.slashdot.org/story/26/10/20/0522251/japan-launches-historic-mmx-sample-return-mission-to-mars-moon-phobos",
        Some(1792479600), summary 3498 chars;
    planet_debian: "planet-debian.xml" from "https://planet.debian.org/rss10.xml"
        => "Planet Debian", 10 entries,
        first "Russell Coker: Cixin Liu is a Bad Person"
        at "https://etbe.coker.com.au/2026/10/20/cixin-liu-bad-person/",
        Some(1792483919), summary 5163 chars;
    // Atom
    omarchy_releases: "omarchy-releases.xml" from "https://github.com/basecamp/omarchy/releases.atom"
        => "Release notes from omarchy", 5 entries,
        first "v4.5.0-rc1: Merge pull request #15932 from omacom/notifications-omapager"
        at "https://github.com/omacom/omarchy/releases/tag/v4.5.0-rc1",
        Some(1792478700), summary 52 chars;
    rust_blog: "rust-blog.xml" from "https://blog.rust-lang.org/feed.xml"
        => "Rust Blog", 10 entries,
        first "Announcing Rust 1.99.1" at "https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/",
        Some(1792022400), summary 3076 chars;
    verge_index: "verge-index.xml" from "https://www.theverge.com/rss/index.xml"
        => "The Verge", 10 entries,
        first "Baseus’ new security camera has a spotlight that tracks and follows moving targets"
        at "https://www.theverge.com/tech/1005073/baseus-security-primesentinel-u2-pro-outdoor-3k-dual-camera-tracking-spotlight",
        Some(1792468800), summary 1539 chars;
    youtube_channel: "youtube-channel.xml"
        from "https://www.youtube.com/feeds/videos.xml?channel_id=UCsBjURrPoezykLs9EqgamOA"
        => "Fireship", 10 entries,
        // No content element: the summary is the escaped media:description.
        first "The runtime wars are over... and Node won" at "https://www.youtube.com/watch?v=l3L_lETiSzc",
        Some(1792085715), summary 335 chars;
    reddit_rust: "reddit-rust.xml" from "https://www.reddit.com/r/rust/.rss"
        => "The Rust Programming Language", 10 entries,
        first "No More Code Dumps" at "https://www.reddit.com/r/rust/comments/1wkmzun/no_more_code_dumps/",
        Some(1789826388), summary 6285 chars;
    // JSON Feed
    jsonfeed_org: "jsonfeed-org.json" from "https://www.jsonfeed.org/feed.json"
        => "JSON Feed", 2 entries,
        first "JSON Feed version 1.1" at "https://www.jsonfeed.org/2020/08/07/json-feed-version.html",
        Some(1596818676), summary 671 chars;
    daringfireball_json: "daringfireball-json.json" from "https://daringfireball.net/feeds/json"
        => "Daring Fireball", 10 entries,
        first "Barack Obama on the Trump Administration’s Intimidation Campaign Against Congresswoman Ilhan Omar"
        at "https://daringfireball.net/linked/2026/10/19/obama-omar",
        Some(1792458746), summary 2086 chars;
    manton_json: "manton-json.json" from "https://www.manton.org/feed.json"
        => "Manton Reece", 10 entries,
        first "Yet more views from the train, earlier today going across a bay on the Hudson Ri…"
        at "https://www.manton.org/2026/10/19/yet-more-views-from-the.html",
        Some(1792466795), summary 295 chars;
}

#[test]
fn rss_descriptions_stay_html() {
    // feed-rs labels RSS 1.0 descriptions as text; they carry HTML.
    let feed = parse(
        "slashdot-main.xml",
        "http://rss.slashdot.org/Slashdot/slashdotMain",
    );
    let summary = feed.entries[0].summary_html.as_deref().unwrap();
    assert!(summary.contains("<a href="), "{}", &summary[..200]);
    assert!(!summary.contains("&lt;a href"));
}

fn bad(name: &str) -> FeedError {
    let err = feed::parse(&fixture(&format!("bad/{name}")), "https://example.com/feed")
        .expect_err("bad input must not parse");
    let message = err.to_string();
    assert!(!message.contains('\n'), "{name}: {message:?}");
    assert!(!message.is_empty());
    err
}

#[test]
fn truncated_xml_is_one_line_of_error() {
    assert!(matches!(bad("truncated.xml"), FeedError::NotAFeed(_)));
}

#[test]
fn a_web_page_is_not_a_feed() {
    assert!(matches!(bad("not-a-feed.html"), FeedError::NotAFeed(_)));
}

#[test]
fn an_empty_body_says_so() {
    assert!(matches!(bad("empty.xml"), FeedError::Empty));
}

#[test]
fn broken_json_is_one_line_of_error() {
    assert!(matches!(bad("broken.json"), FeedError::NotAFeed(_)));
}

#[test]
fn relative_links_resolve_against_the_feed_url() {
    let xml = br#"<rss version="2.0"><channel><title>T</title><link>/</link>
        <item><title>A</title><link>/posts/a</link><guid>a</guid></item></channel></rss>"#;
    let feed = feed::parse(xml, "https://blog.example/feed.xml").unwrap();
    assert_eq!(
        feed.entries[0].url.as_deref(),
        Some("https://blog.example/posts/a")
    );
}

#[test]
fn a_second_xml_declaration_is_dropped_not_a_crash() {
    // S7.5: fuzz/feed found quick-xml panicking on this; the refresh thread
    // went with it.
    let whole = feed::parse(&fixture("slashdot-main.xml"), "https://example.com/feed").unwrap();
    let doubled = feed::parse(
        &fixture("slashdot-second-declaration.xml"),
        "https://example.com/feed",
    )
    .unwrap();
    assert_eq!(doubled.entries.len(), whole.entries.len());
    assert_eq!(doubled.title, whole.title);
}
