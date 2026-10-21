//! The sources pane: what rows it shows, and what a drag does. Pure, so it
//! can be tested without a window.

use std::collections::HashMap;

use gutenberg_store::{Filter, Folder, Source, SourceTarget};

/// One row of the sources pane, or the empty space under the last row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Entry {
    All,
    Folder(i64),
    /// A source; `filed` when it sits in a folder (and is indented).
    Source {
        id: i64,
        filed: bool,
    },
    /// The space below the rows: drop here to take a source out of its folder.
    Unfiled,
}

impl Entry {
    /// The list filter a click on this row selects.
    pub fn filter(self) -> Option<Filter> {
        match self {
            Entry::All => Some(Filter::All),
            Entry::Folder(id) => Some(Filter::Folder(id)),
            Entry::Source { id, .. } => Some(Filter::Source(id)),
            Entry::Unfiled => None,
        }
    }
}

/// "All", then each folder followed by its sources, then unfiled sources.
/// `folders` and `sources` come in stored order.
pub fn entries(folders: &[Folder], sources: &[Source]) -> Vec<Entry> {
    let mut rows = vec![Entry::All];
    for folder in folders {
        rows.push(Entry::Folder(folder.id));
        rows.extend(
            sources
                .iter()
                .filter(|s| s.folder_id == Some(folder.id))
                .map(|s| Entry::Source {
                    id: s.id,
                    filed: true,
                }),
        );
    }
    let filed: Vec<i64> = folders.iter().map(|f| f.id).collect();
    rows.extend(
        sources
            .iter()
            .filter(|s| s.folder_id.is_none_or(|f| !filed.contains(&f)))
            .map(|s| Entry::Source {
                id: s.id,
                filed: false,
            }),
    );
    rows
}

/// Unread counts for a row: a source's own, a folder's sources' sum, or
/// everything for "All".
pub fn count(entry: Entry, sources: &[Source], unread: &HashMap<i64, i64>) -> i64 {
    let of = |id: i64| unread.get(&id).copied().unwrap_or(0);
    match entry {
        Entry::All => unread.values().sum(),
        Entry::Folder(folder) => sources
            .iter()
            .filter(|s| s.folder_id == Some(folder))
            .map(|s| of(s.id))
            .sum(),
        Entry::Source { id, .. } => of(id),
        Entry::Unfiled => 0,
    }
}

/// What dropping `dragged` on `target` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Source(i64, SourceTarget),
    /// A folder, before another (or last with `None`).
    Folder(i64, Option<i64>),
}

pub fn drop_on(dragged: Entry, target: Entry) -> Option<Move> {
    match (dragged, target) {
        (Entry::Source { id, .. }, Entry::Source { id: other, .. }) if id != other => {
            Some(Move::Source(id, SourceTarget::Before(other)))
        }
        (Entry::Source { id, .. }, Entry::Folder(folder)) => {
            Some(Move::Source(id, SourceTarget::EndOf(Some(folder))))
        }
        (Entry::Source { id, .. }, Entry::Unfiled) => {
            Some(Move::Source(id, SourceTarget::EndOf(None)))
        }
        (Entry::Folder(id), Entry::Folder(other)) if id != other => {
            Some(Move::Folder(id, Some(other)))
        }
        (Entry::Folder(id), Entry::Unfiled) => Some(Move::Folder(id, None)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gutenberg_store::SourceKind;

    fn folder(id: i64) -> Folder {
        Folder {
            id,
            name: format!("f{id}"),
            position: id,
        }
    }

    fn source(id: i64, folder_id: Option<i64>) -> Source {
        Source {
            id,
            folder_id,
            kind: SourceKind::Feed,
            title: format!("s{id}"),
            url: None,
            config: "{}".into(),
            position: id,
            created_at: 0,
        }
    }

    #[test]
    fn folders_hold_their_sources_and_unfiled_come_last() {
        let folders = [folder(1), folder(2)];
        let sources = [source(10, None), source(11, Some(2)), source(12, Some(1))];
        assert_eq!(
            entries(&folders, &sources),
            [
                Entry::All,
                Entry::Folder(1),
                Entry::Source {
                    id: 12,
                    filed: true
                },
                Entry::Folder(2),
                Entry::Source {
                    id: 11,
                    filed: true
                },
                Entry::Source {
                    id: 10,
                    filed: false
                },
            ]
        );
    }

    #[test]
    fn folder_counts_add_up_their_sources() {
        let sources = [source(10, Some(1)), source(11, Some(1)), source(12, None)];
        let unread = HashMap::from([(10, 3), (11, 4), (12, 5)]);
        assert_eq!(count(Entry::Folder(1), &sources, &unread), 7);
        assert_eq!(count(Entry::All, &sources, &unread), 12);
        assert_eq!(
            count(
                Entry::Source {
                    id: 12,
                    filed: false
                },
                &sources,
                &unread
            ),
            5
        );
    }

    #[test]
    fn drops() {
        let s = |id| Entry::Source { id, filed: false };
        assert_eq!(
            drop_on(s(1), s(2)),
            Some(Move::Source(1, SourceTarget::Before(2)))
        );
        assert_eq!(drop_on(s(1), s(1)), None);
        assert_eq!(
            drop_on(s(1), Entry::Folder(7)),
            Some(Move::Source(1, SourceTarget::EndOf(Some(7))))
        );
        assert_eq!(
            drop_on(s(1), Entry::Unfiled),
            Some(Move::Source(1, SourceTarget::EndOf(None)))
        );
        assert_eq!(
            drop_on(Entry::Folder(7), Entry::Folder(8)),
            Some(Move::Folder(7, Some(8)))
        );
        assert_eq!(drop_on(Entry::Folder(7), s(1)), None);
        assert_eq!(drop_on(s(1), Entry::All), None);
    }
}
