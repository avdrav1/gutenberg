//! The Add sources screen: categories of catalog feeds and keyword alert
//! presets to check and add, a page for typing keyword alerts of your own,
//! and MCP servers shown for later. State and keys are pure, so they can be
//! tested without a window.

use std::collections::HashSet;

use gutenberg_core::catalog::{Catalog, McpTransport, Paywall, Picks};
use gutenberg_core::keyword::Keyword;
use gutenberg_theme::{Rgb, Tokens};
use iced::keyboard::{Key, Modifiers};
use iced::widget::{checkbox, column, container, mouse_area, scrollable, text, Column, Row, Space};
use iced::{border, font, Background, Border, Color, Element, Font, Length};

use crate::{color, palette, Message};

/// What a key or click does on the screen. Keys are listed in `docs/KEYS.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Down,
    Up,
    NextCategory,
    PrevCategory,
    ToggleCursor,
    ToggleAll,
    /// Go to the keyword alerts page and start typing a query.
    NewQuery,
    Add,
    Close,
    Quit,
    ShowCategory(usize),
    /// A click on the row at this index of `entries`.
    ToggleEntry(usize),
}

/// One row of a page; indices into the catalog's vecs, or into `queries`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Feed(usize),
    Keyword(usize),
    /// The field a query is typed into.
    NewQuery,
    Query(usize),
    Mcp(usize),
}

pub struct Onboarding {
    /// Index into the catalog's categories; one past the end is the keyword
    /// alerts page, and one past that is MCP servers.
    pub category: usize,
    /// Index into `entries` of the page.
    pub cursor: usize,
    /// Checked feeds, as indices into `catalog.feeds`.
    pub checked: HashSet<usize>,
    /// Checked presets, as indices into `catalog.keywords`.
    pub checked_keywords: HashSet<usize>,
    /// Queries typed in, and whether each is checked.
    pub queries: Vec<(String, bool)>,
    /// The query being typed; `None` when not typing.
    pub draft: Option<String>,
    /// Feed URLs the store already has.
    pub known: HashSet<String>,
    /// Keyword alert queries the store already has.
    pub known_queries: HashSet<String>,
    /// Presets whose query the store already has.
    known_presets: HashSet<usize>,
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
        Key::Character("n") => Some(Action::NewQuery),
        Key::Named(Named::Enter) => Some(Action::Add),
        Key::Named(Named::Escape) => Some(Action::Close),
        Key::Character("q") => Some(Action::Quit),
        _ => None,
    }
}

impl Onboarding {
    /// On first run the starter feeds the store doesn't have are checked.
    pub fn new(
        catalog: &Catalog,
        known: HashSet<String>,
        known_queries: HashSet<String>,
        first_run: bool,
    ) -> Self {
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
        let known_presets = catalog
            .keywords
            .iter()
            .enumerate()
            .filter(|(_, k)| {
                Keyword::new(&k.query).is_some_and(|k| known_queries.contains(&k.query))
            })
            .map(|(i, _)| i)
            .collect();
        Self {
            category: 0,
            cursor: 0,
            checked,
            checked_keywords: HashSet::new(),
            queries: Vec::new(),
            draft: None,
            known,
            known_queries,
            known_presets,
        }
    }

    /// The keyword alerts page.
    pub fn keywords_page(catalog: &Catalog) -> usize {
        catalog.categories.len()
    }

    /// The rows of the page: a category's feeds, then its keyword presets;
    /// on the keyword alerts page the query field, the typed queries and
    /// every preset; on the last page every MCP server.
    pub fn entries(&self, catalog: &Catalog) -> Vec<Entry> {
        let keywords_page = Self::keywords_page(catalog);
        match catalog.categories.get(self.category) {
            Some(c) => catalog
                .feeds_in(&c.id)
                .map(|(i, _)| Entry::Feed(i))
                .chain(catalog.keywords_in(&c.id).map(|(i, _)| Entry::Keyword(i)))
                .collect(),
            None if self.category == keywords_page => std::iter::once(Entry::NewQuery)
                .chain((0..self.queries.len()).map(Entry::Query))
                .chain((0..catalog.keywords.len()).map(Entry::Keyword))
                .collect(),
            None => (0..catalog.mcp.len()).map(Entry::Mcp).collect(),
        }
    }

