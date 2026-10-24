//! See CLAUDE.md for what this crate owns.

mod article;
mod commands;
mod list;
mod onboarding;
mod palette;
mod sidebar;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gutenberg_core::catalog;
use gutenberg_core::config::{self, Config};
use gutenberg_core::favicon::Icons;
use gutenberg_core::keyword;
use gutenberg_core::sync::unix_now;
use gutenberg_core::web::{self, Answer, Web};
use gutenberg_store::{ArticleRow, Filter, Folder, Source, SourceKind, Store, View, ViewCounts};
use gutenberg_theme::{Rgb, Scheme, Tokens};
use iced::futures::Stream;
use iced::keyboard;
use iced::mouse::ScrollDelta;
use iced::widget::{
    column, container, image, mouse_area, opaque, row, scrollable, stack, svg, text, Column, Row,
    Space,
};
use iced::{
    border, font, window, Alignment, Border, Color, Element, Font, Length, Size, Subscription, Task,
};

use list::Action;
use sidebar::{Entry, Move};

/// Most articles kept in the list; the PRD plans for 100k.
const LIST_LIMIT: u32 = 100_000;
/// How often to look for writes from the refresh thread or a terminal.
const STORE_POLL: Duration = Duration::from_secs(1);
/// Most search results kept: the newest this many matches. Enough to
/// read through, and cheap enough to run on every key (S3.6 bench).
const SEARCH_LIMIT: u32 = 500;
/// Height of the search line above the list.
const SEARCH_BAR: f32 = 36.0;
/// Site icons in the sources pane, square.
const ICON_SIZE: f32 = 16.0;

