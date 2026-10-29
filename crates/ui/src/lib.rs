//! See CLAUDE.md for what this crate owns.

mod article;
mod commands;
mod hints;
mod list;
mod onboarding;
mod palette;
mod panes;
mod share;
mod sidebar;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gutenberg_core::catalog;
use gutenberg_core::config::{self, Arrangement, Config};
use gutenberg_core::favicon::Icons;
use gutenberg_core::sync::{unix_now, Status};
use gutenberg_core::web::{self, Answer, Web};
use gutenberg_core::{ipc, keyword, mcp, secrets};
use gutenberg_store::{ArticleRow, Filter, Folder, Source, SourceKind, Store, View, ViewCounts};
use gutenberg_theme::{Rgb, Scheme, Tokens};
use iced::advanced::widget::{self, operation, Operation};
use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use iced::futures::Stream;
use iced::keyboard;
use iced::mouse::ScrollDelta;
use iced::widget::pane_grid::{self, PaneGrid};
use iced::widget::{
    column, container, image, mouse_area, opaque, rich_text, row, scrollable, span, stack, svg,
    text, Column, Row, Space,
};
use iced::{
    border, font, window, Alignment, Border, Color, Element, Font, Length, Rectangle, Size,
    Subscription, Task, Vector,
};

use list::Action;
use panes::Arrange;
use sidebar::{Entry, Move, Step};

/// Most articles kept in the list; the PRD plans for 100k.
const LIST_LIMIT: u32 = 100_000;
/// How often to look for writes from the refresh thread or a terminal.
const STORE_POLL: Duration = Duration::from_secs(1);
/// How often the refresh's progress is read while one runs.
const PROGRESS_POLL: Duration = Duration::from_millis(100);
/// Thickness of the refresh's progress bar along the top of the list.
const PROGRESS_BAR: f32 = 3.0;
/// Most search results kept: the newest this many matches. Enough to
/// read through, and cheap enough to run on every key (S3.6 bench).
const SEARCH_LIMIT: u32 = 500;
/// Height of the search line above the list.
const SEARCH_BAR: f32 = 36.0;
/// The key bar along the bottom of the window.
const HINT_BAR: f32 = 28.0;
/// Site icons in the sources pane, square.
const ICON_SIZE: f32 = 16.0;
/// The drop target below the last source, scrolled with the rows: a drop
/// there takes a source out of its folder.
const UNFILED_STRIP: f32 = 32.0;
/// The sources pane's scrollable, and the row the keys are on in it.
const SOURCES_SCROLL: &str = "sources";
const SOURCES_CURSOR: &str = "sources-cursor";

