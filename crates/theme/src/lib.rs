//! See CLAUDE.md for what this crate owns.

pub mod font;
pub mod watch;

use std::path::{Path, PathBuf};

/// An sRGB color, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn hex(hex: u32) -> Self {
        Self {
            r: (hex >> 16) as u8,
            g: (hex >> 8) as u8,
            b: hex as u8,
        }
    }

    pub const BLACK: Rgb = Rgb::hex(0x000000);
    pub const WHITE: Rgb = Rgb::hex(0xffffff);

    /// `#rrggbb`, `rrggbb` or `#rgb`; anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim().trim_start_matches('#');
        let hex = match hex.len() {
            6 => hex.to_owned(),
            3 => hex.chars().flat_map(|c| [c, c]).collect(),
            _ => return None,
        };
        u32::from_str_radix(&hex, 16).ok().map(Self::hex)
    }

    /// This color moved `amount` (0 to 1) of the way toward `other`.
    pub fn mix(self, other: Rgb, amount: f32) -> Rgb {
        let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
        Rgb {
            r: channel(self.r, other.r),
            g: channel(self.g, other.g),
            b: channel(self.b, other.b),
        }
    }

    /// WCAG relative luminance, 0 (black) to 1 (white).
    pub fn luminance(self) -> f32 {
        let linear = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }
}

/// The WCAG contrast ratio of two colors, 1 (none) to 21 (black on white).
pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (light, dark) = if la > lb { (la, lb) } else { (lb, la) };
    (light + 0.05) / (dark + 0.05)
}

/// Body text against any background it's drawn on.
pub const TEXT_CONTRAST: f32 = 4.5;
/// Secondary text (read titles, meta lines) and links, which are also
/// underlined.
pub const SECONDARY_CONTRAST: f32 = 3.0;

/// The colors the UI draws with. Every color in `crates/ui` comes from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    /// Sources and article list.
    pub background: Rgb,
    /// Article pane.
    pub lighter_background: Rgb,
    pub foreground: Rgb,
    /// Read articles and secondary text.
    pub muted: Rgb,
    /// The selected row.
    pub selection: Rgb,
    /// Links and drop targets.
    pub accent: Rgb,
    /// Lines between panes.
    pub border: Rgb,
}

/// The desktop's light or dark preference, which picks the built-in theme
/// when there's no Omarchy theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scheme {
    /// Also what a desktop with no preference gets.
    #[default]
    Dark,
    Light,
}

impl Scheme {
    /// Light when dark text reads better on `background` than light text.
    pub fn of(background: Rgb) -> Self {
        if contrast(background, Rgb::BLACK) > contrast(background, Rgb::WHITE) {
            Self::Light
        } else {
            Self::Dark
        }
    }
}

impl Tokens {
    /// The built-in theme for `scheme`: Omarchy's Tokyo Night or Catppuccin
    /// Latte, as `from_colors_toml` maps them. Used without Omarchy.
    pub const fn fallback(scheme: Scheme) -> Self {
        match scheme {
            Scheme::Dark => Self::DARK,
            Scheme::Light => Self::LIGHT,
        }
    }

    const DARK: Self = Self {
        background: Rgb::hex(0x1a1b26),
        lighter_background: Rgb::hex(0x24283b),
        foreground: Rgb::hex(0xa9b1d6),
        // Tokyo Night's dark_foreground (#565f89) reads at 2.2:1 on its
        // backgrounds; this is it brightened to 3:1.
        muted: Rgb::hex(0x71799c),
        selection: Rgb::hex(0x292e42),
        accent: Rgb::hex(0x7aa2f7),
        border: Rgb::hex(0x414868),
    };

    const LIGHT: Self = Self {
        background: Rgb::hex(0xeff1f5),
        lighter_background: Rgb::hex(0xdce0e8),
        foreground: Rgb::hex(0x4c4f69),
        // Latte's dark_foreground (#9ca0b0) reads at 1.7:1; this is it
        // darkened to 3:1.
        muted: Rgb::hex(0x70737f),
        selection: Rgb::hex(0xccd0da),
        accent: Rgb::hex(0x1e66f5),
        border: Rgb::hex(0xacb0be),
    };