/// Opens the window and blocks until it closes. When `config_path` is set,
/// edits to that file apply while the window is open. `store_path` is the
/// SQLite file to read articles from.
pub fn run(
    config: Config,
    config_path: Option<PathBuf>,
    store_path: Option<PathBuf>,
) -> iced::Result {
    use_desktop_font();
    iced::application(
        move || {
            let app = App::new(config.clone(), config_path.clone(), store_path.as_deref());
            // The desktop's light or dark preference, for the built-in theme.
            (app, iced::system::theme().map(Message::SystemTheme))
        },
        App::update,
        App::view,
    )
    .title("Gutenberg")
    .subscription(App::subscription)
    .theme(App::theme)
    .scale_factor(App::scale_factor)
    .window(window::Settings {
        // Hyprland draws the borders; no client-side decorations.
        decorations: false,
        #[cfg(target_os = "linux")]
        platform_specific: window::settings::PlatformSpecific {
            // Hyprland matches window rules on this class.
            application_id: "gutenberg".to_owned(),
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
}

/// Draws all text in the desktop's font: fontconfig's `monospace` family,
/// which `omarchy-font-set` sets, stands in for iced's sans-serif and
/// monospace families, so plain, bold, italic and code text all use it.
/// Without fontconfig, or for a family no font file provides, iced's own
/// choices stay; then this returns `false`.
fn use_desktop_font() -> bool {
    gutenberg_theme::font::monospace().is_some_and(|face| set_font(&face.family, None))
}

/// Points iced's sans-serif and monospace families at `family`, after
/// loading `file` (the family's font file) if given. Loading a file bumps
/// the font system's version, which makes iced reshape every paragraph, so
/// text already on screen changes too; it also brings in a font installed
/// after the window opened.
fn set_font(family: &str, file: Option<Vec<u8>>) -> bool {
    let mut fonts = iced::advanced::graphics::text::font_system()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(bytes) = file {
        fonts.load_font(std::borrow::Cow::Owned(bytes));
    }
    // `db_mut` also clears cosmic-text's font-match cache.
    let db = fonts.raw().db_mut();
    let installed = db
        .faces()
        .any(|face| face.families.iter().any(|(name, _)| name == family));
    if !installed {
        eprintln!("gutenberg: font {family:?}: no font file has it; using the default font");
        return false;
    }
    db.set_sans_serif_family(family);
    db.set_monospace_family(family);
    true
}

/// Text sizes in the code are for a 14 px font; `[font] size` scales the
/// whole window from there, so rows and padding grow with the text.
fn scale_factor(config: &Config) -> f32 {
    f32::from(config.font.size) / f32::from(config::Font::DEFAULT_SIZE)
}

struct App {
    tokens: Tokens,
    /// Whether `tokens` are the Omarchy theme's; if not, they're the
    /// built-in theme for `scheme`.
    omarchy: bool,
    /// The desktop's light or dark preference.
    scheme: Scheme,
    /// iced's own theme, for the parts it draws itself (scrollbars,
    /// checkboxes), made from `tokens`.
    iced_theme: iced::Theme,
    config: Config,
    config_path: Option<PathBuf>,
    store: Result<Store, String>,
    /// Which feeds the list shows; picked in the sources pane.
    filter: Filter,
    /// Which of their articles: all, unread, starred or today's.
    view: View,
    /// Articles in each view across every feed, for the view rows.
    view_counts: ViewCounts,
    /// What the store held at the last reload, newest first.
    rows: Vec<ArticleRow>,
    source_titles: HashMap<i64, String>,
    /// Keyword alerts, whose rows name the publisher (the article's author).
    keyword_sources: HashSet<i64>,
    folders: Vec<Folder>,
    sources: Vec<Source>,
    /// The favicon cache the refresh thread fills.
    icons: Option<Icons>,
    /// Each source's cached site icon, read at reload.
    source_icons: HashMap<i64, Icon>,
    /// Unread articles per source.
    unread: HashMap<i64, i64>,
    /// A press in the sources pane, which becomes a drag once it moves.
    drag: Option<Drag>,
    data_version: i64,
    selected: usize,
    /// First row on screen.
    top: usize,
    /// Height of the panes, from the window.
    height: f32,
    opened: Option<Opened>,
    /// Fetches the open article's images and reader page; set once its
    /// worker starts.
    web: Option<Web>,
    /// The open article's images, by URL.
    pictures: HashMap<String, article::Picture>,
    pending_g: bool,
    bench: Option<Bench>,
    /// The Add sources screen, shown instead of the panes while open.
    onboarding: Option<onboarding::Onboarding>,
    /// The command palette or help, drawn over the panes.
    overlay: Option<Overlay>,
    /// A search in place of the chosen folder or feed's list.
    search: Option<Search>,
}

/// What `/` started.
struct Search {
    query: String,
    /// Keys type into the query; Enter hands them back to the list.
    editing: bool,
}

enum Overlay {
    Palette(palette::Palette),
    Help,
}

/// Who gets the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum KeyMode {
    List,
    Onboarding,
    /// Typing a keyword alert on the Add sources screen.
    OnboardingQuery,
    Palette,
    Help,
    Search,
}

/// A press on a sources-pane row and where the pointer is now.
struct Drag {
    origin: Entry,
    over: Entry,
}

/// A cached site icon: SVG draws through its own widget.
#[derive(Debug, Clone)]
enum Icon {
    Raster(image::Handle),
    Vector(svg::Handle),
}

impl Icon {
    /// What's cached for each source with a feed URL.
    fn for_sources(icons: &Icons, sources: &[Source]) -> HashMap<i64, Icon> {
        let found = icons.found();
        sources
            .iter()
            .filter_map(|s| {
                let path = found.get(&Icons::key(s.url.as_deref()?))?;
                let icon = if path.extension().is_some_and(|ext| ext == "svg") {
                    Icon::Vector(svg::Handle::from_path(path))
                } else {
                    Icon::Raster(image::Handle::from_path(path))
                };
                Some((s.id, icon))
            })
            .collect()
    }

    fn view<'a>(&self) -> Element<'a, Message> {
        match self {
            Icon::Raster(handle) => image(handle.clone())
                .width(ICON_SIZE)
                .height(ICON_SIZE)
                .into(),
            Icon::Vector(handle) => svg(handle.clone())
                .width(ICON_SIZE)
                .height(ICON_SIZE)
                .into(),
        }
    }
}

/// The article shown in the article pane.
struct Opened {
    title: String,
    meta: String,
    /// The article's own page, for the reader view.
    url: Option<String>,
    /// The feed's summary.
    body: Vec<article::Block>,
    reader: Reader,
}

/// The reader view of the open article: its page cut down to the article.
enum Reader {
    /// Showing the feed's summary.
    Off,
    Loading,
    Ready(Vec<article::Block>),
    /// Why there's no full article; the summary shows instead.
    Failed(String),
}

impl Opened {
    /// What the pane shows: the full article once it's in, else the summary.
    fn shown(&self) -> &[article::Block] {
        match &self.reader {
            Reader::Ready(blocks) => blocks,
            _ => &self.body,
        }
    }
}

/// `GUTENBERG_SCROLL_BENCH=<frames>`: scroll a row per frame, then report.
struct Bench {
    frames_left: usize,
    last: Option<Instant>,
    intervals: Vec<Duration>,
}

#[derive(Debug, Clone)]
enum Message {
    Key(Action),
    Scroll(ScrollDelta),
    Click(usize),
    PollStore,
    Resized(Size),
    Frame(Instant),
    ConfigChanged(Config),
    /// The Omarchy theme, if there is one, and the desktop font if it
    /// changed, read again.
    ThemeReloaded(Option<Tokens>, Option<NewFont>),
    /// The desktop's light or dark preference, at start and when it changes.
    SystemTheme(iced::theme::Mode),
    /// Mouse down on a sources-pane row.
    Press(Entry),
    /// The pointer entered a sources-pane row during a press.
    Over(Entry),
    /// Mouse up anywhere: a click if it never left the row, else a drop.
    Release,
    Onboarding(onboarding::Action),
    /// A key while typing a keyword alert on the Add sources screen.
    OnboardingQuery(palette::Input),
    /// A key while the command palette is open.
    Palette(palette::Input),
    /// A command picked with the mouse in the palette.
    RunCommand(Action),
    /// Closes the palette or help.
    CloseOverlay,
    /// A key while typing in the search line.
    Search(palette::Input),
    /// A link in the article pane was clicked.
    OpenLink(String),
    /// The web worker is up and takes requests.
    WebReady(Web),
    WebAnswer(Arc<Answer>),
    /// A fetched raster image is decoded and ready to draw, or isn't.
    ImageAllocated(String, Result<image::Allocation, image::Error>),
}

impl App {
    fn new(
        config: Config,
        config_path: Option<PathBuf>,
        store_path: Option<&std::path::Path>,
    ) -> Self {
        let store = match store_path {
            Some(path) => Store::open(path).map_err(|e| e.to_string()),
            None => Err("no data directory".to_owned()),
        };
        let mut app = Self::with_store(config, config_path, store, store_path.map(Icons::beside));
        app.use_theme(Tokens::omarchy());
        app
    }

    /// The Omarchy theme's tokens, or the built-in theme for the desktop's
    /// preference when there's none.
    fn use_theme(&mut self, omarchy: Option<Tokens>) {
        self.omarchy = omarchy.is_some();
        self.set_tokens(omarchy.unwrap_or_else(|| Tokens::fallback(self.scheme)));
    }

    fn set_tokens(&mut self, tokens: Tokens) {
        self.iced_theme = iced_theme(&tokens);
        self.tokens = tokens;
    }

    fn theme(&self) -> iced::Theme {
        self.iced_theme.clone()
    }

    fn with_store(
        config: Config,
        config_path: Option<PathBuf>,
        store: Result<Store, String>,
        icons: Option<Icons>,
    ) -> Self {
        let bench = std::env::var("GUTENBERG_SCROLL_BENCH")
            .ok()
            .and_then(|n| n.parse().ok())
            .map(|frames| Bench {
                frames_left: frames,
                last: None,
                intervals: Vec::with_capacity(frames),
            });
        let mut app = Self {
            tokens: Tokens::fallback(Scheme::Dark),
            omarchy: false,
            scheme: Scheme::Dark,
            iced_theme: iced_theme(&Tokens::fallback(Scheme::Dark)),
            config,
            config_path,
            store,
            filter: Filter::All,
            view: View::All,
            view_counts: ViewCounts::default(),
            rows: Vec::new(),
            source_titles: HashMap::new(),
            keyword_sources: HashSet::new(),
            folders: Vec::new(),
            sources: Vec::new(),
            icons,
            source_icons: HashMap::new(),
            unread: HashMap::new(),
            drag: None,
            data_version: 0,
            selected: 0,
            top: 0,
            height: 768.0,
            opened: None,
            web: None,
            pictures: HashMap::new(),
            pending_g: false,
            bench,
            onboarding: None,
            overlay: None,
            search: None,
        };
        app.reload();
        // First run: nothing to read yet, so start by picking sources.
        if app.store.is_ok() && app.sources.is_empty() && app.bench.is_none() {
            app.open_onboarding();
        }
        app
    }

    /// Shows Add sources. With an empty store the starter feeds are checked.
    fn open_onboarding(&mut self) {
        if self.store.is_err() {
            return;
        }
        let known = self
            .sources
            .iter()
            .filter(|s| s.kind == SourceKind::Feed)
            .filter_map(|s| s.url.clone())
            .collect();
        let known_queries = self
            .sources
            .iter()
            .filter(|s| s.kind == SourceKind::Keyword)
            .filter_map(|s| keyword::Keyword::from_config(&s.config).ok())
            .map(|k| k.query)
            .collect();
        self.onboarding = Some(onboarding::Onboarding::new(
            catalog::builtin(),
            known,
            known_queries,
            self.sources.is_empty(),
        ));
    }

    fn update_onboarding(&mut self, action: onboarding::Action) -> Task<Message> {
        use onboarding::Action as A;
        let Some(state) = &mut self.onboarding else {
            return Task::none();
        };
        match action {
            A::Quit => return iced::exit(),
            A::Close => self.onboarding = None,
            A::Add => {
                let picks = state.picks();
                if picks.is_empty() {
                    self.onboarding = None;
                    return Task::none();
                }
                let Ok(store) = &self.store else {
                    return Task::none();
                };
                match catalog::subscribe(store, catalog::builtin(), &picks) {
                    Ok(_) => {
                        self.onboarding = None;
                        self.reload();
                    }
                    Err(e) => eprintln!("gutenberg: adding sources: {e}"),
                }
            }
            other => state.apply(catalog::builtin(), other),
        }
        Task::none()
    }

    /// Re-reads articles and sources, keeping the selection on the same
    /// article when it's still there.
    fn reload(&mut self) {
        let Ok(store) = &self.store else { return };
        let selected_id = self.rows.get(self.selected).map(|r| r.id);
        let read = (|| {
            Ok::<_, gutenberg_store::Error>((
                match self.search.as_ref().map(|s| s.query.as_str()) {
                    Some(query) => store.search(query, SEARCH_LIMIT)?,
                    None => store.list_articles_in(self.filter, self.view, LIST_LIMIT)?,
                },
                store.sources()?,
                store.folders()?,
                store.unread_counts()?,
                store.view_counts()?,
            ))
        })();
        match read {
            Ok((rows, sources, folders, unread, view_counts)) => {
                self.rows = rows;
                self.source_titles = sources.iter().map(|s| (s.id, s.title.clone())).collect();
                self.keyword_sources = sources
                    .iter()
                    .filter(|s| s.kind == SourceKind::Keyword)
                    .map(|s| s.id)
                    .collect();
                if let Some(icons) = &self.icons {
                    self.source_icons = Icon::for_sources(icons, &sources);
                }
                self.sources = sources;
                self.folders = folders;
                self.unread = unread;
                self.view_counts = view_counts;
            }
            Err(e) => eprintln!("gutenberg: reading the store: {e}"),
        }
        if let Some(id) = selected_id {
            if let Some(i) = self.rows.iter().position(|r| r.id == id) {
                self.selected = i;
            }
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.top = self
            .top
            .min(list::max_top(self.rows.len(), self.list_height()));
        self.data_version = store.data_version().unwrap_or(self.data_version);
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        if !matches!(message, Message::Key(Action::PendingG)) {
            self.pending_g = false;
        }
        match message {
            Message::Key(Action::Quit) => return iced::exit(),
            Message::Key(Action::AddSources) => self.open_onboarding(),
            Message::Key(Action::Reader) => self.toggle_reader(),
            Message::Key(Action::Palette) => {
                self.overlay = Some(Overlay::Palette(palette::Palette::default()));
            }
            Message::Key(Action::Help) => self.overlay = Some(Overlay::Help),
            Message::Key(Action::Star) => self.toggle_star(),
            Message::Key(Action::ToggleRead) => self.toggle_read(),
            Message::Key(Action::MarkFolderRead) => self.mark_chosen_read(),
            Message::Key(Action::OpenInBrowser) => {
                if let Some(url) = self.selected_link() {
                    open_in_browser(&url);
                }
            }
            Message::Key(Action::Search) => match &mut self.search {
                Some(search) => search.editing = true,
                None => {
                    self.search = Some(Search {
                        query: String::new(),
                        editing: true,
                    });
                    self.show_from_top();
                }
            },
            Message::Key(Action::ClearSearch) => self.clear_search(),
            Message::Key(Action::View(view)) => self.show_view(view),
            Message::Search(input) => self.search_input(input),
            Message::Key(Action::CopyLink) => {
                if let Some(url) = self.selected_link() {
                    return iced::clipboard::write(url);
                }
            }
            Message::Palette(input) => {
                let Some(Overlay::Palette(state)) = &mut self.overlay else {
                    return Task::none();
                };
                let closes = matches!(input, palette::Input::Run | palette::Input::Close);
                let picked = state.apply(input);
                if closes {
                    self.overlay = None;
                }
                if let Some(command) = picked {
                    return self.update(Message::Key(command.action));
                }
            }
            Message::RunCommand(action) => {
                self.overlay = None;
                return self.update(Message::Key(action));
            }
            Message::CloseOverlay => self.overlay = None,
            Message::Onboarding(action) => return self.update_onboarding(action),
            Message::OnboardingQuery(input) => {
                if let Some(state) = &mut self.onboarding {
                    state.type_query(catalog::builtin(), input);
                }
            }
            Message::Key(Action::PendingG) => self.pending_g = true,
            Message::Key(Action::Down) => self.select(self.selected.saturating_add(1)),
            Message::Key(Action::Up) => self.select(self.selected.saturating_sub(1)),
            Message::Key(Action::First) => self.select(0),
            Message::Key(Action::Last) => self.select(usize::MAX),
            Message::Key(Action::Open) => self.open(self.selected),
            Message::Click(i) => {
                self.select(i);
                self.open(i);
            }
            Message::Scroll(delta) => {
                let rows = match delta {
                    ScrollDelta::Lines { y, .. } => -y * list::WHEEL_ROWS,
                    ScrollDelta::Pixels { y, .. } => -y / list::ROW_HEIGHT,
                };
                let top = (self.top as f32 + rows).round().max(0.0) as usize;
                self.top = top.min(list::max_top(self.rows.len(), self.list_height()));
            }
            Message::PollStore => {
                let changed = match &self.store {
                    Ok(store) => store.data_version().is_ok_and(|v| v != self.data_version),
                    Err(_) => false,
                };
                if changed {
                    self.reload();
                }
            }
            Message::Resized(size) => self.set_height(size.height),
            Message::Frame(at) => return self.bench_frame(at),
            Message::ThemeReloaded(tokens, font) => {
                self.use_theme(tokens);
                if let Some(font) = font {
                    let file = font.file.map(Arc::unwrap_or_clone);
                    set_font(&font.family, file);
                }
            }
            Message::SystemTheme(mode) => {
                self.scheme = match mode {
                    iced::theme::Mode::Light => Scheme::Light,
                    iced::theme::Mode::Dark | iced::theme::Mode::None => Scheme::Dark,
                };
                if !self.omarchy {
                    self.set_tokens(Tokens::fallback(self.scheme));
                }
            }
            Message::ConfigChanged(config) => {
                // iced sends no resize when the scale factor changes, but the
                // same window is now a different number of logical pixels.
                let height = self.height * self.scale_factor() / scale_factor(&config);
                self.config = config;
                self.set_height(height);
            }
            Message::Press(entry) => {
                self.drag = Some(Drag {
                    origin: entry,
                    over: entry,
                })
            }
            Message::Over(entry) => {
                if let Some(drag) = &mut self.drag {
                    drag.over = entry;
                }
            }
            Message::Release => {
                if let Some(drag) = self.drag.take() {
                    if drag.over == drag.origin {
                        self.choose(drag.origin);
                    } else if let Some(change) = sidebar::drop_on(drag.origin, drag.over) {
                        self.apply(change);
                    }
                }
            }
            Message::OpenLink(url) => open_in_browser(&url),
            Message::WebReady(web) => {
                if let Some(Opened {
                    url: Some(url),
                    reader: Reader::Loading,
                    ..
                }) = &self.opened
                {
                    web.page(url.clone());
                }
                self.web = Some(web);
                self.request_pictures();
            }
            Message::WebAnswer(answer) => return self.web_answer(answer),
            Message::ImageAllocated(url, allocated) => {
                if let Some(slot) = self.pictures.get_mut(&url) {
                    *slot = match allocated {
                        Ok(allocation) => article::Picture::Raster(allocation),
                        Err(_) => article::Picture::Failed,
                    };
                }
            }
        }
        Task::none()
    }

    /// Shows the articles a sources-pane row stands for, ending a search.
    /// A view row shows that view across every feed; a folder or feed keeps
    /// the view.
    fn choose(&mut self, entry: Entry) {
        let Some(filter) = entry.filter() else { return };
        let view = match entry {
            Entry::View(view) => view,
            _ => self.view,
        };
        if filter != self.filter || view != self.view || self.search.is_some() {
            self.filter = filter;
            self.view = view;
            self.search = None;
            self.show_from_top();
        }
    }

    /// Re-reads the list and puts the selection at its top.
    fn show_from_top(&mut self) {
        self.selected = 0;
        self.top = 0;
        self.reload();
    }

    /// Ends the search; the chosen folder or feed's list comes back.
    fn clear_search(&mut self) {
        if self.search.take().is_some() {
            self.show_from_top();
        }
    }

    /// A key in the search line: typing narrows the list as you go, the
    /// arrows move through it, Enter hands the keys back to the list, and
    /// Esc ends the search.
    fn search_input(&mut self, input: palette::Input) {
        let Some(search) = &mut self.search else {
            return;
        };
        match input {
            palette::Input::Type(text) => {
                search.query.push_str(&text);
                self.show_from_top();
            }
            palette::Input::Backspace => {
                search.query.pop();
                self.show_from_top();
            }
            palette::Input::Down => self.select(self.selected.saturating_add(1)),
            palette::Input::Up => self.select(self.selected.saturating_sub(1)),
            palette::Input::Run => search.editing = false,
            palette::Input::Close => self.clear_search(),
        }
    }

    /// The list's own height: the pane's, less the search line when shown.
    fn list_height(&self) -> f32 {
        if self.search.is_some() {
            (self.height - SEARCH_BAR).max(0.0)
        } else {
            self.height
        }
    }

    /// The window is `height` logical pixels tall: keeps the selected row
    /// in view and the list from scrolling past its end.
    fn set_height(&mut self, height: f32) {
        self.height = height;
        self.top = list::scroll_to_show(self.top, self.selected, self.list_height())
            .min(list::max_top(self.rows.len(), self.list_height()));
    }

    fn scale_factor(&self) -> f32 {
        scale_factor(&self.config)
    }

    /// Moves a source or folder in the store, then shows the new order.
    fn apply(&mut self, change: Move) {
        let Ok(store) = &mut self.store else { return };
        let moved = match change {
            Move::Source(id, target) => store.move_source(id, target),
            Move::Folder(id, before) => store.move_folder(id, before),
        };
        if let Err(e) = moved {
            eprintln!("gutenberg: moving: {e}");
        }
        self.reload();
    }

    fn select(&mut self, i: usize) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = i.min(self.rows.len() - 1);
        self.top = list::scroll_to_show(self.top, self.selected, self.list_height());
    }

    /// The selected article's own link.
    fn selected_link(&self) -> Option<String> {
        self.rows.get(self.selected)?.url.clone()
    }

    /// Stars or unstars the selected article; the starred count follows.
    fn toggle_star(&mut self) {
        let (Ok(store), Some(row)) = (&self.store, self.rows.get(self.selected)) else {
            return;
        };
        if let Err(e) = store.set_starred(row.id, !row.starred) {
            eprintln!("gutenberg: starring: {e}");
        }
        self.reload();
    }

    /// Read becomes unread and back; the feed's count follows.
    fn toggle_read(&mut self) {
        let (Ok(store), Some(row)) = (&self.store, self.rows.get(self.selected)) else {
            return;
        };
        if let Err(e) = store.set_read(row.id, !row.read) {
            eprintln!("gutenberg: marking read: {e}");
        }
        self.reload();
    }

    /// Everything the list shows for the chosen folder or feed and view
    /// becomes read.
    fn mark_chosen_read(&mut self) {
        let Ok(store) = &self.store else { return };
        if let Err(e) = store.mark_read_in(self.filter, self.view) {
            eprintln!("gutenberg: marking read: {e}");
        }
        self.reload();
    }

    /// Narrows the chosen folder or feed to a view, ending a search.
    fn show_view(&mut self, view: View) {
        self.view = view;
        self.search = None;
        self.show_from_top();
    }

    /// Shows the article and marks it read.
    fn open(&mut self, i: usize) {
        let Some(row) = self.rows.get_mut(i) else {
            return;
        };
        if !row.read {
            if let Ok(store) = &self.store {
                match store.set_read(row.id, true) {
                    Ok(()) => {
                        row.read = true;
                        if let Some(n) = self.unread.get_mut(&row.source_id) {
                            *n = (*n - 1).max(0);
                        }
                        self.view_counts.unread = (self.view_counts.unread - 1).max(0);
                    }
                    Err(e) => eprintln!("gutenberg: marking read: {e}"),
                }
            }
        }
        let summary = match &self.store {
            Ok(store) => store.summary(row.id).ok().flatten(),
            Err(_) => None,
        };
        let source = self
            .source_titles
            .get(&row.source_id)
            .cloned()
            .unwrap_or_default();
        let mut meta = vec![source, list::age(row.sort_at, unix_now())];
        meta.extend(row.author.clone());
        // Relative links in a summary point into the article's own site.
        let base = row.url.clone().or_else(|| {
            self.sources
                .iter()
                .find(|s| s.id == row.source_id)
                .and_then(|s| s.url.clone())
        });
        let body = summary
            .as_deref()
            .map(|html| article::parse(html, base.as_deref()))
            .unwrap_or_default();
        self.opened = Some(Opened {
            title: row.title.clone(),
            meta: meta.join(" · "),
            url: row.url.clone(),
            body,
            reader: Reader::Off,
        });
        self.request_pictures();
    }

    /// Swaps the open article between the feed's summary and its full page,
    /// asking the worker for the page.
    fn toggle_reader(&mut self) {
        let Some(opened) = &mut self.opened else {
            return;
        };
        opened.reader = match (&opened.reader, &opened.url) {
            (Reader::Off, None) => Reader::Failed("this article has no link".into()),
            (Reader::Off, Some(url)) => {
                // Without the worker yet, WebReady sends the request.
                if let Some(web) = &self.web {
                    web.page(url.clone());
                }
                Reader::Loading
            }
            _ => Reader::Off,
        };
        self.request_pictures();
    }

    fn web_answer(&mut self, answer: Arc<Answer>) -> Task<Message> {
        match Arc::unwrap_or_clone(answer) {
            Answer::Image { url, bytes, format } => self.image_loaded(url, Some((bytes, format))),
            Answer::ImageFailed { url } => self.image_loaded(url, None),
            Answer::Page { url, html } => {
                self.page_loaded(&url, Ok(html));
                Task::none()
            }
            Answer::PageFailed { url, error } => {
                self.page_loaded(&url, Err(error));
                Task::none()
            }
        }
    }

    /// Takes the reader page for the open article, if it's still waiting
    /// for that page; anything else arrived too late and is dropped.
    fn page_loaded(&mut self, url: &str, page: Result<String, String>) {
        let Some(opened) = &mut self.opened else {
            return;
        };
        if opened.url.as_deref() != Some(url) || !matches!(opened.reader, Reader::Loading) {
            return;
        }
        opened.reader = match page {
            Ok(html) => Reader::Ready(article::parse(&html, Some(url))),
            Err(error) => Reader::Failed(error),
        };
        self.request_pictures();
    }

    /// Takes a fetched image for the open article. A raster image is
    /// decoded and uploaded first, so it appears at its size in one step.
    fn image_loaded(
        &mut self,
        url: String,
        fetched: Option<(Vec<u8>, web::Format)>,
    ) -> Task<Message> {
        // An answer for an article that's no longer open is dropped.
        let Some(slot) = self.pictures.get_mut(&url) else {
            return Task::none();
        };
        match fetched {
            None => *slot = article::Picture::Failed,
            Some((bytes, web::Format::Svg)) => {
                *slot = article::Picture::Vector(svg::Handle::from_memory(bytes));
            }
            Some((bytes, _)) => {
                return image::allocate(image::Handle::from_bytes(bytes))
                    .map(move |allocated| Message::ImageAllocated(url.clone(), allocated));
            }
        }
        Task::none()
    }

    /// Keeps the images of what the pane shows, drops the rest, and asks
    /// the worker for the ones not yet fetched.
    fn request_pictures(&mut self) {
        let urls = self
            .opened
            .as_ref()
            .map(|o| article::image_urls(o.shown()))
            .unwrap_or_default();
        self.pictures.retain(|url, _| urls.contains(&url.as_str()));
        let Some(web) = &self.web else { return };
        for url in urls {
            if !self.pictures.contains_key(url) {
                self.pictures
                    .insert(url.to_owned(), article::Picture::Loading);
                web.image(url.to_owned());
            }
        }
    }

    fn bench_frame(&mut self, at: Instant) -> Task<Message> {
        let Some(bench) = &mut self.bench else {
            return Task::none();
        };
        if let Some(last) = bench.last {
            bench.intervals.push(at - last);
            bench.frames_left = bench.frames_left.saturating_sub(1);
        }
        bench.last = Some(at);
        // One row per frame, wrapping at the end.
        let max = list::max_top(self.rows.len(), self.height);
        self.top = if self.top >= max { 0 } else { self.top + 1 };
        if bench.frames_left == 0 {
            report(&bench.intervals, self.rows.len(), self.height);
            return iced::exit();
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let mode = match (&self.onboarding, &self.overlay, &self.search) {
            (Some(o), _, _) if o.draft.is_some() => KeyMode::OnboardingQuery,
            (Some(_), _, _) => KeyMode::Onboarding,
            (None, Some(Overlay::Palette(_)), _) => KeyMode::Palette,
            (None, Some(Overlay::Help), _) => KeyMode::Help,
            (None, None, Some(Search { editing: true, .. })) => KeyMode::Search,
            (None, None, _) => KeyMode::List,
        };
        let keys = keyboard::listen().with((self.pending_g, mode)).filter_map(
            |((pending_g, mode), event)| {
                let keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    text,
                    ..
                } = event
                else {
                    return None;
                };
                // Only one thing hears a key: list keys must not fire under
                // Add sources, and typing in the palette is just typing.
                match mode {
                    KeyMode::Onboarding => {
                        onboarding::action_for(&key, modifiers).map(Message::Onboarding)
                    }
                    KeyMode::OnboardingQuery => {
                        palette::input_for(&key, modifiers, text.as_deref())
                            .map(Message::OnboardingQuery)
                    }
                    KeyMode::Palette => {
                        palette::input_for(&key, modifiers, text.as_deref()).map(Message::Palette)
                    }
                    KeyMode::Search => {
                        palette::input_for(&key, modifiers, text.as_deref()).map(Message::Search)
                    }
                    KeyMode::Help => {
                        use keyboard::key::Named;
                        match key.as_ref() {
                            keyboard::Key::Named(Named::Escape)
                            | keyboard::Key::Character("?" | "q") => Some(Message::CloseOverlay),
                            _ => None,
                        }
                    }
                    KeyMode::List => list::action_for(&key, modifiers, pending_g).map(Message::Key),
                }
            },
        );
        let mut subscriptions = vec![
            keys,
            iced::time::every(STORE_POLL).map(|_| Message::PollStore),
            window::resize_events().map(|(_, size)| Message::Resized(size)),
            iced::system::theme_changes().map(Message::SystemTheme),
        ];
        if let Some(path) = &self.config_path {
            subscriptions.push(Subscription::run_with(path.clone(), watch_config));
        }
        if self.bench.is_none() {
            subscriptions.push(Subscription::run(theme_reloads));
            subscriptions.push(Subscription::run(web_worker));
        }
        if self.bench.is_some() {
            subscriptions.push(window::frames().map(Message::Frame));
        }
        if self.drag.is_some() {
            subscriptions.push(iced::event::listen_with(|event, _, _| match event {
                iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                    iced::mouse::Button::Left,
                )) => Some(Message::Release),
                _ => None,
            }));
        }
        Subscription::batch(subscriptions)
    }

    fn view(&self) -> Element<'_, Message> {
        let t = &self.tokens;
        if let Some(state) = &self.onboarding {
            return onboarding::view(state, catalog::builtin(), t);
        }
        let layout = self.config.layout;
        let panes: [(u16, Element<'_, Message>, Rgb); 3] = [
            (layout.sources, self.sidebar_view(), t.background),
            (layout.list, self.list_view(), t.background),
            (layout.article, self.article_view(), t.lighter_background),
        ];
        let panes = Row::with_children(
            panes
                .into_iter()
                .filter(|(width, _, _)| *width > 0)
                .map(|(width, content, background)| pane(content, background, t.border, width)),
        )
        .height(Length::Fill);
        match &self.overlay {
            None => panes.into(),
            Some(overlay) => stack![panes, self.overlay_view(overlay)].into(),
        }
    }

    /// The palette or help, centered over a dimmed window; a click outside
    /// it closes it.
    fn overlay_view<'a>(&self, overlay: &'a Overlay) -> Element<'a, Message> {
        let t = &self.tokens;
        let (prompt, rows, footer) = match overlay {
            Overlay::Palette(state) => {
                let query = if state.query.is_empty() {
                    text("Type to find a command").color(color(t.muted))
                } else {
                    text(format!("{}▏", state.query)).color(color(t.foreground))
                };
                let rows: Vec<Element<'a, Message>> = state
                    .matches
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        mouse_area(command_row(c, i == state.cursor, t))
                            .on_press(Message::RunCommand(c.action))
                            .into()
                    })
                    .collect();
                let prompt = row![text(":").color(color(t.accent)), query]
                    .spacing(8)
                    .padding([10, 12]);
                let footer = if state.matches.is_empty() {
                    "No command matches"
                } else {
                    "Enter runs · ↑↓ choose · Esc closes"
                };
                (Some(prompt), rows, footer)
            }
            Overlay::Help => {
                let rows = commands::ALL
                    .iter()
                    .map(|c| command_row(c, false, t))
                    .collect();
                let title =
                    row![text("Keys").size(16).color(color(t.foreground))].padding([10, 12]);
                (
                    Some(title),
                    rows,
                    "Esc or ? closes · : finds a command by name",
                )
            }
        };
        let (surface, border, backdrop) = (t.lighter_background, t.accent, t.background);
        let card = container(
            Column::new()
                .push(prompt)
                .push(Column::with_children(rows))
                .push(container(text(footer).size(12).color(color(t.muted))).padding([8, 12]))
                .width(Length::Fixed(560.0)),
        )
        .style(move |_| container::Style {
            background: Some(color(surface).into()),
            border: Border {
                color: color(border),
                width: 1.0,
                radius: border::radius(6),
            },
            ..Default::default()
        });
        let dimmed = container(iced::widget::center(opaque(card)))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(
                    Color {
                        a: 0.6,
                        ..color(backdrop)
                    }
                    .into(),
                ),
                ..Default::default()
            });
        mouse_area(dimmed).on_press(Message::CloseOverlay).into()
    }

    fn sidebar_view(&self) -> Element<'_, Message> {
        let t = &self.tokens;
        if self.store.is_err() {
            return Space::new().into();
        }
        let rows = sidebar::entries(&self.folders, &self.sources)
            .into_iter()
            .map(|entry| self.sidebar_row(entry));
        // The space below the rows is a drop target too: out of any folder.
        let rest = mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
            .on_enter(Message::Over(Entry::Unfiled));
        let rest: Element<'_, Message> = if self.drop_line_at(Entry::Unfiled) {
            column![drop_line(t.accent), rest].into()
        } else {
            rest.into()
        };
        Column::with_children(rows)
            .push(rest)
            .padding([8, 0])
            .height(Length::Fill)
            .into()
    }

    /// Whether a drag is over `entry` and dropping there would do something.
    fn drop_line_at(&self, entry: Entry) -> bool {
        self.drag.as_ref().is_some_and(|d| {
            d.over == entry && d.over != d.origin && sidebar::drop_on(d.origin, entry).is_some()
        })
    }

    fn sidebar_row(&self, entry: Entry) -> Element<'_, Message> {
        let t = &self.tokens;
        let (name, bold, indent) = match entry {
            Entry::View(view) => (sidebar::view_name(view).to_owned(), true, 12),
            Entry::Folder(id) => (
                self.folders
                    .iter()
                    .find(|f| f.id == id)
                    .map_or_else(String::new, |f| f.name.clone()),
                true,
                12,
            ),
            Entry::Source { id, filed } => (
                self.source_titles.get(&id).cloned().unwrap_or_default(),
                false,
                if filed { 28 } else { 12 },
            ),
            Entry::Unfiled => (String::new(), false, 12),
        };
        let count = sidebar::count(entry, &self.sources, &self.unread, &self.view_counts);
        // The row being dragged dims until it's dropped.
        let dragged = self
            .drag
            .as_ref()
            .is_some_and(|d| d.origin == entry && d.over != entry);
        let weight = if bold {
            font::Weight::Bold
        } else {
            font::Weight::Normal
        };
        let label = text(name)
            .size(14)
            .color(color(if dragged { t.muted } else { t.foreground }))
            .font(Font {
                weight,
                ..Font::DEFAULT
            })
            .wrapping(text::Wrapping::None)
            .width(Length::Fill);
        let count = text(if count > 0 {
            count.to_string()
        } else {
            String::new()
        })
        .size(12)
        .color(color(t.muted));
        // The view row and the chosen folder or feed are both lit.
        let chosen = match entry {
            Entry::View(view) => view == self.view,
            _ => self.filter != Filter::All && entry.filter() == Some(self.filter),
        };
        let background = chosen.then(|| color(t.selection).into());
        // Feeds get their site's icon, or a blank of its size so titles align.
        let icon = match entry {
            Entry::Source { id, .. } => Some(self.source_icons.get(&id).map_or_else(
                || Space::new().width(ICON_SIZE).height(ICON_SIZE).into(),
                Icon::view,
            )),
            _ => None,
        };
        let line = Row::new()
            .push(icon)
            // A long title stops at the count instead of running under it.
            .push(container(label).width(Length::Fill).clip(true))
            .push(count)
            .spacing(8)
            .align_y(Alignment::Center);
        let row = container(line)
            .padding(iced::Padding::from([6, 12]).left(indent as f32))
            .width(Length::Fill)
            .clip(true)
            .style(move |_| container::Style {
                background,
                ..Default::default()
            });
        let row = mouse_area(row)
            .on_press(Message::Press(entry))
            .on_enter(Message::Over(entry));
        if self.drop_line_at(entry) {
            column![drop_line(t.accent), row].into()
        } else {
            row.into()
        }
    }

    fn list_view(&self) -> Element<'_, Message> {
        let t = &self.tokens;
        if let Err(e) = &self.store {
            return note(format!("Can't open the article store: {e}"), t.muted);
        }
        let list: Element<'_, Message> = if self.rows.is_empty() {
            let message = match &self.search {
                Some(search) if search.query.trim().is_empty() => "Type to search every article",
                Some(_) => "No articles match",
                None if self.sources.is_empty() => {
                    "No feeds yet. Press a to pick some, or run gutenberg --add <url>"
                }
                None => "Fetching your feeds…",
            };
            note(message.to_owned(), t.muted)
        } else {
            let now = unix_now();
            let rows = list::visible(self.top, self.list_height(), self.rows.len())
                .map(|i| self.row_view(i, &self.rows[i], now));
            mouse_area(
                Column::with_children(rows)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .clip(true),
            )
            .on_scroll(Message::Scroll)
            .into()
        };
        match &self.search {
            None => list,
            Some(search) => column![self.search_bar(search), list].into(),
        }
    }

    /// `/` and what's typed, with a cursor while typing, and how many
    /// articles matched.
    fn search_bar<'a>(&self, search: &'a Search) -> Element<'a, Message> {
        let t = &self.tokens;
        let query = if search.editing {
            format!("{}▏", search.query)
        } else {
            search.query.clone()
        };
        let count = match self.rows.len() {
            0 => String::new(),
            n if n as u32 >= SEARCH_LIMIT => format!("newest {n}"),
            1 => "1 match".to_owned(),
            n => format!("{n} matches"),
        };
        let (background, border_color) = (t.lighter_background, t.border);
        container(
            row![
                text("/").size(14).color(color(t.accent)),
                text(query)
                    .size(14)
                    .color(color(t.foreground))
                    .wrapping(text::Wrapping::None)
                    .width(Length::Fill),
                text(count).size(12).color(color(t.muted)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([0, 12])
        .height(Length::Fixed(SEARCH_BAR))
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .clip(true)
        .style(move |_| container::Style {
            background: Some(color(background).into()),
            border: Border {
                color: color(border_color),
                width: 1.0,
                radius: border::radius(0),
            },
            ..Default::default()
        })
        .into()
    }

    fn row_view<'a>(&'a self, i: usize, row: &'a ArticleRow, now: i64) -> Element<'a, Message> {
        let t = &self.tokens;
        let selected = i == self.selected;
        let (title_color, weight) = if row.read {
            (t.muted, font::Weight::Normal)
        } else {
            (t.foreground, font::Weight::Bold)
        };
        let source = self
            .source_titles
            .get(&row.source_id)
            .map_or("", String::as_str);
        let title = text(&row.title)
            .size(15)
            .color(color(title_color))
            .font(Font {
                weight,
                ..Font::DEFAULT
            })
            .wrapping(text::Wrapping::None);
        let age = list::age(row.sort_at, now);
        let meta = match &row.author {
            Some(publisher) if self.keyword_sources.contains(&row.source_id) => {
                format!("{source} · {publisher} · {age}")
            }
            _ => format!("{source} · {age}"),
        };
        let meta = text(meta)
            .size(12)
            .color(color(t.muted))
            .wrapping(text::Wrapping::None);
        let meta = Row::new()
            .push(
                row.starred
                    .then(|| text("★").size(12).color(color(t.accent))),
            )
            .push(meta)
            .spacing(4);
        let background = selected.then(|| color(t.selection).into());
        mouse_area(
            container(column![title, meta].spacing(4))
                .padding([8, 12])
                .width(Length::Fill)
                .height(Length::Fixed(list::ROW_HEIGHT))
                .clip(true)
                .style(move |_| container::Style {
                    background,
                    ..Default::default()
                }),
        )
        .on_press(Message::Click(i))
        .into()
    }

    fn article_view(&self) -> Element<'_, Message> {
        let t = &self.tokens;
        let Some(opened) = &self.opened else {
            return Space::new().into();
        };
        let note = match &opened.reader {
            Reader::Off => None,
            Reader::Loading => Some("Loading the full article…".to_owned()),
            Reader::Ready(_) => Some("Full article · v for the feed's summary".to_owned()),
            Reader::Failed(why) => Some(format!(
                "No full article ({why}); showing the feed's summary"
            )),
        };
        scrollable(
            column![
                text(&opened.title)
                    .size(22)
                    .color(color(t.foreground))
                    .wrapping(text::Wrapping::WordOrGlyph),
                text(&opened.meta).size(13).color(color(t.muted)),
            ]
            .push(note.map(|n| {
                text(n)
                    .size(13)
                    .color(color(t.accent))
                    .wrapping(text::Wrapping::WordOrGlyph)
            }))
            .push(article::view(opened.shown(), t, &self.pictures))
            .spacing(12)
            .padding(24),
        )
        .height(Length::Fill)
        .into()
    }
}