/// Opens the window and blocks until it closes. When `config_path` is set,
/// edits to that file apply while the window is open. `store_path` is the
/// SQLite file to read articles from. `inbox` takes the socket's commands
/// for the window.
pub fn run(
    config: Config,
    config_path: Option<PathBuf>,
    store_path: Option<PathBuf>,
    inbox: Inbox,
    status: Option<Status>,
) -> iced::Result {
    use_desktop_font();
    iced::application(
        move || {
            let mut app = App::new(config.clone(), config_path.clone(), store_path.as_deref());
            app.inbox = Some(inbox.clone());
            app.status = status.clone();
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
        // Hyprland draws the borders; no client-side decorations. macOS
        // has nothing else to move, close or zoom a window with, so there
        // it keeps its title bar.
        decorations: cfg!(target_os = "macos"),
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

/// Hands the window commands from the socket (`ipc::Command::ReloadTheme`
/// and `Focus`); [`run`] takes the [`Inbox`] end.
#[derive(Clone)]
pub struct Outbox(UnboundedSender<ipc::Command>);

impl Outbox {
    /// `false` once the window has closed.
    pub fn send(&self, command: ipc::Command) -> bool {
        self.0.unbounded_send(command).is_ok()
    }
}

/// The window's end of [`inbox`]. Its stream takes the receiver once.
#[derive(Clone)]
pub struct Inbox(Arc<std::sync::Mutex<Option<Channel>>>);

type Channel = (
    UnboundedSender<ipc::Command>,
    UnboundedReceiver<ipc::Command>,
);

/// One window, one inbox: always the same subscription.
impl std::hash::Hash for Inbox {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl Inbox {
    fn take(&self) -> Option<Channel> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

/// A channel from the socket to the window.
pub fn inbox() -> (Outbox, Inbox) {
    let (tx, rx) = iced::futures::channel::mpsc::unbounded();
    (
        Outbox(tx.clone()),
        Inbox(Arc::new(std::sync::Mutex::new(Some((tx, rx))))),
    )
}

/// Draws all text in the desktop's font: fontconfig's `monospace` family,
/// which `omarchy-font-set` sets, stands in for iced's sans-serif and
/// monospace families, so plain, bold, italic and code text all use it.
/// Without fontconfig, or for a family no font file provides, iced's own
/// choices stay; then this returns `false`.
#[cfg(not(target_os = "macos"))]
fn use_desktop_font() -> bool {
    gutenberg_theme::font::monospace().is_some_and(|face| set_font(&face.family, None))
}

/// macOS has no desktop font to follow. Text stays in iced's default
/// sans-serif, which cosmic-text resolves to the system font, and only code
/// is pinned to Menlo. `false` if no font file has it, which no Mac lacks.
#[cfg(target_os = "macos")]
fn use_desktop_font() -> bool {
    let family = gutenberg_theme::font::MACOS_MONOSPACE;
    with_installed(family, None, |db| db.set_monospace_family(family))
}

/// Points iced's sans-serif and monospace families at `family`, after
/// loading `file` (the family's font file) if given. Loading a file bumps
/// the font system's version, which makes iced reshape every paragraph, so
/// text already on screen changes too; it also brings in a font installed
/// after the window opened.
fn set_font(family: &str, file: Option<Vec<u8>>) -> bool {
    with_installed(family, file, |db| {
        db.set_sans_serif_family(family);
        db.set_monospace_family(family);
    })
}

/// Runs `set` on the font database if some font file provides `family`,
/// after loading `file` if given. Without the family, iced's own choices
/// stay and this returns `false`.
fn with_installed(
    family: &str,
    file: Option<Vec<u8>>,
    set: impl FnOnce(&mut iced::advanced::graphics::text::cosmic_text::fontdb::Database),
) -> bool {
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
    set(db);
    true
}

/// Text sizes in the code are for a 14 px font; `[font] size` scales the
/// whole window from there, so rows and padding grow with the text.
/// `arrangement`'s place in `panes::ARRANGEMENTS`, and in `App::sizes`.
fn slot(arrangement: Arrangement) -> usize {
    panes::ARRANGEMENTS
        .iter()
        .position(|a| *a == arrangement)
        .unwrap_or(0)
}

/// A store setting, or `None` with a line on stderr when the store can't
/// be read.
fn read_setting(store: &Result<Store, String>, key: &str) -> Option<String> {
    match store {
        Ok(store) => match store.setting(key) {
            Ok(setting) => setting,
            Err(e) => {
                eprintln!("gutenberg: reading {key}: {e}");
                None
            }
        },
        Err(_) => None,
    }
}

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
    /// Why each source failed its last refresh, for those that did.
    source_errors: HashMap<i64, String>,
    tools: Tools,
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
    /// Width of the window, for the narrowest a pane may be dragged to.
    width: f32,
    /// Where the panes sit.
    arrangement: Arrangement,
    /// Whether `arrangement` came from the store (`L` or the palette), so a
    /// config change keeps it; else it follows `[layout]`.
    arrangement_stored: bool,
    /// The arrangement before reading, which `Esc` goes back to.
    before_reading: Option<Arrangement>,
    /// Each arrangement's divider positions, in `panes::ARRANGEMENTS` order.
    sizes: [panes::Sizes; 4],
    /// Which of `sizes` came from the store (a drag or a key), so a config
    /// change keeps them; else they follow `[layout]`.
    sizes_stored: [bool; 4],
    /// The panes as `pane_grid` draws them, built from the arrangement and
    /// its sizes.
    grid: pane_grid::State<panes::Id>,
    /// The grid's dividers, each with what moving it changes.
    dividers: Vec<(pane_grid::Split, panes::Divider)>,
    opened: Option<Opened>,
    /// Fetches the open article's images and reader page; set once its
    /// worker starts.
    web: Option<Web>,
    /// The open article's images, by URL.
    pictures: HashMap<String, article::Picture>,
    pending_g: bool,
    bench: Option<Bench>,
    /// Commands from the socket; `None` in tests.
    inbox: Option<Inbox>,
    /// The refresh thread's progress and release check; `None` in tests.
    status: Option<Status>,
    /// A newer Gutenberg release, as last read.
    newer: Option<String>,
    /// Sources done and due in the refresh running now, as last read.
    refreshing: Option<(usize, usize)>,
    /// Why the list is empty, when it is and isn't a search.
    empty: Option<list::Empty>,
    /// Articles the memory bench has left to open.
    memory_bench: Option<usize>,
    /// The store's file, for the web worker's search indexing; `None` in
    /// tests, which then keep nothing on disk.
    store_path: Option<PathBuf>,
    /// The Add sources screen, shown instead of the panes while open.
    onboarding: Option<onboarding::Onboarding>,
    /// The command palette, help or the share box, drawn over the panes.
    overlay: Option<Overlay>,
    /// A search in place of the chosen folder or feed's list.
    search: Option<Search>,
    /// Where `j`, `k` and Enter work: the sources pane or the list.
    pane: Pane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Sources,
    List,
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
    /// Where to share the selected article.
    SharePicker(share::Picker),
    /// The share being written.
    ShareCompose(share::Composer),
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
    /// Typing an MCP tool's arguments.
    ToolArguments,
    /// Typing an MCP server's key.
    KeyEntry,
    /// Picking where to share.
    SharePicker,
    /// Writing a share.
    ShareCompose,
}

/// The chosen MCP server's tools, in the list pane while it has no
/// articles or after `T`.
#[derive(Debug, Default)]
struct Tools {
    /// Asked for with `T` while the server has articles.
    asked: bool,
    /// The tool the keys are on.
    cursor: usize,
    editor: Option<ArgumentEditor>,
    /// The server's key being typed, after `K`.
    key: Option<KeyEditor>,
}

/// An MCP server's key being typed. It's shown as dots and goes to the
/// keyring, never the store.
struct KeyEditor {
    /// The key's name, like `JINA_API_KEY`.
    name: String,
    text: String,
    /// Whether Enter sent it to the keyring and no answer came yet.
    saving: bool,
    /// Why the last Enter didn't save it.
    error: Option<String>,
}

/// Never prints the key.
impl std::fmt::Debug for KeyEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyEditor")
            .field("name", &self.name)
            .field("saving", &self.saving)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// A tool's argument template being typed.
#[derive(Debug)]
struct ArgumentEditor {
    tool: String,
    text: String,
    /// Why the last Enter didn't save it.
    error: Option<String>,
}

/// Why the list for `filter` shows nothing: its view hides what it has,
/// or there's nothing yet, and why.
fn why_empty(store: &Store, filter: Filter) -> gutenberg_store::Result<list::Empty> {
    if !store.list_articles_in(filter, View::All, 1)?.is_empty() {
        return Ok(list::Empty::Hidden);
    }
    Ok(match filter {
        Filter::Source(id) => {
            let state = store.fetch_state(id)?;
            match (state.error, state.checked_at) {
                (Some(error), _) => list::Empty::Failed(error),
                (None, None) => list::Empty::Unfetched,
                (None, Some(_)) => list::Empty::NoArticles,
            }
        }
        _ if store.has_unchecked_sources()? => list::Empty::Unfetched,
        _ => list::Empty::NoArticles,
    })
}

/// Scrolls the sources pane just enough to show the cursor's row: finds
/// the pane and the row, then scrolls.
#[derive(Default)]
struct ShowCursor {
    /// The pane's bounds, its content's, and how far it's scrolled.
    pane: Option<(Rectangle, Rectangle, Vector)>,
    row: Option<Rectangle>,
}

impl Operation for ShowCursor {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
        if id == Some(&widget::Id::new(SOURCES_CURSOR)) {
            self.row = Some(bounds);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&widget::Id::new(SOURCES_SCROLL)) {
            self.pane = Some((bounds, content_bounds, translation));
        }
    }

    fn finish(&self) -> operation::Outcome<()> {
        let (Some((pane, content, scrolled)), Some(row)) = (self.pane, self.row) else {
            return operation::Outcome::None;
        };
        match sidebar::scroll_to_show(scrolled.y, pane.height, row.y - content.y, row.height) {
            Some(y) => operation::Outcome::Chain(Box::new(operation::scrollable::scroll_to(
                widget::Id::new(SOURCES_SCROLL),
                operation::scrollable::AbsoluteOffset {
                    x: None,
                    y: Some(y),
                },
            ))),
            None => operation::Outcome::None,
        }
    }
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
    /// The story's other articles: who published each, and its link.
    also: Vec<(String, Option<String>)>,
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

/// `GUTENBERG_MEMORY_BENCH=<articles>`: open that many articles in turn,
/// every fifth in the reader view, then report memory and close. Images and
/// pages come from the web, or the cache once fetched.
const MEMORY_STEP: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone)]
enum Message {
    Key(Action),
    /// The memory bench's next article.
    MemoryStep,
    Scroll(ScrollDelta),
    Click(usize),
    PollStore,
    /// Reads the refresh's progress, while one runs.
    PollProgress,
    Resized(Size),
    /// A divider between panes dragged to a new ratio of its split.
    DividerDragged(pane_grid::ResizeEvent),
    Frame(Instant),
    ConfigChanged(Config),
    /// The Omarchy theme, if there is one, and the desktop font if it
    /// changed, read again.
    ThemeReloaded(Option<Tokens>, Option<NewFont>),
    /// Another launch found this window open: come forward.
    Focus,
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
    /// Closes the palette, help or the share box.
    CloseOverlay,
    /// A key while the share picker is open.
    SharePicker(palette::Input),
    /// A destination clicked in the picker.
    PickDestination(usize),
    /// A key while writing a share.
    ShareCompose(palette::Input),
    /// A key while typing in the search line.
    Search(palette::Input),
    /// A key while typing an MCP tool's arguments.
    ToolArguments(palette::Input),
    /// A key while typing an MCP server's key.
    KeyEntry(palette::Input),
    /// The keyring's answer to saving server `id`'s key.
    KeySaved(i64, Result<(), String>),
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
        app.store_path = store_path.map(ToOwned::to_owned);
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
        let arrangement = config.layout.arrangement;
        let sizes = panes::Sizes::from_layout(config.layout);
        let (grid, dividers) = panes::grid(arrangement, &sizes);
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
            source_errors: HashMap::new(),
            tools: Tools::default(),
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
            width: 1280.0,
            arrangement,
            arrangement_stored: false,
            before_reading: None,
            sizes: [sizes; 4],
            sizes_stored: [false; 4],
            grid,
            dividers,
            opened: None,
            web: None,
            pictures: HashMap::new(),
            pending_g: false,
            pane: Pane::List,
            bench,
            inbox: None,
            status: None,
            newer: None,
            refreshing: None,
            empty: None,
            memory_bench: std::env::var("GUTENBERG_MEMORY_BENCH")
                .ok()
                .and_then(|n| n.parse().ok()),
            store_path: None,
            onboarding: None,
            overlay: None,
            search: None,
        };
        app.load_layout();
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
        let known_mcp: Vec<_> = self.sources.iter().filter_map(mcp::settings).collect();
        self.onboarding = Some(
            onboarding::Onboarding::new(
                catalog::builtin(),
                known,
                known_queries,
                self.sources.is_empty(),
            )
            .with_known_mcp(catalog::builtin(), &known_mcp),
        );
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
                store.source_errors()?,
            ))
        })();
        match read {
            Ok((rows, sources, folders, unread, view_counts, errors)) => {
                self.empty = if rows.is_empty() && self.search.is_none() {
                    why_empty(store, self.filter).ok()
                } else {
                    None
                };
                self.source_errors = errors;
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
        if let Message::Key(action) = message {
            match (self.pane, action) {
                (Pane::Sources, Action::Down) => return self.step_sources(Step::Down),
                (Pane::Sources, Action::Up) => return self.step_sources(Step::Up),
                (Pane::Sources, Action::First) => return self.step_sources(Step::First),
                (Pane::Sources, Action::Last) => return self.step_sources(Step::Last),
                (Pane::Sources, Action::Open) => {
                    self.pane = Pane::List;
                    return Task::none();
                }
                // These work in the list pane, so the keys go there.
                (_, Action::Search | Action::Tools | Action::EnterKey | Action::Share) => {
                    self.pane = Pane::List
                }
                _ => {}
            }
        }
        // While a server's tools are shown, moving and Enter work on them.
        if let (Some(id), Message::Key(action)) = (self.chosen_mcp(), &message) {
            if self.tools_key(id, *action) {
                return Task::none();
            }
        }
        match message {
            Message::Key(Action::Tools) => self.toggle_tools(),
            Message::ToolArguments(input) => self.type_arguments(input),
            Message::Key(Action::EnterKey) => self.open_key_editor(),
            Message::KeyEntry(input) => return self.type_key(input),
            Message::KeySaved(id, saved) => self.key_saved(id, saved),
            Message::Key(Action::Quit) => return iced::exit(),
            Message::Key(Action::NextPane | Action::PrevPane) => return self.switch_pane(),
            Message::Key(action @ (Action::NarrowPane | Action::WidenPane)) => {
                let pane = match self.pane {
                    Pane::Sources => panes::Id::Sources,
                    Pane::List => panes::Id::List,
                };
                let step = if action == Action::WidenPane {
                    panes::STEP
                } else {
                    -panes::STEP
                };
                let (arrangement, width, height) =
                    (self.arrangement, self.width, self.panes_height());
                if self
                    .sizes_mut()
                    .nudge(arrangement, pane, step, width, height)
                {
                    self.rebuild_grid();
                    self.save_sizes();
                }
            }
            Message::Key(Action::ResetPanes) => {
                if let (Ok(store), Some(setting)) = (&self.store, self.arrangement.sizes_setting())
                {
                    if let Err(e) = store.unset_setting(setting) {
                        eprintln!("gutenberg: forgetting pane sizes: {e}");
                    }
                }
                self.sizes_stored[slot(self.arrangement)] = false;
                self.apply_layout();
            }
            Message::Key(Action::NextLayout) => self.set_arrangement(self.arrangement.next()),
            Message::Key(Action::Layout(arrangement)) => self.set_arrangement(arrangement),
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
            Message::Key(Action::ClearSearch) => {
                // Reading is left the way a search is: Esc brings the list back.
                if self.arrangement == Arrangement::Reading {
                    self.set_arrangement(self.before_reading.unwrap_or(Arrangement::Columns));
                } else {
                    self.clear_search();
                }
            }
            Message::Key(Action::View(view)) => self.show_view(view),
            Message::Search(input) => self.search_input(input),
            Message::Key(Action::CopyLink) => {
                if let Some(url) = self.selected_link() {
                    return iced::clipboard::write(url);
                }
            }
            // Like O, nothing happens for an article without a link.
            Message::Key(Action::Share) => {
                if self.selected_link().is_some() {
                    self.overlay = Some(Overlay::SharePicker(self.share_picker()));
                }
            }
            Message::SharePicker(input) => {
                let Some(Overlay::SharePicker(picker)) = &mut self.overlay else {
                    return Task::none();
                };
                let closes = matches!(input, palette::Input::Close);
                let picked = picker.apply(input);
                if closes {
                    self.overlay = None;
                }
                if let Some(destination) = picked {
                    self.compose_share(destination);
                }
            }
            Message::PickDestination(i) => {
                let Some(Overlay::SharePicker(picker)) = &self.overlay else {
                    return Task::none();
                };
                if let Some(destination) = picker.destinations.get(i).cloned() {
                    self.compose_share(destination);
                }
            }
            Message::ShareCompose(input) => {
                let Some(Overlay::ShareCompose(composer)) = &mut self.overlay else {
                    return Task::none();
                };
                if matches!(input, palette::Input::Close) {
                    self.overlay = None;
                    return Task::none();
                }
                match composer.apply(input) {
                    Some(share::Done::Open(url)) => {
                        self.overlay = None;
                        open_in_browser(&url);
                    }
                    Some(share::Done::Copy(text)) => return iced::clipboard::write(text),
                    None => {}
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
                self.pane = Pane::List;
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
            Message::PollProgress => self.read_progress(),
            Message::PollStore => {
                self.read_progress();
                self.newer = self.status.as_ref().and_then(Status::newer_release);
                let changed = match &self.store {
                    Ok(store) => store.data_version().is_ok_and(|v| v != self.data_version),
                    Err(_) => false,
                };
                if changed {
                    self.reload();
                }
            }
            Message::Resized(size) => {
                self.width = size.width;
                self.set_height(size.height);
            }
            Message::Frame(at) => return self.bench_frame(at),
            Message::MemoryStep => return self.memory_step(),
            Message::ThemeReloaded(tokens, font) => {
                self.use_theme(tokens);
                if let Some(font) = font {
                    let file = font.file.map(Arc::unwrap_or_clone);
                    set_font(&font.family, file);
                }
            }
            // Wayland has no "raise": the window asks for activation, which
            // Hyprland grants with `misc:focus_on_activate`, Omarchy's
            // default, switching to the window's workspace.
            #[cfg(not(target_os = "macos"))]
            Message::Focus => {
                return window::latest().and_then(|id| {
                    window::request_user_attention(id, Some(window::UserAttention::Informational))
                });
            }
            // On macOS asking for attention only bounces the Dock icon;
            // taking focus activates the app and makes the window key.
            #[cfg(target_os = "macos")]
            Message::Focus => return window::latest().and_then(window::gain_focus),
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
                self.apply_layout();
                self.set_height(height);
                // An open picker shows the new [[share]] rows at once.
                if matches!(self.overlay, Some(Overlay::SharePicker(_))) {
                    self.overlay = Some(Overlay::SharePicker(self.share_picker()));
                }
            }
            Message::DividerDragged(event) => {
                if let Some(&(_, divider)) = self.dividers.iter().find(|d| d.0 == event.split) {
                    let (width, height) = (self.width, self.panes_height());
                    let ratio = self.sizes_mut().resize(divider, event.ratio, width, height);
                    self.grid.resize(event.split, ratio);
                    self.save_sizes();
                    self.set_height(self.height);
                }
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
            if filter != self.filter {
                self.tools = Tools::default();
            }
            self.filter = filter;
            self.view = view;
            self.search = None;
            self.show_from_top();
        }
    }

    /// With two panes there's one other: Tab and Shift-Tab both go there.
    /// Nothing happens while the sources pane is hidden.
    fn switch_pane(&mut self) -> Task<Message> {
        if !self.sources_shown() || self.store.is_err() {
            return Task::none();
        }
        self.pane = match self.pane {
            Pane::Sources => Pane::List,
            Pane::List => Pane::Sources,
        };
        self.show_sources_cursor()
    }

    /// Chooses the next, previous, first or last row of the sources pane,
    /// as a click on it would, and scrolls the pane to show it.
    fn step_sources(&mut self, to: Step) -> Task<Message> {
        let rows = sidebar::entries(&self.folders, &self.sources);
        let at = sidebar::cursor(&rows, self.filter, self.view);
        if let Some(next) = sidebar::step(rows.len(), at, to) {
            self.choose(rows[next]);
        }
        self.show_sources_cursor()
    }

    fn show_sources_cursor(&self) -> Task<Message> {
        if self.pane != Pane::Sources {
            return Task::none();
        }
        iced::advanced::widget::operate(ShowCursor::default()).discard()
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

    /// The list's own height: the window's, less the key bar and the search
    /// line when they're shown.
    fn list_height(&self) -> f32 {
        let mut height = self.panes_height();
        // Where the list sits above the article, it gets `rows` of the
        // height, as the grid draws it.
        let stacked = matches!(self.arrangement, Arrangement::Classic | Arrangement::Wide)
            && self.dividers.iter().any(|d| d.1 == panes::Divider::Rows);
        if stacked {
            height *= self.sizes().rows;
        }
        if self.search.is_some() {
            height -= SEARCH_BAR;
        }
        height.max(0.0)
    }

    /// What's on screen, for the key bar.
    fn screen(&self) -> hints::Screen {
        use hints::Screen;
        match (&self.search, &self.opened) {
            _ if matches!(self.overlay, Some(Overlay::SharePicker(_))) => Screen::SharePicker,
            _ if matches!(&self.overlay, Some(Overlay::ShareCompose(c)) if c.takes_text()) => {
                Screen::ShareCompose
            }
            _ if matches!(self.overlay, Some(Overlay::ShareCompose(_))) => Screen::ShareLink,
            (Some(search), _) if search.editing => Screen::Searching,
            _ if self.pane == Pane::Sources && !self.sources.is_empty() => Screen::Sources,
            (Some(_), _) => Screen::Results,
            _ if self.tools.editor.is_some() => Screen::Arguments,
            _ if self.tools.key.is_some() => Screen::Key,
            _ if self.chosen_mcp().is_some() => Screen::Tools,
            _ if self.sources.is_empty() => Screen::Empty,
            (None, None) => Screen::List,
            (None, Some(opened)) if matches!(opened.reader, Reader::Ready(_)) => Screen::Reader,
            (None, Some(_)) => Screen::Summary,
        }
    }

    /// The window is `height` logical pixels tall: keeps the selected row
    /// in view and the list from scrolling past its end.
    /// Reads the arrangement, and each arrangement's sizes, that `L`, a
    /// drag or a key left in the store; `[layout]` applies until there are
    /// any.
    fn load_layout(&mut self) {
        if let Some(arrangement) = read_setting(&self.store, panes::ARRANGEMENT_SETTING)
            .and_then(|s| Arrangement::from_name(&s))
        {
            self.arrangement = arrangement;
            self.arrangement_stored = true;
        }
        for arrangement in panes::ARRANGEMENTS {
            let Some(key) = arrangement.sizes_setting() else {
                continue;
            };
            if let Some(sizes) =
                read_setting(&self.store, key).and_then(|s| panes::Sizes::from_setting(&s))
            {
                self.sizes[slot(arrangement)] = sizes;
                self.sizes_stored[slot(arrangement)] = true;
            }
        }
        self.apply_layout();
    }

    /// Lets `[layout]` choose the arrangement and which panes show, and with
    /// nothing stored how big, then rebuilds the grid.
    fn apply_layout(&mut self) {
        let layout = self.config.layout;
        if !self.arrangement_stored {
            self.arrangement = layout.arrangement;
        }
        for arrangement in panes::ARRANGEMENTS {
            let i = slot(arrangement);
            self.sizes[i] = if self.sizes_stored[i] {
                self.sizes[i].shown(layout)
            } else {
                panes::Sizes::from_layout(layout)
            };
        }
        self.leave_hidden_pane();
        self.rebuild_grid();
    }

    /// Switches to `arrangement`, keeping it for the next launch, and where
    /// reading started from, so `Esc` can go back.
    fn set_arrangement(&mut self, arrangement: Arrangement) {
        if arrangement == Arrangement::Reading && self.arrangement != Arrangement::Reading {
            self.before_reading = Some(self.arrangement);
        }
        self.arrangement = arrangement;
        self.arrangement_stored = true;
        if let Ok(store) = &self.store {
            if let Err(e) = store.set_setting(panes::ARRANGEMENT_SETTING, arrangement.name()) {
                eprintln!("gutenberg: saving the layout: {e}");
            }
        }
        self.leave_hidden_pane();
        self.rebuild_grid();
    }

    /// The keys can't stay in a sources pane that isn't showing.
    fn leave_hidden_pane(&mut self) {
        if self.pane == Pane::Sources && !self.sources_shown() {
            self.pane = Pane::List;
        }
    }

    fn sources_shown(&self) -> bool {
        panes::visible(self.arrangement, self.sizes()).contains(&panes::Id::Sources)
    }

    /// The current arrangement's divider positions.
    fn sizes(&self) -> &panes::Sizes {
        &self.sizes[slot(self.arrangement)]
    }

    fn sizes_mut(&mut self) -> &mut panes::Sizes {
        &mut self.sizes[slot(self.arrangement)]
    }

    fn rebuild_grid(&mut self) {
        let (grid, dividers) = panes::grid(self.arrangement, self.sizes());
        self.grid = grid;
        self.dividers = dividers;
        // The list may be a different height now.
        self.set_height(self.height);
    }

    /// Keeps the arrangement's sizes for the next launch.
    fn save_sizes(&mut self) {
        let i = slot(self.arrangement);
        self.sizes_stored[i] = true;
        if let (Ok(store), Some(key)) = (&self.store, self.arrangement.sizes_setting()) {
            if let Err(e) = store.set_setting(key, &self.sizes[i].to_setting()) {
                eprintln!("gutenberg: saving pane sizes: {e}");
            }
        }
    }

    /// The height the panes get: the window's, less the key bar.
    fn panes_height(&self) -> f32 {
        if self.config.keys.hints {
            self.height - HINT_BAR
        } else {
            self.height
        }
    }

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

    /// Who published an article: a keyword alert's publisher, else the
    /// feed's name.
    fn publisher(&self, row: &ArticleRow) -> String {
        match &row.author {
            Some(publisher) if self.keyword_sources.contains(&row.source_id) => publisher.clone(),
            _ => self
                .source_titles
                .get(&row.source_id)
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// The selected article's own link.
    fn selected_link(&self) -> Option<String> {
        self.rows.get(self.selected)?.url.clone()
    }

    /// The share picker's rows: `[[share]]` in the config's order, or the
    /// four networks.
    fn share_picker(&self) -> share::Picker {
        share::Picker::new(gutenberg_core::share::destinations(&self.config.share))
    }

    /// Replaces the picker with the compose box for the selected article,
    /// read again now in case the list moved; without a link it closes.
    fn compose_share(&mut self, destination: gutenberg_core::share::Destination) {
        let selected = self.rows.get(self.selected);
        let linked = selected.and_then(|row| Some((row.title.clone(), row.url.clone()?)));
        self.overlay = linked.map(|(title, url)| {
            Overlay::ShareCompose(share::Composer::new(destination, title, url))
        });
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

    /// Read becomes unread and back, for the whole story; the feed's count
    /// follows.
    fn toggle_read(&mut self) {
        let (Ok(store), Some(row)) = (&self.store, self.rows.get(self.selected)) else {
            return;
        };
        if let Err(e) = store.set_story_read(row.story_id, !row.read) {
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

    /// Shows the article and marks its story read.
    fn open(&mut self, i: usize) {
        let Some(row) = self.rows.get_mut(i) else {
            return;
        };
        if !row.read {
            if let Ok(store) = &self.store {
                match store.set_story_read(row.story_id, true) {
                    Ok(()) => {
                        row.read = true;
                        if row.story_size > 1 {
                            // The story may span feeds; count again.
                            if let (Ok(unread), Ok(views)) =
                                (store.unread_counts(), store.view_counts())
                            {
                                self.unread = unread;
                                self.view_counts = views;
                            }
                        } else {
                            if let Some(n) = self.unread.get_mut(&row.source_id) {
                                *n = (*n - 1).max(0);
                            }
                            self.view_counts.unread = (self.view_counts.unread - 1).max(0);
                        }
                    }
                    Err(e) => eprintln!("gutenberg: marking read: {e}"),
                }
            }
        }
        let row = &self.rows[i];
        let also = match (&self.store, row.story_size > 1) {
            (Ok(store), true) => store
                .story(row.story_id)
                .unwrap_or_default()
                .into_iter()
                .filter(|other| other.id != row.id)
                .map(|other| (self.publisher(&other), other.url))
                .collect(),
            _ => Vec::new(),
        };
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
            also,
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
            Answer::Image { url, picture } => self.image_loaded(url, Some(picture)),
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
    fn image_loaded(&mut self, url: String, fetched: Option<web::Picture>) -> Task<Message> {
        // An answer for an article that's no longer open is dropped.
        let Some(slot) = self.pictures.get_mut(&url) else {
            return Task::none();
        };
        match fetched {
            None => *slot = article::Picture::Failed,
            Some(web::Picture::Svg(bytes)) => {
                *slot = article::Picture::Vector(svg::Handle::from_memory(bytes));
            }
            Some(web::Picture::Rgba {
                width,
                height,
                pixels,
            }) => {
                return image::allocate(image::Handle::from_rgba(width, height, pixels))
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

    fn memory_step(&mut self) -> Task<Message> {
        let Some(left) = self.memory_bench else {
            return Task::none();
        };
        if left == 0 {
            eprintln!(
                "memory bench: {} articles in the list: {}",
                self.rows.len(),
                memory_report()
            );
            return iced::exit();
        }
        self.memory_bench = Some(left - 1);
        let mut tasks = vec![self.update(Message::Key(Action::Down))];
        tasks.push(self.update(Message::Key(Action::Open)));
        if left % 5 == 0 {
            tasks.push(self.update(Message::Key(Action::Reader)));
        }
        if left % 20 == 0 {
            eprintln!("memory bench: {left} to go: {}", memory_report());
        }
        Task::batch(tasks)
    }

    fn subscription(&self) -> Subscription<Message> {
        let mode = match (&self.onboarding, &self.overlay, &self.search) {
            (Some(o), _, _) if o.draft.is_some() => KeyMode::OnboardingQuery,
            (Some(_), _, _) => KeyMode::Onboarding,
            (None, Some(Overlay::Palette(_)), _) => KeyMode::Palette,
            (None, Some(Overlay::Help), _) => KeyMode::Help,
            (None, Some(Overlay::SharePicker(_)), _) => KeyMode::SharePicker,
            (None, Some(Overlay::ShareCompose(_)), _) => KeyMode::ShareCompose,
            (None, None, Some(Search { editing: true, .. })) => KeyMode::Search,
            (None, None, _) if self.tools.editor.is_some() => KeyMode::ToolArguments,
            (None, None, _) if self.tools.key.is_some() => KeyMode::KeyEntry,
            (None, None, _) => KeyMode::List,
        };
        let keys = keyboard::listen().with((self.pending_g, mode)).filter_map(
            |((pending_g, mode), event)| {
                // `modified_key` has Shift applied, so Shift+; is `:` and
                // Shift+/ is `?`; `key` would hand us `;` and `/`.
                let keyboard::Event::KeyPressed {
                    modified_key: key,
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
                    KeyMode::ToolArguments => palette::input_for(&key, modifiers, text.as_deref())
                        .map(Message::ToolArguments),
                    KeyMode::KeyEntry => {
                        palette::input_for(&key, modifiers, text.as_deref()).map(Message::KeyEntry)
                    }
                    KeyMode::SharePicker => palette::input_for(&key, modifiers, text.as_deref())
                        .map(Message::SharePicker),
                    KeyMode::ShareCompose => palette::input_for(&key, modifiers, text.as_deref())
                        .map(Message::ShareCompose),
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
            // Every second notices a refresh start; while it runs, the bar
            // moves more often than that.
            if self.refreshing.is_some() {
                iced::time::every(PROGRESS_POLL).map(|_| Message::PollProgress)
            } else {
                Subscription::none()
            },
            window::resize_events().map(|(_, size)| Message::Resized(size)),
            iced::system::theme_changes().map(Message::SystemTheme),
        ];
        if let Some(path) = &self.config_path {
            subscriptions.push(Subscription::run_with(path.clone(), watch_config));
        }
        if let Some(inbox) = self.inbox.as_ref().filter(|_| self.bench.is_none()) {
            subscriptions.push(Subscription::run_with(inbox.clone(), window_commands));
        }
        if self.bench.is_none() {
            subscriptions.push(Subscription::run_with(
                (self.store_path.clone(), self.config.cache),
                web_worker,
            ));
        }
        if self.bench.is_some() {
            subscriptions.push(window::frames().map(Message::Frame));
        }
        if self.memory_bench.is_some() {
            subscriptions.push(iced::time::every(MEMORY_STEP).map(|_| Message::MemoryStep));
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
        let accent = t.accent;
        let panes = PaneGrid::new(&self.grid, |_, id, _| {
            pane_grid::Content::new(match id {
                panes::Id::Sources => pane(self.sidebar_view(), t.background, t.border),
                panes::Id::List => pane(self.list_view(), t.background, t.border),
                panes::Id::Article => pane(self.article_view(), t.lighter_background, t.border),
            })
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .min_size(panes::MIN_WIDTH)
        // The grab zone is the border line and a few pixels either side; the
        // line itself is each pane's border, drawn as before.
        .on_resize(6, Message::DividerDragged)
        .style(move |_| pane_grid::Style {
            hovered_region: pane_grid::Highlight {
                background: Color::TRANSPARENT.into(),
                border: Border::default(),
            },
            picked_split: pane_grid::Line {
                color: color(accent),
                width: 3.0,
            },
            hovered_split: pane_grid::Line {
                color: color(accent),
                width: 3.0,
            },
        });
        let window: Element<'_, Message> = if self.config.keys.hints {
            column![panes, self.hint_bar()].into()
        } else {
            panes.into()
        };
        match &self.overlay {
            None => window,
            Some(overlay) => stack![window, self.overlay_view(overlay)].into(),
        }
    }

    /// The keys for what's on screen, each key then what it does.
    fn hint_bar(&self) -> Element<'_, Message> {
        let t = &self.tokens;
        let hints = hints::for_screen(self.screen()).into_iter().map(|hint| {
            row![
                text(hint.keys)
                    .size(12)
                    .color(color(t.foreground))
                    .font(Font {
                        weight: font::Weight::Bold,
                        ..Font::MONOSPACE
                    }),
                text(hint.label).size(12).color(color(t.muted)),
            ]
            .spacing(6)
            .into()
        });
        let (background, border_color) = (t.background, t.border);
        let refreshing = self
            .status_line()
            .map(|status| text(status).size(12).color(color(t.muted)));
        container(
            Row::with_children(hints)
                .push(
                    refreshing
                        .is_some()
                        .then(|| Space::new().width(Length::Fill)),
                )
                .push(refreshing)
                .spacing(20)
                .align_y(Alignment::Center),
        )
        .padding([0, 12])
        .height(Length::Fixed(HINT_BAR))
        .width(Length::Fill)
        .align_y(Alignment::Center)
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
                (Some(prompt), rows, footer.to_owned())
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
                    "Esc or ? closes · : finds a command by name".to_owned(),
                )
            }
            Overlay::SharePicker(picker) => {
                let rows: Vec<Element<'a, Message>> = picker
                    .destinations
                    .iter()
                    .zip(&picker.letters)
                    .enumerate()
                    .map(|(i, (destination, letter))| {
                        mouse_area(share_row(
                            *letter,
                            destination.name(),
                            i == picker.cursor,
                            t,
                        ))
                        .on_press(Message::PickDestination(i))
                        .into()
                    })
                    .collect();
                let prompt = row![text("Share to").color(color(t.muted))].padding([10, 12]);
                let footer = if picker.destinations.is_empty() {
                    "Nothing to share to: check [[share]] in config.toml · Esc closes"
                } else {
                    "Enter or a letter chooses · j k move · Esc closes"
                };
                (Some(prompt), rows, footer.to_owned())
            }
            Overlay::ShareCompose(composer) => {
                let prompt = row![
                    text("Share to").color(color(t.muted)),
                    text(composer.destination.name()).color(color(t.accent)),
                ]
                .spacing(8)
                .padding([10, 12]);
                (
                    Some(prompt),
                    vec![self.compose_view(composer)],
                    composer.hint(),
                )
            }
        };
        let (surface, border, backdrop) = (t.lighter_background, t.accent, t.background);
        let card = container(
            Column::new()
                .push(prompt)
                .push(Column::with_children(rows))
                .push(
                    container(
                        text(footer)
                            .size(12)
                            .color(color(t.muted))
                            .wrapping(text::Wrapping::WordOrGlyph),
                    )
                    .padding([8, 12]),
                )
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

    /// The compose box's body: the comment with a cursor where the site
    /// takes it (greyed where it doesn't), the link, and the count left.
    fn compose_view<'a>(&self, composer: &'a share::Composer) -> Element<'a, Message> {
        let t = &self.tokens;
        let (background, border_color) = (color(t.background), color(t.border));
        let comment = if composer.takes_text() {
            text(format!("{}▏", composer.share.text)).color(color(t.foreground))
        } else {
            text(composer.share.text.as_str()).color(color(t.muted))
        };
        let field = container(comment.size(14).wrapping(text::Wrapping::WordOrGlyph))
            .padding([6, 10])
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(background.into()),
                border: Border {
                    color: border_color,
                    width: 1.0,
                    radius: border::radius(4),
                },
                ..Default::default()
            });
        let link = text(composer.share.url.as_str())
            .size(12)
            .color(color(t.muted))
            .wrapping(text::Wrapping::Glyph);
        // Over the limit shows in the accent, as errors do elsewhere.
        let counter = composer.remaining().map(|n| {
            if n < 0 {
                text(format!("{} over", -n)).size(12).color(color(t.accent))
            } else {
                text(format!("{n} left")).size(12).color(color(t.muted))
            }
        });
        column![field, link]
            .push(counter)
            .spacing(6)
            .padding([4, 12])
            .into()
    }

    fn sidebar_view(&self) -> Element<'_, Message> {
        let t = &self.tokens;
        if self.store.is_err() {
            return Space::new().into();
        }
        let entries = sidebar::entries(&self.folders, &self.sources);
        let cursor = sidebar::cursor(&entries, self.filter, self.view);
        let rows = entries
            .into_iter()
            .enumerate()
            .map(|(i, entry)| self.sidebar_row(entry, cursor == Some(i)));
        // The space below the rows is a drop target too: out of any folder.
        // Past the last row a strip of it scrolls with them, so it can be
        // reached when they fill the pane.
        let unfiled = |height| {
            mouse_area(Space::new().width(Length::Fill).height(height))
                .on_enter(Message::Over(Entry::Unfiled))
        };
        let end: Element<'_, Message> = if self.drop_line_at(Entry::Unfiled) {
            column![drop_line(t.accent), unfiled(Length::Fixed(UNFILED_STRIP))].into()
        } else {
            unfiled(Length::Fixed(UNFILED_STRIP)).into()
        };
        // More sources than fit scroll; fewer leave the pane's rest empty.
        let rows = scrollable(Column::with_children(rows).push(end).padding([8, 0]))
            .id(SOURCES_SCROLL)
            .height(Length::Shrink);
        column![rows, unfiled(Length::Fill)]
            .height(Length::Fill)
            .into()
    }

    /// Whether a drag is over `entry` and dropping there would do something.
    fn drop_line_at(&self, entry: Entry) -> bool {
        self.drag.as_ref().is_some_and(|d| {
            d.over == entry && d.over != d.origin && sidebar::drop_on(d.origin, entry).is_some()
        })
    }

    /// `at_cursor` for the row that stands for what the list shows; with
    /// the keys in the sources pane, its name takes the accent.
    fn sidebar_row(&self, entry: Entry, at_cursor: bool) -> Element<'_, Message> {
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
        let focused = at_cursor && self.pane == Pane::Sources;
        let label = text(name)
            .size(14)
            .color(color(if dragged {
                t.muted
            } else if focused {
                t.accent
            } else {
                t.foreground
            }))
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
        // An MCP server that can't be reached says why, under its name.
        let error = match entry {
            Entry::Source { id, .. } if self.is_mcp(id) => self.source_errors.get(&id),
            _ => None,
        };
        let line: Element<'_, Message> = match error {
            Some(error) => column![
                line,
                // Under the title, past the icon's space.
                container(
                    text(error)
                        .size(12)
                        .color(color(t.accent))
                        .wrapping(text::Wrapping::None),
                )
                .padding(iced::Padding::ZERO.left(ICON_SIZE + 8.0)),
            ]
            .spacing(2)
            .into(),
            None => line.into(),
        };
        let row = container(line)
            .id(if at_cursor {
                SOURCES_CURSOR
            } else {
                "sources-row"
            })
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
        let list: Element<'_, Message> = if let Some(id) = self.chosen_mcp() {
            self.mcp_view(id)
        } else if self.rows.is_empty() {
            let message = match &self.search {
                Some(search) if search.query.trim().is_empty() => "Type to search every article",
                Some(_) => "No articles match",
                None if self.sources.is_empty() => {
                    "No feeds yet. Press a to pick some, or run gutenberg --add <url>"
                }
                None => match &self.empty {
                    Some(empty) => &list::empty_message(
                        empty,
                        &self.place_name(),
                        self.view,
                        self.refreshing.is_some(),
                    ),
                    None => "Fetching your feeds…",
                },
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
        let list = match &self.search {
            None => list,
            Some(search) => column![self.search_bar(search), list].into(),
        };
        // Over the top of the list, so the rows don't move when it comes.
        match self.refreshing {
            Some((done, total)) => {
                let (bar, track) = (t.accent, t.selection);
                let progress = iced::widget::progress_bar(0.0..=total as f32, done as f32)
                    .girth(PROGRESS_BAR)
                    .style(move |_| iced::widget::progress_bar::Style {
                        background: color(track).into(),
                        bar: color(bar).into(),
                        border: Border::default(),
                    });
                stack![list, progress].into()
            }
            None => list,
        }
    }

    /// The right of the key bar: a refresh's progress, else a new release.
    fn status_line(&self) -> Option<String> {
        match (self.refreshing, &self.newer) {
            (Some((done, total)), _) => Some(format!("Refreshing {done} of {total}")),
            (None, Some(version)) => {
                Some(format!("Gutenberg {version} is out · gutenberg --update"))
            }
            (None, None) => None,
        }
    }

    /// What the list is showing: a feed's title, a folder's name, or
    /// every feed.
    fn place_name(&self) -> String {
        match self.filter {
            Filter::All => "your feeds".to_owned(),
            Filter::Folder(id) => self
                .folders
                .iter()
                .find(|f| f.id == id)
                .map_or_else(|| "this folder".to_owned(), |f| f.name.clone()),
            Filter::Source(id) => self
                .source_titles
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "this feed".to_owned()),
        }
    }

    /// What the refresh thread has done so far, if it's refreshing.
    fn read_progress(&mut self) {
        self.refreshing = self.status.as_ref().and_then(|s| s.progress.get());
    }

    /// Whether a source is an MCP server.
    fn is_mcp(&self, id: i64) -> bool {
        self.sources
            .iter()
            .any(|s| s.id == id && s.kind == SourceKind::Mcp)
    }

    /// The MCP server chosen in the sources pane whose tools the list pane
    /// shows: while no search is on, and it has no articles or `T` asked.
    fn chosen_mcp(&self) -> Option<i64> {
        match self.filter {
            Filter::Source(id)
                if self.search.is_none() && (self.rows.is_empty() || self.tools.asked) =>
            {
                self.is_mcp(id).then_some(id)
            }
            _ => None,
        }
    }

    /// The tools a source listed when last reached.
    fn tools_of(&self, id: i64) -> Vec<mcp::Tool> {
        self.sources
            .iter()
            .find(|s| s.id == id)
            .map(mcp::tools)
            .unwrap_or_default()
    }

    /// `T`: the chosen MCP server's tools, or its articles again.
    fn toggle_tools(&mut self) {
        if let Filter::Source(id) = self.filter {
            if self.is_mcp(id) {
                self.tools.asked = !self.tools.asked;
                self.tools.editor = None;
                self.tools.key = None;
            }
        }
    }

    /// A list key while server `id`'s tools are shown; whether it was one
    /// for them.
    fn tools_key(&mut self, id: i64, action: Action) -> bool {
        let tools = self.tools_of(id);
        let last = tools.len().saturating_sub(1);
        let cursor = self.tools.cursor;
        match action {
            Action::Down => self.tools.cursor = (cursor + 1).min(last),
            Action::Up => self.tools.cursor = cursor.saturating_sub(1),
            Action::First => self.tools.cursor = 0,
            Action::Last => self.tools.cursor = last,
            Action::Open => {
                let Some(tool) = tools.into_iter().nth(cursor) else {
                    return true;
                };
                let settings = self
                    .sources
                    .iter()
                    .find(|s| s.id == id)
                    .and_then(mcp::settings);
                // The template it calls with, if it's the tool in use.
                let text = settings
                    .filter(|m| m.tool.as_deref() == Some(tool.name.as_str()))
                    .and_then(|m| m.arguments)
                    .or(tool.example)
                    .unwrap_or_else(|| "{}".to_owned());
                self.tools.editor = Some(ArgumentEditor {
                    tool: tool.name,
                    text,
                    error: None,
                });
            }
            Action::ClearSearch if self.tools.asked => self.tools.asked = false,
            _ => return false,
        }
        true
    }

    /// A key while typing a tool's arguments. Enter saves them and the tool,
    /// and the next refresh, within a second, calls it.
    fn type_arguments(&mut self, input: palette::Input) {
        let Some(editor) = &mut self.tools.editor else {
            return;
        };
        match input {
            palette::Input::Type(text) => editor.text.push_str(&text),
            palette::Input::Backspace => {
                editor.text.pop();
            }
            palette::Input::Up | palette::Input::Down => {}
            palette::Input::Close => self.tools.editor = None,
            palette::Input::Run => {
                let (Ok(store), Filter::Source(id)) = (&self.store, self.filter) else {
                    return;
                };
                let Some(source) = self.sources.iter().find(|s| s.id == id) else {
                    return;
                };
                match mcp::pick(store, source, &editor.tool, &editor.text) {
                    Ok(()) => {
                        self.tools = Tools::default();
                        self.reload();
                    }
                    Err(e) => editor.error = Some(e),
                }
            }
        }
    }

    /// `K`: type the key the chosen MCP server needs. Its tools come into
    /// view, with the field under the heading.
    fn open_key_editor(&mut self) {
        let Filter::Source(id) = self.filter else {
            return;
        };
        let name = self
            .sources
            .iter()
            .find(|s| s.id == id)
            .and_then(mcp::settings)
            .and_then(|m| m.secret);
        if let Some(name) = name {
            self.tools.asked = true;
            self.tools.editor = None;
            self.tools.key = Some(KeyEditor {
                name,
                text: String::new(),
                saving: false,
                error: None,
            });
        }
    }

    /// A key while typing a server's key. Enter sends it to the keyring,
    /// off the update loop: the keyring may ask to be unlocked.
    fn type_key(&mut self, input: palette::Input) -> Task<Message> {
        let (Some(editor), Filter::Source(id)) = (&mut self.tools.key, self.filter) else {
            return Task::none();
        };
        if editor.saving {
            return Task::none();
        }
        match input {
            palette::Input::Type(text) => editor.text.push_str(&text),
            palette::Input::Backspace => {
                editor.text.pop();
            }
            palette::Input::Up | palette::Input::Down => {}
            palette::Input::Close => self.tools.key = None,
            palette::Input::Run => {
                editor.saving = true;
                editor.error = None;
                let (name, value) = (editor.name.clone(), editor.text.clone());
                return Task::perform(
                    async move { secrets::save(&name, &value).await },
                    move |saved| Message::KeySaved(id, saved),
                );
            }
        }
        Task::none()
    }

    /// Saved: the next refresh, within a second, reaches the server with it.
    fn key_saved(&mut self, id: i64, saved: Result<(), String>) {
        match saved {
            Ok(()) => {
                if let Ok(store) = &self.store {
                    let _ = store.recheck(id);
                }
                self.tools.key = None;
                self.reload();
            }
            Err(e) => {
                if let Some(editor) = &mut self.tools.key {
                    editor.saving = false;
                    editor.error = Some(e);
                }
            }
        }
    }

    /// An MCP server's tools, the one its refresh calls marked, or why it
    /// couldn't be reached. The keys move through them and Enter edits the
    /// arguments of the one they're on.
    fn mcp_view(&self, id: i64) -> Element<'_, Message> {
        let t = &self.tokens;
        let tools = self.tools_of(id);
        let settings = self
            .sources
            .iter()
            .find(|s| s.id == id)
            .and_then(mcp::settings);
        let answered = settings.as_ref().is_some_and(|m| m.called.is_some());
        let secret = settings.as_ref().and_then(|m| m.secret.clone());
        let picked = settings.and_then(|m| m.tool);
        let heading = match (self.source_errors.get(&id), tools.len(), &picked) {
            (Some(error), _, _) => {
                format!("Can't reach this server: {error}. Gutenberg tries again later.")
            }
            (None, 0, _) => "Reaching the server…".to_owned(),
            (None, _, Some(tool)) if self.rows.is_empty() && answered => format!(
                "Calls {tool} on each refresh; its last answer had nothing to read. j and k move, Enter changes a tool's arguments."
            ),
            (None, _, Some(tool)) if self.rows.is_empty() => format!(
                "Calls {tool} on each refresh; its articles show here once it answers. j and k move, Enter changes a tool's arguments."
            ),
            (None, _, Some(tool)) => format!(
                "Calls {tool} on each refresh. j and k move, Enter changes a tool's arguments, T or Esc shows the articles."
            ),
            (None, _, None) => "Pick the tool to read from: j and k move, Enter edits its arguments.".to_owned(),
        };
        let mut column = Column::new()
            .push(
                text(heading)
                    .size(14)
                    .color(color(t.muted))
                    .wrapping(text::Wrapping::WordOrGlyph),
            )
            .spacing(6)
            .padding(16);
        // Unless the heading already says it's missing.
        let said = |name: &str| {
            self.source_errors.get(&id).map(String::as_str)
                == Some(gutenberg_core::sync::missing_key(name).as_str())
        };
        if let Some(name) = secret.filter(|name| !said(name)) {
            column = column.push(
                text(format!(
                    "Needs {name}: K enters it, kept in your keyring. An environment variable of that name comes first."
                ))
                .size(12)
                .color(color(t.muted))
                .wrapping(text::Wrapping::WordOrGlyph),
            );
        }
        if let Some(editor) = &self.tools.key {
            column = column.push(self.key_field(editor));
        }
        for (i, tool) in tools.into_iter().enumerate() {
            let at_cursor = i == self.tools.cursor;
            let in_use = picked.as_deref() == Some(tool.name.as_str());
            let name = row![text(tool.name.clone()).size(14).color(color(t.foreground))]
                .push(in_use.then(|| text("· calls on refresh").size(12).color(color(t.accent))))
                .spacing(8)
                .align_y(Alignment::Center);
            let mut item = column![
                name,
                text(tool.description.unwrap_or_default())
                    .size(12)
                    .color(color(t.muted))
                    .wrapping(text::Wrapping::WordOrGlyph),
            ]
            .spacing(2);
            if let Some(editor) = self.tools.editor.as_ref().filter(|_| at_cursor) {
                item = item.push(self.argument_field(editor));
            }
            let background = at_cursor.then(|| color(t.selection).into());
            column = column.push(container(item).padding([6, 8]).width(Length::Fill).style(
                move |_| container::Style {
                    background,
                    ..Default::default()
                },
            ));
        }
        scrollable(column).height(Length::Fill).into()
    }

    /// The key being typed, as dots.
    fn key_field<'a>(&self, editor: &'a KeyEditor) -> Element<'a, Message> {
        let t = &self.tokens;
        let (background, border_color) = (color(t.background), color(t.border));
        let dots = "•".repeat(editor.text.chars().count());
        let field = container(text(format!("{dots}▏")).size(13).color(color(t.foreground)))
            .padding([6, 10])
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(background.into()),
                border: Border {
                    color: border_color,
                    width: 1.0,
                    radius: border::radius(4),
                },
                ..Default::default()
            });
        let help = if editor.saving {
            "Saving in the keyring…".to_owned()
        } else {
            format!(
                "{}: paste or type it. Enter saves it in the keyring · Esc cancels",
                editor.name
            )
        };
        column![field]
            .push(editor.error.as_ref().map(|e| {
                text(e.clone())
                    .size(12)
                    .color(color(t.accent))
                    .wrapping(text::Wrapping::WordOrGlyph)
            }))
            .push(
                text(help)
                    .size(12)
                    .color(color(t.muted))
                    .wrapping(text::Wrapping::WordOrGlyph),
            )
            .spacing(4)
            .into()
    }

    /// The arguments being typed, with a cursor, why they didn't save, and
    /// what the placeholders are.
    fn argument_field<'a>(&self, editor: &'a ArgumentEditor) -> Element<'a, Message> {
        let t = &self.tokens;
        let (background, border_color) = (color(t.background), color(t.border));
        let field = container(
            text(format!("{}▏", editor.text))
                .size(13)
                .color(color(t.foreground))
                .wrapping(text::Wrapping::WordOrGlyph),
        )
        .padding([6, 10])
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(background.into()),
            border: Border {
                color: border_color,
                width: 1.0,
                radius: border::radius(4),
            },
            ..Default::default()
        });
        let help = text(
            "Arguments as JSON. {{today}} is today's date, {{since}} the date of the last call. Enter saves · Esc cancels",
        )
        .size(12)
        .color(color(t.muted))
        .wrapping(text::Wrapping::WordOrGlyph);
        column![field]
            .push(editor.error.as_ref().map(|e| {
                text(e.clone())
                    .size(12)
                    .color(color(t.accent))
                    .wrapping(text::Wrapping::WordOrGlyph)
            }))
            .push(help)
            .spacing(4)
            .padding(iced::Padding::ZERO.top(6))
            .into()
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
        let sources = (row.story_size > 1).then(|| {
            row![
                text("·").size(12).color(color(t.muted)),
                text(format!("{} sources", row.story_size))
                    .size(12)
                    .color(color(t.accent))
                    .wrapping(text::Wrapping::None),
            ]
            .spacing(4)
        });
        let meta = Row::new()
            .push(
                row.starred
                    .then(|| text("★").size(12).color(color(t.accent))),
            )
            .push(meta)
            .push(sources)
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
            // Nothing open yet: say how to open something.
            if self.rows.is_empty() {
                return Space::new().into();
            }
            return iced::widget::center(
                column![
                    text("o or Enter opens the selected article")
                        .size(14)
                        .color(color(t.muted)),
                    text("j and k move · ? lists every key")
                        .size(13)
                        .color(color(t.muted)),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .into();
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
            .push(also_view(&opened.also, t))
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

/// "Also from" and the story's other publishers, each a link to its article.
fn also_view<'a>(also: &'a [(String, Option<String>)], t: &Tokens) -> Option<Element<'a, Message>> {
    if also.is_empty() {
        return None;
    }
    let mut pieces = vec![span("Also from ").color(color(t.muted))];
    for (i, (publisher, url)) in also.iter().enumerate() {
        if i > 0 {
            pieces.push(span(" · ").color(color(t.muted)));
        }
        let name = span(publisher.as_str());
        pieces.push(match url {
            Some(url) => name
                .color(color(t.accent))
                .underline(true)
                .link(url.clone()),
            None => name.color(color(t.foreground)),
        });
    }
    Some(
        rich_text(pieces)
            .size(13)
            .wrapping(text::Wrapping::WordOrGlyph)
            .on_link_click(Message::OpenLink)
            .into(),
    )
}

