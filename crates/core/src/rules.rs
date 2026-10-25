//! Rules: `[[rules]]` in `config.toml` star, read or mute new articles by
//! the words in their headline or summary.
//!
//! ```toml
//! [[rules]]
//! match = "sponsored"
//! action = "mute"
//!
//! [[rules]]
//! match = "Linux 7"
//! in = "Phoronix"
//! action = "star"
//! ```
//!
//! `match` is a word or phrase, found whole and in any case: "AI" matches
//! "AI's" but not "said". `in` names a source or a folder; without it a
//! rule covers every source. Rules meet each article once, as a refresh
//! first stores it: a muted article is never stored, and an article stored
//! before a rule was written keeps its flags. Mute wins; star and read can
//! both apply.

use gutenberg_store::{plain, Arrival, NewArticle};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(rename = "match")]
    pub words: String,
    /// A source's title or a folder's name.
    #[serde(rename = "in", default)]
    pub within: Option<String>,
    pub action: Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Star,
    Read,
    Mute,
}

/// The rules ready to match: lowercased once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules(Vec<Rule>);

/// What the rules left of a batch of new articles.
#[derive(Debug, Default)]
pub struct Sorted {
    pub articles: Vec<NewArticle>,
    /// How each of `articles` arrives, in the same order.
    pub arrivals: Vec<Arrival>,
    pub muted: usize,
}

impl Rules {
    pub fn new(rules: &[Rule]) -> Self {
        Self(
            rules
                .iter()
                .map(|rule| Rule {
                    words: rule.words.trim().to_lowercase(),
                    within: rule.within.as_ref().map(|w| w.trim().to_lowercase()),
                    action: rule.action,
                })
                .collect(),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `articles` from the source titled `source`, in the folder named
    /// `folder`, after the rules.
    pub fn sort(&self, source: &str, folder: Option<&str>, articles: Vec<NewArticle>) -> Sorted {
        let (source, folder) = (source.to_lowercase(), folder.map(str::to_lowercase));
        let here: Vec<&Rule> = self
            .0
            .iter()
            .filter(|rule| {
                rule.within
                    .as_ref()
                    .is_none_or(|w| *w == source || folder.as_ref() == Some(w))
            })
            .collect();
        let mut sorted = Sorted::default();
        for article in articles {
            if here.is_empty() {
                sorted.articles.push(article);
                sorted.arrivals.push(Arrival::default());
                continue;
            }
            let text = format!(
                "{} {}",
                article.title,
                article
                    .summary_html
                    .as_deref()
                    .map(plain)
                    .unwrap_or_default()
            )
            .to_lowercase();
            let mut arrival = Arrival::default();
            let mut muted = false;
            for rule in here
                .iter()
                .filter(|rule| contains_words(&text, &rule.words))
            {
                match rule.action {
                    Action::Star => arrival.starred = true,
                    Action::Read => arrival.read = true,
                    Action::Mute => muted = true,
                }
            }
            if muted {
                sorted.muted += 1;
            } else {
                sorted.articles.push(article);
                sorted.arrivals.push(arrival);
            }
        }
        sorted
    }
}

/// Whether `words` is in `text` whole: not inside a longer word at either
/// end. Both are lowercase.
fn contains_words(text: &str, words: &str) -> bool {
    if words.is_empty() {
        return false;
    }
    let edge = |c: Option<char>| !c.is_some_and(char::is_alphanumeric);
    let starts_word = words.starts_with(char::is_alphanumeric);
    let ends_word = words.ends_with(char::is_alphanumeric);
    text.match_indices(words).any(|(at, _)| {
        (!starts_word || edge(text[..at].chars().next_back()))
            && (!ends_word || edge(text[at + words.len()..].chars().next()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(words: &str, within: Option<&str>, action: Action) -> Rule {
        Rule {
            words: words.into(),
            within: within.map(Into::into),
            action,
        }
    }

    fn article(title: &str, summary: Option<&str>) -> NewArticle {
        NewArticle {
            source_id: 1,
            guid: title.into(),
            url: None,
            title: title.into(),
            author: None,
            summary_html: summary.map(Into::into),
            published_at: None,
            fetched_at: 1,
        }
    }

    fn titles(sorted: &Sorted) -> Vec<(&str, Arrival)> {
        sorted
            .articles
            .iter()
            .map(|a| a.title.as_str())
            .zip(sorted.arrivals.iter().copied())
            .collect()
    }

    #[test]
    fn words_match_whole_and_in_any_case() {
        assert!(contains_words("openai's new model", "openai"));
        assert!(contains_words("linux 7.4 is out", "linux 7"));
        assert!(contains_words("what is c++ for", "c++"));
        assert!(!contains_words("he said so", "ai"));
        assert!(!contains_words("linux 70 years", "linux 7"));
        assert!(contains_words("ai", "ai"));
        assert!(contains_words("— ai —", "ai"));
        assert!(!contains_words("anything", ""));
    }

    #[test]
    fn star_read_and_mute_meet_the_headline_and_the_summary() {
        let rules = Rules::new(&[
            rule("Sponsored", None, Action::Mute),
            rule("rust", None, Action::Star),
            rule("rumor", None, Action::Read),
        ]);
        let sorted = rules.sort(
            "Feed",
            None,
            vec![
                article("Rust 2.0 ships", None),
                article("A phone rumor", Some("<p>About <b>Rust</b></p>")),
                article("Gadget", Some("<p>SPONSORED post</p>")),
                article("Plain news", None),
            ],
        );
        let both = Arrival {
            read: true,
            starred: true,
        };
        let starred = Arrival {
            starred: true,
            ..Arrival::default()
        };
        assert_eq!(
            titles(&sorted),
            [
                ("Rust 2.0 ships", starred),
                ("A phone rumor", both),
                ("Plain news", Arrival::default()),
            ]
        );
        assert_eq!(sorted.muted, 1);
    }

    #[test]
    fn mute_wins() {
        let rules = Rules::new(&[
            rule("deal", None, Action::Star),
            rule("deal", None, Action::Mute),
        ]);
        let sorted = rules.sort("Feed", None, vec![article("A deal", None)]);
        assert!(sorted.articles.is_empty());
        assert_eq!(sorted.muted, 1);
    }

    #[test]
    fn in_names_a_source_or_a_folder() {
        let rules = Rules::new(&[rule("kernel", Some("phoronix"), Action::Star)]);
        let new = || vec![article("Kernel news", None)];
        assert!(rules.sort("Phoronix", None, new()).arrivals[0].starred);
        assert!(rules.sort("LWN", Some("Phoronix"), new()).arrivals[0].starred);
        assert!(!rules.sort("LWN", Some("Linux"), new()).arrivals[0].starred);
    }
}