    /// The tokens for an Omarchy `colors.toml`, in either layout: the named
    /// keys of Omarchy 4 (`lighter_background`, `dark_foreground`, `muted`,
    /// `selection`) or the terminal keys of earlier themes (`color0` to
    /// `color15`, `selection_background`). A key the file lacks falls back to
    /// one derived from the others, then to [`Tokens::fallback`] for the
    /// theme's `mode`, or, without one, for how light its background is.
    /// Text that would be hard to read on the backgrounds is made readable.
    pub fn from_colors_toml(text: &str) -> Result<Self, ThemeError> {
        let table: toml::Table =
            toml::from_str(text).map_err(|e| ThemeError::Parse(one_line(&e.to_string())))?;
        let get = |key: &str| table.get(key).and_then(|v| v.as_str()).and_then(Rgb::parse);
        let background = get("background");
        let scheme = match table.get("mode").and_then(|v| v.as_str()) {
            Some("light") => Scheme::Light,
            Some("dark") => Scheme::Dark,
            _ => background.map_or(Scheme::Dark, Scheme::of),
        };
        let fallback = Self::fallback(scheme);

        let background = background.unwrap_or(fallback.background);
        let foreground = get("foreground").unwrap_or(fallback.foreground);
        let accent = get("accent")
            .or_else(|| get("color4"))
            .unwrap_or(fallback.accent);
        let lighter_background =
            get("lighter_background").unwrap_or_else(|| background.mix(foreground, 0.06));
        let muted = get("dark_foreground")
            .or_else(|| get("color8"))
            .unwrap_or_else(|| foreground.mix(background, 0.45));
        // The terminal layout's selection_background is for inverted text
        // (dark on light); a selected row keeps its normal text.
        let selection = get("selection").unwrap_or_else(|| background.mix(accent, 0.25));
        let border = get("muted")
            .or_else(|| get("color8").map(|c| c.mix(background, 0.3)))
            .unwrap_or_else(|| foreground.mix(background, 0.7));

        let backgrounds = [background, lighter_background, selection];
        Ok(Self {
            background,
            lighter_background,
            foreground: readable(foreground, &backgrounds, TEXT_CONTRAST),
            muted: readable(muted, &backgrounds, SECONDARY_CONTRAST),
            selection,
            accent: readable(accent, &backgrounds, SECONDARY_CONTRAST),
            border,
        })
    }

    /// The current Omarchy theme's tokens: the first `colors.toml` of
    /// [`theme_paths`] that exists. `None` without one, and for one that
    /// can't be read, with a line on stderr.
    pub fn omarchy() -> Option<Self> {
        let path = theme_paths().into_iter().find(|p| p.exists())?;
        let read = std::fs::read_to_string(&path)
            .map_err(|e| ThemeError::Read(e.to_string()))
            .and_then(|text| Self::from_colors_toml(&text));
        read.map_err(|err| {
            eprintln!(
                "gutenberg: theme {}: {err}; using the built-in colors",
                path.display()
            );
        })
        .ok()
    }
}

/// `color` as given, or moved toward black or white, whichever the
/// backgrounds call for, just far enough to reach `ratio` against every one
/// of them. A color that already reads is left exactly as it is.
pub fn readable(color: Rgb, backgrounds: &[Rgb], ratio: f32) -> Rgb {
    let reads = |c: Rgb| backgrounds.iter().all(|&bg| contrast(c, bg) >= ratio);
    if reads(color) {
        return color;
    }
    let mean = backgrounds.iter().map(|b| b.luminance()).sum::<f32>() / backgrounds.len() as f32;
    let dark = mean < 0.5;
    let target = if dark { Rgb::WHITE } else { Rgb::BLACK };
    (1..=50)
        .map(|step| color.mix(target, step as f32 / 50.0))
        .find(|&c| reads(c))
        .unwrap_or(target)
}

/// Where Omarchy keeps the current theme, newest layout first:
/// `$XDG_STATE_HOME/omarchy/current/theme` (Omarchy 3 and later), then
/// `$XDG_CONFIG_HOME/omarchy/current/theme`.
pub fn theme_paths() -> Vec<PathBuf> {
    let Some(dirs) = directories::BaseDirs::new() else {
        return Vec::new();
    };
    let state = dirs
        .state_dir()
        .map_or_else(|| dirs.home_dir().join(".local/state"), Path::to_path_buf);
    candidate_paths(&state, dirs.config_dir())
}

fn candidate_paths(state: &Path, config: &Path) -> Vec<PathBuf> {
    [state, config]
        .iter()
        .map(|dir| dir.join("omarchy/current/theme/colors.toml"))
        .collect()
}

/// Why a theme file gave no tokens. Each displays as one line.
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("can't read it: {0}")]
    Read(String),
    #[error("not TOML: {0}")]
    Parse(String),
}