/// For `fuzz/`: the article pane's HTML parser, on a feed summary or a
/// reader page.
#[doc(hidden)]
pub mod fuzz {
    /// Parses `html` as an article at `https://example.com/news/1`, which
    /// relative links resolve against; returns the blocks made.
    pub fn article(html: &str) -> usize {
        let blocks = super::article::parse(html, Some("https://example.com/news/1"));
        super::article::image_urls(&blocks);
        blocks.len()
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

/// For `examples/theme_shots.rs` and `examples/site_media.rs`: the real
/// window, drawn headlessly in a theme's colors, over sample articles. The
/// site's sources, headlines and stories are made up.
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

    pub use crate::panes::{Arrange, ARRANGEMENTS};
    pub use gutenberg_core::config::Arrangement;

    /// Writes `path` (iced_test adds `-tiny-skia.png`): the window at `size`
    /// in `tokens` and `arrangement`, with an article open, read and starred
    /// rows, and a folder chosen in the sources pane.
    pub fn render(
        tokens: Tokens,
        size: (f32, f32),
        arrangement: Arrangement,
        path: &std::path::Path,
    ) {
        let mut store = Store::open_in_memory().expect("in-memory store");
        let news = store.folder_named("News").expect("folder").id;
        let feeds = [
            add_feed(&store, "PennLive", Some(news)),
            add_feed(&store, "ProPublica", Some(news)),
            add_feed(&store, "LWN", None),
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
        app.width = size.0;
        app.set_arrangement(arrangement);
        app.select(4);
        let _ = app.update(Message::Key(Action::Star));
        app.open(3);
        app.open(4);
        app.select(1);
        let _ = app.update(Message::Key(Action::Star));
        app.open(0);
        app.choose(Entry::Folder(news));
        app.open(0);
        snapshot(&app, size, path);
    }

    fn add_feed(store: &Store, title: &str, folder: Option<i64>) -> i64 {
        let slug: String = title
            .to_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        store
            .add_source(&NewSource {
                folder_id: folder,
                kind: SourceKind::Feed,
                title: title.into(),
                url: Some(format!("https://{slug}.example/feed")),
                config: "{}".into(),
            })
            .expect("feed")
            .id
    }

    fn snapshot(app: &App, size: (f32, f32), path: &std::path::Path) {
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

    /// `(source, folder, headline, hours ago, publisher)`. A publisher
    /// marks a keyword alert's result.
    const HEADLINES: &[(&str, &str, &str, f32, Option<&str>)] = &[
        (
            "Field Notes",
            "Science",
            "A quiet comet, a loud discovery: water ice where none should be",
            0.2,
            None,
        ),
        (
            "Harbor Ledger",
            "World",
            "Port cities trade notes on keeping the lights on during heat waves",
            0.6,
            None,
        ),
        (
            "Kernel Weekly",
            "Linux",
            "Scheduler rework lands with a promise: no more stutter under load",
            1.1,
            None,
        ),
        (
            r#""heat pumps""#,
            "Alerts",
            "Cold-climate heat pumps pass their first real winter test",
            1.4,
            Some("Northline Energy Desk"),
        ),
        (
            "Byte Review",
            "Technology",
            "The case for software that does one thing and stays out of the way",
            2.0,
            None,
        ),
        (
            "Harbor Ledger",
            "World",
            "Ferry strike ends after a night of talks and cold coffee",
            2.6,
            None,
        ),
        (
            "Tiling Weekly",
            "Linux",
            "Ten minutes with a tiling window manager changed how I read",
            3.2,
            None,
        ),
        (
            "Field Notes",
            "Science",
            "Why the oldest trees in the valley are suddenly growing faster",
            3.9,
            None,
        ),
        (
            r#""heat pumps""#,
            "Alerts",
            "City council approves rebates for apartment retrofits",
            4.5,
            Some("Metro Bulletin"),
        ),
        (
            "Kernel Weekly",
            "Linux",
            "Filesystem maintainers agree on a slower, safer release rhythm",
            5.3,
            None,
        ),
        (
            "Byte Review",
            "Technology",
            "Keyboard-first apps are having a moment. Here is why it sticks",
            6.0,
            None,
        ),
        (
            "Harbor Ledger",
            "World",
            "A lighthouse keeper's log, digitized after 140 years",
            7.4,
            None,
        ),
        (
            "Tiling Weekly",
            "Linux",
            "Dotfiles as a diary: what five years of config commits reveal",
            8.8,
            None,
        ),
        (
            "Field Notes",
            "Science",
            "Bees remember faces. Researchers now know how",
            10.5,
            None,
        ),
        (
            "Byte Review",
            "Technology",
            "Local-first software, explained with a grocery list",
            12.0,
            None,
        ),
        (
            "Kernel Weekly",
            "Linux",
            "Release candidate 3 is boring, and that is the point",
            14.2,
            None,
        ),
        (
            "Harbor Ledger",
            "World",
            "Night markets return to the old quarter",
            18.0,
            None,
        ),
        (
            "Field Notes",
            "Science",
            "The sound of a glacier calving, recorded from a kayak",
            22.0,
            None,
        ),
    ];

    /// The alert's first story as seven more outlets headlined it.
    const RETOLD: [(&str, &str); 7] = [
        (
            "Cold-climate heat pumps pass first real winter test",
            "Valley Courier",
        ),
        (
            "Heat pumps pass their first real winter test in the cold",
            "Ridge Tribune",
        ),
        (
            "Cold-climate heat pumps pass a real winter test, study finds",
            "The Plainsman",
        ),
        (
            "Heat pumps pass first winter test in cold climates",
            "Lakeside Post",
        ),
        (
            "Cold-climate heat pumps pass first real test of winter",
            "Northern Wire",
        ),
        (
            "First real winter test: cold-climate heat pumps pass",
            "Coastal Daily",
        ),
        (
            "Cold-climate heat pumps pass their winter test",
            "Prairie Herald",
        ),
    ];

    /// An MCP server already reached, its papers tool picked.
    const MCP_SERVER: &str = r#"{"url":"https://huggingface.co/mcp","tool":"paper_search",
        "arguments":"{\"query\":\"small language models\",\"results_limit\":20}",
        "called":"2026-10-26","tools":[
        {"name":"paper_search","description":"Find machine learning research papers on Hugging Face"},
        {"name":"hub_repo_search","description":"Search Hugging Face repositories: models, datasets and Spaces","example":"{\"query\":\"speech to text\",\"repo_types\":[\"model\"],\"limit\":20}"},
        {"name":"space_search","description":"Find Spaces, the apps people build with models"},
        {"name":"hf_doc_search","description":"Search the documentation of Hugging Face's libraries"}]}"#;

    /// What the MCP server's tool answered, oldest of the sample.
    const PAPERS: [(&str, f32); 3] = [
        ("Small language models that know when to stop", 24.0),
        ("Distilling a reasoning model into 1B parameters", 26.0),
        ("A benchmark for models that run on a laptop", 28.0),
    ];

    const PAPER: &str = r#"<p><strong>Abstract.</strong> Small models answer quickly but rarely
        know when they've said enough. We train a 1B model to <em>stop early</em> when its
        answer is settled, cutting output by a third with no loss on four benchmarks.</p>
        <h2>Results</h2>
        <ul><li>34% fewer tokens on average.</li><li>Accuracy within 0.2 points of the full model.</li>
        <li>Runs on a laptop CPU at 40 tokens a second.</li></ul>"#;

    /// The story the reader-view scenes open: what `v` fetches.
    pub const FEATURE: usize = 0;

    const FIGURE: &str = "https://fieldnotes.example/img/comet.svg";

    /// The feed's own summary of the feature: short, as feeds are.
    const FEATURE_SUMMARY: &str = r#"<p>A small comet on a long orbit is carrying <strong>water ice
        near its surface</strong>, closer to the sun than models allow.</p>
        <p>Press <code>v</code> for the whole story.</p>"#;

    /// The feature's page as the reader view keeps it.
    const FEATURE_PAGE: &str = r#"<article>
        <h1>A quiet comet, a loud discovery</h1>
        <p>For three nights in August, a small telescope on a ridge above the valley
        tracked a comet nobody had paid much attention to. It was faint, slow and,
        by every model, <em>dry</em>: too close to the sun for long to keep any ice
        near its surface.</p>
        <p><img src="https://fieldnotes.example/img/comet.svg" alt="The comet's tail against the evening sky"></p>
        <p>The spectrum said otherwise. The light coming off its tail carried the
        clear signature of <strong>water ice</strong>, and not a trace of it, either:
        enough to change how long astronomers think comets like this one last.</p>
        <h2>Why it matters</h2>
        <ul>
          <li>Ice this close in means the crust insulates far better than assumed.</li>
          <li>It may explain the comet's odd brightening in spring.</li>
          <li>It gives the next mission a cheaper target.</li>
        </ul>
        <blockquote><p>"We went looking for dust and found weather," said the
        observatory's director.</p></blockquote>
        <p>The team has published its raw spectra so others can check the work.</p>
    </article>"#;

    const STORY: &str = r#"<p>Reporters spent a week with the people closest to it. What they found
        was less a single turning point than a <em>long series of small, sensible decisions</em>
        that added up.</p>
        <h2>What we know</h2>
        <ul><li>The change began quietly, months before anyone wrote about it.</li>
        <li>It held up through the worst week of the year.</li>
        <li>Others are already copying it.</li></ul>
        <blockquote><p>"Nobody planned for this to be the story," one of them said.</p></blockquote>
        <p>Read on for how it happened, and what comes next.</p>"#;

    const TECH: &str = r#"<p>The release notes are short, which is the best news in them. The
        maintainers spent the cycle on <strong>fewer surprises</strong> rather than more features.</p>
        <h2>Try it</h2>
        <pre>$ sudo pacman -Syu
$ uname -r</pre>
        <p>If something regresses, the <a href="https://example.com/bugs">tracker</a>
        is open and the bisect is cheap.</p>"#;

    /// A simple mark for each source, in the place a site's icon goes.
    fn icon_for(source: &str) -> Option<&'static str> {
        Some(match source {
            "Field Notes" => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" rx="3" fill="#2f9e7a"/><circle cx="8" cy="8" r="3.5" fill="#e8fff6"/></svg>"##
            }
            "Harbor Ledger" => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" rx="3" fill="#2a5db0"/><path d="M3 10h10l-2 3H5z" fill="#eaf1ff"/><path d="M8 3v7" stroke="#eaf1ff" stroke-width="1.5"/></svg>"##
            }
            "Kernel Weekly" => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" rx="3" fill="#d9822b"/><path d="M4 5l3 3-3 3M8 11h4" stroke="#fff6ea" stroke-width="1.6" fill="none"/></svg>"##
            }
            "Byte Review" => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" rx="3" fill="#8b5cf6"/><rect x="4" y="4" width="8" height="8" rx="1" fill="none" stroke="#f3eeff" stroke-width="1.6"/></svg>"##
            }
            "Tiling Weekly" => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" rx="3" fill="#c2415d"/><path d="M3 3h4v10H3zM9 3h4v4H9zM9 9h4v4H9z" fill="#ffeef2"/></svg>"##
            }
            "Hugging Face" => {
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" rx="3" fill="#e0a526"/><circle cx="8" cy="8" r="5" fill="#fff3d6"/><circle cx="6.2" cy="7" r="0.9" fill="#3a2a00"/><circle cx="9.8" cy="7" r="0.9" fill="#3a2a00"/><path d="M5.6 9.4q2.4 2 4.8 0" stroke="#3a2a00" stroke-width="1" fill="none"/></svg>"##
            }
            _ => return None,
        })
    }

    /// The feature's figure: a comet over a ridge.
    const FIGURE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 300" width="640" height="300">
        <defs>
          <linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stop-color="#0b1026"/><stop offset="0.7" stop-color="#2b2f5c"/><stop offset="1" stop-color="#c9725a"/>
          </linearGradient>
          <linearGradient id="tail" x1="1" y1="0" x2="0" y2="1">
            <stop offset="0" stop-color="#e6f4ff" stop-opacity="0.95"/><stop offset="1" stop-color="#7fb6ff" stop-opacity="0"/>
          </linearGradient>
        </defs>
        <rect width="640" height="300" fill="url(#sky)"/>
        <g fill="#ffffff" opacity="0.8">
          <circle cx="40" cy="30" r="1.2"/><circle cx="120" cy="70" r="0.9"/><circle cx="210" cy="24" r="1.4"/>
          <circle cx="300" cy="90" r="0.8"/><circle cx="560" cy="40" r="1.1"/><circle cx="600" cy="120" r="0.9"/>
          <circle cx="80" cy="140" r="0.7"/><circle cx="350" cy="30" r="1"/><circle cx="500" cy="80" r="0.8"/>
        </g>
        <path d="M470 70 L170 190 L190 205 Z" fill="url(#tail)"/>
        <circle cx="470" cy="70" r="7" fill="#f4fbff"/>
        <path d="M0 250 L90 215 L160 238 L250 196 L330 230 L420 205 L520 240 L640 210 L640 300 L0 300 Z" fill="#141626"/>
        <rect x="300" y="219" width="10" height="14" fill="#141626"/><path d="M296 221 l9 -10 l9 10 z" fill="#141626"/>
      </svg>"##;

    /// The window over the sample sources, one scene step at a time.
    pub struct Demo {
        app: App,
        size: (f32, f32),
        icons: HashMap<i64, Icon>,
        url: String,
        /// The MCP server among the sources.
        server: Option<i64>,
    }

    impl Demo {
        /// Five feeds in four folders and a keyword alert, eighteen
        /// articles, a few read and starred, the feature open.
        pub fn new(tokens: Tokens, size: (f32, f32)) -> Self {
            let mut store = Store::open_in_memory().expect("in-memory store");
            let now = unix_now();
            let mut sources: Vec<(&str, i64)> = Vec::new();
            let mut icons = HashMap::new();
            let mut feature_url = String::new();
            let mut articles = Vec::new();
            for (i, &(source, folder, title, hours, publisher)) in HEADLINES.iter().enumerate() {
                let id = match sources.iter().find(|(s, _)| *s == source) {
                    Some(&(_, id)) => id,
                    None => {
                        let folder = (folder != "Alerts")
                            .then(|| store.folder_named(folder).expect("folder").id);
                        let id = match publisher {
                            Some(_) => {
                                let query = keyword::Keyword::new(source).expect("query");
                                store
                                    .add_source(&NewSource {
                                        folder_id: folder,
                                        kind: SourceKind::Keyword,
                                        title: source.into(),
                                        url: None,
                                        config: query.to_config(),
                                    })
                                    .expect("keyword")
                                    .id
                            }
                            None => add_feed(&store, source, folder),
                        };
                        if let Some(svg) = icon_for(source) {
                            icons
                                .insert(id, Icon::Vector(svg::Handle::from_memory(svg.as_bytes())));
                        }
                        sources.push((source, id));
                        id
                    }
                };
                let url = format!("https://stories.example/{i}");
                let summary = if i == FEATURE {
                    feature_url = url.clone();
                    FEATURE_SUMMARY
                } else if folder == "Linux" || folder == "Technology" {
                    TECH
                } else {
                    STORY
                };
                articles.push(NewArticle {
                    source_id: id,
                    guid: i.to_string(),
                    url: Some(url),
                    title: title.into(),
                    author: Some(publisher.unwrap_or("Staff").into()),
                    summary_html: Some(summary.into()),
                    published_at: Some(now - (hours * 3_600.0) as i64),
                    fetched_at: now,
                });
            }
            // The alert's first story, as other outlets ran it: one row.
            let (_, alert) = sources
                .iter()
                .find(|(s, _)| s.starts_with('"'))
                .copied()
                .expect("the keyword alert");
            for (i, &(title, publisher)) in RETOLD.iter().enumerate() {
                articles.push(NewArticle {
                    source_id: alert,
                    guid: format!("retold-{i}"),
                    url: Some(format!("https://retold.example/{i}")),
                    title: title.into(),
                    author: Some(publisher.into()),
                    summary_html: Some(STORY.into()),
                    published_at: Some(now - ((1.5 + i as f32 * 0.4) * 3_600.0) as i64),
                    fetched_at: now,
                });
            }
            let server = store
                .add_source(&NewSource {
                    folder_id: None,
                    kind: SourceKind::Mcp,
                    title: "Hugging Face".into(),
                    url: None,
                    config: MCP_SERVER.into(),
                })
                .expect("MCP server")
                .id;
            if let Some(svg) = icon_for("Hugging Face") {
                icons.insert(
                    server,
                    Icon::Vector(svg::Handle::from_memory(svg.as_bytes())),
                );
            }
            for (i, &(title, hours)) in PAPERS.iter().enumerate() {
                articles.push(NewArticle {
                    source_id: server,
                    guid: format!("paper-{i}"),
                    url: Some(format!("https://huggingface.co/papers/{i}")),
                    title: title.into(),
                    author: Some("Hugging Face Papers".into()),
                    summary_html: Some(PAPER.into()),
                    published_at: Some(now - (hours * 3_600.0) as i64),
                    fetched_at: now,
                });
            }
            store.insert_articles(&articles).expect("articles");
            gutenberg_core::story::group_stored(&mut store).expect("stories");

            let mut app = App::with_store(Config::default(), None, Ok(store), None);
            app.tokens = tokens;
            app.height = size.1;
            for i in [5, 9, 12, 14] {
                app.open(i);
            }
            for i in [2, 7] {
                app.select(i);
                let _ = app.update(Message::Key(Action::Star));
            }
            app.select(FEATURE);
            app.open(FEATURE);
            Self {
                app,
                size,
                icons,
                url: feature_url,
                server: Some(server),
            }
        }

        /// A fresh install: Add sources with the starter feeds ticked.
        pub fn first_run(tokens: Tokens, size: (f32, f32)) -> Self {
            let store = Store::open_in_memory().expect("in-memory store");
            let mut app = App::with_store(Config::default(), None, Ok(store), None);
            app.tokens = tokens;
            app.height = size.1;
            Self {
                app,
                size,
                icons: HashMap::new(),
                url: String::new(),
                server: None,
            }
        }

        pub fn set_tokens(&mut self, tokens: Tokens) {
            self.app.tokens = tokens;
        }

        fn key(&mut self, action: Action) {
            let _ = self.app.update(Message::Key(action));
        }

        /// `j`.
        pub fn down(&mut self) {
            self.key(Action::Down);
        }

        /// `k`.
        pub fn up(&mut self) {
            self.key(Action::Up);
        }

        /// `o`.
        pub fn open(&mut self) {
            self.key(Action::Open);
        }

        /// `s`.
        pub fn star(&mut self) {
            self.key(Action::Star);
        }

        /// `u`.
        pub fn unread(&mut self) {
            self.key(Action::View(View::Unread));
        }

        /// `A`.
        pub fn all(&mut self) {
            self.key(Action::View(View::All));
        }

        /// Picks a folder in the sources pane by name.
        pub fn folder(&mut self, name: &str) {
            if let Some(id) = self
                .app
                .folders
                .iter()
                .find(|f| f.name == name)
                .map(|f| f.id)
            {
                self.app.choose(Entry::Folder(id));
            }
        }

        /// Picks All articles in the sources pane.
        pub fn everything(&mut self) {
            self.app.choose(Entry::View(View::All));
        }

        /// Selects and opens the feature.
        pub fn open_feature(&mut self) {
            if let Some(i) = self
                .app
                .rows
                .iter()
                .position(|r| r.url.as_deref() == Some(&self.url))
            {
                self.app.select(i);
                self.app.open(i);
            }
        }

        /// Selects and opens the story the alert and seven more outlets
        /// carry.
        pub fn open_story(&mut self) {
            if let Some(i) = self.app.rows.iter().position(|r| r.story_size > 1) {
                self.app.select(i);
                self.app.open(i);
            }
        }

        /// Picks the MCP server in the sources pane.
        pub fn server(&mut self) {
            if let Some(id) = self.server {
                self.app.choose(Entry::Source { id, filed: false });
            }
        }

        /// `T`: the server's tools, or its articles again.
        pub fn tools(&mut self) {
            self.key(Action::Tools);
        }

        /// A key typed into a tool's arguments.
        pub fn arguments_type(&mut self, c: char) {
            let _ = self
                .app
                .update(Message::ToolArguments(palette::Input::Type(c.to_string())));
        }

        /// `Backspace` in a tool's arguments.
        pub fn arguments_back(&mut self) {
            let _ = self
                .app
                .update(Message::ToolArguments(palette::Input::Backspace));
        }

        /// `Esc` in a tool's arguments.
        pub fn arguments_close(&mut self) {
            let _ = self
                .app
                .update(Message::ToolArguments(palette::Input::Close));
        }

        /// `v`: the page arrives at once, its figure with it.
        pub fn reader(&mut self) {
            self.key(Action::Reader);
            let page = Answer::Page {
                url: self.url.clone(),
                html: FEATURE_PAGE.into(),
            };
            let _ = self.app.update(Message::WebAnswer(Arc::new(page)));
            self.app.pictures.insert(
                FIGURE.into(),
                article::Picture::Vector(svg::Handle::from_memory(FIGURE_SVG.as_bytes())),
            );
        }

        /// `/`.
        pub fn search(&mut self) {
            self.key(Action::Search);
        }

        /// A key typed into the search line.
        pub fn search_type(&mut self, c: char) {
            let _ = self
                .app
                .update(Message::Search(palette::Input::Type(c.to_string())));
        }

        /// `Esc` from the search line, then from the results.
        pub fn end_search(&mut self) {
            let _ = self.app.update(Message::Search(palette::Input::Close));
        }

        /// `:`.
        pub fn palette(&mut self) {
            self.key(Action::Palette);
        }

        /// A key typed into the palette.
        pub fn palette_type(&mut self, c: char) {
            let _ = self
                .app
                .update(Message::Palette(palette::Input::Type(c.to_string())));
        }

        /// `↓` in the palette.
        pub fn palette_down(&mut self) {
            let _ = self.app.update(Message::Palette(palette::Input::Down));
        }

        /// `?`.
        pub fn help(&mut self) {
            self.key(Action::Help);
        }

        /// `Esc` on the palette or help.
        pub fn close(&mut self) {
            let _ = self.app.update(Message::CloseOverlay);
        }

        /// `a`.
        pub fn add_sources(&mut self) {
            self.key(Action::AddSources);
        }

        /// `j` on Add sources.
        pub fn sources_down(&mut self) {
            let _ = self
                .app
                .update(Message::Onboarding(onboarding::Action::Down));
        }

        /// `l` on Add sources: the next category.
        pub fn sources_next(&mut self) {
            let _ = self
                .app
                .update(Message::Onboarding(onboarding::Action::NextCategory));
        }

        /// `Space` on Add sources.
        pub fn sources_toggle(&mut self) {
            let _ = self
                .app
                .update(Message::Onboarding(onboarding::Action::ToggleCursor));
        }

        /// `n` on Add sources: the keyword alerts page, typing.
        pub fn new_query(&mut self) {
            let _ = self
                .app
                .update(Message::Onboarding(onboarding::Action::NewQuery));
        }

        /// A key typed into a keyword alert's query.
        pub fn query_type(&mut self, c: char) {
            let _ = self
                .app
                .update(Message::OnboardingQuery(palette::Input::Type(
                    c.to_string(),
                )));
        }

        /// `Esc` on Add sources.
        pub fn close_sources(&mut self) {
            let _ = self
                .app
                .update(Message::Onboarding(onboarding::Action::Close));
        }

        /// Writes `path` with `.png` (iced_test names it `<path>-tiny-skia.png`;
        /// this moves it in place).
        pub fn shot(&mut self, path: &std::path::Path) {
            // A reload reads icons from disk, and the sample's live in memory.
            self.app.source_icons = self.icons.clone();
            let written = PathBuf::from(format!("{}-tiny-skia.png", path.display()));
            let _ = std::fs::remove_file(&written);
            snapshot(&self.app, self.size, path);
            std::fs::rename(&written, path.with_extension("png")).expect("rename the screenshot");
        }
    }
}

