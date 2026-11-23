//! Learned interest (S10.4): how likely you are to open an article, learned
//! from what you did with others. Naive Bayes over the headline's words
//! (as story grouping reads them) and the article's source. An article you
//! opened or starred is one you were interested in; one you marked read
//! without opening is one you weren't.
//!
//! A score is the sum of the log likelihood ratios of the features an
//! article has that some signal had too: a word nobody has seen yet says
//! nothing either way. With no signals every score is 0, so an order by
//! score that keeps ties in place is the newest-first order.

use std::collections::HashMap;

use gutenberg_store::Signal;

use crate::story;

/// The least a feature must lift an article, as a likelihood ratio's log,
/// to be named as the reason: twice as likely among the opened as the
/// skipped.
const REASON_LIFT: f64 = std::f64::consts::LN_2;

/// How many interested signals a feature needs before it's named as the
/// reason: one opened article isn't "a lot".
const REASON_SUPPORT: u32 = 2;

/// What lifted an article most, for its reason line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// "you read a lot from LWN": the source's id.
    Source(i64),
    /// "you open headlines with “rust”".
    Word(String),
}

/// How often a feature came with each kind of signal.
#[derive(Debug, Clone, Copy, Default)]
struct Seen {
    interested: u32,
    skipped: u32,
}

/// What signals taught, ready to score articles.
#[derive(Debug, Clone, Default)]
pub struct Model {
    interested: u32,
    skipped: u32,
    sources: HashMap<i64, Seen>,
    words: HashMap<String, Seen>,
}

impl Model {
    pub fn learn(signals: &[Signal]) -> Self {
        let mut model = Model::default();
        for signal in signals {
            let count = |seen: &mut Seen| {
                if signal.interested {
                    seen.interested += 1;
                } else {
                    seen.skipped += 1;
                }
            };
            if signal.interested {
                model.interested += 1;
            } else {
                model.skipped += 1;
            }
            count(model.sources.entry(signal.source_id).or_default());
            for word in story::words(&signal.title) {
                count(model.words.entry(word).or_default());
            }
        }
        model
    }

    /// Whether it learned anything: with nothing, every score is 0.
    pub fn is_empty(&self) -> bool {
        self.interested + self.skipped == 0
    }

    /// How a feature seen `seen` times moves the odds of interest: the log
    /// of its likelihood among interested signals over its likelihood
    /// among skipped ones, each smoothed by one of each.
    fn lift(&self, seen: Seen) -> f64 {
        let given = |n: u32, of: u32| (f64::from(n) + 1.0) / (f64::from(of) + 2.0);
        (given(seen.interested, self.interested) / given(seen.skipped, self.skipped)).ln()
    }

    /// The features of an article that some signal had, with their lift.
    fn features(&self, source_id: i64, title: &str) -> Vec<(Reason, Seen, f64)> {
        let source = self
            .sources
            .get(&source_id)
            .map(|&seen| (Reason::Source(source_id), seen, self.lift(seen)));
        let words = story::words(title).into_iter().filter_map(|word| {
            let seen = *self.words.get(&word)?;
            Some((Reason::Word(word), seen, self.lift(seen)))
        });
        source.into_iter().chain(words).collect()
    }

    /// Higher is likelier to interest you; 0 is no evidence either way.
    pub fn score(&self, source_id: i64, title: &str) -> f64 {
        if self.is_empty() {
            return 0.0;
        }
        self.features(source_id, title)
            .iter()
            .map(|(_, _, lift)| lift)
            .sum()
    }

