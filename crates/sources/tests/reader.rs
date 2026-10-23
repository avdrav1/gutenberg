//! S3.3 acceptance: the reader view works on 9 of 10 outlets in
//! `tests/reader_sites.txt`. Each page is a capture in
//! `tests/fixtures/pages/reader/`. "Works" means the extracted text holds
//! the article's first and last paragraphs, with at most a byline's worth
//! of anything else before or after them. The paragraphs were picked by hand
//! from each page's markup, meta description and JSON-LD, not taken from the
//! extractor.

use gutenberg_sources::reader;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests");

/// Room for a headline, deck, byline or "N comments" around the article.
const SLACK: usize = 1000;

/// slug, then a phrase from the article's first paragraph and one from its
/// last.
const ARTICLES: &[(&str, &str, &str)] = &[
    (
        "bbc",
        "A BBC investigation has found evidence that Sudanese paramilitary fighters",
        "attacks on healthcare continue as part of a wider and devastating humanitarian crisis",
    ),
    (
        "npr",
        "There's no avoiding the word rugged with Tom Skerritt",
        "a program for veterans to take storytelling courses and share their experiences",
    ),
    (
        "guardian",
        "Lewis Hamilton has said he hopes to remain racing in Formula One",
        "Russell finishes ninth or lower and Hamilton is fifth or lower",
    ),
    (
        "propublica",
        "Lonnie Rouse shuffled into a wood-paneled conference room",
        "Less than three months later, Mata died.",
    ),
    (
        "arstechnica",
        "A security researcher has demonstrated a software-only partial jailbreak",
        "trying to keep them out of their console’s deepest recesses",
    ),
    (
        "theverge",
        "Etsy is announcing stricter rules about how AI can be used in listings",
        "a seller must disclose that it was made with AI, Etsy says",
    ),
    (
        "phoronix",
        "Adding to all the exciting performance optimizations",
        "merged today for Linux 7.4",
    ),
    (
        // A photo essay: its last caption sits under a video, which the
        // extractor drops along with the caption. The miss of the ten.
        "quanta",
        "On a frigid February morning in Edinburgh, Scotland, Patrick Hickey",
        "This video, by Andrew Moore of the Howard Hughes Medical Institute",
    ),
    (
        "pbs",
        "Calley Means, a top adviser to Health Secretary Robert F. Kennedy Jr.",
        "fighting for our children to have healthy food in schools",
    ),
    (
        "aljazeera",
        "At Karachi Zoo, Jude Allen sang, You Are My Sunshine",
        "She will literally throw rocks if she can at anyone talking to me",
    ),
];

/// The outlets in `tests/reader_sites.txt`: slug and page URL.
fn sites() -> Vec<(String, String)> {
    std::fs::read_to_string(format!("{ROOT}/reader_sites.txt"))
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (slug, url) = l.split_once(' ').unwrap();
            (slug.to_owned(), url.to_owned())
        })
        .collect()
}

/// Why the article didn't come out whole, or `None` when it did.
fn miss(slug: &str, url: &str) -> Option<String> {
    let (_, first, last) = ARTICLES
        .iter()
        .find(|(s, ..)| *s == slug)
        .unwrap_or_else(|| panic!("{slug} has no expected article"));
    let html =
        std::fs::read_to_string(format!("{ROOT}/fixtures/pages/reader/{slug}.html")).unwrap();
    let text = match reader::extract(&html, url) {
        Ok(extracted) => extracted.text,
        Err(err) => return Some(err.to_string()),
    };
    let Some(start) = text.find(first) else {
        return Some("first paragraph missing".into());
    };
    let Some(end) = text.rfind(last).map(|i| i + last.len()) else {
        return Some("last paragraph missing".into());
    };
    if start > SLACK {
        return Some(format!("{start} bytes before the article"));
    }
    if text.len() - end > SLACK {
        return Some(format!("{} bytes after the article", text.len() - end));
    }
    None
}

#[test]
fn nine_of_ten_outlets_read() {
    let sites = sites();
    assert_eq!(sites.len(), 10);
    let misses: Vec<(String, String)> = sites
        .iter()
        .filter_map(|(slug, url)| miss(slug, url).map(|why| (slug.clone(), why)))
        .collect();
    // Quanta is the known miss; anything else is a regression.
    assert_eq!(
        misses,
        [("quanta".to_owned(), "last paragraph missing".to_owned())]
    );
}

#[test]
fn a_page_with_no_article_says_so() {
    let html = "<html><head><title>Sign in</title></head><body><form><input name=user></form></body></html>";
    let err = reader::extract(html, "https://site.example/login").unwrap_err();
    assert_eq!(err.to_string(), "no article found on the page");
}