/// The process's own memory (heap and other anonymous pages) against all
/// it has resident, which counts the GPU driver's libraries too.
#[cfg(target_os = "linux")]
fn memory_report() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status
            .lines()
            .find_map(|line| {
                line.strip_prefix(name)?
                    .trim()
                    .strip_suffix(" kB")?
                    .parse::<u64>()
                    .ok()
            })
            .map_or("?".to_owned(), |kb| format!("{} MB", kb / 1024))
    };
    format!(
        "{} own (RssAnon), {} resident (VmRSS), {} at most (VmHWM)",
        field("RssAnon:"),
        field("VmRSS:"),
        field("VmHWM:")
    )
}

/// What the process has resident now and at its peak. macOS doesn't set
/// the process's own pages apart from its libraries', so there is no "own"
/// figure here.
#[cfg(target_os = "macos")]
fn memory_report() -> String {
    use std::ffi::{c_int, c_void};
    let mb = |bytes: Option<u64>| {
        bytes.map_or("?".to_owned(), |bytes| {
            format!("{} MB", bytes / (1024 * 1024))
        })
    };
    // SAFETY: each call fills a plain struct of integers on the stack and is
    // given that struct's size; on failure it writes nothing and says so.
    let resident = unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as c_int;
        let written = libc::proc_pidinfo(
            std::process::id() as c_int,
            libc::PROC_PIDTASKINFO,
            0,
            std::ptr::from_mut(&mut info).cast::<c_void>(),
            size,
        );
        (written == size).then_some(info.pti_resident_size)
    };
    // `ru_maxrss` is in bytes on macOS (kilobytes on Linux).
    let peak = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        (libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0)
            .then(|| u64::try_from(usage.ru_maxrss).unwrap_or(0))
    };
    format!(
        "{} resident, {} at most (own memory isn't separable on macOS)",
        mb(resident),
        mb(peak)
    )
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

