//! A screenshot of the list in the interesting order (S10.4), drawn
//! headlessly in the current Omarchy theme (or the built-in dark one).
//!
//! ```sh
//! ICED_TEST_BACKEND=tiny-skia cargo run -p gutenberg-ui --features shots \
//!     --example interest_shot -- docs/screenshots/s10-4 [store.db]
//! ```
//!
//! Writes `<out>-tiny-skia.png`. Made-up articles and signals, or with a
//! store file, its own: a copy, since it's opened and may be migrated.

use std::path::Path;

use gutenberg_theme::Tokens;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (out, store) = match args.as_slice() {
        [out] => (out, None),
        [out, store] => (out, Some(Path::new(store))),
        _ => {
            eprintln!("usage: interest_shot <output> [store.db]");
            std::process::exit(2);
        }
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let tokens = [
        "/.local/state/omarchy/current/theme/colors.toml",
        "/.config/omarchy/current/theme/colors.toml",
    ]
    .iter()
    .find_map(|p| std::fs::read_to_string(format!("{home}{p}")).ok())
    .and_then(|text| Tokens::from_colors_toml(&text).ok())
    .unwrap_or_else(|| {
        Tokens::from_colors_toml(include_str!(
            "../../../tests/fixtures/themes/tokyo-night.toml"
        ))
        .expect("built-in")
    });
    gutenberg_ui::shots::render_interesting(tokens, (1400.0, 520.0), store, Path::new(out));
}
