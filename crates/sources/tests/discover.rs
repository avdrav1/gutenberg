//! S2.2: the feed links real home pages advertise. Fixtures:
//! `tests/fixtures/pages/` (scripts, styles and body content stripped).

use gutenberg_sources::discover::{feed_links, looks_like_html};

fn page(name: &str) -> String {
    let path = format!(
        "{}/../../tests/fixtures/pages/{name}.html",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn omgubuntu() {
    assert_eq!(
        feed_links(&page("omgubuntu"), "https://www.omgubuntu.co.uk/"),
        ["https://www.omgubuntu.co.uk/feed"]
    );
}

#[test]
fn archlinux_lists_news_before_packages() {
    assert_eq!(
        feed_links(&page("archlinux"), "https://archlinux.org/"),
        [
            "https://archlinux.org/feeds/news/",
            "https://archlinux.org/feeds/packages/"
        ]
    );
}

#[test]
fn rust_blog_with_type_before_rel() {
    assert_eq!(
        feed_links(&page("rust-blog"), "https://blog.rust-lang.org/"),
        ["https://blog.rust-lang.org/feed.xml"]
    );
}

#[test]
fn daringfireball_atom_and_json() {
    assert_eq!(
        feed_links(&page("daringfireball"), "https://daringfireball.net/"),
        [
            "https://daringfireball.net/feeds/main",
            "https://daringfireball.net/feeds/json"
        ]
    );
}

#[test]
fn example_links_no_feed() {
    assert!(feed_links(&page("example"), "https://example.com/").is_empty());
}

#[test]
fn every_page_fixture_is_html() {
    for name in [
        "omgubuntu",
        "archlinux",
        "rust-blog",
        "daringfireball",
        "example",
    ] {
        assert!(looks_like_html(page(name).as_bytes()), "{name}");
    }
}