/// For `cargo bench`: an app over N articles in an in-memory store.
#[doc(hidden)]
pub mod bench {
    use super::*;

    pub struct ListFixture(App);

    impl ListFixture {
        pub fn new(articles: usize, height: f32) -> Self {
            let mut store = Store::open_in_memory().expect("in-memory store");
            let source = store
                .add_source(&gutenberg_store::NewSource {
                    folder_id: None,
                    kind: gutenberg_store::SourceKind::Feed,
                    title: "Bench".into(),
                    url: Some("https://bench.example/feed".into()),
                    config: "{}".into(),
                })
                .expect("source");
            let rows: Vec<_> = (0..articles)
                .map(|i| gutenberg_store::NewArticle {
                    source_id: source.id,
                    guid: i.to_string(),
                    url: None,
                    title: format!("Headline number {i} about the news of the day"),
                    author: None,
                    summary_html: None,
                    published_at: Some(1_700_000_000 + i as i64),
                    fetched_at: 1,
                })
                .collect();
            store.insert_articles(&rows).expect("articles");
            let mut app = App::with_store(Config::default(), None, Ok(store), None);
            app.height = height;
            Self(app)
        }

        /// Scrolls one row, as a frame of the scroll bench does, and builds
        /// the view.
        pub fn scroll_and_view(&mut self) {
            let max = list::max_top(self.0.rows.len(), self.0.height);
            self.0.top = if self.0.top >= max { 0 } else { self.0.top + 1 };
            std::hint::black_box(self.0.view());
        }
    }
}

