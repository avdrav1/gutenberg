//! The share picker and compose box: state and keys. Pure, so it's tested
//! without a window; `lib.rs` draws them and opens the browser.

use gutenberg_core::share::{Destination, Share};

use crate::palette::Input;

/// The destinations, one row each, in config order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    pub destinations: Vec<Destination>,
    /// Each row's letter, `None` where the name has no free one.
    pub letters: Vec<Option<char>>,
    pub cursor: usize,
}

impl Picker {
    pub fn new(destinations: Vec<Destination>) -> Self {
        let letters = letters(&destinations);
        Self {
            destinations,
            letters,
            cursor: 0,
        }
    }

    /// Applies a key. Returns the destination picked by Enter or a row's
    /// letter; `Close` and a pick both mean the caller closes the picker.
    /// `Type("j")`/`Down` and `Type("k")`/`Up` move.
    pub fn apply(&mut self, input: Input) -> Option<Destination> {
        match input {
            Input::Type(text) => match text.as_str() {
                "j" => self.down(),
                "k" => self.up(),
                _ => {
                    let mut chars = text.chars();
                    let letter = chars.next().filter(|_| chars.next().is_none())?;
                    let letter = letter.to_ascii_lowercase();
                    let at = self.letters.iter().position(|l| *l == Some(letter))?;
                    return self.destinations.get(at).cloned();
                }
            },
            Input::Down => self.down(),
            Input::Up => self.up(),
            Input::Run => return self.destinations.get(self.cursor).cloned(),
            Input::Backspace | Input::Close => {}
        }
        None
    }

    fn down(&mut self) {
        self.cursor = (self.cursor + 1).min(self.destinations.len().saturating_sub(1));
    }

    fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }
}

/// Each row's letter: the first ASCII letter of its name, lowercased,
/// that no earlier row took and isn't `j` or `k`; then the next letter of
/// the name; else `None`. X, Bluesky, Facebook, LinkedIn give x b f l.
pub fn letters(destinations: &[Destination]) -> Vec<Option<char>> {
    let mut taken = vec!['j', 'k'];
    destinations
        .iter()
        .map(|destination| {
            let letter = destination
                .name()
                .chars()
                .filter(char::is_ascii_alphabetic)
                .map(|c| c.to_ascii_lowercase())
                .find(|c| !taken.contains(c));
            if let Some(c) = letter {
                taken.push(c);
            }
            letter
        })
        .collect()
}