    /// Everything but `Add`, `Close` and `Quit`, which the app handles.
    pub fn apply(&mut self, catalog: &Catalog, action: Action) {
        let last = Self::keywords_page(catalog) + 1;
        match action {
            Action::Down => {
                let len = self.entries(catalog).len();
                self.cursor = (self.cursor + 1).min(len.saturating_sub(1));
            }
            Action::Up => self.cursor = self.cursor.saturating_sub(1),
            Action::NextCategory => self.show(self.category + 1, last),
            Action::PrevCategory => self.show(self.category.saturating_sub(1), last),
            Action::ShowCategory(i) => self.show(i, last),
            Action::ToggleCursor => self.toggle(catalog),
            Action::ToggleEntry(i) => {
                self.cursor = i;
                self.toggle(catalog);
            }
            Action::NewQuery => {
                self.show(Self::keywords_page(catalog), last);
                self.draft = Some(String::new());
            }
            Action::ToggleAll => {
                let addable: Vec<Entry> = self
                    .entries(catalog)
                    .into_iter()
                    .filter(|&e| self.addable(catalog, e))
                    .collect();
                let check = !addable.iter().all(|&e| self.is_checked(catalog, e));
                for e in addable {
                    self.set_checked(e, check);
                }
            }
            Action::Add | Action::Close | Action::Quit => {}
        }
    }

    /// A key while typing a query. Enter checks it: a preset's query checks
    /// the preset, one the store has is left alone, anything else joins the
    /// typed queries. Esc stops typing and drops the query.
    pub fn type_query(&mut self, catalog: &Catalog, input: palette::Input) {
        let Some(draft) = &mut self.draft else {
            return;
        };
        match input {
            palette::Input::Type(text) => draft.push_str(&text),
            palette::Input::Backspace => {
                draft.pop();
            }
            palette::Input::Close => self.draft = None,
            palette::Input::Run => {
                let typed = self.draft.take().and_then(|d| Keyword::new(&d));
                if let Some(Keyword { query }) = typed {
                    self.check_query(catalog, query);
                }
            }
            palette::Input::Up | palette::Input::Down => {}
        }
    }

    fn check_query(&mut self, catalog: &Catalog, query: String) {
        if self.known_queries.contains(&query) {
            return;
        }
        let preset = catalog
            .keywords
            .iter()
            .position(|k| Keyword::new(&k.query).is_some_and(|k| k.query == query));
        if let Some(preset) = preset {
            self.checked_keywords.insert(preset);
        } else if let Some(typed) = self.queries.iter_mut().find(|(q, _)| *q == query) {
            typed.1 = true;
        } else {
            self.queries.push((query, true));
        }
    }

    /// Everything checked: feeds and presets in catalog order, typed queries
    /// in the order they were typed.
    pub fn picks(&self) -> Picks {
        let sorted = |set: &HashSet<usize>| {
            let mut picks: Vec<usize> = set.iter().copied().collect();
            picks.sort_unstable();
            picks
        };
        Picks {
            feeds: sorted(&self.checked),
            keywords: sorted(&self.checked_keywords),
            queries: self
                .queries
                .iter()
                .filter(|(_, checked)| *checked)
                .map(|(q, _)| q.clone())
                .collect(),
        }
    }

    /// Checked rows on a page, for the count beside its name.
    pub fn checked_in(&self, catalog: &Catalog, category: usize) -> usize {
        if let Some(c) = catalog.categories.get(category) {
            let feeds = catalog
                .feeds_in(&c.id)
                .filter(|(i, _)| self.checked.contains(i));
            let presets = catalog
                .keywords_in(&c.id)
                .filter(|(i, _)| self.checked_keywords.contains(i));
            feeds.count() + presets.count()
        } else if category == Self::keywords_page(catalog) {
            self.checked_keywords.len() + self.queries.iter().filter(|(_, c)| *c).count()
        } else {
            0
        }
    }

    /// Whether the store already has what a row adds.
    pub fn is_known(&self, catalog: &Catalog, entry: Entry) -> bool {
        match entry {
            Entry::Feed(i) => self.known.contains(&catalog.feeds[i].url),
            Entry::Keyword(i) => self.known_presets.contains(&i),
            Entry::NewQuery | Entry::Query(_) | Entry::Mcp(_) => false,
        }
    }

