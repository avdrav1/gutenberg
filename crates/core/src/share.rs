//! Sharing an article through the browser: a URL that opens a site's
//! composer with the article in it. X, Bluesky, Facebook and LinkedIn are
//! built in; `[[share]]` in `config.toml` adds any site that takes a share
//! URL. Everything here is pure: the UI builds the URL and hands it to
//! `xdg-open`.

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use unicode_segmentation::UnicodeSegmentation;

use crate::config::ShareEntry;

/// Where a share goes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Destination {
    /// `x.com/intent/tweet?text=&url=`; 280 characters, the URL counting
    /// as a 23-character `t.co` link.
    X,
    /// `bsky.app/intent/compose?text=`; 300 grapheme clusters, the URL on
    /// its own line after the comment.
    Bluesky,
    /// `facebook.com/sharer/sharer.php?u=`; the URL alone.
    Facebook,
    /// `linkedin.com/sharing/share-offsite/?url=`; the URL alone
    /// (verified 2026-10-29, see the S9.1 contract).
    LinkedIn,
    /// A site from `[[share]]`: `template` has `{url}`, `{title}` and
    /// `{text}`, each percent-encoded when the URL is built.
    Custom { name: String, template: String },
}

/// The article and what the person wrote about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Share {
    pub title: String,
    pub url: String,
    /// The comment in the compose box; it starts as the title.
    pub text: String,
}

/// Why a `[[share]]` entry can't become a destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryError {
    /// `name` is blank.
    Unnamed,
    /// No `template`, and `name` isn't a built-in's.
    Unknown,
    /// The template has none of `{url}`, `{title}`, `{text}`.
    NoPlaceholder,
}

/// The three placeholders a custom template may use.
const PLACEHOLDERS: [&str; 3] = ["{url}", "{title}", "{text}"];

impl Destination {
    /// The four networks, in the order the picker shows them when
    /// `[[share]]` is empty.
    pub const BUILT_IN: [Destination; 4] = [
        Destination::X,
        Destination::Bluesky,
        Destination::Facebook,
        Destination::LinkedIn,
    ];

    /// X's limit, and what one of its links counts for.
    pub const X_LIMIT: usize = 280;
    pub const X_LINK: usize = 23;
    /// Bluesky's limit, in grapheme clusters.
    pub const BLUESKY_LIMIT: usize = 300;

    /// A built-in by its `config.toml` name: "x", "bluesky", "facebook",
    /// "linkedin". Exact, lowercase, as `arrangement` names are.
    pub fn built_in(name: &str) -> Option<Destination> {
        match name {
            "x" => Some(Destination::X),
            "bluesky" => Some(Destination::Bluesky),
            "facebook" => Some(Destination::Facebook),
            "linkedin" => Some(Destination::LinkedIn),
            _ => None,
        }
    }

    /// As the picker shows it: "X", "Bluesky", "Facebook", "LinkedIn", or
    /// the custom entry's name.
    pub fn name(&self) -> &str {
        match self {
            Destination::X => "X",
            Destination::Bluesky => "Bluesky",
            Destination::Facebook => "Facebook",
            Destination::LinkedIn => "LinkedIn",
            Destination::Custom { name, .. } => name,
        }
    }

    /// Whether the site takes the comment. Facebook and LinkedIn take the
    /// URL alone; a custom template takes it when it has `{text}`.
    pub fn takes_text(&self) -> bool {
        match self {
            Destination::X | Destination::Bluesky => true,
            Destination::Facebook | Destination::LinkedIn => false,
            Destination::Custom { template, .. } => template.contains("{text}"),
        }
    }

    /// The site's limit on a post, in its own units: 280 for X, 300 for
    /// Bluesky, `None` for the rest.
    pub fn limit(&self) -> Option<usize> {
        match self {
            Destination::X => Some(Self::X_LIMIT),
            Destination::Bluesky => Some(Self::BLUESKY_LIMIT),
            Destination::Facebook | Destination::LinkedIn | Destination::Custom { .. } => None,
        }
    }