/// For `examples/theme_shots.rs`: the real window, drawn headlessly in a
/// theme's colors, over a small store of sample articles.
#[cfg(feature = "shots")]
#[doc(hidden)]
pub mod shots {
    use super::*;
    use gutenberg_store::{NewArticle, NewSource};

    const SUMMARY: &str = r#"<p>Lawmakers in <a href="https://example.com/capitol">Harrisburg</a>
        passed the budget on Thursday, <em>three months late</em>.</p>
        <h2>What changed</h2>
        <ul><li>Transit gets <strong>$300 million</strong>.</li><li>Schools get a formula fix.</li></ul>
        <blockquote><p>"It is a budget we can live with," the governor said.</p></blockquote>
        <pre>total = base + 0.04 * enrollment</pre>"#;

    /// Writes `path` (iced_test adds `-tiny-skia.png`): the window at `size`
    /// in `tokens`, with an article open, read and starred rows, and a
    /// folder chosen in the sources pane.
    pub fn render(tokens: Tokens, size: (f32, f32), path: &std::path::Path) {
        let mut store = Store::open_in_memory().expect("in-memory store");
        let news = store.folder_named("News").expect("folder").id;
        let feed = |store: &Store, title: &str, folder| {
            store
                .add_source(&NewSource {
                    folder_id: folder,
                    kind: SourceKind::Feed,
                    title: title.into(),
                    url: Some(format!("https://{}.example/feed", title.to_lowercase())),
                    config: "{}".into(),
                })
                .expect("feed")
                .id
        };
        let feeds = [
            feed(&store, "PennLive", Some(news)),
            feed(&store, "ProPublica", Some(news)),
            feed(&store, "LWN", None),
        ];
        const TITLES: [&str; 9] = [
            "State budget passes three months late",
            "Transit agencies brace for winter service cuts",
            "Kernel 7.4 merge window opens",
            "How a small town rebuilt its water system",
            "River flood warning lifted after record crest",
            "School board weighs later start times",
            "Rust 1.99 brings const traits",
            "Museum reopens after two-year renovation",
            "Court hears arguments over voting maps",
        ];
        let now = unix_now();
        let articles: Vec<NewArticle> = TITLES
            .iter()
            .enumerate()
            .map(|(i, title)| NewArticle {
                source_id: feeds[i % feeds.len()],
                guid: i.to_string(),
                url: Some(format!("https://example.com/{i}")),
                title: (*title).into(),
                author: Some("Staff".into()),
                summary_html: Some(SUMMARY.into()),
                published_at: Some(now - i as i64 * 3_600),
                fetched_at: now,
            })
            .collect();
        store.insert_articles(&articles).expect("articles");

        let mut app = App::with_store(Config::default(), None, Ok(store), None);
        app.set_tokens(tokens);
        app.height = size.1;
        app.select(4);
        let _ = app.update(Message::Key(Action::Star));
        app.open(3);
        app.open(4);
        app.select(1);
        let _ = app.update(Message::Key(Action::Star));
        app.open(0);
        app.choose(Entry::Folder(news));
        app.open(0);

        // iced_test draws `Font::DEFAULT` in Fira Sans; with the desktop
        // font in place, the monospace family is the same font the app's
        // default (sans-serif) family is.
        let settings = iced_test::core::Settings {
            default_font: if use_desktop_font() {
                Font::MONOSPACE
            } else {
                Font::DEFAULT
            },
            ..Default::default()
        };
        let mut simulator = iced_test::simulator::Simulator::with_size(
            settings,
            iced::Size::new(size.0, size.1),
            app.view(),
        );
        simulator
            .snapshot(&app.theme())
            .expect("snapshot")
            .matches_image(path)
            .expect("write the screenshot");
    }
}