    /// Checked, or already in the store.
    pub fn is_checked(&self, catalog: &Catalog, entry: Entry) -> bool {
        self.is_known(catalog, entry)
            || match entry {
                Entry::Feed(i) => self.checked.contains(&i),
                Entry::Keyword(i) => self.checked_keywords.contains(&i),
                Entry::Query(i) => self.queries.get(i).is_some_and(|(_, c)| *c),
                Entry::NewQuery | Entry::Mcp(_) => false,
            }
    }

    fn show(&mut self, category: usize, last: usize) {
        self.category = category.min(last);
        self.cursor = 0;
        self.draft = None;
    }

    /// Rows that can be checked: not MCP, not the field, not in the store.
    fn addable(&self, catalog: &Catalog, entry: Entry) -> bool {
        matches!(entry, Entry::Feed(_) | Entry::Keyword(_) | Entry::Query(_))
            && !self.is_known(catalog, entry)
    }

    fn set_checked(&mut self, entry: Entry, on: bool) {
        let flip = |set: &mut HashSet<usize>, i: usize| {
            if on {
                set.insert(i);
            } else {
                set.remove(&i);
            }
        };
        match entry {
            Entry::Feed(i) => flip(&mut self.checked, i),
            Entry::Keyword(i) => flip(&mut self.checked_keywords, i),
            Entry::Query(i) => {
                if let Some(typed) = self.queries.get_mut(i) {
                    typed.1 = on;
                }
            }
            Entry::NewQuery | Entry::Mcp(_) => {}
        }
    }

    fn toggle(&mut self, catalog: &Catalog) {
        let Some(entry) = self.entries(catalog).get(self.cursor).copied() else {
            return;
        };
        if entry == Entry::NewQuery {
            self.draft = Some(String::new());
        } else if self.addable(catalog, entry) {
            let on = !self.is_checked(catalog, entry);
            self.set_checked(entry, on);
        }
    }
}