    /// How much of the limit `share` uses, counted as the site counts.
    /// `None` where there's no limit.
    pub fn count(&self, share: &Share) -> Option<usize> {
        match self {
            // X puts a space between the text and the link.
            Destination::X if share.text.is_empty() => Some(Self::X_LINK),
            Destination::X => Some(x_length(&share.text) + 1 + Self::X_LINK),
            Destination::Bluesky => Some(bluesky_post(share).graphemes(true).count()),
            Destination::Facebook | Destination::LinkedIn | Destination::Custom { .. } => None,
        }
    }

    /// Characters left under the limit; negative when over. `None` where
    /// there's no limit.
    pub fn remaining(&self, share: &Share) -> Option<i64> {
        let limit = self.limit()?;
        let count = self.count(share)?;
        Some(limit as i64 - count as i64)
    }

    /// The URL that opens the site's composer with `share`.
    pub fn url(&self, share: &Share) -> String {
        match self {
            Destination::X if share.text.is_empty() => {
                format!("https://x.com/intent/tweet?url={}", encode(&share.url))
            }
            Destination::X => format!(
                "https://x.com/intent/tweet?text={}&url={}",
                encode(&share.text),
                encode(&share.url)
            ),
            Destination::Bluesky => format!(
                "https://bsky.app/intent/compose?text={}",
                encode(&bluesky_post(share))
            ),
            Destination::Facebook => format!(
                "https://www.facebook.com/sharer/sharer.php?u={}",
                encode(&share.url)
            ),
            Destination::LinkedIn => format!(
                "https://www.linkedin.com/sharing/share-offsite/?url={}",
                encode(&share.url)
            ),
            Destination::Custom { template, .. } => template
                .replace("{url}", &encode(&share.url))
                .replace("{title}", &encode(&share.title))
                .replace("{text}", &encode(&share.text)),
        }
    }

    /// What the compose box says under the field for this site, or `None`
    /// when there's nothing to say beyond the keys.
    pub fn note(&self) -> Option<&'static str> {
        match self {
            Destination::X | Destination::Custom { .. } => None,
            Destination::Bluesky => {
                Some("Sign in to Bluesky first: its composer drops the text otherwise.")
            }
            Destination::Facebook => Some("Facebook takes the link alone."),
            Destination::LinkedIn => Some("LinkedIn takes the link alone."),
        }
    }
}

/// The comment, then the URL on its own line; the URL alone when there is
/// no comment. The composer's separator handling can break the link card
/// when they share a line.
fn bluesky_post(share: &Share) -> String {
    if share.text.is_empty() {
        share.url.clone()
    } else {
        format!("{}\n{}", share.text, share.url)
    }
}

impl ShareEntry {
    /// The destination this entry names, or why it can't.
    pub fn destination(&self) -> Result<Destination, EntryError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(EntryError::Unnamed);
        }
        match &self.template {
            None => Destination::built_in(name).ok_or(EntryError::Unknown),
            Some(template) if PLACEHOLDERS.iter().any(|p| template.contains(p)) => {
                Ok(Destination::Custom {
                    name: self.name.clone(),
                    template: template.clone(),
                })
            }
            Some(_) => Err(EntryError::NoPlaceholder),
        }
    }
}

/// The picker's rows: `entries` in file order, or [`Destination::BUILT_IN`]
/// when there are none. Entries that fail [`ShareEntry::destination`] are
/// skipped; `Config::parse` has already rejected them.
pub fn destinations(entries: &[ShareEntry]) -> Vec<Destination> {
    if entries.is_empty() {
        return Destination::BUILT_IN.to_vec();
    }
    entries
        .iter()
        .filter_map(|entry| entry.destination().ok())
        .collect()
}

/// Everything but RFC 3986's unreserved characters.
const ESCAPED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// `value` percent-encoded for a URL: RFC 3986 unreserved characters
/// (letters, digits, `-` `.` `_` `~`) pass, everything else is `%XX`, a
/// space `%20`. Used for every parameter and placeholder here.
pub fn encode(value: &str) -> String {
    utf8_percent_encode(value, ESCAPED).to_string()
}

