//! OPML subscription lists in and out. Fixture: `tests/fixtures/opml/`.

use gutenberg_sources::opml::{self, Folder, Opml, OpmlError, Subscription};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/../../tests/fixtures/opml/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn feed(title: &str, url: &str) -> Subscription {
    Subscription {
        title: title.to_owned(),
        url: url.to_owned(),
    }
}

fn error(input: &str) -> String {
    match opml::parse(input.as_bytes()) {
        Err(OpmlError::NotOpml(why)) => why,
        Ok(parsed) => panic!("parsed: {parsed:?}"),
    }
}

#[test]
fn feedly_export() {
    let parsed = opml::parse(&fixture("feedly.opml")).unwrap();
    let names: Vec<&str> = parsed.folders.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Linux",
            "Programming",
            "Tech & Culture",
            "News",
            "Watching",
            "Saved for later"
        ]
    );
    let sizes: Vec<usize> = parsed.folders.iter().map(|f| f.feeds.len()).collect();
    assert_eq!(sizes, [6, 4, 5, 4, 2, 0]);
    assert!(parsed.feeds.is_empty());
    assert_eq!(
        parsed.folders[0].feeds[0],
        feed(
            "Arch Linux: Recent news updates",
            "https://archlinux.org/feeds/news/"
        )
    );
    // Entities in attributes come back as text.
    assert_eq!(
        parsed.folders[1].feeds[2].url,
        "https://hnrss.org/frontpage?points=100&comments=25"
    );
    assert_eq!(parsed.folders[3].feeds[2].title, "NYT > Top Stories");
}

#[test]
fn nested_folders_flatten_and_loose_feeds_stay_unfiled() {
    let parsed = opml::parse(
        br#"<?xml version="1.0"?>
        <opml version="2.0"><head><title>x</title></head><body>
          <outline type="rss" text="Loose" xmlUrl="https://loose.example/feed"/>
          <outline text="Outer">
            <outline text="A" xmlUrl="https://a.example/feed"/>
            <outline title="Inner" text="ignored">
              <outline xmlUrl="https://b.example/feed"/>
            </outline>
            <outline text="C" xmlurl="https://c.example/feed"/>
          </outline>
          <outline text="Empty"/>
        </body></opml>"#,
    )
    .unwrap();
    assert_eq!(
        parsed,
        Opml {
            folders: vec![
                Folder {
                    name: "Outer".to_owned(),
                    feeds: vec![
                        feed("A", "https://a.example/feed"),
                        feed("C", "https://c.example/feed"),
                    ],
                },
                Folder {
                    name: "Inner".to_owned(),
                    // No title or text: the URL stands in.
                    feeds: vec![feed("https://b.example/feed", "https://b.example/feed")],
                },
                Folder {
                    name: "Empty".to_owned(),
                    feeds: vec![],
                },
            ],
            feeds: vec![feed("Loose", "https://loose.example/feed")],
        }
    );
}

#[test]
fn outlines_outside_the_body_are_not_feeds() {
    let parsed = opml::parse(
        br#"<opml><head><outline xmlUrl="https://head.example/feed"/></head><body/></opml>"#,
    )
    .unwrap();
    assert_eq!(parsed, Opml::default());
}

#[test]
fn bad_input_is_one_line_of_error() {
    let truncated = String::from_utf8(fixture("feedly.opml")).unwrap();
    let truncated = &truncated[..truncated.len() / 2];
    for input in [
        truncated,
        "",
        "<html><body>a page</body></html>",
        "<opml><body><outline></body></opml>",
    ] {
        let why = error(input);
        assert!(!why.is_empty() && !why.contains('\n'), "{input:?}: {why:?}");
    }
    assert_eq!(error("<rss/>"), "no <opml> element");
    // Cut between elements rather than inside a tag.
    let at_line = &truncated[..=truncated.rfind('\n').unwrap()];
    assert_eq!(error(at_line), "file ends early");
}

#[test]
fn written_lists_read_back_the_same() {
    let list = Opml {
        folders: vec![
            Folder {
                name: "Tech & \"Culture\" <3".to_owned(),
                feeds: vec![feed("Q&A's", "https://q.example/rss?a=1&b=2")],
            },
            Folder {
                name: "Empty".to_owned(),
                feeds: vec![],
            },
        ],
        feeds: vec![feed("Loose", "https://loose.example/feed")],
    };
    let written = opml::write(&list);
    assert_eq!(opml::parse(written.as_bytes()).unwrap(), list);

    let feedly = opml::parse(&fixture("feedly.opml")).unwrap();
    assert_eq!(
        opml::parse(opml::write(&feedly).as_bytes()).unwrap(),
        feedly
    );
}
