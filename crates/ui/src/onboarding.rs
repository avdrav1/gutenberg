//! The Add sources screen: categories of catalog feeds to check and add,
//! with keyword presets and MCP servers shown for later. State and keys
//! are pure, so they can be tested without a window.

use std::collections::HashSet;

use gutenberg_core::catalog::{Catalog, McpTransport, Paywall};
use gutenberg_theme::{Rgb, Tokens};
use iced::keyboard::{Key, Modifiers};
use iced::widget::{checkbox, column, container, mouse_area, scrollable, text, Column, Row, Space};
use iced::{border, font, Background, Border, Color, Element, Font, Length};

use crate::{color, Message};

/// What a key or click does on the screen. Keys are listed in `docs/KEYS.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Down,
    Up,
    NextCategory,
    PrevCategory,
    ToggleCursor,
    ToggleAll,
    Add,
    Close,
    Quit,
    ShowCategory(usize),
    /// A click on the row at this index of `entries`.
    ToggleEntry(usize),
}

/// One row of a category; indices into the catalog's vecs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Feed(usize),
    Keyword(usize),
    Mcp(usize),
}

pub struct Onboarding {
    /// Index into the catalog's categories; one past the end is MCP servers.
    pub category: usize,
    /// Index into `entries` of the category.
    pub cursor: usize,
    /// Checked feeds, as indices into `catalog.feeds`.
    pub checked: HashSet<usize>,
    /// Feed URLs the store already has.
    pub known: HashSet<String>,
}

/// The rows of a category: its feeds, then its keyword presets. The last
/// category, MCP servers, lists every server.
pub fn entries(catalog: &Catalog, category: usize) -> Vec<Entry> {
    match catalog.categories.get(category) {
        Some(c) => catalog
            .feeds_in(&c.id)
            .map(|(i, _)| Entry::Feed(i))
            .chain(catalog.keywords_in(&c.id).map(|(i, _)| Entry::Keyword(i)))
            .collect(),
        None => (0..catalog.mcp.len()).map(Entry::Mcp).collect(),
    }
}

pub fn action_for(key: &Key, modifiers: Modifiers) -> Option<Action> {
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    use iced::keyboard::key::Named;
    match key.as_ref() {
        Key::Character("j") | Key::Named(Named::ArrowDown) => Some(Action::Down),
        Key::Character("k") | Key::Named(Named::ArrowUp) => Some(Action::Up),
        Key::Named(Named::Tab) if modifiers.shift() => Some(Action::PrevCategory),
        Key::Character("l") | Key::Named(Named::ArrowRight | Named::Tab) => {
            Some(Action::NextCategory)
        }
        Key::Character("h") | Key::Named(Named::ArrowLeft) => Some(Action::PrevCategory),
        Key::Named(Named::Space) => Some(Action::ToggleCursor),
        Key::Character("a") => Some(Action::ToggleAll),
        Key::Named(Named::Enter) => Some(Action::Add),
        Key::Named(Named::Escape) => Some(Action::Close),
        Key::Character("q") => Some(Action::Quit),
        _ => None,
    }
}

impl Onboarding {
    /// On first run the starter feeds the store doesn't have are checked.
    pub fn new(catalog: &Catalog, known: HashSet<String>, first_run: bool) -> Self {
        let checked = if first_run {
            catalog
                .feeds
                .iter()
                .enumerate()
                .filter(|(_, f)| f.starter && !known.contains(&f.url))
                .map(|(i, _)| i)
                .collect()
        } else {
            HashSet::new()
        };
        Self {
            category: 0,
            cursor: 0,
            checked,
            known,
        }
    }