/// X's count of `text`: each whitespace-separated run that starts with
/// `http://` or `https://` counts [`Destination::X_LINK`], every other
/// character one.
pub fn x_length(text: &str) -> usize {
    let mut count = 0;
    let mut rest = text;
    while !rest.is_empty() {
        let (run, after) = rest
            .find(char::is_whitespace)
            .map_or((rest, ""), |at| rest.split_at(at));
        count += if run.starts_with("http://") || run.starts_with("https://") {
            Destination::X_LINK
        } else {
            run.chars().count()
        };
        // The whitespace character itself counts one.
        let mut after = after.chars();
        if after.next().is_some() {
            count += 1;
        }
        rest = after.as_str();
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://example.com/a/very/long/path?x=1&y=2";

    fn share(text: &str) -> Share {
        Share {
            title: "Title".into(),
            url: URL.into(),
            text: text.into(),
        }
    }

    #[test]
    fn encode_leaves_unreserved_and_escapes_the_rest() {
        assert_eq!(encode("a b&c/d?e=f#g~"), "a%20b%26c%2Fd%3Fe%3Df%23g~");
        assert_eq!(encode("A-z.0_9~"), "A-z.0_9~");
        assert_eq!(encode("Ünïcode é"), "%C3%9Cn%C3%AFcode%20%C3%A9");
        assert_eq!(encode(""), "");
    }

    #[test]
    fn x_counts_links_as_23() {
        assert_eq!(
            x_length("read https://example.com/a/very/long/path now"),
            4 + 1 + 23 + 1 + 3
        );
        assert_eq!(x_length("no link here"), 12);
        assert_eq!(x_length("Ünïcode é"), 9);
        assert_eq!(x_length("http://a.io https://b.io"), 23 + 1 + 23);
        assert_eq!(x_length("httpsnot a link"), 15);
        assert_eq!(x_length(""), 0);
    }

    #[test]
    fn x_counts_the_post_and_builds_the_intent() {
        let title: String = "a".repeat(100);
        let s = Share {
            title: title.clone(),
            url: URL.into(),
            text: title,
        };
        assert_eq!(Destination::X.limit(), Some(280));
        assert_eq!(Destination::X.count(&s), Some(124));
        assert_eq!(Destination::X.remaining(&s), Some(156));
        let url = Destination::X.url(&s);
        assert!(url.starts_with("https://x.com/intent/tweet?text="), "{url}");
        assert!(url.contains("&url=https%3A%2F%2F"), "{url}");

        let empty = share("");
        assert_eq!(Destination::X.count(&empty), Some(23));
        assert_eq!(
            Destination::X.url(&empty),
            "https://x.com/intent/tweet?url=https%3A%2F%2Fexample.com%2Fa%2Fvery%2Flong%2Fpath%3Fx%3D1%26y%3D2"
        );

        let full = share("Hello world & #tags");
        assert_eq!(
            Destination::X.url(&full),
            "https://x.com/intent/tweet?text=Hello%20world%20%26%20%23tags&url=https%3A%2F%2Fexample.com%2Fa%2Fvery%2Flong%2Fpath%3Fx%3D1%26y%3D2"
        );
        // 19 characters of text, a space, the link.
        assert_eq!(Destination::X.count(&full), Some(19 + 1 + 23));

        // A link typed into the comment counts 23 too.
        let linked = share("see https://other.example.org/some/long/thing");
        assert_eq!(Destination::X.count(&linked), Some(4 + 23 + 1 + 23));

        // Over the limit goes negative.
        let long = share(&"b".repeat(280));
        assert_eq!(Destination::X.remaining(&long), Some(-24));
        assert!(Destination::X.takes_text());
        assert_eq!(Destination::X.note(), None);
    }

    #[test]
    fn bluesky_counts_graphemes_with_the_url_on_its_own_line() {
        // A family emoji is several scalars but one grapheme; a thumbs-up
        // with a skin tone is two scalars and one grapheme.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let thumbs = "\u{1F44D}\u{1F3FD}";
        let s = Share {
            title: "Title".into(),
            url: "https://a.io".into(),
            text: format!("{family} {thumbs}"),
        };
        // family, space, thumbs, newline, 12 of URL.
        assert_eq!(Destination::Bluesky.count(&s), Some(1 + 1 + 1 + 1 + 12));
        assert_eq!(Destination::Bluesky.limit(), Some(300));
        assert_eq!(Destination::Bluesky.remaining(&s), Some(300 - 16));
        assert!(Destination::Bluesky.takes_text());

        let url = Destination::Bluesky.url(&share("Hi there"));
        assert_eq!(
            url,
            "https://bsky.app/intent/compose?text=Hi%20there%0Ahttps%3A%2F%2Fexample.com%2Fa%2Fvery%2Flong%2Fpath%3Fx%3D1%26y%3D2"
        );
        assert!(url.contains("%0A"), "{url}");

        // No comment: the URL alone, no newline.
        assert_eq!(
            Destination::Bluesky.url(&share("")),
            "https://bsky.app/intent/compose?text=https%3A%2F%2Fexample.com%2Fa%2Fvery%2Flong%2Fpath%3Fx%3D1%26y%3D2"
        );
        let bare = Share {
            url: "https://a.io".into(),
            ..Share::default()
        };
        assert_eq!(Destination::Bluesky.count(&bare), Some(12));

        // One over the limit: 300 - (12 for the URL + 1 newline) = 287
        // characters of text fit exactly; 288 is one over.
        let fits = Share {
            text: "c".repeat(287),
            ..bare.clone()
        };
        assert_eq!(Destination::Bluesky.remaining(&fits), Some(0));
        let over = Share {
            text: "c".repeat(288),
            ..bare
        };
        assert_eq!(Destination::Bluesky.remaining(&over), Some(-1));
        assert_eq!(
            Destination::Bluesky.note(),
            Some("Sign in to Bluesky first: its composer drops the text otherwise.")
        );
    }

    #[test]
    fn facebook_and_linkedin_take_the_url_alone() {
        let s = share("My secret comment");
        for site in [Destination::Facebook, Destination::LinkedIn] {
            assert!(!site.takes_text(), "{site:?}");
            assert_eq!(site.limit(), None);
            assert_eq!(site.count(&s), None);
            assert_eq!(site.remaining(&s), None);
            let url = site.url(&s);
            assert!(!url.contains("secret"), "{url}");
            assert!(!url.contains("Title"), "{url}");
        }
        assert_eq!(
            Destination::Facebook.url(&s),
            "https://www.facebook.com/sharer/sharer.php?u=https%3A%2F%2Fexample.com%2Fa%2Fvery%2Flong%2Fpath%3Fx%3D1%26y%3D2"
        );
        assert_eq!(
            Destination::LinkedIn.url(&s),
            "https://www.linkedin.com/sharing/share-offsite/?url=https%3A%2F%2Fexample.com%2Fa%2Fvery%2Flong%2Fpath%3Fx%3D1%26y%3D2"
        );
        assert_eq!(
            Destination::Facebook.note(),
            Some("Facebook takes the link alone.")
        );
        assert_eq!(
            Destination::LinkedIn.note(),
            Some("LinkedIn takes the link alone.")
        );
        assert_eq!(Destination::Facebook.name(), "Facebook");
        assert_eq!(Destination::LinkedIn.name(), "LinkedIn");
    }

    #[test]
    fn a_custom_template_fills_each_placeholder_encoded() {
        let mastodon = Destination::Custom {
            name: "Mastodon".into(),
            template: "https://mastodon.social/share?text={title}%20{url}".into(),
        };
        let s = Share {
            title: "Title here & more".into(),
            url: "https://e.com/p?a=1#x".into(),
            text: "ignored".into(),
        };
        assert_eq!(
            mastodon.url(&s),
            "https://mastodon.social/share?text=Title%20here%20%26%20more%20https%3A%2F%2Fe.com%2Fp%3Fa%3D1%23x"
        );
        assert_eq!(
            mastodon.url(&share("x")),
            "https://mastodon.social/share?text=Title%20https%3A%2F%2Fexample.com%2Fa%2Fvery%2Flong%2Fpath%3Fx%3D1%26y%3D2"
        );
        assert!(!mastodon.takes_text());
        assert_eq!(mastodon.limit(), None);
        assert_eq!(mastodon.count(&s), None);
        assert_eq!(mastodon.remaining(&s), None);
        assert_eq!(mastodon.note(), None);
        assert_eq!(mastodon.name(), "Mastodon");

        let pinboard = Destination::Custom {
            name: "Pinboard".into(),
            template: "https://pinboard.in/add?url={url}&title={title}&description={text}".into(),
        };
        assert!(pinboard.takes_text());
        assert_eq!(pinboard.note(), None);
        assert_eq!(
            pinboard.url(&share("a comment")),
            "https://pinboard.in/add?url=https%3A%2F%2Fexample.com%2Fa%2Fvery%2Flong%2Fpath%3Fx%3D1%26y%3D2&title=Title&description=a%20comment"
        );

        let twice = Destination::Custom {
            name: "Twice".into(),
            template: "https://t.io/?u={url}&again={url}&{nope}".into(),
        };
        assert_eq!(
            twice.url(&Share {
                url: "https://a.io".into(),
                ..Share::default()
            }),
            "https://t.io/?u=https%3A%2F%2Fa.io&again=https%3A%2F%2Fa.io&{nope}"
        );
    }

    fn entry(name: &str, template: Option<&str>) -> ShareEntry {
        ShareEntry {
            name: name.into(),
            template: template.map(Into::into),
        }
    }

    #[test]
    fn entries_become_destinations_in_order_or_the_four_networks() {
        assert_eq!(destinations(&[]), Destination::BUILT_IN.to_vec());
        assert_eq!(
            Destination::BUILT_IN
                .iter()
                .map(Destination::name)
                .collect::<Vec<_>>(),
            ["X", "Bluesky", "Facebook", "LinkedIn"]
        );
        let entries = [
            entry("linkedin", None),
            entry("Mastodon", Some("https://m.io/share?text={title}")),
            entry("x", None),
        ];
        assert_eq!(
            destinations(&entries),
            [
                Destination::LinkedIn,
                Destination::Custom {
                    name: "Mastodon".into(),
                    template: "https://m.io/share?text={title}".into(),
                },
                Destination::X,
            ]
        );
        // One entry means a one-row picker.
        assert_eq!(
            destinations(&[entry("bluesky", None)]),
            [Destination::Bluesky]
        );
        // Bad entries are skipped; Config::parse rejects them first.
        assert_eq!(destinations(&[entry("nope", None)]), []);

        for name in ["x", "bluesky", "facebook", "linkedin"] {
            assert!(Destination::built_in(name).is_some(), "{name}");
        }
        assert_eq!(Destination::built_in("X"), None);
        assert_eq!(Destination::built_in("Bluesky"), None);
        assert_eq!(Destination::built_in("twitter"), None);
    }

    #[test]
    fn an_entry_needs_a_name_and_a_known_name_or_a_placeholder() {
        assert_eq!(entry("", None).destination(), Err(EntryError::Unnamed));
        assert_eq!(
            entry("  ", Some("https://a.io/{url}")).destination(),
            Err(EntryError::Unnamed)
        );
        assert_eq!(
            entry("mastodon", None).destination(),
            Err(EntryError::Unknown)
        );
        assert_eq!(
            entry("Mastodon", Some("https://a.io/share")).destination(),
            Err(EntryError::NoPlaceholder)
        );
        assert_eq!(
            entry("facebook", None).destination(),
            Ok(Destination::Facebook)
        );
        // A custom entry may wear a built-in's name; the picker shows it
        // as written.
        assert_eq!(
            entry("x", Some("https://x.example/{text}")).destination(),
            Ok(Destination::Custom {
                name: "x".into(),
                template: "https://x.example/{text}".into(),
            })
        );
    }
}
