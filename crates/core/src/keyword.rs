//! Keyword alerts in the store: which queries it follows, and adding one.
//! The refresh asks Google News for each (`sync`).

use std::collections::HashSet;

pub use gutenberg_sources::keyword::Keyword;
use gutenberg_store::{NewSource, Source, SourceKind, Store};

/// The query of every keyword alert in the store, as `Keyword::new` keeps
/// it. A source whose settings can't be read is left out.
pub fn queries(store: &Store) -> gutenberg_store::Result<HashSet<String>> {
    Ok(store
        .sources()?
        .into_iter()
        .filter(|s| s.kind == SourceKind::Keyword)
        .filter_map(|s| Keyword::from_config(&s.config).ok())
        .map(|k| k.query)
        .collect())
}

/// Adds an alert for `keyword`, named `title`, in `folder_id`. `None` when
/// the store already follows that query.
pub fn add(
    store: &Store,
    keyword: &Keyword,
    title: &str,
    folder_id: Option<i64>,
) -> gutenberg_store::Result<Option<Source>> {
    if queries(store)?.contains(&keyword.query) {
        return Ok(None);
    }
    insert(store, keyword, title, folder_id).map(Some)
}

/// Adds an alert without asking whether the store has one; for callers
/// that already know, from [`queries`].
pub fn insert(
    store: &Store,
    keyword: &Keyword,
    title: &str,
    folder_id: Option<i64>,
) -> gutenberg_store::Result<Source> {
    store.add_source(&NewSource {
        folder_id,
        kind: SourceKind::Keyword,
        title: title.to_owned(),
        url: None,
        config: keyword.to_config(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_is_added_once() {
        let store = Store::open_in_memory().unwrap();
        let booker = Keyword::new("\"Cory Booker\"").unwrap();
        let added = add(&store, &booker, "Cory Booker", None).unwrap().unwrap();
        assert_eq!(added.kind, SourceKind::Keyword);
        assert_eq!(added.url, None);
        assert_eq!(Keyword::from_config(&added.config).unwrap(), booker);
        // The same query with other spacing is the same alert.
        let again = Keyword::new("  \"Cory   Booker\" ").unwrap();
        assert!(add(&store, &again, "Booker", None).unwrap().is_none());
        assert_eq!(
            queries(&store).unwrap(),
            HashSet::from(["\"Cory Booker\"".to_owned()])
        );
    }
}
