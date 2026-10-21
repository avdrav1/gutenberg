//! OPML subscription lists, the format Feedly, Inoreader and most readers
//! import and export: folders are outlines without an `xmlUrl`, feeds are
//! outlines with one.

use std::fmt::Write as _;

use quick_xml::escape::escape;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

/// A subscription list. Folders are flat: one nested in another comes
/// right after it, holding its own feeds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Opml {
    /// In file order, empty ones included.
    pub folders: Vec<Folder>,
    /// Feeds in no folder.
    pub feeds: Vec<Subscription>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub name: String,
    pub feeds: Vec<Subscription>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    pub title: String,
    /// The feed's own URL (`xmlUrl`).
    pub url: String,
}

/// Displays as one line.
#[derive(Debug, thiserror::Error)]
pub enum OpmlError {
    #[error("not OPML ({0})")]
    NotOpml(String),
}

/// Reads a subscription list. Feeds are kept as the file lists them; a feed
/// filed in two folders appears in both.
pub fn parse(bytes: &[u8]) -> Result<Opml, OpmlError> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let mut reader = Reader::from_reader(bytes);
    let mut opml = Opml::default();
    // One entry per open element: the folder it opened, if it is one.
    let mut open: Vec<Option<usize>> = Vec::new();
    let mut root_seen = false;
    let mut in_body = false;
    let not_opml = |e: quick_xml::Error| OpmlError::NotOpml(one_line(&e.to_string()));
    loop {
        let (element, has_children) = match reader.read_event().map_err(not_opml)? {
            Event::Start(e) => (e, true),
            Event::Empty(e) => (e, false),
            Event::End(e) => {
                if e.local_name().as_ref() == "body" {
                    in_body = false;
                }
                open.pop();
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let name = element.local_name();
        if !root_seen {
            if name.as_ref() != "opml" {
                return Err(OpmlError::NotOpml("no <opml> element".to_owned()));
            }
            root_seen = true;
        }
        let mut folder = None;
        match name.as_ref() {
            "body" => in_body = has_children,
            "outline" if in_body => folder = outline(&element, &open, &mut opml),
            _ => {}
        }
        if has_children {
            open.push(folder);
        }
    }
    if !root_seen {
        return Err(OpmlError::NotOpml("no <opml> element".to_owned()));
    }
    if !open.is_empty() {
        return Err(OpmlError::NotOpml("file ends early".to_owned()));
    }
    Ok(opml)
}

/// Files one outline as a feed or a folder; returns the new folder's index.
fn outline(element: &BytesStart, open: &[Option<usize>], opml: &mut Opml) -> Option<usize> {
    let (mut url, mut title, mut text) = (None, None, None);
    for attr in element.attributes().flatten() {
        let slot = match attr.key.local_name().as_ref() {
            k if k.eq_ignore_ascii_case("xmlUrl") => &mut url,
            "title" => &mut title,
            "text" => &mut text,
            _ => continue,
        };
        // A stray `&` shouldn't cost the user a feed: keep the raw text.
        let value = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .unwrap_or_else(|_| attr.value.clone());
        let value = value.trim();
        if !value.is_empty() {
            *slot = Some(value.to_owned());
        }
    }
    let name = title.or(text);
    match url {
        Some(url) => {
            let feed = Subscription {
                title: name.unwrap_or_else(|| url.clone()),
                url,
            };
            match open.iter().rev().find_map(|f| *f) {
                Some(i) => opml.folders[i].feeds.push(feed),
                None => opml.feeds.push(feed),
            }
            None
        }
        None => {
            opml.folders.push(Folder {
                name: name.unwrap_or_else(|| "Untitled".to_owned()),
                feeds: Vec::new(),
            });
            Some(opml.folders.len() - 1)
        }
    }
}

/// Writes OPML 2.0: folders and their feeds, then feeds in no folder.
pub fn write(opml: &Opml) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <opml version=\"2.0\">\n  \
         <head>\n    <title>Gutenberg subscriptions</title>\n  </head>\n  \
         <body>\n",
    );
    for folder in &opml.folders {
        let name = escape(folder.name.as_str());
        if folder.feeds.is_empty() {
            let _ = writeln!(out, "    <outline text=\"{name}\" title=\"{name}\"/>");
            continue;
        }
        let _ = writeln!(out, "    <outline text=\"{name}\" title=\"{name}\">");
        for feed in &folder.feeds {
            write_feed(&mut out, "      ", feed);
        }
        out.push_str("    </outline>\n");
    }
    for feed in &opml.feeds {
        write_feed(&mut out, "    ", feed);
    }
    out.push_str("  </body>\n</opml>\n");
    out
}

fn write_feed(out: &mut String, indent: &str, feed: &Subscription) {
    let title = escape(feed.title.as_str());
    let url = escape(feed.url.as_str());
    let _ = writeln!(
        out,
        "{indent}<outline type=\"rss\" text=\"{title}\" title=\"{title}\" xmlUrl=\"{url}\"/>"
    );
}

fn one_line(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}
