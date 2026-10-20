//! See CLAUDE.md for what this crate owns.

use std::path::PathBuf;

use gutenberg_core::config::{self, Config};
use gutenberg_theme::{Rgb, Tokens};
use iced::futures::Stream;
use iced::keyboard::{self, Key, Modifiers};
use iced::widget::{container, Row, Space};
use iced::{border, window, Border, Color, Element, Length, Subscription, Task};

/// Opens the window and blocks until it closes. When `config_path` is set,
/// edits to that file apply while the window is open.
pub fn run(config: Config, config_path: Option<PathBuf>) -> iced::Result {
    iced::application(
        move || App::new(config.clone(), config_path.clone()),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Message {
    Quit,
    ConfigChanged(Config),
}

impl App {
    fn new(config: Config, config_path: Option<PathBuf>) -> Self {
        Self {
            tokens: Tokens::fallback(),
            config,
            config_path,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Quit => iced::exit(),
            Message::ConfigChanged(config) => {
                self.config = config;
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let keys = keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed { key, modifiers, .. } => action_for(&key, modifiers),
            _ => None,
        });
        match &self.config_path {
            Some(path) => {
                Subscription::batch([keys, Subscription::run_with(path.clone(), watch_config)])
            }
            None => keys,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let t = &self.tokens;
        let layout = self.config.layout;
        let panes = [
            (layout.sources, t.background),
            (layout.list, t.background),
            (layout.article, t.lighter_background),
        ];
        Row::with_children(
            panes
                .into_iter()
                .filter(|&(width, _)| width > 0)
                .map(|(width, background)| pane(background, t.border, width)),
        )
        .height(Length::Fill)
        .into()
    }
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

/// Maps a key press to an action. Keys are listed in `docs/KEYS.md`.
fn action_for(key: &Key, modifiers: Modifiers) -> Option<Message> {
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    match key.as_ref() {
        Key::Character("q") => Some(Message::Quit),
        _ => None,
    }
}

fn pane<'a>(background: Rgb, border_color: Rgb, portion: u16) -> Element<'a, Message> {
    container(Space::new())
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

    #[test]
    fn q_quits() {
        let q = Key::Character("q".into());
        assert_eq!(action_for(&q, Modifiers::empty()), Some(Message::Quit));
    }

    #[test]
    fn q_with_a_modifier_does_nothing() {
        let q = Key::Character("q".into());
        assert_eq!(action_for(&q, Modifiers::CTRL), None);
        assert_eq!(action_for(&q, Modifiers::LOGO), None);
    }

    #[test]
    fn capital_q_does_nothing() {
        let q = Key::Character("Q".into());
        assert_eq!(action_for(&q, Modifiers::SHIFT), None);
    }
}