pub fn view<'a>(state: &'a Onboarding, catalog: &'a Catalog, t: &Tokens) -> Element<'a, Message> {
    let header = container(
        column![
            text("Add sources").size(22).color(color(t.foreground)),
            text(
                "Pick what to read. Space checks, a checks a whole page, n types a keyword alert, \
                 Enter adds, Esc skips."
            )
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

/// The name of a page in the left column.
fn page_name(catalog: &Catalog, page: usize) -> &str {
    match catalog.categories.get(page) {
        Some(c) => &c.name,
        None if page == Onboarding::keywords_page(catalog) => "Keyword alerts",
        None => "MCP servers",
    }
}

/// The pages: catalog categories, which scroll, then keyword alerts and MCP
/// servers pinned below them, so they show however many categories there
/// are.
fn categories_view<'a>(
    state: &'a Onboarding,
    catalog: &'a Catalog,
    t: &Tokens,
) -> Element<'a, Message> {
    let page = |i: usize| -> Element<'a, Message> {
        let n = state.checked_in(catalog, i);
        let count = text(if n > 0 { n.to_string() } else { String::new() })
            .size(12)
            .color(color(t.muted));
        let label = text(page_name(catalog, i))
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
    };
    let keywords = Onboarding::keywords_page(catalog);
    let categories = Column::with_children((0..keywords).map(page)).padding([8, 0]);
    let pinned = Column::new()
        .push(page(keywords))
        .push(page(keywords + 1))
        .padding([8, 0]);
    let column = Column::new()
        .push(scrollable(categories).height(Length::Fill))
        .push(line(t.border))
        .push(pinned);
    container(column)
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
    let name = page_name(catalog, state.category);
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
    let keywords_page = state.category == Onboarding::keywords_page(catalog);
    let mut presets_started = false;
    for (i, entry) in state.entries(catalog).into_iter().enumerate() {
        let at_cursor = i == state.cursor;
        let checked = state.is_checked(catalog, entry);
        let known = state.is_known(catalog, entry);
        let row = match entry {
            Entry::Feed(f) => {
                let f = &catalog.feeds[f];
                let paywall = match f.paywall {
                    Paywall::None => None,
                    Paywall::Metered => Some("metered paywall"),
                    Paywall::Hard => Some("paywall"),
                };
                let detail = format!("{} · {}", f.blurb, host(&f.site));
                let tags = [known.then_some("added"), paywall];
                check_row(checked, known, &f.title, tags, detail, i, at_cursor, t)
            }
            Entry::Keyword(k) => {
                if !presets_started {
                    presets_started = true;
                    let title = if keywords_page {
                        "Presets"
                    } else {
                        "Keyword alerts"
                    };
                    list = list.push(section(title, t));
                }
                let k = &catalog.keywords[k];
                let detail = if keywords_page {
                    let category = catalog.categories.iter().find(|c| c.id == k.category);
                    format!("{} · {}", k.query, category.map_or("", |c| c.name.as_str()))
                } else {
                    k.query.clone()
                };
                let tags = [known.then_some("added"), None];
                check_row(checked, known, &k.title, tags, detail, i, at_cursor, t)
            }
            Entry::NewQuery => query_field(state, i, at_cursor, t),
            Entry::Query(q) => {
                let detail = "Google News search".to_owned();
                let query = &state.queries[q].0;
                check_row(checked, false, query, [None, None], detail, i, at_cursor, t)
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

/// Where a keyword alert is typed: what's typed so far with a cursor, or
/// how to start.
fn query_field<'a>(
    state: &Onboarding,
    index: usize,
    at_cursor: bool,
    t: &Tokens,
) -> Element<'a, Message> {
    let (line, line_color) = match &state.draft {
        Some(draft) => (format!("{draft}▏"), t.foreground),
        None => (
            "Type a news search, like \"Cory Booker\" or climate -opinion".to_owned(),
            t.muted,
        ),
    };
    let hint = if state.draft.is_some() {
        "Enter checks it · Esc stops typing"
    } else {
        "n or Space to type"
    };
    let (background, border_color) = (color(t.background), color(t.border));
    let field = container(
        text(line)
            .size(15)
            .color(color(line_color))
            .wrapping(text::Wrapping::None),
    )
    .padding([6, 10])
    .width(Length::Fill)
    .clip(true)
    .style(move |_| container::Style {
        background: Some(background.into()),
        border: Border {
            color: border_color,
            width: 1.0,
            radius: border::radius(4),
        },
        ..Default::default()
    });
    let content = column![field, text(hint).size(12).color(color(t.muted)),].spacing(4);
    mouse_area(cursor_row(content.into(), at_cursor, t))
        .on_press(Message::Onboarding(Action::ToggleEntry(index)))
        .into()
}

/// A checkable row: a box, a bold title with tags, and a line of detail.
/// A click toggles it.
#[allow(clippy::too_many_arguments, reason = "one row's parts, all drawn")]
fn check_row<'a>(
    checked: bool,
    known: bool,
    title: &'a str,
    tags: [Option<&'static str>; 2],
    detail: String,
    index: usize,
    at_cursor: bool,
    t: &Tokens,
) -> Element<'a, Message> {
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
            text(title)
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
    for tag in tags.into_iter().flatten() {
        title = title.push(text(tag).size(12).color(color(t.muted)));
    }
    let detail = text(detail)
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
    let n = state.picks().len();
    let status = match n {
        0 => "Nothing selected".to_owned(),
        1 => "1 source selected".to_owned(),
        n => format!("{n} sources selected"),
    };
    let add_label = match n {
        0 => "Add sources".to_owned(),
        1 => "Add 1 source".to_owned(),
        n => format!("Add {n} sources"),
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

    fn fresh(catalog: &Catalog, first_run: bool) -> Onboarding {
        Onboarding::new(catalog, HashSet::new(), HashSet::new(), first_run)
    }

    #[test]
    fn first_run_checks_the_starters_the_store_lacks() {
        let catalog = catalog::builtin();
        let starters: Vec<usize> = (0..catalog.feeds.len())
            .filter(|&i| catalog.feeds[i].starter)
            .collect();
        assert_eq!(fresh(catalog, true).picks().feeds, starters);

        let known = HashSet::from([catalog.feeds[starters[0]].url.clone()]);
        let state = Onboarding::new(catalog, known, HashSet::new(), true);
        assert_eq!(state.picks().feeds, starters[1..]);
        assert!(fresh(catalog, false).picks().is_empty());
    }

    #[test]
    fn known_rows_do_not_toggle_and_presets_do() {
        let catalog = catalog::builtin();
        let mut state = fresh(catalog, false);
        let rows = state.entries(catalog);
        let Entry::Feed(first) = rows[0] else {
            panic!("a category starts with feeds")
        };
        let preset = rows
            .iter()
            .position(|e| matches!(e, Entry::Keyword(_)))
            .expect("World has keyword presets");
        let Entry::Keyword(k) = rows[preset] else {
            unreachable!()
        };
        let known = HashSet::from([catalog.feeds[first].url.clone()]);
        // The store has the preset's query, spaced differently.
        let queries = HashSet::from([Keyword::new(&catalog.keywords[k].query).unwrap().query]);
        state = Onboarding::new(catalog, known, queries, false);
        state.apply(catalog, Action::ToggleCursor);
        state.apply(catalog, Action::ToggleEntry(preset));
        assert_eq!(state.cursor, preset);
        assert!(state.picks().is_empty());

        let mut state = fresh(catalog, false);
        state.apply(catalog, Action::ToggleEntry(preset));
        state.apply(catalog, Action::ToggleEntry(1));
        let picks = state.picks();
        assert_eq!((picks.feeds.len(), picks.len()), (1, 2));
        assert_eq!(picks.keywords, [k]);
        state.apply(catalog, Action::ToggleCursor);
        state.apply(catalog, Action::ToggleEntry(preset));
        assert!(state.picks().is_empty());
    }

    #[test]
    fn toggle_all_twice_checks_then_clears_the_category() {
        let catalog = catalog::builtin();
        let world = &catalog.categories[0].id;
        let rows = catalog.feeds_in(world).count() + catalog.keywords_in(world).count();
        let mut state = fresh(catalog, false);
        state.apply(catalog, Action::ToggleAll);
        assert_eq!(state.checked_in(catalog, 0), rows);
        assert_eq!(state.picks().len(), rows);
        state.apply(catalog, Action::ToggleAll);
        assert!(state.picks().is_empty());

        // With some checked, it checks the rest first.
        let mut state = fresh(catalog, true);
        let before = state.checked_in(catalog, 0);
        assert!(before > 0 && before < rows);
        state.apply(catalog, Action::ToggleAll);
        assert_eq!(state.checked_in(catalog, 0), rows);
    }

    fn type_query(state: &mut Onboarding, catalog: &Catalog, query: &str) {
        state.apply(catalog, Action::NewQuery);
        for c in query.chars() {
            state.type_query(catalog, palette::Input::Type(c.to_string()));
        }
        state.type_query(catalog, palette::Input::Run);
    }

    #[test]
    fn a_typed_query_is_checked_once_and_a_preset_query_checks_the_preset() {
        let catalog = catalog::builtin();
        let known = HashSet::from(["\"Already followed\"".to_owned()]);
        let mut state = Onboarding::new(catalog, HashSet::new(), known, false);
        state.apply(catalog, Action::NextCategory);
        type_query(&mut state, catalog, "  \"Cory  Booker\" ");
        // `n` went to the keyword alerts page; Enter stopped typing.
        let page = Onboarding::keywords_page(catalog);
        assert_eq!((state.category, state.draft.as_deref()), (page, None));
        type_query(&mut state, catalog, "\"Cory Booker\"");
        type_query(&mut state, catalog, "\"Already followed\"");
        type_query(
            &mut state,
            catalog,
            &format!(" {}", catalog.keywords[0].query),
        );
        type_query(&mut state, catalog, "   ");
        let picks = state.picks();
        assert_eq!(picks.queries, ["\"Cory Booker\""]);
        assert_eq!(picks.keywords, [0]);
        assert_eq!(state.checked_in(catalog, page), 2);

        // The typed query is a row after the field; Space unchecks it.
        assert_eq!(
            state.entries(catalog)[..2],
            [Entry::NewQuery, Entry::Query(0)]
        );
        state.apply(catalog, Action::Down);
        state.apply(catalog, Action::ToggleCursor);
        assert!(state.picks().queries.is_empty());

        // Esc drops what was being typed.
        state.apply(catalog, Action::NewQuery);
        state.type_query(catalog, palette::Input::Type("x".into()));
        state.type_query(catalog, palette::Input::Close);
        assert_eq!((state.draft.as_deref(), state.queries.len()), (None, 1));
    }

    #[test]
    fn categories_clamp_at_both_ends() {
        let catalog = catalog::builtin();
        let mcp = Onboarding::keywords_page(catalog) + 1;
        let mut state = fresh(catalog, false);
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
            action_for(&Key::Character("n".into()), none),
            Some(Action::NewQuery)
        );
        assert_eq!(
            action_for(&Key::Character("a".into()), Modifiers::CTRL),
            None
        );
    }
}
