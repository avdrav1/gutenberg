//! The command palette's state and keys. Pure, so it's tested without a
//! window; `lib.rs` draws it and runs what it picks.

use iced::keyboard::{key::Named, Key, Modifiers};

use crate::commands::{self, Command};

/// What a key does while the palette is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Type(String),
    Backspace,
    Up,
    Down,
    Run,
    Close,
}

/// `text` is what the key typed, layout and Shift applied.
pub fn input_for(key: &Key, modifiers: Modifiers, text: Option<&str>) -> Option<Input> {
    if modifiers.control() {
        return match key.as_ref() {
            Key::Character("n") => Some(Input::Down),
            Key::Character("p") => Some(Input::Up),
            _ => None,
        };
    }
    match key.as_ref() {
        Key::Named(Named::Escape) => Some(Input::Close),
        Key::Named(Named::Enter) => Some(Input::Run),
        Key::Named(Named::Backspace) => Some(Input::Backspace),
        Key::Named(Named::ArrowDown) => Some(Input::Down),
        Key::Named(Named::ArrowUp) => Some(Input::Up),
        Key::Named(Named::Tab) if modifiers.shift() => Some(Input::Up),
        Key::Named(Named::Tab) => Some(Input::Down),
        _ if modifiers.alt() || modifiers.logo() => None,
        _ => text
            .filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
            .map(|t| Input::Type(t.to_owned())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub query: String,
    /// The commands matching the query, best first.
    pub matches: Vec<&'static Command>,
    /// Index into `matches`.
    pub cursor: usize,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            query: String::new(),
            matches: commands::search(""),
            cursor: 0,
        }
    }
}

impl Palette {
    /// Applies a key. Returns the command to run on Enter; `Close` and a
    /// run both mean the caller closes the palette.
    pub fn apply(&mut self, input: Input) -> Option<&'static Command> {
        match input {
            Input::Type(text) => {
                self.query.push_str(&text);
                self.refilter();
            }
            Input::Backspace => {
                self.query.pop();
                self.refilter();
            }
            Input::Up => self.cursor = self.cursor.saturating_sub(1),
            Input::Down => {
                self.cursor = (self.cursor + 1).min(self.matches.len().saturating_sub(1));
            }
            Input::Run => return self.matches.get(self.cursor).copied(),
            Input::Close => {}
        }
        None
    }

    fn refilter(&mut self) {
        self.matches = commands::search(&self.query);
        self.cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(palette: &mut Palette, text: &str) {
        for c in text.chars() {
            palette.apply(Input::Type(c.to_string()));
        }
    }

    #[test]
    fn typing_filters_and_returns_to_the_top() {
        let mut palette = Palette::default();
        assert_eq!(palette.matches.len(), commands::ALL.len());
        palette.apply(Input::Down);
        palette.apply(Input::Down);
        typed(&mut palette, "art");
        assert_eq!(palette.cursor, 0);
        assert!(palette
            .matches
            .iter()
            .all(|c| commands::score("art", c.name).is_some()));
        assert!(palette.matches.len() < commands::ALL.len());
    }

    #[test]
    fn the_cursor_stops_at_both_ends() {
        let mut palette = Palette::default();
        palette.apply(Input::Up);
        assert_eq!(palette.cursor, 0);
        for _ in 0..50 {
            palette.apply(Input::Down);
        }
        assert_eq!(palette.cursor, commands::ALL.len() - 1);
        typed(&mut palette, "zzz");
        palette.apply(Input::Down);
        assert_eq!((palette.cursor, palette.apply(Input::Run)), (0, None));
    }

    #[test]
    fn enter_runs_the_highlighted_command() {
        let mut palette = Palette::default();
        typed(&mut palette, "quit");
        assert_eq!(palette.apply(Input::Run).map(|c| c.name), Some("Quit"));
    }

    #[test]
    fn backspace_on_nothing_stays_empty() {
        let mut palette = Palette::default();
        palette.apply(Input::Backspace);
        assert_eq!(palette, Palette::default());
        typed(&mut palette, "q");
        palette.apply(Input::Backspace);
        assert_eq!(palette, Palette::default());
    }

    #[test]
    fn keys_type_text_and_move_the_cursor() {
        let none = Modifiers::empty();
        let ch = |c: &str| Key::Character(c.into());
        // j types a j here; it doesn't move the list.
        assert_eq!(
            input_for(&ch("j"), none, Some("j")),
            Some(Input::Type("j".into()))
        );
        assert_eq!(
            input_for(&ch("n"), Modifiers::CTRL, Some("n")),
            Some(Input::Down)
        );
        assert_eq!(
            input_for(&Key::Named(Named::Escape), none, Some("\u{1b}")),
            Some(Input::Close)
        );
        assert_eq!(
            input_for(&Key::Named(Named::Space), none, Some(" ")),
            Some(Input::Type(" ".into()))
        );
        assert_eq!(input_for(&Key::Named(Named::Shift), none, None), None);
    }
}