/// The socket's commands for the window, and live theme reload. Two
/// things ask for a reload: an Omarchy hook through the socket (`gutenberg
/// --reload-theme`), and the inotify watch on Omarchy's theme and
/// fontconfig, for installs without the hooks. Each reads the theme and
/// the font again here, off the UI thread.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Subscription::run_with passes &Inbox"
)]
fn window_commands(inbox: &Inbox) -> impl Stream<Item = Message> + use<> {
    use gutenberg_theme::{font, watch};
    use iced::futures::{SinkExt, StreamExt};
    let channel = inbox.take();
    iced::stream::channel(4, async move |mut output| {
        let Some((asks, mut asked)) = channel else {
            return;
        };
        let on_watch = move || {
            let _ = asks.unbounded_send(ipc::Command::ReloadTheme);
        };
        let _watching = watch::watch(watch::watched(), on_watch)
            .map_err(|err| eprintln!("gutenberg: watching the theme: {err}"))
            .ok();
        let mut family = font::monospace().map(|face| face.family);
        while let Some(command) = asked.next().await {
            if command != ipc::Command::ReloadTheme {
                if command == ipc::Command::Focus && output.send(Message::Focus).await.is_err() {
                    return;
                }
                continue;
            }
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
/// answer. With a store, the worker keeps what it fetches in the disk cache
/// under `[cache]`'s cap; a new cap starts a new worker.
fn web_worker(
    (store_path, cache): &(Option<PathBuf>, config::Cache),
) -> impl Stream<Item = Message> + use<> {
    use iced::futures::{SinkExt, StreamExt};
    let keep = web::Keep {
        // Opened, and pruned to the cap, on the worker's thread.
        cache: store_path
            .as_ref()
            .and_then(|_| gutenberg_core::cache::Cache::default_dir())
            .map(|dir| (dir, cache.bytes())),
        store: store_path.clone(),
    };
    iced::stream::channel(16, async move |mut output| {
        let (answer_tx, mut answers) = iced::futures::channel::mpsc::unbounded();
        let web = match Web::spawn_keeping(keep, move |answer| {
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

/// Opens `url` in the default browser. `xdg-open` (`open` on macOS) runs
/// detached; a thread reaps it so the window never waits on it.
fn open_in_browser(url: &str) {
    use std::process::{Command, Stdio};
    const OPENER: &str = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let spawned = Command::new(OPENER)
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

/// A share destination's letter and name, highlighted when the cursor is
/// on it.
fn share_row<'a>(
    letter: Option<char>,
    name: &str,
    highlighted: bool,
    t: &Tokens,
) -> Element<'a, Message> {
    let letter = letter.map_or_else(|| "  ".to_owned(), |c| c.to_string());
    let selection = t.selection;
    container(
        row![
            text(letter)
                .size(13)
                .font(Font::MONOSPACE)
                .color(color(t.accent)),
            text(name.to_owned()).size(14).color(color(t.foreground)),
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
) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
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
        app_from(articles, false)
    }

    /// `app_with`, with a link on every article, so `O`, `y` and `p` have
    /// something to work on.
    fn app_with_links(articles: usize) -> App {
        app_from(articles, true)
    }

    fn app_from(articles: usize, links: bool) -> App {
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
                url: links.then(|| format!("https://site.example/{i}")),
                title: format!("Article {i}"),
                author: None,
                summary_html: Some(format!("<p>Body {i}</p>")),
                published_at: Some(1_000 + i as i64),
                fetched_at: 1,
            })
            .collect();
        store.insert_articles(&rows).unwrap();
        let mut app = App::with_store(Config::default(), None, Ok(store), None);
        app.height = 520.0 + HINT_BAR; // 10 whole rows above the key bar
        app
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn dragging_a_divider_resizes_the_panes_and_keeps_the_widths_in_the_store() {
        let mut app = app_with(1);
        app.width = 1200.0;
        assert_eq!(app.dividers.len(), 2);
        let split = app.dividers[0].0;
        let _ = app.update(Message::DividerDragged(pane_grid::ResizeEvent {
            split,
            ratio: 0.3,
        }));
        assert!(
            near(app.sizes().widths.sources, 0.3),
            "{:?}",
            app.sizes().widths
        );
        assert!(near(
            app.sizes().widths.list / app.sizes().widths.article,
            2.0 / 3.0
        ));
        let stored = app.store.as_ref().unwrap().setting(panes::SETTING).unwrap();
        assert_eq!(stored.as_deref(), Some(app.sizes().to_setting().as_str()));
        // The grid moved with it, without being rebuilt mid-drag.
        assert_eq!(app.grid.layout().splits().count(), 2);
    }

    #[test]
    fn stored_widths_beat_config_at_launch_and_reset_forgets_them() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting(panes::SETTING, "0.3,0.3,0.4").unwrap();
        let mut app = App::with_store(Config::default(), None, Ok(store), None);
        assert!(
            near(app.sizes().widths.sources, 0.3),
            "{:?}",
            app.sizes().widths
        );
        assert!(app.sizes_stored[0]);

        let _ = app.update(Message::Key(Action::ResetPanes));
        assert_eq!(
            app.sizes().widths,
            panes::Widths::from_layout(Config::default().layout)
        );
        assert!(!app.sizes_stored[0]);
        assert_eq!(
            app.store.as_ref().unwrap().setting(panes::SETTING).unwrap(),
            None
        );
    }

    #[test]
    fn junk_in_the_store_falls_back_to_config() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting(panes::SETTING, "wide").unwrap();
        let app = App::with_store(Config::default(), None, Ok(store), None);
        assert_eq!(
            app.sizes().widths,
            panes::Widths::from_layout(Config::default().layout)
        );
        assert!(!app.sizes_stored[0]);
    }

    #[test]
    fn the_keys_resize_the_pane_that_has_them() {
        let mut app = app_with(1);
        app.width = 1200.0;
        let before = app.sizes().widths;
        let _ = app.update(Message::Key(Action::WidenPane));
        assert!(
            near(app.sizes().widths.list, before.list + panes::STEP),
            "{:?}",
            app.sizes().widths
        );
        assert!(near(
            app.sizes().widths.article,
            before.article - panes::STEP
        ));
        assert!(near(app.sizes().widths.sources, before.sources));
        // Tab: the sources pane has the keys, and `<` narrows it.
        let _ = app.update(Message::Key(Action::NextPane));
        let _ = app.update(Message::Key(Action::NarrowPane));
        assert!(
            near(app.sizes().widths.sources, before.sources - panes::STEP),
            "{:?}",
            app.sizes().widths
        );
        assert!(app.sizes_stored[0]);
    }

    #[test]
    fn a_config_change_keeps_dragged_widths_but_still_decides_what_shows() {
        let mut app = app_with(1);
        app.width = 1200.0;
        let _ = app.update(Message::Key(Action::WidenPane));
        let dragged = app.sizes().widths;
        let mut config = Config::default();
        config.layout.sources = 0;
        let _ = app.update(Message::ConfigChanged(config));
        assert_eq!(app.sizes().widths.sources, 0.0);
        assert_eq!(app.dividers.len(), 1);
        assert!(near(
            app.sizes().widths.list / app.sizes().widths.article,
            dragged.list / dragged.article
        ));
        // Showing it again gives it the config's share; the others keep theirs.
        let _ = app.update(Message::ConfigChanged(Config::default()));
        assert_eq!(app.dividers.len(), 2);
        assert!(app.sizes().widths.sources > 0.0);
        assert!(near(
            app.sizes().widths.list / app.sizes().widths.article,
            dragged.list / dragged.article
        ));
        // Without stored widths, a config change is just applied.
        let _ = app.update(Message::Key(Action::ResetPanes));
        let mut config = Config::default();
        config.layout.list = 8;
        let layout = config.layout;
        let _ = app.update(Message::ConfigChanged(config));
        assert_eq!(app.sizes().widths, panes::Widths::from_layout(layout));
    }

    fn setting(app: &App, key: &str) -> Option<String> {
        app.store.as_ref().unwrap().setting(key).unwrap()
    }

    #[test]
    fn l_cycles_the_layouts_and_each_is_kept_for_the_next_launch() {
        let mut app = app_with(1);
        assert_eq!(app.arrangement, Arrangement::Columns);
        for expected in [
            Arrangement::Classic,
            Arrangement::Wide,
            Arrangement::Reading,
            Arrangement::Columns,
        ] {
            let _ = app.update(Message::Key(Action::NextLayout));
            assert_eq!(app.arrangement, expected);
            assert_eq!(
                setting(&app, panes::ARRANGEMENT_SETTING).as_deref(),
                Some(expected.name())
            );
        }
        let _ = app.update(Message::Key(Action::Layout(Arrangement::Wide)));
        assert_eq!(app.arrangement, Arrangement::Wide);
        assert_eq!(app.grid.len(), 2);
        assert_eq!(app.dividers.len(), 1);
    }

    #[test]
    fn a_stored_layout_beats_config_at_launch_and_junk_falls_back() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_setting(panes::ARRANGEMENT_SETTING, "classic")
            .unwrap();
        store
            .set_setting("layout.classic", "0.3,0.3,0.4;0.6")
            .unwrap();
        let app = App::with_store(Config::default(), None, Ok(store), None);
        assert_eq!(app.arrangement, Arrangement::Classic);
        assert!(near(app.sizes().widths.sources, 0.3) && near(app.sizes().rows, 0.6));
        assert_eq!(app.grid.len(), 3);

        let store = Store::open_in_memory().unwrap();
        store
            .set_setting(panes::ARRANGEMENT_SETTING, "stacked")
            .unwrap();
        let mut config = Config::default();
        config.layout.arrangement = Arrangement::Reading;
        let app = App::with_store(config, None, Ok(store), None);
        assert_eq!(app.arrangement, Arrangement::Reading);
        assert!(!app.arrangement_stored);
    }

    #[test]
    fn esc_leaves_reading_for_the_layout_before_it_and_only_then_clears_a_search() {
        let mut app = app_with(3);
        let _ = app.update(Message::Key(Action::Layout(Arrangement::Classic)));
        let _ = app.update(Message::Key(Action::Search));
        let _ = app.update(Message::Key(Action::Layout(Arrangement::Reading)));
        assert_eq!(app.grid.len(), 1);
        let _ = app.update(Message::Key(Action::ClearSearch));
        assert_eq!(app.arrangement, Arrangement::Classic);
        assert!(app.search.is_some(), "the search is still there");
        let _ = app.update(Message::Key(Action::ClearSearch));
        assert!(app.search.is_none());
        // Reading straight from the config has nothing to go back to but
        // columns.
        let mut config = Config::default();
        config.layout.arrangement = Arrangement::Reading;
        let mut app = App::with_store(config, None, Ok(Store::open_in_memory().unwrap()), None);
        let _ = app.update(Message::Key(Action::ClearSearch));
        assert_eq!(app.arrangement, Arrangement::Columns);
    }

    #[test]
    fn tab_has_nowhere_to_go_in_wide_and_reading_and_the_keys_leave_a_hidden_pane() {
        let mut app = app_with(1);
        let _ = app.update(Message::Key(Action::NextPane));
        assert_eq!(app.pane, Pane::Sources);
        let _ = app.update(Message::Key(Action::Layout(Arrangement::Wide)));
        assert_eq!(app.pane, Pane::List, "the sources pane is gone");
        let _ = app.update(Message::Key(Action::NextPane));
        assert_eq!(app.pane, Pane::List);
        let _ = app.update(Message::Key(Action::Layout(Arrangement::Reading)));
        let _ = app.update(Message::Key(Action::NextPane));
        assert_eq!(app.pane, Pane::List);
        let _ = app.update(Message::Key(Action::Layout(Arrangement::Classic)));
        let _ = app.update(Message::Key(Action::NextPane));
        assert_eq!(app.pane, Pane::Sources);
    }

    #[test]
    fn each_layout_keeps_its_own_divider_positions_and_reset_touches_one() {
        let mut app = app_with(1);
        app.width = 1200.0;
        app.height = 800.0 + HINT_BAR;
        // Columns: widen the list.
        let _ = app.update(Message::Key(Action::WidenPane));
        let columns = *app.sizes();
        // Classic: the same key makes the list taller, and a drag on the
        // rows divider moves rows.
        let _ = app.update(Message::Key(Action::Layout(Arrangement::Classic)));
        let _ = app.update(Message::Key(Action::WidenPane));
        assert!(near(app.sizes().rows, panes::DEFAULT_ROWS + panes::STEP));
        let rows_split = app
            .dividers
            .iter()
            .find(|d| d.1 == panes::Divider::Rows)
            .unwrap()
            .0;
        let _ = app.update(Message::DividerDragged(pane_grid::ResizeEvent {
            split: rows_split,
            ratio: 0.7,
        }));
        assert!(near(app.sizes().rows, 0.7));
        assert_eq!(
            setting(&app, "layout.classic").as_deref(),
            Some(app.sizes().to_setting().as_str())
        );
        // Back in columns, nothing moved.
        let _ = app.update(Message::Key(Action::Layout(Arrangement::Columns)));
        assert_eq!(*app.sizes(), columns);
        // Reset forgets only this arrangement's sizes.
        let _ = app.update(Message::Key(Action::ResetPanes));
        assert_eq!(
            *app.sizes(),
            panes::Sizes::from_layout(Config::default().layout)
        );
        assert_eq!(setting(&app, panes::SETTING), None);
        let _ = app.update(Message::Key(Action::Layout(Arrangement::Classic)));
        assert!(near(app.sizes().rows, 0.7));
    }

    #[test]
    fn the_list_is_only_as_tall_as_its_pane_where_it_sits_above_the_article() {
        let mut app = app_with(40);
        app.height = 800.0 + HINT_BAR;
        app.width = 1200.0;
        assert!(near(app.list_height(), 800.0));
        let _ = app.update(Message::Key(Action::Layout(Arrangement::Classic)));
        assert!(
            near(app.list_height(), 800.0 * panes::DEFAULT_ROWS),
            "{}",
            app.list_height()
        );
        // The selection stays in view in the shorter list.
        let _ = app.update(Message::Key(Action::Last));
        assert!(list::visible(app.top, app.list_height(), app.rows.len()).contains(&app.selected));
        let rows_split = app
            .dividers
            .iter()
            .find(|d| d.1 == panes::Divider::Rows)
            .unwrap()
            .0;
        let _ = app.update(Message::DividerDragged(pane_grid::ResizeEvent {
            split: rows_split,
            ratio: 0.2,
        }));
        assert!(near(app.list_height(), 160.0), "{}", app.list_height());
        assert!(list::visible(app.top, app.list_height(), app.rows.len()).contains(&app.selected));
        // Reading has no list on screen; its height needn't be meaningful,
        // but columns gets the whole height back.
        let _ = app.update(Message::Key(Action::Layout(Arrangement::Columns)));
        assert!(near(app.list_height(), 800.0));
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
    fn the_key_bar_follows_what_is_on_screen() {
        use hints::Screen;
        let mut app = app_with(12);
        assert_eq!(app.screen(), Screen::List);
        let _ = app.update(Message::Key(Action::Open));
        assert_eq!(app.screen(), Screen::Summary);
        let _ = app.update(Message::Key(Action::Search));
        assert_eq!(app.screen(), Screen::Searching);
        let _ = app.update(Message::Search(palette::Input::Run));
        assert_eq!(app.screen(), Screen::Results);
        let _ = app.update(Message::Key(Action::ClearSearch));
        assert_eq!(app.screen(), Screen::Summary);

        let empty = App::with_store(
            Config::default(),
            None,
            Ok(Store::open_in_memory().unwrap()),
            None,
        );
        assert_eq!(empty.screen(), Screen::Empty);
    }

    #[test]
    fn hiding_the_key_bar_gives_its_row_to_the_list() {
        let mut app = app_with(100);
        assert_eq!(app.list_height(), 520.0);
        let mut config = Config::default();
        config.keys.hints = false;
        let _ = app.update(Message::ConfigChanged(config));
        assert_eq!(app.list_height(), 520.0 + HINT_BAR);
        let _ = app.update(Message::Key(Action::Last));
        // 10 whole rows, and the partly shown 11th: the last row is 99.
        assert_eq!(app.top, 90);
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
        // Twice the size: 274 logical px less the 28 px key bar leaves 4
        // whole rows, so row 9 is the fourth.
        assert_eq!(app.top, 6);
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
        app.height = 520.0 + HINT_BAR;
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
    fn an_mcp_server_shows_its_tools_or_why_it_cant_be_reached() {
        let (mut app, ..) = app_with_folders();
        let (reached, broken) = {
            let store = app.store.as_ref().unwrap();
            let add = |title: &str, config: &str| {
                store
                    .add_source(&NewSource {
                        folder_id: None,
                        kind: SourceKind::Mcp,
                        title: title.into(),
                        url: None,
                        config: config.into(),
                    })
                    .unwrap()
                    .id
            };
            let reached = add(
                "Hugging Face",
                r#"{"url":"https://huggingface.co/mcp","tools":[
                    {"name":"hub_repo_search","description":"Search Hugging Face repositories"},
                    {"name":"hf_fs"}]}"#,
            );
            let broken = add("Brave", r#"{"command":"npx","args":["brave-search"]}"#);
            store
                .record_failure(broken, "the server stopped: BRAVE_API_KEY is not set", 9, 1)
                .unwrap();
            (reached, broken)
        };
        app.reload();
        assert_eq!(
            app.source_errors.get(&broken).map(String::as_str),
            Some("the server stopped: BRAVE_API_KEY is not set")
        );
        assert!(app.is_mcp(reached) && !app.is_mcp(app.sources[0].id));

        for (id, filed) in [(reached, false), (broken, false)] {
            let _ = app.update(Message::Press(Entry::Source { id, filed }));
            let _ = app.update(Message::Release);
            assert_eq!(app.chosen_mcp(), Some(id));
            let _ = app.view();
        }
        let tools: Vec<_> = app
            .sources
            .iter()
            .find(|s| s.id == reached)
            .map(mcp::tools)
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(tools, ["hub_repo_search", "hf_fs"]);
        // A search over everything isn't the server's page.
        let _ = app.update(Message::Key(Action::Search));
        assert_eq!(app.chosen_mcp(), None);
    }

    #[test]
    fn a_tool_is_picked_and_its_arguments_typed_in_the_window() {
        let (mut app, ..) = app_with_folders();
        let id = app
            .store
            .as_ref()
            .unwrap()
            .add_source(&NewSource {
                folder_id: None,
                kind: SourceKind::Mcp,
                title: "Hugging Face".into(),
                url: None,
                config: r#"{"url":"https://huggingface.co/mcp","tools":[
                    {"name":"hf_whoami","example":"{}"},
                    {"name":"hf_fs","example":"{\"operations\":[]}"}]}"#
                    .into(),
            })
            .unwrap()
            .id;
        app.reload();
        let _ = app.update(Message::Press(Entry::Source { id, filed: false }));
        let _ = app.update(Message::Release);
        assert_eq!(app.chosen_mcp(), Some(id));
        assert_eq!(app.screen(), hints::Screen::Tools);

        // j moves to hf_fs; Enter starts from its example.
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Open));
        assert_eq!(app.screen(), hints::Screen::Arguments);
        let editor = app.tools.editor.as_ref().unwrap();
        assert_eq!(
            (editor.tool.as_str(), editor.text.as_str()),
            ("hf_fs", r#"{"operations":[]}"#)
        );
        let _ = app.view();

        // Not JSON yet: it says so and keeps the text.
        let _ = app.update(Message::ToolArguments(palette::Input::Backspace));
        let _ = app.update(Message::ToolArguments(palette::Input::Run));
        assert!(app.tools.editor.as_ref().unwrap().error.is_some());
        let _ = app.update(Message::ToolArguments(palette::Input::Type("}".into())));
        let _ = app.update(Message::ToolArguments(palette::Input::Run));
        assert!(app.tools.editor.is_none());
        let store = app.store.as_ref().unwrap();
        let picked = store
            .sources()
            .unwrap()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap();
        let settings = mcp::settings(&picked).unwrap();
        assert_eq!(settings.tool.as_deref(), Some("hf_fs"));
        assert_eq!(settings.arguments.as_deref(), Some(r#"{"operations":[]}"#));
        assert!(store.has_unchecked_sources().unwrap());

        // With articles, the list shows them; T brings the tools back and
        // Esc the articles.
        app.store
            .as_mut()
            .unwrap()
            .insert_articles(&[NewArticle {
                source_id: id,
                guid: "p1".into(),
                url: Some("https://huggingface.co/papers/1".into()),
                title: "A paper".into(),
                author: None,
                summary_html: None,
                published_at: Some(5),
                fetched_at: 5,
            }])
            .unwrap();
        app.reload();
        assert_eq!(app.chosen_mcp(), None);
        let _ = app.update(Message::Key(Action::Tools));
        assert_eq!(app.chosen_mcp(), Some(id));
        let _ = app.update(Message::Key(Action::ClearSearch));
        assert_eq!(app.chosen_mcp(), None);
        assert_eq!(app.rows[0].title, "A paper");
    }

    #[test]
    fn a_servers_key_is_typed_as_dots_and_goes_to_the_keyring() {
        secrets::use_memory_keyring();
        let (mut app, ..) = app_with_folders();
        let id = app
            .store
            .as_ref()
            .unwrap()
            .add_source(&NewSource {
                folder_id: None,
                kind: SourceKind::Mcp,
                title: "Jina".into(),
                url: None,
                config: r#"{"url":"https://mcp.jina.ai/v1","tool":"search_web","secret":"GUTENBERG_UI_TEST_KEY"}"#
                    .into(),
            })
            .unwrap()
            .id;
        app.store
            .as_ref()
            .unwrap()
            .record_failure(id, "needs GUTENBERG_UI_TEST_KEY", 999, 1)
            .unwrap();
        app.reload();
        let _ = app.update(Message::Press(Entry::Source { id, filed: false }));
        let _ = app.update(Message::Release);
        let _ = app.update(Message::Key(Action::EnterKey));
        assert_eq!(app.screen(), hints::Screen::Key);
        for typed in ["jina_", "abc", "d"] {
            let _ = app.update(Message::KeyEntry(palette::Input::Type(typed.into())));
        }
        let _ = app.update(Message::KeyEntry(palette::Input::Backspace));
        let editor = app.tools.key.as_ref().unwrap();
        assert_eq!(editor.text, "jina_abc");
        assert!(!format!("{editor:?}").contains("jina_abc"));
        let _ = app.view();

        // Enter hands it to the keyring and waits; keys do nothing meanwhile.
        let _ = app.update(Message::KeyEntry(palette::Input::Run));
        assert!(app.tools.key.as_ref().unwrap().saving);
        let _ = app.update(Message::KeyEntry(palette::Input::Type("x".into())));
        assert_eq!(app.tools.key.as_ref().unwrap().text, "jina_abc");
        let _ = app.update(Message::KeySaved(id, Err("the keyring is locked".into())));
        let editor = app.tools.key.as_ref().unwrap();
        assert_eq!(editor.error.as_deref(), Some("the keyring is locked"));
        assert!(!editor.saving);

        // The memory keyring answers at once, so any executor does.
        let saved = iced::futures::executor::block_on(async {
            secrets::save("GUTENBERG_UI_TEST_KEY", "jina_abc").await?;
            secrets::lookup("GUTENBERG_UI_TEST_KEY").await
        });
        assert_eq!(saved, Ok(Some("jina_abc".to_owned())));
        let _ = app.update(Message::KeySaved(id, Ok(())));
        assert!(app.tools.key.is_none());
        let store = app.store.as_ref().unwrap();
        assert!(store.has_unchecked_sources().unwrap());
        assert_eq!(store.fetch_state(id).unwrap().error, None);
        assert!(!store
            .sources()
            .unwrap()
            .iter()
            .any(|s| s.config.contains("jina_abc")));

        // A feed has no key to enter.
        let (mut app, _, a, _) = app_with_folders();
        let _ = app.update(Message::Press(Entry::Source { id: a, filed: true }));
        let _ = app.update(Message::Release);
        let _ = app.update(Message::Key(Action::EnterKey));
        assert!(app.tools.key.is_none());
    }

    #[test]
    fn a_story_from_two_feeds_is_one_row_read_at_once() {
        let (mut app, _, a, b) = app_with_folders();
        let id = |app: &App, title: &str| {
            let store = app.store.as_ref().unwrap();
            let rows = store.list_articles(None, 10).unwrap();
            rows.iter().find(|r| r.title == title).unwrap().id
        };
        let (a2, b1) = (id(&app, "a2"), id(&app, "b1"));
        if let Ok(store) = &mut app.store {
            store.set_stories(&[(b1, b1), (a2, b1)]).unwrap();
        }
        app.reload();
        let titles: Vec<_> = app
            .rows
            .iter()
            .map(|r| (r.title.as_str(), r.story_size))
            .collect();
        assert_eq!(titles, [("a1", 1), ("a2", 2)]);
        assert_eq!(counts(&app, ALL), 2);

        app.selected = 1;
        let _ = app.update(Message::Key(Action::Open));
        let b_row = Entry::Source {
            id: b,
            filed: false,
        };
        let a_row = Entry::Source { id: a, filed: true };
        assert_eq!(
            (counts(&app, ALL), counts(&app, a_row), counts(&app, b_row)),
            (1, 1, 0)
        );
        let opened = app.opened.as_ref().unwrap();
        assert_eq!(opened.also, [("b".to_owned(), None)]);

        // m makes the whole story unread again.
        let _ = app.update(Message::Key(Action::ToggleRead));
        assert_eq!(counts(&app, b_row), 1);
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
    fn the_key_bar_names_a_new_release_unless_a_refresh_is_running() {
        let (mut app, ..) = app_with_folders();
        assert_eq!(app.status_line(), None);
        app.newer = Some("0.25.0".into());
        assert_eq!(
            app.status_line().as_deref(),
            Some("Gutenberg 0.25.0 is out · gutenberg --update")
        );
        app.refreshing = Some((1, 4));
        assert_eq!(app.status_line().as_deref(), Some("Refreshing 1 of 4"));
        let _ = app.view();
    }

    #[test]
    fn a_refresh_shows_its_progress_until_it_ends() {
        let (mut app, ..) = app_with_folders();
        app.refreshing = Some((3, 10));
        let _ = app.view();
        // The refresh thread says it's done (here: there is none).
        let _ = app.update(Message::PollProgress);
        assert_eq!(app.refreshing, None);
        let _ = app.view();
    }

    #[test]
    fn tab_gives_the_keys_to_the_sources_pane_where_they_choose_rows() {
        let (mut app, tech, a, b) = app_with_folders();
        let key = |app: &mut App, action| {
            let _ = app.update(Message::Key(action));
        };
        key(&mut app, Action::NextPane);
        assert_eq!(app.screen(), hints::Screen::Sources);

        // Each row is chosen as the keys land on it, as a click would.
        key(&mut app, Action::Last);
        assert_eq!((app.filter, app.rows.len()), (Filter::Source(b), 1));
        key(&mut app, Action::Up);
        assert_eq!((app.filter, app.rows.len()), (Filter::Source(a), 2));
        key(&mut app, Action::Up);
        assert_eq!(app.filter, Filter::Folder(tech));
        key(&mut app, Action::Up);
        assert_eq!((app.filter, app.view), (Filter::All, View::Today));
        key(&mut app, Action::PendingG);
        key(&mut app, Action::First);
        assert_eq!((app.filter, app.view), (Filter::All, View::All));
        key(&mut app, Action::Down);
        assert_eq!(app.view, View::Unread);
        key(&mut app, Action::Up);
        assert_eq!(app.rows.len(), 3);

        // Enter hands the keys to the list, where j moves the selection.
        key(&mut app, Action::Open);
        assert_eq!(app.pane, Pane::List);
        assert!(app.opened.is_none());
        key(&mut app, Action::Down);
        assert_eq!((app.selected, app.filter), (1, Filter::All));

        // Shift-Tab goes back; a click in the list takes the keys again.
        key(&mut app, Action::PrevPane);
        assert_eq!(app.pane, Pane::Sources);
        let _ = app.update(Message::Click(0));
        assert_eq!(app.pane, Pane::List);
        // A search types in the list, wherever the keys were.
        key(&mut app, Action::NextPane);
        key(&mut app, Action::Search);
        assert_eq!(app.pane, Pane::List);
    }

    #[test]
    fn tab_does_nothing_while_the_sources_pane_is_hidden() {
        let (mut app, ..) = app_with_folders();
        let mut config = Config::default();
        config.layout.sources = 0;
        let _ = app.update(Message::ConfigChanged(config));
        let _ = app.update(Message::Key(Action::NextPane));
        assert_eq!(app.pane, Pane::List);
    }

    #[test]
    fn an_empty_list_knows_why() {
        let (mut app, _, _, b) = app_with_folders();
        let choose = |app: &mut App, entry| {
            let _ = app.update(Message::Press(entry));
            let _ = app.update(Message::Release);
        };
        // b's one article is from 1970: the Today view hides it.
        choose(
            &mut app,
            Entry::Source {
                id: b,
                filed: false,
            },
        );
        let _ = app.update(Message::Key(Action::View(View::Today)));
        assert!(app.rows.is_empty());
        assert_eq!(app.empty, Some(list::Empty::Hidden));
        assert_eq!(app.place_name(), "b");
        let _ = app.update(Message::Key(Action::View(View::All)));
        assert_eq!((app.rows.len(), app.empty.clone()), (1, None));

        // A feed added just now waits for the refresh; one that failed says why.
        let store = app.store.as_ref().unwrap();
        let c = store
            .add_source(&NewSource {
                folder_id: None,
                kind: SourceKind::Feed,
                title: "c".into(),
                url: Some("https://c.example/feed".into()),
                config: "{}".into(),
            })
            .unwrap()
            .id;
        app.reload();
        choose(
            &mut app,
            Entry::Source {
                id: c,
                filed: false,
            },
        );
        assert_eq!(app.empty, Some(list::Empty::Unfetched));
        let store = app.store.as_ref().unwrap();
        store.record_failure(c, "HTTP 503", 999, 1).unwrap();
        app.reload();
        assert_eq!(app.empty, Some(list::Empty::Failed("HTTP 503".into())));
        let _ = app.view();
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

    fn composer(app: &App) -> &share::Composer {
        match &app.overlay {
            Some(Overlay::ShareCompose(composer)) => composer,
            _ => panic!("the compose box isn't open"),
        }
    }

    #[test]
    fn p_opens_the_picker_and_a_pick_opens_the_composer_with_the_title() {
        use gutenberg_core::share::Destination;
        let mut app = app_with_links(3);
        let _ = app.update(Message::Key(Action::Down));
        let _ = app.update(Message::Key(Action::Share));
        assert_eq!(app.screen(), hints::Screen::SharePicker);
        let Some(Overlay::SharePicker(picker)) = &app.overlay else {
            panic!("no picker");
        };
        assert_eq!(picker.destinations, Destination::BUILT_IN.to_vec());
        let _ = app.view();

        // Enter on the first row, X: the comment starts as the title.
        let _ = app.update(Message::SharePicker(palette::Input::Run));
        assert_eq!(app.screen(), hints::Screen::ShareCompose);
        let x = composer(&app);
        assert_eq!(x.destination, Destination::X);
        assert_eq!(x.share.text, "Article 1");
        assert_eq!(x.share.url, "https://site.example/1");
        // "Article 1" is 9 characters, a space and a 23-character link.
        assert_eq!(x.remaining(), Some(280 - 9 - 1 - 23));
        let _ = app.view();

        // Typing edits the comment, and the URL Enter would open follows.
        let _ = app.update(Message::ShareCompose(palette::Input::Type("!".into())));
        assert_eq!(
            composer(&app).url(),
            "https://x.com/intent/tweet?text=Article%201%21&url=https%3A%2F%2Fsite.example%2F1"
        );
        let _ = app.update(Message::ShareCompose(palette::Input::Close));
        assert!(app.overlay.is_none());
        assert_eq!(app.screen(), hints::Screen::List);

        // A letter picks its row from anywhere; a click picks a row too.
        let _ = app.update(Message::Key(Action::Share));
        let _ = app.update(Message::SharePicker(palette::Input::Type("f".into())));
        assert_eq!(composer(&app).destination, Destination::Facebook);
        assert!(!composer(&app).takes_text());
        // The bar offers y only here, where it copies instead of typing.
        assert_eq!(app.screen(), hints::Screen::ShareLink);
        let _ = app.view();
        let _ = app.update(Message::ShareCompose(palette::Input::Type("y".into())));
        assert!(composer(&app).copied);
        let _ = app.update(Message::CloseOverlay);
        let _ = app.update(Message::Key(Action::Share));
        let _ = app.update(Message::PickDestination(1));
        assert_eq!(composer(&app).destination, Destination::Bluesky);

        // Esc in the picker closes it without picking.
        let _ = app.update(Message::CloseOverlay);
        let _ = app.update(Message::Key(Action::Share));
        let _ = app.update(Message::SharePicker(palette::Input::Close));
        assert!(app.overlay.is_none());
    }

    #[test]
    fn p_does_nothing_for_an_article_without_a_link() {
        let mut app = app_with(3);
        let _ = app.update(Message::Key(Action::Share));
        assert!(app.overlay.is_none());
        assert_eq!(app.screen(), hints::Screen::List);
        // Nor with no articles at all.
        let mut app = app_with(0);
        let _ = app.update(Message::Key(Action::Share));
        assert!(app.overlay.is_none());
    }

    #[test]
    fn config_order_is_picker_order() {
        use gutenberg_core::config::ShareEntry;
        use gutenberg_core::share::Destination;
        let mut app = app_with_links(1);
        let config = Config {
            share: vec![
                ShareEntry {
                    name: "bluesky".into(),
                    template: None,
                },
                ShareEntry {
                    name: "Mastodon".into(),
                    template: Some("https://mastodon.social/share?text={title}%20{url}".into()),
                },
            ],
            ..Config::default()
        };
        let _ = app.update(Message::ConfigChanged(config));
        let _ = app.update(Message::Key(Action::Share));
        let Some(Overlay::SharePicker(picker)) = &app.overlay else {
            panic!("no picker");
        };
        assert_eq!(picker.destinations.len(), 2);
        assert_eq!(picker.destinations[0], Destination::Bluesky);
        assert_eq!(picker.destinations[1].name(), "Mastodon");
        assert_eq!(picker.letters, [Some('b'), Some('m')]);

        // A config reloaded while the picker is open changes its rows.
        let _ = app.update(Message::ConfigChanged(Config::default()));
        let Some(Overlay::SharePicker(picker)) = &app.overlay else {
            panic!("no picker");
        };
        assert_eq!(picker.destinations, Destination::BUILT_IN.to_vec());
        let _ = app.view();
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
