//! The notification history (S10.5): every notification, newest first,
//! with why it fired and what became of it, over the panes after `N`.
//! Pure, so it can be tested without a window.

use std::ops::Range;

use gutenberg_store::{Fate, Notified};

use crate::palette::Input;

/// Rows the history shows at once; the rest scroll with the cursor.
pub const ROWS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    pub entries: Vec<Notified>,
    /// The row the keys are on.
    pub cursor: usize,
    /// The first row shown.
    pub top: usize,
}

impl History {
    pub fn new(entries: Vec<Notified>) -> Self {
        Self {
            entries,
            cursor: 0,
            top: 0,
        }
    }

    /// Applies a key. Returns the article Enter opens; the caller closes
    /// the history then, and on `Close`. `j`/`k` and the arrows move, `gg`
    /// isn't needed: `g` and `G` go to the first and the last.
    pub fn apply(&mut self, input: Input) -> Option<i64> {
        match input {
            Input::Type(text) => match text.as_str() {
                "j" => self.move_to(self.cursor.saturating_add(1)),
                "k" => self.move_to(self.cursor.saturating_sub(1)),
                "g" => self.move_to(0),
                "G" => self.move_to(usize::MAX),
                _ => {}
            },
            Input::Down => self.move_to(self.cursor.saturating_add(1)),
            Input::Up => self.move_to(self.cursor.saturating_sub(1)),
            Input::Run => return self.entries.get(self.cursor).map(|n| n.article_id),
            Input::Backspace | Input::Close => {}
        }
        None
    }

    /// Whether this key closes the history without opening anything.
    pub fn closes(input: &Input) -> bool {
        match input {
            Input::Close => true,
            Input::Type(text) => matches!(text.as_str(), "N" | "q"),
            _ => false,
        }
    }

    fn move_to(&mut self, i: usize) {
        self.cursor = i.min(self.entries.len().saturating_sub(1));
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + ROWS {
            self.top = self.cursor + 1 - ROWS;
        }
    }

    /// The rows on screen.
    pub fn visible(&self) -> Range<usize> {
        self.top..(self.top + ROWS).min(self.entries.len())
    }
}

/// What became of a notification, as its row says it.
pub fn fate(fate: Fate) -> &'static str {
    match fate {
        Fate::Shown => "shown",
        Fate::Updated => "updated its pop-up",
        Fate::Summary => "in a summary",
        Fate::Held => "held for a summary",
        Fate::Seen => "seen in the window",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(n: usize) -> History {
        History::new(
            (0..n)
                .map(|i| Notified {
                    article_id: 100 + i as i64,
                    title: format!("Headline {i}"),
                    source: "LWN".into(),
                    reason: "rule: CVE".into(),
                    at: 1_000 - i as i64,
                    fate: Fate::Shown,
                })
                .collect(),
        )
    }

    fn typed(s: &str) -> Input {
        Input::Type(s.into())
    }

    #[test]
    fn enter_opens_the_row_under_the_cursor() {
        let mut h = history(3);
        assert_eq!(h.apply(Input::Run), Some(100));
        assert_eq!(h.apply(typed("j")), None);
        assert_eq!(h.apply(Input::Down), None);
        assert_eq!(h.apply(Input::Run), Some(102));
        // The last row is as far as it goes.
        h.apply(typed("j"));
        assert_eq!(h.cursor, 2);
        h.apply(typed("k"));
        h.apply(Input::Up);
        h.apply(Input::Up);
        assert_eq!(h.cursor, 0);
    }

    #[test]
    fn an_empty_history_opens_nothing() {
        let mut h = history(0);
        h.apply(typed("j"));
        h.apply(typed("G"));
        assert_eq!(h.cursor, 0);
        assert_eq!(h.apply(Input::Run), None);
        assert_eq!(h.visible(), 0..0);
    }

    #[test]
    fn a_long_history_scrolls_with_the_cursor() {
        let mut h = history(30);
        assert_eq!(h.visible(), 0..ROWS);
        for _ in 0..ROWS {
            h.apply(typed("j"));
        }
        assert_eq!(h.cursor, ROWS);
        assert_eq!(h.visible(), 1..ROWS + 1);
        h.apply(typed("G"));
        assert_eq!(h.visible(), 30 - ROWS..30);
        h.apply(typed("g"));
        assert_eq!((h.cursor, h.visible()), (0, 0..ROWS));
    }

    #[test]
    fn esc_n_and_q_close_it() {
        assert!(History::closes(&Input::Close));
        assert!(History::closes(&typed("N")));
        assert!(History::closes(&typed("q")));
        assert!(!History::closes(&typed("j")));
        assert!(!History::closes(&Input::Run));
    }

    #[test]
    fn every_fate_reads_as_words() {
        for f in [
            Fate::Shown,
            Fate::Updated,
            Fate::Summary,
            Fate::Held,
            Fate::Seen,
        ] {
            assert!(!fate(f).is_empty());
        }
    }
}
