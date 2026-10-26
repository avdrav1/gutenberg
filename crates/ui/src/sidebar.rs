//! The sources pane: what rows it shows, and what a drag does. Pure, so it
//! can be tested without a window.

use std::collections::HashMap;

use gutenberg_store::{Filter, Folder, Source, SourceTarget, View, ViewCounts};

/// The views, in the order their rows show at the top of the pane.
pub const VIEWS: [View; 4] = [View::All, View::Unread, View::Starred, View::Today];

/// One row of the sources pane, or the empty space under the last row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Entry {
    /// A view across every feed: all articles, unread, starred or today.
    View(View),
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
    /// The folder or feed a click on this row shows; a view row shows
    /// every feed.
    pub fn filter(self) -> Option<Filter> {
        match self {
            Entry::View(_) => Some(Filter::All),
            Entry::Folder(id) => Some(Filter::Folder(id)),
            Entry::Source { id, .. } => Some(Filter::Source(id)),
            Entry::Unfiled => None,
        }
    }
}

/// What a view's row is called.
pub fn view_name(view: View) -> &'static str {
    match view {
        View::All => "All articles",
        View::Unread => "Unread",
        View::Starred => "Starred",
        View::Today => "Today",
    }
}

/// The views, then each folder followed by its sources, then unfiled
/// sources. `folders` and `sources` come in stored order.
pub fn entries(folders: &[Folder], sources: &[Source]) -> Vec<Entry> {
    let mut rows: Vec<Entry> = VIEWS.into_iter().map(Entry::View).collect();
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

/// Which row stands for what the list shows: the chosen folder or feed,
/// or the view's row when every feed is shown.
pub fn cursor(entries: &[Entry], filter: Filter, view: View) -> Option<usize> {
    entries.iter().position(|&entry| match filter {
        Filter::All => entry == Entry::View(view),
        _ => entry.filter() == Some(filter) && !matches!(entry, Entry::View(_)),
    })
}

/// The row `j`, `k`, `gg` or `G` moves to from `at`, within the rows.
pub fn step(len: usize, at: Option<usize>, to: Step) -> Option<usize> {
    let last = len.checked_sub(1)?;
    Some(match (to, at) {
        (Step::First, _) | (Step::Up, None) => 0,
        (Step::Last, _) => last,
        (Step::Down, None) => 0,
        (Step::Down, Some(at)) => (at + 1).min(last),
        (Step::Up, Some(at)) => at.saturating_sub(1),
    })
}

/// The pane's new scroll offset to show a row `row_height` tall at
/// `row_top` in its content, moving as little as possible from `top`;
/// `None` when it's in view already.
pub fn scroll_to_show(top: f32, height: f32, row_top: f32, row_height: f32) -> Option<f32> {
    if row_top < top {
        Some(row_top)
    } else if row_top + row_height > top + height {
        Some(row_top + row_height - height)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Down,
    Up,
    First,
    Last,
}

/// The number beside a row: a feed's unread articles, a folder's feeds'
/// sum, unread everywhere for "All articles" and "Unread", and how many
/// articles are starred or came today for those views.
pub fn count(
    entry: Entry,
    sources: &[Source],
    unread: &HashMap<i64, i64>,
    views: &ViewCounts,
) -> i64 {
    let of = |id: i64| unread.get(&id).copied().unwrap_or(0);
    match entry {
        Entry::View(View::All | View::Unread) => views.unread,
        Entry::View(View::Starred) => views.starred,
        Entry::View(View::Today) => views.today,
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
    fn views_come_first_then_folders_hold_their_sources_and_unfiled_come_last() {
        let folders = [folder(1), folder(2)];
        let sources = [source(10, None), source(11, Some(2)), source(12, Some(1))];
        assert_eq!(
            entries(&folders, &sources),
            [
                Entry::View(View::All),
                Entry::View(View::Unread),
                Entry::View(View::Starred),
                Entry::View(View::Today),
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
    fn folder_counts_add_up_their_sources_and_views_use_their_own() {
        let sources = [source(10, Some(1)), source(11, Some(1)), source(12, None)];
        let unread = HashMap::from([(10, 3), (11, 4), (12, 5)]);
        let views = ViewCounts {
            unread: 12,
            starred: 2,
            today: 6,
        };
        let count = |entry| count(entry, &sources, &unread, &views);
        assert_eq!(count(Entry::Folder(1)), 7);
        assert_eq!(
            count(Entry::Source {
                id: 12,
                filed: false
            }),
            5
        );
        assert_eq!(VIEWS.map(|v| count(Entry::View(v))), [12, 12, 2, 6]);
    }

    #[test]
    fn the_cursor_is_the_chosen_folder_or_feed_else_the_view() {
        let rows = entries(&[folder(10)], &[source(1, Some(10)), source(2, None)]);
        let at = |filter, view| cursor(&rows, filter, view).map(|i| rows[i]);
        assert_eq!(at(Filter::All, View::Today), Some(Entry::View(View::Today)));
        assert_eq!(
            at(Filter::Folder(10), View::Unread),
            Some(Entry::Folder(10))
        );
        assert_eq!(
            at(Filter::Source(2), View::All),
            Some(Entry::Source {
                id: 2,
                filed: false
            })
        );
        assert_eq!(at(Filter::Source(9), View::All), None);
    }

    #[test]
    fn steps_stay_within_the_rows() {
        assert_eq!(step(5, Some(2), Step::Down), Some(3));
        assert_eq!(step(5, Some(4), Step::Down), Some(4));
        assert_eq!(step(5, Some(0), Step::Up), Some(0));
        assert_eq!(step(5, Some(3), Step::First), Some(0));
        assert_eq!(step(5, Some(1), Step::Last), Some(4));
        assert_eq!(step(5, None, Step::Down), Some(0));
        assert_eq!(step(0, None, Step::Down), None);
    }

    #[test]
    fn the_pane_scrolls_only_as_far_as_the_row() {
        assert_eq!(scroll_to_show(0.0, 300.0, 100.0, 30.0), None);
        assert_eq!(scroll_to_show(0.0, 300.0, 290.0, 30.0), Some(20.0));
        assert_eq!(scroll_to_show(200.0, 300.0, 150.0, 30.0), Some(150.0));
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
        assert_eq!(drop_on(s(1), Entry::View(View::All)), None);
    }
}
