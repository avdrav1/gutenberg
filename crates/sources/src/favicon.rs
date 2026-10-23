//! Site icons: which URLs a page names as its icon, and whether a download
//! is an icon at all. Pure; fetching and caching belong to the sync engine.

use url::Url;

use crate::discover;
use crate::image::{self, Format};

/// Larger than any favicon; a bigger download isn't one.
pub const MAX_BYTES: usize = 512 * 1024;

/// The icons a page names, best first, absolute, without repeats:
/// `rel="icon"` (and `shortcut icon`) in page order, then
/// `apple-touch-icon`. `data:` URLs and Safari's one-color `mask-icon` are
/// left out.
pub fn icon_links(html: &str, page_url: &str) -> Vec<String> {
    const RELS: [&[&str]; 2] = [
        &["icon"],
        &["apple-touch-icon", "apple-touch-icon-precomposed"],
    ];
    let links = discover::links(html, page_url);
    let mut found = Vec::new();
    for rels in RELS {
        for link in &links {
            let named = rels.iter().any(|rel| discover::has_token(&link.rel, rel));
            if named && matches!(link.href.scheme(), "http" | "https") {
                let url = link.href.to_string();
                if !found.contains(&url) {
                    found.push(url);
                }
            }
        }
    }
    found
}

/// `/favicon.ico` on the root of the page's site.
pub fn root_icon(page_url: &str) -> Option<String> {
    let url = Url::parse(page_url).ok()?.join("/favicon.ico").ok()?;
    Some(url.into())
}

/// The format of an icon download, or `None` when it isn't an image or is
/// too big to be an icon.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    image::sniff(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

    fn fixture(path: &str) -> Vec<u8> {
        std::fs::read(format!("{FIXTURES}/{path}")).unwrap()
    }

    fn page(name: &str) -> String {
        String::from_utf8(fixture(&format!("pages/{name}.html"))).unwrap()
    }

    #[test]
    fn icons_come_before_touch_icons_and_repeats_drop() {
        assert_eq!(
            icon_links(&page("archlinux"), "https://archlinux.org/"),
            [
                "https://archlinux.org/static/archlinux_common_style/favicon.png",
                "https://archlinux.org/static/archlinux_common_style/apple-touch-icon-57x57.png",
                "https://archlinux.org/static/archlinux_common_style/apple-touch-icon-72x72.png",
                "https://archlinux.org/static/archlinux_common_style/apple-touch-icon-114x114.png",
                "https://archlinux.org/static/archlinux_common_style/apple-touch-icon-144x144.png",
            ]
        );
    }

    #[test]
    fn shortcut_icon_wins_over_a_touch_icon_listed_first_and_mask_icon_is_skipped() {
        assert_eq!(
            icon_links(&page("daringfireball"), "https://daringfireball.net/"),
            [
                "https://daringfireball.net/graphics/favicon.ico?v=005",
                "https://daringfireball.net/graphics/apple-touch-icon.png",
            ]
        );
    }

    #[test]
    fn a_data_url_icon_is_no_icon() {
        assert!(icon_links(&page("example"), "https://example.com/").is_empty());
    }

    #[test]
    fn icons_resolve_against_base() {
        let html = r#"<base href="https://cdn.example/s/"><LINK REL="Shortcut Icon" href=fav.ico>"#;
        assert_eq!(
            icon_links(html, "https://site.example/blog/"),
            ["https://cdn.example/s/fav.ico"]
        );
    }

    #[test]
    fn the_root_icon_is_on_the_site_root() {
        assert_eq!(
            root_icon("https://www.example.com/news/world?x=1").as_deref(),
            Some("https://www.example.com/favicon.ico")
        );
        assert_eq!(root_icon("not a url"), None);
    }

    #[test]
    fn an_image_too_big_for_an_icon_is_refused() {
        let mut png = fixture("icons/archlinux.png");
        assert_eq!(sniff(&png), Some(Format::Png));
        png.resize(MAX_BYTES + 1, 0);
        assert_eq!(sniff(&png), None);
    }
}
