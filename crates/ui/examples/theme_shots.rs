//! Screenshots of the window in every Omarchy theme, without touching the
//! desktop's theme: each `colors.toml` is mapped to tokens and the real
//! window is drawn headlessly over sample articles.
//!
//! ```sh
//! ICED_TEST_BACKEND=tiny-skia cargo run -p gutenberg-ui --features shots \
//!     --example theme_shots -- tests/fixtures/themes docs/themes
//! ```
//!
//! Reads `<dir>/*.toml` (or `<dir>/*/colors.toml`, as in
//! `/usr/share/omarchy/themes`) and writes `<out>/<name>.png`, replacing
//! any there. Prints each theme's contrast ratios.

use std::path::{Path, PathBuf};

use gutenberg_theme::{contrast, Tokens};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [themes, out] = args.as_slice() else {
        eprintln!("usage: theme_shots <themes dir> <output dir>");
        std::process::exit(2);
    };
    let out = Path::new(out);
    std::fs::create_dir_all(out).expect("output directory");
    for (name, path) in theme_files(Path::new(themes)) {
        let text = std::fs::read_to_string(&path).expect("theme file");
        let tokens = match Tokens::from_colors_toml(&text) {
            Ok(tokens) => tokens,
            Err(err) => {
                eprintln!("{name}: {err}");
                continue;
            }
        };
        let min = |c| {
            [
                tokens.background,
                tokens.lighter_background,
                tokens.selection,
            ]
            .iter()
            .map(|&bg| contrast(c, bg))
            .fold(f32::MAX, f32::min)
        };
        println!(
            "{name:<24} text {:>5.2}  muted {:>4.2}  links {:>4.2}",
            min(tokens.foreground),
            min(tokens.muted),
            min(tokens.accent)
        );
        // iced_test names the file `<stem>-<renderer>.png`; move it in place.
        let stem = out.join(&name);
        let written = PathBuf::from(format!("{}-tiny-skia.png", stem.display()));
        let _ = std::fs::remove_file(&written);
        gutenberg_ui::shots::render(tokens, (1200.0, 720.0), &stem);
        std::fs::rename(&written, stem.with_extension("png")).expect("rename screenshot");
    }
}

/// `(name, colors.toml)` for every theme in `dir`, by name.
fn theme_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .expect("themes directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_stem()?.to_string_lossy().into_owned();
            if path.extension().is_some_and(|e| e == "toml") {
                Some((name, path))
            } else {
                let colors = path.join("colors.toml");
                colors.exists().then_some((name, colors))
            }
        })
        .collect();
    found.sort();
    found
}