fn one_line(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_and_splits_channels() {
        assert_eq!(
            Rgb::hex(0x1a1b26),
            Rgb {
                r: 0x1a,
                g: 0x1b,
                b: 0x26
            }
        );
        assert_eq!(Rgb::parse("#1a1b26"), Some(Rgb::hex(0x1a1b26)));
        assert_eq!(Rgb::parse(" 1A1B26 "), Some(Rgb::hex(0x1a1b26)));
        assert_eq!(Rgb::parse("#fa0"), Some(Rgb::hex(0xffaa00)));
        assert_eq!(Rgb::parse("rgba(1,2,3)"), None);
        assert_eq!(Rgb::parse("#12345"), None);
    }

    #[test]
    fn contrast_matches_wcag() {
        assert!((contrast(Rgb::BLACK, Rgb::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Rgb::hex(0x777777), Rgb::hex(0x777777)) - 1.0).abs() < 0.001);
        // WebAIM's checker gives 4.48 for #777777 on white.
        assert!((contrast(Rgb::hex(0x777777), Rgb::WHITE) - 4.48).abs() < 0.01);
    }

    #[test]
    fn readable_leaves_good_colors_and_nudges_bad_ones_just_enough() {
        let bg = [Rgb::hex(0x101315)];
        let good = Rgb::hex(0xcacccc);
        assert_eq!(readable(good, &bg, 4.5), good);
        let dim = Rgb::hex(0x4b4e55);
        let fixed = readable(dim, &bg, 3.0);
        assert!(contrast(fixed, bg[0]) >= 3.0);
        // One step less would not have been enough: it stopped as soon as it read.
        let before = fixed.mix(dim, 1.0 / 25.0);
        assert!(
            fixed.luminance() > dim.luminance(),
            "lighter on a dark background"
        );
        assert!(contrast(before, bg[0]) < 3.0 + 0.2);
        // On a light background it darkens.
        let pale = readable(Rgb::hex(0xbbbbbb), &[Rgb::hex(0xeeeeee)], 4.5);
        assert!(pale.luminance() < Rgb::hex(0xbbbbbb).luminance());
        assert!(contrast(pale, Rgb::hex(0xeeeeee)) >= 4.5);
    }

    #[test]
    fn missing_keys_are_derived_and_the_rest_is_used() {
        let tokens = Tokens::from_colors_toml(
            "background = \"#000000\"\nforeground = \"#ffffff\"\naccent = \"#ff0000\"",
        )
        .unwrap();
        assert_eq!(tokens.background, Rgb::BLACK);
        assert_eq!(tokens.foreground, Rgb::WHITE);
        assert_eq!(tokens.lighter_background, Rgb::BLACK.mix(Rgb::WHITE, 0.06));
        assert_eq!(tokens.selection, Rgb::BLACK.mix(Rgb::hex(0xff0000), 0.25));
        // An empty file takes the fallback's base colors and derives the
        // rest from them; a broken file is an error.
        let empty = Tokens::from_colors_toml("").unwrap();
        let fallback = Tokens::fallback(Scheme::Dark);
        assert_eq!(
            (empty.background, empty.foreground, empty.accent),
            (fallback.background, fallback.foreground, fallback.accent)
        );
        assert!(Tokens::from_colors_toml("background = ").is_err());
    }

    #[test]
    fn mode_picks_the_colors_a_file_leaves_out() {
        let base = |text: &str| {
            let t = Tokens::from_colors_toml(text).unwrap();
            (t.background, t.foreground, t.accent)
        };
        let light = Tokens::fallback(Scheme::Light);
        let dark = Tokens::fallback(Scheme::Dark);
        let light = (light.background, light.foreground, light.accent);
        let dark = (dark.background, dark.foreground, dark.accent);
        assert_eq!(base("mode = \"light\""), light);
        assert_eq!(base("mode = \"dark\""), dark);
        // Without a mode, the background says which.
        assert_eq!(base("background = \"#fdf6e3\"").2, light.2);
        assert_eq!(base("background = \"#002b36\"").2, dark.2);
        // A mode it doesn't know is no mode.
        assert_eq!(
            base("mode = \"sepia\"\nbackground = \"#fdf6e3\"").2,
            light.2
        );
    }

    #[test]
    fn a_light_terminal_theme_derives_light_panes() {
        // Solarized Light in the terminal layout: no pane keys at all.
        let t = Tokens::from_colors_toml(
            "background = \"#fdf6e3\"\nforeground = \"#657b83\"\ncolor4 = \"#268bd2\"\ncolor8 = \"#002b36\"",
        )
        .unwrap();
        for pane in [t.lighter_background, t.selection] {
            assert_eq!(Scheme::of(pane), Scheme::Light, "{pane:?}");
            assert!(pane.luminance() < t.background.luminance(), "{pane:?}");
        }
    }

    #[test]
    fn the_state_path_comes_before_the_config_path() {
        assert_eq!(
            candidate_paths(Path::new("/s"), Path::new("/c")),
            [
                PathBuf::from("/s/omarchy/current/theme/colors.toml"),
                PathBuf::from("/c/omarchy/current/theme/colors.toml"),
            ]
        );
    }
}