    /// Everything but `Add`, `Close` and `Quit`, which the app handles.
    pub fn apply(&mut self, catalog: &Catalog, action: Action) {
        let last_category = catalog.categories.len();
        match action {
            Action::Down => {
                let len = entries(catalog, self.category).len();
                self.cursor = (self.cursor + 1).min(len.saturating_sub(1));
            }
            Action::Up => self.cursor = self.cursor.saturating_sub(1),
            Action::NextCategory => self.show(self.category + 1, last_category),
            Action::PrevCategory => self.show(self.category.saturating_sub(1), last_category),
            Action::ShowCategory(i) => self.show(i, last_category),
            Action::ToggleCursor => self.toggle(catalog),
            Action::ToggleEntry(i) => {
                self.cursor = i;
                self.toggle(catalog);
            }
            Action::ToggleAll => {
                let addable: Vec<usize> = entries(catalog, self.category)
                    .into_iter()
                    .filter_map(|e| match e {
                        Entry::Feed(i) if self.addable(catalog, i) => Some(i),
                        _ => None,
                    })
                    .collect();
                if addable.iter().all(|i| self.checked.contains(i)) {
                    for i in &addable {
                        self.checked.remove(i);
                    }
                } else {
                    self.checked.extend(addable);
                }
            }
            Action::Add | Action::Close | Action::Quit => {}
        }
    }

    /// Checked feeds, in catalog order.
    pub fn picks(&self) -> Vec<usize> {
        let mut picks: Vec<usize> = self.checked.iter().copied().collect();
        picks.sort_unstable();
        picks
    }

    /// Checked feeds in a category.
    pub fn checked_in(&self, catalog: &Catalog, category: usize) -> usize {
        let Some(c) = catalog.categories.get(category) else {
            return 0;
        };
        catalog
            .feeds_in(&c.id)
            .filter(|(i, _)| self.checked.contains(i))
            .count()
    }

    fn show(&mut self, category: usize, last: usize) {
        self.category = category.min(last);
        self.cursor = 0;
    }

    fn addable(&self, catalog: &Catalog, feed: usize) -> bool {
        !self.known.contains(&catalog.feeds[feed].url)
    }

    fn toggle(&mut self, catalog: &Catalog) {
        let Some(Entry::Feed(i)) = entries(catalog, self.category).get(self.cursor).copied() else {
            return;
        };
        if self.addable(catalog, i) && !self.checked.remove(&i) {
            self.checked.insert(i);
        }
    }
}

pub fn view<'a>(state: &'a Onboarding, catalog: &'a Catalog, t: &Tokens) -> Element<'a, Message> {
    let header = container(
        column![
            text("Add sources").size(22).color(color(t.foreground)),
            text("Pick what to read. Space checks, a checks a whole category, Enter adds, Esc skips.")
                .size(13)
                .color(color(t.muted)),
        ]
        .spacing(6),
    )
    .padding(24)
    .width(Length::Fill)
    .style(filled(t.background));

    let body = Row::new()
        .push(categories_view(state, catalog, t))
        .push(entries_view(state, catalog, t))
        .height(Length::Fill);

    column![header, body, line(t.border), footer(state, t)]
        .height(Length::Fill)
        .into()
}

fn categories_view<'a>(
    state: &'a Onboarding,
    catalog: &'a Catalog,
    t: &Tokens,
) -> Element<'a, Message> {
    let names = catalog
        .categories
        .iter()
        .map(|c| c.name.as_str())
        .chain(["MCP servers"]);
    let rows = names.enumerate().map(|(i, name)| {
        let n = state.checked_in(catalog, i);
        let count = text(if n > 0 { n.to_string() } else { String::new() })
            .size(12)
            .color(color(t.muted));
        let label = text(name)
            .size(14)
            .color(color(t.foreground))
            .wrapping(text::Wrapping::None)
            .width(Length::Fill);
        let background = (i == state.category).then(|| color(t.selection).into());
        let row = container(Row::new().push(label).push(count).spacing(8))
            .padding([6, 12])
            .width(Length::Fill)
            .clip(true)
            .style(move |_| container::Style {
                background,
                ..Default::default()
            });
        mouse_area(row)
            .on_press(Message::Onboarding(Action::ShowCategory(i)))
            .into()
    });
    container(scrollable(Column::with_children(rows).padding([8, 0])))
        .width(Length::Fixed(220.0))
        .height(Length::Fill)
        .style(filled(t.background))
        .into()
}

