//! The article list: which rows are on screen, and what keys do. Pure, so
//! it can be tested without a window.

use iced::keyboard::{Key, Modifiers};

/// Height of one row in logical pixels: a title line and a meta line.
pub const ROW_HEIGHT: f32 = 52.0;
/// Rows one notch of the mouse wheel moves.
pub const WHEEL_ROWS: f32 = 3.0;

/// What a key does to the list. Keys are listed in `docs/KEYS.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Down,
    Up,
    First,
    Last,
    Open,
    /// The first `g` of `gg`.
    PendingG,
}

/// `pending_g` is whether the last key was a lone `g`.
pub fn action_for(key: &Key, modifiers: Modifiers, pending_g: bool) -> Option<Action> {
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    use iced::keyboard::key::Named;
    match key.as_ref() {
        Key::Character("q") => Some(Action::Quit),
        Key::Character("j") | Key::Named(Named::ArrowDown) => Some(Action::Down),
        Key::Character("k") | Key::Named(Named::ArrowUp) => Some(Action::Up),
        Key::Character("g") if pending_g => Some(Action::First),
        Key::Character("g") => Some(Action::PendingG),
        Key::Character("G") => Some(Action::Last),
        Key::Character("o") | Key::Named(Named::Enter) => Some(Action::Open),
        _ => None,
    }
}

/// Rows that fit in `height`, counting a partly visible last row.
pub fn rows_that_fit(height: f32) -> usize {
    (height / ROW_HEIGHT).ceil().max(1.0) as usize
}

/// The range of rows to build: from `top`, as many as fit, within `len`.
pub fn visible(top: usize, height: f32, len: usize) -> std::ops::Range<usize> {
    let start = top.min(len);
    start..(start + rows_that_fit(height)).min(len)
}

/// The top row that keeps `selected` fully in view, moving as little as
/// possible from `top`.
pub fn scroll_to_show(top: usize, selected: usize, height: f32) -> usize {
    let whole = ((height / ROW_HEIGHT).floor() as usize).max(1);
    if selected < top {
        selected
    } else if selected >= top + whole {
        selected + 1 - whole
    } else {
        top
    }
}

/// The highest top row: the last row sits at the bottom.
pub fn max_top(len: usize, height: f32) -> usize {
    let whole = ((height / ROW_HEIGHT).floor() as usize).max(1);
    len.saturating_sub(whole)
}

/// "5m", "3h", "2d" within a week, else "Oct 3" (or "Oct 3 2025" in
/// another year). Times are unix seconds.
pub fn age(then: i64, now: i64) -> String {
    let secs = (now - then).max(0);
    match secs {
        s if s < 3600 => format!("{}m", (s / 60).max(1)),
        s if s < 86_400 => format!("{}h", s / 3600),
        s if s < 7 * 86_400 => format!("{}d", s / 86_400),
        _ => {
            const MONTHS: [&str; 12] = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            let (y, m, d) = civil(then.div_euclid(86_400));
            let (now_y, _, _) = civil(now.div_euclid(86_400));
            let month = MONTHS[(m - 1) as usize];
            if y == now_y {
                format!("{month} {d}")
            } else {
                format!("{month} {d} {y}")
            }
        }
    }
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, i64, i64) {
    // Howard Hinnant's days_from_civil, inverted.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Text of an HTML summary, for the article pane until S3.2 renders HTML.
pub fn plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut tag = String::new();
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name = tag
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                if matches!(
                    name.to_ascii_lowercase().as_str(),
                    "p" | "br" | "br/" | "div" | "li" | "h1" | "h2" | "h3" | "blockquote" | "pre"
                ) {
                    text.push('\n');
                }
            }
            _ if in_tag => tag.push(c),
            _ => text.push(c),
        }
    }
    let text = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: &str) -> Key {
        Key::Character(c.into())
    }

    #[test]
    fn vim_keys() {
        let none = Modifiers::empty();
        assert_eq!(action_for(&key("j"), none, false), Some(Action::Down));
        assert_eq!(action_for(&key("k"), none, false), Some(Action::Up));
        assert_eq!(
            action_for(&key("G"), Modifiers::SHIFT, false),
            Some(Action::Last)
        );
        assert_eq!(action_for(&key("o"), none, false), Some(Action::Open));
        assert_eq!(action_for(&key("q"), none, false), Some(Action::Quit));
        assert_eq!(action_for(&key("q"), Modifiers::CTRL, false), None);
        assert_eq!(action_for(&key("Q"), Modifiers::SHIFT, false), None);
    }

    #[test]
    fn gg_takes_two_presses() {
        let none = Modifiers::empty();
        assert_eq!(action_for(&key("g"), none, false), Some(Action::PendingG));
        assert_eq!(action_for(&key("g"), none, true), Some(Action::First));
    }

    #[test]
    fn only_rows_that_fit_are_built() {
        // A 4K tile 2136 px tall shows 42 rows (41 whole and a partial one).
        assert_eq!(visible(0, 2136.0, 10_000), 0..42);
        assert_eq!(visible(5_000, 2136.0, 10_000), 5_000..5_042);
        assert_eq!(visible(9_990, 2136.0, 10_000), 9_990..10_000);
        assert_eq!(visible(0, 2136.0, 3), 0..3);
        assert_eq!(visible(0, 2136.0, 0), 0..0);
    }

    #[test]
    fn selection_stays_in_view() {
        // 10 whole rows fit in 520 px.
        assert_eq!(scroll_to_show(0, 9, 520.0), 0);
        assert_eq!(scroll_to_show(0, 10, 520.0), 1);
        assert_eq!(scroll_to_show(50, 20, 520.0), 20);
        assert_eq!(max_top(100, 520.0), 90);
    }

    #[test]
    fn ages_read_like_a_feed_reader() {
        let now = 1_792_500_000; // 2026-10-20 12:40 UTC
        assert_eq!(age(now - 30, now), "1m");
        assert_eq!(age(now - 3 * 3600, now), "3h");
        assert_eq!(age(now - 2 * 86_400, now), "2d");
        assert_eq!(age(1_790_068_167, now), "Sep 22");
        assert_eq!(age(1_596_818_676, now), "Aug 7 2020");
    }

    #[test]
    fn summaries_become_paragraphs() {
        assert_eq!(
            plain_text("<p>One &amp; two</p><p>Three<br>four</p>"),
            "One & two\n\nThree\n\nfour"
        );
    }
}
