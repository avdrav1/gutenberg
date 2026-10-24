//! S4.1 acceptance, by number: every stock Omarchy theme maps to tokens whose
//! text reads on every background it's drawn on. The themes are copies of
//! `/usr/share/omarchy/themes/*/colors.toml` in `tests/fixtures/themes/`.

use gutenberg_theme::{contrast, Rgb, Scheme, Tokens, SECONDARY_CONTRAST, TEXT_CONTRAST};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/themes");

fn themes() -> Vec<(String, String)> {
    let mut themes: Vec<(String, String)> = std::fs::read_dir(FIXTURES)
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            (path.extension()? == "toml").then(|| {
                (
                    path.file_stem().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&path).unwrap(),
                )
            })
        })
        .collect();
    themes.sort();
    themes
}

fn key(text: &str, name: &str) -> Option<Rgb> {
    let table: toml::Table = toml::from_str(text).unwrap();
    Rgb::parse(table.get(name)?.as_str()?)
}

#[test]
fn every_stock_theme_reads_on_every_background() {
    let themes = themes();
    assert_eq!(
        themes.len(),
        24,
        "23 stock themes and one terminal-layout one"
    );
    for (name, text) in &themes {
        let t = Tokens::from_colors_toml(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        for bg in [t.background, t.lighter_background, t.selection] {
            assert!(
                contrast(t.foreground, bg) >= TEXT_CONTRAST,
                "{name}: text on {bg:?}"
            );
            assert!(
                contrast(t.muted, bg) >= SECONDARY_CONTRAST,
                "{name}: muted on {bg:?}"
            );
            assert!(
                contrast(t.accent, bg) >= SECONDARY_CONTRAST,
                "{name}: accent on {bg:?}"
            );
        }
    }
}

#[test]
fn a_theme_keeps_its_own_colors() {
    for (name, text) in themes() {
        let t = Tokens::from_colors_toml(&text).unwrap();
        // Backgrounds are never changed; they are the theme.
        if let Some(bg) = key(&text, "background") {
            assert_eq!(t.background, bg, "{name}");
        }
        if let Some(lighter) = key(&text, "lighter_background") {
            assert_eq!(t.lighter_background, lighter, "{name}");
        }
        // Text the theme already made readable is used as given.
        let backgrounds = [t.background, t.lighter_background, t.selection];
        if let Some(fg) = key(&text, "foreground") {
            if backgrounds
                .iter()
                .all(|&bg| contrast(fg, bg) >= TEXT_CONTRAST)
            {
                assert_eq!(t.foreground, fg, "{name}");
            }
        }
    }
}

#[test]
fn the_built_in_themes_are_tokyo_night_and_catppuccin_latte() {
    for (file, scheme) in [
        ("tokyo-night", Scheme::Dark),
        ("catppuccin-latte", Scheme::Light),
    ] {
        let text = std::fs::read_to_string(format!("{FIXTURES}/{file}.toml")).unwrap();
        assert_eq!(
            Tokens::from_colors_toml(&text).unwrap(),
            Tokens::fallback(scheme),
            "{file}"
        );
    }
}

#[test]
fn the_terminal_layout_maps_too() {
    let text = std::fs::read_to_string(format!("{FIXTURES}/terminal-style-solitude.toml")).unwrap();
    let t = Tokens::from_colors_toml(&text).unwrap();
    assert_eq!(t.background, Rgb::hex(0x101315));
    assert_eq!(t.foreground, Rgb::hex(0xcacccc));
    // Not its selection_background (#798186), which is meant for dark text.
    assert_ne!(t.selection, Rgb::hex(0x798186));
}

#[test]
fn light_themes_never_show_dark_panes() {
    let light: Vec<_> = themes()
        .into_iter()
        .filter(|(_, text)| text.contains("mode = \"light\""))
        .collect();
    assert_eq!(
        light.len(),
        5,
        "catppuccin-latte, flexoki-light, lupine, rose-pine, white"
    );
    for (name, text) in light {
        // As shipped, and as a file that sets only its base colors, the way
        // the older terminal layout does.
        let bare: String = text
            .lines()
            .filter(|l| {
                ![
                    "lighter_background",
                    "selection",
                    "dark_foreground",
                    "muted",
                ]
                .iter()
                .any(|k| l.starts_with(&format!("{k} ")))
            })
            .map(|l| format!("{l}\n"))
            .collect();
        for (layout, text) in [("as shipped", &text), ("bare", &bare)] {
            let t = Tokens::from_colors_toml(text).unwrap();
            for pane in [t.background, t.lighter_background, t.selection] {
                assert_eq!(Scheme::of(pane), Scheme::Light, "{name} {layout}: {pane:?}");
            }
        }
    }
}
