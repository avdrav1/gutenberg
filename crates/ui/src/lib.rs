//! See CLAUDE.md for what this crate owns.

mod list;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use gutenberg_core::config::{self, Config};
use gutenberg_core::sync::unix_now;
use gutenberg_store::{ArticleRow, Store};
use gutenberg_theme::{Rgb, Tokens};
use iced::futures::Stream;
use iced::keyboard;
use iced::mouse::ScrollDelta;
use iced::widget::{column, container, mouse_area, scrollable, text, Column, Row, Space};
use iced::{border, font, window, Border, Color, Element, Font, Length, Size, Subscription, Task};

use list::Action;

/// Most articles kept in the list; the PRD plans for 100k.
const LIST_LIMIT: u32 = 100_000;
/// How often to look for writes from the refresh thread or a terminal.
const STORE_POLL: Duration = Duration::from_secs(1);

/// Opens the window and blocks until it closes. When `config_path` is set,
/// edits to that file apply while the window is open. `store_path` is the
/// SQLite file to read articles from.
pub fn run(
    config: Config,
    config_path: Option<PathBuf>,
    store_path: Option<PathBuf>,
) -> iced::Result {
    iced::application(
        move || App::new(config.clone(), config_path.clone(), store_path.as_deref()),
        App::update,
        App::view,
    )
    .title("Gutenberg")
    .subscription(App::subscription)
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

struct App {
    tokens: Tokens,
    config: Config,
    config_path: Option<PathBuf>,
    store: Result<Store, String>,
    /// What the store held at the last reload, newest first.
    rows: Vec<ArticleRow>,
    source_titles: HashMap<i64, String>,
    data_version: i64,
    selected: usize,
    /// First row on screen.
    top: usize,
    /// Height of the panes, from the window.
    height: f32,
    opened: Option<Opened>,
    pending_g: bool,
    bench: Option<Bench>,
}

/// The article shown in the article pane.
struct Opened {
    title: String,
    meta: String,
    body: String,
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
        Self::with_store(config, config_path, store)
    }

    fn with_store(
        config: Config,
        config_path: Option<PathBuf>,
        store: Result<Store, String>,
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
            tokens: Tokens::fallback(),
            config,
            config_path,
            store,
            rows: Vec::new(),
            source_titles: HashMap::new(),
            data_version: 0,
            selected: 0,
            top: 0,
            height: 768.0,
            opened: None,
            pending_g: false,
            bench,
        };
        app.reload();
        app
    }

    /// Re-reads articles and sources, keeping the selection on the same
    /// article when it's still there.
    fn reload(&mut self) {
        let Ok(store) = &self.store else { return };
        let selected_id = self.rows.get(self.selected).map(|r| r.id);
        match (store.list_articles(None, LIST_LIMIT), store.sources()) {
            (Ok(rows), Ok(sources)) => {
                self.rows = rows;
                self.source_titles = sources.into_iter().map(|s| (s.id, s.title)).collect();
            }
            (Err(e), _) | (_, Err(e)) => eprintln!("gutenberg: reading the store: {e}"),
        }
        if let Some(id) = selected_id {
            if let Some(i) = self.rows.iter().position(|r| r.id == id) {
                self.selected = i;
            }
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.top = self.top.min(list::max_top(self.rows.len(), self.height));
        self.data_version = store.data_version().unwrap_or(self.data_version);
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        if !matches!(message, Message::Key(Action::PendingG)) {
            self.pending_g = false;
        }
        match message {
            Message::Key(Action::Quit) => return iced::exit(),
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
                self.top = top.min(list::max_top(self.rows.len(), self.height));
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
            Message::Resized(size) => {
                self.height = size.height;
                self.top = list::scroll_to_show(self.top, self.selected, self.height)
                    .min(list::max_top(self.rows.len(), self.height));
            }
            Message::Frame(at) => return self.bench_frame(at),
            Message::ConfigChanged(config) => self.config = config,
        }
        Task::none()
    }

    fn select(&mut self, i: usize) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = i.min(self.rows.len() - 1);
        self.top = list::scroll_to_show(self.top, self.selected, self.height);
    }

    /// Shows the article and marks it read.
    fn open(&mut self, i: usize) {
        let Some(row) = self.rows.get_mut(i) else {
            return;
        };
        if !row.read {
            if let Ok(store) = &self.store {
                match store.set_read(row.id, true) {
                    Ok(()) => row.read = true,
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
        self.opened = Some(Opened {
            title: row.title.clone(),
            meta: meta.join(" · "),
            body: summary.as_deref().map(list::plain_text).unwrap_or_default(),
        });
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
        let keys = keyboard::listen()
            .with(self.pending_g)
            .filter_map(|(pending_g, event)| match event {
                keyboard::Event::KeyPressed { key, modifiers, .. } => {
                    list::action_for(&key, modifiers, pending_g).map(Message::Key)
                }
                _ => None,
            });
        let mut subscriptions = vec![
            keys,
            iced::time::every(STORE_POLL).map(|_| Message::PollStore),
            window::resize_events().map(|(_, size)| Message::Resized(size)),
        ];
        if let Some(path) = &self.config_path {
            subscriptions.push(Subscription::run_with(path.clone(), watch_config));
        }
        if self.bench.is_some() {
            subscriptions.push(window::frames().map(Message::Frame));
        }
        Subscription::batch(subscriptions)
    }

    fn view(&self) -> Element<'_, Message> {
        let t = &self.tokens;
        let layout = self.config.layout;
        let panes: [(u16, Element<'_, Message>, Rgb); 3] = [
            (layout.sources, Space::new().into(), t.background),
            (layout.list, self.list_view(), t.background),
            (layout.article, self.article_view(), t.lighter_background),
        ];
        Row::with_children(
            panes
                .into_iter()
                .filter(|(width, _, _)| *width > 0)
                .map(|(width, content, background)| pane(content, background, t.border, width)),
        )
        .height(Length::Fill)
        .into()
    }

    fn list_view(&self) -> Element<'_, Message> {
        let t = &self.tokens;
        if let Err(e) = &self.store {
            return note(format!("Can't open the article store: {e}"), t.muted);
        }
        if self.rows.is_empty() {
            return note(
                "No articles yet. Add a feed with gutenberg --add <url>".to_owned(),
                t.muted,
            );
        }
        let now = unix_now();
        let rows = list::visible(self.top, self.height, self.rows.len())
            .map(|i| self.row_view(i, &self.rows[i], now));
        mouse_area(
            Column::with_children(rows)
                .width(Length::Fill)
                .height(Length::Fill)
                .clip(true),
        )
        .on_scroll(Message::Scroll)
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
        let meta = text(format!("{source} · {}", list::age(row.sort_at, now)))
            .size(12)
            .color(color(t.muted))
            .wrapping(text::Wrapping::None);
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
        scrollable(
            column![
                text(&opened.title).size(22).color(color(t.foreground)),
                text(&opened.meta).size(13).color(color(t.muted)),
                text(&opened.body).size(15).color(color(t.foreground)),
            ]
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
            let mut app = App::with_store(Config::default(), None, Ok(store));
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

#[cfg(test)]
mod tests {
    use super::*;
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
        let mut app = App::with_store(Config::default(), None, Ok(store));
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
        assert_eq!(app.opened.as_ref().unwrap().body, "Body 1");
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
}