fn entries_view<'a>(
    state: &'a Onboarding,
    catalog: &'a Catalog,
    t: &Tokens,
) -> Element<'a, Message> {
    let name = catalog
        .categories
        .get(state.category)
        .map_or("MCP servers", |c| c.name.as_str());
    let mut list = Column::new()
        .push(
            container(text(name).size(18).color(color(t.foreground))).padding(iced::Padding {
                bottom: 8.0,
                ..iced::Padding::ZERO
            }),
        )
        .spacing(2)
        .padding(24)
        .width(Length::Fill);
    let mut keywords_started = false;
    for (i, entry) in entries(catalog, state.category).into_iter().enumerate() {
        let at_cursor = i == state.cursor;
        let row = match entry {
            Entry::Feed(f) => feed_row(state, catalog, f, i, at_cursor, t),
            Entry::Keyword(k) => {
                if !keywords_started {
                    keywords_started = true;
                    list = list.push(section("Keyword alerts · coming with keyword alerts", t));
                }
                let k = &catalog.keywords[k];
                let lines = column![
                    text(&k.title).size(14).color(color(t.muted)),
                    text(&k.query).size(12).color(color(t.muted)),
                ]
                .spacing(2);
                cursor_row(lines.into(), at_cursor, t)
            }
            Entry::Mcp(m) => {
                if i == 0 {
                    list = list.push(section("MCP servers · coming with MCP sources", t));
                }
                mcp_row(catalog, m, at_cursor, t)
            }
        };
        list = list.push(row);
    }
    container(scrollable(list))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(filled(t.lighter_background))
        .into()
}

fn feed_row<'a>(
    state: &Onboarding,
    catalog: &'a Catalog,
    feed: usize,
    index: usize,
    at_cursor: bool,
    t: &Tokens,
) -> Element<'a, Message> {
    let f = &catalog.feeds[feed];
    let known = state.known.contains(&f.url);
    let checked = known || state.checked.contains(&feed);
    let (accent, background, border_color) =
        (color(t.accent), color(t.background), color(t.border));
    // No `on_toggle`: it draws as disabled and leaves the click to the row.
    let check = checkbox(checked).size(16).style(move |_, _| {
        if checked {
            checkbox::Style {
                background: Background::Color(accent),
                icon_color: background,
                border: Border {
                    color: accent,
                    width: 1.0,
                    radius: border::radius(3),
                },
                text_color: None,
            }
        } else {
            checkbox::Style {
                background: Background::Color(Color::TRANSPARENT),
                icon_color: background,
                border: Border {
                    color: border_color,
                    width: 1.0,
                    radius: border::radius(3),
                },
                text_color: None,
            }
        }
    });
    let mut title = Row::new()
        .push(
            text(&f.title)
                .size(15)
                .color(color(if known { t.muted } else { t.foreground }))
                .font(Font {
                    weight: font::Weight::Bold,
                    ..Font::DEFAULT
                })
                .wrapping(text::Wrapping::None),
        )
        .spacing(10)
        .align_y(iced::Alignment::Center);
    let tags = [
        known.then_some("added"),
        match f.paywall {
            Paywall::None => None,
            Paywall::Metered => Some("metered paywall"),
            Paywall::Hard => Some("paywall"),
        },
    ];
    for tag in tags.into_iter().flatten() {
        title = title.push(text(tag).size(12).color(color(t.muted)));
    }
    let detail = text(format!("{} · {}", f.blurb, host(&f.site)))
        .size(12)
        .color(color(t.muted))
        .wrapping(text::Wrapping::None);
    let content = Row::new()
        .push(check)
        .push(column![title, detail].spacing(4))
        .spacing(12)
        .align_y(iced::Alignment::Center);
    mouse_area(cursor_row(content.into(), at_cursor, t))
        .on_press(Message::Onboarding(Action::ToggleEntry(index)))
        .into()
}

