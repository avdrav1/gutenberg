//! Story grouping: articles from any source whose headlines share most of
//! their words, published within a few days of each other, are one story.
//! Similarity is the Jaccard index of the headlines' one-word shingles:
//! their sets of words, less the commonest. On headlines from real feeds and
//! keyword alerts, pairs at 0.5 or more were the same story; below it,
//! different stories about the same person began to pair.

use std::collections::HashMap;

use gutenberg_store::{Store, StoryCandidate};

/// Articles further apart than this are different stories.
pub const WINDOW: i64 = 3 * 86_400;

/// The least similarity that makes two headlines one story.
pub const THRESHOLD: f64 = 0.5;

/// A headline with fewer words than this left is a story on its own:
/// release titles such as "v4.0.2" would otherwise pair.
const MIN_WORDS: usize = 3;

/// Words too common to tell stories apart, in the languages the catalog
/// carries most.
#[rustfmt::skip]
const STOP: &[&str] = &[
    // English
    "a", "about", "after", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has",
    "have", "he", "her", "his", "how", "in", "into", "is", "it", "its", "new", "of", "on", "or",
    "over", "says", "she", "than", "that", "the", "their", "this", "to", "up", "was", "were",
    "what", "when", "who", "why", "will", "with",
    // French
    "au", "aux", "avec", "ce", "dans", "de", "des", "du", "en", "est", "et", "la", "le", "les",
    "par", "pour", "qui", "sa", "se", "son", "sur", "un", "une",
    // Spanish and German
    "al", "con", "del", "el", "los", "las", "por", "que", "das", "dem", "den", "der", "die",
    "ein", "eine", "im", "ist", "mit", "und", "von", "zu",
];

/// The words of a headline that count: lowercase, letters only, two or
/// more of them, without the commonest. Sorted and unique; empty when
/// fewer than [`MIN_WORDS`] are left.
pub fn shingles(title: &str) -> Vec<String> {
    let mut words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1 && w.chars().all(char::is_alphabetic))
        .map(str::to_lowercase)
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect();
    words.sort();
    words.dedup();
    if words.len() < MIN_WORDS {
        words.clear();
    }
    words
}

