//! See CLAUDE.md for what this crate owns.

use gutenberg_theme::{Rgb, Tokens};
use iced::keyboard::{self, Key, Modifiers};
use iced::widget::{container, row, Space};
use iced::{border, window, Border, Color, Element, Length, Subscription, Task};

/// Opens the window and blocks until it closes.
pub fn run() -> iced::Result {
    iced::application(App::new, App::update, App::view)
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Message {
    Quit,
}

impl App {
    fn new() -> Self {
        Self {
            tokens: Tokens::fallback(),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Quit => iced::exit(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed { key, modifiers, .. } => action_for(&key, modifiers),
            _ => None,
        })
    }

    fn view(&self) -> Element<'_, Message> {
        let t = &self.tokens;
        row![
            pane(t.background, t.border, 2),
            pane(t.background, t.border, 4),
            pane(t.lighter_background, t.border, 6),
        ]
        .height(Length::Fill)
        .into()
    }
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