/// Frame-time percentiles for the scroll bench, on stderr.
fn report(intervals: &[Duration], rows: usize, height: f32) {
    let mut ms: Vec<f64> = intervals.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    ms.sort_by(f64::total_cmp);
    let at = |p: f64| ms[((ms.len() - 1) as f64 * p).round() as usize];
    let slow = ms.iter().filter(|&&m| m > 1000.0 / 60.0 + 0.5).count();
    eprintln!(
        "scroll bench: {} frames over {rows} articles, {height} px tall: \
         p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms; {:.1}% over 16.7 ms",
        ms.len(),
        at(0.5),
        at(0.95),
        ms[ms.len() - 1],
        slow as f64 * 100.0 / ms.len() as f64
    );
}

/// Reloads arrive as messages; a config that fails to load is reported on
/// stderr and the window keeps the last good one.
#[allow(clippy::ptr_arg, reason = "Subscription::run_with passes &PathBuf")]
fn watch_config(path: &PathBuf) -> impl Stream<Item = Message> + use<> {
    let path = path.clone();
    iced::stream::channel(8, async move |output| {
        let watching = config::watch(&path, move |loaded| match loaded {
            Ok(config) => {
                let _ = output.clone().try_send(Message::ConfigChanged(config));
            }
            Err(err) => eprintln!("gutenberg: {err}"),
        });
        match watching {
            Ok(handle) => {
                std::future::pending::<()>().await;
                drop(handle);
            }
            Err(err) => eprintln!("gutenberg: {err}"),
        }
    })
}

/// A desktop font that changed while the window was open.
#[derive(Debug, Clone)]
struct NewFont {
    family: String,
    /// Its font file, read off the UI thread.
    file: Option<Arc<Vec<u8>>>,
}