/// What Enter or `y` did in the compose box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Done {
    /// Open this URL in the browser.
    Open(String),
    /// Put this text on the clipboard.
    Copy(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composer {
    pub destination: Destination,
    pub share: Share,
    /// `y` copied the comment; the hint says so.
    pub copied: bool,
}

impl Composer {
    /// The text starts as the title.
    pub fn new(destination: Destination, title: String, url: String) -> Self {
        Self {
            destination,
            share: Share {
                text: title.clone(),
                title,
                url,
            },
            copied: false,
        }
    }

    /// Applies a key. `Type` and `Backspace` edit the text where the site
    /// takes it; where it doesn't, `Type("y")` copies the text and other
    /// typing does nothing. `Run` opens the site's URL. `Up`, `Down` and
    /// `Close` do nothing here; the caller closes on `Close`.
    pub fn apply(&mut self, input: Input) -> Option<Done> {
        match input {
            Input::Type(text) if self.takes_text() => self.share.text.push_str(&text),
            Input::Type(text) if text == "y" => {
                self.copied = true;
                return Some(Done::Copy(self.share.text.clone()));
            }
            Input::Type(_) => {}
            Input::Backspace => {
                if self.takes_text() {
                    self.share.text.pop();
                }
            }
            Input::Run => return Some(Done::Open(self.url())),
            Input::Up | Input::Down | Input::Close => {}
        }
        None
    }

    pub fn takes_text(&self) -> bool {
        self.destination.takes_text()
    }

    pub fn remaining(&self) -> Option<i64> {
        self.destination.remaining(&self.share)
    }

    /// The URL Enter opens: the site's composer with the share in it.
    pub fn url(&self) -> String {
        self.destination.url(&self.share)
    }

    /// The line under the field: the site's note, then the keys.
    pub fn hint(&self) -> String {
        let name = self.destination.name();
        if self.takes_text() {
            return match self.destination.note() {
                Some(note) => format!("{note} Enter opens it · Esc cancels"),
                None => format!("Enter opens {name} in the browser · Esc cancels"),
            };
        }
        if self.copied {
            return format!("Comment copied. Enter opens {name} · Esc cancels");
        }
        match self.destination.note() {
            Some(note) => format!("{note} y copies your comment · Enter opens it · Esc cancels"),
            None => {
                format!("{name} takes the link alone. y copies your comment · Enter opens it · Esc cancels")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(name: &str, template: &str) -> Destination {
        Destination::Custom {
            name: name.into(),
            template: template.into(),
        }
    }

    fn typed(input: &str) -> Input {
        Input::Type(input.into())
    }

    #[test]
    fn letters_come_from_the_names_and_skip_j_k_and_repeats() {
        assert_eq!(
            letters(&Destination::BUILT_IN),
            [Some('x'), Some('b'), Some('f'), Some('l')]
        );
        let sites = [
            custom("Journal", "{url}"),
            custom("Kiosk", "{url}"),
            Destination::Bluesky,
            custom("Bluesky", "{url}"),
        ];
        assert_eq!(
            letters(&sites),
            [Some('o'), Some('i'), Some('b'), Some('l')]
        );
        assert_eq!(letters(&[custom("jk", "{url}")]), [None]);
    }

    #[test]
    fn the_picker_moves_picks_and_closes() {
        let mut picker = Picker::new(Destination::BUILT_IN.to_vec());
        assert_eq!(picker.apply(Input::Up), None);
        assert_eq!(picker.cursor, 0);
        assert_eq!(picker.apply(typed("j")), None);
        assert_eq!(picker.apply(Input::Down), None);
        assert_eq!(picker.cursor, 2);
        for _ in 0..5 {
            picker.apply(typed("j"));
        }
        assert_eq!(picker.cursor, 3);
        assert_eq!(picker.apply(typed("k")), None);
        assert_eq!(picker.cursor, 2);
        assert_eq!(picker.apply(Input::Run), Some(Destination::Facebook));
        picker.cursor = 0;
        assert_eq!(picker.apply(typed("f")), Some(Destination::Facebook));
        assert_eq!(picker.apply(typed("L")), Some(Destination::LinkedIn));
        assert_eq!(picker.apply(typed("q")), None);
        assert_eq!(picker.apply(Input::Close), None);
        assert_eq!(picker.apply(Input::Backspace), None);
        assert_eq!(Picker::new(Vec::new()).apply(Input::Run), None);
    }

    #[test]
    fn the_composer_starts_with_the_title_and_edits_it() {
        let mut composer = Composer::new(
            Destination::X,
            "A title".into(),
            "https://site.example/a".into(),
        );
        assert_eq!(composer.share.text, "A title");
        assert_eq!(composer.apply(typed(" ok")), None);
        assert_eq!(composer.apply(Input::Backspace), None);
        assert_eq!(composer.share.text, "A title o");
        assert_eq!(composer.apply(Input::Up), None);
        assert_eq!(composer.apply(Input::Close), None);
        let url =
            "https://x.com/intent/tweet?text=A%20title%20o&url=https%3A%2F%2Fsite.example%2Fa";
        assert_eq!(composer.url(), url);
        assert_eq!(composer.apply(Input::Run), Some(Done::Open(url.into())));
        assert_eq!(
            composer.hint(),
            "Enter opens X in the browser · Esc cancels"
        );
        let bluesky = Composer::new(Destination::Bluesky, "T".into(), "https://e.example".into());
        assert_eq!(
            bluesky.hint(),
            "Sign in to Bluesky first: its composer drops the text otherwise. Enter opens it · Esc cancels"
        );
    }

    #[test]
    fn a_url_only_site_greys_the_text_and_y_copies_it() {
        let mut composer = Composer::new(
            Destination::Facebook,
            "A title".into(),
            "https://site.example/a".into(),
        );
        assert!(!composer.takes_text());
        assert_eq!(composer.apply(typed("a")), None);
        assert_eq!(composer.apply(Input::Backspace), None);
        assert_eq!(composer.share.text, "A title");
        assert_eq!(
            composer.hint(),
            "Facebook takes the link alone. y copies your comment · Enter opens it · Esc cancels"
        );
        assert_eq!(
            composer.apply(typed("y")),
            Some(Done::Copy("A title".into()))
        );
        assert!(composer.copied);
        assert_eq!(
            composer.hint(),
            "Comment copied. Enter opens Facebook · Esc cancels"
        );
        let url = "https://www.facebook.com/sharer/sharer.php?u=https%3A%2F%2Fsite.example%2Fa";
        assert_eq!(composer.apply(Input::Run), Some(Done::Open(url.into())));

        let pinboard = Composer::new(
            custom("Pinboard", "https://pinboard.in/add?url={url}"),
            "T".into(),
            "https://e.example".into(),
        );
        assert_eq!(
            pinboard.hint(),
            "Pinboard takes the link alone. y copies your comment · Enter opens it · Esc cancels"
        );
        let mastodon = Composer::new(
            custom("Mastodon", "https://mastodon.social/share?text={text}"),
            "T".into(),
            "https://e.example".into(),
        );
        assert!(mastodon.takes_text());
        assert_eq!(
            mastodon.hint(),
            "Enter opens Mastodon in the browser · Esc cancels"
        );
    }

    #[test]
    fn the_counter_follows_the_site() {
        let url = "https://site.example/a".to_owned();
        let mut x = Composer::new(Destination::X, "a".repeat(256), url.clone());
        assert_eq!(x.remaining(), Some(0));
        x.apply(typed("b"));
        assert_eq!(x.remaining(), Some(-1));
        let bluesky = Composer::new(Destination::Bluesky, "Hi".into(), url.clone());
        assert_eq!(bluesky.remaining(), Some(300 - 2 - 1 - 22));
        let facebook = Composer::new(Destination::Facebook, "Hi".into(), url);
        assert_eq!(facebook.remaining(), None);
    }
}