fn mcp_row<'a>(
    catalog: &'a Catalog,
    m: usize,
    at_cursor: bool,
    t: &Tokens,
) -> Element<'a, Message> {
    let m = &catalog.mcp[m];
    let muted = color(t.muted);
    let how = match m.transport {
        McpTransport::Stdio => std::iter::once(m.command.as_str())
            .chain(m.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        McpTransport::Http => m.url.clone(),
    };
    let mut lines = column![
        text(&m.title).size(15).color(muted),
        text(&m.blurb).size(12).color(muted),
        text(how)
            .size(12)
            .color(muted)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(2);
    if !m.secret.is_empty() {
        lines = lines.push(text(format!("needs {}", m.secret)).size(12).color(muted));
    }
    lines = lines.push(text(&m.maintainer).size(12).color(muted));
    cursor_row(lines.into(), at_cursor, t)
}

/// A row with the cursor's background when it's on it.
fn cursor_row<'a>(
    content: Element<'a, Message>,
    at_cursor: bool,
    t: &Tokens,
) -> Element<'a, Message> {
    let background = at_cursor.then(|| color(t.selection).into());
    container(content)
        .padding([8, 12])
        .width(Length::Fill)
        .clip(true)
        .style(move |_| container::Style {
            background,
            ..Default::default()
        })
        .into()
}

fn section<'a>(title: &'static str, t: &Tokens) -> Element<'a, Message> {
    container(text(title).size(13).color(color(t.muted)))
        .padding(iced::Padding {
            top: 16.0,
            right: 12.0,
            bottom: 4.0,
            left: 12.0,
        })
        .into()
}

fn footer<'a>(state: &Onboarding, t: &Tokens) -> Element<'a, Message> {
    let n = state.checked.len();
    let status = match n {
        0 => "Nothing selected".to_owned(),
        1 => "1 feed selected".to_owned(),
        n => format!("{n} feeds selected"),
    };
    let add_label = match n {
        0 => "Add feeds".to_owned(),
        1 => "Add 1 feed".to_owned(),
        n => format!("Add {n} feeds"),
    };
    let (foreground, background, accent, border_color) = (
        color(t.foreground),
        color(t.background),
        color(t.accent),
        color(t.border),
    );
    let skip = container(text("Skip").size(14).color(foreground))
        .padding([6, 14])
        .style(move |_| container::Style {
            border: Border {
                color: border_color,
                width: 1.0,
                radius: border::radius(4),
            },
            ..Default::default()
        });
    let add = container(text(add_label).size(14).color(background))
        .padding([6, 14])
        .style(move |_| container::Style {
            background: Some(accent.into()),
            border: Border {
                color: accent,
                width: 1.0,
                radius: border::radius(4),
            },
            ..Default::default()
        });
    container(
        Row::new()
            .push(text(status).size(13).color(color(t.muted)))
            .push(Space::new().width(Length::Fill))
            .push(mouse_area(skip).on_press(Message::Onboarding(Action::Close)))
            .push(mouse_area(add).on_press(Message::Onboarding(Action::Add)))
            .spacing(12)
            .align_y(iced::Alignment::Center),
    )
    .padding([12, 24])
    .width(Length::Fill)
    .style(filled(t.background))
    .into()
}

/// A one-pixel rule.
fn line<'a>(c: Rgb) -> Element<'a, Message> {
    container(Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(filled(c))
        .into()
}

fn filled(c: Rgb) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color(c).into()),
        ..Default::default()
    }
}