/// Live theme reload. Two things ask for one: an Omarchy hook through the
/// socket (`gutenberg --reload-theme`), and the inotify watch on Omarchy's
/// theme and fontconfig, for installs without the hooks. Each reads the
/// theme and the font again here, off the UI thread.
fn theme_reloads() -> impl Stream<Item = Message> {
    use gutenberg_core::ipc;
    use gutenberg_theme::{font, watch};
    use iced::futures::{channel::mpsc, SinkExt, StreamExt};
    iced::stream::channel(4, async |mut output| {
        let (asks, mut asked) = mpsc::unbounded::<()>();
        let on_watch = {
            let asks = asks.clone();
            move || {
                let _ = asks.unbounded_send(());
            }
        };
        let _watching = watch::watch(watch::watched(), on_watch)
            .map_err(|err| eprintln!("gutenberg: watching the theme: {err}"))
            .ok();
        let _listening = ipc::socket_path().and_then(|path| {
            ipc::listen(&path, move |ipc::Command::ReloadTheme| {
                let _ = asks.unbounded_send(());
            })
            .map_err(|err| eprintln!("gutenberg: {err}"))
            .ok()
        });
        let mut family = font::monospace().map(|face| face.family);
        while asked.next().await.is_some() {
            let tokens = Tokens::omarchy();
            let font = match font::monospace() {
                Some(face) if family.as_deref() != Some(face.family.as_str()) => {
                    family = Some(face.family.clone());
                    let file = face.file.and_then(|path| std::fs::read(path).ok());
                    Some(NewFont {
                        family: face.family,
                        file: file.map(Arc::new),
                    })
                }
                _ => None,
            };
            if output
                .send(Message::ThemeReloaded(tokens, font))
                .await
                .is_err()
            {
                return;
            }
        }
    })
}

/// Starts the web worker, hands the app its handle, then relays each
/// answer.
fn web_worker() -> impl Stream<Item = Message> {
    use iced::futures::{SinkExt, StreamExt};
    iced::stream::channel(16, async |mut output| {
        let (answer_tx, mut answers) = iced::futures::channel::mpsc::unbounded();
        let web = match Web::spawn(move |answer| {
            let _ = answer_tx.unbounded_send(answer);
        }) {
            Ok(web) => web,
            Err(err) => {
                eprintln!("gutenberg: web: {err}");
                return;
            }
        };
        if output.send(Message::WebReady(web)).await.is_err() {
            return;
        }
        while let Some(answer) = answers.next().await {
            if output
                .send(Message::WebAnswer(Arc::new(answer)))
                .await
                .is_err()
            {
                return;
            }
        }
    })
}

/// Opens `url` in the default browser. `xdg-open` runs detached; a thread
/// reaps it so the window never waits on it.
fn open_in_browser(url: &str) {
    use std::process::{Command, Stdio};
    let spawned = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => eprintln!("gutenberg: opening {url}: {err}"),
    }
}

/// Where a dragged row will land.
fn drop_line<'a>(accent: Rgb) -> Element<'a, Message> {
    container(Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(2.0))
        .style(move |_| container::Style {
            background: Some(color(accent).into()),
            ..Default::default()
        })
        .into()
}

/// A command's name and its keys, highlighted when the cursor is on it.
fn command_row<'a>(
    command: &'static commands::Command,
    highlighted: bool,
    t: &Tokens,
) -> Element<'a, Message> {
    let keys = command.keys.join("  ");
    let selection = t.selection;
    container(
        row![
            text(command.name)
                .size(14)
                .color(color(t.foreground))
                .width(Length::Fill),
            text(keys)
                .size(13)
                .font(Font::MONOSPACE)
                .color(color(t.muted)),
        ]
        .spacing(16),
    )
    .padding([7, 12])
    .width(Length::Fill)
    .style(move |_| container::Style {
        background: highlighted.then(|| color(selection).into()),
        ..Default::default()
    })
    .into()
}

fn note<'a>(message: String, muted: Rgb) -> Element<'a, Message> {
    container(text(message).size(14).color(color(muted)))
        .padding(16)
        .into()
}

fn pane<'a>(
    content: Element<'a, Message>,
    background: Rgb,
    border_color: Rgb,
    portion: u16,
) -> Element<'a, Message> {
    container(content)
        .width(Length::FillPortion(portion))
        .height(Length::Fill)
        // Whatever a pane holds is drawn inside it, never over its neighbor.
        .clip(true)
        .style(move |_| container::Style {
            background: Some(color(background).into()),
            border: Border {
                color: color(border_color),
                width: 1.0,
                radius: border::radius(0),
            },
            ..Default::default()
        })
        .into()
}

fn color(c: Rgb) -> Color {
    Color::from_rgb8(c.r, c.g, c.b)
}