/// The Jaccard index of two headlines' shingles.
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (shingles(a), shingles(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.iter().filter(|w| b.binary_search(w).is_ok()).count();
    shared as f64 / (a.len() + b.len() - shared) as f64
}

/// A story for each candidate not yet in one, `(article, story)`: the
/// story of the most similar article within [`WINDOW`] of it, or its own.
/// `candidates` are oldest first, as [`Store::story_candidates`] gives them.
pub fn group(candidates: &[StoryCandidate]) -> Vec<(i64, i64)> {
    // Words become numbers, so the index and the sets are cheap.
    let mut vocabulary: HashMap<String, u32> = HashMap::new();
    let words: Vec<Vec<u32>> = candidates
        .iter()
        .map(|c| {
            let mut ids: Vec<u32> = shingles(&c.title)
                .into_iter()
                .map(|w| {
                    let next = vocabulary.len() as u32;
                    *vocabulary.entry(w).or_insert(next)
                })
                .collect();
            ids.sort_unstable();
            ids
        })
        .collect();
    let mut story: Vec<Option<i64>> = candidates.iter().map(|c| c.story_id).collect();
    // For each word, the grouped candidates within the window that have it.
    let mut index: HashMap<u32, Vec<usize>> = HashMap::new();
    let mut indexed = vec![false; candidates.len()];
    let add = |index: &mut HashMap<u32, Vec<usize>>, indexed: &mut [bool], i: usize| {
        for &w in &words[i] {
            index.entry(w).or_default().push(i);
        }
        indexed[i] = true;
    };
    let (mut left, mut right) = (0, 0);
    let mut assigned = Vec::new();
    for i in 0..candidates.len() {
        let at = candidates[i].sort_at;
        // Grouped articles up to WINDOW after this one come in, and any
        // more than WINDOW before it go out.
        while right < candidates.len() && candidates[right].sort_at <= at + WINDOW {
            if story[right].is_some() && !indexed[right] {
                add(&mut index, &mut indexed, right);
            }
            right += 1;
        }
        while candidates[left].sort_at < at - WINDOW {
            if indexed[left] {
                for w in &words[left] {
                    if let Some(list) = index.get_mut(w) {
                        list.retain(|&j| j != left);
                    }
                }
                indexed[left] = false;
            }
            left += 1;
        }
        if story[i].is_some() {
            continue;
        }
        let mut shared: HashMap<usize, usize> = HashMap::new();
        for w in &words[i] {
            for &j in index.get(w).into_iter().flatten() {
                *shared.entry(j).or_default() += 1;
            }
        }
        let best = shared
            .into_iter()
            .map(|(j, n)| (n as f64 / (words[i].len() + words[j].len() - n) as f64, j))
            .filter(|&(score, _)| score >= THRESHOLD)
            // The most similar; of equals, the earliest.
            .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
        let id = best.and_then(|(_, j)| story[j]).unwrap_or(candidates[i].id);
        story[i] = Some(id);
        assigned.push((candidates[i].id, id));
        if !words[i].is_empty() {
            add(&mut index, &mut indexed, i);
        }
    }
    assigned
}

/// Files every article stored since the last grouping under a story;
/// returns how many joined another article's.
pub fn group_stored(store: &mut Store) -> gutenberg_store::Result<usize> {
    let stories = group(&store.story_candidates(WINDOW)?);
    store.set_stories(&stories)?;
    Ok(stories.iter().filter(|(a, s)| a != s).count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: i64, title: &str, sort_at: i64, story_id: Option<i64>) -> StoryCandidate {
        StoryCandidate {
            id,
            title: title.to_owned(),
            sort_at,
            story_id,
        }
    }

    #[test]
    fn shingles_are_the_telling_words() {
        assert_eq!(
            shingles("Cory Booker's 'unlikely partner': Lindsey Graham"),
            ["booker", "cory", "graham", "lindsey", "partner", "unlikely"]
        );
        // Numbers and single letters don't count, and too few words is none.
        assert!(shingles("v4.0.2").is_empty());
        assert!(shingles("The end of an era").is_empty());
    }

    #[test]
    fn rephrased_headlines_pair_and_different_stories_do_not() {
        // From the captured Cory Booker searches and live feeds.
        let pairs = [
            (
                "Seven shot dead by gunmen at church funeral in Jordan, state media say",
                "Seven shot dead by gunmen at church funeral in Jordan, officials say",
                true,
            ),
            (
                "Sen. Cory Booker remembers Lindsey Graham as an ‘unlikely partner’",
                "Sen. Cory Booker on his time with Sen. Lindsey Graham",
                true,
            ),
            (
                "Missiles hit Kyiv after Russia agrees to US call to resume Ukraine peace talks",
                "Ukraine hit by strikes after Russia agrees to new peace talks",
                true,
            ),
            (
                "Cory Booker outraises GOP challenger Justin Murphy by factor of 50",
                "Justin Murphy wins GOP nod to take on Sen. Cory Booker",
                false,
            ),
            (
                "U.S. Senator Cory Booker on testosterone policy",
                "Senator Cory Booker on PIX11 Morning News",
                false,
            ),
            ("v4.0.1", "v4.0.2", false),
        ];
        for (a, b, same) in pairs {
            assert_eq!(similarity(a, b) >= THRESHOLD, same, "{a:?} / {b:?}");
        }
    }

    #[test]
    fn new_articles_join_the_story_they_match() {
        const DAY: i64 = 86_400;
        let grouped = group(&[
            candidate(1, "Seven shot dead at church funeral in Jordan", 0, Some(1)),
            candidate(2, "Storm floods the coast of Maine overnight", DAY, Some(2)),
            // Matches 1.
            candidate(
                3,
                "Gunmen kill seven at church funeral in Jordan",
                DAY,
                None,
            ),
            // Matches 2 and the new 5; 2 is grouped, so 2.
            candidate(4, "Storm floods Maine coast overnight", 2 * DAY, None),
            candidate(
                5,
                "Storm floods Maine coast overnight, again",
                2 * DAY,
                None,
            ),
            // Matches 1, but four days later.
            candidate(
                6,
                "Seven shot dead at church funeral in Jordan",
                4 * DAY + 1,
                None,
            ),
            candidate(7, "Nothing like the others at all", 4 * DAY + 2, None),
        ]);
        assert_eq!(grouped, [(3, 1), (4, 2), (5, 2), (6, 6), (7, 7)]);
    }

    #[test]
    fn an_article_can_join_a_later_grouped_one() {
        let grouped = group(&[
            candidate(9, "Storm floods the coast of Maine overnight", 0, None),
            candidate(2, "Storm floods Maine coast overnight", 100, Some(2)),
        ]);
        assert_eq!(grouped, [(9, 2)]);
    }
}
