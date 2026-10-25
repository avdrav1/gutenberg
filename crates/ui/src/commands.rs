//! Every action the main window has, by name and key: what the command
//! palette searches and what help lists. One row here, one line in
//! `docs/KEYS.md`, and a key in `list::action_for` make a new action.

use gutenberg_store::View;

use crate::list::Action;

/// An action as a person finds it: by name, or by its keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub action: Action,
    pub name: &'static str,
    /// As shown, first the main one: `["o", "Enter"]`.
    pub keys: &'static [&'static str],
}

/// In the order help shows them.
pub const ALL: &[Command] = &[
    Command {
        action: Action::Down,
        name: "Next article",
        keys: &["j", "↓"],
    },
    Command {
        action: Action::Up,
        name: "Previous article",
        keys: &["k", "↑"],
    },
    Command {
        action: Action::First,
        name: "First article",
        keys: &["gg"],
    },
    Command {
        action: Action::Last,
        name: "Last article",
        keys: &["G"],
    },
    Command {
        action: Action::Open,
        name: "Open article",
        keys: &["o", "Enter"],
    },
    Command {
        action: Action::Reader,
        name: "Reader view: full article or the feed's summary",
        keys: &["v"],
    },
    Command {
        action: Action::Star,
        name: "Star or unstar article",
        keys: &["s"],
    },
    Command {
        action: Action::ToggleRead,
        name: "Mark article unread or read",
        keys: &["m"],
    },
    Command {
        action: Action::MarkFolderRead,
        name: "Mark folder read: everything in the chosen folder or feed",
        keys: &["M"],
    },
    Command {
        action: Action::OpenInBrowser,
        name: "Open article in browser",
        keys: &["O"],
    },
    Command {
        action: Action::CopyLink,
        name: "Copy article link",
        keys: &["y"],
    },
    Command {
        action: Action::Search,
        name: "Search articles",
        keys: &["/"],
    },
    Command {
        action: Action::ClearSearch,
        name: "Clear search",
        keys: &["Esc"],
    },
    Command {
        action: Action::View(View::All),
        name: "View all articles",
        keys: &["A"],
    },
    Command {
        action: Action::View(View::Unread),
        name: "View unread articles only",
        keys: &["u"],
    },
    Command {
        action: Action::View(View::Starred),
        name: "View starred articles",
        keys: &["S"],
    },
    Command {
        action: Action::View(View::Today),
        name: "View today's articles",
        keys: &["t"],
    },
    Command {
        action: Action::Tools,
        name: "MCP server's tools: pick the one it calls",
        keys: &["T"],
    },
    Command {
        action: Action::AddSources,
        name: "Add sources",
        keys: &["a"],
    },
    Command {
        action: Action::Palette,
        name: "Command palette",
        keys: &[":"],
    },
    Command {
        action: Action::Help,
        name: "Help: every key",
        keys: &["?"],
    },
    Command {
        action: Action::Quit,
        name: "Quit",
        keys: &["q"],
    },
];

/// The commands whose name matches `query`, best first; all of them, in
/// table order, for an empty query.
pub fn search(query: &str) -> Vec<&'static Command> {
    let mut scored: Vec<(i32, usize, &Command)> = ALL
        .iter()
        .enumerate()
        .filter_map(|(i, c)| score(query, c.name).map(|s| (s, i, c)))
        .collect();
    // Higher score first; ties keep table order.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, c)| c).collect()
}

/// How well `query` matches `text`, or `None` when it doesn't: every
/// character of the query must appear in the text in order, ignoring case
/// and spaces in the query. A character scores more at the start of a word
/// and right after the previous match, so "rv" finds "Reader view" first.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let mut total = 0;
    let mut from = 0;
    let mut last: Option<usize> = None;
    for q in query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
    {
        let at = from + text[from..].iter().position(|&c| c == q)?;
        let word_start = at == 0 || !text[at - 1].is_alphanumeric();
        total += 1;
        if word_start {
            total += 8;
        }
        if last.is_some_and(|l| l + 1 == at) {
            total += 5;
        }
        // Matches that start late count a little less.
        if last.is_none() {
            total -= at.min(10) as i32;
        }
        last = Some(at);
        from = at + 1;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list;
    use iced::keyboard::{key::Named, Key, Modifiers};

    fn names(query: &str) -> Vec<&'static str> {
        search(query).iter().map(|c| c.name).collect()
    }

    #[test]
    fn an_empty_query_lists_everything_in_order() {
        assert_eq!(names(""), ALL.iter().map(|c| c.name).collect::<Vec<_>>());
        assert_eq!(names("   "), names(""));
    }

    #[test]
    fn letters_must_appear_in_order_ignoring_case() {
        assert_eq!(names("QUIT"), ["Quit"]);
        assert_eq!(score("tiuq", "Quit"), None);
        assert!(names("zz").is_empty());
    }

    #[test]
    fn word_starts_and_runs_rank_first() {
        assert_eq!(
            names("rv")[0],
            "Reader view: full article or the feed's summary"
        );
        assert_eq!(names("next")[0], "Next article");
        assert_eq!(names("add s")[0], "Add sources");
        // "art" is in many names; whole-word "article" ones lead.
        assert_eq!(&names("first art")[..1], ["First article"]);
    }

    #[test]
    fn every_action_has_a_row_and_its_keys_do_it() {
        let named = |k: &str| match k {
            "↓" => Some(Key::Named(Named::ArrowDown)),
            "↑" => Some(Key::Named(Named::ArrowUp)),
            "Enter" => Some(Key::Named(Named::Enter)),
            "Esc" => Some(Key::Named(Named::Escape)),
            _ => None,
        };
        for command in ALL {
            for keys in command.keys {
                let action = if *keys == "gg" {
                    let g = Key::Character("g".into());
                    assert_eq!(
                        list::action_for(&g, Modifiers::empty(), false),
                        Some(list::Action::PendingG)
                    );
                    list::action_for(&g, Modifiers::empty(), true)
                } else {
                    let key = named(keys).unwrap_or_else(|| Key::Character((*keys).into()));
                    list::action_for(&key, Modifiers::empty(), false)
                };
                assert_eq!(action, Some(command.action), "{keys} for {}", command.name);
            }
        }
        // Every action a person can take has a row. The match fails to
        // compile when a new action is added, pointing here.
        use list::Action as A;
        let needs_row = |a: A| match a {
            // The first half of gg, not an action of its own.
            A::PendingG => false,
            A::Quit
            | A::Down
            | A::Up
            | A::First
            | A::Last
            | A::Open
            | A::AddSources
            | A::Reader
            | A::Palette
            | A::Help
            | A::Star
            | A::ToggleRead
            | A::MarkFolderRead
            | A::OpenInBrowser
            | A::CopyLink
            | A::Search
            | A::ClearSearch
            | A::Tools
            | A::View(_) => true,
        };
        let every = [
            A::Quit,
            A::Down,
            A::Up,
            A::First,
            A::Last,
            A::Open,
            A::PendingG,
            A::AddSources,
            A::Reader,
            A::Palette,
            A::Help,
            A::Star,
            A::ToggleRead,
            A::MarkFolderRead,
            A::OpenInBrowser,
            A::CopyLink,
            A::Search,
            A::ClearSearch,
            A::View(View::All),
            A::View(View::Unread),
            A::View(View::Starred),
            A::View(View::Today),
            A::Tools,
        ];
        for action in every.into_iter().filter(|a| needs_row(*a)) {
            assert!(
                ALL.iter().any(|c| c.action == action),
                "{action:?} has no row"
            );
        }
    }
}