/// iced's theme for `t`, so what iced styles itself (scrollbars, checkbox
/// and text-input defaults) matches the panes.
fn iced_theme(t: &Tokens) -> iced::Theme {
    iced::Theme::custom(
        "Gutenberg",
        iced::theme::Palette {
            background: color(t.background),
            text: color(t.foreground),
            primary: color(t.accent),
            success: color(t.accent),
            warning: color(t.accent),
            danger: color(t.accent),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gutenberg_core::favicon::Format;
    use gutenberg_store::{NewArticle, NewSource, SourceKind};

    fn app_with(articles: usize) -> App {
        let mut store = Store::open_in_memory().unwrap();
        let source = store
            .add_source(&NewSource {
                folder_id: None,
                kind: SourceKind::Feed,
                title: "Feed".into(),
                url: Some("https://site.example/feed".into()),
                config: "{}".into(),
            })
            .unwrap();
        let rows: Vec<NewArticle> = (0..articles)
            .map(|i| NewArticle {
                source_id: source.id,
                guid: i.to_string(),
                url: None,
                title: format!("Article {i}"),
                author: None,
                summary_html: Some(format!("<p>Body {i}</p>")),
                published_at: Some(1_000 + i as i64),
                fetched_at: 1,
            })
            .collect();
        store.insert_articles(&rows).unwrap();
        let mut app = App::with_store(Config::default(), None, Ok(store), None);
        app.height = 520.0; // 10 whole rows
        app
    }

    #[test]
    fn opening_marks_read_in_the_store() {
        let mut app = app_with(3);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Open));
        let id = app.rows[1].id;
        assert!(app.rows[1].read);
        let stored = app.store.as_ref().unwrap().list_articles(None, 10).unwrap();
        assert!(stored.iter().find(|r| r.id == id).unwrap().read);
        assert_eq!(
            app.opened.as_ref().unwrap().body,
            [article::Block::Paragraph(vec![article::Span {
                text: "Body 1".into(),
                style: article::Style::default(),
                link: None,
            }])]
        );
    }

    #[test]
    fn images_load_for_the_open_article_only() {
        let mut app = app_with(2);
        // Port 9 on loopback: the worker's own request fails without
        // leaving the machine; the answers below are delivered by hand.
        let url = "http://127.0.0.1:9/a.png".to_owned();
        let source_id = app.rows[0].source_id;
        app.store
            .as_mut()
            .unwrap()
            .insert_articles(&[NewArticle {
                source_id,
                guid: "pic".into(),
                url: None,
                title: "With a picture".into(),
                author: None,
                summary_html: Some(format!("<p>Look</p><img src=\"{url}\">")),
                published_at: Some(5_000),
                fetched_at: 1,
            }])
            .unwrap();
        app.reload();
        let _ = app.update(Message::WebReady(Web::spawn(|_| {}).unwrap()));

        // Newest first: the article with the picture is the top row.
        let _ = app.update(Message::Key(Action::First));
        let _ = app.update(Message::Key(Action::Open));
        assert!(matches!(
            app.pictures.get(&url),
            Some(article::Picture::Loading)
        ));
        let failed = Answer::ImageFailed { url: url.clone() };
        let _ = app.update(Message::WebAnswer(Arc::new(failed)));
        assert!(matches!(
            app.pictures.get(&url),
            Some(article::Picture::Failed)
        ));

        // Another article: its images replace the last one's, and a late
        // answer for the old one is dropped.
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Open));
        assert!(app.pictures.is_empty());
        let late = Answer::ImageFailed { url: url.clone() };
        let _ = app.update(Message::WebAnswer(Arc::new(late)));
        assert!(app.pictures.is_empty());
    }

    fn shown_text(app: &App) -> String {
        app.opened
            .as_ref()
            .unwrap()
            .shown()
            .iter()
            .filter_map(|b| match b {
                article::Block::Paragraph(spans) => {
                    Some(spans.iter().map(|s| s.text.as_str()).collect::<String>())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    #[test]
    fn v_swaps_the_summary_for_the_full_article_and_back() {
        let mut app = app_with(2);
        // Loopback port 9: the worker's own requests fail without leaving
        // the machine; its answers below are delivered by hand.
        let page = "http://127.0.0.1:9/story".to_owned();
        let source_id = app.rows[0].source_id;
        app.store
            .as_mut()
            .unwrap()
            .insert_articles(&[NewArticle {
                source_id,
                guid: "story".into(),
                url: Some(page.clone()),
                title: "A story".into(),
                author: None,
                summary_html: Some("<p>Short</p>".into()),
                published_at: Some(5_000),
                fetched_at: 1,
            }])
            .unwrap();
        app.reload();
        let _ = app.update(Message::WebReady(Web::spawn(|_| {}).unwrap()));
        let _ = app.update(Message::Key(Action::First));
        let _ = app.update(Message::Key(Action::Open));
        assert_eq!(shown_text(&app), "Short");

        // While the page loads, the summary stays.
        let _ = app.update(Message::Key(Action::Reader));
        assert!(matches!(
            app.opened.as_ref().unwrap().reader,
            Reader::Loading
        ));
        assert_eq!(shown_text(&app), "Short");
        let picture = "http://127.0.0.1:9/b.png".to_owned();
        let full = Answer::Page {
            url: page.clone(),
            html: format!("<p>The whole story</p><img src=\"{picture}\">"),
        };
        let _ = app.update(Message::WebAnswer(Arc::new(full.clone())));
        assert_eq!(shown_text(&app), "The whole story");
        assert!(
            app.pictures.contains_key(&picture),
            "its images are asked for"
        );

        // v again: back to the summary; a page arriving now changes nothing.
        let _ = app.update(Message::Key(Action::Reader));
        assert_eq!(shown_text(&app), "Short");
        assert!(app.pictures.is_empty());
        let _ = app.update(Message::WebAnswer(Arc::new(full)));
        assert_eq!(shown_text(&app), "Short");
    }

    #[test]
    fn without_a_page_the_summary_stays_and_says_why() {
        let mut app = app_with(2);
        let _ = app.update(Message::Key(Action::Open));
        // app_with's articles have no link.
        let _ = app.update(Message::Key(Action::Reader));
        assert!(matches!(
            &app.opened.as_ref().unwrap().reader,
            Reader::Failed(why) if why == "this article has no link"
        ));
        assert_eq!(shown_text(&app), "Body 1", "newest first");
    }

    #[test]
    fn a_failed_page_keeps_the_summary_and_a_late_one_is_dropped() {
        let mut app = app_with(3);
        let page = |i: usize| format!("http://127.0.0.1:9/{i}");
        for i in 0..2 {
            app.rows[i].url = Some(page(i));
        }
        let _ = app.update(Message::Key(Action::Open));
        let _ = app.update(Message::Key(Action::Reader));
        let failed = Answer::PageFailed {
            url: page(0),
            error: "HTTP 404 Not Found".into(),
        };
        let _ = app.update(Message::WebAnswer(Arc::new(failed)));
        assert!(matches!(
            &app.opened.as_ref().unwrap().reader,
            Reader::Failed(why) if why == "HTTP 404 Not Found"
        ));
        assert_eq!(shown_text(&app), "Body 2", "newest first");

        // Ask for one page, move on, and the first answer finds nobody.
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Open));
        let _ = app.update(Message::Key(Action::Reader));
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Open));
        let late = Answer::Page {
            url: page(1),
            html: "<p>Too late</p>".into(),
        };
        let _ = app.update(Message::WebAnswer(Arc::new(late)));
        assert_eq!(shown_text(&app), "Body 0");
    }

    #[test]
    fn j_past_the_screen_scrolls_and_gg_g_jump() {
        let mut app = app_with(100);
        for _ in 0..12 {
            let _ = app.update(Message::Key(Action::Down));
        }
        assert_eq!((app.selected, app.top), (12, 3));
        let _ = app.update(Message::Key(Action::Last));
        assert_eq!((app.selected, app.top), (99, 90));
        let _ = app.update(Message::Key(Action::PendingG));
        let _ = app.update(Message::Key(Action::First));
        assert_eq!((app.selected, app.top), (0, 0));
    }

    #[test]
    fn the_wheel_scrolls_without_moving_past_the_end() {
        let mut app = app_with(100);
        let _ = app.update(Message::Scroll(ScrollDelta::Lines { x: 0.0, y: -2.0 }));
        assert_eq!(app.top, 6);
        let _ = app.update(Message::Scroll(ScrollDelta::Lines { x: 0.0, y: -100.0 }));
        assert_eq!(app.top, 90);
    }

    #[test]
    fn the_desktop_preference_picks_the_built_in_theme_but_never_overrides_omarchy() {
        use iced::theme::Mode;
        let mut app = app_with(1);
        // No Omarchy theme: light and dark follow the desktop, and no
        // preference is dark.
        let _ = app.update(Message::SystemTheme(Mode::Light));
        assert_eq!(app.tokens, Tokens::fallback(Scheme::Light));
        assert_eq!(
            app.theme().palette().background,
            color(app.tokens.background)
        );
        let _ = app.update(Message::SystemTheme(Mode::None));
        assert_eq!(app.tokens, Tokens::fallback(Scheme::Dark));

        // An Omarchy theme wins whatever the desktop says.
        let omarchy = Tokens::from_colors_toml("background = \"#101315\"").unwrap();
        let _ = app.update(Message::ThemeReloaded(Some(omarchy.clone()), None));
        let _ = app.update(Message::SystemTheme(Mode::Light));
        assert_eq!(app.tokens, omarchy);

        // Omarchy removed: back to the built-in theme the desktop asks for.
        let _ = app.update(Message::ThemeReloaded(None, None));
        assert_eq!(app.tokens, Tokens::fallback(Scheme::Light));
    }

    #[test]
    fn a_bigger_font_keeps_the_selected_row_on_screen() {
        let mut app = app_with(100);
        for _ in 0..9 {
            let _ = app.update(Message::Key(Action::Down));
        }
        // Row 9 is the last of 10 whole rows at 14 px.
        assert_eq!((app.selected, app.top), (9, 0));
        let mut config = Config::default();
        config.font.size = 28;
        let _ = app.update(Message::ConfigChanged(config.clone()));
        // Twice the size, half the rows: 5 fit, so row 9 is the fifth.
        assert_eq!(app.top, 5);
        config.font.size = 14;
        let _ = app.update(Message::ConfigChanged(config));
        // Back to 10 rows; `G` lands where it did before.
        let _ = app.update(Message::Key(Action::Last));
        assert_eq!((app.selected, app.top), (99, 90));
    }

    #[test]
    fn a_write_from_elsewhere_reloads_and_keeps_the_selection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("g.db");
        let mut writer = Store::open(&path).unwrap();
        let source = writer
            .add_source(&NewSource {
                folder_id: None,
                kind: SourceKind::Feed,
                title: "Feed".into(),
                url: Some("https://site.example/feed".into()),
                config: "{}".into(),
            })
            .unwrap();
        let article = |guid: &str, at: i64| NewArticle {
            source_id: source.id,
            guid: guid.into(),
            url: None,
            title: guid.into(),
            author: None,
            summary_html: None,
            published_at: Some(at),
            fetched_at: 1,
        };
        writer.insert_articles(&[article("old", 1)]).unwrap();

        let mut app = App::new(Config::default(), None, Some(&path));
        assert_eq!(app.rows.len(), 1);

        // The refresh thread adds a newer article on its own connection.
        writer.insert_articles(&[article("new", 2)]).unwrap();
        let _ = app.update(Message::PollStore);
        assert_eq!(app.rows.len(), 2);
        assert_eq!(app.rows[app.selected].title, "old");
    }

    /// Two folders' worth: Tech holds "a" (2 articles), "b" is unfiled (1).
    fn app_with_folders() -> (App, i64, i64, i64) {
        let mut store = Store::open_in_memory().unwrap();
        let tech = store.folder_named("Tech").unwrap().id;
        let add = |store: &Store, name: &str, folder| {
            store
                .add_source(&NewSource {
                    folder_id: folder,
                    kind: SourceKind::Feed,
                    title: name.into(),
                    url: Some(format!("https://{name}.example/feed")),
                    config: "{}".into(),
                })
                .unwrap()
                .id
        };
        let a = add(&store, "a", Some(tech));
        let b = add(&store, "b", None);
        let article = |source_id, guid: &str, at| NewArticle {
            source_id,
            guid: guid.into(),
            url: None,
            title: guid.into(),
            author: None,
            summary_html: None,
            published_at: Some(at),
            fetched_at: 1,
        };
        store
            .insert_articles(&[
                article(a, "a1", 3),
                article(a, "a2", 2),
                article(b, "b1", 1),
            ])
            .unwrap();
        let mut app = App::with_store(Config::default(), None, Ok(store), None);
        app.height = 520.0;
        (app, tech, a, b)
    }

    fn counts(app: &App, entry: Entry) -> i64 {
        sidebar::count(entry, &app.sources, &app.unread, &app.view_counts)
    }

    const ALL: Entry = Entry::View(View::All);

    #[test]
    fn counts_drop_as_articles_are_read() {
        let (mut app, tech, a, _) = app_with_folders();
        let a_row = Entry::Source { id: a, filed: true };
        assert_eq!(
            (
                counts(&app, ALL),
                counts(&app, Entry::Folder(tech)),
                counts(&app, a_row)
            ),
            (3, 2, 2)
        );
        let _ = app.update(Message::Key(Action::Open)); // a1, newest
        assert_eq!(
            (
                counts(&app, ALL),
                counts(&app, Entry::Folder(tech)),
                counts(&app, a_row)
            ),
            (2, 1, 1)
        );
        // Opening it again doesn't count twice.
        let _ = app.update(Message::Key(Action::Open));
        assert_eq!(counts(&app, ALL), 2);
        // And a reload from the store agrees.
        app.reload();
        assert_eq!(counts(&app, Entry::Folder(tech)), 1);
    }

    #[test]
    fn icons_the_refresh_saves_show_at_the_next_reload() {
        let (mut app, _, a, b) = app_with_folders();
        let dir = tempfile::tempdir().unwrap();
        let icons = Icons::new(dir.path());
        app.icons = Some(icons.clone());
        app.reload();
        assert!(app.source_icons.is_empty());

        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        icons
            .save("https://a.example/feed", Some((svg, Format::Svg)))
            .unwrap();
        icons.save("https://b.example/feed", None).unwrap();
        app.reload();
        assert!(matches!(app.source_icons.get(&a), Some(Icon::Vector(_))));
        assert!(!app.source_icons.contains_key(&b), "a miss draws no icon");
    }

    #[test]
    fn a_click_filters_the_list() {
        let (mut app, tech, _, b) = app_with_folders();
        let b_row = Entry::Source {
            id: b,
            filed: false,
        };
        let _ = app.update(Message::Press(b_row));
        let _ = app.update(Message::Release);
        assert_eq!(app.filter, Filter::Source(b));
        assert_eq!(app.rows.len(), 1);
        let _ = app.update(Message::Press(Entry::Folder(tech)));
        let _ = app.update(Message::Release);
        assert_eq!(app.rows.len(), 2);
        let _ = app.update(Message::Press(ALL));
        let _ = app.update(Message::Release);
        assert_eq!(app.rows.len(), 3);
    }

    #[test]
    fn dragging_a_source_onto_a_folder_files_it_in_the_store() {
        let (mut app, tech, a, b) = app_with_folders();
        let b_row = Entry::Source {
            id: b,
            filed: false,
        };
        let _ = app.update(Message::Press(b_row));
        let _ = app.update(Message::Over(Entry::Folder(tech)));
        let _ = app.update(Message::Release);
        // No filter change: it was a drag, not a click.
        assert_eq!(app.filter, Filter::All);
        let stored = app.store.as_ref().unwrap().sources().unwrap();
        let ids: Vec<_> = stored.iter().map(|s| (s.id, s.folder_id)).collect();
        assert_eq!(ids, [(a, Some(tech)), (b, Some(tech))]);
        assert_eq!(counts(&app, Entry::Folder(tech)), 3);

        // Back out of the folder, by dropping below the rows.
        let _ = app.update(Message::Press(Entry::Source { id: a, filed: true }));
        let _ = app.update(Message::Over(Entry::Unfiled));
        let _ = app.update(Message::Release);
        let stored = app.store.as_ref().unwrap().sources().unwrap();
        assert_eq!(stored.iter().find(|s| s.id == a).unwrap().folder_id, None);
    }

    #[test]
    fn a_fresh_store_opens_on_add_sources_and_enter_adds_the_starters() {
        let mut app = App::with_store(
            Config::default(),
            None,
            Ok(Store::open_in_memory().unwrap()),
            None,
        );
        assert_eq!(app.onboarding.as_ref().unwrap().picks().len(), 12);
        let _ = app.update(Message::Onboarding(onboarding::Action::Add));
        assert!(app.onboarding.is_none());
        let store = app.store.as_ref().unwrap();
        assert_eq!(store.sources().unwrap().len(), 12);
        let folders: Vec<String> = store
            .folders()
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(
            folders,
            [
                "World",
                "U.S.",
                "Investigations",
                "Science",
                "Climate",
                "Technology",
                "Linux",
                "Programming"
            ]
        );
        assert_eq!(app.sources.len(), 12);
    }

    #[test]
    fn a_press_on_a_opens_add_sources_with_nothing_checked() {
        let (mut app, ..) = app_with_folders();
        assert!(app.onboarding.is_none());
        let _ = app.update(Message::Key(Action::AddSources));
        assert!(app.onboarding.as_ref().unwrap().picks().is_empty());
        let _ = app.update(Message::Onboarding(onboarding::Action::Close));
        assert!(app.onboarding.is_none());
    }

    fn type_in_palette(app: &mut App, text: &str) {
        for c in text.chars() {
            let _ = app.update(Message::Palette(palette::Input::Type(c.to_string())));
        }
    }

    #[test]
    fn the_palette_runs_what_you_pick() {
        let mut app = app_with(3);
        let _ = app.update(Message::Key(Action::Open));
        let _ = app.update(Message::Key(Action::Palette));
        // A j in the palette is a letter, not a move down the list.
        type_in_palette(&mut app, "j");
        assert_eq!(app.selected, 0);
        let _ = app.update(Message::Palette(palette::Input::Backspace));
        type_in_palette(&mut app, "reader");
        let _ = app.update(Message::Palette(palette::Input::Run));
        assert!(app.overlay.is_none());
        // app_with's articles have no link, so the reader view says why.
        assert!(matches!(
            &app.opened.as_ref().unwrap().reader,
            Reader::Failed(_)
        ));
    }

    #[test]
    fn esc_closes_the_palette_without_running_anything() {
        let mut app = app_with(3);
        let _ = app.update(Message::Key(Action::Palette));
        type_in_palette(&mut app, "next");
        let _ = app.update(Message::Palette(palette::Input::Close));
        assert!(app.overlay.is_none());
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn a_click_in_the_palette_runs_that_command() {
        let mut app = app_with(3);
        let _ = app.update(Message::Key(Action::Palette));
        let _ = app.update(Message::RunCommand(Action::Last));
        assert!(app.overlay.is_none());
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn help_opens_and_closes() {
        let mut app = app_with(3);
        let _ = app.update(Message::Key(Action::Help));
        assert!(matches!(app.overlay, Some(Overlay::Help)));
        let _ = app.update(Message::CloseOverlay);
        assert!(app.overlay.is_none());
    }

    fn stored(app: &App, id: i64) -> ArticleRow {
        let rows = app
            .store
            .as_ref()
            .unwrap()
            .list_articles(None, 100)
            .unwrap();
        rows.into_iter().find(|r| r.id == id).unwrap()
    }

    #[test]
    fn s_stars_and_unstars_the_selected_article() {
        let (mut app, ..) = app_with_folders();
        let _ = app.update(Message::Key(Action::Down));
        let id = app.rows[1].id;
        let _ = app.update(Message::Key(Action::Star));
        assert!(app.rows[1].starred && stored(&app, id).starred);
        assert!(
            !stored(&app, app.rows[0].id).starred,
            "only the selected one"
        );
        let _ = app.update(Message::Key(Action::Star));
        assert!(!app.rows[1].starred && !stored(&app, id).starred);
    }

    #[test]
    fn m_makes_a_read_article_unread_and_the_count_follows() {
        let (mut app, _, a, _) = app_with_folders();
        let a_row = Entry::Source { id: a, filed: true };
        let _ = app.update(Message::Key(Action::Open));
        let id = app.rows[0].id;
        assert_eq!(counts(&app, a_row), 1);
        let _ = app.update(Message::Key(Action::ToggleRead));
        assert!(!stored(&app, id).read);
        assert_eq!(counts(&app, a_row), 2);
        assert_eq!(app.rows[app.selected].id, id, "the selection stays put");
        let _ = app.update(Message::Key(Action::ToggleRead));
        assert!(stored(&app, id).read);
        assert_eq!(counts(&app, a_row), 1);
    }

    #[test]
    fn shift_m_marks_only_the_chosen_folder_read() {
        let (mut app, tech, _, b) = app_with_folders();
        app.choose(Entry::Folder(tech));
        let _ = app.update(Message::Key(Action::MarkFolderRead));
        assert_eq!(counts(&app, Entry::Folder(tech)), 0);
        assert_eq!(
            counts(
                &app,
                Entry::Source {
                    id: b,
                    filed: false
                }
            ),
            1
        );
        assert!(app.rows.iter().all(|r| r.read));

        app.choose(ALL);
        let _ = app.update(Message::Key(Action::MarkFolderRead));
        assert_eq!(counts(&app, ALL), 0);
    }

    fn search_keys(app: &mut App, typed: &str) {
        for c in typed.chars() {
            let _ = app.update(Message::Search(palette::Input::Type(c.to_string())));
        }
    }

    fn titles(app: &App) -> Vec<&str> {
        app.rows.iter().map(|r| r.title.as_str()).collect()
    }

    #[test]
    fn slash_searches_as_you_type_and_esc_brings_the_list_back() {
        let mut app = app_with(12);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Search));
        assert!(app.rows.is_empty(), "nothing typed, nothing found");
        // "body 1" matches Body 1 and Body 10, 11 by prefix; newest first.
        search_keys(&mut app, "body 1");
        assert_eq!(titles(&app), ["Article 11", "Article 10", "Article 1"]);
        search_keys(&mut app, "1");
        assert_eq!(titles(&app), ["Article 11"]);
        let _ = app.update(Message::Search(palette::Input::Backspace));
        assert_eq!(app.rows.len(), 3);

        let _ = app.update(Message::Search(palette::Input::Close));
        assert!(app.search.is_none());
        assert_eq!(app.rows.len(), 12);
    }

    #[test]
    fn enter_hands_the_keys_back_to_the_results() {
        let mut app = app_with(12);
        let _ = app.update(Message::Key(Action::Search));
        search_keys(&mut app, "body 1");
        // While typing, j is a letter: no match for "body 1j".
        search_keys(&mut app, "j");
        assert!(app.rows.is_empty());
        let _ = app.update(Message::Search(palette::Input::Backspace));
        let _ = app.update(Message::Search(palette::Input::Run));
        let _ = app.update(Message::Key(Action::Down));
        assert_eq!(app.rows[app.selected].title, "Article 10");
        // `/` again edits the same search; Esc from the list ends it.
        let _ = app.update(Message::Key(Action::Search));
        assert_eq!(app.search.as_ref().unwrap().query, "body 1");
        let _ = app.update(Message::Search(palette::Input::Run));
        let _ = app.update(Message::Key(Action::ClearSearch));
        assert_eq!(app.rows.len(), 12);
    }

    #[test]
    fn choosing_a_feed_ends_the_search() {
        let (mut app, tech, ..) = app_with_folders();
        let _ = app.update(Message::Key(Action::Search));
        search_keys(&mut app, "b1");
        assert_eq!(titles(&app), ["b1"]);
        app.choose(Entry::Folder(tech));
        assert!(app.search.is_none());
        assert_eq!(titles(&app), ["a1", "a2"]);
    }

    #[test]
    fn a_view_key_narrows_the_chosen_folder_and_a_returns() {
        let (mut app, tech, ..) = app_with_folders();
        let _ = app.update(Message::Key(Action::Open)); // a1 is read
        let _ = app.update(Message::Key(Action::View(View::Unread)));
        assert_eq!(titles(&app), ["a2", "b1"]);
        // Choosing a folder keeps the view.
        app.choose(Entry::Folder(tech));
        assert_eq!(titles(&app), ["a2"]);
        let _ = app.update(Message::Key(Action::View(View::All)));
        assert_eq!(titles(&app), ["a1", "a2"]);
        assert_eq!(app.filter, Filter::Folder(tech));
    }

    #[test]
    fn a_view_row_shows_that_view_everywhere() {
        let (mut app, tech, ..) = app_with_folders();
        app.choose(Entry::Folder(tech));
        let _ = app.update(Message::Press(Entry::View(View::Unread)));
        let _ = app.update(Message::Release);
        assert_eq!((app.filter, app.view), (Filter::All, View::Unread));
        assert_eq!(titles(&app), ["a1", "a2", "b1"]);
    }

    #[test]
    fn starring_fills_the_starred_view_and_its_count() {
        let (mut app, ..) = app_with_folders();
        let starred = Entry::View(View::Starred);
        let _ = app.update(Message::Key(Action::Last)); // b1
        let _ = app.update(Message::Key(Action::Star));
        assert_eq!(counts(&app, starred), 1);
        let _ = app.update(Message::Key(Action::View(View::Starred)));
        assert_eq!(titles(&app), ["b1"]);
        // Unstarring takes it out of the view and the count.
        let _ = app.update(Message::Key(Action::Star));
        assert!(app.rows.is_empty());
        assert_eq!(counts(&app, starred), 0);
    }

    #[test]
    fn today_holds_what_came_since_midnight() {
        let (mut app, _, a, _) = app_with_folders();
        let _ = app.update(Message::Key(Action::View(View::Today)));
        assert!(app.rows.is_empty(), "the fixture's articles are from 1970");
        app.store
            .as_mut()
            .unwrap()
            .insert_articles(&[NewArticle {
                source_id: a,
                guid: "now".into(),
                url: None,
                title: "Just in".into(),
                author: None,
                summary_html: None,
                published_at: Some(unix_now()),
                fetched_at: unix_now(),
            }])
            .unwrap();
        app.reload();
        assert_eq!(titles(&app), ["Just in"]);
        assert_eq!(counts(&app, Entry::View(View::Today)), 1);
    }
}