/// `https://www.bbc.co.uk/news/world` → `bbc.co.uk`.
fn host(site: &str) -> &str {
    let rest = site.strip_prefix("https://").unwrap_or(site);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    rest.split('/').next().unwrap_or(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gutenberg_core::catalog;
    use iced::keyboard::key::Named;

    #[test]
    fn first_run_checks_the_starters_the_store_lacks() {
        let catalog = catalog::builtin();
        let starters: Vec<usize> = (0..catalog.feeds.len())
            .filter(|&i| catalog.feeds[i].starter)
            .collect();
        assert_eq!(
            Onboarding::new(catalog, HashSet::new(), true).picks(),
            starters
        );

        let known = HashSet::from([catalog.feeds[starters[0]].url.clone()]);
        assert_eq!(Onboarding::new(catalog, known, true).picks(), starters[1..]);
        assert!(Onboarding::new(catalog, HashSet::new(), false)
            .picks()
            .is_empty());
    }

    #[test]
    fn known_feeds_and_keyword_rows_do_not_toggle() {
        let catalog = catalog::builtin();
        let rows = entries(catalog, 0);
        let Entry::Feed(first) = rows[0] else {
            panic!("a category starts with feeds")
        };
        let known = HashSet::from([catalog.feeds[first].url.clone()]);
        let mut state = Onboarding::new(catalog, known, false);
        state.apply(catalog, Action::ToggleCursor);
        assert!(state.picks().is_empty());

        let keyword = rows
            .iter()
            .position(|e| matches!(e, Entry::Keyword(_)))
            .expect("World has keyword presets");
        state.apply(catalog, Action::ToggleEntry(keyword));
        assert_eq!(state.cursor, keyword);
        assert!(state.picks().is_empty());

        state.apply(catalog, Action::ToggleEntry(1));
        assert_eq!(state.picks().len(), 1);
        state.apply(catalog, Action::ToggleCursor);
        assert!(state.picks().is_empty());
    }

    #[test]
    fn toggle_all_twice_checks_then_clears_the_category() {
        let catalog = catalog::builtin();
        let mut state = Onboarding::new(catalog, HashSet::new(), false);
        state.apply(catalog, Action::ToggleAll);
        let world = catalog.feeds_in(&catalog.categories[0].id).count();
        assert_eq!(state.checked_in(catalog, 0), world);
        assert_eq!(state.picks().len(), world);
        state.apply(catalog, Action::ToggleAll);
        assert!(state.picks().is_empty());

        // With some checked, it checks the rest first.
        let mut state = Onboarding::new(catalog, HashSet::new(), true);
        let before = state.checked_in(catalog, 0);
        assert!(before > 0 && before < world);
        state.apply(catalog, Action::ToggleAll);
        assert_eq!(state.checked_in(catalog, 0), world);
    }

    #[test]
    fn categories_clamp_at_both_ends() {
        let catalog = catalog::builtin();
        let mcp = catalog.categories.len();
        let mut state = Onboarding::new(catalog, HashSet::new(), false);
        state.apply(catalog, Action::PrevCategory);
        assert_eq!(state.category, 0);
        state.apply(catalog, Action::Down);
        state.apply(catalog, Action::ShowCategory(usize::MAX));
        assert_eq!((state.category, state.cursor), (mcp, 0));
        state.apply(catalog, Action::NextCategory);
        assert_eq!(state.category, mcp);
        for _ in 0..100 {
            state.apply(catalog, Action::Down);
        }
        assert_eq!(state.cursor, catalog.mcp.len() - 1);
        // Nothing to check among MCP servers.
        state.apply(catalog, Action::ToggleAll);
        state.apply(catalog, Action::ToggleCursor);
        assert!(state.picks().is_empty());
    }

    #[test]
    fn keys() {
        let none = Modifiers::empty();
        let named = |n| Key::Named(n);
        assert_eq!(
            action_for(&named(Named::Space), none),
            Some(Action::ToggleCursor)
        );
        assert_eq!(
            action_for(&named(Named::Tab), none),
            Some(Action::NextCategory)
        );
        assert_eq!(
            action_for(&named(Named::Tab), Modifiers::SHIFT),
            Some(Action::PrevCategory)
        );
        assert_eq!(action_for(&named(Named::Escape), none), Some(Action::Close));
        assert_eq!(action_for(&named(Named::Enter), none), Some(Action::Add));
        assert_eq!(
            action_for(&Key::Character("a".into()), Modifiers::CTRL),
            None
        );
    }
}
