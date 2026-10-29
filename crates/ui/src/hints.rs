//! The key bar along the bottom of the window: the few keys that matter
//! for what's on screen. Actions take their keys from `commands::ALL`, so
//! the bar says what the palette and help say.

use gutenberg_store::View;

use crate::commands;
use crate::list::Action;

/// What the window shows, as far as the keys go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// No sources yet.
    Empty,
    /// The list, nothing open.
    List,
    /// The keys are in the sources pane.
    Sources,
    /// An article open, as the feed sent it.
    Summary,
    /// An article open as its full page.
    Reader,
    /// Typing in the search line.
    Searching,
    /// Moving through search results.
    Results,
    /// An MCP server's tools in the list pane.
    Tools,
    /// Typing a tool's arguments.
    Arguments,
    /// Typing an MCP server's key.
    Key,
    /// Picking where to share.
    SharePicker,
    /// Writing the share for a site that takes the comment.
    ShareCompose,
    /// The share for a site that takes the link alone: `y` copies the
    /// comment instead of typing it.
    ShareLink,
}

/// One entry in the bar: keys, then what they do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint {
    pub keys: &'static str,
    pub label: &'static str,
}

/// Most useful first: a narrow window cuts the bar from the end.
pub fn for_screen(screen: Screen) -> Vec<Hint> {
    let moves = Hint {
        keys: "j k",
        label: "move",
    };
    match screen {
        Screen::Empty => vec![
            key(Action::AddSources, "add sources"),
            key(Action::Help, "keys"),
            key(Action::Quit, "quit"),
        ],
        Screen::List => vec![
            moves,
            key(Action::Open, "open"),
            key(Action::Search, "search"),
            key(Action::View(View::Unread), "unread"),
            key(Action::Share, "share"),
            key(Action::AddSources, "add sources"),
            key(Action::Palette, "commands"),
            key(Action::Help, "keys"),
            key(Action::NextLayout, "layout"),
        ],
        Screen::Sources => vec![
            moves,
            key(Action::Open, "articles"),
            key(Action::NextPane, "list"),
            key(Action::View(View::Unread), "unread"),
            key(Action::AddSources, "add sources"),
            key(Action::Palette, "commands"),
            key(Action::Help, "keys"),
        ],
        Screen::Summary => vec![
            moves,
            key(Action::Open, "open"),
            key(Action::Reader, "full story"),
            key(Action::Star, "star"),
            key(Action::OpenInBrowser, "browser"),
            key(Action::Share, "share"),
            key(Action::Search, "search"),
            key(Action::Palette, "commands"),
            key(Action::Help, "keys"),
        ],
        Screen::Reader => vec![
            key(Action::Reader, "summary"),
            moves,
            key(Action::Star, "star"),
            key(Action::OpenInBrowser, "browser"),
            key(Action::CopyLink, "copy link"),
            key(Action::Share, "share"),
            key(Action::Palette, "commands"),
            key(Action::Help, "keys"),
        ],
        // The search line takes every key; these are its own.
        Screen::Searching => vec![
            Hint {
                keys: "Enter",
                label: "browse results",
            },
            Hint {
                keys: "↑ ↓",
                label: "move",
            },
            key(Action::ClearSearch, "back to the list"),
        ],
        Screen::Results => vec![
            moves,
            key(Action::Open, "open"),
            Hint {
                keys: "/",
                label: "edit search",
            },
            key(Action::ClearSearch, "back to the list"),
            key(Action::Help, "keys"),
        ],
        Screen::Tools => vec![
            moves,
            Hint {
                keys: "Enter",
                label: "edit arguments",
            },
            key(Action::Tools, "articles"),
            key(Action::Palette, "commands"),
            key(Action::Help, "keys"),
        ],
        // The fields take every key; these are their own.
        Screen::Arguments | Screen::Key => vec![
            Hint {
                keys: "Enter",
                label: "save",
            },
            Hint {
                keys: "Esc",
                label: "cancel",
            },
        ],
        // The picker and the compose box take every key too.
        Screen::SharePicker => vec![
            moves,
            Hint {
                keys: "Enter",
                label: "choose",
            },
            Hint {
                keys: "letter",
                label: "choose by name",
            },
            Hint {
                keys: "Esc",
                label: "cancel",
            },
        ],
        // Where the site takes the comment, `y` types it, so only the
        // link-only screen offers the copy.
        Screen::ShareCompose => vec![
            Hint {
                keys: "Enter",
                label: "open in the browser",
            },
            Hint {
                keys: "Esc",
                label: "cancel",
            },
        ],
        Screen::ShareLink => vec![
            Hint {
                keys: "Enter",
                label: "open in the browser",
            },
            Hint {
                keys: "y",
                label: "copy the comment",
            },
            Hint {
                keys: "Esc",
                label: "cancel",
            },
        ],
    }
}

/// `action`'s main key, from its row in the command table.
fn key(action: Action, label: &'static str) -> Hint {
    let command = commands::ALL
        .iter()
        .find(|c| c.action == action)
        .unwrap_or_else(|| panic!("{action:?} has no row in commands::ALL"));
    Hint {
        keys: command.keys[0],
        label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY: [Screen; 13] = [
        Screen::Empty,
        Screen::List,
        Screen::Sources,
        Screen::Summary,
        Screen::Reader,
        Screen::Searching,
        Screen::Results,
        Screen::Tools,
        Screen::Arguments,
        Screen::Key,
        Screen::SharePicker,
        Screen::ShareCompose,
        Screen::ShareLink,
    ];

    #[test]
    fn every_screen_has_hints_and_their_keys_come_from_the_table() {
        // `key` panics on an action with no row, so building them is the test.
        for screen in EVERY {
            assert!(!for_screen(screen).is_empty(), "{screen:?}");
        }
        let open = for_screen(Screen::List)[1];
        assert_eq!((open.keys, open.label), ("o", "open"));
    }

    #[test]
    fn help_is_a_key_away_wherever_the_list_has_the_keys() {
        let typing = [
            Screen::Searching,
            Screen::Arguments,
            Screen::Key,
            Screen::SharePicker,
            Screen::ShareCompose,
            Screen::ShareLink,
        ];
        for screen in EVERY.into_iter().filter(|s| !typing.contains(s)) {
            assert!(
                for_screen(screen).iter().any(|h| h.keys == "?"),
                "{screen:?}"
            );
        }
    }

    #[test]
    fn only_the_link_only_share_offers_y_to_copy() {
        let offers_y = |s| for_screen(s).iter().any(|h| h.keys == "y");
        assert!(!offers_y(Screen::ShareCompose));
        assert!(offers_y(Screen::ShareLink));
    }

    #[test]
    fn the_bar_follows_the_reader_view() {
        let first = |s| for_screen(s)[0].label;
        assert_eq!(first(Screen::Reader), "summary");
        assert!(for_screen(Screen::Summary)
            .iter()
            .any(|h| h.label == "full story"));
    }
}