    /// The feature that lifted an article most, when it lifted it enough
    /// and enough interested signals had it to say so.
    pub fn reason(&self, source_id: i64, title: &str) -> Option<Reason> {
        self.features(source_id, title)
            .into_iter()
            .filter(|(_, seen, lift)| seen.interested >= REASON_SUPPORT && *lift >= REASON_LIFT)
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(reason, ..)| reason)
    }

    /// Reorders `items` most interesting first; equal scores keep their
    /// order, so with no signals nothing moves.
    pub fn order<T>(&self, items: &mut Vec<T>, key: impl Fn(&T) -> (i64, &str)) {
        if self.is_empty() {
            return;
        }
        let mut scored: Vec<(f64, T)> = items
            .drain(..)
            .map(|item| {
                let (source_id, title) = key(&item);
                (self.score(source_id, title), item)
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        items.extend(scored.into_iter().map(|(_, item)| item));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(source_id: i64, title: &str, interested: bool) -> Signal {
        Signal {
            source_id,
            title: title.to_owned(),
            interested,
        }
    }

    #[test]
    fn with_no_signals_nothing_moves_and_nothing_is_named() {
        let model = Model::learn(&[]);
        assert!(model.is_empty());
        assert_eq!(model.score(1, "Rust 2.0 released"), 0.0);
        assert_eq!(model.reason(1, "Rust 2.0 released"), None);
        let mut items = vec![(2, "b"), (1, "a"), (3, "c")];
        model.order(&mut items, |&(id, title)| (id, title));
        assert_eq!(items, [(2, "b"), (1, "a"), (3, "c")]);
    }

    #[test]
    fn a_source_you_open_lifts_its_articles_and_one_you_skip_sinks_them() {
        let model = Model::learn(&[
            signal(1, "Kernel scheduler rework lands", true),
            signal(1, "Filesystem changes merged", true),
            signal(1, "Memory folios explained", true),
            signal(2, "Transfer window gossip roundup", false),
            signal(2, "Derby ends goalless again", false),
        ]);
        let lwn = model.score(1, "Something else entirely");
        let sport = model.score(2, "Something else entirely");
        let unknown = model.score(3, "Something else entirely");
        assert!(lwn > unknown && unknown > sport, "{lwn} {unknown} {sport}");
        assert_eq!(unknown, 0.0);
        assert_eq!(
            model.reason(1, "Something else entirely"),
            Some(Reason::Source(1))
        );
        assert_eq!(model.reason(2, "Something else entirely"), None);
    }

    #[test]
    fn a_word_you_keep_opening_is_named_once_it_has_been_opened_twice() {
        let once = Model::learn(&[
            signal(1, "Rust compiler speeds up", true),
            signal(2, "Election results tonight", false),
            signal(3, "Weather warning issued", false),
        ]);
        assert_eq!(once.reason(4, "Rust in the kernel"), None);
        let twice = Model::learn(&[
            signal(1, "Rust compiler speeds up", true),
            signal(2, "Rust editions explained", true),
            signal(3, "Election results tonight", false),
            signal(4, "Weather warning issued", false),
        ]);
        assert_eq!(
            twice.reason(5, "Rust in the kernel"),
            Some(Reason::Word("rust".into()))
        );
        assert!(twice.score(5, "Rust in the kernel") > twice.score(5, "Cricket in the rain"));
    }

    /// The acceptance line: six days of reading teach it, and on a seventh,
    /// held out, the articles like the ones starred come first.
    #[test]
    fn after_a_week_the_held_out_days_starred_like_articles_come_first() {
        const LWN: i64 = 1;
        const SPORT: i64 = 2;
        const WORLD: i64 = 3;
        // What each source writes about; the reader opens LWN and anything
        // about Rust, stars kernel news, and skips sport and most world news.
        let day = |d: usize| -> Vec<(i64, String, Option<bool>)> {
            let n = |list: &[&str]| list[d % list.len()].to_owned();
            vec![
                (
                    LWN,
                    format!("Kernel {} patches land", n(&["memory", "scheduler", "io"])),
                    Some(true),
                ),
                (
                    LWN,
                    format!("{} release notes", n(&["Debian", "Fedora", "Arch"])),
                    Some(true),
                ),
                (
                    WORLD,
                    format!("Rust adopted by {} agency", n(&["tax", "space", "rail"])),
                    Some(true),
                ),
                (
                    WORLD,
                    format!("Talks resume in {}", n(&["Geneva", "Doha", "Vienna"])),
                    Some(false),
                ),
                (
                    WORLD,
                    format!(
                        "Storm hits {} coast",
                        n(&["eastern", "western", "northern"])
                    ),
                    None,
                ),
                (
                    SPORT,
                    format!("{} win the derby", n(&["Rovers", "United", "City"])),
                    Some(false),
                ),
                (
                    SPORT,
                    format!(
                        "Transfer rumours: {} striker",
                        n(&["young", "veteran", "loan"])
                    ),
                    Some(false),
                ),
            ]
        };
        let signals: Vec<Signal> = (0..6)
            .flat_map(day)
            .filter_map(|(source, title, did)| did.map(|did| signal(source, &title, did)))
            .collect();
        let model = Model::learn(&signals);

        let mut held_out = day(6);
        model.order(&mut held_out, |(source, title, _)| {
            (*source, title.as_str())
        });
        let liked = held_out
            .iter()
            .filter(|(.., did)| *did == Some(true))
            .count();
        assert!(
            held_out[..liked].iter().all(|(.., did)| *did == Some(true)),
            "{held_out:#?}"
        );
        assert!(held_out[liked..].iter().all(|(.., did)| *did != Some(true)));
        let (lwn, title, _) = &held_out[0];
        assert!(model.reason(*lwn, title).is_some());
    }
}
